// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! A `WM_COMMAND` addressed to a classic window applies to that window.
//!
//! The reference carries its commands as `WM_COMMAND` ids (S03 §1): 40064
//! toggles the windowshade, 40165 toggles the double size. The reference
//! applies such a command to the window it was addressed to — the
//! equalizer's shade is its own flag (`eq_ws`), the playlist editor's is its
//! own (`pe_height_ws`), beside the main window's (`windowshade`), S01 §1.
//!
//! The three states the base-2.91 campaign measured and our player could not
//! reach are exactly these: the equalizer's shade (275 x 14), the
//! equalizer's double size (550 x 232) and the playlist editor's shade
//! (275 x 14) — S05 §1.1, §2.1 and S01 §1.
//!
//! The tests are headless. They drive [`App::apply_window_command`], the
//! entry point the presenter uses for a command the operating system posted,
//! and they read the sizes from the app ([`App::eq_frame_size`],
//! [`App::playlist_frame_size`], [`App::frame_size`]) — the one place the
//! geometry lives, so the presenter resizes the OS window from the same
//! numbers. No window opens and no audio device is touched.
//!
//! What the tests cannot do: prove that the operating system delivers a
//! posted `WM_COMMAND` to the equalizer's or the playlist editor's window.
//! That is the native seam in `crate::window` and it needs a real window, so
//! the VM capture campaign is what confirms it.

use rustamp_app::app::{App, AppOptions, WindowCommand};
use rustamp_app::cli::OutputChoice;
use rustamp_core::config::Config;
use rustamp_core::state::WindowId;

/// The id the reference's "toggle windowshade" carries (S03 §1).
const TOGGLE_WINDOWSHADE: u32 = 40064;
/// The id the reference's "toggle double size" carries (S03 §1).
const TOGGLE_DOUBLE_SIZE: u32 = 40165;

/// The headless player: the null output, the built-in skin, no file.
fn app() -> App {
    let options = AppOptions {
        config: Config::default(),
        skin: rustamp_app::skin::builtin(),
        output: OutputChoice::Null,
        volume: 200,
        media: None,
        seconds: None,
    };
    App::new(options).expect("the headless app builds")
}

/// The command the id carries, as the native seam reads it.
fn command(id: u32) -> WindowCommand {
    WindowCommand::from_command_id(id).expect("the id carries a window command")
}

/// Applies the command the id carries to `window`.
fn post(app: &mut App, window: WindowId, id: u32) {
    app.apply_window_command(window, command(id))
        .expect("the window command applies");
}

// ----- the ids -----

#[test]
fn only_the_two_measured_ids_carry_a_window_command() {
    assert_eq!(
        command(TOGGLE_WINDOWSHADE),
        WindowCommand::ToggleWindowShade
    );
    assert_eq!(command(TOGGLE_DOUBLE_SIZE), WindowCommand::ToggleDoubleSize);
    // Every other id belongs to a command the classic windows do not
    // answer: the main window's own ids (40036 opens the equalizer, 40040
    // the playlist editor) are the external control's business.
    for id in [0, 123, 40036, 40040, 40045, 40258, 65536] {
        assert_eq!(
            WindowCommand::from_command_id(id),
            None,
            "id {id} carries no window command"
        );
    }
}

// ----- the equalizer -----

#[test]
fn a_windowshade_command_addressed_to_the_equalizer_collapses_it() {
    let mut app = app();
    assert_eq!(app.eq_frame_size(), (275, 116));

    post(&mut app, WindowId::Equalizer, TOGGLE_WINDOWSHADE);
    assert!(app.eq_state().windowshade, "the equalizer is shaded");
    // S01 §1: the shade band is 14 px tall.
    assert_eq!(app.eq_frame_size(), (275, 14));
    // The docked stack follows the shaded window (S05 §3.3).
    assert_eq!(app.docked_stack().equalizer.height, 14);

    // The same command again restores it.
    post(&mut app, WindowId::Equalizer, TOGGLE_WINDOWSHADE);
    assert!(!app.eq_state().windowshade, "the equalizer is unshaded");
    assert_eq!(app.eq_frame_size(), (275, 116));
}

