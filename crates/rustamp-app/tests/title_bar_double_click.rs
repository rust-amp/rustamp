// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Double-clicking a title bar toggles the windowshade (S03 §5).
//!
//! The spec settles the gesture: "Double-click the title bar: toggle
//! windowshade" (verified on the 2.95). What counts as a double click is the
//! rule the playlist list already used — the second press within 500 ms and
//! within 4 px of the first — and it is shared now
//! ([`rustamp_app::double_click`]).
//!
//! The tests are headless. They drive [`App::title_bar_double_click`], the
//! entry point the presenter asks about on **every** press, and the ordinary
//! press paths for the single click. No window opens and no audio device is
//! touched.

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_core::command::Command;
use rustamp_core::config::Config;
use rustamp_core::state::WindowId;
use rustamp_ui::{MainWindowLayout, Widget, playlist_window};

/// A spot on the main window's title bar that is not a control.
const MAIN_TITLE: (i32, i32) = (137, 7);
/// A spot on the equalizer's title bar.
const EQ_TITLE: (i32, i32) = (137, 7);
/// A spot on the playlist editor's title bar that is not a control.
const PLAYLIST_TITLE: (i32, i32) = (137, 7);
/// The second press of a double click: 120 ms later, on the same spot.
const SECOND_PRESS_MS: u64 = 120;

/// The headless player: the null output, the built-in skin, no file.
fn app() -> App {
    let options = AppOptions {
        config: Config::default(),
        skin: rustamp_app::skin::builtin(),
        output: OutputChoice::Null,
        volume: 200,
        file: None,
        seconds: None,
    };
    App::new(options).expect("the headless app builds")
}

/// The centre of the main window's windowshade button (S02 §7).
fn main_shade_button() -> (i32, i32) {
    let rect = Widget::WindowShade.rect(&MainWindowLayout::CLASSIC);
    (
        rect.x + rect.width as i32 / 2,
        rect.y + rect.height as i32 / 2,
    )
}

/// The centre of the playlist editor's windowshade button (S01 §4).
fn playlist_shade_button() -> (i32, i32) {
    let rect = playlist_window::SHADE_BUTTON_RECT;
    (
        rect.x + rect.width as i32 / 2,
        rect.y + rect.height as i32 / 2,
    )
}

/// A press on the main window, the way the presenter does it: the title bar's
/// double click first, then the ordinary press.
fn press_main(app: &mut App, x: i32, y: i32, now_ms: u64) -> Vec<Command> {
    let mut commands = app
        .title_bar_double_click(WindowId::Main, x, y, now_ms)
        .expect("press");
    if commands.is_empty() {
        commands.extend(app.mouse_down(x, y).expect("press"));
        commands.extend(app.mouse_up(x, y).expect("release"));
    }
    commands
}

/// A press on the playlist editor, the way the presenter does it.
fn press_playlist(app: &mut App, x: i32, y: i32, now_ms: u64) -> Vec<Command> {
    let mut commands = app
        .title_bar_double_click(WindowId::Playlist, x, y, now_ms)
        .expect("press");
    if commands.is_empty() {
        commands.extend(app.playlist_mouse_down(x, y).expect("press"));
    }
    commands
}

// ----- the gesture on each window -----

#[test]
fn a_double_click_on_the_main_title_bar_toggles_the_windowshade() {
    let mut app = app();
    assert!(!app.state().windowshade);
    assert_eq!(app.frame_size(), (275, 116));

    // The first press only starts the sequence.
    assert!(
        app.title_bar_double_click(WindowId::Main, MAIN_TITLE.0, MAIN_TITLE.1, 0)
            .expect("press")
            .is_empty()
    );
    assert!(!app.state().windowshade);

    // The second one, fast and close by, is the double click (S03 §5).
    let commands = app
        .title_bar_double_click(WindowId::Main, MAIN_TITLE.0, MAIN_TITLE.1, SECOND_PRESS_MS)
        .expect("press");
    assert_eq!(commands, vec![Command::ToggleWindowShade(WindowId::Main)]);
    assert!(app.state().windowshade);
    // The shade band is 14 px tall (S01 §1).
    assert_eq!(app.frame_size(), (275, 14));
}

