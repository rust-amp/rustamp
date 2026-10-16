//! Host window for the 32-bit plugin host.
//!
//! S09 sections 1 and 8 fix the main window's class name as **`Winamp v1.x`**
//! (see [`rustamp_plugin_abi::common::CLASS_MAIN`]); the host answers classic
//! `WM_WA_IPC` messages on a window of exactly that class, and plugins reach
//! it through the handle stored in [`IpcSurface::main_hwnd`] (the value the
//! window-selector ids, e.g. IPC 260 `wParam == 0`, report back).
//!
//! # Threading and lifetime (S09 sections 3.6, 4.2, 5.3, 7.2)
//!
//! * **The window is owned by exactly one thread: the UI thread of
//!   `rustamp-plughost32`.**  Every Win32 message is processed on that
//!   thread; `RegisterClassW`, `CreateWindowExW`, `GetMessageW`/`PeekMessageW`
//!   and `DestroyWindow` all run there.
//! * **Plugins run on their own threads** (decode, vis, DSP) and reach the
//!   window only through `SendMessage`/`PostMessage`.  The OS marshals a
//!   `SendMessage` from another thread into the owning thread and blocks the
//!   caller until the window procedure returns, so a plugin that asks a query
//!   gets the reply as the message result without the host having to do any
//!   locking.
//! * **This surface is not re-entrant.**  A plugin running on the UI thread
//!   (a subclass or a `gen_` plug-in callback) may call `IpcSurface::answer`
//!   directly, but cross-thread access requires holding the UI thread; the
//!   window procedure below is the single point where plugin-thread calls meet
//!   the surface, via the pointer stashed in `GWLP_USERDATA`.
//! * **Lifetime:** the window (and therefore the handle plugins hold) must
//!   outlive every loaded plugin.  It is created before any DLL is loaded and
//!   destroyed only after every `quit` has run.

use crate::ipc::IpcSurface;

/// Window class name the ABI fixes for the main / "Winamp v1.x" window
/// (S09 sections 1 and 8).  Taken from [`rustamp_plugin_abi::common::CLASS_MAIN`],
/// not invented here.
pub const HOST_WINDOW_CLASS: &str = "Winamp v1.x";

/// The same value as a UTF-16 buffer, ready for `RegisterClassW`/`CreateWindowExW`.
#[cfg(windows)]
pub fn host_window_class_wide() -> Vec<u16> {
    let mut v: Vec<u16> = HOST_WINDOW_CLASS.encode_utf16().collect();
    v.push(0);
    v
}

#[derive(Debug)]
pub enum HostWindowError {
    Message(String),
}

impl std::fmt::Display for HostWindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for HostWindowError {}

/// The host's main window.  Owns a raw window handle (`HWND`); the handle is
/// also published into the [`IpcSurface`] so the IPC selectors can report it.
pub struct HostWindow {
    #[cfg(windows)]
    hwnd: windows::Win32::Foundation::HWND,
    #[cfg(not(windows))]
    hwnd: usize,
}

impl HostWindow {
    /// Class name the ABI fixes (S09 sections 1, 8).
    pub const CLASS_NAME: &'static str = HOST_WINDOW_CLASS;

    /// Headless accessor for tests that never create a real window.
    pub fn hwnd_raw(&self) -> usize {
        #[cfg(windows)]
        {
            self.hwnd.0 as usize
        }
        #[cfg(not(windows))]
        {
            self.hwnd
        }
    }

    /// Dispatch one classic `WM_WA_IPC` query to the shared surface and return
    /// the raw 32-bit reply the message result must carry.  This is the single
    /// dispatch entry point, used both by the real window procedure and by the
    /// in-process test plugin, so the two can never drift apart.
    pub fn dispatch_ipc(surface: &mut IpcSurface, wparam: i64, ipc_id: u32) -> u32 {
        surface.answer(ipc_id, wparam).raw()
    }

    /// Register the window class once.  Idempotent across calls.
    #[cfg(windows)]
    pub fn register_class() -> Result<(), HostWindowError> {
        use std::sync::OnceLock;
        static DONE: OnceLock<Result<(), String>> = OnceLock::new();
        let r = DONE.get_or_init(register_class_inner);
        r.clone().map_err(HostWindowError::Message)
    }

    #[cfg(windows)]
    pub fn create(surface: &mut IpcSurface) -> Result<Self, HostWindowError> {
        Self::register_class()?;
        create_window_inner(surface)
    }

    #[cfg(not(windows))]
    pub fn register_class() -> Result<(), HostWindowError> {
        Err(HostWindowError::Message("windows-only".into()))
    }

