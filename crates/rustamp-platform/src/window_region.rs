// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The shape of a window: a skin's region, handed to the operating system.
//!
//! A skin's `region.txt` says which pixels of a window are visible (S17 §7);
//! the rest of the window rectangle is transparent. The operating system
//! expresses such a shape as a **region**: a set of rectangles that the
//! window manager clips the window to (`SetWindowRgn`).
//!
//! This module is the Win32 half and nothing more: it takes a list of
//! rectangles, builds a region out of them and gives it to a window. It
//! knows nothing about skins, masks or the player's windows — the caller
//! decides which rectangles a window has.
//!
//! # Who owns the region
//!
//! `SetWindowRgn` **takes the region over**: after a successful call the
//! *system* owns the handle and deletes it when the window no longer needs
//! it. The caller must not delete it then. A call that *fails* leaves the
//! handle with the caller, which then deletes it — otherwise every refused
//! apply would leak a GDI object.
//!
//! # Which coordinates
//!
//! A window region is expressed in **window** coordinates, not in the
//! coordinates of the client area (MSDN, `SetWindowRgn`). The rectangles
//! this module takes are the client's, so they are moved by the offset
//! between the two origins. The player's windows are borderless, so that
//! offset is zero; the translation keeps the module right for a window that
//! does have a frame.
//!
//! # The headless seam
//!
//! [`RegionRect`] is plain data: it can be built and asserted without an
//! operating system. Applying a region needs a window, and so does reading
//! one back ([`window_region_contains`]): both are VM-only.

/// One rectangle of a window's region, in the window's **client**
/// coordinates.
///
/// The right and the bottom edge are exclusive, so the rectangle covers the
/// pixels `left..right` and `top..bottom`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionRect {
    /// The left edge, inclusive.
    pub left: i32,
    /// The top edge, inclusive.
    pub top: i32,
    /// The right edge, exclusive.
    pub right: i32,
    /// The bottom edge, exclusive.
    pub bottom: i32,
}

impl RegionRect {
    /// The rectangle of a half-open run of pixels: the columns
    /// `x_start..x_end` of the row `y`.
    ///
    /// The run is half-open, so the rectangle is too: it covers the
    /// `x_end - x_start` pixels of that row and no other row.
    pub const fn run(x_start: u32, x_end: u32, y: u32) -> Self {
        Self {
            left: x_start as i32,
            top: y as i32,
            right: x_end as i32,
            bottom: y as i32 + 1,
        }
    }

    /// Whether the rectangle covers no pixel at all.
    pub const fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }
}

/// Applies `rects` as the shape of the window `raw_window`.
///
/// The rectangles are the window's **client** coordinates; the region the
/// operating system stores is in window coordinates (see the module docs).
///
/// An **empty** list clears the window's region: the window becomes a plain
/// rectangle again. That is the case for a skin without a `region.txt`, and
/// for a mask that covers no pixel at all — a window with an empty region
/// would be invisible, which is never what a caller means.
///
/// Returns whether the operating system took the region.
#[cfg(windows)]
pub fn apply_window_region(raw_window: isize, rects: &[RegionRect]) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        CombineRgn, CreateRectRgn, DeleteObject, RGN_OR, SetWindowRgn,
    };

    if raw_window == 0 {
        return false;
    }
    let hwnd = HWND(raw_window as *mut core::ffi::c_void);
    let (offset_x, offset_y) = client_origin_offset(hwnd);

    // The union of the rectangles: the first one is the region, every other
    // one is combined into it and then deleted.
    let mut region = None;
    for rect in rects {
        if rect.is_empty() {
            continue;
        }
        // SAFETY: a plain GDI call with plain coordinates; the handle is
        // ours until the window takes it.
        let piece = unsafe {
            CreateRectRgn(
                rect.left - offset_x,
                rect.top - offset_y,
                rect.right - offset_x,
                rect.bottom - offset_y,
            )
        };
        if piece.is_invalid() {
            continue;
        }
        match region {
            None => region = Some(piece),
            Some(destination) => {
                // The union of the two. The destination may also be the
                // first source (MSDN, `CombineRgn`).
                // SAFETY: both regions are live and owned by us.
                unsafe { CombineRgn(Some(destination), Some(destination), Some(piece), RGN_OR) };
                // SAFETY: the piece is ours, and its content is in the
                // destination now.
                let _ = unsafe { DeleteObject(piece.into()) };
            }
        }
    }

    let Some(region) = region else {
        // No rectangle: the window keeps its rectangle.
        // SAFETY: a null region clears the window's region.
        return unsafe { SetWindowRgn(hwnd, None, true) } != 0;
    };
    // SAFETY: the region is live and owned by us.
    if unsafe { SetWindowRgn(hwnd, Some(region), true) } != 0 {
        // The system owns the region now and deletes it when the window no
        // longer needs it; we must not delete it here.
        true
    } else {
        // The call failed, so the region never left our hands.
        // SAFETY: the region is ours and was not handed over.
        let _ = unsafe { DeleteObject(region.into()) };
        false
    }
}

