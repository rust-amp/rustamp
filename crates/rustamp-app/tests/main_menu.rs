// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The main-menu popup wiring, end to end but **headless** (task card wave21b).
//!
//! GUI must never run on the host, so every test here drives [`App`] with the
//! null output and opens **no window**. That is now the whole file: the menu
//! is the operating system's own popup (wave122), so there is no window of
//! ours left to measure, and the probe that used to check this popup's
//! 323 x 569 client size is gone with it. That probe is also the lesson: it
//! measured a size and passed while the popup carried a standard window
//! caption and drew unreadable text.
//!
//! What the headless tests pin:
//!
//! * the popup the player shows is a **native menu tree**, with the S04 §2
//!   row order, the four separators, the six submenu parents (which the OS
//!   draws an arrow for), the measured accelerators in their own column and
//!   the three checkable window rows following live state - and every id it
//!   can report maps back to exactly one action;
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

// ---------------------------------------------------------------------------
// The native popup (wave122)
// ---------------------------------------------------------------------------

/// The frame-and-font defect, pinned at the decision: the menu the player
/// shows is a native menu tree, not a window of ours.
///
/// Everything the reference screenshot shows has to be in the tree, because
/// the OS draws it from there: separator rows between the groups, a submenu
/// row for every parent (the OS draws the arrow), an accelerator carried
/// separately from the label (the OS right-aligns it) and a tick flag on the
/// toggles. The system font and the frameless popup shape come with the menu
/// and need no data at all.
///
/// What this cannot check: that the OS then draws it legibly. That needs a
/// desktop, so only the owner's machine can confirm it - but the unreadable
/// bitmap font is gone by construction, because no font of ours is used.
#[test]
fn the_main_menu_is_a_native_menu_tree_with_the_measured_shape() {
    use rustamp_platform::PopupRow;
    use rustamp_ui::menu::{MAIN_MENU_ROWS, MAIN_MENU_SEPARATORS, MAIN_MENU_SUBMENUS};

    let app = headless_app();
    let menu = app.native_main_menu();

    assert_eq!(
        menu.rows.len(),
        MAIN_MENU_ROWS,
        "every S04 §2 row reaches the popup"
    );
    let separators = menu
        .rows
        .iter()
        .filter(|row| matches!(row, PopupRow::Separator))
        .count();
    assert_eq!(
        separators, MAIN_MENU_SEPARATORS,
        "the four separator rules are separator rows, not drawn lines"
    );
    let submenus = menu
        .rows
        .iter()
        .filter(|row| matches!(row, PopupRow::Submenu { .. }))
        .count();
    assert_eq!(
        submenus, MAIN_MENU_SUBMENUS,
        "the six arrow rows are real submenus, so the OS draws the arrow and opens them"
    );
    // Every submenu has entries: the self-drawn popup could only note that a
    // submenu was deferred, because it had no way to draw one.
    for row in &menu.rows {
        if let PopupRow::Submenu { label, rows } = row {
            assert!(!rows.is_empty(), "{label} opens nothing");
        }
    }

    // The accelerators S04 §3.3/§3.4 measured are carried as accelerators,
    // separately from the label, which is what gets them right-aligned.
    let accelerators: Vec<(&str, &str)> = menu
        .rows
        .iter()
        .filter_map(|row| match row {
            PopupRow::Entry {
                label,
                accelerator: Some(accelerator),
                ..
            } => Some((label.as_str(), accelerator.as_str())),
            _ => None,
        })
        .collect();
    for (label, accelerator) in [
        ("View file info", "Alt+3"),
        ("Main Window", "Alt+W"),
        ("Playlist Editor", "Alt+E"),
        ("Equalizer", "Alt+G"),
        ("Library", "Alt+L"),
        ("Minibrowser", "Alt+T"),
        ("Video", "Alt+V"),
    ] {
        assert!(
            accelerators.contains(&(label, accelerator)),
            "{label} must carry {accelerator}: {accelerators:?}"
        );
    }
    assert!(
        menu.rows.iter().all(|row| match row {
            PopupRow::Entry { label, .. } => !label.contains('\t'),
            _ => true,
        }),
        "the label must not carry the accelerator itself"
    );
}

