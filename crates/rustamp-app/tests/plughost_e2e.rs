// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! End to end across the plugin host chain: the app publishes, the host window answers.
//!
//! What was already covered without this file: the app publishes snapshots and
//! runs `Hello`/`Ping` liveness ([`rustamp_app::host`]), the supervisor spawns
//! and reaps the host ([`rustamp_app::supervise`]), and the host applies
//! snapshots to its answering surface (`rustamp-plughost/src/state_bridge.rs`).
//! What was never exercised together is the whole path: **app spawns the real
//! host, publishes a known snapshot over the real control channel, and the
//! host's real window answers plugin-shaped queries with what was published**.
//! That is this file's single test.
//!
//! The test asks the way a plugin does: `WM_WA_IPC` (`WM_USER`) straight at the
//! host window, IPC number in `lparam`, argument in `wparam`. It asserts the
//! version any plugin asks first (id 0), then the bridge fields the published
//! snapshot implies: playing state (104), position and length (105 modes 0/2),
//! volume and pan queries (122/123 with `-666`), list length/position
//! (124/125), stream info (126 modes 0/1/2) and shuffle/repeat (250/251).
//!
//! # VM-only rails
//!
//! * `#[ignore]` plus `#![cfg(windows)]`, exactly like the host's own window
//!   test: a real process is spawned and a real (hidden) window answers, so
//!   this must never run as part of the ordinary suite. Run it in the VM with
//!   `cargo test -p rustamp-app -- --ignored`.
//! * No console window can appear: the only spawn goes through
//!   [`rustamp_app::supervise::Supervisor`], which sets `CREATE_NO_WINDOW`;
//!   this file constructs no child (see the guard below) and the Win32 calls
//!   used here (`FindWindowExW`, `SendMessageTimeoutW`) open no windows. The
//!   host window itself is created hidden (no `WS_VISIBLE`).
//! * Cleanup: [`rustamp_app::supervise::Supervisor::stop`] kills the managed
//!   child (best-effort, without waiting) and detaches the link. Dropping the
//!   supervisor kills it too, so even a failed assertion leaves no host
//!   behind. Each query additionally caps at `QUERY_TIMEOUT_MS`, so a wedged
//!   host fails the test instead of hanging it.
//! * The `user32` surface is declared by hand (`#[link]` + `unsafe extern`)
//!   because the crate under test owns no Win32 dependency and this lane adds
//!   none: no `cargo add`, no new crate, no build change.
//!
//! # If the host binary is missing
//!
//! A `cargo test --workspace` build does not build the i686 host, so
//! [`rustamp_app::supervise::find_plugin_host`] may resolve nothing. That is a
//! loud failure here, never a silent pass: the test panics, lists every
//! candidate it checked, and names the runner step that would stage the host.

#![cfg(windows)]

use std::ffi::c_void;
use std::time::{Duration, Instant};

use rustamp_app::host::{HostStateLink, PollOutcome, PublishOutcome};
use rustamp_app::supervise::{Supervisor, find_plugin_host, plugin_host_candidates};
use rustamp_core::state::{PlayerState, StreamInfo, TransportState};

/// `WM_WA_IPC` (`WM_USER`): the message number classic plugins send the host
/// window (S09 section 3.6; `rustamp_plugin_abi::common::WM_WA_IPC`).
const WM_WA_IPC: u32 = 0x0400;
/// `SMTO_ABORTIFHUNG`: a hung host aborts the query instead of hanging us.
const SMTO_ABORTIFHUNG: u32 = 0x0002;
/// Per-query cap: the host answers in milliseconds; anything slower counts as
/// not-ready and the deadline below ends the test instead of hanging it.
const QUERY_TIMEOUT_MS: u32 = 2_000;
/// How long the published state gets to reach the window: cold VM startup,
/// process spawn, window creation and three thread hops (app writer, host
/// reader, host UI loop).
const E2E_DEADLINE: Duration = Duration::from_secs(15);
/// `IPC_GETVERSION` answer measured in S10 (RFC 0009): `0x2905`.
const EXPECTED_VERSION: u32 = 0x2905;
/// Window class of the host's main window (S09 sections 1 and 8;
/// `rustamp-plughost/src/host_window.rs::HOST_WINDOW_CLASS`).
const HOST_WINDOW_CLASS: &str = "Winamp v1.x";

// Raw `user32` surface, declared by hand so this test adds no dependency.
#[link(name = "user32")]
unsafe extern "system" {
    fn FindWindowExW(
        parent: *mut c_void,
        child_after: *mut c_void,
        class: *const u16,
        window: *const u16,
    ) -> *mut c_void;
    fn GetWindowThreadProcessId(hwnd: *mut c_void, pid: *mut u32) -> u32;
    #[allow(clippy::too_many_arguments)]
    fn SendMessageTimeoutW(
        hwnd: *mut c_void,
        msg: u32,
        wparam: usize,
        lparam: isize,
        flags: u32,
        timeout_ms: u32,
        result: *mut usize,
    ) -> isize;
}