#[test]
fn a_double_click_on_the_equalizer_title_bar_toggles_the_windowshade() {
    let mut app = app();
    assert!(!app.eq_state().windowshade);
    assert_eq!(app.eq_frame_size(), (275, 116));

    assert!(
        app.title_bar_double_click(WindowId::Equalizer, EQ_TITLE.0, EQ_TITLE.1, 0)
            .expect("press")
            .is_empty()
    );
    let commands = app
        .title_bar_double_click(WindowId::Equalizer, EQ_TITLE.0, EQ_TITLE.1, SECOND_PRESS_MS)
        .expect("press");
    assert_eq!(
        commands,
        vec![Command::ToggleWindowShade(WindowId::Equalizer)]
    );
    assert!(app.eq_state().windowshade);
    assert_eq!(app.eq_frame_size(), (275, 14));
}

#[test]
fn a_double_click_on_the_playlist_title_bar_toggles_the_windowshade() {
    let mut app = app();
    assert!(!app.playlist_shaded());
    assert_eq!(app.playlist_frame_size(), (275, 116));

    assert!(
        app.title_bar_double_click(WindowId::Playlist, PLAYLIST_TITLE.0, PLAYLIST_TITLE.1, 0)
            .expect("press")
            .is_empty()
    );
    let commands = app
        .title_bar_double_click(
            WindowId::Playlist,
            PLAYLIST_TITLE.0,
            PLAYLIST_TITLE.1,
            SECOND_PRESS_MS,
        )
        .expect("press");
    assert_eq!(
        commands,
        vec![Command::ToggleWindowShade(WindowId::Playlist)]
    );
    assert!(app.playlist_shaded());
    assert_eq!(app.playlist_frame_size(), (275, 14));
}

#[test]
fn a_double_click_in_the_shade_leaves_the_windowshade_again() {
    let mut app = app();
    app.submit(Command::ToggleWindowShade(WindowId::Main))
        .expect("shade");
    assert!(app.state().windowshade);

    // The shaded title bar is the band too, so the gesture works both ways.
    assert!(
        app.title_bar_double_click(WindowId::Main, MAIN_TITLE.0, MAIN_TITLE.1, 0)
            .expect("press")
            .is_empty()
    );
    let commands = app
        .title_bar_double_click(WindowId::Main, MAIN_TITLE.0, MAIN_TITLE.1, SECOND_PRESS_MS)
        .expect("press");
    assert_eq!(commands, vec![Command::ToggleWindowShade(WindowId::Main)]);
    assert!(!app.state().windowshade);
    assert_eq!(app.frame_size(), (275, 116));
}

// ----- what a button click wins -----

#[test]
fn a_double_click_on_the_main_shade_button_is_the_button_s_own_action() {
    let mut app = app();
    let (x, y) = main_shade_button();

    // Two clicks on the button: the button toggles the shade twice, so the
    // window is back where it started. The double click adds nothing.
    let first = press_main(&mut app, x, y, 0);
    assert_eq!(first, vec![Command::ToggleWindowShade(WindowId::Main)]);
    assert!(app.state().windowshade);

    let second = press_main(&mut app, x, y, SECOND_PRESS_MS);
    assert_eq!(second, vec![Command::ToggleWindowShade(WindowId::Main)]);
    assert!(
        !app.state().windowshade,
        "the double click on the button must not toggle the shade a third time"
    );
}

#[test]
fn a_double_click_on_the_playlist_shade_button_is_the_button_s_own_action() {
    let mut app = app();
    let (x, y) = playlist_shade_button();

    let first = press_playlist(&mut app, x, y, 0);
    assert_eq!(first, vec![Command::ToggleWindowShade(WindowId::Playlist)]);
    assert!(app.playlist_shaded());

    let second = press_playlist(&mut app, x, y, SECOND_PRESS_MS);
    assert_eq!(second, vec![Command::ToggleWindowShade(WindowId::Playlist)]);
    assert!(
        !app.playlist_shaded(),
        "the double click on the button must not toggle the shade a third time"
    );
}

#[test]
fn a_button_click_ends_the_title_bar_sequence() {
    let mut app = app();
    // A press on the title bar starts a sequence ...
    assert!(
        app.title_bar_double_click(WindowId::Main, MAIN_TITLE.0, MAIN_TITLE.1, 0)
            .expect("press")
            .is_empty()
    );
    // ... and the press on the button in between ends it: the button wins.
    let (x, y) = main_shade_button();
    assert!(
        app.title_bar_double_click(WindowId::Main, x, y, 50)
            .expect("press")
            .is_empty()
    );
    let commands = app
        .title_bar_double_click(WindowId::Main, MAIN_TITLE.0, MAIN_TITLE.1, 100)
        .expect("press");
    assert!(
        commands.is_empty(),
        "the title-bar press after a button press must not pair with it"
    );
}

