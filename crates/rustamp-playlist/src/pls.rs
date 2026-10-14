//! PLS.
//!
//! The PLS dialect is an INI-like list of numbered keys:
//!
//! ```text
//! [playlist]
//! File1=C:\Music\a.mp3
//! Title1=A
//! Length1=123
//! NumberOfEntries=1
//! Version=2
//! ```
//!
//! Deterministic rules for ambiguous input:
//! * Keys are case-insensitive; `FileN`, `TitleN` and `LengthN` must carry a
//!   decimal index. `FileX=`, `File=` and a non-numeric index are ignored.
//! * Keys are read **before the first section header** and inside
//!   `[playlist]`; keys inside any other section are ignored. A PLS file
//!   without a `[playlist]` header therefore still parses.
//! * Entries are emitted in **ascending index order**, not in the order the
//!   lines appear, and gaps in the numbering are allowed (`File1`, `File3`).
//! * A **duplicate index** is resolved by "last occurrence wins", for the file,
//!   the title and the length independently; an empty value counts as a
//!   removal, so `File1=a.mp3` followed by `File1=` leaves no entry.
//! * `FileN` without a value, or with an empty value, is dropped. A `TitleN`
//!   or `LengthN` without a matching `FileN` is ignored.
//! * `NumberOfEntries` and `Version` are informational and ignored: the actual
//!   `FileN` keys decide how many entries there are.
//! * A length that is missing, non-numeric or negative means "no duration".
//! * Values are trimmed of surrounding whitespace and otherwise taken
//!   **verbatim**. PLS has no escaping mechanism, so surrounding quotes are
//!   *not* interpreted: stripping them would make a title that begins and ends
//!   with a quote character impossible to represent. `File1=""` is a file
//!   whose name is two quote characters.

use std::collections::BTreeMap;
use std::path::Path;

use crate::entry::{LoadedEntry, normalize_title};
use crate::location::Base;
use crate::time::{format_seconds, parse_seconds_ms};

/// Reads PLS text.
pub fn read(bytes: &[u8], base: &Path) -> Vec<LoadedEntry> {
    read_text(&crate::cp1252::decode_text(bytes), base)
}

