//! VM-only acceptance test for the **output** plugin kind: the `out_` half of
//! `docs/tasks/P3-dll-acceptance.md` (§2, §4 steps 2–10).
//!
//! It loads the clean-room `out_` fixture
//! (`crates/rustamp-plughost/examples/out_rustamp_fixture.rs`) through the
//! production loader and proves, in one real i686 process:
//!
//! 1. the approved bytes are loaded from a **separate DLL** through the OS
//!    loader, the export `winampGetOutModule` resolves, and the module
//!    reports version `0x10` with a readable description (S09 §2, §4.1);
//! 2. the host filled the main window and the DLL instance **before** `init`
//!    ran, and the plugin saw the `Winamp v1.x` class (S09 §1, §4.1
//!    lines 233–237);
//! 3. the host left the module `id` alone (S09 §4.1 line 231, U-4);
//! 4. the host called **no** playback callback: `open`, `write`, `config` and
//!    `about` are driven by the input plugin or belong to another card
//!    (S09 §4.2 step 3);
//! 5. `quit` reaches the plugin and the DLL stays mapped (S09 §4.2 step 4,
//!    §1 line 46).
//!
//! The plugin's own trace file — written from inside the DLL — is checked for
//! the same events, so the evidence does not come from the runner alone.
//!
//! What this is **not**: a claim about third-party output plugins, about the
//! other plugin kinds, about audio, about the input plugin that would drive
//! the output, or about the output selection in the preferences. P3 §7 lists
//! the surface test, `PluginLoaded` and a green build as false passes; this
//! test proves none of them.
//!
//! It refuses loudly (never skips) when it is not in the test VM, not on
//! i686, or when the fixture DLL is missing.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

#![cfg(windows)]

#[path = "common/out_protocol.rs"]
mod out_protocol;

use out_protocol::*;

use std::path::PathBuf;

use rustamp_plughost::approval::ApprovalStore;
use rustamp_plughost::host_window::{HostWindow, require_test_vm};
use rustamp_plughost::ipc::IpcSurface;
use rustamp_plughost::loader::load_plugin_with_store;
use rustamp_plugin_abi::PluginKind;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;

/// The file name of the fixture DLL.
const FIXTURE_FILE_NAME: &str = "out_rustamp_fixture.dll";

/// The fixture's own trace file, in the process temp directory.
fn trace_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "{TRACE_FILE_PREFIX}{}.{TRACE_FILE_EXTENSION}",
        std::process::id()
    ))
}

/// Find the fixture DLL, or refuse loudly.
///
/// The search starts at the test executable's own directory, because the
/// runner stages the fixture beside the test binaries. A missing fixture must
/// never read as a pass or a quiet skip.
fn find_fixture_dll() -> PathBuf {
    let mut checked = Vec::new();

    // 1. Explicit override by the runner.
    if let Some(value) = std::env::var_os("RUSTAMP_OUT_FIXTURE") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return path;
        }
        checked.push(path);
    }

    // 2. Next to the test binary, and in the usual cargo output layout.
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let mut candidates = vec![dir.join(FIXTURE_FILE_NAME)];
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join(FIXTURE_FILE_NAME));
            if let Some(root) = parent.parent() {
                candidates.push(
                    root.join("i686-pc-windows-msvc")
                        .join("debug")
                        .join("examples")
                        .join(FIXTURE_FILE_NAME),
                );
            }
        }
        for candidate in candidates {
            if candidate.is_file() {
                return candidate;
            }
            checked.push(candidate);
        }
    }

    // 3. The runner's staged tree inside the guest.
    for candidate in [
        PathBuf::from(r"C:\rustamp\code\Rustamp\target\i686-pc-windows-msvc\debug\examples")
            .join(FIXTURE_FILE_NAME),
        PathBuf::from(r"C:\rustamp\code\Rustamp\target\debug\deps").join(FIXTURE_FILE_NAME),
    ] {
        if candidate.is_file() {
            return candidate;
        }
        checked.push(candidate);
    }

    panic!(
        "REFUSING LOUDLY: missing the output fixture {FIXTURE_FILE_NAME}. Checked {checked:?}. \
         Build it with `cargo build -p rustamp-plughost --example out_rustamp_fixture --target \
         i686-pc-windows-msvc` and stage it beside the test binaries. This test refuses rather \
         than passing vacuously."
    );
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

#[test]
#[ignore = "VM-only: maps a real 32-bit output plugin DLL into the host; run in test VM"]
fn a_real_output_plugin_is_loaded_gets_its_host_fields_and_quits_cleanly() {
    // 1. Gate before anything is mapped or created.
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

    // 4. Load the DLL through the approval store — the production-shaped path.
    let mut store = ApprovalStore::new();
    store
        .approve_bytes(FIXTURE_FILE_NAME, &bytes)
        .expect("the fixture must pass the passive preflight as an output plugin");
    // SAFETY: running inside the gated test VM on i686 with the owner's own
    // fixture, whose bytes were approved above.
    let mut loaded = unsafe {
        load_plugin_with_store(&dll, &store, &window).expect("the fixture must load into the host")
    };

    // 5. The module the host got from the DLL.
    assert_eq!(
        loaded.kind(),
        PluginKind::Output,
        "winampGetOutModule must select the output kind (S09 §2)"
    );
    assert!(
        !loaded.description().is_empty(),
        "the plugin's own description must be readable"
    );
    println!(
        "PLUGIN_LOADED kind={:?} name={:?}",
        loaded.kind(),
        loaded.description()
    );
    assert!(loaded.is_initialized(), "plugin must be marked initialized");

    // 6. The host-filled members and the untouched id.
    let info = loaded
        .output_info()
        .expect("a loaded output plugin must expose its module info");
    println!("PLUGIN_INFO {info:?}");
    assert!(
        info.has_init && info.has_quit,
        "the fixture must provide both slots: {info:?}"
    );
    assert!(info.init_called, "init must have run");
    assert_eq!(
        info.main_window, host_hwnd,
        "the host must fill the main window before init (S09 §4.1)"
    );
    assert_eq!(
        info.dll_instance,
        loaded.hinstance() as usize,
        "the host must fill the plugin's own module handle before init (S09 §4.1)"
    );
    assert_eq!(
        info.id, FIXTURE_MODULE_ID,
        "the host must not overwrite the module id (S09 §4.1, U-4)"
    );

    // 7. The plugin's own evidence, written from inside the DLL.
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    println!("FIXTURE_TRACE\n{trace}");
    assert!(
        trace.contains(&init_line(
            host_hwnd,
            loaded.hinstance() as usize,
            FIXTURE_MODULE_ID
        )),
        "the plugin must have seen the host's main window and its own instance:\n{trace}"
    );
    assert!(
        trace.contains(&parent_class_line(true)),
        "the parent window's class must be Winamp v1.x (S09 §1):\n{trace}"
    );
    for forbidden in [EVENT_OPEN, EVENT_WRITE, EVENT_CONFIG, EVENT_ABOUT] {
        assert!(
            !trace.contains(forbidden),
            "the host must not call '{forbidden}':\n{trace}"
        );
    }

    // 8. Quit (S09 §4.2 step 4): the plugin is told, the DLL stays mapped.
    loaded.quit();
    assert!(!loaded.is_initialized());
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    assert!(
        trace.contains(QUIT_LINE),
        "quit must reach the plugin:\n{trace}"
    );
    // The output plugin's DLL is not freed at shutdown (S09 §1 line 46).
    let mapped = mapped_path(loaded.hinstance());
    println!("MAPPED {mapped:?}");

    window.destroy();
}
