//! Acceptance test loading a real 32-bit classic Winamp DSP plugin DLL through
//! the Rustamp DSP bridge (task P6, card P8).
//!
//! Gated on the Hyper-V test VM ([`rustamp_plughost::require_test_vm`]).
//! Fixture: `dsp_optilab_core.dll`, OptiLab Core 1.4.0 (Apache-2.0,
//! github.com/dgl1984/optilab release v1.4.0), pinned below by size and SHA-256.
//!
//! A pass proves, with real plugin code in an i686 process:
//! 1. the approved bytes are the pinned fixture and a tampered copy is refused
//!    before any code runs;
//! 2. `winampDSPGetHeader2` resolves and yields a live header and module 0
//!    (kind `Dsp`, description read from plugin memory);
//! 3. the plugin is initialized: `init` ran and returned 0, the host filled
//!    `parent_window`/`dll_instance`, and the host `dsp_is_active` callback
//!    answers 1 for exactly this module;
//! 4. its `process` callback is driven through the host `dsp_process` callback
//!    (the path an input plugin's decode loop takes) and the audio comes back
//!    changed, measured against the unprocessed copy;
//! 5. `quit` runs and deactivates the module.
//!
//! The plugin's `config` callback is deliberately NOT invoked by this test:
//! for a DSP it opens a dialog, which this headless run must not do. The
//! host does route the callback (S09 §6.2, offset 0x0C) — the clean-room
//! fixture in `tests/dsp_fixture_acceptance.rs` proves that route with a
//! `config` that opens no window. "Configured" here means initialized with
//! host fields set, in the plugin's default state.
//!
//! Must refuse loudly (not skip silently) when the VM or the DLL is missing.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::path::PathBuf;

use rustamp_plughost::approval::{ApprovalStore, Sha256};
use rustamp_plughost::host_window::{HostWindow, require_test_vm};
use rustamp_plughost::ipc::IpcSurface;
use rustamp_plughost::loader::{PluginLoadError, load_plugin, load_plugin_with_store};
use rustamp_plughost::message_loop;
use rustamp_plugin_abi::PluginKind;

/// Pinned identity of the owner-approved fixture (host file, 283 KB).
const FIXTURE_NAME: &str = "dsp_optilab_core.dll";
const FIXTURE_SIZE: usize = 289_792;
const FIXTURE_SHA256: &str = "78a140c9114b60b361dc7e9a0203dd6d6f23d567e1f309396cd78baea7797874";

const CHANNELS: usize = 2;
const SAMPLE_RATE: u32 = 44_100;
const BLOCK_FRAMES: usize = 1152;
const BLOCKS: usize = 16;

/// Find the DSP fixture or refuse loudly.
fn find_dsp_dll() -> PathBuf {
    let mut checked = Vec::new();
    // 1. Explicit file set by the runner (RUSTAMP_TESTPLUGIN_DSP).
    if let Some(v) = std::env::var_os("RUSTAMP_TESTPLUGIN_DSP") {
        let p = PathBuf::from(v);
        if p.is_file() {
            return p;
        }
        checked.push(p);
    }
    // 2. The staged plugin directory.
    if let Some(v) = std::env::var_os("RUSTAMP_TESTPLUGINS") {
        let p = PathBuf::from(v);
        for cand in [p.join("dsp").join(FIXTURE_NAME), p.join(FIXTURE_NAME)] {
            if cand.is_file() {
                return cand;
            }
            checked.push(cand);
        }
    }
    // 3. Standard guest and orchestrator locations.
    for cand in [
        PathBuf::from(r"C:\rustamp\testplugins\dsp").join(FIXTURE_NAME),
        PathBuf::from(r"D:\code\rustamp-testplugins\dsp").join(FIXTURE_NAME),
    ] {
        if cand.is_file() {
            return cand;
        }
        checked.push(cand);
    }
    panic!(
        "REFUSING LOUDLY: missing DSP plugin binary {FIXTURE_NAME}. Checked {checked:?}. \
         DLL acceptance tests must refuse loudly by design when reference binaries are missing."
    );
}

