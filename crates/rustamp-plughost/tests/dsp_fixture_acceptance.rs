//! VM-only acceptance test for the **DSP** plugin kind: the `dsp_` half of
//! the classic-plugin ABI of `specs/S09-plugin-abi.md` §6, driven by our own
//! clean-room fixture.
//!
//! It loads the `dsp_` fixture
//! (`crates/rustamp-plughost/examples/dsp_rustamp_fixture.rs`) through the
//! production loader and proves, in one real i686 process:
//!
//! 1. the approved bytes are loaded from a **separate DLL** through the OS
//!    loader, the export `winampDSPGetHeader2` resolves, and the header
//!    reports version `0x20` with a readable library name (S09 §2, §6.1);
//! 2. the module list is the library's own: the host asks `get_module(0)`,
//!    `get_module(1)`, ... until the answer is null, and reads each module's
//!    name and callback slots out of the plugin's own struct (S09 §6.1,
//!    §6.2);
//! 3. the host filled the parent window and the DLL instance into the chosen
//!    module **before** its `init` ran (S09 §6.2, §6.3 step 1);
//! 4. the module's `process` runs in place and the audio comes back changed
//!    by exactly the fixture's gain, measured against the unprocessed copy
//!    (S09 §6.2);
//! 5. only one DSP module is active at a time: changing the selection quits
//!    the previous module, and a module whose `init` refuses is not activated
//!    (S09 §6.3 step 2, step 4);
//! 6. the `config` callback is routed to the active module and reaches the
//!    plugin (S09 §6.2, offset 0x0C);
//! 7. `quit` reaches the plugin and the DLL stays mapped (S09 §6.3 step 4,
//!    §1 line 46).
//!
//! The plugin's own trace file — written from inside the DLL — is checked for
//! the same events, so the evidence does not come from the runner alone.
//!
//! What this is **not**: a claim about third-party DSP plugins, about the
//! audio path (the `dsp_process` call site does not exist yet), about the
//! equalizer order (S09 U-12) or about the Plug-ins list in the preferences.
//! The separate `tests/dsp_acceptance.rs` runs a real third-party plugin.
//!
//! It refuses loudly (never skips) when it is not in the test VM, not on
//! i686, or when the fixture DLL is missing.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

#![cfg(windows)]

#[path = "common/dsp_protocol.rs"]
mod dsp_protocol;

use dsp_protocol::*;

use std::path::PathBuf;

use rustamp_plughost::approval::ApprovalStore;
use rustamp_plughost::dsp::DspError;
use rustamp_plughost::host_window::{HostWindow, require_test_vm};
use rustamp_plughost::ipc::IpcSurface;
use rustamp_plughost::loader::load_plugin_with_store;
use rustamp_plugin_abi::PluginKind;

/// The file name of the fixture DLL.
const FIXTURE_FILE_NAME: &str = "dsp_rustamp_fixture.dll";

/// The number of channels of the test signal.
const CHANNELS: usize = 2;

/// The sample rate of the test signal.
const SAMPLE_RATE: u32 = 44_100;

/// The frames per processed block.
const BLOCK_FRAMES: usize = 512;

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
    if let Some(value) = std::env::var_os("RUSTAMP_DSP_FIXTURE") {
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
        "REFUSING LOUDLY: missing the DSP fixture {FIXTURE_FILE_NAME}. Checked {checked:?}. \
         Build it with `cargo build -p rustamp-plughost --example dsp_rustamp_fixture --target \
         i686-pc-windows-msvc` and stage it beside the test binaries. This test refuses rather \
         than passing vacuously."
    );
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

/// A deterministic interleaved stereo test signal, `frames` long.
///
/// The phase starts one sample in, so the first sample is not silence and the
/// fixture's trace of the first processed sample is meaningful.
fn signal_block(frames: usize) -> Vec<i16> {
    let mut pcm = Vec::with_capacity(frames * CHANNELS);
    for n in 0..frames {
        let t = (n + 1) as f64 / f64::from(SAMPLE_RATE);
        let left = 12_000.0 * (2.0 * std::f64::consts::PI * 440.0 * t).sin();
        let right = 8_000.0 * (2.0 * std::f64::consts::PI * 660.0 * t).sin();
        pcm.push(left.round() as i16);
        pcm.push(right.round() as i16);
    }
    pcm
}

