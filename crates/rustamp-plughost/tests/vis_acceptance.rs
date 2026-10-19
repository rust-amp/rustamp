//! VM-only acceptance test for the **visualization** plugin kind: the `vis_`
//! half of the classic-plugin ABI of `specs/S09-plugin-abi.md` §5.
//!
//! It loads the clean-room `vis_` fixture
//! (`crates/rustamp-plughost/examples/vis_rustamp_fixture.rs`) through the
//! production loader and proves, in one real i686 process:
//!
//! 1. the approved bytes are loaded from a **separate DLL** through the OS
//!    loader, the export `winampVisGetHeader` resolves, and the header
//!    reports version `0x101` with a readable library name (S09 §2, §5.1);
//! 2. the module list is the library's own: the host asks `get_module(0)`,
//!    `get_module(1)`, ... until the answer is null, and reads each module's
//!    name, latency, delay and row counts out of the plugin's own struct
//!    (S09 §5.1, §5.2);
//! 3. the host filled the parent window, the DLL instance, the sample rate
//!    and the channel count into the chosen module **before** its `init` ran
//!    (S09 §5.2, §5.3 step 2);
//! 4. a frame is drawn: the data arrays are zeroed (no frame is available
//!    yet) and the plugin's `render` is called; the plugin's own return value
//!    is the host's answer (S09 §5.2, §5.3 step 4b);
//! 5. only one visualization runs at a time: starting the second module quits
//!    the first, and a module whose `init` refuses is not activated
//!    (S09 §5.2, §5.3 step 1);
//! 6. `quit` reaches the plugin and the DLL stays mapped (S09 §5.3 step 5,
//!    §1 line 46).
//!
//! The plugin's own trace file — written from inside the DLL — is checked for
//! the same events, so the evidence does not come from the runner alone.
//!
//! What this is **not**: a claim about third-party visualization plugins
//! (MilkDrop, AVS, Geiss), about the frame data (the audio path does not
//! exist yet), about the vis thread (the host owns it) or about the
//! visualization list in the preferences. P3 §7 lists the surface test,
//! `PluginLoaded` and a green build as false passes; this test proves none of
//! them.
//!
//! It refuses loudly (never skips) when it is not in the test VM, not on
//! i686, or when the fixture DLL is missing.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

#![cfg(windows)]

#[path = "common/vis_protocol.rs"]
mod vis_protocol;

use vis_protocol::*;

use std::path::PathBuf;

use rustamp_plughost::approval::ApprovalStore;
use rustamp_plughost::host_window::{HostWindow, require_test_vm};
use rustamp_plughost::ipc::IpcSurface;
use rustamp_plughost::loader::load_plugin_with_store;
use rustamp_plughost::vis::VisError;
use rustamp_plugin_abi::PluginKind;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;

