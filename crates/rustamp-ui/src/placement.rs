// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Where a classic window may sit: the clamp-to-screen rule.
//!
//! A saved position can point at a screen that is gone: the monitor was
//! unplugged, the resolution changed, or the configuration file was edited by
//! hand. The platform layer knows *which* screens exist
//! ([`rustamp_platform::DisplaySnapshot`]); this module owns the window-side
//! rule: **the window's title bar must stay reachable**.
//!
//! # The rule
//!
//! * The **reachable strip** of a classic window is its **title bar** - the
//!   only part of a borderless window the user can drag (S01 §2, S01 §4.1,
//!   S05 §1.1). It is 14 px high for the main window and the equalizer and
//!   20 px for the playlist editor ([`title_bar_height`]).
//! * A window is placed so that its reachable strip lies **entirely inside**
//!   a work area. The window itself may hang off the far edges of that work
//!   area: the title bar is the promise, not the whole window.
//! * A window whose strip is already inside a work area is **reachable** and
//!   is **not moved** (minimal intervention, the same policy the platform's
//!   own clamp uses).
//! * A work area too small to hold the strip pins the window to the work
//!   area's top-left corner - the most of the strip that can be reached at
//!   all.
//! * With no screen at all there is nothing to clamp against, and the saved
//!   position is kept unchanged (the platform's degradation rule).
//!
//! # What is measured and what is ours
//!
//! S05 §3.4 leaves the persistence of a window position across a monitor
//! change **open**, and the reference behaviour is unmeasured, so the rule
//! above is **Rustamp's own choice**. The numbers it uses are S01's (the
//! title bar heights) and the platform's (the work areas).
//!
//! The platform's own clamp ([`rustamp_platform::DisplaySnapshot::clamp_position`])
//! answers a different question - *which* screen the window belongs to and
//! whether it is on any screen at all - and this module composes with it:
//! the platform's answer is the input, the title-bar rule is the guarantee.
//!
//! [`clamp_stack_to_screen`] applies the rule to the three classic windows
//! of a docked stack: the windows that hang on the anchor come back in one
//! piece, so a player that returns from a vanished monitor shows its windows
//! stacked, not piled on the same pixel.

use rustamp_platform::{DisplaySnapshot, ScreenRect};

use crate::docking::{ClassicWindow, DockedStack, WindowKind, touches};
use crate::equalizer::EqualizerLayout;
use crate::layout::MainWindowLayout;
use crate::playlist_window::TITLE_BAR_HEIGHT;

/// The height of a classic window's title bar: the strip that has to stay
/// reachable, because it is the only part of a borderless window the user
/// can drag (S01 §2, S01 §4.1, S05 §1.1).
pub fn title_bar_height(kind: WindowKind) -> u32 {
    match kind {
        WindowKind::Main => MainWindowLayout::CLASSIC.title_bar.height,
        WindowKind::Equalizer => EqualizerLayout::CLASSIC.title_bar.height,
        WindowKind::Playlist => u32::try_from(TITLE_BAR_HEIGHT).unwrap_or(0),
    }
}

/// The reachable strip of a window: its title bar, in screen coordinates.
///
/// A shaded window is nothing but its title bar, so the strip is the whole
/// window there.
pub fn title_bar_rect(window: &ClassicWindow) -> ScreenRect {
    let height = title_bar_height(window.kind).min(window.height);
    ScreenRect {
        x: window.x,
        y: window.y,
        width: i32::try_from(window.width).unwrap_or(i32::MAX),
        height: i32::try_from(height).unwrap_or(i32::MAX),
    }
}

/// Is the window's title bar inside the work area?
///
/// The strip is clipped to what the work area can hold: a window wider than
/// the screen cannot show its whole title bar, and the part that fits is
/// what has to be inside.
pub fn title_bar_inside(window: &ClassicWindow, work_area: &ScreenRect) -> bool {
    if work_area.width <= 0 || work_area.height <= 0 {
        return false;
    }
    let bar = title_bar_rect(window);
    let bar_width = i64::from(bar.width).min(i64::from(work_area.width));
    let bar_height = i64::from(bar.height).min(i64::from(work_area.height));
    i64::from(bar.x) >= i64::from(work_area.x)
        && i64::from(bar.x) + bar_width <= i64::from(work_area.x) + i64::from(work_area.width)
        && i64::from(bar.y) >= i64::from(work_area.y)
        && i64::from(bar.y) + bar_height <= i64::from(work_area.y) + i64::from(work_area.height)
}

