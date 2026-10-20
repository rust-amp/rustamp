// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The playlist extraction of the media library (spec S20 §4).
//!
//! The media library can write its selection out as a playlist. The **file
//! names** come from a path template in a language of its own (S20 §4.2) -
//! bare words, no `%name%` and no functions - and the template engine is
//! the `extract` module of `rustamp-library`: it parses a template, renders
//! a row through it and plans the extraction of a selection. It writes no
//! file and asks the user nothing.
//!
//! This module is the other half: it writes the files the plan names, it
//! writes the playlist that lists them, and it reports what happened.
//!
//! # What an extraction writes
//!
//! * **The files.** Every planned file is copied from the library into the
//!   target directory, under the path the template rendered. The name of
//!   the copy carries the extension of the source file: the template has no
//!   token for an extension (S20 §4.2 covers five of the seven field names
//!   of S20 §3), and a file without its extension is not playable.
//! * **The playlist.** One M3U file in the target directory, named
//!   [`PLAYLIST_NAME`], listing the files that were written. The paths in
//!   it are relative to the playlist, so the directory can be moved
//!   somewhere else and still play.
//!
//! # The question about an existing playlist
//!
//! When the playlist file is already there, the reference asks
//! `Overwrite playlist (No will append)?` (S20 §4) before anything is
//! written. The decision behind the answer is the library's
//! [`choose_playlist_action`]; the question itself is asked by the caller,
//! through the `ask` closure of [`extract`].
//!
//! **One failure does not abort the rest** (S20 §4): a track that cannot be
//! written is reported with the reference's own wording, `Error extracting
//! track #N: <reason>`, and every other track is written anyway.
//!
//! # The rules that are ours
//!
//! S20 measures nothing about the reference's own page, so the following
//! are our own choices, stated here:
//!
//! * the playlist is an M3U file called `playlist.m3u` in the target
//!   directory;
//! * the copied file keeps the extension of its source;
//! * the target directory has to be there: an extraction into a directory
//!   that does not exist is refused, because every copy would fail and one
//!   error per track would hide the reason;
//! * a selection with nothing in it is refused: an empty playlist file
//!   would be the only thing the extraction could leave behind;
//! * a rendered path that would leave the target directory - a segment with
//!   a separator in it, or one that names the directory above the target -
//!   is refused: the extraction writes inside the target directory and
//!   nowhere else;
//! * the number in `Error extracting track #N` is the position of the track
//!   in the selection, counted from one.
//!
//! # Known limits
//!
//! * The extraction runs on the **interface thread**: a large selection
//!   blocks the window while the files are copied. A worker thread would
//!   need the plan and the answer of the question to travel to it, and the
//!   question has to be asked on the interface thread.
//! * The playlist is written as M3U, and the M3U format has no room for a
//!   path that begins with a `#` (it would read back as a comment). A
//!   template that renders such a name writes its files but not its
//!   playlist, and the note says so.
//!
//! # What is not here
//!
//! The **configuration step** - the path template and the target directory -
//! is asked by the presenter in a native dialog (see
//! `rustamp_platform::ask_extract_settings`), and the **selection** is the
//! library window's own (see [`crate::library_window`]).

use std::path::{Path, PathBuf};

use rustamp_core::playlist::MediaLocation;
use rustamp_library::{
    OVERWRITE_PROMPT, OverwriteAnswer, PathTemplate, PlannedFile, PlaylistAction, TrackRow,
    choose_playlist_action, extracted_message,
};
use rustamp_platform::ExtractSettings;
use rustamp_playlist::{LoadedEntry, PlaylistFormat, write_playlist};

use crate::app::App;

#[cfg(test)]
#[path = "library_extract_tests.rs"]
mod library_extract_tests;

/// The name of the playlist file the extraction writes.
///
/// **Own choice** (see the module docs): the file lives in the target
/// directory and carries the M3U extension.
pub const PLAYLIST_NAME: &str = "playlist.m3u";

/// The path template the configuration dialog starts with.
///
/// **Own choice**, and a string the reference carries: S20 §4.1 names
/// `## - Artist - Title` as one of the two templates of the plug-in.
pub const DEFAULT_TEMPLATE: &str = "## - Artist - Title";

/// The outcome of an extraction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractOutcome {
    /// The number of tracks that were written.
    pub extracted: usize,
    /// The messages of the tracks that were not written, in the wording of
    /// the reference: `Error extracting track #N: <reason>`.
    pub errors: Vec<String>,
    /// The playlist file the extraction wrote, if it wrote one.
    pub playlist: Option<PathBuf>,
    /// Why the extraction did not run at all, if it did not.
    ///
    /// A template that names no file and a question that was not answered
    /// both leave the disk untouched, and both say so here.
    pub refused: Option<String>,
}

