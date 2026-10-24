// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The HTML playlist (S03 §2, command id `40292`).
//!
//! `Ctrl+Alt+G` in the playlist editor writes the current queue as one HTML
//! document, and the MISC menu's "Generate HTML playlist" row — which carries
//! the same id — does the same thing. The document is a table of the entries
//! with their lengths and their locations, under a title, next to the moment
//! it was written and the number of entries. It names no external file and
//! runs no script, so it opens and reads anywhere.
//!
//! The module has two halves:
//!
//! * [`generate_html_playlist`] is **pure**: it turns the entries and the
//!   date into the document's text. Everything that can go wrong inside a
//!   document — a title with a `<` in it, an entry without a length — is
//!   decided here and is unit-tested without a desktop.
//! * the [`App`] half collects the entries from the queue and writes the
//!   document to the file the user chose. The save dialog is the platform's
//!   (`rustamp_platform::save_html_playlist_file`); the app only learns the
//!   path, so the whole action stays testable headlessly.
//!
//! **Own choices.** S03 §2 fixes the id and the chord and nothing else about
//! the document: its title, its columns, the wording of the summary line and
//! the date format are ours.

use std::path::Path;

use rustamp_audio::TrackSource;
use rustamp_core::error::Result;
use rustamp_core::playlist::MediaLocation;

use crate::app::App;

/// One row of the HTML playlist.
///
/// The three fields are what the player knows about an entry: a title (the
/// name the last playlist load carried, else the file name), a length (when
/// the load carried one) and the location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlPlaylistEntry {
    /// The title of the entry.
    pub title: String,
    /// The length in milliseconds, when it is known.
    pub duration_ms: Option<u64>,
    /// The location: the full path or the URL.
    pub path: String,
}

/// The cell an entry without a known length shows.
const UNKNOWN_LENGTH: &str = "--:--";

/// The document's own style: a plain table, no external file, no script.
const STYLE: &str = "<style>\n\
body { font-family: \"Segoe UI\", Arial, sans-serif; margin: 2em; color: #1c1c1c; background: #ffffff; }\n\
h1 { font-size: 1.4em; margin: 0 0 0.3em 0; }\n\
p.summary { color: #505050; margin: 0 0 1.2em 0; }\n\
table { border-collapse: collapse; }\n\
th, td { border: 1px solid #b4b4b4; padding: 0.2em 0.7em; text-align: left; vertical-align: top; }\n\
th { background: #ececec; }\n\
td.index, td.length { text-align: right; white-space: nowrap; }\n\
</style>\n";

/// Writes the HTML document for `entries`.
///
/// The document is self-contained: a title, the summary line (when it was
/// written, how many entries it holds, how long they are) and one table row
/// per entry with its number, its title, its length and its location.
///
/// `generated_at` is the moment the document names, as text. The generator
/// takes it as an argument so it stays pure: the clock belongs to the
/// caller.
///
/// Every text that comes from the playlist is escaped
/// ([`escape_html`]), so a title with a `<` in it stays a title.
pub fn generate_html_playlist(entries: &[HtmlPlaylistEntry], generated_at: &str) -> String {
    let mut document = String::new();
    document.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    document.push_str("<meta charset=\"utf-8\">\n");
    document.push_str("<title>Rustamp playlist</title>\n");
    document.push_str(STYLE);
    document.push_str("</head>\n<body>\n");
    document.push_str("<h1>Rustamp playlist</h1>\n");
    document.push_str("<p class=\"summary\">");
    document.push_str(&escape_html(&summary(entries, generated_at)));
    document.push_str("</p>\n");
    document.push_str(
        "<table>\n<thead>\n<tr><th>#</th><th>Title</th><th>Length</th><th>File</th></tr>\n\
         </thead>\n<tbody>\n",
    );
    for (index, entry) in entries.iter().enumerate() {
        document.push_str("<tr><td class=\"index\">");
        document.push_str(&(index + 1).to_string());
        document.push_str("</td><td>");
        document.push_str(&escape_html(&entry.title));
        document.push_str("</td><td class=\"length\">");
        match entry.duration_ms {
            Some(duration) => document.push_str(&format_duration(duration)),
            None => document.push_str(UNKNOWN_LENGTH),
        }
        document.push_str("</td><td>");
        document.push_str(&escape_html(&entry.path));
        document.push_str("</td></tr>\n");
    }
    document.push_str("</tbody>\n</table>\n</body>\n</html>\n");
    document
}

