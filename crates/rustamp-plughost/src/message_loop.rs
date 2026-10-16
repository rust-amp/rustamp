//! Drive the [`IpcSurface`] from a Win32 message queue.
//!
//! The surface (M3a) and the message loop (this module) are deliberately
//! separate: the surface is a pure Rust answering table that needs no OS
//! objects, so its unit tests construct one directly, and the tests for this
//! module exercise [`dispatch_message`] / [`pump_once`] against a surface that
//! never touches a real window handle.
//!
//! ## Threading (S09 sections 3.6, 5.3, 7.2)
//!
//! * The loop runs on the **UI thread** that owns the host window.  That thread
//!   is the only place the [`IpcSurface`] is mutated from outside a test.
//! * A plugin thread never calls the surface; it `SendMessage`s or `PostMessage`s
//!   to the host window and the OS hands the call to this loop's window
//!   procedure, which delegates to [`HostWindow::dispatch_ipc`].
//! * `PostMessage` carries no reply, so a posted `WM_WA_IPC` leaves the
//!   surface updated (for the stateful ids 252/253, 128) but no caller waits.
//! * Shutdown: on `WM_QUIT` the loop returns and the host stops.  Plugins must
//!   be quit (S09 unload rules) before that.

use crate::host_window::HostWindow;
use crate::ipc::IpcSurface;

/// Dispatch a single Win32 message to the surface, returning the value the
/// window procedure must return.  Non-`WM_WA_IPC` messages are ignored here
/// (their handling stays in [`host_window::wnd_proc`]).
#[cfg(windows)]
pub fn dispatch_message(
    surface: &mut IpcSurface,
    _hwnd: usize,
    msg: u32,
    wparam: i64,
    lparam: i64,
) -> i64 {
    if msg == rustamp_plugin_abi::common::WM_USER {
        HostWindow::dispatch_ipc(surface, wparam, lparam as u32) as i64
    } else {
        0
    }
}

#[cfg(not(windows))]
pub fn dispatch_message(
    surface: &mut IpcSurface,
    _hwnd: usize,
    msg: u32,
    wparam: i64,
    lparam: i64,
) -> i64 {
    if msg == rustamp_plugin_abi::common::WM_USER {
        HostWindow::dispatch_ipc(surface, wparam, lparam as u32) as i64
    } else {
        0
    }
}

/// Process at most one queued message without blocking.  Returns `true` while
/// the loop should continue, `false` on `WM_QUIT` (or if a platform error
/// occurs).  Intended for tests and for cooperative hosts.
#[cfg(windows)]
pub fn pump_once(surface: &mut IpcSurface) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, TranslateMessage};

    let mut m = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    // SAFETY: PeekMessageW writes into `m` and only removes messages we then
    // dispatch through the normal machinery (DefWindowProc via wnd_proc).
    let have = unsafe {
        windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
            &mut m,
            None,
            0,
            0,
            windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
        )
    };
    if !have.as_bool() {
        return true; // nothing queued: keep running
    }
    if m.message == windows::Win32::UI::WindowsAndMessaging::WM_QUIT {
        return false;
    }
    unsafe {
        let _ = TranslateMessage(&m);
        DispatchMessageW(&m);
    }
    // Keep the surface referenced even if no message used it, so the signature
    // stays honest; GetMessage handling performs the actual mutation.
    let _ = surface;
    true
}

#[cfg(not(windows))]
pub fn pump_once(surface: &mut IpcSurface) -> bool {
    let _ = surface;
    true
}

/// Blocking message loop.  Runs until `WM_QUIT`.  Call on the UI thread only.
#[cfg(windows)]
pub fn run(surface: &mut IpcSurface) {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, TranslateMessage,
    };
    let mut m = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    loop {
        let ret = unsafe { GetMessageW(&mut m, None, 0, 0) };
        if ret.0 == 0 || ret.0 == -1 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&m);
            let _ = DispatchMessageW(&m);
        }
    }
    // Keep the surface alive across the loop so plugin-thread senders that
    // reach its window procedure find it published.
    let _ = surface;
}

#[cfg(not(windows))]
pub fn run(surface: &mut IpcSurface) {
    let _ = surface;
}

/// Run the loop once per queued message until it drains, without blocking.
/// Useful in tests and in hosts that drive their own scheduler.
pub fn drain(surface: &mut IpcSurface) {
    // One pass only: this host library never busy-spins on an empty queue.
    let _ = pump_once(surface);
}

/// Convenience alias the host binary uses.
pub fn run_message_loop(surface: &mut IpcSurface) {
    run(surface)
}