    #[cfg(not(windows))]
    pub fn create(surface: &mut IpcSurface) -> Result<Self, HostWindowError> {
        let _ = surface;
        Err(HostWindowError::Message("windows-only".into()))
    }

    /// Hide or drop the handle (the OS window is destroyed by `Drop`).
    pub fn destroy(self) {
        // Drop runs the destructor.
        drop(self)
    }
}

#[cfg(windows)]
impl Drop for HostWindow {
    fn drop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

// ----------------------------------------------------------------------------
// Windows-only implementation details.
// ----------------------------------------------------------------------------

#[cfg(windows)]
fn register_class_inner() -> Result<(), String> {
    use windows::Win32::Foundation::HINSTANCE;
    use windows::Win32::UI::WindowsAndMessaging::{RegisterClassW, WNDCLASSW};
    use windows::core::PCWSTR;

    let class_wide = host_window_class_wide();
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: HINSTANCE(std::ptr::null_mut()),
        lpszClassName: PCWSTR(class_wide.as_ptr()),
        ..Default::default()
    };
    // SAFETY: the structure points at a NUL-terminated wide string that lives
    // for the duration of the call; the OS copies what it needs.
    let atom = unsafe { RegisterClassW(&wc) };
    if atom == 0 {
        Err("RegisterClassW failed".to_string())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn create_window_inner(surface: &mut IpcSurface) -> Result<HostWindow, HostWindowError> {
    use windows::Win32::UI::WindowsAndMessaging::CreateWindowExW;
    use windows::core::PCWSTR;

    let title: Vec<u16> = "Rustamp plughost"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let class_wide = host_window_class_wide();

    // SAFETY: the window class is registered above; `surface` outlives the
    // window.  The pointer is stashed during `WM_NCCREATE` (see `wnd_proc`).
    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            PCWSTR(class_wide.as_ptr()),
            PCWSTR(title.as_ptr()),
            Default::default(), // not WS_VISIBLE: hidden host window
            0,
            0,
            0,
            0,
            None,
            None,
            Some(windows::Win32::Foundation::HINSTANCE(std::ptr::null_mut())),
            Some(surface as *mut IpcSurface as *mut std::ffi::c_void),
        )
    }
    .map_err(|e| HostWindowError::Message(e.to_string()))?;

    surface.main_hwnd = hwnd.0 as usize as u32;
    // The EQ / playlist / video / minibrowser handles default to 0 (unset).
    // The menu handle is likewise 0 until a menu is built, which is honest:
    // an unset handle reports 0.

    Ok(HostWindow { hwnd })
}

#[cfg(windows)]
unsafe extern "system" fn wnd_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, GWLP_USERDATA, GetWindowLongPtrW, PostQuitMessage, SetWindowLongPtrW,
        WM_DESTROY, WM_NCCREATE, WM_USER,
    };

    match msg {
        x if x == WM_NCCREATE => {
            // SAFETY: on WM_NCCREATE, lParam points to a CREATESTRUCT whose
            // lpCreateParams we set to the IpcSurface pointer in create_window_inner.
            let cs = lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
            if !cs.is_null() {
                let surface_ptr = unsafe { (*cs).lpCreateParams };
                #[cfg(target_pointer_width = "64")]
                let value = surface_ptr as isize;
                #[cfg(target_pointer_width = "32")]
                let value = surface_ptr as i32;
                unsafe {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, value);
                }
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_USER => {
            let surface_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut IpcSurface;
            if surface_ptr.is_null() {
                return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
            }
            // Classic IPC: the IPC id rides in LPARAM (the second parameter),
            // the argument in WPARAM.  S09 section 8 item 2.
            let ipc_id = lparam.0 as u32;
            // `WPARAM` is unsigned and pointer-sized: a 32-bit `-666` query
            // arrives as `0xFFFFFD66`, which a plain `as i64` would
            // zero-extend to a large positive on a 32-bit host and miss the
            // signed `-666` sentinels `answer` compares against (the C
            // comparison runs in the unsigned domain, so it matches there).
            // Reinterpret through `isize` before widening, so a query sent
            // from either width reads back signed on either host.
            let wparam = wparam.0 as isize as i64;
            let reply = unsafe {
                let surface = &mut *surface_ptr;
                surface.answer(ipc_id, wparam).raw() as isize
            };
            LRESULT(reply)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

// Alias kept for convenience; callers can reach the ABI constants via
// `rustamp_plugin_abi::common` (re-exported from this crate as `common`).
