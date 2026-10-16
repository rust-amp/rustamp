// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The `rustamp` binary, end to end and **headless** (task card N1).
//!
//! `--null` is the deterministic test mode: the app drives a virtual device
//! instead of WASAPI/cpal and opens **no window**, so these tests can run the
//! real binary on the real decoder path without touching the desktop.
//!
//! The reference file is the repo's own 1.5 s, 440 Hz FLAC fixture (generated
//! with ffmpeg, never downloaded; see
//! `crates/rustamp-decoders/tests/fixtures/FIXTURES.txt`).
//!
//! # GUI rule: no spawned child may raise a window
//!
//! GUI must never run on the host (everything graphical runs in the
//! disposable headless test VM). **A console window counts as a window on
//! the host**, so every child spawned here goes through [`no_window`], which
//! sets `CREATE_NO_WINDOW`; a console-subsystem child (`rustamp.exe`) otherwise
//! gets a console window of its own when the parent has no console.
//!
//! Two further rails:
//!
//! * every spawn carries `RUSTAMP_NO_GUI=1` (see [`isolate`]), so a test that
//!   loses `--null` fails instead of opening the app's window;
//! * the tests that do open the app's windows,
//!   [`the_windowed_player_opens_a_window_inside_the_test_vm`] (plus the
//!   playlist, equalizer and menu probes), are `#[ignore]`d
//!   **and** refuse unless they can prove they run inside the headless test VM
//!   ([`rustamp_app::window::require_test_vm`]). An ignored test that can open
//!   a window on the host is a loaded gun; it must not be runnable by accident.
//!   In-process probes build their loop off the main thread on purpose
//!   ([`rustamp_app::window::build_event_loop`]); the spawned player builds
//!   its own on its main thread.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

// OS geometry of a spawned player's windows (VM-only window tests).
#[cfg(windows)]
#[path = "child_windows/mod.rs"]
mod child_windows;

/// `CreateProcess` flag: run the child with a console but **no console window**
/// (`CREATE_NO_WINDOW`).
///
/// Without it a console-subsystem child gets a console window of its own when
/// the parent has no console — and a console window counts as a window on the
/// host.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Makes `command` incapable of raising a window, and returns the creation
/// flags applied so a test can assert them (see
/// [`the_spawn_helper_applies_create_no_window`]).
///
/// **Every** spawn site in this file must go through this. On Windows it sets
/// `CREATE_NO_WINDOW`; elsewhere there is nothing to suppress.
fn no_window(command: &mut Command) -> u32 {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
        CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    {
        let _ = command;
        0
    }
}

/// The `rustamp` binary under test, spawned without a window.
///
/// This is the **only** place in this file that constructs a child process;
/// [`every_spawn_site_is_guarded`] enforces that.
fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustamp"));
    no_window(&mut command);
    command
}

/// An isolated config path so the test never reads the user's real config, the
/// GUI rail (`RUSTAMP_NO_GUI`), and the no-window flag again (belt and braces:
/// `isolate` is called on commands built by [`binary`], but applying it here
/// too means the flag survives any refactor of the constructor).
fn isolate(command: &mut Command, dir: &tempfile::TempDir) {
    no_window(command);
    command.env("RUSTAMP_CONFIG", dir.path().join("rustamp.toml"));
    command.env(rustamp_app::window::NO_GUI_ENV, "1");
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("rustamp-decoders")
        .join("tests")
        .join("fixtures")
        .join("sine.flac")
}

#[test]
fn the_binary_plays_a_reference_file_headless_and_exits_cleanly() {
    let fixture = fixture();
    assert!(fixture.exists(), "missing fixture {}", fixture.display());
    let dir = tempfile::tempdir().unwrap();

    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command
        .arg("--null")
        .arg("--seconds")
        .arg("2")
        .arg(&fixture)
        .output()
        .expect("spawn rustamp");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "exit {:?}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
        output.status
    );
    assert!(stdout.contains("headless"), "stdout:\n{stdout}");
    assert!(stdout.contains("playing"), "stdout:\n{stdout}");
    assert!(stdout.contains("played"), "stdout:\n{stdout}");
    assert!(
        !stdout.contains("played 0 frames"),
        "the engine decoded nothing:\n{stdout}"
    );
}

