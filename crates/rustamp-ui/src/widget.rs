//! Hit testing over the main-window layout.
//!
//! [`hit_test`] answers "which widget is at this window-relative point?" so a
//! later card can wire clicks without re-deriving the geometry. It is a pure
//! geometry function: it does not know about clicks, state or behaviour.
//!
//! Only the *interactive* regions are hit-testable. The play/pause/stop and
//! decoder indicators are pure status readouts (and overlap the transport area),
//! so they are drawn but not returned here.

use crate::layout::MainWindowLayout;
use rustamp_gfx::Rect;

/// A hit-testable widget of the classic main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Widget {
    /// The title-bar band (drag target; the window buttons are inside its
    /// sprite and are not split out — S01 open question 1).
    TitleBar,
    /// Clutter-bar letter `O`.
    ClutterO,
    /// Clutter-bar letter `A`.
    ClutterA,
    /// Clutter-bar letter `I`.
    ClutterI,
    /// Clutter-bar letter `D`.
    ClutterD,
    /// Clutter-bar letter `V`.
    ClutterV,
    /// Previous transport button.
    Previous,
    /// Play transport button.
    Play,
    /// Pause transport button.
    Pause,
    /// Stop transport button.
    Stop,
    /// Next transport button.
    Next,
    /// Eject transport button.
    Eject,
    /// Shuffle button.
    Shuffle,
    /// Repeat button.
    Repeat,
    /// Equalizer toggle button.
    EqToggle,
    /// Playlist toggle button.
    PlaylistToggle,
    /// The position slider.
    PositionSlider,
    /// The volume slider.
    VolumeSlider,
    /// The balance slider.
    BalanceSlider,
    /// The time display (clicking it toggles elapsed/remaining in S02).
    TimeDisplay,
    /// The track-title marquee.
    Marquee,
    /// The mono indicator.
    Mono,
    /// The stereo indicator.
    Stereo,
}

impl Widget {
    /// Every hit-testable widget, in a stable order.
    pub const ALL: &'static [Widget] = &[
        Widget::TitleBar,
        Widget::ClutterO,
        Widget::ClutterA,
        Widget::ClutterI,
        Widget::ClutterD,
        Widget::ClutterV,
        Widget::Previous,
        Widget::Play,
        Widget::Pause,
        Widget::Stop,
        Widget::Next,
        Widget::Eject,
        Widget::Shuffle,
        Widget::Repeat,
        Widget::EqToggle,
        Widget::PlaylistToggle,
        Widget::PositionSlider,
        Widget::VolumeSlider,
        Widget::BalanceSlider,
        Widget::TimeDisplay,
        Widget::Marquee,
        Widget::Mono,
        Widget::Stereo,
    ];

    /// The widget's window-relative rectangle.
    ///
    /// These are the S01 §2.1 destinations, except that the shuffle region stops
    /// one pixel short of the repeat region: S01's sprite rectangles overlap by
    /// one column at x = 210 (the shuffle sprite is 47 px wide from x = 164 and
    /// the repeat sprite starts at x = 210). The pixel belongs to the repeat
    /// button, which is drawn on top.
    pub fn rect(self, layout: &MainWindowLayout) -> Rect {
        match self {
            Widget::TitleBar => layout.title_bar,
            Widget::ClutterO => layout.clutter_o,
            Widget::ClutterA => layout.clutter_a,
            Widget::ClutterI => layout.clutter_i,
            Widget::ClutterD => layout.clutter_d,
            Widget::ClutterV => layout.clutter_v,
            Widget::Previous => layout.previous,
            Widget::Play => layout.play,
            Widget::Pause => layout.pause,
            Widget::Stop => layout.stop,
            Widget::Next => layout.next,
            Widget::Eject => layout.eject,
            Widget::Shuffle => Rect::new(
                layout.shuffle.x,
                layout.shuffle.y,
                layout.shuffle.width - 1,
                layout.shuffle.height,
            ),
            Widget::Repeat => layout.repeat,
            Widget::EqToggle => layout.eq_toggle,
            Widget::PlaylistToggle => layout.playlist_toggle,
            Widget::PositionSlider => layout.position_track,
            Widget::VolumeSlider => layout.volume_track,
            Widget::BalanceSlider => layout.balance_track,
            Widget::TimeDisplay => Rect::new(
                layout.time_digits[0].x,
                layout.time_digits[0].y,
                (layout.time_digits[3].right() - layout.time_digits[0].x) as u32,
                layout.time_digits[0].height,
            ),
            Widget::Marquee => layout.marquee,
            Widget::Mono => layout.mono,
            Widget::Stereo => layout.stereo,
        }
    }
}

/// True when `rect` contains the point `(x, y)`.
fn contains(rect: &Rect, x: i32, y: i32) -> bool {
    x >= rect.x && y >= rect.y && x < rect.right() && y < rect.bottom()
}

/// Returns the widget at the window-relative point `(x, y)`, or `None` when the
/// point is outside every hit-testable region (including outside the window).
pub fn hit_test(layout: &MainWindowLayout, x: i32, y: i32) -> Option<Widget> {
    Widget::ALL
        .iter()
        .copied()
        .find(|widget| contains(&widget.rect(layout), x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> MainWindowLayout {
        MainWindowLayout::CLASSIC
    }

    #[test]
    fn every_widget_is_hit_at_its_centre_and_corners() {
        let l = layout();
        for &widget in Widget::ALL {
            let r = widget.rect(&l);
            let centre = (r.x + r.width as i32 / 2, r.y + r.height as i32 / 2);
            assert_eq!(
                hit_test(&l, centre.0, centre.1),
                Some(widget),
                "{widget:?} centre"
            );
            for (x, y) in [
                (r.x, r.y),
                (r.right() - 1, r.y),
                (r.x, r.bottom() - 1),
                (r.right() - 1, r.bottom() - 1),
            ] {
                assert_eq!(
                    hit_test(&l, x, y),
                    Some(widget),
                    "{widget:?} corner ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn hit_regions_do_not_overlap() {
        let l = layout();
        for (i, &a) in Widget::ALL.iter().enumerate() {
            for &b in &Widget::ALL[i + 1..] {
                assert_eq!(
                    a.rect(&l).intersect(&b.rect(&l)),
                    None,
                    "{a:?} overlaps {b:?}"
                );
            }
        }
    }

    #[test]
    fn nothing_outside_the_window() {
        let l = layout();
        // Just outside each edge, and far away.
        for (x, y) in [(-1, 0), (0, -1), (275, 0), (0, 116), (-1, -1), (1000, 1000)] {
            assert_eq!(hit_test(&l, x, y), None, "point ({x},{y})");
        }
        // Inside the window but not on any widget.
        assert_eq!(hit_test(&l, 274, 115), None);
        assert_eq!(hit_test(&l, 5, 100), None);
    }

    #[test]
    fn the_shared_shuffle_repeat_column_belongs_to_repeat() {
        let l = layout();
        assert_eq!(hit_test(&l, 210, 96), Some(Widget::Repeat));
        assert_eq!(hit_test(&l, 209, 96), Some(Widget::Shuffle));
    }
}
