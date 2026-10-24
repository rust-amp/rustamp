//! VM-only acceptance test for the **general-purpose** plugin kind: the first
//! milestone of `docs/tasks/P3-dll-acceptance.md` (§2, §4 steps 2–10).
//!
//! It loads the clean-room `gen_` fixture
//! (`crates/rustamp-plughost/examples/gen_rustamp_fixture.rs`) through the
//! production loader and proves, in one real i686 process with real Win32
//! windows:
//!
//! 1. the approved bytes are loaded from a **separate DLL** through the OS
//!    loader, the export `winampGetGeneralPurposePlugin` resolves, and the
//!    module reports version `0x10` with the three callback slots;
//! 2. the host filled the parent window and the DLL instance before `init`
//!    ran, and the plugin saw the `Winamp v1.x` class (S09 §1, §7.1);
//! 3. the plugin **subclassed the host's window procedure** — the host's
//!    procedure is no longer installed, and the plugin's own window exists
//!    with its own class, process and thread (S09 §8.6);
//! 4. a challenge sent to the **plugin's own window** is answered by the
//!    plugin's own window procedure (P3 §4 step 8);
//! 5. an IPC query sent **from the plugin's own code** reaches the host's
//!    original window procedure and comes back as `0x2905`, and the host's
//!    own trace of that query agrees with what the plugin saw (P3 §4 step 6);
//! 6. an **unknown** IPC id comes back as `1` (S10 §1.5, P3 §4 step 7);
//! 7. `config` opens a window **owned by** the host's main window, in the
//!    same process and thread, which the guest closes explicitly; the host
//!    keeps answering messages while it is open (P3 §4 step 9);
//! 8. `quit` destroys the plugin's windows and **restores the host's
//!    original window procedure**; the host still answers afterwards, and the
//!    DLL stays mapped (S09 §1 line 46, P3 §4 step 10).
//!
//! The plugin's own trace file — written from inside the DLL — is checked for
//! the same events, so the evidence does not come from the runner alone.
//!
//! What this is **not**: a claim about third-party plugins, about the other
//! four plugin kinds, about audio, about visualization rendering or about
//! embedding. P3 §7 lists the surface test, `PluginLoaded` and a green build
//! as false passes; this test proves none of them.
//!
//! It refuses loudly (never skips) when it is not in the test VM, not on
//! i686, or when the fixture DLL is missing.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

#![cfg(windows)]

#[path = "common/gen_protocol.rs"]
mod gen_protocol;

#[path = "common/gen_fixture.rs"]
mod gen_fixture;

use gen_fixture::{FIXTURE_FILE_NAME, find_fixture_dll};
use gen_protocol::*;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rustamp_plughost::approval::ApprovalStore;
use rustamp_plughost::host_window::{HostWindow, require_test_vm};
use rustamp_plughost::ipc::IpcSurface;
use rustamp_plughost::loader::load_plugin_with_store;
use rustamp_plughost::message_loop;
use rustamp_plugin_abi::PluginKind;
use rustamp_plugin_abi::common::{IPC_GETVERSION, WINAMP_VERSION_295, WM_WA_IPC};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWLP_WNDPROC, GetClassNameW, GetWindow, GetWindowLongPtrW,
    GetWindowThreadProcessId, IsWindow, PostMessageW, RegisterWindowMessageW, SMTO_ABORTIFHUNG,
    SendMessageTimeoutW, WM_CLOSE,
};
use windows::core::BOOL;

/// How long any single message exchange may take.
///
/// A deadline on a condition, never a sleep used to order events: a hung or
/// crashed plugin must fail the test, not hang it.
const MESSAGE_TIMEOUT_MS: u32 = 20_000;

/// How long the guest waits for the configuration window to appear or to be
/// gone.
const DIALOG_DEADLINE: Duration = Duration::from_secs(20);

/// The fixture's own trace file, in the process temp directory.
fn trace_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "{TRACE_FILE_PREFIX}{}.{TRACE_FILE_EXTENSION}",
        std::process::id()
    ))
}

