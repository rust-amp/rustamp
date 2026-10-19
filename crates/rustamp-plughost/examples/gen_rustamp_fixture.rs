//! A clean-room **general-purpose** (`gen_`) plugin fixture for the P3 DLL
//! acceptance (`docs/tasks/P3-dll-acceptance.md`).
//!
//! # What this is
//!
//! A separate `cdylib` artifact — never a library linked into the test or the
//! host — that implements the general-purpose plugin ABI of
//! `specs/S09-plugin-abi.md` §7 and nothing else:
//!
//! * one undecorated cdecl export, `winampGetGeneralPurposePlugin`, that takes
//!   no arguments and returns the plugin's static module (S09 §2, §7.1);
//! * a static module with version `0x10` and the three callback slots
//!   `init`/`config`/`quit` (S09 §7.1);
//! * `init` records the parent window and the DLL instance the host filled in,
//!   **subclasses the host's window procedure** and creates a window of its
//!   own (S09 §7.2, §8.6);
//! * `config` opens a configuration window **owned by** the parent the host
//!   handed over and keeps pumping messages until the guest closes it
//!   (S09 §7.2, §8.7);
//! * `quit` destroys the windows and **restores the host's original window
//!   procedure** (S09 §7.2).
//!
//! # What this is not
//!
//! * It is not a mock: the export is resolved by name through the OS loader
//!   and called through the real cdecl ABI, and every callback is a real
//!   function in a real DLL (P3 §7 lists "mocked/statically linked callbacks"
//!   as a false pass).
//! * It is not a third-party plugin and proves nothing about one. It is the
//!   instrumented fixture P3 §2 asks for: our own DLL, built only from S09.
//! * It does not invent an ABI. The private challenge protocol it uses to
//!   talk to its acceptance test is a *test* transport
//!   (`tests/common/gen_protocol.rs`), not a Winamp IPC channel.
//!
//! # Evidence
//!
//! Every step the fixture takes is appended to its own trace file in the
//! process temp directory, written **from inside the plugin**. The trace is
//! plugin-side evidence: the host and the test runner cannot forge it.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004), like the rest of this package.

#![cfg(windows)]

#[path = "../tests/common/gen_protocol.rs"]
mod gen_protocol;

use gen_protocol::*;

use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use rustamp_plugin_abi::common::{GEN_VERSION, IPC_GETVERSION, WM_WA_IPC};
use rustamp_plugin_abi::general::GeneralPlugin;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GWLP_WNDPROC, GetClassNameW, GetMessageW, IsWindow, MSG, PostQuitMessage, RegisterClassW,
    RegisterWindowMessageW, SendMessageW, SetTimer, SetWindowLongPtrW, TranslateMessage,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_TIMER, WNDCLASSW, WS_CAPTION, WS_POPUP, WS_SYSMENU,
};
use windows::core::PCWSTR;

/// The window class the ABI fixes for the host's main window (S09 §1, §8).
const HOST_WINDOW_CLASS: &str = "Winamp v1.x";

/// The id of the configuration window's safety timer.
const CONFIG_TIMER_ID: usize = 1;

/// How long the configuration window may stay open before the fixture closes
/// it itself.
///
/// A real modal dialog blocks until the user closes it. This one is bounded,
/// so a guest that never closes it cannot wedge the whole test run: the
/// fixture closes the window, says so in its trace, and the acceptance test
/// still fails because the guest's own report says "not closed".
const CONFIG_TIMEOUT_MS: u32 = 60_000;

/// The plugin's static module (S09 §7.1).
///
/// It lives in this DLL's static data, so its address is stable for as long
/// as the DLL is loaded, and the host writes the two host-owned members into
/// it before it calls `init` (S09 §0, §7.1).
static mut MODULE: GeneralPlugin = GeneralPlugin {
    version: GEN_VERSION,
    description: c"Rustamp gen fixture".as_ptr(),
    init: Some(fixture_init),
    config: Some(fixture_config),
    quit: Some(fixture_quit),
    parent_window: core::ptr::null_mut(),
    dll_instance: core::ptr::null_mut(),
};

