// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The path template of the playlist extraction.
//!
//! The media library of the reference can write its selection out as a
//! playlist (spec S20 §4). The file names come from a **path template** in a
//! language of its own: bare words, no `%name%` and no functions (S20 §4.2).
//! This module is that language and nothing else - it parses a template,
//! renders a row through it and plans the extraction of a selection. It
//! touches no file, opens no dialog and asks the user nothing: the writing
//! and the question to the user are the caller's.
//!
//! ## The language
//!
//! A template is a list of **segments** separated by a backslash. The last
//! segment names the file, the ones before it name the directories. A
//! segment is literal text and **tokens**:
//!
//! | Token | Inserts |
//! |---|---|
//! | `Artist` | the artist, as the track carries it |
//! | `ARTIST` | the artist in upper case |
//! | `artist` | the artist in lower case |
//! | `Album` / `ALBUM` / `album` | the album, the same three cases |
//! | `Title` / `TITLE` / `title` | the title, the same three cases |
//! | `Genre` / `GENRE` / `genre` | the genre, the same three cases |
//! | `year` | the album year |
//! | `#`, `##`, `###` | the track number, zero-padded to the number of `#` |
//!
//! The two template strings the reference carries render like this:
//!
//! ```text
//! Artist - Album\Artist - Album   ->  "ABBA - Gold/ABBA - Gold"
//! ## - Artist - Title              ->  "07 - ABBA - Dancing Queen"
//! ```
//!
//! The rules of the parser:
//!
//! * A **word** is a run of ASCII letters and digits. A word that is one of
//!   the spellings of the table above is a token; **every other word is
//!   literal text** - `Hello` in a template stays `Hello`.
//! * `year` is carried in **one** spelling only (S20 §4.2), so `Year` and
//!   `YEAR` are literal text like any other word.
//! * A run of `#` is the track number, padded to the length of the run. The
//!   table carries `#`, `##` and `###`; a longer run is not a token of the
//!   language and stays literal text.
//! * The backslash separates the segments. Whether the backslash is literal
//!   on every platform or a separator that follows the platform is open
//!   (S20 §8, open question 8), so this module never joins the segments
//!   itself: it hands the caller the segments, and
//!   [`RenderedPath::relative_path`] is the one place that joins them with
//!   the separator of the platform.
//!
//! ## The capitalisation
//!
//! The spelling of the token chooses the capitalisation of the value: the
//! mixed case keeps the value as the track carries it, the upper case makes
//! it upper, the lower case makes it lower. The material carries the three
//! spellings side by side, and what the plug-in does with them is a reading
//! (S20 §4.2, S20 §8 open question 12). This module implements the
//! reading and marks it as one.
//!
//! ## A value the track does not carry
//!
//! A field the track has no value for renders as an **empty text**, and the
//! renderer **reports** it: [`RenderedPath::missing`] names every field the
//! template asked for and the row could not answer. The caller decides what
//! to do with such a track - skip it, or write the file with the hole in
//! the name. The plan of the extraction keeps the file and carries the
//! missing fields with it, so the decision stays with the caller.
//!
//! ## The plan of an extraction
//!
//! [`PathTemplate::plan`] turns a selection into the list of files the
//! extraction writes and the list of the tracks it cannot write:
//!
//! * A track whose template renders **no file name** is an error: there is
//!   nothing to write.
//! * A track that renders a path **another track of the same extraction
//!   already renders** is an error too. The first track of the selection
//!   keeps the path, the second one is reported - two tracks must not
//!   silently write over each other.
//! * Every other track is a planned file, with the fields the track did not
//!   carry named in [`PlannedFile::path`].
//!
//! The counts the report needs are in [`ExtractPlan::counts`]: the number
//! of tracks the plan writes is the `N` of `N tracks extracted`, and every
//! error carries the message `Error extracting track #N: <reason>`.
//!
//! ## Writing over an existing playlist
//!
//! The reference asks `Overwrite playlist (No will append)?` before it
//! writes over an existing playlist, and the prompt's own text says that
//! answering "no" appends (S20 §4). [`choose_playlist_action`] is the
//! decision: it is given whether the target exists and the answer of the
//! user, and it says what the extraction does with the target. The question
//! itself is the caller's.
//!
//! ## Known limits
//!
//! * The template carries no token for the **file extension** - the token
//!   table of S20 §4.2 covers five of the seven field names of §3 - so the
//!   planned file name is exactly what the template renders. A caller that
//!   wants a playable file appends the extension of the source file.
//! * The duplicate check compares the paths **exactly**. Two tracks whose
//!   paths differ only in the case collide on a case-insensitive file
//!   system; the caller, which does the writing, sees the error of the file
//!   system there.
//! * The number of the failed track in `Error extracting track #N` is the
//!   position of the track in the selection, counted from one. The material
//!   does not say what the number counts (S20 §4 carries the message only),
//!   so the choice is ours and marked as one.
//! * Whether the token list of S20 §4.2 is complete is open (S20 §8, open
//!   question 17), and what `track` and `ntracks` do in a template is open
//!   with it. This module implements the thirteen spellings of the table
//!   and nothing else: `track` and `ntracks` are literal words here.

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;