/// Places `window` so that its title bar lies inside `work_area`.
///
/// The window is moved only as far as the rule demands: a window whose title
/// bar is already inside the work area keeps its position. A work area that
/// cannot hold the strip (or is empty) leaves the window alone - there is no
/// range to clamp into.
pub fn clamp_into_work_area(window: ClassicWindow, work_area: &ScreenRect) -> ClassicWindow {
    if work_area.width <= 0 || work_area.height <= 0 {
        return window;
    }
    if title_bar_inside(&window, work_area) {
        return window;
    }
    let bar = title_bar_rect(&window);
    // The strip is clipped to what the work area can hold at all: a window
    // wider than the screen cannot show its whole title bar.
    let bar_width = i64::from(bar.width).min(i64::from(work_area.width));
    let bar_height = i64::from(bar.height).min(i64::from(work_area.height));
    let left = i64::from(work_area.x);
    let top = i64::from(work_area.y);
    let right = left + i64::from(work_area.width) - bar_width;
    let bottom = top + i64::from(work_area.height) - bar_height;
    let mut placed = window;
    placed.x = clamp_axis(window.x, left, right);
    placed.y = clamp_axis(window.y, top, bottom);
    placed
}

/// The placement of a window whose saved position may be off-screen.
///
/// The platform's rule comes first: a window that still touches a work area
/// is left where it is, a window that touches none is seated wholly inside
/// the primary monitor's work area
/// ([`DisplaySnapshot::clamp_position`]). Then the window-side rule applies:
/// the title bar of the placed window must be inside a work area. When it is
/// on no screen at all, the work area of the monitor it belongs to
/// ([`DisplaySnapshot::monitor_for_rect`]) is the one it is placed in.
///
/// A window whose title bar is already inside a work area is reachable and
/// keeps its place, whatever else it touches: the minimal intervention the
/// platform's own clamp uses.
///
/// With no monitor at all the window keeps its saved position: there is no
/// screen to place it on.
pub fn clamp_to_screen(snapshot: &DisplaySnapshot, window: ClassicWindow) -> ClassicWindow {
    let (x, y) = snapshot
        .clamp_position(window.x, window.y, window.width, window.height)
        .unwrap_or((window.x, window.y));
    let mut placed = window;
    placed.x = x;
    placed.y = y;
    if snapshot
        .monitors()
        .iter()
        .any(|monitor| title_bar_inside(&placed, &monitor.work_area))
    {
        return placed;
    }
    let rect = ScreenRect {
        x,
        y,
        width: i32::try_from(window.width).unwrap_or(i32::MAX),
        height: i32::try_from(window.height).unwrap_or(i32::MAX),
    };
    match snapshot.monitor_for_rect(&rect) {
        Some(monitor) => clamp_into_work_area(placed, &monitor.work_area),
        None => placed,
    }
}

/// Clamps one axis into `low..=high` without the panics of `i32::clamp` on an
/// inverted range, and without the overflow of an `i32` sum.
fn clamp_axis(value: i32, low: i64, high: i64) -> i32 {
    let value = i64::from(value);
    let clamped = value.max(low).min(high);
    i32::try_from(clamped).unwrap_or(if clamped > 0 { i32::MAX } else { i32::MIN })
}

/// Clamps the three classic windows of a docked stack onto the screens.
///
/// The main window is the anchor: it is clamped first, and a window that is
/// **docked** to it (their outside edges coincide, S05 §4.3) follows the
/// anchor. A group that pointed at a monitor that is gone therefore keeps its
/// shape instead of piling three windows on the same pixel. Every window is
/// then clamped on its own, because a docked window can still hang off the
/// far edge of the work area the anchor sits in - and the reachability of the
/// title strip wins over the shape of the group.
///
/// The windows that are not docked to the anchor keep their own place and
/// are clamped on their own, exactly as [`clamp_to_screen`] does.
///
/// With no screen at all every window keeps its place.
pub fn clamp_stack_to_screen(snapshot: &DisplaySnapshot, stack: DockedStack) -> DockedStack {
    let main = clamp_to_screen(snapshot, stack.main);
    let equalizer_source = if touches(stack.main, stack.equalizer) {
        shifted(stack.equalizer, delta(stack.main, main))
    } else {
        stack.equalizer
    };
    let equalizer = clamp_to_screen(snapshot, equalizer_source);
    let playlist_source = if touches(stack.equalizer, stack.playlist) {
        shifted(stack.playlist, delta(stack.equalizer, equalizer))
    } else {
        stack.playlist
    };
    let playlist = clamp_to_screen(snapshot, playlist_source);
    DockedStack {
        main,
        equalizer,
        playlist,
    }
}

/// The distance a window moved, without the overflow of an `i32` difference.
fn delta(from: ClassicWindow, to: ClassicWindow) -> (i32, i32) {
    (to.x.saturating_sub(from.x), to.y.saturating_sub(from.y))
}

/// A window moved by a distance, without the overflow of an `i32` sum.
fn shifted(window: ClassicWindow, (dx, dy): (i32, i32)) -> ClassicWindow {
    ClassicWindow {
        x: window.x.saturating_add(dx),
        y: window.y.saturating_add(dy),
        ..window
    }
}
