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
/// (the only host for anything graphical). It needs **no audio endpoint**: it
/// spawns the player with `--no-audio` (the silent windowed mode — the null
/// output with the window open), so a Hyper-V VM without a device can still
/// prove the window opened.
///
/// ```text
/// cargo test -p rustamp-app -- --ignored
/// ```
///
/// Outside the test VM it refuses before spawning anything
/// ([`rustamp_app::window::require_test_vm`]). Inside, it checks the player
/// stays up (the window opened and the event loop is live), reads back the
/// main window's OS geometry — client 275 x 116 (S01 §4), visible — and
/// kills the player. The final assertion pins `output null`: the run must
/// have taken the silent path, never a real device.
///
/// The window's **title and class must round-trip in full** (task card
/// N12): both are read back through the wide API into a wide buffer and
/// compared exactly, so a wide/ANSI mismatch at the Win32 boundary — on
/// the player's writing side or the reader's side — fails here instead of
/// surfacing as a one-character title or class.
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
    // The silent windowed mode: the window opens, but playback runs on the
    // null output, so the test needs no audio endpoint (a Hyper-V VM has
    // none). The subject is the window, not the device.
    let mut child = command
        .arg("--no-audio")
        .arg(fixture())
        .spawn()
        .expect("spawn rustamp");
    let pid = child.id();

    // If it is still running after three seconds, the window opened and the
    // event loop is live. If it exited, the smoke test failed (and
    // `--no-audio` rules out a missing audio endpoint as the cause).
    std::thread::sleep(Duration::from_secs(3));
    if let Some(status) = child.try_wait().expect("try_wait") {
        let output = child.wait_with_output().expect("wait_with_output");
        panic!(
            "the windowed player exited early ({status}); it runs silent (`--no-audio`), \
             so this is not a missing audio endpoint\n\
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
        // The title and the class round-trip in full (task card N12).
        // `find_window` already matched the exact multi-character title; the
        // explicit read-backs below pin the title and add the class, which
        // no test read before. S10 requires the main-window class
        // `Winamp v1.x` for discovery by external controllers; a
        // one-character title or class is the wide/ANSI-mismatch
        // signature, not a real window property.
        assert_eq!(
            child_windows::window_title(main),
            "Rustamp",
            "the main window title must round-trip in full"
        );
        assert_eq!(
            child_windows::window_class(main),
            "Winamp v1.x",
            "the main window class must round-trip in full: S10's \
             external-control class; a one-character class is the wide/ANSI-mismatch \
             signature (task card N12)"
        );
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
        stdout.contains("output null"),
        "the player never reported the silent device; stdout:\n{stdout}"
    );
}

/// The GUI rail covers the silent windowed mode too.
///
/// `--no-audio` would open a window, so with `RUSTAMP_NO_GUI=1` it must refuse
/// before the audio device or the event loop exists — exactly like the default
/// windowed run. This refuses fast without opening anything, so it runs
/// anywhere (no `#[ignore]`, no VM gate): it proves the new flag cannot slip
/// past the rail.
#[test]
fn the_gui_rail_refuses_the_silent_windowed_mode_too() {
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary();
    isolate(&mut command, &dir); // sets RUSTAMP_NO_GUI=1
    let output = command
        .arg("--no-audio")
        .arg(fixture())
        .output()
        .expect("spawn rustamp");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "the silent windowed run must fail under the GUI rail\nstdout:\n{stdout}\nstderr:\n{stderr}"
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

/// An external process drives the actual player HWND, not an emulated host.
/// Never run this on the host: it opens the real player window.
#[cfg(windows)]
#[test]
#[ignore = "VM-only: opens a real player window; requires an interactive guest session"]
fn external_control_drives_the_player_window_in_the_test_vm() {
    use std::ffi::c_void;
    use std::time::Instant;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SendMessageTimeoutW(
            hwnd: *mut c_void,
            message: u32,
            wparam: usize,
            lparam: isize,
            flags: u32,
            timeout: u32,
            result: *mut usize,
        ) -> isize;
        fn PostMessageW(hwnd: *mut c_void, message: u32, wparam: usize, lparam: isize) -> i32;
    }

    fn query(hwnd: *mut c_void, id: isize, argument: i32) -> u32 {
        let mut result = 0;
        // SAFETY: HWND belongs to our live child; the result buffer is valid,
        // the query carries no pointers, and the send has a bounded timeout.
        let ok = unsafe {
            SendMessageTimeoutW(
                hwnd,
                0x0400,
                argument as u32 as usize,
                id,
                2,
                1000,
                &mut result,
            )
        };
        assert_ne!(ok, 0, "IPC {id} timed out or failed");
        result as u32
    }

    fn post(hwnd: *mut c_void, message: u32, argument: usize, id: isize) {
        // SAFETY: only integer command/query parameters go to our child HWND.
        assert_ne!(unsafe { PostMessageW(hwnd, message, argument, id) }, 0);
    }

    fn eventually(hwnd: *mut c_void, id: isize, argument: i32, expected: u32) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let actual = query(hwnd, id, argument);
            if actual == expected {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "IPC {id}: expected {expected}, got {actual}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    rustamp_app::window::require_test_vm();
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary(); // Always CREATE_NO_WINDOW, even in the guest.
    isolate(&mut command, &dir);
    command.env_remove(rustamp_app::window::NO_GUI_ENV);
    let child = ChildGuard(
        command
            .args(["--no-audio", "--volume", "128"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn player"),
    );
    let hwnd = child_windows::find_window(child.0.id(), "Rustamp", Duration::from_secs(15));
    assert_eq!(child_windows::window_class(hwnd), "Winamp v1.x");
    eventually(hwnd, 0, 0, 0x2905); // Wait until the subclass is installed.
    for (id, argument, expected) in [
        (104, -1, 0),
        (105, 0, u32::MAX),
        (105, 1, u32::MAX),
        (105, 2, 0),
        (122, -666, 128),
        (123, -666, 0),
        (124, 0, 0),
        (125, 0, 0),
        (250, 0, 0),
        (251, 0, 0),
        (9999, 0, 1),
        (-1, 0, 1),
    ] {
        assert_eq!(query(hwnd, id, argument), expected, "IPC {id}");
    }
    // Sent setter replies are a spec gap: refusal must not mutate the player.
    assert_eq!(query(hwnd, 122, 64), 1);
    assert_eq!(query(hwnd, 122, -666), 128);
    post(hwnd, 0x0111, (0x1234 << 16) | 40058, 0); // Existing VolumeUp path.
    eventually(hwnd, 122, -666, 132);
    post(hwnd, 0x0111, 40023, 0); // Existing ToggleShuffle path.
    eventually(hwnd, 250, 0, 1);
    post(hwnd, 0x0400, 0, 252); // Measured posted setter, not a sent-reply guess.
    eventually(hwnd, 250, 0, 0);
    post(hwnd, 0x0400, 255, 123);
    eventually(hwnd, 123, -666, u32::MAX);
    post(hwnd, 0x0400, 1, 253);
    eventually(hwnd, 251, 0, 1);
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