impl ExtractOutcome {
    /// The note the extraction leaves for the user.
    ///
    /// The counts come first, in the reference's own wording (`N tracks
    /// extracted`), then the playlist that was written, and every track that
    /// was not written, so the user sees the failures of a partial
    /// extraction.
    pub fn note(&self) -> String {
        if let Some(refused) = &self.refused {
            return refused.clone();
        }
        let mut note = extracted_message(self.extracted);
        if let Some(playlist) = &self.playlist {
            note.push_str(&format!("; playlist: {}", playlist.display()));
        }
        for error in &self.errors {
            note.push_str("; ");
            note.push_str(error);
        }
        note
    }
}

/// Writes the selection out as a playlist.
///
/// `ask` answers the question about an existing playlist: it is called with
/// the reference's own prompt and answers with the user's answer. `None`
/// means the question was not answered, and then **nothing is written at
/// all** - not the files, not the playlist.
///
/// The files are written before the playlist, so a playlist that lists a
/// file always names a file that is there.
pub fn extract(
    rows: &[TrackRow],
    settings: &ExtractSettings,
    ask: &mut dyn FnMut(&str) -> Option<OverwriteAnswer>,
) -> ExtractOutcome {
    let template = PathTemplate::parse(&settings.template);
    if rows.is_empty() {
        // There is nothing to extract: an empty playlist file would be the
        // only thing the extraction could leave behind.
        return ExtractOutcome {
            refused: Some("there is nothing to extract: nothing was written".to_owned()),
            ..Default::default()
        };
    }
    if !names_a_file(&template) {
        // A template whose last segment is empty cannot name a file, ever:
        // nothing can be written, so the template is reported and the disk
        // is left alone.
        return ExtractOutcome {
            refused: Some("the path template names no file: nothing was written".to_owned()),
            ..Default::default()
        };
    }
    if !settings.directory.is_dir() {
        // The directory the user named is not there: every copy would fail,
        // and one error per track would hide the reason.
        return ExtractOutcome {
            refused: Some(format!(
                "the target directory is not a directory: {}: nothing was written",
                settings.directory.display()
            )),
            ..Default::default()
        };
    }

    let plan = template.plan(rows);
    let playlist = settings.directory.join(PLAYLIST_NAME);
    let action = if playlist.is_file() {
        match ask(OVERWRITE_PROMPT) {
            Some(answer) => choose_playlist_action(true, answer),
            None => {
                return ExtractOutcome {
                    refused: Some(
                        "the question about the existing playlist was not answered: nothing was \
                         written"
                            .to_owned(),
                    ),
                    ..Default::default()
                };
            }
        }
    } else {
        // The target does not exist: the answer does not matter, and the
        // library's own decision says so.
        choose_playlist_action(false, OverwriteAnswer::Yes)
    };

    let mut errors: Vec<(usize, String)> = plan
        .errors
        .iter()
        .map(|error| (error.number, error.to_string()))
        .collect();
    let mut written: Vec<PathBuf> = Vec::new();
    for file in &plan.files {
        let number = number_in(rows, file.track_id);
        let Some(relative) = relative_path(file) else {
            errors.push((
                number,
                format!("Error extracting track #{number}: the path leaves the target directory"),
            ));
            continue;
        };
        match copy_into(&file.source, &settings.directory.join(&relative)) {
            Ok(()) => written.push(relative),
            Err(error) => errors.push((
                number,
                format!("Error extracting track #{number}: the file could not be copied: {error}"),
            )),
        }
    }
    // The failures are reported in the order of the selection.
    errors.sort_by_key(|(number, _)| *number);

    let entries: Vec<LoadedEntry> = written
        .iter()
        .map(|relative| LoadedEntry::new(MediaLocation::File(relative.clone())))
        .collect();
    let mut messages: Vec<String> = errors.into_iter().map(|(_, message)| message).collect();
    let written_playlist = match write_playlist_file(&playlist, action, &entries) {
        Ok(()) => Some(playlist),
        Err(error) => {
            messages.push(error);
            None
        }
    };

    ExtractOutcome {
        extracted: written.len(),
        errors: messages,
        playlist: written_playlist,
        refused: None,
    }
}

/// Whether the template can name a file at all.
///
/// The template is never rejected by the parser - a word the language does
/// not know is literal text - so the check is the one that matters for the
/// writing: the **last** segment of the template has to carry something.
/// An empty last segment renders an empty file name for every track, and a
/// file without a name cannot be written.
fn names_a_file(template: &PathTemplate) -> bool {
    template
        .segments()
        .last()
        .is_some_and(|segment| !segment.parts().is_empty())
}

