//! Drive the [`IpcSurface`] from a Win32 message queue.
//!
//! The surface (M3a) and the message loop (this module) are deliberately
//! separate: the surface is a pure Rust answering table that needs no OS
//! objects, so its unit tests construct one directly. Headless tests exercise
//! [`dispatch_message`] against standalone state. Message-pump entry points
//! take a shared [`HostWindow`] owner and hold no surface borrow during OS calls.
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

/// What one non-blocking pump step did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PumpStep {
    /// One message was taken off the queue and dispatched.
    Dispatched,
    /// The queue was empty (or the platform has no queue).
    Empty,
    /// `WM_QUIT` was taken off the queue: the caller must stop.
    Quit,
}

/// Take at most one queued message off the queue and dispatch it, without
/// blocking. The UI-thread owner is shared; no mutable surface borrow spans
/// PeekMessageW, TranslateMessage or dispatch, any of which can deliver
/// reentrant window messages.
#[cfg(windows)]
pub fn pump_step(_window: &HostWindow) -> PumpStep {
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
        return PumpStep::Empty;
    }
    if m.message == windows::Win32::UI::WindowsAndMessaging::WM_QUIT {
        return PumpStep::Quit;
    }
    // SAFETY: m is initialized by PeekMessageW; no surface borrow is alive.
    unsafe {
        let _ = TranslateMessage(&m);
        DispatchMessageW(&m);
    }
    PumpStep::Dispatched
}

#[cfg(not(windows))]
pub fn pump_step(_window: &HostWindow) -> PumpStep {
    PumpStep::Empty
}

/// Process at most one queued message without blocking.  Returns `true` while
/// the loop should continue, `false` on `WM_QUIT` (or if a platform error
/// occurs). Intended for cooperative hosts.
pub fn pump_once(window: &HostWindow) -> bool {
    pump_step(window) != PumpStep::Quit
}

/// Blocking UI-thread message loop until `WM_QUIT`, without a surface borrow.
#[cfg(windows)]
pub fn run(_window: &HostWindow) {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, TranslateMessage,
    };
    let mut m = windows::Win32::UI::WindowsAndMessaging::MSG::default();
    loop {
        // SAFETY: m is writable; the shared owner keeps state alive without a borrow.
        let ret = unsafe { GetMessageW(&mut m, None, 0, 0) };
        if ret.0 == 0 || ret.0 == -1 {
            break;
        }
        // SAFETY: m holds the retrieved message; no surface borrow spans dispatch.
        unsafe {
            let _ = TranslateMessage(&m);
            let _ = DispatchMessageW(&m);
        }
    }
}

#[cfg(not(windows))]
pub fn run(_window: &HostWindow) {}

/// Largest number of queued messages one [`pump_bounded`] call dispatches.
///
/// A budget rather than "until empty": a plugin that posts a message from its
/// own window procedure can refill the queue as fast as it is drained, and an
/// unbounded loop would never return to the caller. Sixteen matches the host
/// binary's inbound work budget.
pub const PUMP_BUDGET: usize = 16;

/// Dispatch up to `budget` queued messages without blocking.
///
/// Returns `false` as soon as `WM_QUIT` is seen (the caller must stop), `true`
/// otherwise — including when the queue was empty or the budget ran out. This
/// is the call to use while a plugin dialog is open (S09 §7.2 "config on the
/// UI thread"): it keeps the window answering `SendMessage` queries without
/// ever sleeping and without spinning on an empty queue.
pub fn pump_bounded(window: &HostWindow, budget: usize) -> bool {
    for _ in 0..budget {
        match pump_step(window) {
            PumpStep::Dispatched => {}
            // An empty queue ends the pass: this never spins and never sleeps.
            PumpStep::Empty => return true,
            PumpStep::Quit => return false,
        }
    }
    true
}

/// Dispatch the queued messages, up to [`PUMP_BUDGET`], without blocking.
/// Useful in tests and in hosts that drive their own scheduler.
pub fn drain(window: &HostWindow) {
    let _ = pump_bounded(window, PUMP_BUDGET);
}

/// Convenience alias the host binary uses.
pub fn run_message_loop(window: &HostWindow) {
    run(window)
}
