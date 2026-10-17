// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The main-menu popup wiring, end to end but **headless** (task card wave21b).
//!
//! GUI must never run on the host, so every test here drives [`App`] with the
//! null output and opens **no window** — except
//! [`the_main_menu_popup_opens_at_the_measured_client_size`], which is
//! `#[ignore]`d **and** refuses unless it can prove it runs inside the
//! headless test VM, exactly like `tests/eq_window.rs`. There is
//! deliberately **no** override. Run it only inside the headless test VM:
//!
//! ```text
//! cargo test -p rustamp-app --test main_menu -- --ignored
//! ```
//!
//! What the headless tests pin:
//!
//! * the time toggle is wired to the existing UI-local flip (RFC 0005 §9):
//!   activating it flips the flag and the elapsed/remaining check pair
//!   (S04 §4.1) moves with it — those rows live in the Options popup, not on
//!   the main menu (S04 §2);
//! * the check state comes from live app state (the Equalizer/Playlist rows
//!   reflect what is actually on);
//! * leaves emit the app's existing commands and notes: dialog requests keep
//!   the "not implemented yet" note, and the note names the menu item when
//!   the action has no command yet.

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_app::menu::MenuOutcome;
use rustamp_core::command::Command;
use rustamp_core::config::Config;
use rustamp_ui::ShortcutAction;
use rustamp_ui::menu::{MainMenuId, MenuCheck, SubmenuId, submenu_entries, top_level};

/// A headless player: null output, built-in skin, nothing queued.
fn headless_app() -> App {
    let options = AppOptions {
        config: Config::default(),
        skin: rustamp_app::skin::builtin(),
        output: OutputChoice::Null,
        volume: 200,
        file: None,
        seconds: None,
    };
    App::new(options).expect("headless app builds")
}

#[test]
fn the_time_toggle_is_wired_to_the_ui_local_flip() {
    // The elapsed/remaining rows live in the Options popup (S04 §4.1), not
    // on the main menu (S04 §2), so there is no top-level time row to
    // activate: drive the same UI-local flip (RFC 0005 §9) directly and pin
    // that the check pair moves with it.
    let mut app = headless_app();
    assert!(!app.state().time_remaining);
    assert!(app.menu_check(MenuCheck::TimeElapsed));
    assert!(!app.menu_check(MenuCheck::TimeRemaining));

    let outcome = app
        .activate_menu_item("Time elapsed", ShortcutAction::ToggleTimeRemaining)
        .expect("activation submits");
    assert_eq!(outcome, MenuOutcome::TimeDisplay(true));
    assert!(app.state().time_remaining);
    assert!(!app.menu_check(MenuCheck::TimeElapsed));
    assert!(app.menu_check(MenuCheck::TimeRemaining));

    // Flipping back restores the elapsed check.
    let outcome = app
        .activate_menu_item("Time remaining", ShortcutAction::ToggleTimeRemaining)
        .expect("activation submits");
    assert_eq!(outcome, MenuOutcome::TimeDisplay(false));
    assert!(!app.state().time_remaining);
}

#[test]
fn the_window_checks_follow_what_is_actually_on() {
    let mut app = headless_app();
    assert!(app.menu_check(MenuCheck::MainWindow));
    // The presenter opens the playlist editor at launch, so it is checked
    // (BETA-07: the check used to say closed while the window was up).
    assert!(app.menu_check(MenuCheck::PlaylistEditor));
    assert!(!app.menu_check(MenuCheck::Equalizer));

    let editor = top_level(MainMenuId::PlaylistEditor).unwrap();
    let outcome = app
        .activate_menu_item(editor.label, ShortcutAction::TogglePlaylistEditor)
        .expect("activation submits");
    assert_eq!(
        outcome,
        MenuOutcome::Commands(vec![Command::ToggleWindow(
            rustamp_core::state::WindowId::Playlist
        )])
    );
    assert!(!app.ui().state().playlist_open);
    assert!(!app.menu_check(MenuCheck::PlaylistEditor));

    let equalizer = top_level(MainMenuId::Equalizer).unwrap();
    app.activate_menu_item(equalizer.label, ShortcutAction::ToggleEqualizer)
        .expect("activation submits");
    assert!(app.menu_check(MenuCheck::Equalizer));
}

