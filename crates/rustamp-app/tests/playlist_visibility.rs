// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Verification tests for BETA-07: playlist visibility and menu check marks
//! tell the truth in both directions.
//!
//! Pinned contract:
//! - Fresh launch defaults to playlist open, and the menu check is checked.
//! - Closing via skinned close button or native close box flips the model to
//!   closed, unchecks the menu mark, and does not quit the application.
//! - Toggling via PL button (left click or right click), shortcut (Alt+E), or
//!   menu activation opens/closes the window and keeps the check mark in sync.
//! - Restoring from configuration with `visible: false` starts with the
//!   playlist closed and unchecked.

use rustamp_app::app::{App, AppOptions};
use rustamp_core::command::Command;
use rustamp_core::config::{Config, WindowLayout};
use rustamp_core::state::WindowId;
use rustamp_ui::{MainMenuId, MenuAction, MenuCheck, MenuOutcome, ShortcutAction, Widget};

fn headless_app(config: Option<Config>) -> App {
    let mut options = AppOptions::headless();
    if let Some(cfg) = config {
        options.config = cfg;
    }
    App::new(options).expect("create headless app")
}

fn centre(widget: Widget) -> (i32, i32) {
    let layout = rustamp_ui::MainWindowLayout::default();
    let rect = layout.widget_rect(widget).expect("widget rect");
    (
        rect.x + rect.width as i32 / 2,
        rect.y + rect.height as i32 / 2,
    )
}

fn click(app: &mut App, widget: Widget) {
    let (x, y) = centre(widget);
    app.mouse_down(x, y).expect("mouse down");
    app.mouse_up(x, y).expect("mouse up");
}

#[test]
fn beta07_playlist_check_marks_and_visibility_tell_the_truth_in_both_directions() {
    let mut app = headless_app(None);

    // 1. Initial state: default config opens the playlist, check mark is checked.
    assert!(
        app.playlist_open(),
        "default launch must start with the playlist window open"
    );
    assert!(
        app.menu_check(MenuCheck::PlaylistEditor),
        "playlist menu check mark must be true when window is open"
    );

    // 2. Skinned title-bar close button: hides editor, unchecks menu mark, does not quit.
    let commands = app.playlist_mouse_down(264, 3).expect("playlist close");
    assert_eq!(
        commands,
        vec![Command::ToggleWindow(WindowId::Playlist)],
        "skinned close must emit ToggleWindow(Playlist)"
    );
    assert!(
        !app.playlist_open(),
        "playlist must be closed after title-bar close"
    );
    assert!(
        !app.menu_check(MenuCheck::PlaylistEditor),
        "playlist menu check mark must be false when window is closed"
    );
    assert!(!app.quit(), "playlist close must never quit the player");

    // 3. Main window PL toggle button: brings it back, checks menu mark.
    click(&mut app, Widget::PlaylistToggle);
    assert!(
        app.playlist_open(),
        "playlist must be open after clicking PL toggle button"
    );
    assert!(
        app.menu_check(MenuCheck::PlaylistEditor),
        "playlist menu check mark must be true after PL toggle"
    );

    // 4. Native OS close: close_playlist_window hides it, unchecks menu mark.
    let close_cmds = app.close_playlist_window().expect("close_playlist_window");
    assert_eq!(
        close_cmds,
        vec![Command::ToggleWindow(WindowId::Playlist)],
        "close_playlist_window must submit ToggleWindow(Playlist)"
    );
    assert!(
        !app.playlist_open(),
        "playlist must be closed after close_playlist_window"
    );
    assert!(
        !app.menu_check(MenuCheck::PlaylistEditor),
        "menu check must be unchecked after close_playlist_window"
    );

    // Closing when already closed is a harmless no-op.
    assert!(
        app.close_playlist_window()
            .expect("idempotent close")
            .is_empty(),
        "closing an already closed playlist must return no commands"
    );

    // 5. Right-click on PL button toggles window (S03 §5).
    let (pl_x, pl_y) = centre(Widget::PlaylistToggle);
    app.mouse_right_down(pl_x, pl_y).expect("right-click PL");
    assert!(
        app.playlist_open(),
        "right-click on PL toggle must open the playlist"
    );
    assert!(
        app.menu_check(MenuCheck::PlaylistEditor),
        "menu check must follow right-click toggle"
    );

    // 6. Main menu activation toggles window and flips check mark.
    let outcome = app
        .activate_menu_item("Playlist Editor", ShortcutAction::TogglePlaylistEditor)
        .expect("activate menu item");
    assert_eq!(
        outcome,
        MenuOutcome::Commands(vec![Command::ToggleWindow(WindowId::Playlist)])
    );
    assert!(
        !app.playlist_open(),
        "menu item toggle must close the playlist"
    );
    assert!(
        !app.menu_check(MenuCheck::PlaylistEditor),
        "menu check mark must be false after menu toggle"
    );

    // 7. Shortcut dispatch (Alt+E) toggles window back.
    let outcome = app
        .dispatch_shortcut(ShortcutAction::TogglePlaylistEditor)
        .expect("dispatch shortcut");
    assert_eq!(
        outcome,
        vec![Command::ToggleWindow(WindowId::Playlist)]
    );
    assert!(
        app.playlist_open(),
        "shortcut toggle must reopen the playlist"
    );
    assert!(
        app.menu_check(MenuCheck::PlaylistEditor),
        "menu check mark must be true after shortcut toggle"
    );
}

#[test]
fn beta07_configured_hidden_playlist_starts_closed_and_unchecked() {
    let mut config = Config::default();
    config.ui.windows.push(WindowLayout {
        id: WindowId::Playlist,
        x: 26,
        y: 261,
        width: 275,
        height: 116,
        visible: false,
        shaded: false,
    });

    let mut app = headless_app(Some(config));
    assert!(
        !app.playlist_open(),
        "configured hidden playlist must start closed"
    );
    assert!(
        !app.menu_check(MenuCheck::PlaylistEditor),
        "configured hidden playlist must start unchecked"
    );

    // Toggling via menu opens it and sets the check mark.
    let outcome = app
        .activate_menu_item("Playlist Editor", ShortcutAction::TogglePlaylistEditor)
        .expect("activate menu item");
    assert_eq!(
        outcome,
        MenuOutcome::Commands(vec![Command::ToggleWindow(WindowId::Playlist)])
    );
    assert!(
        app.playlist_open(),
        "toggling hidden playlist must open it"
    );
    assert!(
        app.menu_check(MenuCheck::PlaylistEditor),
        "menu check mark must become true"
    );
}
