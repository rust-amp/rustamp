// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The playlist extraction of the media library (spec S20 §4).
//!
//! Every test is headless: null output, the built-in skin, no OS window and
//! no child process. The library is a real index in a temporary directory,
//! the tracks are real MP3 files with the tags the template renders, and
//! everything the extraction writes lands in another temporary directory.
//!
//! The two dialogs of the extraction - the configuration and the question
//! about an existing playlist - need a desktop, so they are not part of
//! these tests: the question is answered by a closure instead, and a
//! question that is not answered is the "cancelled" case.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rustamp_core::config::Config;
use rustamp_core::meta::TrackMetadata;
use rustamp_library::OverwriteAnswer;
use rustamp_platform::ExtractSettings;
use rustamp_ui::{Key, Modifiers};
use tempfile::TempDir;

use super::*;
use crate::app::{App, AppOptions};
use crate::cli::OutputChoice;

/// The media fixture: a real MP3.
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rustamp-decoders/tests/fixtures")
        .join("sine.mp3")
}

/// A track with the given tags, in `dir`.
///
/// The tags are written into the file, so the library's scan reads them and
/// the template has something to render.
fn track(dir: &Path, name: &str, title: &str, artist: &str, album: &str, number: u32) -> PathBuf {
    let path = dir.join(name);
    std::fs::copy(fixture(), &path).expect("copy the fixture");
    let metadata = TrackMetadata {
        title: Some(title.to_owned()),
        artist: Some(artist.to_owned()),
        album: Some(album.to_owned()),
        track_no: Some(number),
        ..Default::default()
    };
    rustamp_meta::write_tags_with_report(&path, &metadata, &[]).expect("write the tags");
    path
}

/// A headless player whose configuration lives in `dir`.
fn library_app(dir: &TempDir) -> App {
    let options = AppOptions {
        config: Config::default(),
        skin: crate::skin::builtin(),
        output: OutputChoice::Null,
        volume: 200,
        media: None,
        seconds: None,
    };
    let mut app = App::new(options).expect("create the player");
    app.set_config_path(Some(dir.path().join("rustamp.toml")));
    app
}