fn wide_nul(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The host window of one process: the top-level `Winamp v1.x` window owned by
/// `pid`, or null. Matching by pid — read from the host's `Hello` — keeps this
/// exact when other ignored tests own same-class windows in the same session.
fn find_host_window(class: &[u16], pid: u32) -> *mut c_void {
    let mut after: *mut c_void = std::ptr::null_mut();
    loop {
        // SAFETY: null parent/title enumerate top-level windows; the class
        // buffer outlives the call; a null return ends the enumeration.
        let hwnd = unsafe {
            FindWindowExW(
                std::ptr::null_mut(),
                after,
                class.as_ptr(),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            return std::ptr::null_mut();
        }
        let mut owner = 0u32;
        // SAFETY: `owner` is a valid out-pointer; the handle came from the OS.
        unsafe {
            GetWindowThreadProcessId(hwnd, &mut owner);
        }
        if owner == pid {
            return hwnd;
        }
        after = hwnd;
    }
}

/// Ask the host window one `WM_WA_IPC` query the way a plugin does: the IPC
/// number rides in `lparam`, the argument in `wparam`. `None` on timeout.
fn ask(hwnd: *mut c_void, wparam: usize, ipc_id: u32) -> Option<u32> {
    let mut result: usize = 0;
    // SAFETY: plain integers cross the call; `result` is a valid out-pointer.
    let sent = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_WA_IPC,
            wparam,
            ipc_id as isize,
            SMTO_ABORTIFHUNG,
            QUERY_TIMEOUT_MS,
            &mut result,
        )
    };
    (sent != 0).then_some(result as u32)
}

fn check(label: &str, got: Option<u32>, want: u32) -> Option<String> {
    match got {
        Some(value) if value == want => None,
        other => Some(format!("{label}: got {other:?}, want {want}")),
    }
}

/// Every plugin-shaped query the test asks, in one pass. `None` when the whole
/// published snapshot is visible through the window; otherwise the first
/// mismatch (the wait loop retries until the deadline).
fn probe(hwnd: *mut c_void) -> Option<String> {
    const QUERY: usize = (-666isize) as usize;
    if let Some(mismatch) = check("version(0)", ask(hwnd, 0, 0), EXPECTED_VERSION) {
        return Some(mismatch);
    }
    if let Some(mismatch) = check("isplaying(104)", ask(hwnd, 0, 104), 1) {
        return Some(mismatch);
    }
    if let Some(mismatch) = check("position(105,0)", ask(hwnd, 0, 105), 12_345) {
        return Some(mismatch);
    }
    if let Some(mismatch) = check("length-ms(105,2)", ask(hwnd, 2, 105), 200_000) {
        return Some(mismatch);
    }
    if let Some(mismatch) = check("volume(122,-666)", ask(hwnd, QUERY, 122), 200) {
        return Some(mismatch);
    }
    if let Some(mismatch) = check("pan(123,-666)", ask(hwnd, QUERY, 123), (-10i32) as u32) {
        return Some(mismatch);
    }
    if let Some(mismatch) = check("listlen(124)", ask(hwnd, 0, 124), 60) {
        return Some(mismatch);
    }
    if let Some(mismatch) = check("listpos(125)", ask(hwnd, 0, 125), 7) {
        return Some(mismatch);
    }
    if let Some(mismatch) = check("samplerate(126,0)", ask(hwnd, 0, 126), 44_100) {
        return Some(mismatch);
    }
    if let Some(mismatch) = check("bitrate(126,1)", ask(hwnd, 1, 126), 128) {
        return Some(mismatch);
    }
    if let Some(mismatch) = check("channels(126,2)", ask(hwnd, 2, 126), 2) {
        return Some(mismatch);
    }
    if let Some(mismatch) = check("shuffle(250)", ask(hwnd, 0, 250), 1) {
        return Some(mismatch);
    }
    check("repeat(251)", ask(hwnd, 0, 251), 1)
}