#[test]
fn the_binary_with_no_file_exits_immediately_in_headless_mode() {
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command.arg("--null").output().expect("spawn rustamp");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("nothing to play"), "stdout:\n{stdout}");
}

#[test]
fn help_prints_the_usage_and_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command.arg("--help").output().expect("spawn rustamp");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(stdout.contains("usage: rustamp"), "stdout:\n{stdout}");
    assert!(stdout.contains("--null"), "stdout:\n{stdout}");
}

#[test]
fn an_unknown_option_fails_with_the_usage() {
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command.arg("--nope").output().expect("spawn rustamp");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2));
    assert!(stderr.contains("unknown option"), "stderr:\n{stderr}");
    assert!(stderr.contains("usage: rustamp"), "stderr:\n{stderr}");
}

#[test]
fn a_missing_skin_falls_back_to_the_builtin_and_still_plays() {
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command
        .arg("--null")
        .arg("--seconds")
        .arg("2")
        .arg("--skin")
        .arg(dir.path().join("no-such-skin.wsz"))
        .arg(&fixture)
        .output()
        .expect("spawn rustamp");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("built-in default skin"),
        "stdout:\n{stdout}"
    );
    assert!(stdout.contains("played"), "stdout:\n{stdout}");
}

/// The GUI rail, exercised: a run that would open a window refuses to.
///
/// This is the check that the default test run cannot start a GUI by accident:
/// the binary is asked for its **windowed** mode (no `--null`) but with
/// `RUSTAMP_NO_GUI=1`, and it must fail before the audio device or the event
/// loop exists. It is headless and enabled by default.
#[test]
fn the_gui_rail_refuses_to_open_a_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary();
    isolate(&mut command, &dir); // sets RUSTAMP_NO_GUI=1
    let output = command
        .arg("--seconds")
        .arg("1")
        .arg(fixture())
        .output()
        .expect("spawn rustamp");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "the windowed run must fail under the GUI rail\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("test VM"),
        "the rail message must name the rule; stderr:\n{stderr}"
    );
    assert!(
        !stdout.contains("output "),
        "the run must stop before the audio device is opened; stdout:\n{stdout}"
    );
}

// The shared test-VM gate lives in `rustamp_app::window::require_test_vm`:
// this file keeps no copy of it.

