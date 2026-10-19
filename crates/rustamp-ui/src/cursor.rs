// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Which cursor a control shows.
//!
//! **This mapping is Rustamp's own choice.** S17 §8 verified that a skin
//! ships a set of cursor files and that they are ordinary Windows `.cur`
//! files, but it leaves the mapping itself open: *"Not determined: which
//! window region each cursor belongs to, whether the program falls back to the
//! built-in cursors when one is missing, and whether a cursor file of the
//! wrong size is rejected."* Nothing below is a measured fact.
//!
//! The rules follow the **meaning of the classic cursor names** — the names
//! say what a cursor is for (`posbar` the position slider, `volbal` the volume
//! and balance sliders, `ptbar` the playlist title bar, `pvscroll` the
//! playlist scrollbar, …), so the mapping assigns each control the cursor
//! whose name describes it. Every slot the base skin ships is reachable
//! except `psize` (the playlist's resize grip), because the playlist editor
//! has no resize region to hit-test yet.
//!
//! The functions are pure: they turn a hit-test result into the
//! [`CursorSlot`] the skin is asked for. They never touch the skin, the
//! pointer or the OS — the caller looks the slot up in the skin
//! ([`rustamp_skin::Skin::cursor_slot`]) and applies it.

use rustamp_skin::CursorSlot;

use crate::equalizer::EqWidget;
use crate::playlist_window::{self, PlaylistTitleButton};
use crate::widget::Widget;

/// The cursor the main window shows over `widget`.
///
/// `widget` is the result of [`crate::widget::hit_test`]: `None` is the
/// pointer over the window's background, which is the normal cursor.
pub const fn main_window_cursor(widget: Option<Widget>) -> CursorSlot {
    match widget {
        // The four window buttons, the title band and the song title.
        Some(Widget::WindowMenu) => CursorSlot::MainMenu,
        Some(Widget::WindowMinimize) => CursorSlot::Min,
        Some(Widget::WindowShade) => CursorSlot::WinBut,
        Some(Widget::WindowClose) => CursorSlot::Close,
        Some(Widget::TitleBar) => CursorSlot::TitleBar,
        Some(Widget::Marquee) => CursorSlot::SongName,
        // The three sliders of the main window.
        Some(Widget::PositionSlider) => CursorSlot::PosBar,
        Some(Widget::VolumeSlider) | Some(Widget::BalanceSlider) => CursorSlot::VolBal,
        // Everything else is the plain background.
        _ => CursorSlot::Normal,
    }
}

/// The cursor the equalizer window shows over `widget`.
///
/// `widget` is the result of [`crate::equalizer::hit_test`]: `None` is the
/// pointer over the window's background, which is the equalizer's own
/// normal cursor.
pub const fn equalizer_cursor(widget: Option<EqWidget>) -> CursorSlot {
    match widget {
        Some(EqWidget::TitleBar) => CursorSlot::EqTitle,
        Some(EqWidget::Slider(_)) => CursorSlot::EqSlid,
        // The On, Auto and Presets buttons are plain background.
        _ => CursorSlot::EqNormal,
    }
}

/// The cursor the playlist window shows at the window-relative point
/// `(x, y)`.
///
/// The playlist's hit test is split over several functions (the title bar,
/// the list, the scrollbar, the bottom bar), so this one takes the point and
/// asks the same functions the input path asks.
pub fn playlist_cursor_at(x: i32, y: i32) -> CursorSlot {
    match playlist_window::hit_test_title_bar(x, y) {
        Some(PlaylistTitleButton::Close) => return CursorSlot::PClose,
        Some(PlaylistTitleButton::Shade) => return CursorSlot::PWinBut,
        None => {}
    }
    // The scrollbar: the track with the handle and the arrow pair below it.
    if in_rect(playlist_window::SCROLL_TRACK_RECT, x, y)
        || in_rect(playlist_window::SCROLL_ARROWS_RECT, x, y)
    {
        return CursorSlot::PVScroll;
    }
    // The title band itself is a drag target.
    if (0..playlist_window::TITLE_BAR_HEIGHT).contains(&y) {
        return CursorSlot::PTBar;
    }
    // The list, the bottom bar and the rest of the window.
    CursorSlot::PNormal
}

/// True when `rect` contains the point `(x, y)`.
fn in_rect(rect: rustamp_gfx::Rect, x: i32, y: i32) -> bool {
    x >= rect.x && y >= rect.y && x < rect.right() && y < rect.bottom()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::equalizer::{EqualizerLayout, hit_test as eq_hit_test};
    use crate::layout::MainWindowLayout;
    use crate::widget::hit_test;

    #[test]
    fn the_main_window_buttons_and_sliders_get_their_own_cursor() {
        let layout = MainWindowLayout::CLASSIC;
        let cases = [
            (10, 7, CursorSlot::MainMenu),
            (248, 7, CursorSlot::Min),
            (258, 7, CursorSlot::WinBut),
            (268, 7, CursorSlot::Close),
            (137, 7, CursorSlot::TitleBar),
            (120, 30, CursorSlot::SongName),
            (100, 77, CursorSlot::PosBar),
            (140, 63, CursorSlot::VolBal),
            (190, 63, CursorSlot::VolBal),
        ];
        for (x, y, expected) in cases {
            let widget = hit_test(&layout, x, y);
            assert_eq!(
                main_window_cursor(widget),
                expected,
                "at ({x},{y}) the widget is {widget:?}"
            );
        }
    }

    #[test]
    fn the_main_window_background_is_the_normal_cursor() {
        let layout = MainWindowLayout::CLASSIC;
        // A point on no widget at all.
        assert_eq!(hit_test(&layout, 5, 100), None);
        assert_eq!(main_window_cursor(None), CursorSlot::Normal);
        // A widget that owns no cursor of its own.
        assert_eq!(
            main_window_cursor(hit_test(&layout, 50, 96)),
            CursorSlot::Normal
        );
    }

    #[test]
    fn the_equalizer_sliders_and_title_bar_get_their_own_cursor() {
        let layout = EqualizerLayout::CLASSIC;
        // A band slider (x = 79 + 18k) and the title bar.
        assert_eq!(
            equalizer_cursor(eq_hit_test(&layout, 79, 60)),
            CursorSlot::EqSlid
        );
        assert_eq!(
            equalizer_cursor(eq_hit_test(&layout, 22, 60)),
            CursorSlot::EqSlid
        );
        assert_eq!(
            equalizer_cursor(eq_hit_test(&layout, 100, 5)),
            CursorSlot::EqTitle
        );
        // The On button is plain background.
        assert_eq!(
            equalizer_cursor(eq_hit_test(&layout, 20, 24)),
            CursorSlot::EqNormal
        );
        assert_eq!(equalizer_cursor(None), CursorSlot::EqNormal);
    }

    #[test]
    fn the_playlist_regions_get_their_own_cursor() {
        // The title band, the two title-bar buttons, the scrollbar and the
        // list.
        assert_eq!(playlist_cursor_at(120, 5), CursorSlot::PTBar);
        assert_eq!(playlist_cursor_at(268, 7), CursorSlot::PClose);
        assert_eq!(playlist_cursor_at(258, 7), CursorSlot::PWinBut);
        assert_eq!(playlist_cursor_at(263, 40), CursorSlot::PVScroll);
        assert_eq!(playlist_cursor_at(263, 82), CursorSlot::PVScroll);
        assert_eq!(playlist_cursor_at(100, 40), CursorSlot::PNormal);
        assert_eq!(playlist_cursor_at(100, 100), CursorSlot::PNormal);
    }
}
