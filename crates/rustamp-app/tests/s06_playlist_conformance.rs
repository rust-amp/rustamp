// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! S06 conformance at the app seam: the rules the model alone cannot carry.
//!
//! `rustamp-ui`'s `s06_playlist_conformance.rs` pins the model and the
//! renderer. The cases here pin what only the app can enforce: that the
//! settled click and arrow points **reach** the model, that the buttons
//! **open** the dialogs and the popup the spec names, and that the removal
//! reaches the engine queue.
//!
//! The spec's facts under test (S06 §2, §3, §4, §5):
//!
//! * §2 - the single click at (60,28) selects the first visible row; SEL at
//!   (74,92) selects all visible rows.
//! * §3 - the arrows at (263,81) and (263,87) move the view by one page of
//!   four rows.
//! * §4 - ADD at (23,92) opens "Add file(s) to playlist", LIST OPTS at
//!   (242,92) opens "Load Playlist", MISC at (110,92) opens the `#32768`
//!   popup.
//! * §5 - REM at (46,92) removes the staged entry, with no menu choice
//!   required, and leaves the list renumbered.
//!
//! No window is opened: the app runs on the null output.

use std::path::PathBuf;

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_core::command::{Command, OpenMediaKind, SelectOp};
use rustamp_core::config::Config;
use rustamp_core::playlist::{EntryId, MediaLocation};
use rustamp_ui::{DialogKind, Modifiers};

/// The headless app the cases drive (null output, no window).
fn test_app() -> App {
    let options = AppOptions {
        config: Config::default(),
        skin: rustamp_app::skin::builtin(),
        output: OutputChoice::Null,
        volume: 200,
        file: None,
        seconds: None,
    };
    App::new(options).expect("the test app builds headless")
}

/// Grows the engine queue (no decoding: the queue only stores) so the app
/// assigns the stable entry ids the playlist shows.
fn add_entries(app: &mut App, n: u32) {
    let items = (0..n)
        .map(|i| MediaLocation::File(PathBuf::from(format!("track{i}.mp3"))))
        .collect::<Vec<_>>();
    app.submit(Command::AddItems {
        items,
        at: None,
        play: false,
    })
    .expect("AddItems applies");
}

/// S06 §2: the settled click at (60,28) reaches the model and selects
/// exactly one row - the first visible one.
#[test]
fn s06_section_2_the_settled_click_selects_one_row_through_the_app() {
    let mut app = test_app();
    add_entries(&mut app, 60);
    let commands = app
        .playlist_mouse_down(60, 28, Modifiers::NONE)
        .expect("the click applies");
    assert_eq!(
        commands,
        vec![Command::Select(SelectOp::Set(vec![EntryId(0)]))]
    );
    assert_eq!(app.playlist().selected_ids(), vec![EntryId(0)]);
    assert_eq!(app.playlist().selected_ids().len(), 1);
    assert!(app.drain_notes().is_empty());
}

/// S06 §2: SEL at (74,92) selects exactly the visible rows - the four rows of
/// the page, not the whole list.
#[test]
fn s06_section_2_sel_selects_the_visible_rows_through_the_app() {
    let mut app = test_app();
    add_entries(&mut app, 60);
    let commands = app
        .playlist_mouse_down(74, 92, Modifiers::NONE)
        .expect("SEL applies");
    let visible = vec![EntryId(0), EntryId(1), EntryId(2), EntryId(3)];
    assert_eq!(
        commands,
        vec![Command::Select(SelectOp::Set(visible.clone()))]
    );
    assert_eq!(app.playlist().selected_ids(), visible);
    assert_eq!(app.playlist().selected_ids().len(), 4);
    assert_ne!(app.playlist().selected_ids().len(), app.playlist().len());
    assert!(app.drain_notes().is_empty());
}

/// S06 §3: the settled arrow points move the view by one page of four rows,
/// at the top, in the middle and at the end of the list.
#[test]
fn s06_section_3_the_settled_arrow_points_page_the_view() {
    let mut app = test_app();
    add_entries(&mut app, 60);
    // The down arrow at (263,87): one page.
    app.playlist_mouse_down(263, 87, Modifiers::NONE)
        .expect("the down arrow applies");
    assert_eq!(app.playlist().first_visible(), 4);
    // The up arrow at (263,81): one page back.
    app.playlist_mouse_down(263, 81, Modifiers::NONE)
        .expect("the up arrow applies");
    assert_eq!(app.playlist().first_visible(), 0);
    // In the middle: seven pages down is row 29 (the spec's own reading of
    // the first visible row), and one page up is row 25.
    for _ in 0..7 {
        app.playlist_mouse_down(263, 87, Modifiers::NONE)
            .expect("the down arrow applies");
    }
    assert_eq!(app.playlist().first_visible(), 28);
    app.playlist_mouse_down(263, 81, Modifiers::NONE)
        .expect("the up arrow applies");
    assert_eq!(app.playlist().first_visible(), 24);
    // At the end of the list: the down arrow is clamped at the bottom page.
    for _ in 0..30 {
        app.playlist_mouse_down(263, 87, Modifiers::NONE)
            .expect("the down arrow applies");
    }
    assert_eq!(app.playlist().first_visible(), 56);
    assert_eq!(app.playlist().visible_ids().len(), 4);
    assert!(app.drain_notes().is_empty());
}