use rustamp_core::meta::TrackMetadata;

use crate::TrackRow;

/// The prompt the reference shows before it writes over an existing
/// playlist.
///
/// The prompt's own text says that answering "no" appends to the existing
/// file (S20 §4).
pub const OVERWRITE_PROMPT: &str = "Overwrite playlist (No will append)?";

/// The capitalisation a token chooses for the value it inserts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldCase {
    /// The value as the track carries it: the token spells the field in
    /// mixed case (`Artist`).
    Default,
    /// The value in upper case: the token spells the field in upper case
    /// (`ARTIST`).
    Upper,
    /// The value in lower case: the token spells the field in lower case
    /// (`artist`).
    Lower,
}

/// The value a template token inserts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateField {
    /// The artist.
    Artist,
    /// The album.
    Album,
    /// The title.
    Title,
    /// The genre.
    Genre,
    /// The album year.
    Year,
    /// The track number.
    TrackNumber,
}

/// One token of the template language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateToken {
    /// The artist, in the capitalisation the token chose.
    Artist(FieldCase),
    /// The album, in the capitalisation the token chose.
    Album(FieldCase),
    /// The title, in the capitalisation the token chose.
    Title(FieldCase),
    /// The genre, in the capitalisation the token chose.
    Genre(FieldCase),
    /// The album year.
    ///
    /// The language carries `year` in one spelling only, so the token
    /// chooses no capitalisation.
    Year,
    /// The track number, zero-padded to the number of `#` in the token.
    TrackNumber {
        /// The number of digits the number is padded to.
        width: u8,
    },
}

impl TemplateToken {
    /// The value the token inserts.
    pub fn field(self) -> TemplateField {
        match self {
            TemplateToken::Artist(_) => TemplateField::Artist,
            TemplateToken::Album(_) => TemplateField::Album,
            TemplateToken::Title(_) => TemplateField::Title,
            TemplateToken::Genre(_) => TemplateField::Genre,
            TemplateToken::Year => TemplateField::Year,
            TemplateToken::TrackNumber { .. } => TemplateField::TrackNumber,
        }
    }
}

/// One part of a segment: literal text or a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplatePart {
    /// Text the template carries verbatim.
    Literal(String),
    /// A field the template inserts.
    Token(TemplateToken),
}

/// One segment of a template: the text between two backslashes.
///
/// The last segment of a template names the file; the ones before it name
/// the directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateSegment {
    parts: Vec<TemplatePart>,
}

impl TemplateSegment {
    /// The parts of the segment, in the order they stand in it.
    pub fn parts(&self) -> &[TemplatePart] {
        &self.parts
    }

    /// Parses one segment of a template.
    fn parse(text: &str) -> TemplateSegment {
        let mut parts: Vec<TemplatePart> = Vec::new();
        let mut literal = String::new();
        let mut chars = text.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch.is_ascii_alphanumeric() {
                // A word: a run of letters and digits. It is a token only
                // when it is one of the carried spellings.
                let mut word = String::new();
                word.push(ch);
                while let Some(next) = chars.peek().copied() {
                    if !next.is_ascii_alphanumeric() {
                        break;
                    }
                    word.push(next);
                    chars.next();
                }
                match word_token(&word) {
                    Some(token) => {
                        flush(&mut literal, &mut parts);
                        parts.push(TemplatePart::Token(token));
                    }
                    None => literal.push_str(&word),
                }
            } else if ch == '#' {
                // A run of `#`: the track number, padded to the length of
                // the run. The language carries `#`, `##` and `###`; a
                // longer run is not a token of it and stays literal.
                let mut run = String::new();
                run.push(ch);
                while let Some(next) = chars.peek().copied() {
                    if next != '#' {
                        break;
                    }
                    run.push(next);
                    chars.next();
                }
                match run.len() {
                    1..=3 => {
                        flush(&mut literal, &mut parts);
                        parts.push(TemplatePart::Token(TemplateToken::TrackNumber {
                            width: run.len() as u8,
                        }));
                    }
                    _ => literal.push_str(&run),
                }
            } else {
                literal.push(ch);
            }
        }