/// Stub: there is no window region off Windows yet.
#[cfg(not(windows))]
pub fn apply_window_region(_raw_window: isize, _rects: &[RegionRect]) -> bool {
    false
}

/// Whether the point `(x, y)` of the client area is inside the window's
/// region.
///
/// `None` when the window has no region at all (or no handle): a window
/// without a region is a plain rectangle, so the question has no answer
/// rather than the answer "yes".
#[cfg(windows)]
pub fn window_region_contains(raw_window: isize, x: i32, y: i32) -> Option<bool> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{
        CreateRectRgn, DeleteObject, GetWindowRgn, PtInRegion, RGN_ERROR,
    };

    if raw_window == 0 {
        return None;
    }
    let hwnd = HWND(raw_window as *mut core::ffi::c_void);
    // SAFETY: a plain GDI call; the region is ours and is deleted below.
    let region = unsafe { CreateRectRgn(0, 0, 0, 0) };
    if region.is_invalid() {
        return None;
    }
    // SAFETY: the region is live and owned by us; the call fills it with a
    // copy of the window's region.
    let kind = unsafe { GetWindowRgn(hwnd, region) };
    if kind == RGN_ERROR {
        // The window has no region (or the call failed).
        // SAFETY: the region is ours.
        let _ = unsafe { DeleteObject(region.into()) };
        return None;
    }
    let (offset_x, offset_y) = client_origin_offset(hwnd);
    // SAFETY: the region is live; the point is in window coordinates.
    let inside = unsafe { PtInRegion(region, x - offset_x, y - offset_y) }.as_bool();
    // SAFETY: the region is ours and the copy is not needed any more.
    let _ = unsafe { DeleteObject(region.into()) };
    Some(inside)
}

/// Stub: there is no window region off Windows yet.
#[cfg(not(windows))]
pub fn window_region_contains(_raw_window: isize, _x: i32, _y: i32) -> Option<bool> {
    None
}

/// The offset of the client area's origin inside the window, in window
/// coordinates.
///
/// The window region is expressed in window coordinates (MSDN), while the
/// rectangles describe the client area. A borderless window has no offset;
/// a window with a frame has one, and the region has to follow it.
#[cfg(windows)]
fn client_origin_offset(hwnd: windows::Win32::Foundation::HWND) -> (i32, i32) {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::Graphics::Gdi::ClientToScreen;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let mut origin = POINT::default();
    // SAFETY: `origin` is a valid out-parameter.
    if !unsafe { ClientToScreen(hwnd, &mut origin) }.as_bool() {
        return (0, 0);
    }
    let mut window = RECT::default();
    // SAFETY: `window` is a valid out-parameter.
    if unsafe { GetWindowRect(hwnd, &mut window) }.is_err() {
        return (0, 0);
    }
    (origin.x - window.left, origin.y - window.top)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_of_pixels_is_one_rectangle() {
        let rect = RegionRect::run(10, 20, 3);
        assert_eq!(rect.left, 10);
        assert_eq!(rect.right, 20);
        assert_eq!(rect.top, 3);
        assert_eq!(rect.bottom, 4);
        // The run is half-open: ten pixels of one row.
        assert_eq!((rect.right - rect.left) * (rect.bottom - rect.top), 10);
    }

    #[test]
    fn a_rectangle_without_pixels_is_empty() {
        assert!(RegionRect::run(10, 10, 0).is_empty());
        assert!(!RegionRect::run(10, 11, 0).is_empty());
        assert!(
            RegionRect {
                left: 5,
                top: 0,
                right: 5,
                bottom: 10,
            }
            .is_empty()
        );
    }
}