/// Adds `media` to the library and waits for the background scan.
///
/// The scan runs on the app's own thread, so the test pumps the app until
/// the rows are there.
fn scan(app: &mut App, media: &Path, rows: usize) {
    app.library_add_folder(media.to_path_buf())
        .expect("add the folder");
    for _ in 0..600 {
        app.pump().expect("pump");
        if app.library_view().total() == rows {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the library scan did not finish");
}

/// The notes the app left for the user.
fn notes(app: &mut App) -> Vec<String> {
    app.drain_notes()
}

/// The settings of an extraction into `directory` with `template`.
fn settings(template: &str, directory: &Path) -> ExtractSettings {
    ExtractSettings {
        template: template.to_owned(),
        directory: directory.to_path_buf(),
    }
}

/// The target directory of an extraction.
///
/// The directory has to be there: the folder dialog only offers the
/// directories that exist, and an extraction into a directory that is not
/// there is refused.
fn target_dir(dir: &TempDir, name: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::create_dir_all(&path).expect("the target folder");
    path
}

#[test]
fn the_extract_action_reaches_the_app() {
    let dir = TempDir::new().expect("temp dir");
    let mut app = library_app(&dir);

    assert!(!app.take_library_extract_request());
    app.library_key_down(Key::Letter(b'X'), Modifiers::CTRL.union(Modifiers::SHIFT))
        .expect("the key");
    assert!(app.take_library_extract_request());
    // The request is taken once: the presenter asks for the settings.
    assert!(!app.take_library_extract_request());
}

#[test]
fn a_temp_library_with_two_rows_writes_the_expected_file_names() {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("the media folder");
    track(
        &media,
        "one.mp3",
        "Dancing Queen",
        "Abba Gold",
        "Greatest Hits",
        7,
    );
    track(
        &media,
        "two.mp3",
        "Money Money Money",
        "Abba Gold",
        "Greatest Hits",
        8,
    );

    let mut app = library_app(&dir);
    scan(&mut app, &media, 2);

    let target = target_dir(&dir, "target");
    let mut ask = |_: &str| Some(OverwriteAnswer::Yes);
    app.library_extract(
        Some(&settings(r"Artist - Album\## - Title", &target)),
        &mut ask,
    )
    .expect("the extraction");

    // The names come from the template, and the extension of the source
    // travels with the copy.
    let album = target.join("Abba Gold - Greatest Hits");
    assert!(album.join("07 - Dancing Queen.mp3").is_file(), "{album:?}");
    assert!(
        album.join("08 - Money Money Money.mp3").is_file(),
        "{album:?}"
    );

    // The playlist lists the files that were written, relative to itself.
    let playlist = target.join(PLAYLIST_NAME);
    let text = std::fs::read_to_string(&playlist).expect("the playlist");
    assert!(
        text.contains(r"Abba Gold - Greatest Hits\07 - Dancing Queen.mp3"),
        "{text}"
    );
    assert!(
        text.contains(r"Abba Gold - Greatest Hits\08 - Money Money Money.mp3"),
        "{text}"
    );

    // The note carries the count of the extraction.
    let notes = notes(&mut app);
    assert!(
        notes.iter().any(|note| note.contains("2 tracks extracted")),
        "{notes:?}"
    );
}

#[test]
fn a_duplicate_reports_the_second_row_and_writes_the_first() {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("the media folder");
    // Two tracks that render to the same path.
    track(
        &media,
        "one.mp3",
        "Dancing Queen",
        "Abba Gold",
        "Greatest Hits",
        7,
    );
    track(
        &media,
        "two.mp3",
        "Dancing Queen",
        "Abba Gold",
        "Greatest Hits",
        8,
    );

    let mut app = library_app(&dir);
    scan(&mut app, &media, 2);

    let target = target_dir(&dir, "target");
    let mut ask = |_: &str| Some(OverwriteAnswer::Yes);
    app.library_extract(Some(&settings("Artist - Title", &target)), &mut ask)
        .expect("the extraction");

    // The first track keeps the path and is written.
    assert!(target.join("Abba Gold - Dancing Queen.mp3").is_file());

    // The second one is reported in the wording of the reference, and the
    // extraction goes on.
    let notes = notes(&mut app);
    assert!(
        notes.iter().any(|note| note.contains(
            "Error extracting track #2: another track already extracts to Abba Gold - Dancing Queen"
        )),
        "{notes:?}"
    );
    assert!(
        notes.iter().any(|note| note.contains("1 tracks extracted")),
        "{notes:?}"
    );
}

#[test]
fn a_cancelled_configuration_writes_nothing() {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("the media folder");
    track(
        &media,
        "one.mp3",
        "Dancing Queen",
        "Abba Gold",
        "Greatest Hits",
        7,
    );

    let mut app = library_app(&dir);
    scan(&mut app, &media, 1);

    // The configuration dialog was closed: there are no settings.
    let mut ask = |_: &str| Some(OverwriteAnswer::Yes);
    app.library_extract(None, &mut ask).expect("the extraction");

    let notes = notes(&mut app);
    assert!(notes.is_empty(), "{notes:?}");
    // Nothing was written anywhere.
    let target = dir.path().join("target");
    assert!(!target.exists());
}

#[test]
fn a_question_that_was_not_answered_changes_nothing() {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("the media folder");
    track(
        &media,
        "one.mp3",
        "Dancing Queen",
        "Abba Gold",
        "Greatest Hits",
        7,
    );

    let mut app = library_app(&dir);
    scan(&mut app, &media, 1);

    let target = target_dir(&dir, "target");
    let playlist = target.join(PLAYLIST_NAME);
    std::fs::write(&playlist, b"#EXTM3U\r\nold.mp3\r\n").expect("the playlist");

    // The question about the existing playlist is not answered.
    let mut ask = |_: &str| -> Option<OverwriteAnswer> { None };
    app.library_extract(Some(&settings("Artist - Title", &target)), &mut ask)
        .expect("the extraction");

    // No file was written and the playlist is untouched.
    assert!(!target.join("Abba Gold - Dancing Queen.mp3").exists());
    assert_eq!(
        std::fs::read(&playlist).expect("the playlist"),
        b"#EXTM3U\r\nold.mp3\r\n"
    );

    let notes = notes(&mut app);
    assert!(
        notes
            .iter()
            .any(|note| note.contains("nothing was written")),
        "{notes:?}"
    );
}

#[test]
fn the_answer_no_appends_to_the_existing_playlist() {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("the media folder");
    track(
        &media,
        "one.mp3",
        "Dancing Queen",
        "Abba Gold",
        "Greatest Hits",
        7,
    );

    let mut app = library_app(&dir);
    scan(&mut app, &media, 1);

    let target = target_dir(&dir, "target");
    let mut ask = |_: &str| Some(OverwriteAnswer::Yes);
    app.library_extract(Some(&settings("Artist - Title", &target)), &mut ask)
        .expect("the first extraction");

    let playlist = target.join(PLAYLIST_NAME);
    let before = std::fs::read_to_string(&playlist).expect("the playlist");
    assert_eq!(before.matches(".mp3").count(), 1, "{before}");

    // The second extraction finds the playlist and is answered with "no":
    // the entries are appended to the file that is there.
    let mut ask = |_: &str| Some(OverwriteAnswer::No);
    app.library_extract(Some(&settings("Artist - Title", &target)), &mut ask)
        .expect("the second extraction");

    let after = std::fs::read_to_string(&playlist).expect("the playlist");
    assert!(after.starts_with(&before), "{after}");
    assert_eq!(after.matches(".mp3").count(), 2, "{after}");
}

#[test]
fn the_answer_yes_replaces_the_existing_playlist() {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("the media folder");
    track(
        &media,
        "one.mp3",
        "Dancing Queen",
        "Abba Gold",
        "Greatest Hits",
        7,
    );

    let mut app = library_app(&dir);
    scan(&mut app, &media, 1);

    let target = target_dir(&dir, "target");
    let playlist = target.join(PLAYLIST_NAME);
    std::fs::write(&playlist, b"#EXTM3U\r\nold.mp3\r\n").expect("the playlist");

    let mut ask = |_: &str| Some(OverwriteAnswer::Yes);
    app.library_extract(Some(&settings("Artist - Title", &target)), &mut ask)
        .expect("the extraction");

    let after = std::fs::read_to_string(&playlist).expect("the playlist");
    assert!(!after.contains("old.mp3"), "{after}");
    assert_eq!(after.matches(".mp3").count(), 1, "{after}");
}

#[test]
fn a_template_that_names_no_file_is_reported_and_writes_nothing() {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("the media folder");
    track(
        &media,
        "one.mp3",
        "Dancing Queen",
        "Abba Gold",
        "Greatest Hits",
        7,
    );

    let mut app = library_app(&dir);
    scan(&mut app, &media, 1);

    // A template whose last segment is empty cannot name a file.
    let target = dir.path().join("target");
    let mut ask = |_: &str| Some(OverwriteAnswer::Yes);
    app.library_extract(Some(&settings(r"Artist\", &target)), &mut ask)
        .expect("the extraction");

    assert!(!target.exists(), "nothing was written");
    let notes = notes(&mut app);
    assert!(
        notes
            .iter()
            .any(|note| note.contains("the path template names no file")),
        "{notes:?}"
    );
}

#[test]
fn a_target_directory_that_is_not_there_is_reported_and_writes_nothing() {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("the media folder");
    track(
        &media,
        "one.mp3",
        "Dancing Queen",
        "Abba Gold",
        "Greatest Hits",
        7,
    );

    let mut app = library_app(&dir);
    scan(&mut app, &media, 1);

    // The directory the user named is not there.
    let target = dir.path().join("nowhere");
    let mut ask = |_: &str| Some(OverwriteAnswer::Yes);
    app.library_extract(Some(&settings("Artist - Title", &target)), &mut ask)
        .expect("the extraction");

    assert!(!target.exists(), "nothing was written");
    let notes = notes(&mut app);
    assert!(
        notes
            .iter()
            .any(|note| note.contains("the target directory is not a directory")),
        "{notes:?}"
    );
}

#[test]
fn the_selected_row_is_written() {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("the media folder");
    track(
        &media,
        "one.mp3",
        "Dancing Queen",
        "Abba Gold",
        "Greatest Hits",
        7,
    );
    track(
        &media,
        "two.mp3",
        "Money Money Money",
        "Abba Gold",
        "Greatest Hits",
        8,
    );

    let mut app = library_app(&dir);
    scan(&mut app, &media, 2);

    // The first row is selected.
    app.library_key_down(Key::Down, Modifiers::NONE)
        .expect("select a row");
    assert!(app.library_view().selected().is_some());

    let target = target_dir(&dir, "target");
    let mut ask = |_: &str| Some(OverwriteAnswer::Yes);
    app.library_extract(Some(&settings("## - Title", &target)), &mut ask)
        .expect("the extraction");

    // Only the selected row was written.
    assert!(target.join("07 - Dancing Queen.mp3").is_file());
    assert!(!target.join("08 - Money Money Money.mp3").exists());
    let notes = notes(&mut app);
    assert!(
        notes.iter().any(|note| note.contains("1 tracks extracted")),
        "{notes:?}"
    );
}

#[test]
fn without_a_selection_the_whole_match_set_is_written() {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("the media folder");
    track(
        &media,
        "one.mp3",
        "Dancing Queen",
        "Abba Gold",
        "Greatest Hits",
        7,
    );
    track(
        &media,
        "two.mp3",
        "Money Money Money",
        "Abba Gold",
        "Greatest Hits",
        8,
    );

    let mut app = library_app(&dir);
    scan(&mut app, &media, 2);
    assert!(app.library_view().selected().is_none());

    let target = target_dir(&dir, "target");
    let mut ask = |_: &str| Some(OverwriteAnswer::Yes);
    app.library_extract(Some(&settings("## - Title", &target)), &mut ask)
        .expect("the extraction");

    // The page the window shows is only a window onto the match set: the
    // whole match set is written.
    assert!(target.join("07 - Dancing Queen.mp3").is_file());
    assert!(target.join("08 - Money Money Money.mp3").is_file());
    let notes = notes(&mut app);
    assert!(
        notes.iter().any(|note| note.contains("2 tracks extracted")),
        "{notes:?}"
    );
}
