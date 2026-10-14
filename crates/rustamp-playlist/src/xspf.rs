//! XSPF (XML Shareable Playlist Format).
//!
//! ```xml
//! <?xml version="1.0" encoding="UTF-8"?>
//! <playlist version="1" xmlns="http://xspf.org/ns/0/" xml:base="file:///C:/Music/">
//!   <title>Example</title>
//!   <trackList>
//!     <track>
//!       <location>a.mp3</location>
//!       <title>A</title>
//!       <duration>123456</duration>
//!     </track>
//!   </trackList>
//! </playlist>
//! ```
//!
//! Deterministic rules:
//! * Every `<track>` element in document order becomes an entry, whether or
//!   not it sits inside a `<trackList>`; a track without a `<location>` is
//!   skipped.
//! * `<location>`, `<title>` and `<duration>` are read from the text content
//!   of those elements (including CDATA), entity-unescaped and trimmed.
//!   `duration` is in **milliseconds**.
//! * Element and attribute names are matched on their local part
//!   (namespace prefixes are ignored) and compared case-insensitively.
//! * `xml:base` on `<playlist>` and on `<track>` is honoured. It is treated as
//!   a directory prefix: a URL base gets a trailing `/` added, a file base
//!   becomes a directory, and a relative base is a sub-directory.
//! * Unknown elements, `<meta>` elements, comments and processing
//!   instructions are ignored.
//! * The XML encoding declaration is not honoured: the bytes are decoded as
//!   UTF-8 first with Windows-1252 as the fallback.
//! * Malformed XML yields whatever was parsed before the error; it never
//!   fails and never panics. A document cut off in the middle of a `<track>`
//!   still yields that track.

use std::path::Path;

use rustamp_core::playlist::MediaLocation;

use crate::entry::{LoadedEntry, normalize_title};
use crate::location::{Base, path_to_file_url};
use crate::time::parse_ms;
use crate::xml::{Event, Reader, attr, local_name, unescape};

/// Reads XSPF bytes.
pub fn read(bytes: &[u8], base: &Path) -> Vec<LoadedEntry> {
    read_text(&crate::cp1252::decode_text(bytes), base)
}

/// Reads already-decoded XSPF text.
pub fn read_text(text: &str, base: &Path) -> Vec<LoadedEntry> {
    let playlist_base = Base::dir(base);
    let mut effective_base = playlist_base;
    let mut entries = Vec::new();
    let mut track: Option<Track> = None;
    let mut track_base: Option<String> = None;
    let mut capture: Option<Field> = None;
    let mut buffer = String::new();
    let mut reader = Reader::new(text);

    while let Some(event) = reader.next_event() {
        match event {
            Event::Start { name, attrs } => {
                let local = local_name(name).to_ascii_lowercase();
                match local.as_str() {
                    "playlist" if track.is_none() => {
                        if let Some(spec) = attr(&attrs, "base") {
                            effective_base = effective_base.resolve_spec(spec);
                        }
                    }
                    "track" if track.is_none() => {
                        track_base = attr(&attrs, "base").map(str::to_owned);
                        track = Some(Track::default());
                    }
                    "location" | "title" | "duration" if track.is_some() => {
                        capture = Some(Field::from_name(&local));
                        buffer.clear();
                    }
                    _ => {}
                }
            }
            Event::End { name } => {
                let local = local_name(name).to_ascii_lowercase();
                if local == "track" {
                    capture = None;
                    buffer.clear();
                    if let Some(finished) = track.take() {
                        let base = match &track_base {
                            Some(spec) => effective_base.resolve_spec(spec),
                            None => effective_base.clone(),
                        };
                        push_track(&mut entries, finished, &base);
                    }
                    track_base = None;
                } else if let Some(field) = capture {
                    if field.matches(&local) {
                        if let Some(open) = &mut track {
                            field.assign(open, unescape(&buffer));
                        }
                        capture = None;
                        buffer.clear();
                    }
                }
            }
            Event::Text(text) => {
                if capture.is_some() {
                    buffer.push_str(text);
                }
            }
        }
    }

    // A document that was cut off mid-track still contributes the track it was
    // working on, so a partially written playlist is not thrown away.
    if let Some(unfinished) = track.take() {
        let base = match &track_base {
            Some(spec) => effective_base.resolve_spec(spec),
            None => effective_base.clone(),
        };
        push_track(&mut entries, unfinished, &base);
    }
    entries
}

