// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The playlist's title format (spec **S14**) as a setting of the app.
//!
//! The language itself is the engine of [`rustamp_ui::title_format`]: it
//! renders a format string with the fields a row carries. This module is the
//! app's part of it: the setting the user types, the check that the format
//! string can be used, and the fields a playlist row offers.
//!
//! # Why the setting belongs to the app
//!
//! The format string is a view setting of the playlist editor, exactly like
//! the editor's font size (see [`crate::config::PLAYLIST_FONT_SIZE_KEY`]). It
//! is **not** part of `rustamp-core`'s configuration tree: adding a field
//! there is a contract change that needs its own RFC. The app therefore
//! keeps the value in the same file, under the `[ui]` table, next to the
//! other UI state it persists.
//!
//! # The format is off until the user turns it on
//!
//! Without a setting the rows show their plain text, which is what the
//! playlist shows today. The switch belongs to the setting, so a user who
//! never touches the preferences page never sees a different row.
//!
//! # What the app checks, and what the engine does
//!
//! The engine never fails: a format string it cannot read is literal text
//! (S14's own choice). The app is stricter, because a format string is
//! something the user typed, and a typo in it should be **reported** rather
//! than shown as literal text in every row of the playlist. The check is
//! therefore a check of the *shape* of the format string:
//!
//! * a `%` that never finds its partner is a field that never closes;
//! * a `[` that never finds its `]` is a bracket that never closes — the
//!   engine would read the rest of the format as the group;
//! * a `$name(` that never finds its `)` is a call that never closes;
//! * a format string that nests deeper than the engine's limit is not read
//!   as a format at all.
//!
//! A `]` without a `[` and a `)` without a `(` are text, exactly as the
//! engine reads them.
//!
//! A format string that fails the check is **not** used: the rows keep their
//! plain text, the reason is reported in the player's notes (the app's
//! report channel), and the text the user typed is kept, so it can be fixed
//! in the preferences page.
//!
//! # The fields a row offers
//!
//! The fields come from what the row has, and nothing is read from the disk
//! while a frame is painted: the file name comes from the row's path, and
//! the tags come from the metadata the app has for the row (the tags the
//! engine reported when the track opened, and the tags the app's tag reader
//! read off the UI thread — see [`crate::playlist_tags`]).

use rustamp_audio::TrackSource;
use rustamp_core::meta::TrackMetadata;
use rustamp_ui::title_format::{DEFAULT_TEMPLATE, MAX_NESTING_DEPTH, TitleFields, TitleFormat};

/// The key the playlist's title format is stored under.
///
/// The key lives in the `[ui]` table of the configuration file, next to the
/// other UI state the app persists (see the module docs).
pub const PLAYLIST_TITLE_FORMAT_KEY: &str = "playlist_title_format";

/// The playlist's title format: the switch and the format string.
///
/// The default is the format **off** with the engine's own default template
/// ([`DEFAULT_TEMPLATE`]) in the field: the rows show their plain text until
/// the user turns the format on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleFormatSetting {
    /// Whether the format is used for the playlist's rows.
    pub enabled: bool,
    /// The format string.
    pub format: String,
}

impl Default for TitleFormatSetting {
    /// Off, with the default template in the field.
    fn default() -> Self {
        Self {
            enabled: false,
            format: DEFAULT_TEMPLATE.to_string(),
        }
    }
}

impl TitleFormatSetting {
    /// The setting with the format string it was given.
    ///
    /// An empty format string is the engine's default template: there is no
    /// format that says nothing, and a user who clears the field means the
    /// default one.
    pub fn new(enabled: bool, format: impl Into<String>) -> Self {
        let format = format.into();
        Self {
            enabled,
            format: if format.is_empty() {
                DEFAULT_TEMPLATE.to_string()
            } else {
                format
            },
        }
    }

    /// The reason the format string cannot be used, if it cannot be used.
    ///
    /// The format string is checked whether the format is on or not: a
    /// format the user typed is a format the user must be able to fix, and
    /// the report says what is wrong with it.
    pub fn check(&self) -> Result<(), String> {
        validate_format(&self.format)
    }

