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
//! * **Message pumping can re-enter the window procedure**, including during
//!   creation and destruction. Checked interior borrows access stable, owned
//!   state. No surface borrow spans a Win32 call or caller-provided callback.
//!   A nested request while the surface is borrowed returns unsupported (`1`).
//! * **UI-thread ownership is enforced:** `HostWindow` is neither `Send` nor
//!   `Sync`. Public methods expose owned snapshots and replies, never surface
//!   references. Win32 marshals messages onto the window's owning thread.
//! * **Lifetime:** the window (and therefore the handle plugins hold) must
//!   outlive every loaded plugin.  It is created before any DLL is loaded and
//!   destroyed only after every `quit` has run.

#[cfg(any(windows, test))]
use std::cell::Cell;
use std::cell::RefCell;
use std::rc::Rc;

use crate::ipc::IpcSurface;
#[cfg(any(windows, test))]
use crate::ipc::UNKNOWN_REPLY;
use rustamp_proto::state::PlayerState;

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

/// Computer name of the disposable headless test VM, as last reported by the
/// test runner.
///
/// This mirrors the canonical rule in `rustamp-app/src/window.rs` (same
/// constant, same marker, same predicate). It is duplicated rather than
/// shared because this crate is MIT OR Apache-2.0 by design (RFC 0004) and
/// cannot depend on the GPL application crate: the rule is implemented once
/// per license domain with identical semantics. Keep the two in step.
pub const TEST_VM_COMPUTER: &str = "DESKTOP-UL3J9H6";

/// Path the test runner stages the repository tree to inside the guest
/// (`docs/notes/vm-run-tests.ps1` copies the staged tree to
/// `C:\rustamp\code\Rustamp`). The host never has this path, so its presence
/// proves the process runs inside the disposable headless test VM. This is
/// the primary signal in [`is_test_vm`]: unlike a computer name it survives
/// a VM rebuild.
pub const TEST_VM_MARKER: &str = r"C:\rustamp\code\Rustamp";

/// True when this process provably runs inside the disposable headless test
/// VM, the only host for anything graphical.
///
/// Either signal counts: the runner's staged tree ([`TEST_VM_MARKER`]) or the
/// VM's computer name ([`TEST_VM_COMPUTER`]). Neither is a wildcard — a plain
/// developer machine has neither — and there is deliberately **no**
/// environment-variable override.
pub fn is_test_vm() -> bool {
    if std::path::Path::new(TEST_VM_MARKER).exists() {
        return true;
    }
    std::env::var("COMPUTERNAME").is_ok_and(|name| name.eq_ignore_ascii_case(TEST_VM_COMPUTER))
}

/// Refuses unless this process provably runs inside the disposable headless
/// test VM.
///
/// The single gate window-creating tests in this crate share: call this
/// before any window exists. There is deliberately **no** override. GUI must
/// never run on the host.
pub fn require_test_vm() {
    assert!(
        is_test_vm(),
        "refusing to create a window: this test is VM-only, but this machine is neither the \
         headless test VM ({TEST_VM_COMPUTER}) nor carries the runner's staged tree \
         ({TEST_VM_MARKER}). GUI must never run on the host; run it inside the headless test \
         VM."
    );
}

/// Failure creating a host window or accessing its UI-thread state.
#[derive(Debug)]
pub enum HostWindowError {
    /// Window-system failure.
    Message(String),
    /// The surface is already in use by a nested operation.
    SurfaceBusy,
}

impl std::fmt::Display for HostWindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(m) => f.write_str(m),
            Self::SurfaceBusy => f.write_str("host IPC surface is already borrowed"),
        }
    }
}

impl std::error::Error for HostWindowError {}

// Rc gives stable storage and explicitly makes every owner !Send + !Sync.
struct WindowState {
    surface: RefCell<IpcSurface>,
    #[cfg(any(windows, test))]
    window_alive: Cell<bool>,
    ipc_queries: RefCell<Vec<(u32, i64, u32)>>,
    window_messages: RefCell<Vec<(u32, usize, isize)>>,
}

impl WindowState {
    #[cfg(any(windows, test))]
    fn new(surface: IpcSurface) -> Self {
        Self {
            surface: RefCell::new(surface),
            window_alive: Cell::new(false),
            ipc_queries: RefCell::new(Vec::new()),
            window_messages: RefCell::new(Vec::new()),
        }
    }

