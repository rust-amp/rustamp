//! VM-only acceptance test for the **input → output hand-over**: the
//! `in_` half of S09 §3.5 step 2 and §3.7, proven with two real 32-bit
//! plug-in DLLs.
//!
//! It loads two clean-room fixtures through the production loader and the
//! production registry:
//!
//! * the **output** fixture (`crates/rustamp-plughost/examples/
//!   out_rustamp_fixture.rs`), the `out_` half of S09 §4;
//! * the **hand-over input** fixture
//!   (`crates/rustamp-plughost/examples/in_handoff_rustamp_fixture.rs`),
//!   the `in_` half of S09 §3 — the one that sets **flag bit 0** and drives
//!   the output itself.
//!
//! and proves, in one real i686 process:
//!
//! 1. both approved images are loaded from **separate DLLs** through the OS
//!    loader, and the modules report the kinds the exports name
//!    (S09 §2, §3.1, §4.1);
//! 2. the host writes the selected output into the input module's
//!    `out_module` member (S09 §3.1 offset 0x94) **before** `play`, and
//!    applies the player's volume and pan to the output (S09 §3.5 step 2);
//! 3. the **input plugin drives the output itself**: `open`, `can_write`,
//!    `write`, `is_playing` and `close` reach the output plug-in's own code
//!    (S09 §3.5 steps 3–5, §3.7, §4.2 step 3) — the host calls none of
//!    them;
//! 4. the input plug-in **posts** the end-of-track notification `0x402` to
//!    the host's main window with both parameters zero, and the host records
//!    it and reports it to its caller (S09 §3.6).
//!
//! The evidence is plugin-side: both fixtures write their own trace files
//! from inside their DLL, so the host and the test runner cannot forge the
//! calls they show.
//!
//! What this is **not**: a claim about third-party plug-ins, about audio, about
//! a real output device, or about the output selection in the preferences
//! (S12 owns `config_outname`). P3 §7 lists the surface test, `PluginLoaded`
//! and a green build as false passes; this test proves none of them.
//!
//! It refuses loudly (never skips) when it is not in the test VM, not on
//! i686, or when a fixture DLL is missing.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

#![cfg(windows)]

#[path = "common/in_handoff_protocol.rs"]
mod in_handoff_protocol;
#[path = "common/out_protocol.rs"]
mod out_protocol;

use in_handoff_protocol as inp;
use out_protocol as outp;

use std::path::{Path, PathBuf};

use rustamp_ipc::Message;
use rustamp_plughost::approval::ApprovalStore;
use rustamp_plughost::host_window::{HostWindow, require_test_vm};
use rustamp_plughost::ipc::IpcSurface;
use rustamp_plughost::message_loop;
use rustamp_plughost::registry::PluginRegistry;
use rustamp_plughost::{END_OF_TRACK_MESSAGE, take_end_of_track};
use rustamp_plugin_abi::PluginKind;

/// The file name of the hand-over input fixture.
const IN_FIXTURE_FILE_NAME: &str = "in_handoff_rustamp_fixture.dll";

/// The file name of the output fixture.
const OUT_FIXTURE_FILE_NAME: &str = "out_rustamp_fixture.dll";

/// The environment variable that overrides the hand-over input fixture's
/// path.
const IN_FIXTURE_ENV: &str = "RUSTAMP_IN_HANDOFF_FIXTURE";

/// The environment variable that overrides the output fixture's path.
const OUT_FIXTURE_ENV: &str = "RUSTAMP_OUT_FIXTURE";

/// The path the host hands to the plugin's `play` (S09 §3.1 offset 0x38).
///
/// The fixture opens no file, so the path only has to be one the host can
/// hand over unchanged. It is written in ASCII, so its ANSI form and its
/// byte form are the same.
const PLAY_PATH: &str = r"C:\rustamp\code\Rustamp\media\fixture.mp3";

/// The volume the host applies to the output before `play` (S09 §3.5
/// step 2).
const VOLUME: u8 = 200;

/// The balance the host applies to the output before `play` (S09 §3.5
/// step 2).
const PAN: i8 = -10;

/// The hand-over input fixture's own trace file, in the process temp
/// directory.
fn in_trace_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "{}{}.{}",
        inp::TRACE_FILE_PREFIX,
        std::process::id(),
        inp::TRACE_FILE_EXTENSION
    ))
}

