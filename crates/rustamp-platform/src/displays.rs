// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Display (monitor) discovery, scale factors and window-position clamping
//! (task card F4).
//!
//! [`DisplaySnapshot::capture`] enumerates the current monitors — their full
//! extent, their work area (the part free of the taskbar), which one is
//! primary, and each monitor's effective DPI. The rest of this module is
//! pure decision logic over snapshots, so every rule is unit-tested against
//! fake display sets without an OS object; the Win32 enumeration is the thin
//! backend.
//!
//! # What the spec settles and what is our choice
//!
//! * Settled (S05 §6, via the F1 contract): the window **keeps its physical
//!   pixel size** across monitors — it looks smaller on a high-DPI screen,
//!   and logical-size preservation is app-layer/S05 future work. So a scale
//!   factor chosen here is *reported*, not applied: it never triggers a
//!   re-render or a resize.
//! * Own choice (S05 §3.4 leaves persistence across a monitor change **open**,
//!   and the reference behaviour is unmeasured): that a saved position whose
//!   window no longer intersects any monitor's work area must be clamped at
//!   all; that "visible" means intersecting a **work area** (a window parked
//!   entirely under the taskbar is not visible); that a clamped window is
//!   seated **wholly** inside the target's work area; that the target is the
//!   **primary** monitor (else the first enumerated); and that a window that
//!   still touches a work area is left exactly where it was (minimal
//!   intervention).
//! * Own choice: the monitor's identity is its Win32 device name
//!   (`\\.\DISPLAY1`, ...) — the only per-monitor identity the API provides —
//!   and a monitor's scale factor is its **effective** DPI (`MDT_EFFECTIVE_DPI`)
//!   as a multiple of 96.
//!
//! # Choosing a scale factor
//!
//! A per-monitor-V2 window is scaled by the monitor it sits on, so "the
//! window's scale factor" is a question about position:
//! [`DisplaySnapshot::monitor_for_rect`] picks the monitor a window belongs
//! to (largest work-area intersection, ties to the primary), and
//! [`Monitor::scale_factor`] gives its scale as a multiple of 96 DPI
//! (96 → 1.0, 144 → 1.5). [`dpi_at`] answers the same question for a single
//! point.
//!
//! # Reporting a display change
//!
//! [`display_changes`] diffs two snapshots by monitor id: added and removed
//! monitors, a primary change, per-monitor scale (effective DPI) changes and
//! geometry changes. The window side polls [`DisplaySnapshot::capture`]
//! periodically from its pump and compares against its last snapshot
//! (see the task card's seam section) — Windows does not deliver a global
//! hot-plug message to an application, and `winit` surfaces only the
//! per-window `WM_DPICHANGED` (as `ScaleFactorChanged`), which does not name
//! the old and new scale or any monitor at all.
//!
//! # Headless seam
//!
//! [`clamp_saved_position`] and [`dpi_at`] are the two convenience entries
//! the window side calls: both capture the live display set and **degrade
//! instead of wedging** — when no display set can be captured (a non-Windows
//! target, a headless session, an OS failure) the saved position is returned
//! unchanged and the DPI comes back as `None`. Nothing here opens a window.

/// A rectangle in virtual-screen physical-pixel coordinates.
///
/// The virtual screen is the bounding box of all monitors; its top-left
/// corner is `(0, 0)` and monitors to the left or above it have negative
/// coordinates, exactly as Win32 reports them. Sizes are non-negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
}

impl ScreenRect {
    /// True when the two rectangles share at least one pixel.
    pub fn intersects(&self, other: &ScreenRect) -> bool {
        if self.width <= 0 || self.height <= 0 || other.width <= 0 || other.height <= 0 {
            return false;
        }
        // i64 keeps the edge sums overflow-free for any i32 input.
        let (ax, ay, aw, ah) = (
            i64::from(self.x),
            i64::from(self.y),
            i64::from(self.width),
            i64::from(self.height),
        );
        let (bx, by, bw, bh) = (
            i64::from(other.x),
            i64::from(other.y),
            i64::from(other.width),
            i64::from(other.height),
        );
        ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
    }

