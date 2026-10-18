// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! OS geometry of a spawned player's windows, for the VM-only window tests.
//!
//! Shared by `headless.rs` (main window) and `playlist_window.rs` (main plus
//! playlist editor): find a spawned child's top-level windows by process id
//! and title, then read back their title, class, client size and outer
//! origin. The `user32` surface is declared by hand, mirroring
//! `plughost_e2e.rs`, so this adds no dependency — and the assertions
//! themselves open no window.
//!
//! Every read here pairs the wide API with a wide buffer (`GetWindowTextW`
//! / `GetClassNameW` into a `u16` array): the A/W-correct pattern of task
//! card N12. A reader that mixes variants (a wide API handed an ANSI
//! buffer) truncates every string to its first character and can never
//! match the exact multi-character titles `find_window` waits for.
//!
//! Windows-only: the headless test VM is Windows. Including tests declare
//! this module behind `#[cfg(windows)]` and skip the geometry half
//! elsewhere.
#![cfg(windows)]

use std::ffi::c_void;
use std::time::{Duration, Instant};

/// A Win32 `RECT`, as `GetClientRect`/`GetWindowRect` fill it.
#[repr(C)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn EnumWindows(
        enum_proc: unsafe extern "system" fn(hwnd: *mut c_void, lparam: isize) -> i32,
        lparam: isize,
    ) -> i32;
    fn GetWindowThreadProcessId(hwnd: *mut c_void, pid: *mut u32) -> u32;
    fn GetWindowTextW(hwnd: *mut c_void, text: *mut u16, max: i32) -> i32;
    fn GetClassNameW(hwnd: *mut c_void, text: *mut u16, max: i32) -> i32;
    fn GetClientRect(hwnd: *mut c_void, rect: *mut Rect) -> i32;
    fn GetWindowRect(hwnd: *mut c_void, rect: *mut Rect) -> i32;
    fn IsWindowVisible(hwnd: *mut c_void) -> i32;
}

/// Every top-level window handle, collected without touching any window.
fn top_level_windows() -> Vec<*mut c_void> {
    unsafe extern "system" fn collect(hwnd: *mut c_void, lparam: isize) -> i32 {
        // SAFETY: `lparam` is the `Vec` below, alive for the whole
        // enumeration; pushing a handle performs no window call.
        let out = unsafe { &mut *(lparam as *mut Vec<*mut c_void>) };
        out.push(hwnd);
        1 // TRUE: keep enumerating.
    }
    let mut out: Vec<*mut c_void> = Vec::new();
    // SAFETY: `collect` only pushes into `out`, which outlives the call;
    // plain handles cross, nothing is dereferenced.
    let ok = unsafe { EnumWindows(collect, &mut out as *mut _ as isize) };
    assert_ne!(
        ok, 0,
        "EnumWindows failed while looking for the player windows"
    );
    out
}

/// The window's title, read with the wide API into a wide buffer
/// (`GetWindowTextW` + a `u16` array): the A/W-correct pattern of task card
/// N12. `find_window` matches this exactly, so a reader that truncates to
/// one character can never pass unnoticed.
pub fn window_title(hwnd: *mut c_void) -> String {
    let mut buffer = [0u16; 256];
    // SAFETY: `buffer` is a valid 256-wide out-buffer. Only windows owned by
    // the spawned child are read, and those answer promptly.
    let len = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

/// The window's class name, read with the wide API into a wide buffer
/// (`GetClassNameW` + a `u16` array): the same A/W-correct pattern as
/// [`window_title`] (task card N12). A one-character result is the
/// signature of a wide/ANSI mismatch at the Win32 boundary, never of a
/// one-character class.
pub fn window_class(hwnd: *mut c_void) -> String {
    let mut buffer = [0u16; 256];
    // SAFETY: `buffer` is a valid 256-wide out-buffer. Only windows owned by
    // the spawned child are read, and those answer promptly.
    let len = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

fn owner_pid(hwnd: *mut c_void) -> u32 {
    let mut pid = 0u32;
    // SAFETY: `pid` is a valid out-pointer; the handle came from the OS.
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }
    pid
}

/// The visible top-level window titled `title` owned by `pid`, waited for
/// until `deadline`.
///
/// Panics instead of passing vacuously: a missing window means the player
/// never opened it.
pub fn find_window(pid: u32, title: &str, deadline: Duration) -> *mut c_void {
    let start = Instant::now();
    loop {
        let mut matches: Vec<*mut c_void> = Vec::new();
        for hwnd in top_level_windows() {
            if owner_pid(hwnd) != pid || window_title(hwnd) != title {
                continue;
            }
            // SAFETY: the handle came from the OS enumeration above.
            if unsafe { IsWindowVisible(hwnd) } == 0 {
                continue;
            }
            matches.push(hwnd);
        }
        if matches.len() == 1 {
            return matches[0];
        }
        if start.elapsed() >= deadline {
            panic!(
                "expected exactly one visible {title:?} window for pid {pid}, found {}",
                matches.len()
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The client size of `hwnd`, in physical pixels.
pub fn client_size(hwnd: *mut c_void) -> (i32, i32) {
    let mut rect = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `rect` is a valid out-pointer; the handle was found above.
    let ok = unsafe { GetClientRect(hwnd, &mut rect) };
    assert_ne!(ok, 0, "GetClientRect failed on the player window");
    (rect.right - rect.left, rect.bottom - rect.top)
}

/// The outer (screen) origin and extent of `hwnd`, in physical pixels.
pub fn outer_rect(hwnd: *mut c_void) -> (i32, i32, i32, i32) {
    let mut rect = Rect {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `rect` is a valid out-pointer; the handle was found above.
    let ok = unsafe { GetWindowRect(hwnd, &mut rect) };
    assert_ne!(ok, 0, "GetWindowRect failed on the player window");
    (rect.left, rect.top, rect.right, rect.bottom)
}