/// The parent window the host filled in before `init`.
static PARENT_HWND: AtomicUsize = AtomicUsize::new(0);

/// The fixture's own window, created in `init`.
static OWN_HWND: AtomicUsize = AtomicUsize::new(0);

/// The configuration window, while `config` is running.
static CONFIG_HWND: AtomicUsize = AtomicUsize::new(0);

/// The host's original window procedure, saved when the subclass was
/// installed.
static ORIGINAL_PROC: AtomicUsize = AtomicUsize::new(0);

/// 0 = never installed, 1 = installed, 2 = restored in `quit`.
static SUBCLASS_STATE: AtomicUsize = AtomicUsize::new(0);

/// The raw `int` `init` returned.
static INIT_RESULT: AtomicUsize = AtomicUsize::new(0);

/// How many messages the subclass forwarded to the host's original procedure.
static SUBCLASS_ENTRIES: AtomicUsize = AtomicUsize::new(0);

/// 1 once `quit` ran.
static QUIT_CALLED: AtomicUsize = AtomicUsize::new(0);

/// 1 when the parent window's class was `Winamp v1.x`.
static PARENT_CLASS_OK: AtomicUsize = AtomicUsize::new(0);

/// The OS-registered id of the private challenge message.
static CHALLENGE_MESSAGE: AtomicU32 = AtomicU32::new(0);

/// The general-purpose entry point (S09 §2).
///
/// cdecl, no arguments, returns the plugin's static module. The name is
/// undecorated, as the ABI requires (S09 §1).
#[unsafe(no_mangle)]
pub extern "C" fn winampGetGeneralPurposePlugin() -> *mut GeneralPlugin {
    // `MODULE` is this DLL's static data. Its address is stable for the life
    // of the DLL, which is exactly the lifetime S09 §0 gives it. The raw
    // pointer keeps the static itself unborrowed.
    core::ptr::addr_of_mut!(MODULE)
}

/// `init` (S09 §7.1 offset 0x08): called on the UI thread after the host
/// filled in the parent window and the DLL instance.
unsafe extern "C" fn fixture_init() -> c_int {
    let module = core::ptr::addr_of_mut!(MODULE);
    // SAFETY: the host filled both members in before calling us (S09 §7.1
    // offsets 0x14 and 0x18); reading them through the raw pointer keeps the
    // static itself unborrowed.
    let parent = unsafe { (*module).parent_window } as usize;
    let instance = unsafe { (*module).dll_instance } as usize;
    PARENT_HWND.store(parent, Ordering::SeqCst);
    trace(&format!(
        "init parent={parent:#x} instance={instance:#x} version={}",
        GEN_VERSION
    ));

    // The parent must be the host's real main window: a fixture that cannot
    // see the `Winamp v1.x` class is not testing the real thing (S09 §1).
    let class_ok = u32::from(window_class_name(parent) == HOST_WINDOW_CLASS);
    PARENT_CLASS_OK.store(class_ok as usize, Ordering::SeqCst);
    trace(&format!("parent class ok={class_ok}"));

    // Subclass the host's window procedure and keep the original one, so the
    // host's messages can be observed and forwarded unchanged (S09 §8.6).
    if parent != 0 {
        let previous = replace_window_proc(parent, subclass_proc as *const () as usize);
        ORIGINAL_PROC.store(previous, Ordering::SeqCst);
        SUBCLASS_STATE.store(1, Ordering::SeqCst);
        trace(&format!("subclass installed original={previous:#x}"));
    }

    // A general purpose plugin typically creates windows of its own (S09 §7.2).
    let own = create_window(
        FIXTURE_WINDOW_CLASS,
        FIXTURE_WINDOW_CLASS,
        instance,
        None,
        WINDOW_STYLE(WS_POPUP.0),
    );
    OWN_HWND.store(own, Ordering::SeqCst);
    trace(&format!(
        "window created hwnd={own:#x} class={FIXTURE_WINDOW_CLASS}"
    ));

    let result = 0;
    INIT_RESULT.store(result as u32 as usize, Ordering::SeqCst);
    result
}