    /// True when the point lies inside (left/top inclusive, right/bottom
    /// exclusive).
    pub fn contains_point(&self, x: i32, y: i32) -> bool {
        let (ax, ay) = (i64::from(self.x), i64::from(self.y));
        let (px, py) = (i64::from(x), i64::from(y));
        self.width > 0
            && self.height > 0
            && px >= ax
            && px < ax + i64::from(self.width)
            && py >= ay
            && py < ay + i64::from(self.height)
    }
}

/// One display monitor, in virtual-screen physical pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    /// Stable identity of the monitor (the Win32 device name, e.g.
    /// `\\.\DISPLAY1`).
    pub id: String,
    /// The monitor's full extent, including any area under the taskbar.
    pub bounds: ScreenRect,
    /// The part of the monitor free of the taskbar and desktop edges.
    pub work_area: ScreenRect,
    /// True for the primary monitor.
    pub is_primary: bool,
    /// The monitor's effective DPI (96 = 100 %).
    pub dpi: u32,
}

impl Monitor {
    /// The monitor's scale factor as a multiple of 96 DPI (96 → 1.0,
    /// 144 → 1.5).
    pub fn scale_factor(&self) -> f64 {
        f64::from(self.dpi) / 96.0
    }
}

/// The set of monitors at one moment, in the OS's enumeration order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DisplaySnapshot {
    monitors: Vec<Monitor>,
}

impl DisplaySnapshot {
    /// Builds a snapshot from already-collected monitors (the Win32
    /// [`capture`](Self::capture) and the tests both use this).
    pub fn new(monitors: Vec<Monitor>) -> Self {
        Self { monitors }
    }

    /// The monitors, in enumeration order.
    pub fn monitors(&self) -> &[Monitor] {
        &self.monitors
    }

    /// True when no monitor was collected.
    pub fn is_empty(&self) -> bool {
        self.monitors.is_empty()
    }

    /// The primary monitor: the first one flagged primary, else the first
    /// enumerated (a display set without the flag is degenerate, and the
    /// fallback keeps every later decision total).
    pub fn primary(&self) -> Option<&Monitor> {
        self.monitors
            .iter()
            .find(|m| m.is_primary)
            .or_else(|| self.monitors.first())
    }

    /// The monitor a window `rect` belongs to: the one whose work area has
    /// the largest intersection with the window, a tie going to the primary
    /// and then to enumeration order. When the window misses every work
    /// area the primary is returned, so while any monitor exists a window
    /// always gets one.
    pub fn monitor_for_rect(&self, rect: &ScreenRect) -> Option<&Monitor> {
        let mut best_area: i64 = 0;
        let mut best: Option<usize> = None;
        for (i, m) in self.monitors.iter().enumerate() {
            let area = intersection_area(rect, &m.work_area);
            if area <= 0 {
                continue;
            }
            let take = match best {
                None => true,
                Some(j) => {
                    if area > best_area {
                        true
                    } else {
                        // Same overlap: the primary wins, otherwise the
                        // earlier enumeration order is kept.
                        area == best_area && m.is_primary && !self.monitors[j].is_primary
                    }
                }
            };
            if take {
                best = Some(i);
                best_area = area;
            }
        }
        match best {
            Some(i) => self.monitors.get(i),
            None => self.primary(),
        }
    }

    /// The monitor whose full bounds contain the point, if any (a point
    /// outside the virtual screen has no monitor).
    pub fn monitor_for_point(&self, x: i32, y: i32) -> Option<&Monitor> {
        self.monitors.iter().find(|m| m.bounds.contains_point(x, y))
    }