/// The summary line: when the document was written, how many entries it holds
/// and how long they are.
///
/// The total is the sum of the lengths the playlist knows. When only some of
/// the entries carry one, the line says how many do, so the total is not
/// read as the length of the whole list.
fn summary(entries: &[HtmlPlaylistEntry], generated_at: &str) -> String {
    let count = entries.len();
    let known: Vec<u64> = entries
        .iter()
        .filter_map(|entry| entry.duration_ms)
        .collect();
    let length = if known.is_empty() {
        "Total length unknown.".to_string()
    } else {
        let total: u64 = known.iter().sum();
        let mut sentence = format!("Total length {}.", format_duration(total));
        if known.len() < count {
            sentence.push_str(&format!(
                " The length is known for {} of {count} entries.",
                known.len()
            ));
        }
        sentence
    };
    format!(
        "Generated on {generated_at}. {count} {}. {length}",
        if count == 1 { "entry" } else { "entries" }
    )
}

/// Escapes the five characters that change the meaning of HTML text.
///
/// `&` comes first, so the ampersands this function writes are not escaped
/// twice. `<` and `>` keep a title from becoming a tag, `"` and `'` keep it
/// from breaking out of an attribute value.
fn escape_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

/// A length in milliseconds as `mm:ss`, or as `h:mm:ss` from an hour on.
fn format_duration(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let rest = seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{rest:02}")
    } else {
        format!("{minutes:02}:{rest:02}")
    }
}

/// The UTC calendar date and time of `epoch_seconds` as
/// `YYYY-MM-DD HH:MM:SS UTC`.
///
/// Own choice: the document names the moment it was written, and the
/// generator takes that text as an argument, so the clock stays with the
/// caller. The conversion is the platform's calendar (see
/// [`rustamp_platform::utc_calendar`]): no lookup table, every date works.
fn utc_timestamp(epoch_seconds: u64) -> String {
    let at = rustamp_platform::utc_calendar(epoch_seconds as i64);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        at.year, at.month, at.day, at.hour, at.minute, at.second
    )
}

/// The seconds since the epoch, right now.
fn now_epoch_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

impl App {
    /// The entries the HTML playlist lists, in queue order.
    ///
    /// The title is the name the last playlist load carried, else the file
    /// name (the same rule the title sort uses); the length is the one the
    /// load carried, if any. Nothing is probed: an entry the player knows
    /// nothing about is listed with its location and without a length.
    fn html_playlist_entries(&self) -> Vec<HtmlPlaylistEntry> {
        let titles = self.loaded_titles();
        let durations = self.loaded_durations();
        self.engine()
            .queue()
            .iter()
            .map(|source| {
                let location = match source {
                    TrackSource::Path(path) => MediaLocation::File(path.clone()),
                    TrackSource::Url(url) => MediaLocation::Url(url.clone()),
                };
                HtmlPlaylistEntry {
                    title: titles
                        .get(&location)
                        .cloned()
                        .unwrap_or_else(|| location.file_name()),
                    duration_ms: durations.get(&location).copied(),
                    path: location.to_display_string(),
                }
            })
            .collect()
    }

    /// Finishes an HTML playlist request with the save dialog's answer.
    ///
    /// `chosen` is where the document goes. `None` means the user closed the
    /// dialog without choosing a file: nothing is written, and the
    /// cancellation is noted, so the key press is not silently swallowed.
    ///
    /// A failed write is reported like a failed playlist save: a note, the
    /// error the window title shows, the published error, and `Err`.
    pub fn finish_html_playlist(&mut self, chosen: Option<&Path>) -> Result<()> {
        let Some(path) = chosen else {
            self.note(
                "the HTML playlist was not written: the save dialog was cancelled".to_string(),
            );
            return Ok(());
        };
        let result = self.write_html_playlist(path);
        if let Err(error) = &result {
            self.record_failure(
                format!(
                    "failed to write the HTML playlist {}: {}",
                    path.display(),
                    error
                ),
                error,
            );
        }
        result
    }