        flush(&mut literal, &mut parts);
        TemplateSegment { parts }
    }
}

/// Puts the literal text that has been collected so far into the parts and
/// clears the text.
fn flush(literal: &mut String, parts: &mut Vec<TemplatePart>) {
    if literal.is_empty() {
        return;
    }
    parts.push(TemplatePart::Literal(std::mem::take(literal)));
}

/// The token a word stands for, if the word is one of the carried
/// spellings.
fn word_token(word: &str) -> Option<TemplateToken> {
    Some(match word {
        "Artist" => TemplateToken::Artist(FieldCase::Default),
        "ARTIST" => TemplateToken::Artist(FieldCase::Upper),
        "artist" => TemplateToken::Artist(FieldCase::Lower),
        "Album" => TemplateToken::Album(FieldCase::Default),
        "ALBUM" => TemplateToken::Album(FieldCase::Upper),
        "album" => TemplateToken::Album(FieldCase::Lower),
        "Title" => TemplateToken::Title(FieldCase::Default),
        "TITLE" => TemplateToken::Title(FieldCase::Upper),
        "title" => TemplateToken::Title(FieldCase::Lower),
        "Genre" => TemplateToken::Genre(FieldCase::Default),
        "GENRE" => TemplateToken::Genre(FieldCase::Upper),
        "genre" => TemplateToken::Genre(FieldCase::Lower),
        "year" => TemplateToken::Year,
        _ => return None,
    })
}

/// A parsed path template.
///
/// See the module documentation for the language and the rules of the
/// parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathTemplate {
    template: String,
    segments: Vec<TemplateSegment>,
}

impl PathTemplate {
    /// Parses `template`.
    ///
    /// The template is never rejected: a word the language does not know is
    /// literal text, and a template that is empty renders an empty path.
    pub fn parse(template: &str) -> PathTemplate {
        let segments = template.split('\\').map(TemplateSegment::parse).collect();
        PathTemplate {
            template: template.to_owned(),
            segments,
        }
    }

    /// The template as it was given.
    pub fn source(&self) -> &str {
        &self.template
    }

    /// The segments of the template: the directories first, the file name
    /// last.
    pub fn segments(&self) -> &[TemplateSegment] {
        &self.segments
    }

    /// Renders `row` through the template.
    ///
    /// A field the row does not carry renders as an empty text and is
    /// reported in [`RenderedPath::missing`].
    pub fn render(&self, row: &TrackRow) -> RenderedPath {
        let mut segments: Vec<String> = Vec::with_capacity(self.segments.len());
        let mut missing: Vec<TemplateField> = Vec::new();

        for segment in &self.segments {
            let mut text = String::new();
            for part in &segment.parts {
                match part {
                    TemplatePart::Literal(literal) => text.push_str(literal),
                    TemplatePart::Token(token) => match render_token(*token, &row.metadata) {
                        Some(value) => text.push_str(&value),
                        None => {
                            let field = token.field();
                            if !missing.contains(&field) {
                                missing.push(field);
                            }
                        }
                    },
                }
            }
            segments.push(text);
        }

        RenderedPath { segments, missing }
    }

    /// Plans the extraction of `rows` through the template.
    ///
    /// The plan holds the files the extraction writes and the tracks it
    /// cannot write; see the module documentation for the rules.
    pub fn plan(&self, rows: &[TrackRow]) -> ExtractPlan {
        let mut files: Vec<PlannedFile> = Vec::new();
        let mut errors: Vec<ExtractError> = Vec::new();
        // The paths the plan has already handed out. The second track that
        // renders to a path is an error, not a silent overwrite.
        let mut taken: BTreeSet<Vec<String>> = BTreeSet::new();

        for (index, row) in rows.iter().enumerate() {
            let number = index + 1;
            let path = self.render(row);

            if path.file_name().is_empty() {
                errors.push(ExtractError {
                    track_id: row.id,
                    number,
                    reason: ExtractErrorReason::EmptyPath,
                });
                continue;
            }

            if !taken.insert(path.segments.clone()) {
                errors.push(ExtractError {
                    track_id: row.id,
                    number,
                    reason: ExtractErrorReason::DuplicatePath {
                        path: join_segments(&path.segments),
                    },
                });
                continue;
            }

            files.push(PlannedFile {
                track_id: row.id,
                source: row.path.clone(),
                path,
            });
        }

        ExtractPlan { files, errors }
    }
}

