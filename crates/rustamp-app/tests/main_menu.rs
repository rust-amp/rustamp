// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The main-menu popup wiring, end to end but **headless** (task card wave21b).
//!
//! GUI must never run on the host, so every test here drives [`App`] with the
//! null output and opens **no window** — except
//! [`the_main_menu_popup_opens_at_the_measured_client_size`], which is
//! `#[ignore]`d **and** refuses unless it can prove it runs as the sandbox
//! user `WDAGUtilityAccount`, exactly like `tests/eq_window.rs`. There is
//! deliberately **no** override. Run it only inside the Windows Sandbox:
//!
//! ```text
//! cargo test -p rustamp-app --test main_menu -- --ignored
//! ```
//!
//! What the headless tests pin:
//!
//! * the time rows are a real toggle wired to the existing UI-local flip
//!   (RFC 0005 §9): the check moves between *Time elapsed* and
//!   *Time remaining*;
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
fn the_time_rows_are_a_real_toggle() {
    let mut app = headless_app();
    assert!(!app.state().time_remaining);
    assert!(app.menu_check(MenuCheck::TimeElapsed));
    assert!(!app.menu_check(MenuCheck::TimeRemaining));

    let elapsed = top_level(MainMenuId::TimeElapsed).unwrap();
    let outcome = app
        .activate_menu_item(
            elapsed.label,
            match elapsed.action {
                rustamp_ui::MenuAction::Shortcut(action) => action,
                _ => panic!("the time row is a leaf"),
            },
        )
        .expect("activation submits");
    assert_eq!(outcome, MenuOutcome::TimeDisplay(true));
    assert!(app.state().time_remaining);
    assert!(!app.menu_check(MenuCheck::TimeElapsed));
    assert!(app.menu_check(MenuCheck::TimeRemaining));

    // The sibling row flips it back: the check moves between the two.
    let remaining = top_level(MainMenuId::TimeRemaining).unwrap();
    let outcome = app
        .activate_menu_item(
            remaining.label,
            match remaining.action {
                rustamp_ui::MenuAction::Shortcut(action) => action,
                _ => panic!("the time row is a leaf"),
            },
        )
        .expect("activation submits");
    assert_eq!(outcome, MenuOutcome::TimeDisplay(false));
    assert!(!app.state().time_remaining);
}

#[test]
fn the_window_checks_follow_what_is_actually_on() {
    let mut app = headless_app();
    assert!(app.menu_check(MenuCheck::MainWindow));
    assert!(!app.menu_check(MenuCheck::PlaylistEditor));
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
    assert!(app.ui().state().playlist_open);
    assert!(app.menu_check(MenuCheck::PlaylistEditor));

    let equalizer = top_level(MainMenuId::Equalizer).unwrap();
    app.activate_menu_item(equalizer.label, ShortcutAction::ToggleEqualizer)
        .expect("activation submits");
    assert!(app.menu_check(MenuCheck::Equalizer));
}

#[test]
fn dialog_leaves_keep_the_existing_not_implemented_note() {
    let mut app = headless_app();
    let preferences = submenu_entries(SubmenuId::Options)
        .iter()
        .find(|entry| entry.action == ShortcutAction::Preferences)
        .expect("Preferences is in the Options submenu");
    let outcome = app
        .activate_menu_item(preferences.label, preferences.action)
        .expect("activation submits");
    assert_eq!(outcome, MenuOutcome::Commands(vec![Command::OpenPrefs]));
    let notes = app.drain_notes();
    assert!(
        notes
            .iter()
            .any(|note| note.contains("OpenPrefs") && note.contains("not implemented yet")),
        "the existing dialog note is kept: {notes:?}"
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

/// The Windows Sandbox account. Everything inside the sandbox runs as this
/// user, which is the marker that proves a run is not on the host.
const SANDBOX_USER: &str = "WDAGUtilityAccount";

/// Refuses unless this process provably runs inside the Windows Sandbox.
///
/// There is deliberately **no** override: the test below opens the menu
/// popup, and an ignored test that can do that on the host is a loaded gun.
fn require_windows_sandbox() {
    let user = std::env::var("USERNAME").unwrap_or_default();
    assert!(
        user.eq_ignore_ascii_case(SANDBOX_USER),
        "refusing to open a GUI window: this test is sandbox-only, but the user is {user:?} \
         rather than {SANDBOX_USER:?}. GUI must never run on the host; run it inside the \
         Windows Sandbox (see crates/rustamp-app/README.md)."
    );
}

/// Opens the real main-menu popup and checks its client size.
///
/// The size is the task-card measurement (323 x 569, pending S04); the
/// position is probe-only (the popup anchor is unmeasured). It creates no
/// softbuffer surface and paints nothing; painting is covered by the headless
/// render tests.
#[test]
#[ignore = "sandbox-only: opens the main-menu popup; run inside the Windows Sandbox"]
fn the_main_menu_popup_opens_at_the_measured_client_size() {
    use std::sync::{Arc, Mutex};

    use winit::application::ApplicationHandler;
    use winit::dpi::{PhysicalPosition, PhysicalSize};
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::window::{Window, WindowId};

    // The gate, before any window exists.
    require_windows_sandbox();

    let (width, height) = rustamp_ui::menu::MAIN_MENU_CLIENT_SIZE;
    assert_eq!((width, height), (323, 569));

    struct Probe {
        seen: Arc<Mutex<Option<(u32, u32)>>>,
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
    let mut probe = Probe {
        seen: Arc::clone(&seen),
    };
    let event_loop = EventLoop::new().expect("create the event loop");
    event_loop.run_app(&mut probe).expect("run the event loop");
    assert_eq!(*seen.lock().unwrap(), Some((323, 569)));
}
