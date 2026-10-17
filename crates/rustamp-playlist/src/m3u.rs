//! M3U and M3U8.
//!
//! M3U is the least structured of the playlist formats: one location per line,
//! `#` starts a comment, blank lines are ignored, and everything else is
//! optional. The reader therefore accepts both the extended form and a bare
//! list of file names.
//!
//! Recognized directives:
//!
//! | Directive | Handling |
//! |---|---|
//! | `#EXTM3U` | header; ignored |
//! | `#EXTINF:<length>,<title>` | length in seconds (fractional allowed, `-1` = unknown) and the title |
//! | `#EXTALBUM:<album>`, `#EXTART:<artist>` | recognized and dropped, see below |
//! | any other `#…` line | comment; ignored |
//!
//! `#EXTALBUM`/`#EXTART` are parsed so they cannot be mistaken for a location,
//! but [`LoadedEntry`] has no album/artist field and this crate must not do
//! title formatting (spec S14), so the values are dropped. Preserving them
//! would need a core contract change (see `CONTRACT_NOTES.md`).
//!
//! Deterministic rules for ambiguous input:
//! * The **most recent** `#EXTINF` before a location line applies to it.
//! * A line is trimmed of surrounding whitespace before it is resolved.
//! * A title that is empty after trimming becomes `None`; a length of `-1`,
//!   an unparseable length or a negative length becomes "no duration".
//! * The length is parsed from the leading number, so IPTV-style attributes
//!   (`#EXTINF:5 tvg-id="x",Title`) do not break it.
//! * Lines are split on LF, CRLF and lone CR.

use std::path::Path;

use crate::entry::{LoadedEntry, PlaylistReport, SkippedEntry, normalize_title};
use crate::location::Base;
use crate::time::{format_seconds, parse_leading_seconds_ms};

/// Reads M3U text, decoding UTF-8 first and falling back to Windows-1252.
pub fn read(bytes: &[u8], base: &Path) -> Vec<LoadedEntry> {
    read_report(bytes, base).into_entries()
}

/// Reads M3U text, returning surviving entries and any skipped entries.
pub fn read_report(bytes: &[u8], base: &Path) -> PlaylistReport {
    read_text_report(&crate::cp1252::decode_text(bytes), base)
}

/// Reads M3U8 text, which is UTF-8 by definition.
pub fn read_utf8(bytes: &[u8], base: &Path) -> Vec<LoadedEntry> {
    read_utf8_report(bytes, base).into_entries()
}

/// Reads M3U8 text, returning surviving entries and any skipped entries.
pub fn read_utf8_report(bytes: &[u8], base: &Path) -> PlaylistReport {
    read_text_report(&crate::cp1252::decode_utf8_lossy(bytes), base)
}

/// Reads already-decoded M3U text.
pub fn read_text(text: &str, base: &Path) -> Vec<LoadedEntry> {
    read_text_report(text, base).into_entries()
}

/// Reads already-decoded M3U text, returning surviving entries and any skipped entries.
pub fn read_text_report(text: &str, base: &Path) -> PlaylistReport {
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let base = Base::dir(base);
    let mut entries = Vec::new();
    let mut skips = Vec::new();
    let mut pending_title: Option<String> = None;
    let mut pending_duration: Option<u64> = None;

    for line in text.split(['\r', '\n']) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            if let Some(tail) = strip_keyword(rest, "EXTINF") {
                if pending_title.is_some() || pending_duration.is_some() {
                    let desc = match pending_title.take() {
                        Some(t) => format!("dangling #EXTINF without a location line: {t}"),
                        None => "dangling #EXTINF without a location line".to_string(),
                    };
                    skips.push(SkippedEntry::new(desc));
                }
                let args = tail.strip_prefix(':').unwrap_or(tail);
                let (length, title) = match args.split_once(',') {
                    Some((length, title)) => (length, Some(title)),
                    None => (args, None),
                };
                pending_duration = parse_leading_seconds_ms(length);
                pending_title = title.and_then(normalize_title);
            } else if strip_keyword(rest, "EXTALBUM").is_some()
                || strip_keyword(rest, "EXTART").is_some()
            {
                // Recognized, deliberately dropped: no album/artist field
                // exists in `LoadedEntry` and title formatting is S14.
            }
            // `#EXTM3U`, `#EXTENC`, `#PLAYLIST` and plain comments: ignored.
            continue;
        }
        if let Some(location) = base.resolve(line) {
            entries.push(LoadedEntry {
                location,
                title: pending_title.take(),
                duration_ms: pending_duration.take(),
            });
        } else {
            let desc = match pending_title.take() {
                Some(t) => format!("unresolvable location `{line}` ({t})"),
                None => format!("unresolvable location `{line}`"),
            };
            skips.push(SkippedEntry::new(desc));
        }
        pending_title = None;
        pending_duration = None;
    }
    if pending_title.is_some() || pending_duration.is_some() {
        let desc = match pending_title.take() {
            Some(t) => format!("dangling #EXTINF without a location line: {t}"),
            None => "dangling #EXTINF without a location line".to_string(),
        };
        skips.push(SkippedEntry::new(desc));
    }
    PlaylistReport::new(entries, skips)
}