    /// Whether the format string can be used.
    pub fn is_usable(&self) -> bool {
        self.check().is_ok()
    }

    /// The format the playlist's rows use.
    ///
    /// Off while the switch is off, and off as well when the format string
    /// cannot be used: a format that fails the check leaves the plain text
    /// alone.
    pub fn engine_format(&self) -> TitleFormat {
        if !self.enabled {
            return TitleFormat::off();
        }
        match self.check() {
            Ok(()) => TitleFormat::new(self.format.clone()),
            Err(_) => TitleFormat::off(),
        }
    }
}

/// Checks the format string the user typed.
///
/// The engine reads a malformed format string as literal text, so this check
/// is the app's own: it names the shape a format string must have, and the
/// reason it does not have it.
///
/// A backslash escapes the next character (the engine's own rule), so an
/// escaped `%` or `[` is text and not the start of a field or a group.
pub fn validate_format(format: &str) -> Result<(), String> {
    let mut chars = format.chars().peekable();
    // The groups (`[ … ]`) that are still open.
    let mut groups = 0usize;
    // The deepest the groups nested.
    let mut deepest = 0usize;
    // A `%` that waits for the `%` that closes it.
    let mut field_open = false;
    // The parenthesis depth of the calls that are still open.
    let mut calls: Vec<usize> = Vec::new();
    // The parenthesis depth the text is at.
    let mut parens = 0usize;

    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                // The escaped character is text: it does not open anything.
                chars.next();
            }
            '%' => field_open = !field_open,
            '[' => {
                groups += 1;
                deepest = deepest.max(groups);
            }
            ']' => {
                // A closing bracket without an opening one is text: the
                // engine reads it that way, and so does the check.
                groups = groups.saturating_sub(1);
            }
            '$' => {
                // A call is a name and an opening parenthesis; a `$` that is
                // not followed by a name is text.
                let mut name = String::new();
                while let Some(&next) = chars.peek() {
                    if next.is_ascii_alphanumeric() || next == '_' {
                        name.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if name.is_empty() {
                    continue;
                }
                if chars.peek() == Some(&'(') {
                    chars.next();
                    calls.push(parens);
                    parens += 1;
                }
            }
            '(' => parens += 1,
            ')' => {
                if parens == 0 {
                    // A parenthesis that closes nothing is text.
                    continue;
                }
                parens -= 1;
                if calls.last() == Some(&parens) {
                    calls.pop();
                }
            }
            _ => {}
        }
    }

    if deepest > MAX_NESTING_DEPTH {
        return Err(format!(
            "the format string nests deeper than the {MAX_NESTING_DEPTH} levels the engine reads"
        ));
    }
    if field_open {
        return Err("the format string has a field that is never closed".to_string());
    }
    if groups > 0 {
        return Err("the format string has a bracket that is never closed".to_string());
    }
    if !calls.is_empty() {
        return Err("the format string has a function call that is never closed".to_string());
    }
    Ok(())
}