/// S06 §4: ADD opens the **"Add file(s) to playlist"** dialog and LIST OPTS
/// opens **"Load Playlist"** - the settled titles, read from the reference's
/// window metadata.
#[test]
fn s06_section_4_add_and_list_opts_open_the_settled_dialogs() {
    let mut app = test_app();
    add_entries(&mut app, 3);
    // ADD at (23,92).
    let commands = app
        .playlist_mouse_down(23, 92, Modifiers::NONE)
        .expect("ADD applies");
    assert_eq!(
        commands,
        vec![Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        }]
    );
    let dialog = app
        .dialog_open_kind()
        .expect("the add dialog is open (S06 §4)");
    assert_eq!(
        dialog,
        DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        }
    );
    assert_eq!(dialog.title(), "Add file(s) to playlist");

    // LIST OPTS at (242,92).
    let commands = app
        .playlist_mouse_down(242, 92, Modifiers::NONE)
        .expect("LIST OPTS applies");
    assert_eq!(commands, vec![Command::OpenPlaylist]);
    let dialog = app
        .dialog_open_kind()
        .expect("the load-playlist dialog is open (S06 §4)");
    assert_eq!(dialog, DialogKind::LoadPlaylist);
    assert_eq!(dialog.title(), "Load Playlist");
    assert!(app.drain_notes().is_empty());
}

/// S06 §4: MISC opens the **`#32768` popup** - a request the presenter
/// takes, not a command the player sends.
#[test]
fn s06_section_4_misc_requests_the_popup() {
    let mut app = test_app();
    add_entries(&mut app, 3);
    assert!(!app.take_playlist_misc_menu(), "no popup is pending");
    let commands = app
        .playlist_mouse_down(110, 92, Modifiers::NONE)
        .expect("MISC applies");
    assert!(commands.is_empty(), "the popup is not a command");
    assert!(app.take_playlist_misc_menu(), "the popup was requested");
    assert!(!app.take_playlist_misc_menu(), "the request is taken once");
    assert!(app.drain_notes().is_empty());
}

/// S06 §5: REM at (46,92) removes the staged entry, **with no menu choice
/// required** (§5.2), and the list renumbers after it (§5.4).
#[test]
fn s06_section_5_rem_removes_the_staged_entry_without_a_menu() {
    let mut app = test_app();
    add_entries(&mut app, 60);
    // One row staged (S06 §5.3's staged selection).
    app.playlist_mouse_down(60, 28, Modifiers::NONE)
        .expect("the click applies");
    assert_eq!(app.playlist().selected_ids(), vec![EntryId(0)]);

    // REM at (46,92).
    let commands = app
        .playlist_mouse_down(46, 92, Modifiers::NONE)
        .expect("REM applies");
    assert_eq!(commands, vec![Command::Remove(vec![EntryId(0)])]);
    // No popup and no dialog were involved in the removal (§5.2).
    assert!(!app.take_playlist_misc_menu());
    assert_eq!(app.dialog_open_kind(), None);

    // The after-state: the queue, the id table and the model all lost the
    // entry, the selection is gone and the list renumbered.
    assert_eq!(app.engine().queue().len(), 59);
    assert_eq!(app.playlist().len(), 59);
    assert!(app.playlist().selected_ids().is_empty());
    assert_eq!(
        app.playlist().visible_ids(),
        vec![EntryId(1), EntryId(2), EntryId(3), EntryId(4)],
        "the rows below the removed one shift up"
    );
    assert!(app.drain_notes().is_empty());
}

/// S06 §5.3: without a staged selection the button is a no-op - the
/// reference's unstaged REM click changed **0 px**.
#[test]
fn s06_section_5_rem_without_a_staged_selection_changes_nothing() {
    let mut app = test_app();
    add_entries(&mut app, 60);
    let commands = app
        .playlist_mouse_down(46, 92, Modifiers::NONE)
        .expect("REM applies");
    assert!(commands.is_empty());
    assert_eq!(app.engine().queue().len(), 60);
    assert_eq!(app.playlist().len(), 60);
    assert!(app.drain_notes().is_empty());
}