    fn answer(&self, wparam: i64, ipc_id: u32) -> Result<u32, HostWindowError> {
        let mut surface = self
            .surface
            .try_borrow_mut()
            .map_err(|_| HostWindowError::SurfaceBusy)?;
        Ok(HostWindow::dispatch_ipc(&mut surface, wparam, ipc_id))
    }

    #[cfg(any(windows, test))]
    fn message_reply(&self, wparam: i64, ipc_id: u32) -> u32 {
        self.answer(wparam, ipc_id).unwrap_or(UNKNOWN_REPLY)
    }
}

// SAFETY: ptr must come from a live UI-thread Rc<WindowState>, with exactly
// one corresponding release after the pointer is unpublished.
#[cfg(any(windows, test))]
unsafe fn retain_window_state(ptr: *const WindowState) {
    // SAFETY: the caller guarantees a live Rc allocation on this thread.
    unsafe { Rc::increment_strong_count(ptr) };
    // SAFETY: the retained strong reference keeps the allocation alive.
    unsafe { (*ptr).window_alive.set(true) };
}

// SAFETY: ptr must hold the one retained window reference, already unpublished
// from GWLP_USERDATA; release must run on its UI thread exactly once.
#[cfg(any(windows, test))]
unsafe fn release_window_state(ptr: *const WindowState) {
    // SAFETY: the caller transfers the retained reference back into an Rc.
    let state = unsafe { Rc::from_raw(ptr) };
    state.window_alive.set(false);
}

/// The host's UI-thread window and its owned, stable IPC state.
///
/// Creation consumes the surface. An OS-owned reference lasts from
/// `WM_NCCREATE` through `WM_NCDESTROY`, even if `DestroyWindow` fails.
/// Plugins must finish using the window before its owner is destroyed.
///
/// The former borrowed construction shape is rejected at compile time:
/// ```compile_fail
/// use rustamp_plughost::{HostWindow, IpcSurface};
/// let mut surface = IpcSurface::new();
/// let _window = HostWindow::create(&mut surface);
/// ```
/// Ownership cannot cross threads:
/// ```compile_fail
/// fn assert_send<T: Send>() {}
/// assert_send::<rustamp_plughost::HostWindow>();
/// ```
/// Shared access cannot cross threads either:
/// ```compile_fail
/// fn assert_sync<T: Sync>() {}
/// assert_sync::<rustamp_plughost::HostWindow>();
/// ```
pub struct HostWindow {
    #[cfg(windows)]
    hwnd: windows::Win32::Foundation::HWND,
    #[cfg(not(windows))]
    hwnd: usize,
    state: Rc<WindowState>,
}

impl HostWindow {
    /// Class name the ABI fixes (S09 sections 1, 8).
    pub const CLASS_NAME: &'static str = HOST_WINDOW_CLASS;

    /// Return the raw window handle without exposing the owned surface.
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

    /// Dispatch a query against a standalone surface, without OS calls.
    /// The window procedure and headless surface tests share this entry point.
    /// Calling it directly exercises neither a window procedure nor a DLL.
    pub fn dispatch_ipc(surface: &mut IpcSurface, wparam: i64, ipc_id: u32) -> u32 {
        surface.answer(ipc_id, wparam).raw()
    }

    /// Copy the UI-thread surface without exposing any interior borrow.
    pub fn snapshot(&self) -> Result<IpcSurface, HostWindowError> {
        self.state
            .surface
            .try_borrow()
            .map(|surface| surface.clone())
            .map_err(|_| HostWindowError::SurfaceBusy)
    }

    /// Apply a player snapshot on the UI thread, without pumping messages.
    pub fn apply_player_state(&self, state: &PlayerState) -> Result<(), HostWindowError> {
        let mut surface = self
            .state
            .surface
            .try_borrow_mut()
            .map_err(|_| HostWindowError::SurfaceBusy)?;
        crate::state_bridge::apply_player_state(&mut surface, state);
        Ok(())
    }

    /// Answer one UI-thread query without exposing surface storage.
    pub fn answer(&self, wparam: i64, ipc_id: u32) -> Result<u32, HostWindowError> {
        self.state.answer(wparam, ipc_id)
    }

