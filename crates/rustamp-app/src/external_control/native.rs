// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Same-thread, lifetime-owned Win32 subclass. No callback borrows `App`, runs
//! a dialog, or calls into winit. Only the main HWND gets this subclass.

use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;
use std::sync::Arc;

use rustamp_core::state::PlayerState;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use super::{Request, UNSUPPORTED, answer, decode};
use crate::app::App;

type SubclassProc =
    unsafe extern "system" fn(*mut c_void, u32, usize, isize, usize, usize) -> isize;

#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(hwnd: *mut c_void, callback: SubclassProc, id: usize, data: usize) -> i32;
    fn RemoveWindowSubclass(hwnd: *mut c_void, callback: SubclassProc, id: usize) -> i32;
    fn DefSubclassProc(hwnd: *mut c_void, message: u32, wparam: usize, lparam: isize) -> isize;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn InSendMessageEx(reserved: *const c_void) -> u32;
}

struct Bridge {
    state: PlayerState,
    pending: Vec<Request>,
    attached: bool,
}

/// Owns both the callback allocation and a strong HWND owner. Not Send/Sync:
/// installation, polling, removal and destruction all occur on the UI thread.
pub(crate) struct ExternalControl {
    _window: Arc<Window>,
    hwnd: *mut c_void,
    bridge: Rc<RefCell<Bridge>>,
}

impl ExternalControl {
    pub(crate) fn attach(window: Arc<Window>, state: PlayerState) -> std::io::Result<Self> {
        let hwnd = match window.window_handle().map(|handle| handle.as_raw()) {
            Ok(RawWindowHandle::Win32(handle)) => handle.hwnd.get() as *mut c_void,
            _ => return Err(std::io::Error::other("main window has no Win32 handle")),
        };
        let this = Self {
            _window: window,
            hwnd,
            bridge: Rc::new(RefCell::new(Bridge {
                state,
                pending: Vec::new(),
                attached: false,
            })),
        };
        let data = (&*this.bridge as *const RefCell<Bridge>) as usize;
        // SAFETY: winit's window belongs to this thread. The Rc allocation
        // stays fixed until Drop removes the callback; `window` keeps HWND alive.
        if unsafe { SetWindowSubclass(hwnd, callback, data, data) } == 0 {
            return Err(std::io::Error::other(
                "could not install external-control subclass",
            ));
        }
        this.bridge.borrow_mut().attached = true;
        Ok(this)
    }

    /// Drained before the normal app pump, never while a native procedure is
    /// running. Shortcut activation is exactly the existing menu/key route.
    pub(crate) fn drain(&self, app: &mut App) {
        let pending = std::mem::take(&mut self.bridge.borrow_mut().pending);
        for request in pending {
            let result = match request {
                Request::Shortcut(action) => app
                    .activate_menu_item("External control", action)
                    .map(|_| ()),
                Request::PostedCommand(command) => app.submit(command),
                _ => continue,
            };
            if let Err(error) = result {
                app.note(format!("external control: {error}"));
            }
        }
    }

    pub(crate) fn publish(&self, state: PlayerState) {
        self.bridge.borrow_mut().state = state;
    }
}

impl Drop for ExternalControl {
    fn drop(&mut self) {
        if !self.bridge.borrow().attached {
            return;
        }
        let id = (&*self.bridge as *const RefCell<Bridge>) as usize;
        // SAFETY: same UI thread, HWND and allocation are still alive.
        // This also safely does nothing after WM_NCDESTROY removed it.
        if unsafe { RemoveWindowSubclass(self.hwnd, callback, id) } == 0 {
            // A failed removal must never leave a dangling callback data
            // pointer. Retain the small allocation rather than risk UAF.
            std::mem::forget(Rc::clone(&self.bridge));
        }
    }
}

// SAFETY: installed only by ExternalControl::attach on the owning UI thread;
// its Rc allocation remains alive until removal. No application code is called.
unsafe extern "system" fn callback(
    hwnd: *mut c_void,
    message: u32,
    wparam: usize,
    lparam: isize,
    id: usize,
    data: usize,
) -> isize {
    // SAFETY: the pointer is attach's live Rc allocation, never an IPC pointer.
    let bridge = unsafe { &*(data as *const RefCell<Bridge>) };
    if message == 0x0082 {
        // WM_NCDESTROY
        // SAFETY: the OS is destroying this HWND on its owner thread.
        if unsafe { RemoveWindowSubclass(hwnd, callback, id) } != 0
            && let Ok(mut state) = bridge.try_borrow_mut()
        {
            state.attached = false;
        }
    }
    let request = decode(message, wparam, lparam);
    if request == Request::NotOurs {
        // SAFETY: original parameters are forwarded along the subclass chain.
        return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    }
    let Ok(mut bridge) = bridge.try_borrow_mut() else {
        return UNSUPPORTED as isize;
    };
    let reply = match request {
        Request::Reply(value) => value,
        Request::Query(query) => answer(query, &bridge.state).unwrap_or(UNSUPPORTED),
        Request::Shortcut(_) => {
            bridge.pending.push(request);
            0 // WM_COMMAND has no reply contract (S10 §4 / §9).
        }
        Request::PostedCommand(_) => {
            // SAFETY: null is the required reserved parameter. Only queued
            // delivery is supported: S10 §13.11 leaves sent setter replies open.
            if unsafe { InSendMessageEx(std::ptr::null()) } == 0 {
                bridge.pending.push(request);
            }
            UNSUPPORTED
        }
        Request::Unsupported | Request::NotOurs => UNSUPPORTED,
    };
    // Preserve the classic unsigned-32-bit result shape, including -1 pan.
    reply as isize
}