#[test]
fn a_double_size_command_addressed_to_the_equalizer_doubles_it() {
    let mut app = app();
    assert_eq!(app.eq_frame_size(), (275, 116));

    post(&mut app, WindowId::Equalizer, TOGGLE_DOUBLE_SIZE);
    assert!(app.eq_state().double_size, "the equalizer is double-sized");
    // S05 §2.1: the equalizer's double size is 550 x 232.
    assert_eq!(app.eq_frame_size(), (550, 232));
    // Measured 2026-10-21 (capture job `base291-global-double`): the same
    // command posted to the equalizer also took the MAIN window from 275x116 to
    // 550x232, while the same treatment with the windowshade command left it
    // alone. Double size is one `dsize` flag for the player (S01 §1), not a
    // per-window one - the first version of this code had that wrong.
    assert_eq!(
        app.frame_size(),
        (550, 232),
        "one double-size flag for the player"
    );

    // The same command again restores it.
    post(&mut app, WindowId::Equalizer, TOGGLE_DOUBLE_SIZE);
    assert!(!app.eq_state().double_size, "the equalizer is 1x again");
    assert_eq!(app.eq_frame_size(), (275, 116));
    assert_eq!(
        app.frame_size(),
        (275, 116),
        "and the main window follows the same flag back"
    );
}

/// The size a window comes back to is the one it had, not a fixed default:
/// the double-size flag survives the shade (S05 §2.3 for the main window,
/// S01 §1 for the equalizer's shade height).
#[test]
fn unshading_the_equalizer_returns_to_the_size_it_had() {
    let mut app = app();
    post(&mut app, WindowId::Equalizer, TOGGLE_DOUBLE_SIZE);
    assert_eq!(app.eq_frame_size(), (550, 232));

    post(&mut app, WindowId::Equalizer, TOGGLE_WINDOWSHADE);
    // The shaded double-sized equalizer is the 2x shade (550 x 28); that
    // combination is our own choice for the equalizer (S05 §1.5 leaves the
    // equalizer's shade drawing and size open).
    assert_eq!(app.eq_frame_size(), (550, 28));

    post(&mut app, WindowId::Equalizer, TOGGLE_WINDOWSHADE);
    assert_eq!(
        app.eq_frame_size(),
        (550, 232),
        "unshading returns to the previous size, not to a hard-coded 275 x 116"
    );
}

// ----- the playlist editor -----

#[test]
fn a_windowshade_command_addressed_to_the_playlist_collapses_it() {
    let mut app = app();
    assert_eq!(app.playlist_frame_size(), (275, 116));

    post(&mut app, WindowId::Playlist, TOGGLE_WINDOWSHADE);
    assert!(app.playlist_shaded(), "the playlist editor is shaded");
    // S01 §1: the shade band is 14 px tall.
    assert_eq!(app.playlist_frame_size(), (275, 14));

    // The same command again restores it.
    post(&mut app, WindowId::Playlist, TOGGLE_WINDOWSHADE);
    assert!(!app.playlist_shaded(), "the playlist editor is unshaded");
    assert_eq!(app.playlist_frame_size(), (275, 116));
}

/// The playlist editor is **not** affected by double size (S05 §2.1, S01 §1:
/// it stays 275 x 116), so the double-size command has no geometry to
/// change there — and it changes nothing else either.
#[test]
fn a_double_size_command_addressed_to_the_playlist_changes_nothing() {
    let mut app = app();
    let before = (
        app.playlist_frame_size(),
        app.frame_size(),
        app.eq_frame_size(),
    );

    post(&mut app, WindowId::Playlist, TOGGLE_DOUBLE_SIZE);

    assert_eq!(
        app.playlist_frame_size(),
        (275, 116),
        "the playlist editor is not affected by double size (S05 §2.1)"
    );
    assert_eq!(app.frame_size(), before.1, "the main window did not move");
    assert_eq!(app.eq_frame_size(), before.2, "the equalizer did not move");
}

// ----- the main window keeps its own answer -----

#[test]
fn the_main_window_keeps_its_own_answer() {
    let mut app = app();

    post(&mut app, WindowId::Main, TOGGLE_WINDOWSHADE);
    assert!(app.state().windowshade, "the main window is shaded");
    // S05 §1.1: the main window's shade is 275 x 14.
    assert_eq!(app.frame_size(), (275, 14));

    post(&mut app, WindowId::Main, TOGGLE_WINDOWSHADE);
    assert!(!app.state().windowshade, "the main window is unshaded");
    assert_eq!(app.frame_size(), (275, 116));

    post(&mut app, WindowId::Main, TOGGLE_DOUBLE_SIZE);
    assert!(app.state().double_size, "the main window is double-sized");
    // S05 §2.1: the main window's double size is 550 x 232.
    assert_eq!(app.frame_size(), (550, 232));
}

/// A window that is not one of the classic three answers nothing.
#[test]
fn a_window_outside_the_classic_three_answers_nothing() {
    let mut app = app();
    for window in [WindowId::Vis, WindowId::Video, WindowId::Gen(7)] {
        assert!(
            app.apply_window_command(window, WindowCommand::ToggleWindowShade)
                .is_err(),
            "{window:?} has no window command"
        );
    }
    assert_eq!(app.frame_size(), (275, 116));
    assert_eq!(app.eq_frame_size(), (275, 116));
    assert_eq!(app.playlist_frame_size(), (275, 116));
}
