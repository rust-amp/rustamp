// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
/// Z-order management for the classic window group.
///
/// Winamp treats the three classic windows (main, equalizer, playlist editor)
/// as one player: activating any of them raises the whole group. This module
/// provides the Win32 call to raise a window in the Z-order without stealing
/// focus from the window that was clicked.
#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos,
};

/// Raises the other two classic windows when one gains focus.
///
/// `focused` is the window that just received focus (and must stay active **and
/// in front of the group**). `others` are the handles of the other two classic
/// windows; each is inserted **behind** the focused one (`SetWindowPos` with the
/// focused handle as the insert-after target) and without activation.
///
/// `HWND_TOP` would be wrong here: it would put the siblings above the window
/// the user just clicked. The first version did exactly that; the whole point of
/// this call is to move the group up while the clicked window stays on top of
/// it.
pub fn raise_classic_group(focused: isize, others: &[isize]) {
    if focused == 0 {
        return;
    }
    for &other in others {
        if other != 0 && other != focused {
            raise_behind(focused, other);
        }
    }
}

/// Inserts `window` directly behind `reference` without activating or resizing.
#[cfg(windows)]
fn raise_behind(reference: isize, window: isize) {
    if reference == 0 || window == 0 {
        return;
    }
    // SAFETY: both handles come from winit; the insert-after target is the
    // window that keeps the focus, so the sibling lands one step below it.
    let insert_after = HWND(reference as *mut _);
    let hwnd = HWND(window as *mut _);
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(insert_after),
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        );
    }
}

/// Stub: no Z-order management off Windows yet.
#[cfg(not(windows))]
fn raise_behind(_reference: isize, _window: isize) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raise_classic_group_skips_null_and_focused() {
        // This only tests the control flow; the actual Win32 call is a no-op
        // off-Windows and in the test environment.
        raise_classic_group(100, &[0, 100, 200]);
        raise_classic_group(0, &[100, 200]);
    }
}
