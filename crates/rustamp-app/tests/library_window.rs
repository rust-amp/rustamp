// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The media library window: the command that opens it, the glue between the
//! view and the index, the input map, and the path from `Enter` to playback.
//!
//! Everything here is headless: null output, the built-in skin, no OS window
//! and no child process. What needs a desktop - the window itself, the
//! native folder dialog and the drawn frame on the screen - stays VM-only.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_app::library_window::{
    LIBRARY_CLIENT_SIZE, LIBRARY_DATABASE_NAME, LIBRARY_WINDOW_ID, LIBRARY_WINDOW_NUMBER,
};
use rustamp_audio::TrackSource;
use rustamp_core::command::Command;
use rustamp_core::config::Config;
use rustamp_core::meta::TrackMetadata;
use rustamp_core::playlist::MediaLocation;
use rustamp_gfx::Rect;
use rustamp_ui::library_render::{LIBRARY_CELL_WIDTH, LIBRARY_LINE_HEIGHT};
use rustamp_ui::{Key, LibraryChrome, LibrarySort, Modifiers, ShortcutOutcome};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The media fixture the library indexes: a real MP3 with tags.
fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rustamp-decoders/tests/fixtures")
        .join("sine.mp3")
}

/// A temp directory with `count` tagged tracks, and the folder that holds
/// them.
fn media_library(count: usize) -> (TempDir, PathBuf) {
    let dir = TempDir::new().expect("temp dir");
    let media = dir.path().join("media");
    std::fs::create_dir_all(&media).expect("media folder");
    for index in 0..count {
        let path = media.join(format!("track{index}.mp3"));
        std::fs::copy(fixture(), &path).expect("copy the fixture");
        // The fixture carries tags of its own; the test needs rows it can
        // tell apart, so every copy gets its own title.
        rustamp_meta::write_tags(
            &path,
            &TrackMetadata {
                title: Some(format!("Track {index}")),
                artist: Some("Rustamp".into()),
                album: Some("Tests".into()),
                ..TrackMetadata::default()
            },
        )
        .expect("write the tags");
    }
    (dir, media)
}