#[test]
fn the_preferences_leaf_opens_the_dialog_instead_of_noting_it() {
    let mut app = headless_app();
    let preferences = submenu_entries(SubmenuId::Options)
        .iter()
        .find(|entry| entry.action == ShortcutAction::Preferences)
        .expect("Preferences is in the Options submenu");
    let outcome = app
        .activate_menu_item(preferences.label, preferences.action)
        .expect("activation submits");
    assert_eq!(outcome, MenuOutcome::Commands(vec![Command::OpenPrefs]));
    // The dialog really opens now, so the old "not implemented" note is gone.
    assert!(app.dialog_open_kind().is_some(), "a dialog is open");
    let notes = app.drain_notes();
    assert!(
        !notes.iter().any(|note| note.contains("OpenPrefs")),
        "an implemented dialog records no note: {notes:?}"
    );
}

#[test]
fn an_action_without_a_command_notes_the_item_by_name() {
    let mut app = headless_app();
    let outcome = app
        .activate_menu_item("New instance", ShortcutAction::NewInstance)
        .expect("activation submits");
    assert_eq!(outcome, MenuOutcome::Noted);
    let notes = app.drain_notes();
    assert_eq!(notes.len(), 1);
    assert!(
        notes[0].contains("New instance"),
        "the note names the item: {:?}",
        notes[0]
    );
    assert!(
        notes[0].contains("40305"),
        "the note keeps the id: {:?}",
        notes[0]
    );
}

// The shared test-VM gate lives in `rustamp_app::window::require_test_vm`:
// this file keeps no copy of it. There is deliberately **no** override:
// the test below opens the menu popup, and an ignored test that can do that
// on the host is a loaded gun.

/// Opens the real main-menu popup and checks its client size and position.
///
/// The size is the settled S04 measurement (323 x 569, S04 §1/§8); the
/// position is probe-only (the popup anchor is unmeasured, so the test asks
/// for (50,50) and checks the window opened there). It creates no
/// softbuffer surface and paints nothing; painting is covered by the headless
/// render tests.
///
/// The loop is built off the main thread on purpose: tests run on harness
/// worker threads, so `EventLoop::new()` would panic. See
/// [`rustamp_app::window::build_event_loop`].
#[test]
#[ignore = "VM-only: opens the main-menu popup; run inside the headless test VM"]
fn the_main_menu_popup_opens_at_the_measured_client_size() {
    use std::sync::{Arc, Mutex};

    use winit::application::ApplicationHandler;
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::event::WindowEvent;
    use winit::event_loop::ActiveEventLoop;
    use winit::window::{Window, WindowId};

    // The gate, before any window exists.
    rustamp_app::window::require_test_vm();

    let (width, height) = rustamp_ui::menu::MAIN_MENU_CLIENT_SIZE;
    assert_eq!((width, height), (323, 569));

    struct Probe {
        seen: Arc<Mutex<Option<(u32, u32)>>>,
        seen_pos: Arc<Mutex<Option<(i32, i32)>>>,
    }

    impl ApplicationHandler for Probe {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            let (w, h) = rustamp_ui::menu::MAIN_MENU_CLIENT_SIZE;
            let window: Window = event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("Rustamp Main Menu (ignored probe)")
                        .with_inner_size(PhysicalSize::new(w, h))
                        .with_position(PhysicalPosition::new(50, 50))
                        .with_resizable(false),
                )
                .expect("create the main-menu popup");
            let size = window.inner_size();
            *self.seen.lock().unwrap() = Some((size.width, size.height));
            let pos = window.outer_position().expect("read the popup position");
            *self.seen_pos.lock().unwrap() = Some((pos.x, pos.y));
            event_loop.exit();
        }

        fn window_event(
            &mut self,
            _event_loop: &ActiveEventLoop,
            _id: WindowId,
            _event: WindowEvent,
        ) {
        }
    }

    let seen = Arc::new(Mutex::new(None));
    let seen_pos = Arc::new(Mutex::new(None));
    let mut probe = Probe {
        seen: Arc::clone(&seen),
        seen_pos: Arc::clone(&seen_pos),
    };
    // Off the main thread on purpose: this test runs on a harness worker
    // thread, so `EventLoop::new()` would panic. The product keeps the
    // default; see `rustamp_app::window::build_event_loop`.
    let event_loop = rustamp_app::window::build_event_loop(true).expect("create the event loop");
    event_loop.run_app(&mut probe).expect("run the event loop");
    assert_eq!(*seen.lock().unwrap(), Some((323, 569)));
    assert_eq!(
        *seen_pos.lock().unwrap(),
        Some((50, 50)),
        "the popup must open where it was asked to (probe-only anchor)"
    );
}