#[test]
#[ignore = "VM-only: maps a real 32-bit DSP plugin DLL into the host; run in test VM"]
fn a_real_dsp_plugin_is_loaded_listed_started_processed_and_quit_cleanly() {
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

    // Start the plugin's trace from scratch: a stale file must never be read
    // as this run's evidence.
    let _ = std::fs::remove_file(trace_path());

    // 4. Load the DLL through the approval store — the production-shaped path.
    let mut store = ApprovalStore::new();
    store
        .approve_bytes(FIXTURE_FILE_NAME, &bytes)
        .expect("the fixture must pass the passive preflight as a DSP plugin");
    // SAFETY: running inside the gated test VM on i686 with our own fixture,
    // whose bytes were approved above.
    let mut loaded = unsafe {
        load_plugin_with_store(&dll, &store, &window).expect("the fixture must load into the host")
    };

    // 5. The header and the module list.
    assert_eq!(
        loaded.kind(),
        PluginKind::Dsp,
        "winampDSPGetHeader2 must select the DSP bridge (S09 §2)"
    );
    let library = loaded
        .dsp_info()
        .expect("a loaded DSP plugin must expose its library info");
    println!("PLUGIN_LOADED kind={:?} library={library:?}", loaded.kind());
    assert_eq!(
        library.version, 0x20,
        "the header version must be 0x20 (S09 §6.1)"
    );
    assert_eq!(
        library.description,
        description_text(FIXTURE_LIBRARY_DESCRIPTION),
        "the library name must be readable from plugin memory"
    );
    assert_eq!(
        library.modules.len(),
        2,
        "the module list stops at the first null answer of get_module (S09 §6.1)"
    );
    assert_eq!(library.modules[0].index, 0);
    assert_eq!(
        library.modules[0].description,
        description_text(FIXTURE_MODULE_A_DESCRIPTION)
    );
    assert!(library.modules[0].has_init);
    assert!(library.modules[0].has_process);
    assert!(library.modules[0].has_quit);
    assert!(library.modules[0].has_config);
    assert_eq!(library.modules[1].index, 1);
    assert_eq!(
        library.modules[1].description,
        description_text(FIXTURE_MODULE_B_DESCRIPTION)
    );

    // 6. The selected module was started at load time (S09 §6.3 step 1).
    assert!(
        loaded.is_initialized(),
        "the selected DSP module must be active after the load"
    );
    let run = loaded
        .dsp_run_info()
        .expect("the active DSP module must report its state");
    println!("DSP_RUN {run:?}");
    assert_eq!(run.index, 0);
    assert_eq!(
        run.parent_window, host_hwnd,
        "the host must fill parent_window with the main window before init (S09 §6.2)"
    );
    assert_eq!(
        run.dll_instance,
        loaded.hinstance() as usize,
        "the host must fill dll_instance with the plugin's own module handle before init (S09 §6.2)"
    );
    assert!(run.init_called);
    assert_eq!(run.init_result, 0, "S09 §6.2: init returns 0 on success");
    assert!(
        run.host_active,
        "dsp_is_active must answer 1 for the active module (S09 §3.1 offset 0x84)"
    );
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    println!("FIXTURE_TRACE\n{trace}");
    assert!(
        trace.contains(&init_line(0, host_hwnd, loaded.hinstance() as usize)),
        "the plugin must have seen the host's window and its own instance \
         before its init ran:\n{trace}"
    );

    // 7. The audio path: one block through the module's `process`.
    let original = signal_block(BLOCK_FRAMES);
    let mut work = original.clone();
    let out_frames = loaded
        .dsp_process_i16(&mut work, CHANNELS, SAMPLE_RATE)
        .expect("process must be driven for an active DSP plugin");
    assert_eq!(
        out_frames, BLOCK_FRAMES,
        "the fixture returns the frame count it was given (S09 §6.2)"
    );
    assert_eq!(work.len(), out_frames * CHANNELS);
    for (before, after) in original.iter().zip(&work) {
        assert_eq!(
            *after,
            before.saturating_mul(FIXTURE_GAIN),
            "the fixture's gain must be visible sample by sample"
        );
    }
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    assert!(
        trace.contains(&process_line(
            0,
            BLOCK_FRAMES as i32,
            BLOCK_FRAMES as i32,
            original[0].saturating_mul(FIXTURE_GAIN)
        )),
        "the block must have reached the plugin's process:\n{trace}"
    );

    // 8. Only one DSP module is active at a time: changing the selection
    //    quits the previous module (S09 §6.3 step 2 and step 4), and a module
    //    whose `init` refuses is not activated (S09 §6.2, offset 0x10).
    assert_eq!(
        loaded.dsp_start(1),
        Err(DspError::InitFailed(FIXTURE_INIT_REFUSAL_CODE)),
        "a refusing init must abort the start"
    );
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    assert_eq!(
        occurrences(&trace, &quit_line(0)),
        1,
        "the module that was active must be quit when the selection changes:\n{trace}"
    );
    assert!(
        trace.contains(&init_line(1, host_hwnd, loaded.hinstance() as usize)),
        "the second module's init must have run and refused:\n{trace}"
    );
    assert!(
        !loaded.is_initialized(),
        "a refused start must leave no DSP module active"
    );

    // 9. Start the first module again and drive one more block.
    loaded
        .dsp_start(0)
        .expect("the first module must start again");
    assert!(loaded.is_initialized());
    let mut work = signal_block(BLOCK_FRAMES);
    let out_frames = loaded
        .dsp_process_i16(&mut work, CHANNELS, SAMPLE_RATE)
        .expect("process must be driven for the active DSP plugin");
    assert_eq!(out_frames, BLOCK_FRAMES);

    // 10. The `config` callback is routed to the active module. This fixture's
    //     `config` opens no window, so the route is proven headlessly.
    loaded
        .config()
        .expect("the DSP module's config must be routed");
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    assert_eq!(
        occurrences(&trace, &format!("{EVENT_CONFIG}module=0")),
        1,
        "the config callback must reach the plugin exactly once:\n{trace}"
    );

    // 11. Clean shutdown: `quit` reaches the plugin and the DLL stays mapped.
    loaded.quit();
    assert!(!loaded.is_initialized());
    assert!(
        loaded.dsp_run_info().is_none(),
        "no DSP module is active after the quit, so dsp_is_active reads 0"
    );
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    println!("FIXTURE_TRACE\n{trace}");
    assert_eq!(
        occurrences(&trace, &quit_line(0)),
        2,
        "quit must reach the plugin exactly once per start:\n{trace}"
    );
    assert_eq!(
        loaded.dsp_process_i16(&mut signal_block(16), CHANNELS, SAMPLE_RATE),
        None,
        "process must not be driven after the quit"
    );

    window.destroy();
}