/// A headless player whose configuration lives in `dir`.
fn library_app(dir: &TempDir) -> App {
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

/// The chrome of the library window, as the app draws it.
fn chrome(app: &App) -> LibraryChrome {
    let (width, height) = app.library_client_size();
    LibraryChrome::new(app.library_view(), Rect::new(0, 0, width, height))
}

/// Adds `media` to the library and waits for the background scan to finish.
///
/// The scan runs on the app's own thread, so the test pumps the app until
/// the rows are there.
fn scan(app: &mut App, media: &Path) {
    app.library_add_folder(media.to_path_buf())
        .expect("add the folder");
    wait_for_rows(app);
}

/// Pumps the app until the running scan has committed its rows.
fn wait_for_rows(app: &mut App) {
    for _ in 0..600 {
        app.pump().expect("pump");
        if app.library_view().total() > 0 {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the library scan did not finish");
}

/// Writes a row whose path the file system refuses to name.
///
/// The path holds a NUL byte, so the file system cannot even answer whether
/// the file is there: such a row is one the removal must keep, and report.
/// No scan can produce such a path, so the row is written straight into the
/// index's own database.
fn insert_unreadable_row(database: &Path, path: &Path) {
    let connection = rusqlite::Connection::open(database).expect("open the library database");
    connection
        .execute(
            "INSERT INTO tracks (path, title) VALUES (?1, ?2)",
            rusqlite::params![path.to_string_lossy().to_string(), "Odd"],
        )
        .expect("insert the row");
}

/// A player with an open library window and an indexed folder.
fn library_with_tracks(count: usize) -> (TempDir, PathBuf, App) {
    let (dir, media) = media_library(count);
    let mut app = library_app(&dir);
    app.submit(Command::ToggleWindow(LIBRARY_WINDOW_ID))
        .expect("open the library");
    scan(&mut app, &media);
    (dir, media, app)
}

// ---------------------------------------------------------------------------
// The command that opens the window
// ---------------------------------------------------------------------------

#[test]
fn the_library_command_opens_and_closes_the_window() {
    let dir = TempDir::new().expect("temp dir");
    let mut app = library_app(&dir);
    assert!(!app.library_open(), "the window starts closed");

    app.submit(Command::ToggleWindow(LIBRARY_WINDOW_ID))
        .expect("open the library");
    assert!(app.library_open(), "the command opens the window");

    // The index lives beside the configuration file.
    let database = dir.path().join(LIBRARY_DATABASE_NAME);
    assert_eq!(app.library_database_path(), Some(database.clone()));
    assert!(
        database.is_file(),
        "the index must be created beside the configuration file"
    );

    app.submit(Command::ToggleWindow(LIBRARY_WINDOW_ID))
        .expect("close the library");
    assert!(!app.library_open(), "the command closes the window again");
    assert!(
        !app.quit(),
        "the library is not the main window: it never quits"
    );
}

#[test]
fn the_window_id_is_a_generic_window_of_its_own() {
    // The `Gen` range is the one `WindowId` reserves for the library; the
    // number is the app's own choice (see `rustamp_app::library_window`).
    assert_eq!(
        LIBRARY_WINDOW_ID,
        rustamp_core::state::WindowId::Gen(LIBRARY_WINDOW_NUMBER)
    );
    assert_ne!(LIBRARY_WINDOW_NUMBER, 0);
}

#[test]
fn alt_l_opens_the_library() {
    // S04 §3.3 gives the `Library` menu row the accelerator `Alt+L`.
    let dir = TempDir::new().expect("temp dir");
    let mut app = library_app(&dir);
    let outcome = app
        .key_down(Key::Letter(b'L'), Modifiers::ALT)
        .expect("Alt+L");
    assert_eq!(
        outcome,
        Some(ShortcutOutcome::Commands(vec![Command::ToggleWindow(
            LIBRARY_WINDOW_ID
        )]))
    );
    assert!(app.library_open());
}

// ---------------------------------------------------------------------------
// The glue: request -> query -> page
// ---------------------------------------------------------------------------

#[test]
fn the_request_becomes_a_query_and_the_rows_become_the_page() {
    let (_dir, _media, app) = library_with_tracks(3);
    let view = app.library_view();

    assert_eq!(view.total(), 3, "the index holds the scanned tracks");
    assert_eq!(view.rows().len(), 3, "the page holds the visible rows");
    assert_eq!(view.rows()[0].title, "Track 0");
    assert_eq!(view.rows()[0].artist, "Rustamp");
    assert_eq!(view.rows()[0].album, "Tests");

    // The request is scroll-aligned: it asks for the rows at the scroll, and
    // its limit is the number of rows the window shows.
    let request = view.request();
    assert_eq!(request.offset as usize, view.scroll());
    assert_eq!(request.limit as usize, view.viewport());
    assert!(
        request.limit as usize >= 3,
        "the window shows the three rows"
    );
    assert_eq!(request.sort, LibrarySort::Id);
    assert!(!request.descending);
    assert_eq!(request.text, None);
}

#[test]
fn the_search_field_narrows_the_match_set() {
    let (_dir, _media, mut app) = library_with_tracks(3);
    app.library_text("Track 1")
        .expect("type into the search field");
    let view = app.library_view();
    assert_eq!(view.text(), "Track 1", "the field holds the text as typed");
    assert_eq!(view.total(), 1, "only the matching row is left");
    assert_eq!(view.rows()[0].title, "Track 1");

    // Backspace deletes the last character, and the match set grows again.
    app.library_key_down(Key::Backspace, Modifiers::NONE)
        .expect("backspace");
    assert_eq!(app.library_view().text(), "Track ");
    assert_eq!(app.library_view().total(), 3, "every row matches again");
}

#[test]
fn the_text_only_reaches_the_field_that_has_the_focus() {
    let (_dir, _media, mut app) = library_with_tracks(2);
    // The field has the focus when the window opens.
    app.library_text("a").expect("type");
    assert_eq!(app.library_view().text(), "a");

    // Tab moves the focus to the list: the text goes nowhere now.
    app.library_key_down(Key::Tab, Modifiers::NONE)
        .expect("tab");
    app.library_text("b").expect("type");
    assert_eq!(
        app.library_view().text(),
        "a",
        "the list does not take text"
    );
}

// ---------------------------------------------------------------------------
// The input map
// ---------------------------------------------------------------------------

#[test]
fn the_movement_keys_move_the_selection() {
    let (_dir, _media, mut app) = library_with_tracks(3);
    // With no selection the movement starts at the visible rows.
    app.library_key_down(Key::Down, Modifiers::NONE)
        .expect("down");
    assert_eq!(app.library_view().selected_index(), Some(0));

    app.library_key_down(Key::Down, Modifiers::NONE)
        .expect("down");
    assert_eq!(app.library_view().selected_index(), Some(1));

    app.library_key_down(Key::Up, Modifiers::NONE).expect("up");
    assert_eq!(app.library_view().selected_index(), Some(0));

    app.library_key_down(Key::End, Modifiers::NONE)
        .expect("end");
    assert_eq!(app.library_view().selected_index(), Some(2));

    app.library_key_down(Key::Home, Modifiers::NONE)
        .expect("home");
    assert_eq!(app.library_view().selected_index(), Some(0));
}

#[test]
fn a_click_on_a_row_selects_it() {
    let (_dir, _media, mut app) = library_with_tracks(3);
    let chrome = chrome(&app);
    // The second row of the list.
    let y = chrome.rows.y + LIBRARY_LINE_HEIGHT as i32 + 2;
    app.library_click(chrome.rows.x + 4, y)
        .expect("click a row");
    assert_eq!(app.library_view().selected_index(), Some(1));
}

#[test]
fn a_click_on_a_column_header_sorts_by_that_column() {
    let (_dir, _media, mut app) = library_with_tracks(3);
    let chrome = chrome(&app);
    // The layout is sized to the list band, so the columns are where the
    // app draws them.
    let layout =
        rustamp_ui::LibraryLayout::default().resized_to(chrome.rows.width / LIBRARY_CELL_WIDTH);
    let x = (layout.column_x(rustamp_ui::LibraryColumn::Artist) * LIBRARY_CELL_WIDTH) as i32 + 2;
    app.library_click(x, chrome.header.y + 2)
        .expect("click the header");
    assert_eq!(app.library_view().sort(), LibrarySort::Artist);
    assert!(
        !app.library_view().descending(),
        "a new column sorts ascending"
    );

    // The same column again reverses the order.
    app.library_click(x, chrome.header.y + 2)
        .expect("click the header");
    assert_eq!(app.library_view().sort(), LibrarySort::Artist);
    assert!(app.library_view().descending());
}

#[test]
fn a_click_in_the_scrollbar_scrolls_to_the_place_it_sits_at() {
    let (_dir, _media, mut app) = library_with_tracks(3);
    let chrome = chrome(&app);
    // The track of a match set that fits into the window has no scroll, so
    // the click asks for the first row.
    app.library_click(
        chrome.scrollbar.x + 2,
        chrome.scrollbar.y + chrome.scrollbar.height as i32 - 1,
    )
    .expect("click the scrollbar");
    assert_eq!(app.library_view().scroll(), 0);
}

#[test]
fn ctrl_o_asks_for_a_folder_to_add() {
    let (_dir, _media, mut app) = library_with_tracks(1);
    app.library_key_down(Key::Letter(b'O'), Modifiers::CTRL)
        .expect("ctrl+o");
    assert!(
        app.take_library_folder_request(),
        "the presenter shows the folder dialog"
    );
    assert!(
        !app.take_library_folder_request(),
        "the request is taken exactly once"
    );
}

#[test]
fn ctrl_f_focuses_the_search_field() {
    let (_dir, _media, mut app) = library_with_tracks(1);
    // The field has the focus when the window opens; Tab takes it away.
    app.library_key_down(Key::Tab, Modifiers::NONE)
        .expect("tab");
    app.library_text("x").expect("type");
    assert_eq!(app.library_view().text(), "");
    app.library_key_down(Key::Letter(b'F'), Modifiers::CTRL)
        .expect("ctrl+f");
    app.library_text("x").expect("type");
    assert_eq!(app.library_view().text(), "x");
}

// ---------------------------------------------------------------------------
// Enter plays the selected row
// ---------------------------------------------------------------------------

#[test]
fn enter_plays_the_selected_row_through_the_add_items_path() {
    let (_dir, media, mut app) = library_with_tracks(2);
    app.library_key_down(Key::Down, Modifiers::NONE)
        .expect("select the first row");
    let selected = app
        .library_view()
        .selected_row()
        .expect("a selected row")
        .path
        .clone();

    let commands = app
        .library_key_down(Key::Enter, Modifiers::NONE)
        .expect("enter");
    assert_eq!(
        commands,
        vec![Command::AddItems {
            items: vec![MediaLocation::File(selected.clone())],
            at: None,
            play: true,
        }],
        "Enter plays the selected row through the existing command"
    );

    // The command went through the app's own path: the track is queued.
    assert_eq!(
        app.engine().queue(),
        &[TrackSource::path(selected.clone())],
        "the track is in the queue"
    );
    assert_eq!(
        selected,
        media.join("track0.mp3"),
        "the selected row is the file the index holds"
    );
}

#[test]
fn enter_without_a_selection_plays_nothing() {
    let (_dir, _media, mut app) = library_with_tracks(2);
    let commands = app
        .library_key_down(Key::Enter, Modifiers::NONE)
        .expect("enter");
    assert!(
        commands.is_empty(),
        "nothing is selected, so nothing is played"
    );
    assert!(app.engine().queue().is_empty());
}

// ---------------------------------------------------------------------------
// Ctrl+M removes the rows whose file is gone
// ---------------------------------------------------------------------------

#[test]
fn ctrl_m_removes_the_row_whose_file_is_gone() {
    let (_dir, media, mut app) = library_with_tracks(2);
    assert_eq!(app.library_view().total(), 2, "the index holds both tracks");

    // The file of the second track is gone: the index still lists it.
    std::fs::remove_file(media.join("track1.mp3")).expect("remove the track");

    app.library_key_down(Key::Letter(b'M'), Modifiers::CTRL)
        .expect("ctrl+m");

    // The view is reloaded: the row of the gone file is not in the list.
    let view = app.library_view();
    assert_eq!(view.total(), 1, "the row of the gone file is gone");
    assert_eq!(view.rows().len(), 1, "the page holds the row that is left");
    assert_eq!(
        view.rows()[0].path,
        media.join("track0.mp3"),
        "the row that is left is the track that is still there"
    );

    // The report is a note.
    let notes = app.drain_notes();
    assert!(
        notes
            .iter()
            .any(|note| note.contains("removed 1 of 2 entries")),
        "the report is a note: {notes:?}"
    );
}

#[test]
fn ctrl_m_reports_a_path_it_cannot_check_and_keeps_the_row() {
    let (dir, media, mut app) = library_with_tracks(1);
    // A row whose path the system refuses to name: the file system cannot
    // even answer whether the file is there, so the row is not a dead row.
    let odd = media.join("odd\0name.mp3");
    insert_unreadable_row(&dir.path().join(LIBRARY_DATABASE_NAME), &odd);

    app.library_key_down(Key::Letter(b'M'), Modifiers::CTRL)
        .expect("ctrl+m");

    let notes = app.drain_notes();
    assert!(
        notes
            .iter()
            .any(|note| note.contains("could not be checked") && note.contains("odd")),
        "the path that could not be checked is reported: {notes:?}"
    );

    // The row is kept: a path that cannot be checked is not a dead path.
    let view = app.library_view();
    assert_eq!(view.total(), 2, "the row that could not be checked stays");
    assert!(
        view.rows().iter().any(|row| row.path == odd),
        "the row that could not be checked is still in the list"
    );
}

#[test]
fn ctrl_m_refuses_while_a_scan_is_running() {
    let (dir, media) = media_library(2);
    let mut app = library_app(&dir);
    app.submit(Command::ToggleWindow(LIBRARY_WINDOW_ID))
        .expect("open the library");

    // The scan holds the index: the action must not run beside it.
    app.library_add_folder(media.clone())
        .expect("add the folder");
    app.library_key_down(Key::Letter(b'M'), Modifiers::CTRL)
        .expect("ctrl+m");

    let notes = app.drain_notes();
    assert!(
        notes
            .iter()
            .any(|note| note.contains("a scan is still running")),
        "the action refuses while the scan runs: {notes:?}"
    );

    // The scan is over: the action works then.
    wait_for_rows(&mut app);
    std::fs::remove_file(media.join("track1.mp3")).expect("remove the track");
    app.library_key_down(Key::Letter(b'M'), Modifiers::CTRL)
        .expect("ctrl+m");

    let notes = app.drain_notes();
    assert!(
        notes
            .iter()
            .any(|note| note.contains("removed 1 of 2 entries")),
        "the action runs once the scan is done: {notes:?}"
    );
    assert_eq!(
        app.library_view().total(),
        1,
        "the row of the gone file is gone"
    );
}

#[test]
fn ctrl_m_on_an_empty_library_is_a_no_op_with_a_note() {
    let dir = TempDir::new().expect("temp dir");
    let mut app = library_app(&dir);
    app.submit(Command::ToggleWindow(LIBRARY_WINDOW_ID))
        .expect("open the library");

    app.library_key_down(Key::Letter(b'M'), Modifiers::CTRL)
        .expect("ctrl+m");

    let notes = app.drain_notes();
    assert!(
        notes.iter().any(|note| note.contains("nothing to remove")),
        "the empty library is a no-op with a note: {notes:?}"
    );
    assert_eq!(app.library_view().total(), 0, "the list stays empty");
}

#[test]
fn the_plain_letter_removes_nothing() {
    let (_dir, media, mut app) = library_with_tracks(2);
    std::fs::remove_file(media.join("track1.mp3")).expect("remove the track");

    app.library_key_down(Key::Letter(b'M'), Modifiers::NONE)
        .expect("m");

    assert_eq!(
        app.library_view().total(),
        2,
        "the letter without the modifier removes nothing"
    );
    assert!(app.drain_notes().is_empty(), "the letter notes nothing");
}

// ---------------------------------------------------------------------------
// The window's own frame
// ---------------------------------------------------------------------------

#[test]
fn the_window_renders_a_frame_of_its_client_size() {
    let dir = TempDir::new().expect("temp dir");
    let mut app = library_app(&dir);
    let image = app.render_library();
    assert_eq!((image.width(), image.height()), LIBRARY_CLIENT_SIZE);
}

#[test]
fn the_window_title_names_the_library() {
    let dir = TempDir::new().expect("temp dir");
    let app = library_app(&dir);
    assert_eq!(app.library_title(), "Rustamp Library");
}
