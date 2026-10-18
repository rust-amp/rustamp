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
use crate::state::{TransportButton, WindowButton};
use rustamp_gfx::Rect;

/// A hit-testable widget of the classic main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Widget {
    /// The title-bar menu window button (S02 §7).
    WindowMenu,
    /// The title-bar minimize window button (S02 §7).
    WindowMinimize,
    /// The title-bar windowshade button (S02 §7).
    WindowShade,
    /// The title-bar close window button (S02 §7).
    WindowClose,
    /// The title-bar band (drag target; the window buttons sit inside it).
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
    /// Rustamp's UI-local analyser/scope mode cycle (S08 Q9 open, own choice).
    Visualization,
    /// The mono indicator.
    Mono,
    /// The stereo indicator.
    Stereo,
}

impl Widget {
    /// Every hit-testable widget, in a stable order.
    ///
    /// The window buttons come first so [`hit_test`] returns them before the
    /// title-bar band they sit inside.
    pub const ALL: &'static [Widget] = &[
        Widget::WindowMenu,
        Widget::WindowMinimize,
        Widget::WindowShade,
        Widget::WindowClose,
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
        Widget::Visualization,
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
            Widget::WindowMenu => layout.window_button(WindowButton::Menu),
            Widget::WindowMinimize => layout.window_button(WindowButton::Minimize),
            Widget::WindowShade => layout.window_button(WindowButton::Shade),
            Widget::WindowClose => layout.window_button(WindowButton::Close),
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
            Widget::TimeDisplay => layout.time_display,
            Widget::Marquee => layout.marquee,
            Widget::Visualization => layout.visualization,
            Widget::Mono => layout.mono,
            Widget::Stereo => layout.stereo,
        }
    }

    /// The transport button this widget is, if any.
    pub const fn transport_button(self) -> Option<TransportButton> {
        match self {
            Widget::Previous => Some(TransportButton::Previous),
            Widget::Play => Some(TransportButton::Play),
            Widget::Pause => Some(TransportButton::Pause),
            Widget::Stop => Some(TransportButton::Stop),
            Widget::Next => Some(TransportButton::Next),
            Widget::Eject => Some(TransportButton::Eject),
            _ => None,
        }
    }

    /// The window button this widget is, if any (S02 §7).
    pub const fn window_button(self) -> Option<WindowButton> {
        match self {
            Widget::WindowMenu => Some(WindowButton::Menu),
            Widget::WindowMinimize => Some(WindowButton::Minimize),
            Widget::WindowShade => Some(WindowButton::Shade),
            Widget::WindowClose => Some(WindowButton::Close),
            _ => None,
        }
    }

    /// Whether this widget is one of the three continuous sliders.
    pub const fn is_slider(self) -> bool {
        matches!(
            self,
            Widget::PositionSlider | Widget::VolumeSlider | Widget::BalanceSlider
        )
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
                // The four window buttons are nested inside the title-bar band
                // (S02 §7); every other pair must be disjoint.
                let nested = (a == Widget::TitleBar && b.window_button().is_some())
                    || (b == Widget::TitleBar && a.window_button().is_some());
                if nested {
                    continue;
                }
                assert_eq!(
                    a.rect(&l).intersect(&b.rect(&l)),
                    None,
                    "{a:?} overlaps {b:?}"
                );
            }
        }
    }

    #[test]
    fn window_buttons_sit_inside_the_title_bar_and_win_the_hit_test() {
        let l = layout();
        for button in [
            WindowButton::Menu,
            WindowButton::Minimize,
            WindowButton::Shade,
            WindowButton::Close,
        ] {
            let r = l.window_button(button);
            assert_eq!(
                r.intersect(&l.title_bar),
                Some(r),
                "{button:?} must be inside the title bar"
            );
        }
        // The window buttons take priority over the band they sit in.
        assert_eq!(hit_test(&l, 10, 7), Some(Widget::WindowMenu));
        assert_eq!(hit_test(&l, 248, 7), Some(Widget::WindowMinimize));
        assert_eq!(hit_test(&l, 258, 7), Some(Widget::WindowShade));
        assert_eq!(hit_test(&l, 268, 7), Some(Widget::WindowClose));
        assert_eq!(hit_test(&l, 137, 7), Some(Widget::TitleBar));
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