/// Reads already-decoded PLS text.
pub fn read_text(text: &str, base: &Path) -> Vec<LoadedEntry> {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let base = Base::dir(base);
    let mut section: Option<String> = None;
    let mut files: BTreeMap<u32, String> = BTreeMap::new();
    let mut titles: BTreeMap<u32, String> = BTreeMap::new();
    let mut lengths: BTreeMap<u32, String> = BTreeMap::new();

    for line in text.split(['\r', '\n']) {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(name) = section_name(line) {
            section = Some(name.to_ascii_lowercase());
            continue;
        }
        let active = match &section {
            Some(name) => name == "playlist",
            None => true,
        };
        if !active {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().to_owned();
        let (name, index) = split_indexed_key(key.trim());
        let Some(index) = index else {
            continue;
        };
        let map = match name.as_str() {
            "file" => &mut files,
            "title" => &mut titles,
            "length" => &mut lengths,
            _ => continue,
        };
        // "Last occurrence wins" applies to empty values too: `File1=` removes
        // a previously seen `File1`.
        if value.is_empty() {
            map.remove(&index);
        } else {
            map.insert(index, value);
        }
    }

    let mut entries = Vec::with_capacity(files.len());
    for (index, file) in &files {
        let Some(location) = base.resolve(file) else {
            continue;
        };
        entries.push(LoadedEntry {
            location,
            title: titles.get(index).and_then(|t| normalize_title(t)),
            duration_ms: lengths.get(index).and_then(|l| parse_seconds_ms(l)),
        });
    }
    entries
}

/// Writes PLS with CRLF line endings, UTF-8, 1-based indices and an accurate
/// `NumberOfEntries`.
///
/// `TitleN` and `LengthN` are written only when the entry has them, so an
/// absent title or duration survives a round-trip.
pub fn write(entries: &[LoadedEntry]) -> Vec<u8> {
    let mut out = String::new();
    out.push_str("[playlist]\r\n");
    for (offset, entry) in entries.iter().enumerate() {
        let index = offset + 1;
        out.push_str(&format!(
            "File{index}={}\r\n",
            crate::m3u::sanitize_line(&entry.location.to_display_string())
        ));
        if let Some(title) = &entry.title {
            out.push_str(&format!(
                "Title{index}={}\r\n",
                crate::m3u::sanitize_line(title)
            ));
        }
        if let Some(ms) = entry.duration_ms {
            out.push_str(&format!("Length{index}={}\r\n", format_seconds(ms)));
        }
    }
    out.push_str(&format!("NumberOfEntries={}\r\n", entries.len()));
    out.push_str("Version=2\r\n");
    out.into_bytes()
}

/// The section name of a `[name]` line.
fn section_name(line: &str) -> Option<&str> {
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    let name = inner.trim();
    (!name.is_empty()).then_some(name)
}

/// Splits `File12` into `("file", Some(12))`. Keys without a decimal suffix
/// (`File`, `NumberOfEntries`) get `None`.
fn split_indexed_key(key: &str) -> (String, Option<u32>) {
    let lower = key.to_ascii_lowercase();
    let end = lower.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (name, digits) = lower.split_at(end);
    let index = if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok()
    };
    (name.to_owned(), index)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rustamp_core::playlist::MediaLocation;

    use super::*;

    fn file(name: &str) -> MediaLocation {
        MediaLocation::File(PathBuf::from(name))
    }

    fn base() -> PathBuf {
        PathBuf::from("C:/Music/Lists")
    }

    #[test]
    fn reads_a_classic_pls() {
        let text = "[playlist]\r\n\
                    File1=a.mp3\r\nTitle1=First\r\nLength1=123\r\n\
                    File2=b.mp3\r\nTitle2=Second\r\nLength2=4.5\r\n\
                    NumberOfEntries=2\r\nVersion=2\r\n";
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].location, file("C:/Music/Lists/a.mp3"));
        assert_eq!(entries[0].title.as_deref(), Some("First"));
        assert_eq!(entries[0].duration_ms, Some(123_000));
        assert_eq!(entries[1].duration_ms, Some(4500));
    }

    #[test]
    fn keys_are_case_insensitive() {
        let text = "[PlayList]\nFILE1=a.mp3\ntitle1=T\nlEnGtH1=2\n";
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title.as_deref(), Some("T"));
        assert_eq!(entries[0].duration_ms, Some(2000));
    }

    #[test]
    fn missing_indices_are_tolerated_and_sorted() {
        let text = "[playlist]\nFile10=ten.mp3\nFile1=one.mp3\nFile3=three.mp3\n";
        let entries = read_text(text, &base());
        let names: Vec<_> = entries.iter().map(|e| e.location.file_name()).collect();
        assert_eq!(names, ["one.mp3", "three.mp3", "ten.mp3"]);
    }

    #[test]
    fn duplicate_indices_last_wins() {
        let text = "[playlist]\nFile1=first.mp3\nFile1=second.mp3\nTitle1=A\nTitle1=B\nLength1=1\nLength1=2\n";
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].location.file_name(), "second.mp3");
        assert_eq!(entries[0].title.as_deref(), Some("B"));
        assert_eq!(entries[0].duration_ms, Some(2000));
    }

    #[test]
    fn non_numeric_and_unknown_keys_are_ignored() {
        let text = "[playlist]\nFileX=a.mp3\nFile=b.mp3\nFile1=c.mp3\nTitle0=orphan\nLength0=9\nVersion=x\n";
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].location.file_name(), "c.mp3");
        assert_eq!(entries[0].title, None);
        assert_eq!(entries[0].duration_ms, None);
    }

    #[test]
    fn keys_outside_the_playlist_section_are_ignored_but_a_missing_header_is_not_fatal() {
        let text = "[other]\nFile1=x.mp3\n[playlist]\nFile2=y.mp3\n";
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].location.file_name(), "y.mp3");

        let headerless = read_text("File1=z.mp3\n", &base());
        assert_eq!(headerless.len(), 1);
        assert_eq!(headerless[0].location.file_name(), "z.mp3");
    }

    #[test]
    fn values_are_verbatim_and_empty_ones_are_dropped() {
        let text = "[playlist]\nFile1=\"a.mp3\"\nFile2=\nFile3=   \n";
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].location, file("C:/Music/Lists/\"a.mp3\""));
    }

    #[test]
    fn an_empty_duplicate_removes_the_entry() {
        let text = "[playlist]\nFile1=a.mp3\nFile1=\n";
        assert!(read_text(text, &base()).is_empty());
    }

    #[test]
    fn negative_and_broken_lengths_mean_no_duration() {
        let text = "[playlist]\nFile1=a.mp3\nLength1=-1\nFile2=b.mp3\nLength2=abc\nFile3=c.mp3\n";
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().all(|e| e.duration_ms.is_none()));
    }

    #[test]
    fn write_then_read_round_trips() {
        let entries = vec![
            LoadedEntry::new(file("C:/Music/a.mp3"))
                .with_title("A = B")
                .with_duration_ms(123_456),
            LoadedEntry::new(MediaLocation::Url("http://example.com/live".into())).with_title("R"),
            LoadedEntry::new(file("C:/Music/b.mp3")),
        ];
        let bytes = write(&entries);
        assert!(bytes.starts_with(b"[playlist]\r\n"));
        let back = read(&bytes, &base());
        assert_eq!(back, entries);
    }

    #[test]
    fn a_value_containing_equals_is_preserved() {
        let entries = vec![LoadedEntry::new(file("C:/a=b.mp3"))];
        let back = read(&write(&entries), &base());
        assert_eq!(back, entries);
    }

    #[test]
    fn empty_input_yields_no_entries() {
        assert!(read(b"", &base()).is_empty());
        assert!(read(b"[playlist]\r\nVersion=2\r\n", &base()).is_empty());
    }
}