/// The OS-registered id of the private challenge message.
fn challenge_message() -> u32 {
    let name: Vec<u16> = CHALLENGE_MESSAGE_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: the name is NUL-terminated and outlives the call.
    let id = unsafe { RegisterWindowMessageW(windows::core::PCWSTR(name.as_ptr())) };
    assert_ne!(id, 0, "the challenge message must be registerable");
    id
}

/// Send one challenge to the fixture's own window and return its answer.
///
/// A timeout, not an unbounded wait: a wedged plugin fails the test.
fn challenge(hwnd: usize, cmd: usize, arg: usize) -> u32 {
    let mut result: usize = 0;
    // SAFETY: the handle is the fixture's live window; the result slot is
    // writable.
    let sent = unsafe {
        SendMessageTimeoutW(
            HWND(hwnd as *mut core::ffi::c_void),
            challenge_message(),
            WPARAM(cmd),
            LPARAM(arg as isize),
            SMTO_ABORTIFHUNG,
            MESSAGE_TIMEOUT_MS,
            Some(&mut result),
        )
    };
    assert_ne!(
        sent.0, 0,
        "the fixture did not answer challenge {cmd} within {MESSAGE_TIMEOUT_MS} ms; \
         a hung plugin is a failure, not a pass"
    );
    result as u32
}

/// The window procedure currently installed on a window.
fn window_proc(hwnd: usize) -> usize {
    // SAFETY: the handle is a live window of this process.
    let value = unsafe { GetWindowLongPtrW(HWND(hwnd as *mut core::ffi::c_void), GWLP_WNDPROC) };
    value as usize
}

/// The process id that owns a window.
fn window_pid(hwnd: usize) -> u32 {
    let mut pid = 0_u32;
    // SAFETY: the pointer is writable and the handle is live.
    unsafe { GetWindowThreadProcessId(HWND(hwnd as *mut core::ffi::c_void), Some(&mut pid)) };
    pid
}

/// The thread id that owns a window.
fn window_tid(hwnd: usize) -> u32 {
    // SAFETY: the handle is live.
    unsafe { GetWindowThreadProcessId(HWND(hwnd as *mut core::ffi::c_void), None) }
}

/// The class name of a window.
fn window_class(hwnd: usize) -> String {
    let mut buffer = [0_u16; 128];
    // SAFETY: the buffer is writable for its length and the handle is live.
    let length = unsafe { GetClassNameW(HWND(hwnd as *mut core::ffi::c_void), &mut buffer) };
    if length <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..length as usize])
}

