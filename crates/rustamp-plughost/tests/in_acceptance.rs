//! VM-only acceptance test for the **input** plugin kind: the `in_` half of
//! the classic-plugin ABI of `specs/S09-plugin-abi.md` §3.
//!
//! It loads the clean-room `in_` fixture
//! (`crates/rustamp-plughost/examples/in_rustamp_fixture.rs`) through the
//! production loader and proves, in one real i686 process:
//!
//! 1. the approved bytes are loaded from a **separate DLL** through the OS
//!    loader, the export `winampGetInModule2` resolves, and the module
//!    reports version `0x100` with a readable description (S09 §2, §3.1,
//!    §3.2);
//! 2. the host filled the main window, the DLL instance and the analyzer,
//!    visualization and DSP callbacks **before** `init` ran, and the plugin
//!    saw the `Winamp v1.x` class (S09 §1, §3.1, §3.5 step 2);
//! 3. `set_info` was still empty during `init` and filled after it returned
//!    (S09 §3.5 lines 154–155), and `out_module` stayed empty because no
//!    output plugin is selected (S09 §3.1 offset 0x94);
//! 4. the host hands the plugin the path it was given, byte for byte
//!    (S09 §3.1 offset 0x38), and the plugin's own `play` answer reaches the
//!    host;
//! 5. the plugin announces its stream through the host's `analyzer_init`,
//!    `vis_set_format` and `set_info` callbacks (S09 §3.5 step 3);
//! 6. the plugin's own state answers reach the host: the track length, the
//!    pause state and a position that runs with the plugin's own clock and
//!    stands still while the plugin is paused (S09 §3.1 offsets 0x44, 0x4C,
//!    0x50, S09 §3.7);
//! 7. the player's Configure request reaches the module's `config` exactly
//!    once, with the parent window (S09 §3.1 offset 0x1C);
//! 8. the host calls **no** callback it has no path for — the volume, the
//!    balance, the equalizer, the seek, the file-info and the module
//!    selection calls stay untouched;
//! 9. `quit` reaches the plugin exactly once and the DLL stays mapped
//!    (S09 §3.5 line 166, §1 line 46).
//!
//! The plugin's own trace file — written from inside the DLL — is checked for
//! the same events, so the evidence does not come from the runner alone.
//!
//! What this is **not**: a claim about third-party input plugins, about
//! audio, about the output plugin an input plugin normally drives, about the
//! module selection between several input plugins or about the input plugin
//! list in the preferences. P3 §7 lists the surface test, `PluginLoaded` and
//! a green build as false passes; this test proves none of them.
//!
//! It refuses loudly (never skips) when it is not in the test VM, not on
//! i686, or when the fixture DLL is missing.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

#![cfg(windows)]

#[path = "common/in_protocol.rs"]
mod in_protocol;

use in_protocol::*;

use std::path::{Path, PathBuf};
use std::time::Duration;

use rustamp_plughost::approval::ApprovalStore;
use rustamp_plughost::host_window::{HostWindow, require_test_vm};
use rustamp_plughost::ipc::IpcSurface;
use rustamp_plughost::loader::load_plugin_with_store;
use rustamp_plugin_abi::PluginKind;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;

/// The file name of the fixture DLL.
const FIXTURE_FILE_NAME: &str = "in_rustamp_fixture.dll";

/// The path the host hands to the plugin's `play` (S09 §3.1 offset 0x38).
///
/// The fixture opens no file, so the path only has to be one the host can
/// hand over unchanged. It is written in ASCII, so its ANSI form and its
/// byte form are the same.
const PLAY_PATH: &str = r"C:\rustamp\code\Rustamp\media\fixture.mp3";

