// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The double-click rule (S03 §5).
//!
//! One rule for every double click the player recognises: the second press
//! must follow the first within [`DOUBLE_CLICK_INTERVAL_MS`] and within
//! [`DOUBLE_CLICK_DISTANCE_PX`] pixels of it on each axis.
//!
//! The playlist list used it first (double-clicking an entry plays it), and
//! the three title bars use the same one (double-clicking a title bar toggles
//! the windowshade), so the numbers live here instead of being written twice.

use rustamp_core::state::WindowId;

/// The double-click interval: the second press must follow the first within
/// this many milliseconds.
pub const DOUBLE_CLICK_INTERVAL_MS: u64 = 500;

/// The double-click distance: the second press must be within this many
/// pixels of the first, on each axis.
pub const DOUBLE_CLICK_DISTANCE_PX: i32 = 4;

/// The press a double click is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Press {
    /// When the press happened, in milliseconds.
    at_ms: u64,
    /// Where the press landed, in window-relative pixels.
    x: i32,
    y: i32,
}

/// Remembers the last press, so the next one can be recognised as its double
/// click.
///
/// The tracker is passive: it never remembers or forgets a press on its own.
/// The caller says what a press means — a press that may still become half of
/// a double click is remembered with [`DoubleClick::remember`], one that ends
/// a double click with [`DoubleClick::clear`].
#[derive(Debug, Default, Clone, Copy)]
pub struct DoubleClick {
    last: Option<Press>,
}

impl DoubleClick {
    /// Whether the press at `(x, y)` and `at_ms` completes a double click
    /// with the remembered press.
    ///
    /// The interval is measured from the first press to the second one, and
    /// the distance on each axis.
    pub fn is_double_click(&self, at_ms: u64, x: i32, y: i32) -> bool {
        self.last.is_some_and(|last| {
            at_ms.saturating_sub(last.at_ms) < DOUBLE_CLICK_INTERVAL_MS
                && (x - last.x).abs() <= DOUBLE_CLICK_DISTANCE_PX
                && (y - last.y).abs() <= DOUBLE_CLICK_DISTANCE_PX
        })
    }

    /// Remembers the press at `(x, y)` and `at_ms` as the partner of the
    /// next one.
    pub fn remember(&mut self, at_ms: u64, x: i32, y: i32) {
        self.last = Some(Press { at_ms, x, y });
    }

    /// Forgets the remembered press: the sequence ends here, and the next
    /// press starts a new one.
    pub fn clear(&mut self) {
        self.last = None;
    }
}

/// The title-bar double-click trackers of the three classic windows.
///
/// One tracker per window, so a press in one window never pairs with a press
/// in another.
#[derive(Debug, Default)]
pub struct TitleBarClicks {
    main: DoubleClick,
    equalizer: DoubleClick,
    playlist: DoubleClick,
}

impl TitleBarClicks {
    /// The tracker of `window`, if it is one of the three classic windows.
    pub fn tracker_mut(&mut self, window: WindowId) -> Option<&mut DoubleClick> {
        match window {
            WindowId::Main => Some(&mut self.main),
            WindowId::Equalizer => Some(&mut self.equalizer),
            WindowId::Playlist => Some(&mut self.playlist),
            WindowId::Vis | WindowId::Gen(_) | WindowId::Video => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rule_is_the_one_the_playlist_list_already_used() {
        assert_eq!(DOUBLE_CLICK_INTERVAL_MS, 500);
        assert_eq!(DOUBLE_CLICK_DISTANCE_PX, 4);
    }

    #[test]
    fn a_second_press_inside_the_interval_and_the_distance_is_a_double_click() {
        let mut tracker = DoubleClick::default();
        tracker.remember(1_000, 100, 7);
        assert!(tracker.is_double_click(1_499, 104, 11));
        assert!(tracker.is_double_click(1_001, 100, 7));
    }

    #[test]
    fn a_slow_or_a_distant_second_press_is_not_a_double_click() {
        let mut tracker = DoubleClick::default();
        tracker.remember(1_000, 100, 7);
        // The interval is exclusive: 500 ms apart is too late.
        assert!(!tracker.is_double_click(1_500, 100, 7));
        assert!(!tracker.is_double_click(1_499 + 1, 100, 7));
        // The distance is inclusive: 4 px away is still the same spot.
        assert!(tracker.is_double_click(1_100, 104, 7));
        assert!(!tracker.is_double_click(1_100, 105, 7));
        assert!(!tracker.is_double_click(1_100, 100, 12));
    }

    #[test]
    fn a_forgotten_press_starts_a_new_sequence() {
        let mut tracker = DoubleClick::default();
        tracker.remember(1_000, 100, 7);
        tracker.clear();
        assert!(!tracker.is_double_click(1_100, 100, 7));
    }
}