    /// The position a window of `width` × `height` should be placed at,
    /// given its saved position (`x`, `y`), so that it is visible on some
    /// monitor.
    ///
    /// A saved window that still intersects any monitor's work area is left
    /// exactly where it was (minimal intervention). Otherwise the window is
    /// seated wholly inside the target's work area: the target is the
    /// primary monitor when one exists, else the first enumerated, and the
    /// saved coordinates are clamped into the work area's range, so a
    /// window larger than the work area pins to the work area's top-left
    /// corner.
    ///
    /// Returns `None` when the snapshot has no monitor to clamp onto.
    pub fn clamp_position(&self, x: i32, y: i32, width: u32, height: u32) -> Option<(i32, i32)> {
        let saved = ScreenRect {
            x,
            y,
            width: i32::try_from(width).unwrap_or(i32::MAX),
            height: i32::try_from(height).unwrap_or(i32::MAX),
        };
        if self.monitors.iter().any(|m| saved.intersects(&m.work_area)) {
            return Some((x, y));
        }
        let wa = self.primary()?.work_area;
        let max_x = wa.x.saturating_add(wa.width).saturating_sub(saved.width);
        let max_y = wa.y.saturating_add(wa.height).saturating_sub(saved.height);
        // `clamp` panics on an inverted range; a window wider than the work
        // area is pinned to the work area's origin instead.
        let cx = if max_x < wa.x {
            wa.x
        } else {
            x.clamp(wa.x, max_x)
        };
        let cy = if max_y < wa.y {
            wa.y
        } else {
            y.clamp(wa.y, max_y)
        };
        Some((cx, cy))
    }

    /// Captures the live set of monitors: each monitor's bounds, work area,
    /// primary flag and effective DPI.
    ///
    /// Windows: `EnumDisplayMonitors` + `GetMonitorInfoW` +
    /// `GetDpiForMonitor(MDT_EFFECTIVE_DPI)`. A monitor whose geometry or
    /// DPI cannot be read fails the whole capture — a partial enumeration
    /// must not be reported as truth, because it would look like a removal.
    /// Off Windows this returns [`DisplayError::Unmanaged`].
    pub fn capture() -> Result<Self, DisplayError> {
        imp::capture()
    }
}

/// Why a display snapshot could not be taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DisplayError {
    /// Not a Windows target: display enumeration belongs to the toolkit, so
    /// there is nothing to enumerate (mirrors `DpiAwareness::Unmanaged`).
    #[error("display enumeration is a Win32 feature and unavailable on this target")]
    Unmanaged,
    /// `EnumDisplayMonitors` failed outright, or it reported no monitor.
    #[error("no display monitor could be enumerated")]
    EnumFailed,
    /// A monitor's geometry or DPI could not be read; carries the first
    /// failing call's code, so the failure stays diagnosable.
    #[error("a monitor's geometry or DPI could not be read (code {0})")]
    MonitorInfo(u32),
}

/// What changed between two snapshots, keyed by monitor id.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DisplayChanges {
    /// Monitors present in the new snapshot but not the old, in new-snapshot
    /// order.
    pub added: Vec<String>,
    /// Monitors present in the old snapshot but not the new, in old-snapshot
    /// order.
    pub removed: Vec<String>,
    /// The primary monitor changed identity: `(old, new)`, where `None`
    /// marks "no monitor was flagged primary" at that moment.
    pub primary_changed: Option<(Option<String>, Option<String>)>,
    /// Monitors present in both snapshots whose effective DPI changed:
    /// `(id, old dpi, new dpi)`, in old-snapshot order.
    pub scale_changed: Vec<(String, u32, u32)>,
    /// Monitors present in both snapshots whose bounds or work area moved or
    /// resized, in old-snapshot order.
    pub geometry_changed: Vec<String>,
}

impl DisplayChanges {
    /// True when nothing changed at all.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.primary_changed.is_none()
            && self.scale_changed.is_empty()
            && self.geometry_changed.is_empty()
    }
}