    /// Writes the HTML playlist of the current queue to `path`.
    fn write_html_playlist(&self, path: &Path) -> Result<()> {
        let entries = self.html_playlist_entries();
        let generated_at = utc_timestamp(now_epoch_seconds());
        let document = generate_html_playlist(&entries, &generated_at);
        std::fs::write(path, document)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rustamp_core::config::Config;

    use super::*;
    use crate::app::AppOptions;
    use crate::cli::OutputChoice;

    /// An entry with a title, a length and a location.
    fn entry(title: &str, duration_ms: Option<u64>, path: &str) -> HtmlPlaylistEntry {
        HtmlPlaylistEntry {
            title: title.to_string(),
            duration_ms,
            path: path.to_string(),
        }
    }

    /// The headless app the action is exercised on.
    fn test_app() -> App {
        let options = AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            media: None,
            seconds: None,
        };
        App::new(options).expect("the test app builds headless")
    }

    /// Grows the engine queue (no decoding) so the app assigns entry ids.
    fn add_entries(app: &mut App, names: &[&str]) {
        let items = names
            .iter()
            .map(|name| MediaLocation::File(std::path::PathBuf::from(name)))
            .collect::<Vec<_>>();
        app.submit(rustamp_core::command::Command::AddItems {
            items,
            at: None,
            play: false,
        })
        .expect("AddItems applies");
    }

    /// The markup characters must not reach the document as markup.
    #[test]
    fn the_generator_escapes_the_markup_characters() {
        let entries = [entry(
            "a <b>title</b> & \"more\"",
            None,
            "C:\\Music\\<dir>\\a & b.mp3",
        )];
        let document = generate_html_playlist(&entries, "2026-10-20 12:00:00 UTC");
        assert!(document.contains("a &lt;b&gt;title&lt;/b&gt; &amp; &quot;more&quot;"));
        assert!(document.contains("C:\\Music\\&lt;dir&gt;\\a &amp; b.mp3"));
        assert!(
            !document.contains("<b>title</b>"),
            "the title must not become markup: {document}"
        );
        assert!(
            !document.contains("&amp;amp;"),
            "the ampersands must not be escaped twice: {document}"
        );
    }

    /// Every entry gets a row, in order, with its own number.
    #[test]
    fn the_generator_lists_every_entry_in_order() {
        let entries = [
            entry("First", Some(200_000), "C:\\a.mp3"),
            entry("Second", Some(65_000), "C:\\b.mp3"),
            entry("Third", None, "C:\\c.mp3"),
        ];
        let document = generate_html_playlist(&entries, "2026-10-20 12:00:00 UTC");
        assert_eq!(document.matches("<tr><td class=\"index\">").count(), 3);
        let first = document.find("First").expect("the first title is listed");
        let second = document.find("Second").expect("the second title is listed");
        let third = document.find("Third").expect("the third title is listed");
        assert!(first < second && second < third, "the rows keep the order");
        assert!(document.contains("3 entries"), "{document}");
    }

    /// The lengths: the known ones as `mm:ss`, the unknown one as the
    /// placeholder, and the total as the sum of the known ones.
    #[test]
    fn the_generator_shows_the_lengths_and_their_total() {
        let entries = [
            entry("First", Some(200_000), "C:\\a.mp3"),
            entry("Second", Some(65_000), "C:\\b.mp3"),
            entry("Third", None, "C:\\c.mp3"),
        ];
        let document = generate_html_playlist(&entries, "2026-10-20 12:00:00 UTC");
        assert!(document.contains("03:20"), "{document}");
        assert!(document.contains("01:05"), "{document}");
        assert!(document.contains(UNKNOWN_LENGTH), "{document}");
        // 200 s + 65 s = 265 s = 04:25.
        assert!(document.contains("Total length 04:25."), "{document}");
        assert!(
            document.contains("The length is known for 2 of 3 entries."),
            "{document}"
        );
    }

