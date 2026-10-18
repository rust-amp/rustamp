// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! BETA-07 regressions through the presenter's headless App entry points.
//!
//! Null output, built-in skin, no files, no child process and no OS window.
//! These tests pin the model, commands and live menu checks, not HWND lifetime
//! or the presenter's native-event routing. Execute only in the test VM.

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_app::config::parse_with_warnings;
use rustamp_app::menu::MenuOutcome;
use rustamp_core::command::Command;
use rustamp_core::config::Config;
use rustamp_core::state::WindowId;
use rustamp_ui::menu::{MainMenuId, MenuAction, MenuCheck, top_level};
use rustamp_ui::{Key, MainWindowLayout, Modifiers, ShortcutAction, ShortcutOutcome, Widget};

fn headless_app(config: Config) -> App {
    let options = AppOptions {
        volume: config.audio.volume,
        config,
        skin: rustamp_app::skin::builtin(),
        output: OutputChoice::Null,
        file: None,
        seconds: None,
    };
    App::new(options).expect("create headless app")
}

fn centre(widget: Widget) -> (i32, i32) {
    let rect = widget.rect(&MainWindowLayout::CLASSIC);
    (
        rect.x + rect.width as i32 / 2,
        rect.y + rect.height as i32 / 2,
    )
}

fn assert_playlist_state(app: &App, open: bool) {
    assert_eq!(app.playlist_open(), open, "playlist visibility model");
    assert_eq!(
        app.state().playlist_open,
        open,
        "main-window PL render state"
    );
    assert_eq!(
        app.menu_check(MenuCheck::PlaylistEditor),
        open,
        "playlist check must follow live visibility, not the startup config"
    );
    assert!(!app.quit(), "hiding the playlist must not quit the player");
}

#[test]
fn beta07_default_launch_starts_open_and_checked() {
    let app = headless_app(Config::default());
    assert_playlist_state(&app, true);
}

#[test]
fn beta07_skinned_close_emits_a_toggle_not_quit() {
    let mut app = headless_app(Config::default());
    assert_eq!(
        app.playlist_mouse_down(264, 3).expect("skinned close"),
        vec![Command::ToggleWindow(WindowId::Playlist)]
    );
    assert_playlist_state(&app, false);
}

#[test]
fn beta07_native_close_entry_point_hides_once_and_never_reopens() {
    let mut app = headless_app(Config::default());
    assert_eq!(
        app.close_playlist_window()
            .expect("native close entry point"),
        vec![Command::ToggleWindow(WindowId::Playlist)]
    );
    assert_playlist_state(&app, false);
    assert!(
        app.close_playlist_window()
            .expect("already closed")
            .is_empty()
    );
    assert_playlist_state(&app, false);
}

#[test]
fn beta07_left_click_pl_toggles_visibility_and_check_both_ways() {
    let mut app = headless_app(Config::default());
    let (x, y) = centre(Widget::PlaylistToggle);
    for open in [false, true] {
        let mut commands = app.mouse_down(x, y).expect("press PL");
        commands.extend(app.mouse_up(x, y).expect("release PL"));
        assert_eq!(commands, vec![Command::ToggleWindow(WindowId::Playlist)]);
        assert_playlist_state(&app, open);
    }
}

#[test]
fn beta07_right_click_pl_toggles_visibility_and_check_both_ways() {
    let mut app = headless_app(Config::default());
    let (x, y) = centre(Widget::PlaylistToggle);
    for open in [false, true] {
        assert_eq!(
            app.mouse_right_down(x, y).expect("right-click PL"),
            vec![Command::ToggleWindow(WindowId::Playlist)]
        );
        assert_playlist_state(&app, open);
    }
}

#[test]
fn beta07_real_menu_row_toggles_visibility_and_check_both_ways() {
    let mut app = headless_app(Config::default());
    let editor = top_level(MainMenuId::PlaylistEditor).expect("playlist menu row");
    assert_eq!(editor.check, MenuCheck::PlaylistEditor);
    assert_eq!(
        editor.action,
        MenuAction::Shortcut(ShortcutAction::TogglePlaylistEditor)
    );
    let MenuAction::Shortcut(action) = editor.action else {
        panic!("playlist menu row must dispatch a shortcut");
    };
    for open in [false, true] {
        assert_eq!(
            app.activate_menu_item(editor.label, action)
                .expect("activate playlist menu row"),
            MenuOutcome::Commands(vec![Command::ToggleWindow(WindowId::Playlist)])
        );
        assert_playlist_state(&app, open);
    }
}

#[test]
fn beta07_alt_e_key_lookup_and_dispatch_toggle_both_ways() {
    let mut app = headless_app(Config::default());
    for open in [false, true] {
        assert_eq!(
            app.key_down(Key::Letter(b'E'), Modifiers::ALT)
                .expect("Alt+E"),
            Some(ShortcutOutcome::Commands(vec![Command::ToggleWindow(
                WindowId::Playlist
            )]))
        );
        assert_playlist_state(&app, open);
    }
}

#[test]
fn beta07_parsed_playlist_visibility_overrides_default_and_other_windows() {
    // A literal saved-config shape exercises the production parser as well as
    // App's layout lookup. The preceding Main entry has opposite visibility:
    // taking the first layout or ignoring the Playlist id must not pass.
    for visible in [false, true] {
        let text = format!(
            r#"
[[ui.windows]]
id = "Main"
x = 26
y = 29
visible = {}
shaded = false

[[ui.windows]]
id = "Playlist"
x = 26
y = 261
width = 275
height = 116
visible = {visible}
shaded = false
"#,
            !visible
        );
        let (config, warnings) = parse_with_warnings(&text, None).expect("parse saved layout");
        assert!(warnings.is_empty(), "supported layout: {warnings:?}");
        let playlist = config
            .ui
            .windows
            .iter()
            .find(|layout| layout.id == WindowId::Playlist)
            .expect("parsed playlist layout");
        assert_eq!(playlist.visible, visible, "parser must preserve visibility");

        let mut app = headless_app(config);
        assert_playlist_state(&app, visible);
        // A restored flag is not a second state owner: a live input must
        // override it, and the check must not keep reporting the saved value.
        app.key_down(Key::Letter(b'E'), Modifiers::ALT)
            .expect("toggle restored playlist");
        assert_playlist_state(&app, !visible);
    }
}