/// The value a token inserts, or `None` when the track does not carry it.
fn render_token(token: TemplateToken, metadata: &TrackMetadata) -> Option<String> {
    Some(match token {
        TemplateToken::Artist(case) => apply_case(metadata.artist.as_deref()?, case),
        TemplateToken::Album(case) => apply_case(metadata.album.as_deref()?, case),
        TemplateToken::Title(case) => apply_case(metadata.title.as_deref()?, case),
        TemplateToken::Genre(case) => apply_case(metadata.genre.as_deref()?, case),
        TemplateToken::Year => metadata.year?.to_string(),
        TemplateToken::TrackNumber { width } => {
            let number = metadata.track_no?;
            let width = usize::from(width);
            format!("{number:0width$}")
        }
    })
}

/// The value in the capitalisation the token chose.
fn apply_case(value: &str, case: FieldCase) -> String {
    match case {
        FieldCase::Default => value.to_owned(),
        FieldCase::Upper => value.to_uppercase(),
        FieldCase::Lower => value.to_lowercase(),
    }
}

/// The segments as one path, with the separator of the platform.
fn join_segments(segments: &[String]) -> String {
    segments.join(std::path::MAIN_SEPARATOR_STR)
}

/// The path a row renders to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedPath {
    /// The segments of the path, relative to the target directory: the
    /// directories first, the file name last.
    pub segments: Vec<String>,
    /// The fields the row does not carry: the template asked for them and
    /// the row has no value. The value renders as an empty text.
    pub missing: Vec<TemplateField>,
}

impl RenderedPath {
    /// The name of the file, without the directories.
    pub fn file_name(&self) -> &str {
        self.segments.last().map(String::as_str).unwrap_or("")
    }

    /// The path of the file, relative to the target directory, with the
    /// separator of the platform.
    ///
    /// Whether the backslash of the template is literal on every platform or
    /// a separator that follows the platform is open (S20 §8, open question
    /// 8). This is the one place that joins the segments, so a caller that
    /// wants the other reading joins [`RenderedPath::segments`] itself.
    pub fn relative_path(&self) -> PathBuf {
        self.segments.iter().collect()
    }
}

/// One file of an extraction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    /// The row id of the track.
    pub track_id: i64,
    /// The path of the source file.
    pub source: PathBuf,
    /// The path of the target file, relative to the target directory.
    pub path: RenderedPath,
}

/// Why a track could not be extracted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtractErrorReason {
    /// The template rendered no file name: there is nothing to write.
    EmptyPath,
    /// Another track of the same extraction renders to the same path.
    DuplicatePath {
        /// The path both tracks render to.
        path: String,
    },
}

impl fmt::Display for ExtractErrorReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtractErrorReason::EmptyPath => write!(f, "the template rendered no file name"),
            ExtractErrorReason::DuplicatePath { path } => {
                write!(f, "another track already extracts to {path}")
            }
        }
    }
}

/// One track an extraction cannot write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractError {
    /// The row id of the track.
    pub track_id: i64,
    /// The number of the track in the selection, counted from one.
    pub number: usize,
    /// Why the track could not be extracted.
    pub reason: ExtractErrorReason,
}

impl fmt::Display for ExtractError {
    /// The message of the reference: `Error extracting track #N: <reason>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Error extracting track #{}: {}",
            self.number, self.reason
        )
    }
}

/// The counts the report of an extraction needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractCounts {
    /// The number of tracks the plan writes: the `N` of `N tracks
    /// extracted`.
    pub extracted: usize,
    /// The number of tracks the plan cannot write.
    pub failed: usize,
}

/// The plan of one extraction: the files it writes and the tracks it cannot
/// write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractPlan {
    /// The files the extraction writes, in the order of the selection.
    pub files: Vec<PlannedFile>,
    /// The tracks the extraction cannot write, in the order of the
    /// selection.
    pub errors: Vec<ExtractError>,
}