    /// Return a copy of all recorded IPC queries handled by this window.
    pub fn recorded_ipc_queries(&self) -> Vec<(u32, i64, u32)> {
        self.state.ipc_queries.borrow().clone()
    }

    /// Return a copy of all recorded non-IPC window messages handled by this window.
    pub fn recorded_messages(&self) -> Vec<(u32, usize, isize)> {
        self.state.window_messages.borrow().clone()
    }

    /// Register the window class once. Idempotent across calls.
    #[cfg(windows)]
    pub fn register_class() -> Result<(), HostWindowError> {
        use std::sync::OnceLock;
        static DONE: OnceLock<Result<(), String>> = OnceLock::new();
        let r = DONE.get_or_init(register_class_inner);
        r.clone().map_err(HostWindowError::Message)
    }

    /// Create a hidden UI-thread window, taking ownership of its IPC surface.
    #[cfg(windows)]
    pub fn create(surface: IpcSurface) -> Result<Self, HostWindowError> {
        Self::register_class()?;
        create_window_inner(surface)
    }

    /// Window class registration is unavailable on other platforms.
    #[cfg(not(windows))]
    pub fn register_class() -> Result<(), HostWindowError> {
        Err(HostWindowError::Message("windows-only".into()))
    }

    /// Host windows are unavailable on other platforms.
    #[cfg(not(windows))]
    pub fn create(surface: IpcSurface) -> Result<Self, HostWindowError> {
        let _ = surface;
        Err(HostWindowError::Message("windows-only".into()))
    }

    /// Destroy the OS window on its owning UI thread.
    pub fn destroy(self) {
        drop(self)
    }

    /// Create an inert HostWindow for testing without creating an OS window.
    #[cfg(test)]
    pub fn dummy() -> Self {
        Self {
            hwnd: Default::default(),
            state: Rc::new(WindowState::new(IpcSurface::new())),
        }
    }
}