/// `config` (S09 §7.1 offset 0x0C): open a configuration window owned by the
/// parent, and keep the UI responsive until the guest closes it.
unsafe extern "C" fn fixture_config() {
    let module = core::ptr::addr_of_mut!(MODULE);
    // SAFETY: the host filled both members in before `init` ran.
    let parent = unsafe { (*module).parent_window } as usize;
    let instance = unsafe { (*module).dll_instance } as usize;

    let config = create_window(
        FIXTURE_CONFIG_CLASS,
        "Rustamp gen fixture config",
        instance,
        Some(parent),
        WINDOW_STYLE((WS_POPUP | WS_CAPTION | WS_SYSMENU).0),
    );
    CONFIG_HWND.store(config, Ordering::SeqCst);
    trace(&format!("config opened hwnd={config:#x} owner={parent:#x}"));
    if config == 0 {
        trace("config failed to open");
        return;
    }
    // Bound the lifetime of the configuration window (see CONFIG_TIMEOUT_MS).
    // SAFETY: the window is this plugin's own.
    unsafe {
        SetTimer(
            Some(HWND(config as *mut c_void)),
            CONFIG_TIMER_ID,
            CONFIG_TIMEOUT_MS,
            None,
        )
    };

    // A nested message loop: while the configuration window is open, the
    // host's window keeps answering messages (P3 §4 step 9, "responsive
    // message pumping"). It ends when the guest closes the window.
    loop {
        let mut msg = MSG::default();
        // SAFETY: `msg` is writable and owned by this frame.
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if got.0 <= 0 {
            if got.0 == 0 {
                // WM_QUIT: pass it on instead of swallowing it.
                unsafe { PostQuitMessage(0) };
            }
            break;
        }
        // SAFETY: the message was filled in by GetMessageW.
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        // SAFETY: the handle is the window this function created.
        if !unsafe { IsWindow(Some(HWND(config as *mut c_void))) }.as_bool() {
            break;
        }
    }

    CONFIG_HWND.store(0, Ordering::SeqCst);
    trace("config closed");
}

/// `quit` (S09 §7.1 offset 0x10): tear the plugin down and give the host its
/// original window procedure back.
unsafe extern "C" fn fixture_quit() {
    let config = CONFIG_HWND.swap(0, Ordering::SeqCst);
    if config != 0 {
        // SAFETY: the handle is the window this plugin created.
        let _ = unsafe { DestroyWindow(HWND(config as *mut c_void)) };
    }

    let parent = PARENT_HWND.load(Ordering::SeqCst);
    let original = ORIGINAL_PROC.swap(0, Ordering::SeqCst);
    let mut restored = false;
    if parent != 0 && original != 0 {
        replace_window_proc(parent, original);
        SUBCLASS_STATE.store(2, Ordering::SeqCst);
        restored = true;
    }

    let own = OWN_HWND.swap(0, Ordering::SeqCst);
    let mut destroyed = false;
    if own != 0 {
        // SAFETY: the handle is the window this plugin created.
        destroyed = unsafe { DestroyWindow(HWND(own as *mut c_void)) }.is_ok();
    }

    QUIT_CALLED.store(1, Ordering::SeqCst);
    trace(&format!(
        "quit subclass_restored={restored} window_destroyed={destroyed}"
    ));
}

/// The fixture's own window procedure: it answers the private challenge
/// message and forwards everything else to the default procedure.
unsafe extern "system" fn fixture_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == challenge_message() {
        return handle_challenge(wparam.0, lparam.0 as usize);
    }
    // SAFETY: the arguments are the ones Win32 delivered.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// The configuration window's procedure: the guest closes it with `WM_CLOSE`.