/// The tick marks are the OS's, and they follow live state.
#[test]
fn the_native_menus_tick_marks_follow_the_open_windows() {
    use rustamp_platform::PopupRow;

    let checked = |menu: &rustamp_ui::menu::NativeMainMenu, wanted: &str| {
        menu.rows.iter().any(|row| match row {
            PopupRow::Entry { label, checked, .. } => label == wanted && *checked,
            _ => false,
        })
    };

    let mut app = headless_app();
    let equalizer = top_level(MainMenuId::Equalizer).unwrap();
    assert!(
        !checked(&app.native_main_menu(), equalizer.label),
        "the equalizer starts closed, so its row carries no tick"
    );
    app.activate_menu_item(equalizer.label, ShortcutAction::ToggleEqualizer)
        .expect("activation submits");
    assert!(
        checked(&app.native_main_menu(), equalizer.label),
        "opening the equalizer ticks its row"
    );
}

/// The ids the OS reports map back to exactly one action each, and nothing
/// else maps at all. A menu that mis-maps one id runs the wrong action, which
/// is the one way this path can be silently wrong.
#[test]
fn every_id_the_native_menu_can_report_maps_back_to_one_action() {
    use rustamp_platform::PopupRow;
    use rustamp_ui::menu::MenuChoice;

    let app = headless_app();
    let menu = app.native_main_menu();

    fn ids(rows: &[PopupRow], into: &mut Vec<(u32, String)>) {
        for row in rows {
            match row {
                PopupRow::Entry { id, label, .. } => into.push((*id, label.clone())),
                PopupRow::Submenu { rows, .. } => ids(rows, into),
                PopupRow::Separator => {}
            }
        }
    }
    let mut reported = Vec::new();
    ids(&menu.rows, &mut reported);
    assert_eq!(
        reported.len(),
        menu.choices.len(),
        "one choice per selectable entry"
    );
    for (id, label) in &reported {
        assert_ne!(*id, 0, "{label}: zero is reserved for a dismissed menu");
        let choice = menu
            .choice_for_id(*id)
            .unwrap_or_else(|| panic!("{label}: id {id} maps to nothing"));
        assert_eq!(
            choice.label(),
            label.as_str(),
            "id {id} maps to the wrong row"
        );
    }
    // Out-of-range ids are not choices.
    assert_eq!(menu.choice_for_id(0), None);
    assert_eq!(menu.choice_for_id(menu.choices.len() as u32 + 1), None);

    // `Exit` quits the player.
    let exit = menu
        .choices
        .iter()
        .find(|choice| choice.label() == "Exit")
        .copied()
        .expect("Exit is a measured row");
    assert!(matches!(exit, MenuChoice::Exit));
    let mut app = headless_app();
    assert_eq!(
        app.activate_menu_choice(exit).expect("quit dispatches"),
        Some(MenuOutcome::Commands(vec![Command::Quit]))
    );
    assert!(app.quit(), "choosing Exit must quit the player");
    assert!(app.drain_notes().is_empty());
}

/// A chosen leaf runs the same dispatch the matching key press runs - the
/// native path reuses the existing activation, it does not add a second one.
#[test]
fn a_chosen_native_leaf_runs_the_existing_key_dispatch() {
    use rustamp_ui::menu::MenuChoice;

    let mut app = headless_app();
    let outcome = app
        .activate_menu_choice(MenuChoice::Action {
            label: "View file info",
            action: ShortcutAction::ViewFileInfo,
        })
        .expect("activation submits");
    assert!(matches!(outcome, Some(MenuOutcome::Commands(_))));
    assert!(app.dialog_is_open(rustamp_ui::DialogKind::FileInfo));
}