// ----- what is not a double click -----

#[test]
fn a_slow_second_press_on_the_title_bar_is_not_a_double_click() {
    let mut app = app();
    assert!(
        app.title_bar_double_click(WindowId::Main, MAIN_TITLE.0, MAIN_TITLE.1, 0)
            .expect("press")
            .is_empty()
    );
    // 500 ms is the rule's limit and it is exclusive: this press is late.
    let commands = app
        .title_bar_double_click(WindowId::Main, MAIN_TITLE.0, MAIN_TITLE.1, 500)
        .expect("press");
    assert!(commands.is_empty());
    assert!(!app.state().windowshade);
}

#[test]
fn a_distant_second_press_on_the_title_bar_is_not_a_double_click() {
    let mut app = app();
    assert!(
        app.title_bar_double_click(WindowId::Main, MAIN_TITLE.0, MAIN_TITLE.1, 0)
            .expect("press")
            .is_empty()
    );
    // 5 px away is outside the 4 px rule.
    let commands = app
        .title_bar_double_click(WindowId::Main, MAIN_TITLE.0 + 5, MAIN_TITLE.1, 100)
        .expect("press");
    assert!(commands.is_empty());
    assert!(!app.state().windowshade);
}

#[test]
fn a_press_outside_the_title_bar_never_toggles_the_shade() {
    let mut app = app();
    // The time display, the transport buttons and the sliders are not the
    // title bar (S02 §4, S03 §5).
    for (x, y) in [(60, 30), (50, 97), (120, 60)] {
        assert!(
            app.title_bar_double_click(WindowId::Main, x, y, 0)
                .expect("press")
                .is_empty()
        );
        let commands = app
            .title_bar_double_click(WindowId::Main, x, y, SECOND_PRESS_MS)
            .expect("press");
        assert!(
            commands.is_empty(),
            "the press at ({x},{y}) toggled the shade"
        );
    }
    assert!(!app.state().windowshade);
}

#[test]
fn a_press_in_one_window_never_pairs_with_a_press_in_another() {
    let mut app = app();
    assert!(
        app.title_bar_double_click(WindowId::Main, MAIN_TITLE.0, MAIN_TITLE.1, 0)
            .expect("press")
            .is_empty()
    );
    // The equalizer's press is a first press of its own.
    assert!(
        app.title_bar_double_click(WindowId::Equalizer, EQ_TITLE.0, EQ_TITLE.1, 50)
            .expect("press")
            .is_empty()
    );
    // The main window's second press is still its double click.
    let commands = app
        .title_bar_double_click(WindowId::Main, MAIN_TITLE.0, MAIN_TITLE.1, 100)
        .expect("press");
    assert_eq!(commands, vec![Command::ToggleWindowShade(WindowId::Main)]);
    assert!(!app.eq_state().windowshade);
}

// ----- the single click still does its own thing -----

#[test]
fn a_single_click_on_the_title_bar_does_nothing() {
    let mut app = app();
    // The title bar is the window's drag target; the press itself sends
    // nothing (S03 §5: "Drag the title bar: move the window").
    assert!(press_main(&mut app, MAIN_TITLE.0, MAIN_TITLE.1, 0).is_empty());
    assert!(!app.state().windowshade);
}

#[test]
fn a_single_click_on_the_shade_button_still_toggles_the_shade() {
    let mut app = app();
    let (x, y) = main_shade_button();
    let commands = press_main(&mut app, x, y, 0);
    assert_eq!(commands, vec![Command::ToggleWindowShade(WindowId::Main)]);
    assert!(app.state().windowshade);
    assert_eq!(app.frame_size(), (275, 14));
}

#[test]
fn a_single_click_on_the_playlist_shade_button_still_toggles_the_shade() {
    let mut app = app();
    let (x, y) = playlist_shade_button();
    let commands = press_playlist(&mut app, x, y, 0);
    assert_eq!(
        commands,
        vec![Command::ToggleWindowShade(WindowId::Playlist)]
    );
    assert!(app.playlist_shaded());
    assert_eq!(app.playlist_frame_size(), (275, 14));
}