    /// An hour and more is written with its hours.
    #[test]
    fn a_length_of_an_hour_keeps_its_hours() {
        let entries = [entry("Long", Some(3_725_000), "C:\\a.mp3")];
        let document = generate_html_playlist(&entries, "2026-10-20 12:00:00 UTC");
        assert!(document.contains("1:02:05"), "{document}");
    }

    /// An empty playlist is a document, not a failure.
    #[test]
    fn an_empty_playlist_is_a_valid_document() {
        let document = generate_html_playlist(&[], "2026-10-20 12:00:00 UTC");
        assert!(document.starts_with("<!DOCTYPE html>"), "{document}");
        assert!(document.contains("<html lang=\"en\">"), "{document}");
        assert!(document.ends_with("</html>\n"), "{document}");
        assert!(document.contains("0 entries"), "{document}");
        assert!(document.contains("Total length unknown."), "{document}");
        assert_eq!(document.matches("<tr><td class=\"index\">").count(), 0);
    }

    /// The document names no external file and runs no script.
    #[test]
    fn the_document_is_self_contained() {
        let entries = [entry("First", Some(1_000), "C:\\a.mp3")];
        let document = generate_html_playlist(&entries, "2026-10-20 12:00:00 UTC");
        assert!(!document.contains("<script"), "{document}");
        assert!(!document.contains("http://"), "{document}");
        assert!(!document.contains("https://"), "{document}");
    }

    /// The date the document names: the epoch and a plain day.
    #[test]
    fn the_timestamp_is_the_utc_calendar_date() {
        assert_eq!(utc_timestamp(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc_timestamp(1_700_000_000), "2023-11-14 22:13:20 UTC");
        assert_eq!(utc_timestamp(1_792_454_400), "2026-10-20 00:00:00 UTC");
    }

    /// The key press asks for the document; the file is written where the
    /// dialog said.
    #[test]
    fn the_shortcut_writes_the_document_where_the_dialog_said() {
        let mut app = test_app();
        add_entries(&mut app, &["C:\\Music\\one.mp3", "C:\\Music\\two.mp3"]);
        app.playlist_key_down(
            rustamp_ui::Key::Letter(b'G'),
            rustamp_ui::Modifiers::CTRL.union(rustamp_ui::Modifiers::ALT),
        )
        .expect("the key applies");
        assert!(app.request_html_playlist, "the document is requested");

        let directory = tempfile::tempdir().expect("a temporary directory");
        let path = directory.path().join("playlist.html");
        app.finish_html_playlist(Some(&path))
            .expect("the document is written");

        let document = std::fs::read_to_string(&path).expect("the document is there");
        assert!(document.contains("one.mp3"), "{document}");
        assert!(document.contains("two.mp3"), "{document}");
        assert!(document.contains("2 entries"), "{document}");
        assert!(app.drain_notes().is_empty(), "writing is not a failure");
    }

    /// A cancelled dialog writes nothing and says so.
    #[test]
    fn a_cancelled_dialog_writes_nothing_and_is_noted() {
        let mut app = test_app();
        add_entries(&mut app, &["C:\\Music\\one.mp3"]);
        app.finish_html_playlist(None)
            .expect("a cancellation is not a failure");
        let notes = app.drain_notes();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("cancelled"), "{notes:?}");
    }

    /// A file that cannot be written is reported like a failed playlist save.
    #[test]
    fn a_failed_write_is_reported() {
        let mut app = test_app();
        add_entries(&mut app, &["C:\\Music\\one.mp3"]);
        let missing = std::path::Path::new("Z:\\no-such-directory\\playlist.html");
        assert!(app.finish_html_playlist(Some(missing)).is_err());
        let notes = app.drain_notes();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].contains("failed to write the HTML playlist"),
            "{notes:?}"
        );
        assert!(app.last_error().is_some());
    }
}