unsafe extern "system" fn config_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_TIMER && wparam.0 == CONFIG_TIMER_ID {
        trace("config timed out");
        // SAFETY: the window is this plugin's own.
        let _ = unsafe { DestroyWindow(hwnd) };
        return LRESULT(0);
    }
    if msg == WM_CLOSE {
        trace("config close requested");
        // SAFETY: the window is this plugin's own.
        let _ = unsafe { DestroyWindow(hwnd) };
        return LRESULT(0);
    }
    // SAFETY: the arguments are the ones Win32 delivered.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// The subclass of the host's window procedure.
///
/// Every message is forwarded to the procedure this one replaced and its
/// result is returned unchanged (P3 §4 step 6).
unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let original = ORIGINAL_PROC.load(Ordering::SeqCst);
    if original == 0 {
        // SAFETY: the arguments are the ones Win32 delivered.
        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: `original` is the address of the window procedure this plugin
    // replaced on this very window.
    let original_fn: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT =
        unsafe { core::mem::transmute(original) };
    // SAFETY: the saved procedure belongs to this window.
    let result = unsafe { CallWindowProcW(Some(original_fn), hwnd, msg, wparam, lparam) };
    SUBCLASS_ENTRIES.fetch_add(1, Ordering::SeqCst);
    result
}

/// Answer one private challenge (see `tests/common/gen_protocol.rs`).
fn handle_challenge(cmd: usize, arg: usize) -> LRESULT {
    match cmd {
        CMD_ECHO => {
            trace(&format!("challenge echo arg={arg:#x}"));
            LRESULT(arg as isize)
        }
        CMD_PARENT_HWND => LRESULT(PARENT_HWND.load(Ordering::SeqCst) as isize),
        CMD_OWN_HWND => LRESULT(OWN_HWND.load(Ordering::SeqCst) as isize),
        CMD_CONFIG_HWND => LRESULT(CONFIG_HWND.load(Ordering::SeqCst) as isize),
        CMD_IPC_VERSION_REPLY => LRESULT(query_parent(IPC_GETVERSION) as isize),
        CMD_IPC_UNKNOWN_REPLY => LRESULT(query_parent(UNKNOWN_IPC_ID) as isize),
        CMD_INIT_RESULT => LRESULT(INIT_RESULT.load(Ordering::SeqCst) as isize),
        CMD_SUBCLASS_STATE => LRESULT(SUBCLASS_STATE.load(Ordering::SeqCst) as isize),
        CMD_SUBCLASS_ENTRIES => LRESULT(SUBCLASS_ENTRIES.load(Ordering::SeqCst) as isize),
        CMD_QUIT_CALLED => LRESULT(QUIT_CALLED.load(Ordering::SeqCst) as isize),
        CMD_PARENT_CLASS_OK => LRESULT(PARENT_CLASS_OK.load(Ordering::SeqCst) as isize),
        _ => {
            trace(&format!("challenge unknown cmd={cmd}"));
            LRESULT(0)
        }
    }
}

/// Send one classic IPC message to the parent window **from this DLL's code**
/// and return the raw 32-bit reply.
///
/// The message goes to the parent's *current* window procedure, which is this
/// plugin's subclass: the subclass forwards it to the host's original
/// procedure and returns the result unchanged (S09 §8.2: the IPC number is in
/// LPARAM, the argument in WPARAM).
fn query_parent(ipc_id: u32) -> u32 {
    let parent = PARENT_HWND.load(Ordering::SeqCst);
    if parent == 0 {
        return 0;
    }
    // SAFETY: the parent is the host's live main window; `SendMessageW`
    // marshals the call to its owning thread and waits for the reply.
    let reply = unsafe {
        SendMessageW(
            HWND(parent as *mut c_void),
            WM_WA_IPC,
            Some(WPARAM(0)),
            Some(LPARAM(ipc_id as isize)),
        )
    };
    let raw = reply.0 as u32;
    trace(&format!("ipc id={ipc_id} reply={raw:#x}"));
    raw
}