#[cfg(windows)]
impl Drop for HostWindow {
    fn drop(&mut self) {
        use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;
        if self.state.window_alive.get() {
            // SAFETY: Rc ownership prevents leaving the UI thread. No surface
            // borrow spans this reentrant call. On failure, the retained OS
            // reference keeps the pointer alive until WM_NCDESTROY; if that
            // never arrives, the allocation is deliberately leaked.
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

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
    // SAFETY: the NUL-terminated class string lives through the call.
    let atom = unsafe { RegisterClassW(&wc) };
    if atom == 0 {
        Err("RegisterClassW failed".to_string())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn create_window_inner(surface: IpcSurface) -> Result<HostWindow, HostWindowError> {
    use windows::Win32::UI::WindowsAndMessaging::CreateWindowExW;
    use windows::core::PCWSTR;
    let title: Vec<u16> = "Rustamp plughost"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let class_wide = host_window_class_wide();
    let state = Rc::new(WindowState::new(surface));
    // SAFETY: the registered class receives this stable Rc allocation. This
    // owner lives through creation. WM_NCCREATE retains another reference
    // before publishing it; WM_NCDESTROY unpublishes and releases it. No
    // surface borrow spans this potentially reentrant call.
    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(),
            PCWSTR(class_wide.as_ptr()),
            PCWSTR(title.as_ptr()),
            Default::default(), // Hidden host window, not WS_VISIBLE.
            0,
            0,
            0,
            0,
            None,
            None,
            Some(windows::Win32::Foundation::HINSTANCE(std::ptr::null_mut())),
            Some(Rc::as_ptr(&state) as *const std::ffi::c_void),
        )
    }
    .map_err(|e| HostWindowError::Message(e.to_string()))?;
    // Creation returned: no callback or surface borrow is running here.
    state.surface.borrow_mut().main_hwnd = hwnd.0 as usize as u32;
    Ok(HostWindow { hwnd, state })
}

// SAFETY: Win32 invokes this with the registered window procedure ABI.
#[cfg(windows)]
unsafe extern "system" fn wnd_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    // Rust panics must not unwind through the system callback boundary.
    std::panic::catch_unwind(|| {
        // SAFETY: Win32 supplies the original message arguments.
        unsafe { handle_window_message(hwnd, msg, wparam, lparam) }
    })
    .unwrap_or(windows::Win32::Foundation::LRESULT(UNKNOWN_REPLY as isize))
}

// SAFETY: arguments must be a Win32 message delivered to this class.
#[cfg(windows)]
unsafe fn handle_window_message(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    use windows::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, GWLP_USERDATA, GetWindowLongPtrW, PostQuitMessage, SetWindowLongPtrW,
        WM_DESTROY, WM_NCCREATE, WM_NCDESTROY, WM_USER,
    };
    // Messages that are not the system's own (WM_USER and up, registered
    // messages, WM_COPYDATA) are what a plug-in sends the host: with
    // RUSTAMP_HOST_TRACE set they are written down, so a plug-in that wants
    // something the host does not answer shows what it asked for.
    if msg >= WM_USER || msg == windows::Win32::UI::WindowsAndMessaging::WM_COPYDATA {
        crate::fault::trace_event(&format!(
            "window message {msg:#x} wparam={:#x} lparam={:#x}",
            wparam.0, lparam.0
        ));
    }
    match msg {
        WM_NCCREATE => {
            let cs = lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
            if cs.is_null() {
                return LRESULT(0);
            }
            // SAFETY: Win32 provides CREATESTRUCTW for this creation message.
            let state_ptr = unsafe { (*cs).lpCreateParams } as *const WindowState;
            if state_ptr.is_null() {
                return LRESULT(0);
            }
            // SAFETY: create_window_inner owns the Rc throughout creation;
            // WM_NCDESTROY releases this reference after clearing the slot.
            unsafe { retain_window_state(state_ptr) };
            #[cfg(target_pointer_width = "64")]
            let value = state_ptr as isize;
            #[cfg(target_pointer_width = "32")]
            let value = state_ptr as i32;
            // SAFETY: this window owns the slot and the retained Rc backs it.
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, value);
            }
            // SAFETY: forward original arguments without a surface borrow.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_USER => {
            // SAFETY: read our UI-thread window's private state slot.
            let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WindowState;
            if state_ptr.is_null() {
                return LRESULT(UNKNOWN_REPLY as isize);
            }
            // Classic IPC carries its id in LPARAM. Reinterpret WPARAM through
            // isize so the -666 sentinel remains signed on either pointer width.
            let ipc_id = lparam.0 as u32;
            // S21: a plug-in's own code, running on this thread during its load or
            // init, asks for the 5.x start-up answers (the service manager, the
            // settings paths, the random generators). A request from another
            // thread or process is not that and keeps the 2.95 answers of S10.
            // SAFETY: no pointer arguments; reads the state of this window's
            // current message.
            let from_this_thread =
                unsafe { windows::Win32::UI::WindowsAndMessaging::InSendMessageEx(None) } == 0;
            if from_this_thread
                && let Some(answer) = crate::host_services::ipc_answer(ipc_id, wparam.0)
            {
                crate::fault::trace_event(&format!("5.x answer for IPC {ipc_id}: {answer:#x}"));
                return LRESULT(answer as isize);
            }
            let wparam = wparam.0 as isize as i64;
            // SAFETY: the window retains this allocation until WM_NCDESTROY.
            // Shared access uses RefCell's checked mutable borrow. message_reply
            // calls no OS/user code, so destruction cannot reenter this access.
            let reply = unsafe { (*state_ptr).message_reply(wparam, ipc_id) };
            if let Ok(mut queries) = unsafe { (*state_ptr).ipc_queries.try_borrow_mut() } {
                queries.push((ipc_id, wparam, reply));
            }
            LRESULT(reply as isize)
        }
        WM_NCDESTROY => {
            // SAFETY: read our UI-thread window's retained reference.
            let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WindowState;
            // SAFETY: clear the live window's slot before any reentrant OS call.
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            if !state_ptr.is_null() {
                // SAFETY: WM_NCCREATE retained this reference; the cleared slot
                // prevents nested messages from accessing or releasing it twice.
                unsafe { release_window_state(state_ptr) };
            }
            // SAFETY: forward original arguments without a surface borrow.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_DESTROY => {
            // SAFETY: request termination of this UI thread's message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        // SAFETY: forward original arguments without a surface borrow.
        msg => {
            // S09 §3.6: an input plugin's decode thread posts the end-of-track
            // notification `0x402` to the main window with both parameters
            // zero. It is not an IPC query: the host records it and reports it
            // to its caller.
            crate::handoff::on_window_message(msg);
            let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const WindowState;
            if let Some(state) = unsafe { state_ptr.as_ref() } {
                let _ = state
                    .window_messages
                    .try_borrow_mut()
                    .map(|mut msgs| msgs.push((msg, wparam.0, lparam.0)));
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_consumes_the_surface() {
        // Compile-only signature evidence: no window is created.
        let _: fn(IpcSurface) -> Result<HostWindow, HostWindowError> = HostWindow::create;
    }

    #[test]
    fn ownership_is_neither_send_nor_sync() {
        // If either bound becomes true, two trait implementations match and
        // inference fails at compile time. This requires no runtime/thread work.
        trait AmbiguousIfSend<A> {
            fn check() {}
        }
        impl<T: ?Sized> AmbiguousIfSend<()> for T {}
        impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}
        trait AmbiguousIfSync<A> {
            fn check() {}
        }
        impl<T: ?Sized> AmbiguousIfSync<()> for T {}
        impl<T: ?Sized + Sync> AmbiguousIfSync<u8> for T {}
        let _ = <HostWindow as AmbiguousIfSend<_>>::check;
        let _ = <HostWindow as AmbiguousIfSync<_>>::check;
    }

    #[test]
    fn nested_surface_access_returns_unsupported_without_panicking() {
        let state = WindowState::new(IpcSurface::new());
        let borrow = state.surface.borrow_mut();
        assert_eq!(state.message_reply(0, 0), UNKNOWN_REPLY);
        assert!(matches!(
            state.answer(0, 0),
            Err(HostWindowError::SurfaceBusy)
        ));
        drop(borrow);
        assert_eq!(state.message_reply(0, 0), 0x2905);
        assert_eq!(state.message_reply(64, 122), 64);
        assert_eq!(state.message_reply(-666, 122), 64);
    }

    #[test]
    fn public_access_returns_owned_snapshots_and_applies_player_state() {
        // An inert owner: no window is created, and window_alive is false so
        // Drop does not call DestroyWindow. All operations stay pure Rust.
        let window = HostWindow {
            hwnd: Default::default(),
            state: Rc::new(WindowState::new(IpcSurface::new())),
        };
        let state = PlayerState {
            volume: 73,
            balance: -4,
            ..PlayerState::default()
        };
        window.apply_player_state(&state).unwrap();
        let mut snapshot = window.snapshot().unwrap();
        assert_eq!(snapshot.volume, 73);
        snapshot.volume = 12;
        assert_eq!(snapshot.volume, 12);
        assert_eq!(window.answer(-666, 122).unwrap(), 73);
        assert_eq!(window.answer(-666, 123).unwrap(), (-4_i32) as u32);
        let borrow = window.state.surface.borrow_mut();
        assert!(matches!(
            window.snapshot(),
            Err(HostWindowError::SurfaceBusy)
        ));
        assert!(matches!(
            window.apply_player_state(&state),
            Err(HostWindowError::SurfaceBusy)
        ));
        drop(borrow);
        assert_eq!(window.snapshot().unwrap().volume, 73);
    }

    #[test]
    fn window_lease_outlives_the_owner_until_unpublished() {
        let state = Rc::new(WindowState::new(IpcSurface::new()));
        let weak = Rc::downgrade(&state);
        let pointer = Rc::as_ptr(&state);
        // SAFETY: test owns the originating Rc and releases the lease once.
        unsafe { retain_window_state(pointer) };
        assert_eq!(Rc::strong_count(&state), 2);
        drop(state); // Models failed DestroyWindow: published lease remains.
        assert!(weak.upgrade().unwrap().window_alive.get());
        // SAFETY: models clearing GWLP_USERDATA before releasing its lease.
        unsafe { release_window_state(pointer) };
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn external_destruction_releases_only_the_window_lease() {
        let state = Rc::new(WindowState::new(IpcSurface::new()));
        let pointer = Rc::as_ptr(&state);
        // SAFETY: originating Rc stays live; lease is released once below.
        unsafe { retain_window_state(pointer) };
        // SAFETY: models WM_NCDESTROY clearing GWLP_USERDATA first.
        unsafe { release_window_state(pointer) };
        assert!(!state.window_alive.get());
        assert_eq!(Rc::strong_count(&state), 1);
        assert_eq!(state.message_reply(0, 0), 0x2905);
    }
}
