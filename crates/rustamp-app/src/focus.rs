// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Window-focus order for `FocusNextWindow` and `ShowMainWindow`
//! ([`Command`](rustamp_core::command::Command)).
//!
//! S03 records both bindings from the resource table: Ctrl+Tab, id 40063,
//! "Switch to the next window" (`provisional`: inferred, not exercised and not
//! labelled by the resources), and Alt+W, id 40258, "Show the main window"
//! (`menu`: the action text comes from the program's own menus). Neither
//! command's effect was exercised in that audit. In particular no capture
//! measures which window comes next, so the order below is an **own choice,
//! not a measurement**.
//!
//! The choice follows the S05 docked stack top-to-bottom (main, equalizer,
//! playlist editor; S05 §3.1), which is also the order [`crate::window`]
//! creates the OS windows in. For the three windows the app has this agrees
//! with [`WindowId`]'s derived `Ord`; that order is still not what is used,
//! because there `Video` sorts after `Gen(u32)` only since `postcard` encodes
//! the enum by declaration index (see its docs), a wire-layout accident rather
//! than a focus order. `Vis`, `Gen` and `Video` are windows the app does not
//! have, so they are not in the cycle; giving them a position is open.
//!
//! What would measure the reference's order: post `WM_COMMAND` 40063 to each
//! of the three windows in turn (as the S03 audit posts other ids) and read
//! back which window holds the keyboard focus after each post, with each
//! window closed in turn to learn whether closed windows are skipped and
//! where the cycle restarts. Until such a run exists, the order below and the
//! closed-window skip stand as stated choices.
//!
//! This module is only the pure order. The model that holds the focus and the
//! pending requests lives in [`crate::app::App`]; the presenter in
//! [`crate::window`] applies them to the OS windows.

use rustamp_core::state::WindowId;

/// The windows `FocusNextWindow` cycles through, in cycle order.
///
/// Own choice (see the module docs): the S05 docked stack top-to-bottom.
pub const FOCUS_ORDER: [WindowId; 3] = [WindowId::Main, WindowId::Equalizer, WindowId::Playlist];

/// The next window after `current` among the open ones, wrapping around.
///
/// `is_open` is the app's [`App::window_open`](crate::app::App::window_open):
/// a closed window cannot take focus and is skipped. When `current` itself is
/// closed (or is a window outside the cycle), the search starts after its
/// cycle position, or at the top when it has none. Falls back to the main
/// window when nothing reports open.
pub fn next_focus(current: WindowId, is_open: impl Fn(WindowId) -> bool) -> WindowId {
    let start = FOCUS_ORDER
        .iter()
        .position(|window| *window == current)
        .map_or(0, |index| index + 1);
    for step in 0..FOCUS_ORDER.len() {
        let candidate = FOCUS_ORDER[(start + step) % FOCUS_ORDER.len()];
        if is_open(candidate) {
            return candidate;
        }
    }
    WindowId::Main
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(main: bool, eq: bool, playlist: bool) -> impl Fn(WindowId) -> bool {
        move |window| match window {
            WindowId::Main => main,
            WindowId::Equalizer => eq,
            WindowId::Playlist => playlist,
            _ => false,
        }
    }

    #[test]
    fn the_cycle_follows_the_docked_stack_top_to_bottom() {
        // Own choice, not a measurement (see the module docs).
        assert_eq!(
            FOCUS_ORDER,
            [WindowId::Main, WindowId::Equalizer, WindowId::Playlist]
        );
        let all = open(true, true, true);
        assert_eq!(next_focus(WindowId::Main, &all), WindowId::Equalizer);
        assert_eq!(next_focus(WindowId::Equalizer, &all), WindowId::Playlist);
        assert_eq!(next_focus(WindowId::Playlist, &all), WindowId::Main);
    }

    #[test]
    fn closed_windows_are_skipped() {
        assert_eq!(
            next_focus(WindowId::Main, open(true, false, true)),
            WindowId::Playlist
        );
        assert_eq!(
            next_focus(WindowId::Main, open(true, true, false)),
            WindowId::Equalizer
        );
        assert_eq!(
            next_focus(WindowId::Equalizer, open(true, false, false)),
            WindowId::Main
        );
    }

    #[test]
    fn a_lone_main_window_keeps_the_focus() {
        assert_eq!(
            next_focus(WindowId::Main, open(true, false, false)),
            WindowId::Main
        );
    }

    #[test]
    fn nothing_open_falls_back_to_main() {
        assert_eq!(
            next_focus(WindowId::Playlist, open(false, false, false)),
            WindowId::Main
        );
    }

    #[test]
    fn a_closed_current_window_continues_after_its_position() {
        // The focused window closed underneath the focus (toggled off while
        // focused): the cycle continues after it rather than restarting.
        assert_eq!(
            next_focus(WindowId::Equalizer, open(true, false, true)),
            WindowId::Playlist
        );
        assert_eq!(
            next_focus(WindowId::Playlist, open(true, true, false)),
            WindowId::Main
        );
    }

    #[test]
    fn windows_outside_the_cycle_start_at_the_top() {
        let all = open(true, true, true);
        assert_eq!(next_focus(WindowId::Vis, &all), WindowId::Main);
        assert_eq!(next_focus(WindowId::Video, &all), WindowId::Main);
        assert_eq!(next_focus(WindowId::Gen(0), &all), WindowId::Main);
    }

    #[test]
    fn windows_the_app_does_not_have_never_take_focus() {
        // Even when the caller reports them open, only cycle members cycle.
        let everything = |_: WindowId| true;
        assert!(FOCUS_ORDER.contains(&next_focus(WindowId::Main, everything)));
        assert!(FOCUS_ORDER.contains(&next_focus(WindowId::Gen(7), everything)));
    }
}