/// Opens the real window. **VM-only, `#[ignore]`d by default, and gated on
/// the test VM.**
///
/// GUI must never run on the host. Run this only inside the headless test VM
/// (the only host for anything graphical), with an audio endpoint
/// available:
///
/// ```text
/// cargo test -p rustamp-app -- --ignored
/// ```
///
/// Outside the test VM it refuses before spawning anything
/// ([`rustamp_app::window::require_test_vm`]). Inside, it checks the player
/// stays up (the window opened and the event loop is live), reads back the
/// main window's OS geometry — client 275 x 116 (S01 §4), visible — and
/// kills the player.
///
/// The spawned child builds its loop on its own main thread, so it needs no
/// opt-in: the off-main-thread mode
/// ([`rustamp_app::window::build_event_loop`]) is only for the in-process
/// probes, which run on harness worker threads.
#[test]
#[ignore = "VM-only: opens the GUI window; run inside the headless test VM"]
fn the_windowed_player_opens_a_window_inside_the_test_vm() {
    // The gate, before any process exists.
    rustamp_app::window::require_test_vm();

    let dir = tempfile::tempdir().unwrap();
    let mut command = binary(); // no console window (CREATE_NO_WINDOW)
    // Deliberately **not** `isolate()`: this test needs the GUI rail off, so
    // the rail variable is removed rather than set. The app's own window is the
    // point of this test; a *console* window is not, and `binary()` suppresses
    // that.
    command.env_remove(rustamp_app::window::NO_GUI_ENV);
    command.env("RUSTAMP_CONFIG", dir.path().join("rustamp.toml"));
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.arg(fixture()).spawn().expect("spawn rustamp");
    let pid = child.id();

    // If it is still running after three seconds, the window opened and the
    // event loop is live. If it exited, the smoke test failed (no endpoint?).
    std::thread::sleep(Duration::from_secs(3));
    if let Some(status) = child.try_wait().expect("try_wait") {
        let output = child.wait_with_output().expect("wait_with_output");
        panic!(
            "the windowed player exited early ({status}); is there an audio endpoint?\n\
             --- stdout ---\n{}\n--- stderr ---\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // The main window's OS geometry. Windows-only: the headless test VM is
    // Windows. The product does not fix the main window's absolute position
    // (no `with_position`; the OS places it), so the test pins what the
    // specs fix — the S01 rest client size — plus visibility and a readable,
    // non-degenerate outer frame. Absolute docked positions belong to the
    // neighbours: the equalizer probe (`eq_window.rs`) and the
    // docked-relative check in `playlist_window.rs`.
    #[cfg(windows)]
    {
        let main = child_windows::find_window(pid, "Rustamp", Duration::from_secs(10));
        assert_eq!(
            child_windows::client_size(main),
            (275, 116),
            "the main window client area must be the S01 rest size"
        );
        let (left, top, right, bottom) = child_windows::outer_rect(main);
        assert!(
            right > left && bottom > top,
            "the main window outer frame must be non-degenerate, got \
             ({left},{top})-({right},{bottom})"
        );
    }
    child.kill().expect("kill the windowed player");
    let output = child.wait_with_output().expect("wait_with_output");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("output "),
        "the player never reported its device; stdout:\n{stdout}"
    );
}

// ---- spawn-site guard: the source check below ignores everything after this
// line, so its own needles cannot match themselves. ----

/// The split marker for [`every_spawn_site_is_guarded`].
const SPAWN_GUARD_MARKER: &str = "// ---- spawn-site guard";

/// Asserts the flag the spawn helper applies, **without spawning anything**
/// (it only builds a `Command` and never calls `spawn`).
///
/// `std::process::Command` exposes no getter for `creation_flags`, so this is
/// the closest thing to asserting the constructed command's flags that stable
/// `std` allows; [`every_spawn_site_is_guarded`] covers the other half.
#[test]
fn the_spawn_helper_applies_create_no_window() {
    let mut command = Command::new("never-spawned-probe");
    let flags = no_window(&mut command);
    #[cfg(windows)]
    assert_eq!(
        flags, CREATE_NO_WINDOW,
        "`no_window` must apply `CREATE_NO_WINDOW` (0x0800_0000)"
    );
    #[cfg(not(windows))]
    assert_eq!(flags, 0);
}

/// Guards the spawn sites: exactly one place constructs a child, and it goes
/// through [`no_window`], so no test can raise a console window on the host.
///
/// This inspects its own source because `std::process::Command` has no
/// `creation_flags` getter. If this test fails, a spawned child can raise a
/// window on the host: restore `CREATE_NO_WINDOW` / route the spawn through
/// [`binary`].
#[test]
fn every_spawn_site_is_guarded() {
    let source = include_str!("headless.rs");
    // The production half only; the needles below live after the marker.
    let production = source.split(SPAWN_GUARD_MARKER).next().unwrap_or(source);

    assert_eq!(
        production.matches("Command::new(").count(),
        1,
        "there must be exactly one spawn site (`binary()`); route new spawns through it so \
         they get `no_window`"
    );
    assert!(
        production.contains("creation_flags(CREATE_NO_WINDOW)"),
        "`no_window` must set `CREATE_NO_WINDOW`"
    );
    assert!(
        production.contains("no_window(&mut command)"),
        "`binary()` must apply the no-window flag"
    );
    assert!(
        production.contains("no_window(command)"),
        "`isolate()` must apply the no-window flag"
    );
}