// The windows `EnumWindows` reported, collected on the calling thread.
thread_local! {
    static FOUND_WINDOWS: std::cell::RefCell<Vec<(usize, String, u32)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The windows of this process, collected by `EnumWindows`.
fn collect_windows() -> Vec<(usize, String, u32)> {
    FOUND_WINDOWS.with(|found| found.borrow_mut().clear());
    // SAFETY: the callback only reads the window and appends to the
    // thread-local list of this thread.
    unsafe {
        let _ = EnumWindows(Some(collect_window), LPARAM(0));
    }
    FOUND_WINDOWS.with(|found| found.borrow().clone())
}

/// The `EnumWindows` callback: record every top-level window.
unsafe extern "system" fn collect_window(hwnd: HWND, _lparam: LPARAM) -> BOOL {
    let handle = hwnd.0 as usize;
    let class = window_class(handle);
    let pid = window_pid(handle);
    FOUND_WINDOWS.with(|found| found.borrow_mut().push((handle, class, pid)));
    BOOL(1)
}

/// The host's own answer to a classic IPC query, sent from this thread.
fn host_ipc(hwnd: usize, ipc_id: u32) -> u32 {
    let mut result: usize = 0;
    // SAFETY: the handle is the host's live window; the result slot is
    // writable.
    let sent = unsafe {
        SendMessageTimeoutW(
            HWND(hwnd as *mut core::ffi::c_void),
            WM_WA_IPC,
            WPARAM(0),
            LPARAM(ipc_id as isize),
            SMTO_ABORTIFHUNG,
            MESSAGE_TIMEOUT_MS,
            Some(&mut result),
        )
    };
    assert_ne!(sent.0, 0, "the host window did not answer IPC {ipc_id}");
    result as u32
}

/// What the guest observed about the configuration window.
struct DialogReport {
    hwnd: usize,
    class: String,
    owner: usize,
    pid: u32,
    tid: u32,
    closed: bool,
}

#[test]
#[ignore = "VM-only: maps a real 32-bit general-purpose plugin DLL, creates real Win32 windows and a configuration window; run in test VM"]
fn a_real_general_purpose_plugin_is_loaded_answers_from_its_own_code_and_quits_cleanly() {
    // 1. Gate before anything is mapped, created or spawned.
    require_test_vm();
    if !cfg!(target_arch = "x86") {
        panic!(
            "REFUSING LOUDLY: loading a 32-bit plugin DLL requires an i686 target; \
             cannot map it into a 64-bit process"
        );
    }

    // 2. The fixture, pinned by its own bytes.
    let dll = find_fixture_dll();
    let bytes = std::fs::read(&dll).expect("the fixture must be readable");
    println!(
        "FIXTURE {} size={} sha256={}",
        dll.display(),
        bytes.len(),
        rustamp_plughost::approval::Sha256::digest_hex(&bytes)
    );

    // 3. The host's real main window (class `Winamp v1.x`).
    let surface = IpcSurface::new();
    let window = HostWindow::create(surface).expect("HostWindow creation failed in test VM");
    let host_hwnd = window.hwnd_raw();
    assert_ne!(host_hwnd, 0, "the host window must exist");
    assert_eq!(
        window_class(host_hwnd),
        "Winamp v1.x",
        "the host window must carry the class the ABI fixes (S09 §1)"
    );
    // Start the plugin's trace from scratch: a stale file must never be read
    // as this run's evidence.
    let _ = std::fs::remove_file(trace_path());
    // The host's own window procedure, before any plugin touches it. The
    // fixture has to put exactly this back when it quits.
    let original_proc = window_proc(host_hwnd);

    // 4. Load the DLL through the approval store — the production-shaped path.
    let mut store = ApprovalStore::new();
    store
        .approve_bytes(FIXTURE_FILE_NAME, &bytes)
        .expect("the fixture must pass the passive preflight as a general plugin");
    // SAFETY: running inside the gated test VM on i686 with the owner's own
    // fixture, whose bytes were approved above.
    let mut loaded = unsafe {
        load_plugin_with_store(&dll, &store, &window).expect("the fixture must load into the host")
    };

    // 5. The module the host got from the DLL.
    assert_eq!(loaded.kind(), PluginKind::General);
    assert!(
        !loaded.description().is_empty(),
        "the plugin's own description must be readable"
    );
    println!(
        "PLUGIN_LOADED kind={:?} name={:?}",
        loaded.kind(),
        loaded.description()
    );
    // Let anything the plugin queued during init settle.
    message_loop::drain(&window);
    let info = loaded
        .general_info()
        .expect("a general plugin must report its module info");
    println!("PLUGIN_INFO {info:?}");
    assert!(
        info.has_init && info.has_config && info.has_quit,
        "the fixture must provide all three slots: {info:?}"
    );
    assert!(info.init_called, "init must have run");
    assert_eq!(
        info.init_result, 0,
        "the fixture's init must report success"
    );
    assert_eq!(
        info.parent_window, host_hwnd,
        "the host must fill the parent window before init"
    );
    assert_eq!(
        info.dll_instance,
        loaded.hinstance() as usize,
        "the host must fill the plugin's own module handle before init"
    );

    // 6. The fixture's own window, found the way an observer would find it:
    //    by class name and process id, not by a handle the host handed out.
    let own = collect_windows()
        .into_iter()
        .find(|(handle, class, pid)| {
            *handle != host_hwnd && class == FIXTURE_WINDOW_CLASS && *pid == std::process::id()
        })
        .map(|(handle, _, _)| handle)
        .expect("the fixture must have created its own window in init");
    println!(
        "FIXTURE_WINDOW hwnd={own:#x} pid={} tid={}",
        window_pid(own),
        window_tid(own)
    );
    assert_eq!(window_pid(own), std::process::id());
    assert_eq!(
        window_tid(own),
        window_tid(host_hwnd),
        "the plugin's window belongs to the UI thread, like the host's"
    );
    assert_ne!(
        window_proc(host_hwnd),
        original_proc,
        "the fixture must have subclassed the host's window procedure"
    );
    assert_eq!(
        challenge(own, CMD_PARENT_HWND, 0) as usize,
        host_hwnd,
        "the plugin must have seen the host's main window"
    );
    assert_eq!(
        challenge(own, CMD_PARENT_CLASS_OK, 0),
        1,
        "the parent window's class must be Winamp v1.x (S09 §1)"
    );
    assert_eq!(challenge(own, CMD_OWN_HWND, 0) as usize, own);
    assert_eq!(challenge(own, CMD_INIT_RESULT, 0), 0);
    assert_eq!(
        challenge(own, CMD_SUBCLASS_STATE, 0),
        1,
        "the subclass must be installed while the plugin is live"
    );
    assert_eq!(challenge(own, CMD_QUIT_CALLED, 0), 0);
    assert_eq!(challenge(own, CMD_CONFIG_HWND, 0), 0);

    // 7. A challenge to the plugin's OWN window is answered by the plugin's
    //    own window procedure (P3 §4 step 8).
    let token = 0x5EED_1234_u32;
    assert_eq!(
        challenge(own, CMD_ECHO, token as usize),
        token,
        "the plugin's own window procedure must answer the challenge"
    );

    // 8. The IPC query the plugin sends **from its own code** (P3 §4 step 6).
    //    The host's own trace must show the same query, which proves it
    //    travelled through the host's original window procedure.
    let before = window.recorded_ipc_queries().len();
    let version_reply = challenge(own, CMD_IPC_VERSION_REPLY, 0);
    assert_eq!(
        version_reply, WINAMP_VERSION_295,
        "the version query from the plugin's own code must answer 0x2905"
    );
    let queries = window.recorded_ipc_queries();
    assert_eq!(
        queries.len(),
        before + 1,
        "exactly one query must have reached the host's window procedure"
    );
    assert_eq!(
        queries[before],
        (IPC_GETVERSION, 0, WINAMP_VERSION_295),
        "the host's own trace must agree with what the plugin saw"
    );

    // 9. An unknown IPC id answers 1 (S10 §1.5; P3 §4 step 7).
    let before = window.recorded_ipc_queries().len();
    let unknown_reply = challenge(own, CMD_IPC_UNKNOWN_REPLY, 0);
    assert_eq!(
        unknown_reply, 1,
        "an unsupported IPC id must answer 1, not 0"
    );
    let queries = window.recorded_ipc_queries();
    assert_eq!(queries.len(), before + 1);
    assert_eq!(queries[before], (UNKNOWN_IPC_ID, 0, 1));

    let entries = challenge(own, CMD_SUBCLASS_ENTRIES, 0);
    assert!(
        entries >= 2,
        "the subclass must have forwarded both plugin-originated queries"
    );

    // 10. The configuration window (P3 §4 step 9). The guest closes it: a
    //     helper thread waits for it, checks its owner and closes it.
    let (report_tx, report_rx) = mpsc::channel();
    let guest = std::thread::spawn(move || {
        let deadline = Instant::now() + DIALOG_DEADLINE;
        let mut dialog = 0_usize;
        while Instant::now() < deadline {
            dialog = challenge(own, CMD_CONFIG_HWND, 0) as usize;
            if dialog != 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if dialog == 0 {
            let _ = report_tx.send(DialogReport {
                hwnd: 0,
                class: String::new(),
                owner: 0,
                pid: 0,
                tid: 0,
                closed: false,
            });
            return;
        }
        // SAFETY: the handle is the fixture's live configuration window.
        let owner = unsafe { GetWindow(HWND(dialog as *mut core::ffi::c_void), GW_OWNER) }
            .map(|hwnd| hwnd.0 as usize)
            .unwrap_or(0);
        let class = window_class(dialog);
        let pid = window_pid(dialog);
        let tid = window_tid(dialog);
        // The explicit guest action: close the configuration window.
        // SAFETY: the handle is the fixture's live configuration window.
        let _ = unsafe {
            PostMessageW(
                Some(HWND(dialog as *mut core::ffi::c_void)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
        // Wait until the window is really gone.
        let deadline = Instant::now() + DIALOG_DEADLINE;
        let mut closed = false;
        while Instant::now() < deadline {
            // SAFETY: the handle is the window the plugin created.
            if !unsafe { IsWindow(Some(HWND(dialog as *mut core::ffi::c_void))) }.as_bool() {
                closed = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = report_tx.send(DialogReport {
            hwnd: dialog,
            class,
            owner,
            pid,
            tid,
            closed,
        });
    });

    // The host calls the plugin's config callback with no arguments, on the
    // UI thread; it returns when the guest closed the window.
    let config = loaded.config();
    assert!(
        config.is_ok(),
        "the host must call the plugin's config: {config:?}"
    );

    let report = report_rx
        .recv_timeout(DIALOG_DEADLINE)
        .expect("the guest must report what it saw of the configuration window");
    guest.join().expect("the guest thread must not panic");
    assert_ne!(
        report.hwnd, 0,
        "the fixture must have opened a configuration window"
    );
    assert_ne!(
        report.hwnd, own,
        "the configuration window must be a window of its own"
    );
    assert_eq!(
        report.class, FIXTURE_CONFIG_CLASS,
        "the configuration window must carry the fixture's own class"
    );
    assert_eq!(
        report.owner, host_hwnd,
        "the configuration window must be owned by the host's main window (S09 §8.7)"
    );
    assert_eq!(report.pid, std::process::id());
    assert_eq!(
        report.tid,
        window_tid(host_hwnd),
        "the configuration window belongs to the UI thread"
    );
    assert!(
        report.closed,
        "the guest's explicit close must destroy the window"
    );
    // SAFETY: the handle is the window the plugin created and destroyed.
    assert!(
        !unsafe { IsWindow(Some(HWND(report.hwnd as *mut core::ffi::c_void))) }.as_bool(),
        "the configuration window must be gone after the close"
    );
    assert_eq!(
        challenge(own, CMD_CONFIG_HWND, 0),
        0,
        "the fixture must forget its configuration window"
    );

    // 11. Quit (P3 §4 step 10): the windows go, the host's original window
    //     procedure comes back, and the DLL stays mapped.
    loaded.quit();
    assert!(!loaded.is_initialized());
    // SAFETY: the handle is the window the plugin created.
    assert!(
        !unsafe { IsWindow(Some(HWND(own as *mut core::ffi::c_void))) }.as_bool(),
        "the fixture must destroy its own window in quit"
    );
    assert_eq!(
        window_proc(host_hwnd),
        original_proc,
        "the fixture must restore the host's original window procedure"
    );
    // The host still answers after the restore.
    assert_eq!(
        host_ipc(host_hwnd, IPC_GETVERSION),
        WINAMP_VERSION_295,
        "the host must still answer after the plugin restored the procedure"
    );
    // The general plugin's DLL is not freed at shutdown (S09 §1 line 46).
    let mapped = mapped_path(loaded.hinstance());
    println!("MAPPED {mapped:?}");

    // 12. The plugin's own trace, written from inside the DLL.
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    println!("FIXTURE_TRACE\n{trace}");
    let mut position = 0_usize;
    for event in [
        "init parent=",
        "parent class ok=1",
        "subclass installed",
        "window created hwnd=",
        "ipc id=0 reply=0x2905",
        "ipc id=9999 reply=0x1",
        "config opened hwnd=",
        "config closed",
        "quit subclass_restored=true window_destroyed=true",
    ] {
        let found = trace[position..]
            .find(event)
            .unwrap_or_else(|| panic!("the fixture's trace must contain '{event}':\n{trace}"));
        position += found + event.len();
    }

    window.destroy();
}

/// The file the OS mapped for a module handle.
fn mapped_path(hinstance: *mut core::ffi::c_void) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleFileNameW(module: *mut core::ffi::c_void, path: *mut u16, size: u32) -> u32;
    }
    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: the module handle belongs to a mapped DLL of this process and
    // the buffer is writable for its length.
    let written =
        unsafe { GetModuleFileNameW(hinstance, buffer.as_mut_ptr(), buffer.len() as u32) };
    assert!(written != 0, "the plugin's module must still be mapped");
    PathBuf::from(std::ffi::OsString::from_wide(&buffer[..written as usize]))
}