/// Turns a finished track into an entry. A track without a usable location is
/// dropped.
fn push_track(entries: &mut Vec<LoadedEntry>, track: Track, base: &Base) {
    let Some(location) = track.location.as_deref().and_then(|raw| base.resolve(raw)) else {
        return;
    };
    entries.push(LoadedEntry {
        location,
        title: track.title.as_deref().and_then(normalize_title),
        duration_ms: track.duration.as_deref().and_then(parse_ms),
    });
}

/// Writes XSPF, UTF-8 with LF line endings and two-space indentation.
///
/// A [`MediaLocation::File`] that is absolute is written as a `file:` URL
/// (percent-encoded, so the round-trip is exact); a relative one is written as
/// a relative location and therefore resolves against the reading base.
pub fn write(entries: &[LoadedEntry]) -> Vec<u8> {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str("<playlist version=\"1\" xmlns=\"http://xspf.org/ns/0/\">\n");
    out.push_str("  <trackList>\n");
    for entry in entries {
        out.push_str("    <track>\n");
        out.push_str(&format!(
            "      <location>{}</location>\n",
            escape_text(&location_text(&entry.location))
        ));
        if let Some(title) = &entry.title {
            out.push_str(&format!("      <title>{}</title>\n", escape_text(title)));
        }
        if let Some(ms) = entry.duration_ms {
            out.push_str(&format!("      <duration>{ms}</duration>\n"));
        }
        out.push_str("    </track>\n");
    }
    out.push_str("  </trackList>\n");
    out.push_str("</playlist>\n");
    out.into_bytes()
}

/// The `<location>` text for a media location.
fn location_text(location: &MediaLocation) -> String {
    match location {
        MediaLocation::Url(url) => url.clone(),
        MediaLocation::File(path) => {
            if path.is_absolute() {
                path_to_file_url(path)
            } else {
                path.to_string_lossy().into_owned()
            }
        }
    }
}

/// Escapes XML text content, including CR and LF as numeric references so that
/// line-ending normalization cannot change a value.
fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            '\n' => out.push_str("&#10;"),
            _ => out.push(c),
        }
    }
    out
}

/// A track being built, holding raw (unescaped later) text values.
#[derive(Debug, Default)]
struct Track {
    location: Option<String>,
    title: Option<String>,
    duration: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Location,
    Title,
    Duration,
}

impl Field {
    fn from_name(local: &str) -> Self {
        match local {
            "location" => Field::Location,
            "title" => Field::Title,
            _ => Field::Duration,
        }
    }

    fn matches(self, local: &str) -> bool {
        matches!(
            (self, local),
            (Field::Location, "location") | (Field::Title, "title") | (Field::Duration, "duration")
        )
    }