impl ExtractPlan {
    /// The counts the report needs.
    pub fn counts(&self) -> ExtractCounts {
        ExtractCounts {
            extracted: self.files.len(),
            failed: self.errors.len(),
        }
    }
}

/// What the extraction does with the target playlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaylistAction {
    /// The target does not exist: the extraction writes it.
    Create,
    /// The target exists and the user answered the prompt with yes: the
    /// extraction replaces it.
    Overwrite,
    /// The target exists and the user answered the prompt with no: the
    /// extraction appends to it.
    Append,
}

/// The answer the user gave to the overwrite prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteAnswer {
    /// "Yes": the existing playlist is replaced.
    Yes,
    /// "No": the entries are appended to the existing playlist.
    No,
}

/// The decision: what the extraction does with the target playlist.
///
/// The prompt is only asked when the target exists (S20 §4). Its own text
/// says that answering "no" appends, so "no" means append and "yes" means
/// overwrite.
pub fn choose_playlist_action(target_exists: bool, answer: OverwriteAnswer) -> PlaylistAction {
    if !target_exists {
        return PlaylistAction::Create;
    }
    match answer {
        OverwriteAnswer::Yes => PlaylistAction::Overwrite,
        OverwriteAnswer::No => PlaylistAction::Append,
    }
}