/// Compares two snapshots by monitor id.
///
/// Pure, so the diff is unit-tested against fake snapshots without any OS
/// object. The primary comparison uses the primary *flag* (not
/// [`DisplaySnapshot::primary`]'s first-enumerated fallback), so a flag
/// appearing or moving is a change, and two flagless snapshots are not.
pub fn display_changes(old: &DisplaySnapshot, new: &DisplaySnapshot) -> DisplayChanges {
    let mut out = DisplayChanges::default();
    for m in new.monitors() {
        if !old.monitors().iter().any(|o| o.id == m.id) {
            out.added.push(m.id.clone());
        }
    }
    for m in old.monitors() {
        match new.monitors().iter().find(|n| n.id == m.id) {
            None => out.removed.push(m.id.clone()),
            Some(n) => {
                if n.dpi != m.dpi {
                    out.scale_changed.push((m.id.clone(), m.dpi, n.dpi));
                }
                if n.bounds != m.bounds || n.work_area != m.work_area {
                    out.geometry_changed.push(m.id.clone());
                }
            }
        }
    }
    let old_primary = old
        .monitors()
        .iter()
        .find(|m| m.is_primary)
        .map(|m| m.id.clone());
    let new_primary = new
        .monitors()
        .iter()
        .find(|m| m.is_primary)
        .map(|m| m.id.clone());
    if old_primary != new_primary {
        out.primary_changed = Some((old_primary, new_primary));
    }
    out
}

/// The clamped position for a saved window position: captures the live
/// display set and applies [`DisplaySnapshot::clamp_position`].
///
/// Degrades instead of wedging: when no display set can be captured (a
/// non-Windows target, a headless session, an OS failure) or when the
/// snapshot has no monitor, the saved position is returned unchanged.
pub fn clamp_saved_position(x: i32, y: i32, width: u32, height: u32) -> (i32, i32) {
    match DisplaySnapshot::capture() {
        Ok(snapshot) => snapshot
            .clamp_position(x, y, width, height)
            .unwrap_or((x, y)),
        Err(_) => (x, y),
    }
}

/// The effective DPI at a virtual-screen point, if a monitor covers the
/// point.
///
/// Degrades instead of wedging like [`clamp_saved_position`]: a failed
/// capture or a point outside every monitor's bounds yields `None`.
pub fn dpi_at(x: i32, y: i32) -> Option<u32> {
    DisplaySnapshot::capture()
        .ok()?
        .monitor_for_point(x, y)
        .map(|m| m.dpi)
}

/// The pixel area shared by two rectangles (0 when they do not intersect).
fn intersection_area(a: &ScreenRect, b: &ScreenRect) -> i64 {
    if !a.intersects(b) {
        return 0;
    }
    let (ax, ay, aw, ah) = (
        i64::from(a.x),
        i64::from(a.y),
        i64::from(a.width),
        i64::from(a.height),
    );
    let (bx, by, bw, bh) = (
        i64::from(b.x),
        i64::from(b.y),
        i64::from(b.width),
        i64::from(b.height),
    );
    let w = (ax + aw).min(bx + bw) - ax.max(bx);
    let h = (ay + ah).min(by + bh) - ay.max(by);
    if w <= 0 || h <= 0 { 0 } else { w * h }
}

#[cfg(windows)]
mod imp {
    //! Win32 backend: `EnumDisplayMonitors` + `GetMonitorInfoW` +
    //! `GetDpiForMonitor(MDT_EFFECTIVE_DPI)`.

