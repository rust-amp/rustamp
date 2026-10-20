// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless tests of the preferences dialog's library page: the roots the
//! media library knows are the roots the page lists, a root can be removed
//! through the page, and the "Add folder..." button asks the presenter for a
//! folder.
//!
//! The roots live in the media library's own database and not in the
//! configuration file, so the tests look at the database to see what the page
//! did.
//!
//! Everything here is headless: null output, the built-in skin, no OS window
//! and no child process. The dialog itself, the folder picker and the drawn
//! controls stay VM-only.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_core::config::Config;
use rustamp_library::Library;
use rustamp_platform::{PrefsAction, library_root_text};
use tempfile::TempDir;

/// The media fixture the library indexes: a real MP3.
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rustamp-decoders/tests/fixtures")
        .join("sine.mp3")
}

/// A headless player whose configuration lives in `dir`.
fn prefs_app(dir: &TempDir) -> App {
    let options = AppOptions {
        config: Config::default(),
        skin: rustamp_app::skin::builtin(),
        output: OutputChoice::Null,
        volume: 200,
        file: None,
        seconds: None,
    };
    let mut app = App::new(options).expect("create the player");
    app.set_config_path(Some(dir.path().join("rustamp.toml")));
    app
}

/// The roots the media library's database holds.
fn database_roots(app: &App) -> Vec<PathBuf> {
    let path = app.library_database_path().expect("the database path");
    let index = Library::open(&path).expect("the database");
    index
        .roots()
        .expect("the roots")
        .into_iter()
        .map(|root| root.path)
        .collect()
}

/// The paths the preferences page lists.
fn page_paths(app: &App) -> Vec<String> {
    app.prefs_settings()
        .library_roots
        .iter()
        .map(|row| row.path.clone())
        .collect()
}

/// Fills the media library's database with `roots`.
fn fill_database(app: &App, roots: &[&Path]) {
    let path = app.library_database_path().expect("the database path");
    let mut index = Library::open(&path).expect("the database");
    for root in roots {
        index.add_root(root).expect("add the root");
    }
}

#[test]
fn the_page_lists_the_roots_the_library_database_holds() {
    let dir = TempDir::new().expect("temp dir");
    let mut app = prefs_app(&dir);
    fill_database(&app, &[Path::new(r"C:\Music"), Path::new(r"D:\Tunes")]);

    // The presenter reads the roots when the dialog opens.
    app.refresh_library_roots().expect("read the roots");
    assert_eq!(
        page_paths(&app),
        [r"C:\Music".to_string(), r"D:\Tunes".to_string()],
        "the page lists the roots of the library's database"
    );
}

#[test]
fn removing_a_root_through_the_page_forgets_it_in_the_database() {
    let dir = TempDir::new().expect("temp dir");
    let mut app = prefs_app(&dir);
    fill_database(&app, &[Path::new(r"C:\Music"), Path::new(r"D:\Tunes")]);
    app.refresh_library_roots().expect("read the roots");

    app.apply_prefs_action(PrefsAction::RemoveLibraryRoot(1));
    assert_eq!(
        page_paths(&app),
        [r"C:\Music".to_string()],
        "the page shows the root that is left"
    );
    assert_eq!(
        database_roots(&app),
        [PathBuf::from(r"C:\Music")],
        "the database does not hold the root any more"
    );
}

#[test]
fn removing_a_row_that_does_not_exist_changes_nothing() {
    let dir = TempDir::new().expect("temp dir");
    let mut app = prefs_app(&dir);
    fill_database(&app, &[Path::new(r"C:\Music")]);
    app.refresh_library_roots().expect("read the roots");

    app.apply_prefs_action(PrefsAction::RemoveLibraryRoot(7));
    assert_eq!(
        page_paths(&app),
        [r"C:\Music".to_string()],
        "a row that does not exist removes nothing"
    );
    assert_eq!(database_roots(&app), [PathBuf::from(r"C:\Music")]);
}

#[test]
fn the_add_button_asks_for_a_folder_and_the_folder_becomes_a_root() {
    let dir = TempDir::new().expect("temp dir");
    let mut app = prefs_app(&dir);

    // The button does not pick the folder itself: the presenter asks the
    // user for it, from the tick.
    app.apply_prefs_action(PrefsAction::AddLibraryRoot);
    assert!(
        app.take_library_folder_request(),
        "the presenter shows the folder dialog"
    );
    assert!(
        !app.take_library_folder_request(),
        "the request is taken exactly once"
    );

    // The rest of the path: the folder the user picked is registered as a
    // root and indexed.
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("media folder");
    app.library_add_folder(media.clone())
        .expect("add the folder");
    assert_eq!(
        page_paths(&app),
        [media.display().to_string()],
        "the page lists the folder that was added"
    );
    assert_eq!(database_roots(&app), [media]);
}