    fn assign(self, track: &mut Track, value: String) {
        match self {
            Field::Location => track.location = Some(value),
            Field::Title => track.title = Some(value),
            Field::Duration => track.duration = Some(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn file(name: &str) -> MediaLocation {
        MediaLocation::File(PathBuf::from(name))
    }

    fn base() -> PathBuf {
        PathBuf::from("C:/Music/Lists")
    }

    #[test]
    fn reads_a_small_xspf() {
        let text = r#"<?xml version="1.0" encoding="UTF-8"?>
<playlist version="1" xmlns="http://xspf.org/ns/0/">
  <title>My list</title>
  <trackList>
    <track>
      <location>a.mp3</location>
      <title>A &amp; B</title>
      <duration>123456</duration>
    </track>
    <track>
      <location>http://example.com/live</location>
      <title>Radio</title>
    </track>
  </trackList>
</playlist>"#;
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].location, file("C:/Music/Lists/a.mp3"));
        assert_eq!(entries[0].title.as_deref(), Some("A & B"));
        assert_eq!(entries[0].duration_ms, Some(123_456));
        assert_eq!(
            entries[1].location,
            MediaLocation::Url("http://example.com/live".into())
        );
        assert_eq!(entries[1].duration_ms, None);
    }

    #[test]
    fn honours_xml_base_on_playlist_and_track() {
        let text = r#"<playlist xml:base="file:///C:/Music/">
  <trackList>
    <track><location>a.mp3</location></track>
    <track xml:base="sub/"><location>b.mp3</location></track>
  </trackList>
</playlist>"#;
        let entries = read_text(text, &base());
        assert_eq!(entries[0].location, file("C:/Music/a.mp3"));
        assert_eq!(entries[1].location, file("C:/Music/sub/b.mp3"));
    }

    #[test]
    fn xml_base_may_be_a_url() {
        let text = r#"<playlist xml:base="http://example.com/music">
  <track><location>a.mp3</location></track>
</playlist>"#;
        let entries = read_text(text, &base());
        assert_eq!(
            entries[0].location,
            MediaLocation::Url("http://example.com/music/a.mp3".into())
        );
    }

    #[test]
    fn ignores_unknown_elements_meta_and_comments() {
        let text = r#"<!-- comment -->
<playlist version="1">
  <title>ignored</title>
  <extension application="http://x/"><extra>nope</extra></extension>
  <trackList>
    <track>
      <meta rel="x">ignored</meta>
      <location><![CDATA[a.mp3]]></location>
      <title>  spaced  </title>
      <unknown>ignored</unknown>
      <duration>not-a-number</duration>
    </track>
  </trackList>
</playlist>"#;
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].location, file("C:/Music/Lists/a.mp3"));
        assert_eq!(entries[0].title.as_deref(), Some("spaced"));
        assert_eq!(entries[0].duration_ms, None);
    }

    #[test]
    fn tracks_without_a_location_are_dropped() {
        let text = "<playlist><trackList><track><title>no location</title></track>\
                    <track><location/></track></trackList></playlist>";
        assert!(read_text(text, &base()).is_empty());
    }

    #[test]
    fn names_are_matched_case_insensitively_and_by_local_name() {
        let text = r#"<x:PLAYLIST xmlns:x="urn:x"><X:TrackList><X:TRACK>
          <X:Location>a.mp3</X:Location><X:Title>A</X:Title></X:TRACK>
        </X:TrackList></x:PLAYLIST>"#;
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title.as_deref(), Some("A"));
    }

    #[test]
    fn truncated_xml_yields_what_was_read() {
        let text = "<playlist><trackList><track><location>a.mp3</location><title>A</title>";
        let entries = read_text(text, &base());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].location, file("C:/Music/Lists/a.mp3"));

        // Unclosed tags, a stray root and plain garbage must all be safe.
        for bad in [
            "<playlist",
            "<playlist><track><location>a.mp3",
            "<playlist><track><location>a.mp3</location>",
            "<",
            "",
            "not xml at all",
            "<playlist><track><location>a.mp3</location></track>",
        ] {
            let _ = read_text(bad, &base());
        }
    }

    #[test]
    fn write_then_read_round_trips() {
        let entries = vec![
            LoadedEntry::new(file("C:/Music/a b&c<d>#e.mp3"))
                .with_title("A & B <C> \"D\"")
                .with_duration_ms(123_456),
            LoadedEntry::new(MediaLocation::Url("http://example.com/a?x=1&y=2".into()))
                .with_title("Radio"),
            LoadedEntry::new(file("C:/Music/plain.mp3")),
        ];
        let bytes = write(&entries);
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n"));
        assert!(text.ends_with("</playlist>\n"));
        let back = read(&bytes, &base());
        assert_eq!(back, entries);
    }

    #[test]
    fn write_escapes_control_characters_in_text() {
        let entries = vec![LoadedEntry::new(file("C:/a.mp3")).with_title("a\nb\rc")];
        let text = String::from_utf8(write(&entries)).unwrap();
        assert!(text.contains("<title>a&#10;b&#13;c</title>"));
        let back = read(text.as_bytes(), &base());
        assert_eq!(back[0].title.as_deref(), Some("a\nb\rc"));
    }

    #[test]
    fn empty_input_yields_no_entries() {
        assert!(read(b"", &base()).is_empty());
        assert!(read(b"<playlist/>", &base()).is_empty());
    }
}