/// The path of a planned file, relative to the target directory.
///
/// `None` when the rendered path would leave the target directory: a segment
/// with a separator in it (a tag can carry one) or one that names the
/// directory above the target.
fn relative_path(file: &PlannedFile) -> Option<PathBuf> {
    let (name, directories) = file.path.segments.split_last()?;
    let mut relative = PathBuf::new();
    for segment in directories {
        if !is_safe_segment(segment) {
            return None;
        }
        relative.push(segment);
    }
    if !is_safe_segment(name) {
        return None;
    }
    relative.push(file_name(file));
    Some(relative)
}

/// Whether a rendered segment can be a directory or a file name.
///
/// A segment comes from the template and from the tags of the track, so it
/// can carry a separator (`..\..`) or a name that points above the target
/// directory. The extraction writes inside the target directory and nowhere
/// else, so such a segment is refused.
fn is_safe_segment(segment: &str) -> bool {
    !segment.is_empty() && segment != "." && segment != ".." && !segment.contains(['/', '\\', ':'])
}

/// The name of the copied file: the rendered name with the extension of the
/// source file.
///
/// The template has no token for an extension (S20 §4.2), and a file
/// without its extension is not playable, so the extension of the source
/// travels with the copy. A name that already carries the extension keeps
/// it: the title of a track can be the file name with its extension.
fn file_name(file: &PlannedFile) -> String {
    let name = file.path.file_name();
    let Some(extension) = file.source.extension().and_then(|value| value.to_str()) else {
        return name.to_owned();
    };
    // A name that already carries the extension keeps it: the title of a
    // track can be the file name with its extension.
    let suffix = format!(".{extension}");
    if name.to_lowercase().ends_with(&suffix.to_lowercase()) {
        return name.to_owned();
    }
    format!("{name}.{extension}")
}

/// The number of the track in the selection, counted from one.
///
/// The plan is built from the selection, so every planned file belongs to a
/// row of it. A row that is not in the selection cannot be written, and the
/// number zero says so.
fn number_in(rows: &[TrackRow], track_id: i64) -> usize {
    rows.iter()
        .position(|row| row.id == track_id)
        .map_or(0, |index| index + 1)
}

/// Copies the source file to the target, creating the directories above it.
fn copy_into(source: &Path, target: &Path) -> std::io::Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, target)?;
    Ok(())
}

/// Writes the playlist file.
///
/// The action says what happens to a file that is already there: it is
/// replaced or the new entries are appended to it.
fn write_playlist_file(
    playlist: &Path,
    action: PlaylistAction,
    entries: &[LoadedEntry],
) -> Result<(), String> {
    let bytes = write_playlist(PlaylistFormat::M3u, entries).map_err(|error| error.to_string())?;
    // The target directory is created when it is not there: the extraction
    // writes into it, and an empty extraction still writes its playlist.
    if let Some(parent) = playlist.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("the target directory could not be created: {error}"))?;
    }
    match action {
        PlaylistAction::Append => {
            let mut existing = std::fs::read(playlist)
                .map_err(|error| format!("the existing playlist could not be read: {error}"))?;
            if !existing.is_empty() && !existing.ends_with(b"\n") {
                // The entries of the file and the new ones stand in one
                // list, so the last line of the old file needs its end.
                existing.extend_from_slice(b"\r\n");
            }
            existing.extend_from_slice(&bytes);
            std::fs::write(playlist, existing)
                .map_err(|error| format!("the playlist could not be written: {error}"))
        }
        PlaylistAction::Create | PlaylistAction::Overwrite => std::fs::write(playlist, bytes)
            .map_err(|error| format!("the playlist could not be written: {error}")),
    }
}

impl App {
    /// The settings the configuration dialog starts with.
    ///
    /// They are the ones of the last extraction, so a second extraction
    /// needs no typing.
    pub fn extract_settings(&self) -> &ExtractSettings {
        &self.last_extract_settings
    }

    /// Writes the library's selection out as a playlist.
    ///
    /// `settings` is `None` when the user closed the configuration dialog:
    /// then **nothing is written at all**, and the app says nothing either.
    ///
    /// `ask` answers the question about an existing playlist; see
    /// [`extract`]. The rows are the library window's own: the selected row,
    /// or every row the search matches when nothing is selected (see
    /// [`crate::library_window::LibraryWindow::extract_rows`]).
    ///
    /// The result is a note for the user: the counts of the extraction and
    /// the tracks that were not written.
    pub fn library_extract(
        &mut self,
        settings: Option<&ExtractSettings>,
        ask: &mut dyn FnMut(&str) -> Option<OverwriteAnswer>,
    ) -> rustamp_core::error::Result<()> {
        let Some(settings) = settings else {
            // The configuration dialog was closed: nothing was asked and
            // nothing is written.
            return Ok(());
        };
        // The settings are kept for the next dialog, whether the extraction
        // wrote anything or not: the user typed them.
        self.last_extract_settings = settings.clone();
        let rows = self.library_window.extract_rows()?;
        let outcome = extract(&rows, settings, ask);
        self.note(format!("Library: {}", outcome.note()));
        Ok(())
    }
}