#[test]
fn the_roots_survive_a_restart() {
    let dir = TempDir::new().expect("temp dir");
    let mut app = prefs_app(&dir);
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("media folder");
    app.library_add_folder(media.clone())
        .expect("add the folder");

    // A second player with the same configuration file finds the root: it
    // lives in the database beside the configuration, not in the
    // configuration itself.
    let mut restarted = prefs_app(&dir);
    restarted.refresh_library_roots().expect("read the roots");
    assert_eq!(
        page_paths(&restarted),
        [media.display().to_string()],
        "the root is still there after a restart"
    );
}

#[test]
fn removing_a_root_through_the_page_forgets_its_tracks() {
    let dir = TempDir::new().expect("temp dir");
    let mut app = prefs_app(&dir);
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("media folder");
    std::fs::copy(fixture(), media.join("track.mp3")).expect("copy the fixture");

    app.library_add_folder(media.clone())
        .expect("add the folder");
    // The scan runs on the app's own thread: the test pumps the app until
    // the track is indexed.
    for _ in 0..600 {
        app.pump().expect("pump");
        if app.library_view().total() > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.library_view().total(), 1, "the track is indexed");

    app.apply_prefs_action(PrefsAction::RemoveLibraryRoot(0));
    let path = app.library_database_path().expect("the database path");
    let index = Library::open(&path).expect("the database");
    assert!(
        index.roots().expect("the roots").is_empty(),
        "the root is gone"
    );
    assert_eq!(
        index.count().expect("the tracks"),
        0,
        "the tracks indexed under the root are gone with it"
    );
}

#[test]
fn a_root_that_was_never_scanned_carries_no_time() {
    let dir = TempDir::new().expect("temp dir");
    let mut app = prefs_app(&dir);
    fill_database(&app, &[Path::new(r"C:\Music")]);
    app.refresh_library_roots().expect("read the roots");

    let roots = app.prefs_settings().library_roots;
    assert_eq!(roots.len(), 1, "the page has one row");
    assert_eq!(roots[0].path, r"C:\Music");
    assert_eq!(
        roots[0].last_scanned_at, None,
        "the root was never scanned, so it has no time"
    );
    assert_eq!(
        library_root_text(&roots[0].path, roots[0].last_scanned_at),
        r"C:\Music — never scanned",
        "the row says that the root was never scanned"
    );
}

#[test]
fn a_scanned_root_carries_the_time_of_its_last_scan() {
    let dir = TempDir::new().expect("temp dir");
    let mut app = prefs_app(&dir);
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("media folder");
    std::fs::copy(fixture(), media.join("track.mp3")).expect("copy the fixture");

    app.library_add_folder(media.clone())
        .expect("add the folder");
    // The scan runs on the app's own thread: the test pumps the app until
    // the track is indexed.
    for _ in 0..600 {
        app.pump().expect("pump");
        if app.library_view().total() > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.library_view().total(), 1, "the track is indexed");

    // The scan ran to the end, so the index recorded the time in the root's
    // row, and the page carries it.
    app.refresh_library_roots().expect("read the roots");
    let roots = app.prefs_settings().library_roots;
    assert_eq!(roots.len(), 1, "the page has one row");
    let at = roots[0]
        .last_scanned_at
        .expect("the finished scan recorded its time");
    assert!(at > 0, "the time is a unix second: {at}");

    let text = library_root_text(&roots[0].path, Some(at));
    assert!(
        text.starts_with(&format!("{} — last scanned ", media.display())),
        "the row names the path and the time: {text}"
    );
    assert!(text.ends_with(" UTC"), "the time is UTC: {text}");
}

#[test]
fn the_time_of_a_root_survives_a_restart() {
    let dir = TempDir::new().expect("temp dir");
    let mut app = prefs_app(&dir);
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("media folder");
    std::fs::copy(fixture(), media.join("track.mp3")).expect("copy the fixture");

    app.library_add_folder(media.clone())
        .expect("add the folder");
    for _ in 0..600 {
        app.pump().expect("pump");
        if app.library_view().total() > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.library_view().total(), 1, "the track is indexed");

    // The time lives in the database beside the configuration, so a second
    // player finds it.
    let mut restarted = prefs_app(&dir);
    restarted.refresh_library_roots().expect("read the roots");
    let roots = restarted.prefs_settings().library_roots;
    assert_eq!(roots.len(), 1, "the page has one row");
    assert!(
        roots[0].last_scanned_at.is_some(),
        "the time of the scan is still there"
    );
}