    use windows::Win32::Foundation::{GetLastError, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::Win32::UI::WindowsAndMessaging::MONITORINFOF_PRIMARY;
    use windows::core::BOOL;

    use super::{DisplayError, DisplaySnapshot, Monitor, ScreenRect};

    /// One callback result: a collected monitor, or the code of the read
    /// that failed for it.
    enum Entry {
        Monitor(Monitor),
        Failed(u32),
    }

    /// The `EnumDisplayMonitors` callback.
    unsafe extern "system" fn enum_monitor(
        hmonitor: HMONITOR,
        _hdc: HDC,
        _lprcmonitor: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        // SAFETY: `data` points at the `Vec<Entry>` owned by `capture`. The
        // callback runs synchronously on the calling thread inside
        // `EnumDisplayMonitors`, and no other reference to that vector is
        // live until `EnumDisplayMonitors` returns, so this is the only
        // mutable access for the whole call.
        let entries = unsafe { &mut *(data.0 as *mut Vec<Entry>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: `info` is fully initialised (`cbSize` names the EXW size)
        // and outlives the call; `hmonitor` comes from the enumeration.
        let got = unsafe {
            GetMonitorInfoW(
                hmonitor,
                &mut info as *mut MONITORINFOEXW as *mut MONITORINFO,
            )
        };
        if !got.as_bool() {
            // SAFETY: reading the last-error value is always sound and was
            // just set by the failing call.
            entries.push(Entry::Failed(unsafe { GetLastError().0 }));
            return BOOL::from(true);
        }
        let mut dpi_x = 0u32;
        let mut dpi_y = 0u32;
        // SAFETY: both outputs are valid `u32` that outlive the call; the
        // monitor handle comes from the enumeration.
        match unsafe { GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) } {
            Ok(()) => {}
            // The `HRESULT` code is stored as a Win32 error code for display.
            Err(error) => {
                entries.push(Entry::Failed(error.code().0 as u32));
                return BOOL::from(true);
            }
        }
        entries.push(Entry::Monitor(Monitor {
            id: device_name(&info.szDevice),
            bounds: rect_to(&info.monitorInfo.rcMonitor),
            work_area: rect_to(&info.monitorInfo.rcWork),
            is_primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
            dpi: dpi_x,
        }));
        BOOL::from(true)
    }

    /// Captures the live display set (see [`DisplaySnapshot::capture`]).
    pub fn capture() -> Result<DisplaySnapshot, DisplayError> {
        let mut entries: Vec<Entry> = Vec::new();
        // SAFETY: `enum_monitor` only writes through the pointer in `data`
        // (see its own SAFETY note); `entries` outlives the call; no window
        // is created.
        let enumerated = unsafe {
            EnumDisplayMonitors(
                None,
                None,
                Some(enum_monitor),
                LPARAM(&mut entries as *mut Vec<Entry> as isize),
            )
        };
        if !enumerated.as_bool() {
            return Err(DisplayError::EnumFailed);
        }
        let mut monitors = Vec::with_capacity(entries.len());
        for entry in entries {
            match entry {
                Entry::Monitor(monitor) => monitors.push(monitor),
                Entry::Failed(code) => return Err(DisplayError::MonitorInfo(code)),
            }
        }
        if monitors.is_empty() {
            return Err(DisplayError::EnumFailed);
        }
        Ok(DisplaySnapshot::new(monitors))
    }

    /// A Win32 `RECT` into a [`ScreenRect`] (saturating, so a malformed
    /// rect degrades to a small rect instead of panicking).
    fn rect_to(rect: &RECT) -> ScreenRect {
        ScreenRect {
            x: rect.left,
            y: rect.top,
            width: rect.right.saturating_sub(rect.left),
            height: rect.bottom.saturating_sub(rect.top),
        }
    }

    /// The NUL-terminated device name of a `MONITORINFOEXW`.
    fn device_name(sz: &[u16; 32]) -> String {
        let end = sz.iter().position(|&unit| unit == 0).unwrap_or(sz.len());
        String::from_utf16_lossy(&sz[..end])
    }
}

#[cfg(not(windows))]
mod imp {
    //! Non-Windows stub: display enumeration is a Win32 feature for now
    //! (same shape as `single_instance`'s stub).

    use super::{DisplayError, DisplaySnapshot};

    pub fn capture() -> Result<DisplaySnapshot, DisplayError> {
        Err(DisplayError::Unmanaged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake monitor: full rect `x,y,w,h` with a 48-px taskbar on the
    /// bottom (work area = `h - 48`).
    fn monitor(id: &str, x: i32, y: i32, w: i32, h: i32, primary: bool, dpi: u32) -> Monitor {
        Monitor {
            id: id.to_owned(),
            bounds: ScreenRect {
                x,
                y,
                width: w,
                height: h,
            },
            work_area: ScreenRect {
                x,
                y,
                width: w,
                height: h - 48,
            },
            is_primary: primary,
            dpi,
        }
    }

    /// Primary 1920x1080 at the origin, secondary 1920x1080 to its right.
    fn two_monitors() -> DisplaySnapshot {
        DisplaySnapshot::new(vec![
            monitor("a", 0, 0, 1920, 1080, true, 96),
            monitor("b", 1920, 0, 1920, 1080, false, 96),
        ])
    }

    #[test]
    fn a_saved_position_off_every_monitor_is_clamped_onto_the_primary() {
        let displays = two_monitors();
        // (5000, 4000) is far right and far below both work areas: both
        // coordinates clamp into the primary work area (1920 x 1032), so the
        // window seats at its bottom-right corner.
        assert_eq!(
            displays.clamp_position(5000, 4000, 275, 116),
            Some((1645, 916)),
            "an orphaned window is clamped to the primary work area's range"
        );
        // Off to the left and above clamps to the work area's origin.
        assert_eq!(
            displays.clamp_position(-5000, -4000, 275, 116),
            Some((0, 0))
        );
    }

    #[test]
    fn a_saved_position_inside_a_work_area_is_left_alone() {
        let displays = two_monitors();
        assert_eq!(
            displays.clamp_position(100, 200, 275, 116),
            Some((100, 200))
        );
        // On the secondary monitor, away from the primary: untouched.
        assert_eq!(
            displays.clamp_position(2500, 600, 275, 116),
            Some((2500, 600))
        );
    }

    #[test]
    fn a_saved_position_partly_on_a_work_area_is_left_alone() {
        let displays = two_monitors();
        // The window straddles the primary's right edge onto the secondary:
        // it still touches a work area, so it is not moved.
        assert_eq!(
            displays.clamp_position(1900, 100, 275, 116),
            Some((1900, 100))
        );
        // A window hanging off the bottom edge into the taskbar band (work
        // area ends at y = 1032) still intersects the work area (1000..1032).
        assert_eq!(
            displays.clamp_position(100, 1000, 275, 116),
            Some((100, 1000))
        );
    }

    #[test]
    fn a_window_larger_than_the_work_area_pins_to_the_work_area_origin() {
        let displays = DisplaySnapshot::new(vec![monitor("a", 0, 0, 1920, 1080, true, 96)]);
        assert_eq!(
            displays.clamp_position(4000, 4000, 4000, 3000),
            Some((0, 0)),
            "an oversized window pins to the top-left corner instead of panicking"
        );
    }

    #[test]
    fn clamping_without_monitors_yields_nothing() {
        let displays = DisplaySnapshot::new(Vec::new());
        assert_eq!(displays.clamp_position(10, 20, 275, 116), None);
        assert_eq!(displays.primary(), None);
        assert!(displays.is_empty());
    }

    #[test]
    fn a_saved_position_off_every_monitor_lands_wholly_inside_the_target() {
        let displays = two_monitors();
        // Every position below is genuinely orphaned (a 275 x 116 window
        // there intersects no work area), so each one clamps onto the
        // primary work area instead of being left alone.
        for (x, y) in [
            (-1000, -1000),
            (10_000, 10_000),
            (-2000, 5000),
            (12_345, -1),
        ] {
            let (cx, cy) = displays.clamp_position(x, y, 275, 116).expect("a clamp");
            let rect = ScreenRect {
                x: cx,
                y: cy,
                width: 275,
                height: 116,
            };
            assert!(
                rect.intersects(&displays.monitors()[0].work_area),
                "({x}, {y}) -> ({cx}, {cy}) must land inside the primary work area"
            );
            assert!(
                rect.x >= 0 && rect.y >= 0 && rect.x + 275 <= 1920 && rect.y + 116 <= 1032,
                "the clamped window ({cx}, {cy}) is wholly inside the work area"
            );
        }
    }

    #[test]
    fn a_removed_monitor_is_reported_and_clamping_still_works() {
        let old = two_monitors();
        let new = DisplaySnapshot::new(vec![monitor("a", 0, 0, 1920, 1080, true, 96)]);
        let changes = display_changes(&old, &new);
        assert_eq!(changes.removed, vec!["b".to_owned()]);
        assert!(changes.added.is_empty());
        assert!(changes.primary_changed.is_none());
        // The window that lived on the removed monitor (x = 2500) is now
        // orphaned: its x clamps to the primary work area's right edge and
        // its y is kept — no panic, no second monitor.
        assert_eq!(
            new.clamp_position(2500, 300, 275, 116),
            Some((1645, 300)),
            "a position on the removed monitor is re-seated on the primary"
        );
    }

    #[test]
    fn an_added_monitor_is_reported() {
        let old = DisplaySnapshot::new(vec![monitor("a", 0, 0, 1920, 1080, true, 96)]);
        let new = two_monitors();
        assert_eq!(display_changes(&old, &new).added, vec!["b".to_owned()]);
    }

    #[test]
    fn a_primary_monitor_change_is_detected() {
        let old = two_monitors(); // primary "a"
        let new = DisplaySnapshot::new(vec![
            monitor("a", 0, 0, 1920, 1080, false, 96),
            monitor("b", 1920, 0, 1920, 1080, true, 96),
        ]);
        let changes = display_changes(&old, &new);
        assert_eq!(
            changes.primary_changed,
            Some((Some("a".to_owned()), Some("b".to_owned())))
        );
        assert!(!changes.is_empty());
        // And the clamp target follows the new primary: the new work area
        // spans x 1920..3840, y 0..1032.
        assert_eq!(
            new.clamp_position(5000, 4000, 275, 116),
            Some((3565, 916)),
            "the orphaned window now clamps onto the new primary"
        );
    }

    #[test]
    fn a_scale_change_is_reported() {
        let old = two_monitors();
        let new = DisplaySnapshot::new(vec![
            monitor("a", 0, 0, 1920, 1080, true, 96),
            monitor("b", 1920, 0, 1920, 1080, false, 144),
        ]);
        let changes = display_changes(&old, &new);
        assert_eq!(
            changes.scale_changed,
            vec![("b".to_owned(), 96, 144)],
            "the secondary's 100% -> 150% is the only scale change"
        );
        assert!(changes.added.is_empty());
        assert!(changes.removed.is_empty());
        assert!(changes.primary_changed.is_none());
    }

    #[test]
    fn a_geometry_change_is_reported() {
        let old = two_monitors();
        let new = DisplaySnapshot::new(vec![
            monitor("a", 0, 0, 1920, 1080, true, 96),
            monitor("b", 2560, 0, 1920, 1080, false, 96), // moved right
        ]);
        let changes = display_changes(&old, &new);
        assert_eq!(changes.geometry_changed, vec!["b".to_owned()]);
        assert!(changes.scale_changed.is_empty());
    }

    #[test]
    fn an_unchanged_display_set_reports_no_changes() {
        let old = two_monitors();
        let new = two_monitors();
        let changes = display_changes(&old, &new);
        assert!(changes.is_empty(), "{changes:?}");
    }

    #[test]
    fn monitor_for_rect_prefers_the_largest_work_area_intersection() {
        let displays = two_monitors();
        // Mostly on the secondary.
        let rect = ScreenRect {
            x: 2000,
            y: 100,
            width: 275,
            height: 116,
        };
        assert_eq!(
            displays.monitor_for_rect(&rect).map(|m| m.id.as_str()),
            Some("b")
        );
        // Mostly on the primary.
        let rect = ScreenRect {
            x: 1700,
            y: 100,
            width: 275,
            height: 116,
        };
        assert_eq!(
            displays.monitor_for_rect(&rect).map(|m| m.id.as_str()),
            Some("a")
        );
    }

    #[test]
    fn monitor_for_rect_breaks_ties_by_the_primary() {
        // A window exactly half on each work area: the primary wins.
        let displays = two_monitors();
        let rect = ScreenRect {
            x: 960,
            y: 100,
            width: 1920,
            height: 116,
        };
        assert_eq!(
            displays.monitor_for_rect(&rect).map(|m| m.id.as_str()),
            Some("a")
        );
    }

    #[test]
    fn a_window_missing_every_work_area_gets_the_primary() {
        let displays = two_monitors();
        let rect = ScreenRect {
            x: 5000,
            y: 4000,
            width: 275,
            height: 116,
        };
        assert_eq!(
            displays.monitor_for_rect(&rect).map(|m| m.id.as_str()),
            Some("a"),
            "no intersection falls back to the primary"
        );
    }

    #[test]
    fn scale_factor_is_dpi_over_ninety_six() {
        assert_eq!(monitor("a", 0, 0, 100, 100, true, 96).scale_factor(), 1.0);
        assert_eq!(monitor("a", 0, 0, 100, 100, true, 144).scale_factor(), 1.5);
        assert_eq!(monitor("a", 0, 0, 100, 100, true, 192).scale_factor(), 2.0);
    }

    #[test]
    fn a_point_on_the_secondary_finds_the_secondary() {
        let displays = two_monitors();
        assert_eq!(
            displays.monitor_for_point(2000, 100).map(|m| m.id.as_str()),
            Some("b")
        );
        assert_eq!(
            displays.monitor_for_point(50, 50).map(|m| m.id.as_str()),
            Some("a")
        );
        assert_eq!(displays.monitor_for_point(10_000, 10_000), None);
        assert_eq!(displays.monitor_for_point(-1, 50), None);
    }

    #[test]
    fn the_primary_falls_back_to_the_first_enumerated() {
        let displays = DisplaySnapshot::new(vec![
            monitor("a", 0, 0, 1920, 1080, false, 96),
            monitor("b", 1920, 0, 1920, 1080, false, 96),
        ]);
        assert_eq!(displays.primary().map(|m| m.id.as_str()), Some("a"));
        // ... and a flagless -> flagged change is still a primary change.
        let flagged = DisplaySnapshot::new(vec![
            monitor("a", 0, 0, 1920, 1080, false, 96),
            monitor("b", 1920, 0, 1920, 1080, true, 96),
        ]);
        assert_eq!(
            display_changes(&displays, &flagged).primary_changed,
            Some((None, Some("b".to_owned())))
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_real_capture_reports_a_sane_display_set() {
        // The same session-0 carve-out as the DPI test in `dpi.rs`: the
        // PowerShell Direct test host runs with no desktop to enumerate,
        // where a typed error is the contract; an interactive session (the
        // test VM's auto-logon session with its virtual monitor) must
        // enumerate.
        let session_name = std::env::var("SESSIONNAME").unwrap_or_default();
        let interactive =
            !session_name.is_empty() && !session_name.eq_ignore_ascii_case("Services");

        match DisplaySnapshot::capture() {
            Ok(snapshot) => {
                assert!(!snapshot.is_empty(), "a capture never yields an empty set");
                let primaries = snapshot.monitors().iter().filter(|m| m.is_primary).count();
                if interactive {
                    assert_eq!(
                        primaries, 1,
                        "exactly one monitor is flagged primary: {snapshot:?}"
                    );
                }
                for m in snapshot.monitors() {
                    assert!(!m.id.is_empty(), "every monitor carries its device name");
                    assert!(m.bounds.width > 0 && m.bounds.height > 0);
                    assert!(m.work_area.width > 0 && m.work_area.height > 0);
                    assert!(
                        m.work_area.intersects(&m.bounds),
                        "the work area sits inside the monitor: {m:?}"
                    );
                    assert!(m.dpi > 0, "every monitor reports a DPI: {m:?}");
                }
            }
            Err(error) => {
                assert!(
                    !interactive,
                    "the display capture failed in an interactive session \
                     (SESSIONNAME={session_name:?}): {error}"
                );
                eprintln!(
                    "note: no display capture in this non-interactive session \
                     (SESSIONNAME={session_name:?}, {error}); there is no desktop here, \
                     which is where the test host runs"
                );
            }
        }
    }
}