/// The message of a finished extraction: `N tracks extracted`.
pub fn extracted_message(tracks: usize) -> String {
    format!("{tracks} tracks extracted")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackRow;

    /// A row with the given tags, for the renderer and the plan.
    fn row(id: i64, metadata: TrackMetadata) -> TrackRow {
        TrackRow {
            id,
            path: PathBuf::from(format!(r"C:\music\{id}.mp3")),
            metadata,
            mtime_ns: None,
            size: None,
        }
    }

    /// A row that carries every field the language knows.
    fn full_row(id: i64) -> TrackRow {
        row(
            id,
            TrackMetadata {
                title: Some("Dancing Queen".to_owned()),
                artist: Some("Abba Gold".to_owned()),
                album: Some("Greatest Hits".to_owned()),
                genre: Some("Pop".to_owned()),
                year: Some(1997),
                track_no: Some(7),
                ..Default::default()
            },
        )
    }

    /// The segments a template renders a row to.
    fn rendered(template: &str, row: &TrackRow) -> Vec<String> {
        PathTemplate::parse(template).render(row).segments
    }

    #[test]
    fn the_template_of_the_artist_and_the_album_builds_a_folder_and_a_file() {
        let template = PathTemplate::parse(r"Artist - Album\Artist - Album");
        let path = template.render(&full_row(1));
        assert_eq!(
            path.segments,
            vec!["Abba Gold - Greatest Hits", "Abba Gold - Greatest Hits"]
        );
        assert_eq!(path.file_name(), "Abba Gold - Greatest Hits");
        assert!(path.missing.is_empty());
    }

    #[test]
    fn the_template_of_the_track_number_renders_the_padded_number() {
        let template = PathTemplate::parse("## - Artist - Title");
        let path = template.render(&full_row(1));
        assert_eq!(path.segments, vec!["07 - Abba Gold - Dancing Queen"]);
    }

    #[test]
    fn the_capitalisation_of_the_token_chooses_the_capitalisation_of_the_value() {
        assert_eq!(rendered("Artist", &full_row(1)), vec!["Abba Gold"]);
        assert_eq!(rendered("ARTIST", &full_row(1)), vec!["ABBA GOLD"]);
        assert_eq!(rendered("artist", &full_row(1)), vec!["abba gold"]);

        assert_eq!(rendered("Album", &full_row(1)), vec!["Greatest Hits"]);
        assert_eq!(rendered("ALBUM", &full_row(1)), vec!["GREATEST HITS"]);
        assert_eq!(rendered("album", &full_row(1)), vec!["greatest hits"]);

        assert_eq!(rendered("Title", &full_row(1)), vec!["Dancing Queen"]);
        assert_eq!(rendered("TITLE", &full_row(1)), vec!["DANCING QUEEN"]);
        assert_eq!(rendered("title", &full_row(1)), vec!["dancing queen"]);

        assert_eq!(rendered("Genre", &full_row(1)), vec!["Pop"]);
        assert_eq!(rendered("GENRE", &full_row(1)), vec!["POP"]);
        assert_eq!(rendered("genre", &full_row(1)), vec!["pop"]);
    }

    #[test]
    fn the_track_number_is_padded_to_the_number_of_hashes() {
        assert_eq!(rendered("#", &full_row(1)), vec!["7"]);
        assert_eq!(rendered("##", &full_row(1)), vec!["07"]);
        assert_eq!(rendered("###", &full_row(1)), vec!["007"]);
    }

    #[test]
    fn a_number_that_is_longer_than_the_padding_is_not_cut() {
        let row = row(
            1,
            TrackMetadata {
                track_no: Some(123),
                ..Default::default()
            },
        );
        assert_eq!(rendered("##", &row), vec!["123"]);
        assert_eq!(rendered("###", &row), vec!["123"]);
    }

    #[test]
    fn a_longer_run_of_hashes_is_not_a_token_of_the_language() {
        assert_eq!(rendered("####", &full_row(1)), vec!["####"]);
        assert_eq!(
            rendered("## - Title", &full_row(1)),
            vec!["07 - Dancing Queen"]
        );
    }

    #[test]
    fn the_year_is_carried_in_one_spelling_only() {
        assert_eq!(rendered("year", &full_row(1)), vec!["1997"]);
        // `Year` and `YEAR` are not spellings of the language: they stay
        // literal text.
        assert_eq!(rendered("Year", &full_row(1)), vec!["Year"]);
        assert_eq!(rendered("YEAR", &full_row(1)), vec!["YEAR"]);
    }

    #[test]
    fn a_backslash_separates_the_directories() {
        let template = PathTemplate::parse(r"Artist\Album\Title");
        assert_eq!(template.segments().len(), 3);
        let path = template.render(&full_row(1));
        assert_eq!(
            path.segments,
            vec!["Abba Gold", "Greatest Hits", "Dancing Queen"]
        );
        assert_eq!(path.file_name(), "Dancing Queen");
    }

    #[test]
    fn a_word_the_language_does_not_know_stays_literal() {
        assert_eq!(
            rendered("Hello - Artist", &full_row(1)),
            vec!["Hello - Abba Gold"]
        );
        assert_eq!(rendered("track", &full_row(1)), vec!["track"]);
        assert_eq!(rendered("ntracks", &full_row(1)), vec!["ntracks"]);
        assert_eq!(rendered("Album2", &full_row(1)), vec!["Album2"]);
    }

    #[test]
    fn a_missing_field_renders_empty_and_is_reported() {
        let row = row(
            1,
            TrackMetadata {
                artist: Some("Abba Gold".to_owned()),
                ..Default::default()
            },
        );
        let path = PathTemplate::parse("Artist - Album").render(&row);
        assert_eq!(path.segments, vec!["Abba Gold - "]);
        assert_eq!(path.missing, vec![TemplateField::Album]);
    }

    #[test]
    fn a_row_without_any_field_does_not_panic() {
        let row = row(1, TrackMetadata::default());
        let path = PathTemplate::parse("Artist - ## - year").render(&row);
        assert_eq!(path.segments, vec![" -  - "]);
        assert_eq!(
            path.missing,
            vec![
                TemplateField::Artist,
                TemplateField::TrackNumber,
                TemplateField::Year
            ]
        );
    }

    #[test]
    fn a_field_the_row_lacks_is_reported_once() {
        let row = row(1, TrackMetadata::default());
        let path = PathTemplate::parse("Artist - ARTIST").render(&row);
        assert_eq!(path.missing, vec![TemplateField::Artist]);
    }

    #[test]
    fn the_plan_writes_every_track_of_the_selection() {
        let template = PathTemplate::parse("## - Artist - Title");
        // Three tracks that render to three different paths.
        let rows: Vec<TrackRow> = (0..3)
            .map(|id| {
                let mut row = full_row(id + 1);
                row.metadata.title = Some(format!("Song {}", id + 1));
                row
            })
            .collect();
        let plan = template.plan(&rows);

        assert_eq!(plan.files.len(), 3);
        assert!(plan.errors.is_empty());
        assert_eq!(plan.counts().extracted, 3);
        assert_eq!(plan.counts().failed, 0);
        assert_eq!(plan.files[0].path.file_name(), "07 - Abba Gold - Song 1");
        assert_eq!(plan.files[0].source, PathBuf::from(r"C:\music\1.mp3"));
    }

    #[test]
    fn two_tracks_that_render_to_the_same_path_are_reported() {
        let template = PathTemplate::parse("Artist - Title");
        let rows = vec![full_row(1), full_row(2)];
        let plan = template.plan(&rows);

        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.errors.len(), 1);
        assert_eq!(plan.counts().extracted, 1);
        assert_eq!(plan.counts().failed, 1);

        let error = &plan.errors[0];
        assert_eq!(error.track_id, 2);
        assert_eq!(error.number, 2);
        assert_eq!(
            error.reason,
            ExtractErrorReason::DuplicatePath {
                path: "Abba Gold - Dancing Queen".to_owned()
            }
        );
        assert_eq!(
            error.to_string(),
            "Error extracting track #2: another track already extracts to Abba Gold - Dancing Queen"
        );
    }

    #[test]
    fn a_track_without_a_file_name_is_an_error() {
        let template = PathTemplate::parse("");
        let plan = template.plan(&[full_row(1)]);

        assert!(plan.files.is_empty());
        assert_eq!(plan.errors.len(), 1);
        assert_eq!(plan.errors[0].reason, ExtractErrorReason::EmptyPath);
        assert_eq!(
            plan.errors[0].to_string(),
            "Error extracting track #1: the template rendered no file name"
        );
    }

    #[test]
    fn the_counts_add_up_to_the_selection() {
        let template = PathTemplate::parse("Artist - Title");
        let rows = vec![full_row(1), full_row(2), full_row(3)];
        let plan = template.plan(&rows);

        let counts = plan.counts();
        assert_eq!(counts.extracted + counts.failed, rows.len());
        assert_eq!(counts.extracted, 1);
        assert_eq!(counts.failed, 2);
    }

    #[test]
    fn a_planned_file_keeps_the_fields_the_track_did_not_carry() {
        let template = PathTemplate::parse("Artist - Album");
        let row = row(
            1,
            TrackMetadata {
                artist: Some("Abba Gold".to_owned()),
                ..Default::default()
            },
        );
        let plan = template.plan(&[row]);

        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.files[0].path.missing, vec![TemplateField::Album]);
    }

    #[test]
    fn the_template_is_parsed_into_segments_and_tokens() {
        let template = PathTemplate::parse(r"Artist - Album\## - Title");
        assert_eq!(template.source(), r"Artist - Album\## - Title");
        assert_eq!(template.segments().len(), 2);

        let first = &template.segments()[0];
        assert_eq!(
            first.parts(),
            &[
                TemplatePart::Token(TemplateToken::Artist(FieldCase::Default)),
                TemplatePart::Literal(" - ".to_owned()),
                TemplatePart::Token(TemplateToken::Album(FieldCase::Default)),
            ]
        );

        let second = &template.segments()[1];
        assert_eq!(
            second.parts(),
            &[
                TemplatePart::Token(TemplateToken::TrackNumber { width: 2 }),
                TemplatePart::Literal(" - ".to_owned()),
                TemplatePart::Token(TemplateToken::Title(FieldCase::Default)),
            ]
        );
    }

    #[test]
    fn the_relative_path_joins_the_segments() {
        let template = PathTemplate::parse(r"Artist\Title");
        let path = template.render(&full_row(1));
        assert_eq!(
            path.relative_path(),
            PathBuf::from("Abba Gold").join("Dancing Queen")
        );
    }

    #[test]
    fn the_overwrite_prompt_decides_between_overwrite_and_append() {
        assert_eq!(OVERWRITE_PROMPT, "Overwrite playlist (No will append)?");
        assert_eq!(
            choose_playlist_action(false, OverwriteAnswer::Yes),
            PlaylistAction::Create
        );
        assert_eq!(
            choose_playlist_action(false, OverwriteAnswer::No),
            PlaylistAction::Create
        );
        assert_eq!(
            choose_playlist_action(true, OverwriteAnswer::Yes),
            PlaylistAction::Overwrite
        );
        assert_eq!(
            choose_playlist_action(true, OverwriteAnswer::No),
            PlaylistAction::Append
        );
    }

    #[test]
    fn the_result_of_the_extraction_carries_the_count() {
        assert_eq!(extracted_message(0), "0 tracks extracted");
        assert_eq!(extracted_message(7), "7 tracks extracted");
    }
}