/// The file name of the fixture DLL.
const FIXTURE_FILE_NAME: &str = "vis_rustamp_fixture.dll";

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
    if let Some(value) = std::env::var_os("RUSTAMP_VIS_FIXTURE") {
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
        "REFUSING LOUDLY: missing the visualization fixture {FIXTURE_FILE_NAME}. Checked {checked:?}. \
         Build it with `cargo build -p rustamp-plughost --example vis_rustamp_fixture --target \
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

/// How often a line appears in the fixture's trace.
fn occurrences(trace: &str, needle: &str) -> usize {
    trace.lines().filter(|line| *line == needle).count()
}

/// The text of a NUL-terminated description constant of the shared protocol.
fn description_text(raw: &[u8]) -> String {
    let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

#[test]
#[ignore = "VM-only: maps a real 32-bit visualization plugin DLL into the host; run in test VM"]
fn a_real_visualization_plugin_is_loaded_listed_started_drawn_and_quit_cleanly() {
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
        .expect("the fixture must pass the passive preflight as a visualization plugin");
    // SAFETY: running inside the gated test VM on i686 with the owner's own
    // fixture, whose bytes were approved above.
    let mut loaded = unsafe {
        load_plugin_with_store(&dll, &store, &window).expect("the fixture must load into the host")
    };

    // 5. The header and the module list.
    assert_eq!(
        loaded.kind(),
        PluginKind::Visualization,
        "winampVisGetHeader must select the visualization kind (S09 §2)"
    );
    let library = loaded
        .vis_info()
        .expect("a loaded visualization plugin must expose its library info");
    println!("PLUGIN_LOADED kind={:?} library={library:?}", loaded.kind());
    assert_eq!(
        library.version, 0x101,
        "the header version must be 0x101 (S09 §5.1)"
    );
    assert_eq!(
        library.description,
        description_text(FIXTURE_LIBRARY_DESCRIPTION),
        "the library name must be readable from plugin memory"
    );
    assert_eq!(
        library.modules.len(),
        2,
        "the module list stops at the first null answer of get_module (S09 §5.1)"
    );
    let first = &library.modules[0];
    assert_eq!(first.index, 0);
    assert_eq!(
        first.description,
        description_text(FIXTURE_MODULE_A_DESCRIPTION)
    );
    assert_eq!(first.latency_ms, FIXTURE_LATENCY_MS);
    assert_eq!(first.delay_ms, FIXTURE_DELAY_MS);
    assert_eq!(first.spectrum_rows, FIXTURE_SPECTRUM_ROWS);
    assert_eq!(first.waveform_rows, FIXTURE_WAVEFORM_ROWS);
    assert!(first.has_init && first.has_render && first.has_quit);
    assert_eq!(library.modules[1].index, 1);
    assert_eq!(
        library.modules[1].description,
        description_text(FIXTURE_MODULE_B_DESCRIPTION)
    );

    // No module runs before the user picks one (S09 §5.3 step 1).
    assert!(
        !loaded.is_initialized(),
        "a visualization is not running before a module is chosen"
    );
    assert_eq!(
        loaded.vis_render(),
        Err(VisError::NotRunning),
        "no frame may be drawn before a module runs"
    );

    // 6. Start the first module: the host fills the members before `init`.
    loaded.vis_start(0).expect("the first module must start");
    assert!(loaded.is_initialized());
    let run = loaded
        .vis_run_info()
        .expect("a running visualization reports its state");
    println!("VIS_RUN {run:?}");
    assert_eq!(run.index, 0);
    assert_eq!(
        run.parent_window, host_hwnd,
        "the host must fill parent_window with the main window before init (S09 §5.2)"
    );
    assert_eq!(
        run.dll_instance,
        loaded.hinstance() as usize,
        "the host must fill dll_instance with the plugin's own module handle (S09 §5.2)"
    );
    assert_eq!(run.sample_rate, 44_100, "the default rate of S09 §5.2");
    assert_eq!(run.channels, 2, "the default channel count of S09 §5.2");
    assert!(run.init_called);
    assert_eq!(run.init_result, 0);
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    println!("FIXTURE_TRACE\n{trace}");
    assert!(
        trace.contains(&init_line(
            0,
            host_hwnd,
            loaded.hinstance() as usize,
            44_100,
            2
        )),
        "the plugin must have seen the host's window, its own instance, 44100 Hz and 2 channels \
         before its init ran:\n{trace}"
    );

    // 7. A frame: the arrays are zeroed, because no frame is available yet.
    assert_eq!(
        loaded.vis_render().expect("a frame must be drawn"),
        0,
        "0 means the visualization continues (S09 §5.2)"
    );
    assert_eq!(
        loaded.vis_render().expect("a frame must be drawn"),
        0,
        "0 means the visualization continues (S09 §5.2)"
    );
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    assert_eq!(
        occurrences(&trace, &render_line(0, 0, 0)),
        2,
        "both frames must reach the plugin, with zeroed data arrays:\n{trace}"
    );

    // 8. Only one visualization runs at a time: the second module's `init`
    //    refuses, and the first one is quit before it (S09 §5.3 step 1).
    assert_eq!(
        loaded.vis_start(1),
        Err(VisError::InitFailed(FIXTURE_INIT_REFUSAL_CODE)),
        "a refusing init must abort the start (S09 §5.2)"
    );
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    assert_eq!(
        occurrences(&trace, &quit_line(0)),
        1,
        "the running visualization must be stopped before the next one starts:\n{trace}"
    );
    assert!(
        trace.contains(&init_line(
            1,
            host_hwnd,
            loaded.hinstance() as usize,
            44_100,
            2
        )),
        "the second module's init must have run and refused:\n{trace}"
    );
    assert!(
        !loaded.is_initialized(),
        "a refused start must leave no visualization running"
    );
    assert_eq!(
        loaded.vis_render(),
        Err(VisError::NotRunning),
        "no frame may be drawn after a refused start"
    );

    // 9. Start the first module again, draw a frame, and quit.
    loaded
        .vis_start(0)
        .expect("the first module must start again");
    assert_eq!(loaded.vis_render().expect("a frame must be drawn"), 0);
    loaded.quit();
    assert!(!loaded.is_initialized());
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    println!("FIXTURE_TRACE\n{trace}");
    assert_eq!(
        occurrences(&trace, &quit_line(0)),
        2,
        "quit must reach the plugin exactly once per start:\n{trace}"
    );
    assert_eq!(
        occurrences(
            &trace,
            &init_line(0, host_hwnd, loaded.hinstance() as usize, 44_100, 2)
        ),
        2,
        "both starts must have run the plugin's init:\n{trace}"
    );
    assert!(
        !trace.contains(EVENT_CONFIG),
        "the host must not call the module's config callback:\n{trace}"
    );

    // The visualization DLL stays mapped after its quit (S09 §1 line 46: the
    // 5.x host frees it, this host keeps the conservative policy).
    let mapped = mapped_path(loaded.hinstance());
    println!("MAPPED {mapped:?}");

    window.destroy();
}