/// Writes M3U. Line endings are CRLF and the encoding is UTF-8, matching what
/// the format conventionally uses.
///
/// An entry with a title or a duration gets an `#EXTINF` line; the `#EXTM3U`
/// header is written exactly when at least one `#EXTINF` is. An unknown
/// duration is written as `-1`, the conventional marker that the reader maps
/// back to "no duration".
pub fn write(entries: &[LoadedEntry]) -> Vec<u8> {
    let mut out = String::new();
    if entries
        .iter()
        .any(|e| e.title.is_some() || e.duration_ms.is_some())
    {
        out.push_str("#EXTM3U\r\n");
    }
    for entry in entries {
        if entry.title.is_some() || entry.duration_ms.is_some() {
            out.push_str("#EXTINF:");
            match entry.duration_ms {
                Some(ms) => out.push_str(&format_seconds(ms)),
                None => out.push_str("-1"),
            }
            out.push(',');
            out.push_str(&sanitize_line(entry.title.as_deref().unwrap_or("")));
            out.push_str("\r\n");
        }
        out.push_str(&sanitize_line(&entry.location.to_display_string()));
        out.push_str("\r\n");
    }
    out.into_bytes()
}

/// Collapses CR/LF (which would break the one-entry-per-line structure) into
/// spaces and trims the result. The readers trim as well, so trimming here
/// keeps write → read round-trips stable.
pub(crate) fn sanitize_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ").trim().to_owned()
}