/// The whole path at once: the supervisor spawns the real host, the link
/// publishes a known snapshot over the real control channel, and the host's
/// real window answers with what was published.
///
/// **VM-only and `#[ignore]`d by default**, like the host's own window test.
/// Each value below differs from the fresh-surface default, so a matching
/// pass proves the snapshot crossed, not the defaults.
#[test]
#[ignore = "spawns the real host and queries its window; VM-only, never part of the default suite"]
fn app_published_state_reaches_the_host_window() {
    // 1. Resolve exactly the way production does: packaged layout first, the
    // dev-tree heuristic after (see `rustamp-app/src/supervise.rs`). A plain
    // `cargo test --workspace` build stages no i686 host, so this may resolve
    // nothing — which fails loudly below, never passes vacuously.
    let Some(exe) = find_plugin_host() else {
        let checked = plugin_host_candidates()
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        panic!(
            "E2E BLOCKED, not passed: no rustamp-plughost32 binary resolvable. \
             Checked: [{checked}]. Stage the host with \
             `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` \
             (the CI shape the supervisor prefers) where the supervisor looks \
             — next to rustamp.exe, or under target/ per the dev-tree heuristic \
             — or `cargo build -p rustamp-plughost` for a same-target host."
        );
    };

    // 2. Spawn the way the supervisor does and attach the link. `supervise`
    // (not a hand-rolled `Command`) is the point: the spawn flags
    // (`CREATE_NO_WINDOW`, piped stdio) are the production ones.
    let mut supervisor = Supervisor::with_path(exe.clone());
    let mut link = HostStateLink::detached();
    supervisor.supervise(&mut link);
    assert!(
        link.is_attached(),
        "the real spawn must attach a transport for {}",
        exe.display()
    );
    assert!(
        supervisor.is_live(),
        "the spawned host must still run for {}",
        exe.display()
    );

    // 3. Publish a known snapshot. First publish after attach always sends;
    // poll-then-publish mirrors `App::pump` (liveness first, mirror second).
    let state = PlayerState {
        transport: TransportState::Playing,
        position_ms: 12_345,
        duration_ms: Some(200_000),
        volume: 200,
        balance: -10,
        playlist_len: 60,
        current_index: Some(7),
        stream: Some(StreamInfo {
            bitrate_kbps: Some(128),
            sample_rate_hz: 44_100,
            channels: 2,
        }),
        shuffle: true,
        repeat: true,
        ..PlayerState::default()
    };
    assert_eq!(
        link.poll(),
        PollOutcome::Healthy,
        "the first poll greets over a fresh spawn"
    );
    assert_eq!(
        link.publish(&state),
        PublishOutcome::Sent,
        "the first publish after attach always sends"
    );

    // 4. Drive liveness like `App::pump` and wait for the window to show the
    // snapshot: the host's `Hello` (with its pid), then its window, then the
    // applied state. Any liveness failure ends the test — a detached link
    // would make the wait meaningless.
    let class = wide_nul(HOST_WINDOW_CLASS);
    let deadline = Instant::now() + E2E_DEADLINE;
    let mut last_mismatch = String::from("no pass yet");
    loop {
        match link.poll() {
            PollOutcome::Healthy => {}
            outcome => panic!("E2E FAILED: liveness {outcome:?} for {}", exe.display()),
        }
        if let Some(greeting) = link.greeting() {
            let hwnd = find_host_window(&class, greeting.pid);
            if hwnd.is_null() {
                last_mismatch = format!("host window not yet visible for pid {}", greeting.pid);
            } else if let Some(mismatch) = probe(hwnd) {
                last_mismatch = mismatch;
            } else {
                last_mismatch.clear();
                break;
            }
        } else {
            last_mismatch = String::from("host Hello not yet read");
        }
        if Instant::now() >= deadline {
            panic!(
                "E2E FAILED: published state never reached the window for {}: \
                 {last_mismatch}; greeting: {:?}; host live: {}",
                exe.display(),
                link.greeting(),
                supervisor.is_live(),
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    // 5. The host answered every query from the published snapshot — and it
    // did so while running, over a live channel (the loop above polled
    // `Hello`/`Ping`/`Pong` the whole way; a missed `Pong` would have
    // panicked there, not here).
    assert!(
        supervisor.is_live(),
        "the host must still run after answering for {}",
        exe.display()
    );
    supervisor.stop(&mut link);
    assert!(!link.is_attached(), "stop detaches the link");
    // Dropping `supervisor` kills what `stop` left, so no host survives this
    // test on any path — including the panics above.
}

// ---- spawn-site guard: the source check below ignores everything after this
// line, so its own needles cannot match themselves. ----

/// This file must never spawn directly: the single spawn lives in
/// `rustamp-app/src/supervise.rs` (which guards `CREATE_NO_WINDOW` there), and
/// this test reaches it only through [`Supervisor`].
#[test]
fn every_spawn_in_this_file_goes_through_the_supervisor() {
    let source = include_str!("plughost_e2e.rs");
    // The test half only; the needles below live after the marker.
    let production = source
        .split("// ---- spawn-site guard")
        .next()
        .unwrap_or(source);

    assert_eq!(
        production.matches("Command::new(").count(),
        0,
        "no direct spawns here: start the host through `Supervisor` so it gets `no_window`"
    );
    assert!(
        !production.contains("std::process::Command"),
        "no direct spawns here: start the host through `Supervisor`"
    );
}