/// Deterministic interleaved stereo test signal, `frames` long, starting at `start`.
fn signal_block(start: usize, frames: usize) -> Vec<i16> {
    let mut pcm = Vec::with_capacity(frames * CHANNELS);
    for n in start..start + frames {
        let t = n as f64 / f64::from(SAMPLE_RATE);
        let l = 12_000.0 * (2.0 * std::f64::consts::PI * 440.0 * t).sin();
        let r = 8_000.0 * (2.0 * std::f64::consts::PI * 660.0 * t).sin();
        pcm.push(l.round() as i16);
        pcm.push(r.round() as i16);
    }
    pcm
}

fn rms(pcm: &[i16]) -> f64 {
    if pcm.is_empty() {
        return 0.0;
    }
    let sum: f64 = pcm.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / pcm.len() as f64).sqrt()
}

#[test]
#[ignore = "VM-only: loads real 32-bit dsp_optilab_core.dll and creates a real Win32 host window; run in test VM"]
fn real_dsp_plugin_loads_initializes_and_changes_audio_through_the_bridge() {
    // 1. Gate on the Hyper-V test VM before any window or DLL is touched.
    require_test_vm();
    if !cfg!(target_arch = "x86") {
        panic!(
            "REFUSING LOUDLY: loading a 32-bit plugin DLL requires an i686 target; \
             cannot map it into a 64-bit process"
        );
    }

    // 2. Locate the fixture and pin its identity before it is ever mapped.
    let path = find_dsp_dll();
    let approved = std::fs::read(&path).expect("DSP fixture must be readable");
    assert_eq!(
        approved.len(),
        FIXTURE_SIZE,
        "fixture size differs from the pinned one"
    );
    assert_eq!(
        Sha256::digest_hex(&approved),
        FIXTURE_SHA256,
        "fixture SHA-256 differs from the pinned one; this test approves only the owner's fixture"
    );

    let surface = IpcSurface::new();
    let window = HostWindow::create(surface).expect("HostWindow creation failed in test VM");
    assert_ne!(window.hwnd_raw(), 0, "HostWindow HWND must be non-zero");

    // 3. Failure mode: bytes that are not the approved image are refused before
    //    the OS loader (and so DllMain) ever sees the file.
    let mut tampered = approved.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xFF;
    // SAFETY: refusal happens before any mapping; no plugin code can run.
    let refused = unsafe { load_plugin(&path, &tampered, &window) };
    assert!(
        matches!(refused, Err(PluginLoadError::NotApproved)),
        "a tampered approved image must be refused with NotApproved, got {:?}",
        refused.as_ref().err()
    );

    // 4. Load through the approval store (pinned hash), the production-shaped path.
    let mut store = ApprovalStore::new();
    store
        .approve_bytes(FIXTURE_NAME, &approved)
        .expect("fixture must pass passive preflight as a DSP plugin");
    // SAFETY: running inside the gated test VM on i686 with the pinned, approved fixture.
    let mut loaded = unsafe {
        load_plugin_with_store(&path, &store, &window)
            .expect("failed to load the real DSP plugin through the bridge")
    };

    // 5. Export resolved, header and module live, plugin initialized.
    assert_eq!(
        loaded.kind(),
        PluginKind::Dsp,
        "winampDSPGetHeader2 must select the DSP bridge"
    );
    println!("DSP description: {:?}", loaded.description());
    assert!(
        !loaded.description().is_empty(),
        "module 0 description must be readable from plugin memory"
    );
    assert!(
        loaded.is_initialized(),
        "plugin must be marked initialized (init returned 0)"
    );
    let library = loaded
        .dsp_info()
        .expect("a loaded DSP plugin must expose its library info");
    println!("DSP library info: {library:?}");
    assert_eq!(
        library.version, 0x20,
        "the header version must be 0x20 (S09 §6.1)"
    );
    assert!(
        !library.modules.is_empty(),
        "the module list must not be empty: {library:?}"
    );
    let info = &library.modules[0];
    assert!(
        info.has_init && info.has_process && info.has_quit,
        "module callbacks missing: {info:?}"
    );
    let run = loaded
        .dsp_run_info()
        .expect("the active DSP module must report its state");
    println!("DSP run info: {run:?}");
    assert_eq!(
        run.parent_window,
        window.hwnd_raw(),
        "host must fill parent_window with the main window before init"
    );
    assert_eq!(
        run.dll_instance,
        loaded.hinstance() as usize,
        "host must fill dll_instance with the plugin's own module handle before init"
    );
    assert!(
        run.host_active,
        "dsp_is_active must answer 1 for the loaded module"
    );

    // Let any window or message the plugin's init queued settle.
    message_loop::drain(&window);

    // 6. Drive `process` and measure.
    let mut frames_in = 0_usize;
    let mut frames_out = 0_usize;
    let mut compared = 0_usize;
    let mut differing = 0_usize;
    let mut max_abs_diff = 0_i32;
    let mut all_in = Vec::new();
    let mut all_out = Vec::new();
    for b in 0..BLOCKS {
        let original = signal_block(b * BLOCK_FRAMES, BLOCK_FRAMES);
        let mut work = original.clone();
        let out_frames = loaded
            .dsp_process_i16(&mut work, CHANNELS, SAMPLE_RATE)
            .expect("process must be driven for an initialized DSP plugin");
        // The bridge only returns counts in the S09 range half..double of the input.
        assert!(
            (BLOCK_FRAMES / 2..=BLOCK_FRAMES * 2).contains(&out_frames),
            "block {b}: output frame count {out_frames} outside half..double"
        );
        assert_eq!(
            work.len(),
            out_frames * CHANNELS,
            "output buffer must match the returned count"
        );
        frames_in += BLOCK_FRAMES;
        frames_out += out_frames;
        let n = original.len().min(work.len());
        compared += n;
        for (a, o) in original[..n].iter().zip(&work[..n]) {
            let d = (i32::from(*a) - i32::from(*o)).abs();
            if d != 0 {
                differing += 1;
                max_abs_diff = max_abs_diff.max(d);
            }
        }
        all_in.extend_from_slice(&original);
        all_out.extend_from_slice(&work);
    }
    let rms_in = rms(&all_in);
    let rms_out = rms(&all_out);
    println!(
        "DSP_MEASUREMENT frames_in={frames_in} frames_out={frames_out} compared={compared} \
         differing={differing} max_abs_diff={max_abs_diff} rms_in={rms_in:.1} rms_out={rms_out:.1}"
    );
    let changed = differing > 0 || frames_out != frames_in;
    assert!(
        changed,
        "FINDING: the plugin ran but returned the audio bit-identical (frames {frames_in}->{frames_out}, \
         0 of {compared} samples differ). In its default, unconfigured state this plugin is inert \
         through the bridge; the bridge drive is proven, an audible effect is not. See card P8 section 5."
    );

    // 7. Edge: an empty buffer is a passthrough, not a call into the plugin.
    let mut empty: Vec<i16> = Vec::new();
    assert_eq!(
        loaded.dsp_process_i16(&mut empty, CHANNELS, SAMPLE_RATE),
        Some(0)
    );
    assert!(empty.is_empty());

    // 8. Clean shutdown: quit runs and deactivates the module.
    loaded.quit();
    assert!(!loaded.is_initialized());
    assert!(
        loaded.dsp_run_info().is_none(),
        "no DSP module is active after the quit, so dsp_is_active reads 0"
    );
    assert_eq!(
        loaded.dsp_process_i16(&mut signal_block(0, 16), CHANNELS, SAMPLE_RATE),
        None,
        "process must not be driven after quit"
    );
    window.destroy();
}