/// The OS-registered id of the private challenge message.
fn challenge_message() -> u32 {
    let cached = CHALLENGE_MESSAGE.load(Ordering::Relaxed);
    if cached != 0 {
        return cached;
    }
    let name = wide(CHALLENGE_MESSAGE_NAME);
    // SAFETY: the name is NUL-terminated and outlives the call.
    let id = unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) };
    CHALLENGE_MESSAGE.store(id, Ordering::SeqCst);
    id
}

/// Register `class` (once) and create a window of it.
///
/// `owner` becomes the window's owner, which is how a configuration window is
/// tied to the host's main window (S09 §8.7).
fn create_window(
    class: &str,
    title: &str,
    instance: usize,
    owner: Option<usize>,
    style: WINDOW_STYLE,
) -> usize {
    let class_wide = wide(class);
    let title_wide = wide(title);
    let procedure: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT =
        if class == FIXTURE_CONFIG_CLASS {
            config_wnd_proc
        } else {
            fixture_wnd_proc
        };
    let wc = WNDCLASSW {
        lpfnWndProc: Some(procedure),
        hInstance: HINSTANCE(instance as *mut c_void),
        lpszClassName: PCWSTR(class_wide.as_ptr()),
        ..Default::default()
    };
    // Registering an already registered class fails harmlessly: the class
    // stays usable.
    // SAFETY: the class name is NUL-terminated and outlives the call.
    unsafe { RegisterClassW(&wc) };

    // SAFETY: both strings are NUL-terminated and outlive the call; the
    // owner handle is the host's main window or none.
    let created = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_wide.as_ptr()),
            PCWSTR(title_wide.as_ptr()),
            style,
            0,
            0,
            320,
            120,
            owner.map(|value| HWND(value as *mut c_void)),
            None,
            Some(HINSTANCE(instance as *mut c_void)),
            None,
        )
    };
    match created {
        Ok(hwnd) => hwnd.0 as usize,
        Err(_) => 0,
    }
}

/// Install a window procedure on a window and return the previous one.
///
/// `SetWindowLongPtrW` is `SetWindowLongW` on a 32-bit target, so the value
/// has to be narrowed there.
fn replace_window_proc(hwnd: usize, address: usize) -> usize {
    let handle = HWND(hwnd as *mut c_void);
    #[cfg(target_pointer_width = "64")]
    let previous = unsafe { SetWindowLongPtrW(handle, GWLP_WNDPROC, address as isize) };
    #[cfg(target_pointer_width = "32")]
    let previous = unsafe { SetWindowLongPtrW(handle, GWLP_WNDPROC, address as i32) };
    previous as usize
}

/// The class name of a window, or an empty string.
fn window_class_name(hwnd: usize) -> String {
    if hwnd == 0 {
        return String::new();
    }
    let mut buffer = [0_u16; 64];
    // SAFETY: the buffer is writable for its length and the handle is live.
    let length = unsafe { GetClassNameW(HWND(hwnd as *mut c_void), &mut buffer) };
    if length <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..length as usize])
}

/// A NUL-terminated UTF-16 string.
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The fixture's own trace file.
fn trace_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "{TRACE_FILE_PREFIX}{}.{TRACE_FILE_EXTENSION}",
        std::process::id()
    ))
}

/// Append one line to the fixture's own trace.
///
/// The trace is written from inside the plugin, so it is plugin-side evidence
/// that neither the host nor the test runner can forge (P3 §5.5, §7).
fn trace(event: &str) {
    use std::io::Write;
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(trace_path())
    else {
        return;
    };
    let _ = writeln!(file, "{event}");
    let _ = file.flush();
}