/// The fields a playlist row offers to the format language.
///
/// The row's own data: the file name comes from the path, the tags from the
/// metadata the app has for the row. A field the row does not carry yields
/// the empty string (S14 §2.1), and the conditional group of S14 §2.3 is how
/// a format hides that gap.
pub fn row_fields(source: &TrackSource, metadata: Option<&TrackMetadata>) -> TitleFields {
    let mut fields = TitleFields::new();
    match source {
        TrackSource::Path(path) => {
            if let Some(name) = path.file_name() {
                fields.set("filename", &name.to_string_lossy());
            }
        }
        TrackSource::Url(url) => fields.set("filename", url),
    }
    if let Some(metadata) = metadata {
        if let Some(title) = &metadata.title {
            fields.set("title", title);
        }
        if let Some(artist) = &metadata.artist {
            fields.set("artist", artist);
        }
        if let Some(album) = &metadata.album {
            fields.set("album", album);
        }
        if let Some(track) = metadata.track_no {
            fields.set("tracknumber", &track.to_string());
        }
        if let Some(comment) = &metadata.comment {
            fields.set("comment", comment);
        }
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row with the fields the spec's own strings use.
    fn fields() -> TitleFields {
        TitleFields::new()
            .with("title", "The Title")
            .with("artist", "The Artist")
            .with("album", "The Album")
            .with("tracknumber", "3")
    }

    #[test]
    fn the_default_setting_is_off_with_the_default_template() {
        let setting = TitleFormatSetting::default();
        assert!(!setting.enabled);
        assert_eq!(setting.format, DEFAULT_TEMPLATE);
        assert!(!setting.engine_format().is_enabled());
    }

    #[test]
    fn an_empty_format_string_is_the_default_template() {
        let setting = TitleFormatSetting::new(true, "");
        assert_eq!(setting.format, DEFAULT_TEMPLATE);
        assert!(setting.is_usable());
        assert!(setting.engine_format().is_enabled());
    }

    #[test]
    fn a_format_string_that_cannot_be_used_is_reported() {
        for (format, reason) in [
            ("%title", "field"),
            ("%artist% - %title", "field"),
            ("[%artist%", "bracket"),
            ("$lower(%title%", "call"),
        ] {
            let error = validate_format(format).expect_err("the format is refused");
            assert!(
                error.contains(reason),
                "the reason for {format:?} names the {reason}: {error}"
            );
            assert!(!TitleFormatSetting::new(true, format).is_usable());
        }
    }

    #[test]
    fn a_format_string_that_can_be_used_is_accepted() {
        for format in [
            "%title%",
            "[%artist% - ]%title%",
            "$lower(%title%)",
            "$if2(%title%,%filename%)[ (%part%)]",
            r"100\% - %title%",
            r"\[%title%]",
            "a ] b",
            "Song (Live)",
        ] {
            assert!(
                validate_format(format).is_ok(),
                "{format:?} is a format the engine can read"
            );
        }
    }

    #[test]
    fn a_format_string_that_nests_too_deeply_is_refused() {
        let deep = "[".repeat(MAX_NESTING_DEPTH + 1) + "%title%";
        let error = validate_format(&deep).expect_err("the format is refused");
        assert!(error.contains("nests deeper"), "{error}");
    }

    #[test]
    fn an_unusable_format_leaves_the_plain_text_alone() {
        let setting = TitleFormatSetting::new(true, "%title");
        assert!(!setting.is_usable());
        assert!(!setting.engine_format().is_enabled());
        assert_eq!(
            setting
                .engine_format()
                .row_text("the plain text", &fields()),
            "the plain text"
        );
    }

    #[test]
    fn a_usable_format_replaces_the_plain_text() {
        let setting = TitleFormatSetting::new(true, "[%artist% - ]%title%");
        assert!(setting.is_usable());
        assert_eq!(
            setting
                .engine_format()
                .row_text("the plain text", &fields()),
            "The Artist - The Title"
        );
    }

    #[test]
    fn the_fields_of_a_row_carry_its_name_and_its_tags() {
        let metadata = TrackMetadata {
            title: Some("The Title".to_string()),
            artist: Some("The Artist".to_string()),
            album: Some("The Album".to_string()),
            track_no: Some(3),
            comment: Some("A comment".to_string()),
            ..TrackMetadata::default()
        };
        let source = TrackSource::path(std::path::Path::new("Music").join("the-title.mp3"));
        let fields = row_fields(&source, Some(&metadata));
        assert_eq!(fields.get("filename"), Some("the-title.mp3"));
        assert_eq!(fields.get("title"), Some("The Title"));
        assert_eq!(fields.get("artist"), Some("The Artist"));
        assert_eq!(fields.get("album"), Some("The Album"));
        assert_eq!(fields.get("tracknumber"), Some("3"));
        assert_eq!(fields.get("comment"), Some("A comment"));
    }

    #[test]
    fn a_row_without_tags_carries_only_its_name() {
        let source = TrackSource::path(std::path::Path::new("Music").join("the-title.mp3"));
        let fields = row_fields(&source, None);
        assert_eq!(fields.get("filename"), Some("the-title.mp3"));
        assert_eq!(fields.get("title"), None);
    }
}