/// Case-insensitive keyword match directly after the `#`.
///
/// The keyword must be followed by `:`, whitespace or the end of the line, so
/// `#EXTINFOMATIC` is not mistaken for `#EXTINF`.
fn strip_keyword<'a>(rest: &'a str, keyword: &str) -> Option<&'a str> {
    if rest.len() < keyword.len() {
        return None;
    }
    if !rest.as_bytes()[..keyword.len()].eq_ignore_ascii_case(keyword.as_bytes()) {
        return None;
    }
    let tail = &rest[keyword.len()..];
    if tail.is_empty() || tail.starts_with(':') || tail.starts_with(char::is_whitespace) {
        Some(tail)
    } else {
        None
    }
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
    fn reads_a_full_extended_playlist() {
        let text = "#EXTM3U\r\n\
                    #EXTINF:123,A Song\r\n\
                    a.mp3\r\n\
                    #EXTINF:4.5,Another\r\n\
                    sub/b.mp3\r\n";
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].location, file("C:/Music/Lists/a.mp3"));
        assert_eq!(entries[0].title.as_deref(), Some("A Song"));
        assert_eq!(entries[0].duration_ms, Some(123_000));
        assert_eq!(entries[1].location, file("C:/Music/Lists/sub/b.mp3"));
        assert_eq!(entries[1].title.as_deref(), Some("Another"));
        assert_eq!(entries[1].duration_ms, Some(4500));
    }

    #[test]
    fn reads_a_headerless_playlist_with_lf_only() {
        let entries = read_text("a.mp3\nb.mp3\n", &base());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].location, file("C:/Music/Lists/b.mp3"));
        assert_eq!(entries[0].title, None);
        assert_eq!(entries[0].duration_ms, None);
    }

    #[test]
    fn reads_crlf_and_lone_cr() {
        let entries = read_text("#EXTM3U\ra.mp3\r#comment\rb.mp3", &base());
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn ignores_comments_blank_lines_and_album_artist_hints() {
        let text = "#EXTM3U\n\n;not a comment in m3u\n#EXTALBUM:Some Album\n#EXTART:Some Artist\n#EXTINF:1,Real\na.mp3\n#EXTM3U\n";
        let entries = read_text(text, &base());
        // ";not a comment in m3u" is a plain location line, as M3U says.
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0].location,
            file("C:/Music/Lists/;not a comment in m3u")
        );
        assert_eq!(entries[1].title.as_deref(), Some("Real"));
    }

    #[test]
    fn handles_streams_and_odd_extinf_forms() {
        let text = "#EXTM3U\n#EXTINF:-1,Radio\nhttp://example.com/live\n\
                    #EXTINF:5 tvg-id=\"x\",IPTV\nc.mp3\n\
                    #EXTINF:7\nd.mp3\n\
                    #EXTINF:notanumber,Bad\nf.mp3\n";
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 4);
        assert_eq!(
            entries[0].location,
            MediaLocation::Url("http://example.com/live".into())
        );
        assert_eq!(entries[0].duration_ms, None);
        assert_eq!(entries[0].title.as_deref(), Some("Radio"));
        assert_eq!(entries[1].duration_ms, Some(5000));
        assert_eq!(entries[1].title.as_deref(), Some("IPTV"));
        assert_eq!(entries[2].duration_ms, Some(7000));
        assert_eq!(entries[2].title, None);
        assert_eq!(entries[3].duration_ms, None);
        assert_eq!(entries[3].title.as_deref(), Some("Bad"));
    }

    #[test]
    fn truncated_extinf_is_tolerated() {
        let text = "#EXTM3U\n#EXTINF:\na.mp3\n#EXTINF\nb.mp3\n#EXTINF:12\n";
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].duration_ms, None);
        assert_eq!(entries[1].duration_ms, None);
    }

    #[test]
    fn most_recent_extinf_wins() {
        let entries = read_text("#EXTINF:1,First\n#EXTINF:2,Second\na.mp3\n", &base());
        assert_eq!(entries[0].title.as_deref(), Some("Second"));
        assert_eq!(entries[0].duration_ms, Some(2000));
    }

    #[test]
    fn windows_1252_fallback_for_m3u() {
        // 0xE4 is 'ä' in Windows-1252 and invalid UTF-8 on its own.
        let mut bytes = b"#EXTM3U\r\n#EXTINF:1,M".to_vec();
        bytes.push(0xE4);
        bytes.extend_from_slice(b"rchen\r\na.mp3\r\n");
        let entries = read(&bytes, &base());
        assert_eq!(entries[0].title.as_deref(), Some("M\u{00E4}rchen"));
    }

    #[test]
    fn m3u8_uses_utf8_with_replacement_characters() {
        let mut bytes = b"#EXTM3U\r\n#EXTINF:1,M".to_vec();
        bytes.push(0xFF);
        bytes.extend_from_slice(b"\r\na.mp3\r\n");
        let entries = read_utf8(&bytes, &base());
        assert_eq!(entries[0].title.as_deref(), Some("M\u{FFFD}"));
    }

    #[test]
    fn write_then_read_round_trips() {
        let entries = vec![
            LoadedEntry::new(file("C:/Music/a.mp3"))
                .with_title("A, with comma")
                .with_duration_ms(123_456),
            LoadedEntry::new(MediaLocation::Url("http://example.com/live".into()))
                .with_title("Radio"),
            LoadedEntry::new(file("C:/Music/b.mp3")).with_duration_ms(1000),
            LoadedEntry::new(file("C:/Music/c.mp3")),
        ];
        let bytes = write(&entries);
        assert!(bytes.starts_with(b"#EXTM3U\r\n"));
        let back = read(&bytes, &base());
        assert_eq!(back, entries);
    }

    #[test]
    fn write_without_metadata_has_no_header() {
        let entries = vec![LoadedEntry::new(file("C:/Music/a.mp3"))];
        let bytes = write(&entries);
        assert_eq!(String::from_utf8(bytes).unwrap(), "C:/Music/a.mp3\r\n");
    }

    #[test]
    fn write_sanitizes_newlines() {
        let entries = vec![LoadedEntry::new(file("C:/a.mp3")).with_title("a\nb")];
        let text = String::from_utf8(write(&entries)).unwrap();
        assert_eq!(text, "#EXTM3U\r\n#EXTINF:-1,a b\r\nC:/a.mp3\r\n");
    }

    #[test]
    fn empty_input_yields_no_entries() {
        assert!(read(b"", &base()).is_empty());
        assert!(read(b"#EXTM3U\r\n\r\n", &base()).is_empty());
    }
}