/// How long the test waits for the plugin's own clock to run.
const CLOCK_WAIT: Duration = Duration::from_millis(60);

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
    if let Some(value) = std::env::var_os("RUSTAMP_IN_FIXTURE") {
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
        "REFUSING LOUDLY: missing the input fixture {FIXTURE_FILE_NAME}. Checked {checked:?}. \
         Build it with `cargo build -p rustamp-plughost --example in_rustamp_fixture --target \
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

/// Whether any trace line starts with `prefix`.
fn has_line(trace: &str, prefix: &str) -> bool {
    trace.lines().any(|line| line.starts_with(prefix))
}

/// The index of the first trace line that starts with `prefix`.
fn first_index(trace: &str, prefix: &str) -> Option<usize> {
    trace.lines().position(|line| line.starts_with(prefix))
}

/// The text of a NUL-terminated description constant of the shared protocol.
fn description_text(raw: &[u8]) -> String {
    let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

#[test]
#[ignore = "VM-only: maps a real 32-bit input plugin DLL into the host; run in test VM"]
fn a_real_input_plugin_is_loaded_filled_plays_and_quits_cleanly() {
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
        .expect("the fixture must pass the passive preflight as an input plugin");
    // SAFETY: running inside the gated test VM on i686 with the owner's own
    // fixture, whose bytes were approved above.
    let mut loaded = unsafe {
        load_plugin_with_store(&dll, &store, &window).expect("the fixture must load into the host")
    };

    // 5. The module the host got from the DLL.
    assert_eq!(
        loaded.kind(),
        PluginKind::Input,
        "winampGetInModule2 must select the input kind (S09 §2)"
    );
    assert_eq!(
        loaded.description(),
        description_text(FIXTURE_DESCRIPTION),
        "the plugin's own description must be readable from plugin memory"
    );
    println!(
        "PLUGIN_LOADED kind={:?} name={:?}",
        loaded.kind(),
        loaded.description()
    );
    assert!(loaded.is_initialized(), "plugin must be marked initialized");

    // 6. The host-filled members and the slots the plugin supplied.
    let info = loaded
        .input_info()
        .expect("a loaded input plugin must expose its module info");
    println!("PLUGIN_INFO {info:?}");
    assert!(
        info.has_init && info.has_quit && info.has_play && info.has_stop,
        "the fixture must provide the lifecycle slots: {info:?}"
    );
    assert!(info.has_config, "the fixture must provide a config slot");
    assert!(info.init_called, "init must have run");
    assert_eq!(
        info.main_window, host_hwnd,
        "the host must fill the main window before init (S09 §3.1 offset 0x08)"
    );
    assert_eq!(
        info.dll_instance,
        loaded.hinstance() as usize,
        "the host must fill the plugin's own module handle before init (S09 §3.1 offset 0x0C)"
    );
    assert!(
        info.has_analyzer && info.has_vis && info.has_dsp,
        "the host must fill the analyzer, visualization and DSP callbacks: {info:?}"
    );
    assert!(
        info.has_set_info,
        "the host must fill set_info after init returned (S09 §3.5 line 155): {info:?}"
    );
    assert!(
        !info.has_out_module,
        "no output plugin is selected, so out_module must stay empty (S09 §3.1 offset 0x94): \
         {info:?}"
    );

    // 7. The plugin's own evidence, written from inside the DLL.
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    println!("FIXTURE_TRACE\n{trace}");
    assert!(
        trace.contains(&init_line(host_hwnd, loaded.hinstance() as usize)),
        "the plugin must have seen the host's main window and its own instance:\n{trace}"
    );
    assert!(
        trace.contains(&parent_class_line(true)),
        "the parent window's class must be Winamp v1.x (S09 §1):\n{trace}"
    );
    assert!(
        trace.contains(&host_slots_line(true, true, true, false, false)),
        "the host callbacks must have been filled before init, and set_info must still have \
         been empty (S09 §3.5 line 154):\n{trace}"
    );

    // 8. Playback: the host hands the plugin its path and reports the
    //    plugin's own answer (S09 §3.5 step 3, S09 §3.1 offset 0x38).
    loaded
        .input_play(Path::new(PLAY_PATH))
        .expect("the plugin must accept the path the host handed it");
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    assert!(
        trace.contains(&play_line(PLAY_PATH)),
        "the plugin must have received the path the host was given:\n{trace}"
    );
    assert!(
        trace.contains(&host_slots_line(true, true, true, true, false)),
        "the host must have filled set_info after init returned (S09 §3.5 line 155):\n{trace}"
    );
    assert!(
        trace.contains(&analyzer_init_line(FIXTURE_LATENCY_MS, FIXTURE_SAMPLE_RATE)),
        "the plugin must have announced the stream to the host's analyzer_init:\n{trace}"
    );
    assert!(
        trace.contains(&vis_set_format_line(FIXTURE_SAMPLE_RATE, FIXTURE_CHANNELS)),
        "the plugin must have announced the stream format to the host's vis_set_format:\n{trace}"
    );
    assert!(
        trace.contains(&set_info_line(
            FIXTURE_BITRATE_KBPS,
            FIXTURE_SAMPLE_RATE_KHZ,
            FIXTURE_CHANNELS,
            0
        )),
        "the plugin must have reported the stream properties to the host's set_info:\n{trace}"
    );

    // 9. The plugin's own answers reach the host.
    assert_eq!(
        loaded
            .input_length_ms()
            .expect("the plugin must report a length"),
        FIXTURE_LENGTH_MS,
        "the host must report the plugin's own track length (S09 §3.1 offset 0x4C)"
    );
    assert!(
        !loaded
            .input_is_paused()
            .expect("the plugin must report its pause state"),
        "the plugin is not paused right after play"
    );
    let start = loaded
        .input_position_ms()
        .expect("the plugin must report a position");
    assert!(start >= 0, "the position must not be negative: {start}");
    std::thread::sleep(CLOCK_WAIT);
    let moved = loaded
        .input_position_ms()
        .expect("the plugin must report a position");
    assert!(
        moved >= start + 30,
        "the plugin's own clock must run while it plays: {start} -> {moved}"
    );

    // 10. Pause: the plugin's clock stands still and its answer says so.
    loaded
        .input_pause()
        .expect("the host must be able to pause the plugin");
    assert!(
        loaded
            .input_is_paused()
            .expect("the plugin must report its pause state"),
        "the plugin must report that it is paused"
    );
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    assert!(
        trace.contains(&paused_line(true)),
        "the pause callback must reach the plugin:\n{trace}"
    );
    let frozen = loaded
        .input_position_ms()
        .expect("the plugin must report a position");
    std::thread::sleep(CLOCK_WAIT);
    let still = loaded
        .input_position_ms()
        .expect("the plugin must report a position");
    assert_eq!(
        still, frozen,
        "the plugin's own clock must stand still while it is paused"
    );

    // 11. Resume: the clock runs again.
    loaded
        .input_unpause()
        .expect("the host must be able to resume the plugin");
    assert!(
        !loaded
            .input_is_paused()
            .expect("the plugin must report its pause state"),
        "the plugin must report that it runs again"
    );
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    assert!(
        trace.contains(&paused_line(false)),
        "the resume callback must reach the plugin:\n{trace}"
    );
    std::thread::sleep(CLOCK_WAIT);
    let resumed = loaded
        .input_position_ms()
        .expect("the plugin must report a position");
    assert!(
        resumed > still,
        "the plugin's own clock must run again after the resume: {still} -> {resumed}"
    );

    // 12. Stop: the plugin tears its stream down (S09 §3.5 step 6).
    loaded
        .input_stop()
        .expect("the host must be able to stop the plugin");
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    assert!(
        trace.contains(EVENT_STOP),
        "the stop callback must reach the plugin:\n{trace}"
    );
    assert!(
        trace.contains(EVENT_ANALYZER_DEINIT),
        "the plugin must have released the host's analyzer (S09 §3.5 step 6):\n{trace}"
    );

    // 13. The player's Configure request reaches the module's `config` with
    //     the parent window (S09 §3.1 offset 0x1C). This fixture's `config`
    //     opens no window, so the route is proven from the plugin's own
    //     trace.
    loaded
        .config()
        .expect("the input module's config must be routed");
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    assert_eq!(
        occurrences(&trace, &config_line(host_hwnd)),
        1,
        "the config callback must reach the plugin exactly once, with the parent \
         window:\n{trace}"
    );

    // 14. The host calls no callback it has no path for: the volume, the
    //     balance, the equalizer, the seek, the file info and the module
    //     selection calls stay untouched.
    for forbidden in [
        EVENT_ABOUT,
        EVENT_SET_VOLUME,
        EVENT_SET_PAN,
        EVENT_EQ_SET,
        EVENT_SEEK,
        EVENT_FILE_INFO,
        EVENT_INFO_BOX,
        EVENT_IS_OUR_FILE,
    ] {
        assert!(
            !has_line(&trace, forbidden),
            "the host must not call '{forbidden}':\n{trace}"
        );
    }

    // 15. Quit (S09 §3.5 line 166): the plugin is told once, and the DLL
    //     stays mapped.
    loaded.quit();
    assert!(!loaded.is_initialized());
    let trace =
        std::fs::read_to_string(trace_path()).expect("the fixture must have written its own trace");
    println!("FIXTURE_TRACE\n{trace}");
    assert_eq!(
        occurrences(&trace, EVENT_QUIT),
        1,
        "quit must reach the plugin exactly once:\n{trace}"
    );
    // The input plugin's DLL is not freed at shutdown (S09 §1 line 46).
    let mapped = mapped_path(loaded.hinstance());
    println!("MAPPED {mapped:?}");

    // 16. The calls came in the order the ABI fixes (S09 §3.5).
    let order = [
        EVENT_INIT.to_string(),
        EVENT_HOST_SLOTS.to_string(),
        EVENT_PLAY.to_string(),
        paused_line(true),
        paused_line(false),
        EVENT_STOP.to_string(),
        EVENT_QUIT.to_string(),
    ];
    let mut previous = 0;
    for prefix in &order {
        let index = first_index(&trace, prefix.as_str())
            .unwrap_or_else(|| panic!("the trace must carry a '{prefix}' line:\n{trace}"));
        assert!(
            index >= previous,
            "the '{prefix}' line must come after the previous one:\n{trace}"
        );
        previous = index;
    }

    window.destroy();
}