/// The output fixture's own trace file, in the process temp directory.
fn out_trace_path() -> PathBuf {
    std::env::temp_dir().join(format!(
        "{}{}.{}",
        outp::TRACE_FILE_PREFIX,
        std::process::id(),
        outp::TRACE_FILE_EXTENSION
    ))
}

/// Find a fixture DLL, or refuse loudly.
///
/// The search starts at the test executable's own directory, because the
/// runner stages the fixtures beside the test binaries. A missing fixture
/// must never read as a pass or a quiet skip.
fn find_fixture(file_name: &str, env_var: &str) -> PathBuf {
    let mut checked = Vec::new();

    // 1. Explicit override by the runner.
    if let Some(value) = std::env::var_os(env_var) {
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
        let mut candidates = vec![dir.join(file_name)];
        if let Some(parent) = dir.parent() {
            candidates.push(parent.join(file_name));
            if let Some(root) = parent.parent() {
                candidates.push(
                    root.join("i686-pc-windows-msvc")
                        .join("debug")
                        .join("examples")
                        .join(file_name),
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
            .join(file_name),
        PathBuf::from(r"C:\rustamp\code\Rustamp\target\debug\deps").join(file_name),
    ] {
        if candidate.is_file() {
            return candidate;
        }
        checked.push(candidate);
    }

    panic!(
        "REFUSING LOUDLY: missing the fixture {file_name}. Checked {checked:?}. Build it with \
         `cargo build -p rustamp-plughost --example {} --target i686-pc-windows-msvc` and stage \
         it beside the test binaries. This test refuses rather than passing vacuously.",
        file_name.trim_end_matches(".dll")
    );
}

/// Load one approved fixture into the registry and return its id.
///
/// The registry's own load request is the production path: it preflights the
/// image, maps the approved bytes, resolves the kind's export and calls the
/// plug-in's `init` (S09 §2, §3.5 step 2).
fn load(
    registry: &mut PluginRegistry,
    dll: &Path,
    kind: PluginKind,
    window: &HostWindow,
    expected_description: &[u8],
) -> u32 {
    let path = dll.to_str().expect("the fixture path must be valid UTF-8");
    // SAFETY: running inside the gated test VM on i686 with the owner's own
    // fixtures, whose bytes were approved by the caller.
    let reply = unsafe { registry.handle_load_request(path, kind, window) };
    match reply {
        Message::PluginLoaded {
            plugin_id,
            name,
            kind: loaded,
        } => {
            println!("PLUGIN_LOADED kind={loaded:?} name={name:?} id={plugin_id}");
            assert_eq!(
                loaded, kind,
                "the loaded plug-in must be the requested kind"
            );
            assert_eq!(
                name,
                description_text(expected_description),
                "the plug-in's own description must be readable from plug-in memory"
            );
            plugin_id
        }
        Message::PluginError { message, .. } => {
            panic!("the {kind:?} fixture must load into the host: {message}")
        }
        other => panic!("unexpected answer to the load request: {other:?}"),
    }
}

/// The text of a NUL-terminated description constant of a shared protocol.
fn description_text(raw: &[u8]) -> String {
    let end = raw.iter().position(|byte| *byte == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

/// How many trace lines start with `prefix`.
fn count(trace: &str, prefix: &str) -> usize {
    trace
        .lines()
        .filter(|line| line.starts_with(prefix))
        .count()
}

/// The index of the first trace line that starts with `prefix`.
fn first_index(trace: &str, prefix: &str) -> Option<usize> {
    trace.lines().position(|line| line.starts_with(prefix))
}

#[test]
#[ignore = "VM-only: maps two real 32-bit plugin DLLs into the host; run in test VM"]
fn a_real_input_plugin_hands_its_audio_to_the_selected_output_plugin() {
    // 1. Gate before anything is mapped or created.
    require_test_vm();
    if !cfg!(target_arch = "x86") {
        panic!(
            "REFUSING LOUDLY: loading a 32-bit plugin DLL requires an i686 target; \
             cannot map it into a 64-bit process"
        );
    }

    // 2. The two fixtures, pinned by their own bytes.
    let in_dll = find_fixture(IN_FIXTURE_FILE_NAME, IN_FIXTURE_ENV);
    let out_dll = find_fixture(OUT_FIXTURE_FILE_NAME, OUT_FIXTURE_ENV);
    let in_bytes = std::fs::read(&in_dll).expect("the input fixture must be readable");
    let out_bytes = std::fs::read(&out_dll).expect("the output fixture must be readable");
    println!(
        "FIXTURE {} size={} sha256={}",
        in_dll.display(),
        in_bytes.len(),
        rustamp_plughost::approval::Sha256::digest_hex(&in_bytes)
    );
    println!(
        "FIXTURE {} size={} sha256={}",
        out_dll.display(),
        out_bytes.len(),
        rustamp_plughost::approval::Sha256::digest_hex(&out_bytes)
    );

    // 3. The host's real main window (class `Winamp v1.x`).
    let surface = IpcSurface::new();
    let window = HostWindow::create(surface).expect("HostWindow creation failed in test VM");
    let host_hwnd = window.hwnd_raw();
    assert_ne!(host_hwnd, 0, "the host window must exist");

    // Start both traces from scratch: a stale file must never be read as this
    // run's evidence.
    let _ = std::fs::remove_file(in_trace_path());
    let _ = std::fs::remove_file(out_trace_path());
    // Drop whatever an earlier run in this process left behind.
    let _ = take_end_of_track();

    // 4. Approve both images and load them through the registry.
    let mut store = ApprovalStore::new();
    store
        .approve_bytes(IN_FIXTURE_FILE_NAME, &in_bytes)
        .expect("the input fixture must pass the passive preflight");
    store
        .approve_bytes(OUT_FIXTURE_FILE_NAME, &out_bytes)
        .expect("the output fixture must pass the passive preflight");
    let mut registry = PluginRegistry::new(store);
    // The output is loaded first: the registry quits the plug-ins in reverse
    // load order (S09 §3.5 line 166).
    let out_id = load(
        &mut registry,
        &out_dll,
        PluginKind::Output,
        &window,
        outp::FIXTURE_DESCRIPTION,
    );
    let in_id = load(
        &mut registry,
        &in_dll,
        PluginKind::Input,
        &window,
        inp::FIXTURE_DESCRIPTION,
    );

    // 5. The output must be selected before the play (S09 §3.5 step 2).
    assert_eq!(registry.selected_output(), None);
    registry
        .select_output(out_id)
        .expect("the output plug-in must be selectable");
    assert_eq!(registry.selected_output(), Some(out_id));

    // 6. Play: the host hands the output over and the input plug-in drives
    //    it (S09 §3.5 steps 2–3, §3.7).
    registry
        .play_input(in_id, Path::new(PLAY_PATH), VOLUME, PAN)
        .expect("the hand-over and the play must succeed");

    // 7. The host wrote the output into the input module (S09 §3.1
    //    offset 0x94) and left the output's own id alone (S09 §4.1, U-4).
    let in_info = registry
        .input_info(in_id)
        .expect("a loaded input plug-in must expose its module info");
    let out_info = registry
        .output_info(out_id)
        .expect("a loaded output plug-in must expose its module info");
    assert!(
        in_info.has_out_module,
        "the host must have written the selected output into the input module (S09 §3.1 \
         offset 0x94): {in_info:?}"
    );
    assert_eq!(in_info.main_window, host_hwnd);
    assert_eq!(out_info.main_window, host_hwnd);
    assert_eq!(out_info.id, outp::FIXTURE_MODULE_ID);

    // 8. The input plug-in's own evidence: it drove the output itself.
    let in_trace = std::fs::read_to_string(in_trace_path())
        .expect("the input fixture must have written its own trace");
    println!("INPUT_FIXTURE_TRACE\n{in_trace}");
    assert!(
        in_trace.contains(&inp::init_line(host_hwnd, in_info.dll_instance)),
        "the plugin must have seen the host's main window and its own instance:\n{in_trace}"
    );
    assert!(
        in_trace.contains(&inp::parent_class_line(true)),
        "the parent window's class must be Winamp v1.x (S09 §1):\n{in_trace}"
    );
    assert!(
        in_trace.contains(&inp::host_slots_line(true, true, true, false, false)),
        "at init, set_info must still be empty (S09 §3.5 line 154) and out_module too, because \
         the hand-over happens before play (S09 §3.5 step 2):\n{in_trace}"
    );
    assert!(
        in_trace.contains(&inp::host_slots_line(true, true, true, true, true)),
        "at play, the host must have filled set_info (S09 §3.5 line 155) and written the output \
         into out_module (S09 §3.5 step 2):\n{in_trace}"
    );
    assert!(
        in_trace.contains(&inp::play_line(PLAY_PATH)),
        "the plugin must have received the path the host was given:\n{in_trace}"
    );

    // The output calls, one for one (S09 §3.5 steps 3–5, §3.7).
    assert_eq!(
        count(&in_trace, inp::EVENT_OUT_OPEN),
        1,
        "the plugin must have opened the output exactly once:\n{in_trace}"
    );
    assert!(
        in_trace.contains(&inp::out_open_line(
            inp::FIXTURE_SAMPLE_RATE,
            inp::FIXTURE_CHANNELS,
            inp::FIXTURE_BITS,
            inp::FIXTURE_BUFFER_MS,
            inp::FIXTURE_PREBUFFER_MS
        )),
        "the plugin must have opened the output with the stream's format:\n{in_trace}"
    );
    assert_eq!(
        count(&in_trace, inp::EVENT_OUT_CAN_WRITE),
        inp::FIXTURE_FRAMES,
        "the plugin must have asked the output for room once per frame:\n{in_trace}"
    );
    assert_eq!(
        count(&in_trace, inp::EVENT_OUT_WRITE),
        inp::FIXTURE_FRAMES,
        "the plugin must have written one frame per round:\n{in_trace}"
    );
    assert!(
        in_trace.contains(&inp::out_write_line(inp::FIXTURE_FRAME_BYTES as i32)),
        "the plugin must have written its silent frame:\n{in_trace}"
    );
    assert_eq!(
        count(&in_trace, inp::EVENT_OUT_IS_PLAYING),
        1,
        "the plugin must have asked the output whether it still plays (S09 §3.5 step 5):\n{in_trace}"
    );
    assert_eq!(
        count(&in_trace, inp::EVENT_OUT_TIME),
        1,
        "the plugin must have taken the time from the output (S09 §3.7):\n{in_trace}"
    );
    assert_eq!(
        count(&in_trace, inp::EVENT_OUT_CLOSE),
        1,
        "the plugin must have closed the output exactly once:\n{in_trace}"
    );

    // 9. The end-of-track notification (S09 §3.6): the plugin posts `0x402`
    //    to the main window with both parameters zero, and the host records
    //    it and reports it to its caller.
    assert!(
        in_trace.contains(&inp::end_of_track_line(true)),
        "the plugin must have posted the end-of-track notification:\n{in_trace}"
    );
    let mut end_of_track = 0;
    for _ in 0..64 {
        message_loop::pump_bounded(&window, 16);
        end_of_track = take_end_of_track();
        if end_of_track != 0 {
            break;
        }
    }
    assert_eq!(
        end_of_track, 1,
        "the end-of-track notification must reach the host's caller exactly once"
    );
    let messages = window.recorded_messages();
    assert!(
        messages.contains(&(END_OF_TRACK_MESSAGE, 0_usize, 0_isize)),
        "the host window must have received 0x402 with both parameters zero (S09 §3.6): {messages:?}"
    );

    // 10. The output plug-in's own evidence: the calls really reached its
    //     code, and the host's half of the hand-over came first.
    let out_trace = std::fs::read_to_string(out_trace_path())
        .expect("the output fixture must have written its own trace");
    println!("OUTPUT_FIXTURE_TRACE\n{out_trace}");
    assert!(
        out_trace.contains(&outp::init_line(
            host_hwnd,
            out_info.dll_instance,
            outp::FIXTURE_MODULE_ID
        )),
        "the output plugin must have seen the host's main window and its own instance:\n{out_trace}"
    );
    assert!(
        out_trace.contains(&outp::parent_class_line(true)),
        "the parent window's class must be Winamp v1.x (S09 §1):\n{out_trace}"
    );
    assert!(
        out_trace.contains(&format!("{}{VOLUME}", outp::EVENT_SET_VOLUME)),
        "the host must have applied the volume to the output before play (S09 §3.5 step 2):\n{out_trace}"
    );
    assert!(
        out_trace.contains(&format!("{}{PAN}", outp::EVENT_SET_PAN)),
        "the host must have applied the balance to the output before play (S09 §3.5 step 2):\n{out_trace}"
    );
    assert_eq!(
        count(&out_trace, outp::EVENT_OPEN),
        1,
        "the output must have been opened exactly once:\n{out_trace}"
    );
    assert_eq!(
        count(&out_trace, outp::EVENT_CAN_WRITE),
        inp::FIXTURE_FRAMES,
        "the output must have been asked for room once per frame:\n{out_trace}"
    );
    assert_eq!(
        count(&out_trace, outp::EVENT_WRITE),
        inp::FIXTURE_FRAMES,
        "the output must have received one frame per round:\n{out_trace}"
    );
    assert!(
        out_trace.contains(&format!(
            "{}len={}",
            outp::EVENT_WRITE,
            inp::FIXTURE_FRAME_BYTES
        )),
        "the output must have received the plugin's silent frame:\n{out_trace}"
    );
    assert_eq!(
        count(&out_trace, outp::EVENT_IS_PLAYING),
        1,
        "the output must have been asked whether it still plays (S09 §3.5 step 5):\n{out_trace}"
    );
    assert_eq!(
        count(&out_trace, outp::EVENT_CLOSE),
        1,
        "the output must have been closed exactly once:\n{out_trace}"
    );

    // The host's levels come first, the plugin's playback calls after: the
    // host applies the levels before `play` (S09 §3.5 step 2) and drives no
    // playback callback itself (S09 §4.2 step 3).
    let levels = first_index(&out_trace, outp::EVENT_SET_VOLUME)
        .unwrap_or_else(|| panic!("the output must have seen the host's levels:\n{out_trace}"));
    let opened = first_index(&out_trace, outp::EVENT_OPEN)
        .unwrap_or_else(|| panic!("the output must have been opened:\n{out_trace}"));
    assert!(
        levels < opened,
        "the host applies the levels before the plugin opens the output:\n{out_trace}"
    );

    // The player's Configure request reaches the output module's `config`
    // with the parent window (S09 §4.1 offset 0x14), and the host has no
    // route to the `about` box at all.
    registry
        .configure_plugin(out_id, PluginKind::Output)
        .expect("the output module's config must be routed");
    let out_trace = std::fs::read_to_string(out_trace_path())
        .expect("the output fixture must have written its own trace");
    assert_eq!(
        count(&out_trace, &outp::config_line(host_hwnd)),
        1,
        "the config callback must reach the output plugin exactly once, with the parent \
         window:\n{out_trace}"
    );
    assert!(
        !out_trace
            .lines()
            .any(|line| line.starts_with(outp::EVENT_ABOUT)),
        "the host must not open the output plugin's about box:\n{out_trace}"
    );

    // 11. The calls came in the order the ABI fixes (S09 §3.5 steps 3–5).
    let order = [
        inp::EVENT_INIT,
        inp::EVENT_PLAY,
        inp::EVENT_OUT_OPEN,
        inp::EVENT_OUT_CAN_WRITE,
        inp::EVENT_OUT_WRITE,
        inp::EVENT_OUT_IS_PLAYING,
        inp::EVENT_OUT_TIME,
        inp::EVENT_OUT_CLOSE,
        inp::EVENT_END_OF_TRACK,
    ];
    let mut previous = 0;
    for prefix in &order {
        let index = first_index(&in_trace, prefix)
            .unwrap_or_else(|| panic!("the trace must carry a '{prefix}' line:\n{in_trace}"));
        assert!(
            index >= previous,
            "the '{prefix}' line must come after the previous one:\n{in_trace}"
        );
        previous = index;
    }

    // 12. The player's Configure request reaches the input module's `config`
    //     with the parent window (S09 §3.1 offset 0x1C). This fixture's
    //     `config` opens no window, so the route is proven from the plugin's
    //     own trace.
    registry
        .configure_plugin(in_id, PluginKind::Input)
        .expect("the input module's config must be routed");
    let in_trace = std::fs::read_to_string(in_trace_path())
        .expect("the input fixture must have written its own trace");
    assert_eq!(
        count(&in_trace, &inp::config_line(host_hwnd)),
        1,
        "the config callback must reach the plugin exactly once, with the parent \
         window:\n{in_trace}"
    );

    // 13. Quit both plug-ins in reverse load order (S09 §3.5 line 166).
    assert_eq!(registry.quit_all(), vec![in_id, out_id]);
    let in_trace = std::fs::read_to_string(in_trace_path())
        .expect("the input fixture must have written its own trace");
    let out_trace = std::fs::read_to_string(out_trace_path())
        .expect("the output fixture must have written its own trace");
    assert!(
        in_trace.contains(inp::EVENT_QUIT),
        "quit must reach the input plugin:\n{in_trace}"
    );
    assert!(
        out_trace.contains(outp::QUIT_LINE),
        "quit must reach the output plugin:\n{out_trace}"
    );

    window.destroy();
}
