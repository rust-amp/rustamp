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

#[cfg(windows)]
#[path = "../src/external_control/readiness.rs"]
mod ipc_readiness;

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

/// A playlist with one playable entry and two malformed ones.
///
/// The app notes what it skipped: the report and one line per skipped
/// entry, in the order it skipped them. The malformed entries are the two
/// `#EXTINF` lines that never find their location line.
fn playlist_with_two_skips(dir: &tempfile::TempDir) -> PathBuf {
    let source = fixture();
    std::fs::copy(&source, dir.path().join("sine.flac")).expect("the fixture is copied");
    let list = dir.path().join("list.m3u");
    std::fs::write(
        &list,
        "#EXTM3U\n#EXTINF:1,First\n#EXTINF:2,Second\nsine.flac\n#EXTINF:3,Third\n",
    )
    .expect("the playlist is written");
    list
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

/// The notes reach the standard error in a headless run.
///
/// The windowed presenter was the only printer of a note, so a headless run
/// swallowed every diagnostic the app raised. The playlist with two
/// malformed entries makes the app raise three: the report and one line per
/// skipped entry.
#[test]
fn a_headless_run_prints_the_notes_it_raised() {
    let dir = tempfile::tempdir().unwrap();
    let list = playlist_with_two_skips(&dir);
    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command
        .arg("--null")
        .arg("--seconds")
        .arg("2")
        .arg(&list)
        .output()
        .expect("spawn rustamp");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        stderr.contains("rustamp: loaded 1 tracks, skipped 2 malformed entries from"),
        "the note must reach the standard error: {stderr}"
    );
}

/// The notes keep the order they were raised in.
#[test]
fn the_notes_of_a_headless_run_keep_their_order() {
    let dir = tempfile::tempdir().unwrap();
    let list = playlist_with_two_skips(&dir);
    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command
        .arg("--null")
        .arg("--seconds")
        .arg("2")
        .arg(&list)
        .output()
        .expect("spawn rustamp");

    let stderr = String::from_utf8_lossy(&output.stderr);
    let report = stderr
        .find("loaded 1 tracks, skipped 2 malformed entries from")
        .expect("the report is printed");
    let first = stderr
        .find("skipped: dangling #EXTINF without a location line: First")
        .expect("the first skipped entry is printed");
    let second = stderr
        .find("skipped: dangling #EXTINF without a location line: Third")
        .expect("the second skipped entry is printed");
    assert!(
        report < first,
        "the report comes before the skips: {stderr}"
    );
    assert!(first < second, "the skips keep their order: {stderr}");
}

/// A note the run itself raises reaches the standard error as well.
///
/// The file does not exist: the engine reports the failure while the run is
/// going, and the app notes it.
#[test]
fn a_note_raised_during_a_headless_run_reaches_the_standard_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-file");
    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command
        .arg("--null")
        .arg(&missing)
        .output()
        .expect("spawn rustamp");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr
            .lines()
            .any(|line| line.starts_with("rustamp: error:")),
        "the note the run raised must reach the standard error: {stderr}"
    );
}

/// A run that raises no note prints none.
#[test]
fn a_headless_run_without_notes_prints_none() {
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command
        .arg("--null")
        .arg("--seconds")
        .arg("2")
        .arg(fixture())
        .output()
        .expect("spawn rustamp");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        !stderr.lines().any(|line| line.starts_with("rustamp: ")),
        "a run without notes must print none: {stderr}"
    );
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
    assert!(stdout.contains("/CLASS="), "stdout:\n{stdout}");
    assert!(stdout.contains("/ONLYINSTALL"), "stdout:\n{stdout}");
}

#[test]
fn an_enqueued_file_is_added_and_not_played() {
    // `--enqueue` is the file the shell's `Enqueue` verb runs: the file is
    // added to the playlist and the playback does not start.
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command
        .arg("--null")
        .arg("--enqueue")
        .arg(&fixture)
        .output()
        .expect("spawn rustamp");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        !stdout.contains("playing"),
        "an enqueued file must not start the playback:\n{stdout}"
    );
}

#[test]
fn a_bookmark_is_stored_and_not_played() {
    // S13 §1: `/BOOKMARK <path>` adds the path as a bookmark rather than
    // playing it.
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command
        .arg("--null")
        .arg("/BOOKMARK")
        .arg(&fixture)
        .output()
        .expect("spawn rustamp");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        !stdout.contains("playing"),
        "a bookmark must not start the playback:\n{stdout}"
    );
    let store = std::fs::read_to_string(dir.path().join("bookmarks.m3u8")).expect("the store");
    assert!(store.contains("sine.flac"), "store:\n{store}");
}

#[test]
fn an_unknown_slash_token_is_a_file_argument() {
    // S13 §1: a token the reference's switch list does not name is a file
    // argument and not an error.
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command
        .arg("--null")
        .arg("/PLAY")
        .output()
        .expect("spawn rustamp");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // The run does not fail on the token itself; the file it names simply
    // does not exist.
    assert!(
        !stderr.contains("unknown option"),
        "an unknown `/` token must not be refused:\n{stderr}"
    );
    // The outcome the token leads to: the token was taken as a file
    // argument, so the run tried to open it and failed with the file's own
    // error. The **exit** is the outcome — the transport's state while the
    // run starts is not: the engine sets it before the decode thread has
    // opened the file, so it says `Playing` for a file that never plays.
    assert!(
        !output.status.success(),
        "the run must fail on the file the token names:\n{stdout}\n{stderr}"
    );
    assert!(
        stderr.contains("I/O error"),
        "the failure must be the file that does not exist:\n{stderr}"
    );
    // Nothing plays: the file never opened, so the run never announced a
    // track. The announcement follows the opened track, not the transport
    // (see `App::run_headless`), so this is a fact about the run and not a
    // race with the decode thread.
    assert!(!stdout.contains("playing"), "stdout:\n{stdout}");
}

#[test]
fn a_first_start_with_an_added_path_plays_it() {
    // S13 §1: a first start with `/ADD <path>` appends the path and plays the
    // list. Only a running instance adds without playing.
    let fixture = fixture();
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command
        .arg("--null")
        .arg("/ADD")
        .arg(&fixture)
        .output()
        .expect("spawn rustamp");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("playing"),
        "a first start with an added path plays:\n{stdout}"
    );
    assert!(
        stderr.contains(&fixture.display().to_string()),
        "the run must report the path it added:\n{stderr}"
    );
}

#[test]
fn an_add_without_a_path_is_reported() {
    // S13 §5.3 leaves the form of a path-taking switch without a path open,
    // so the run reports it instead of guessing.
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary();
    isolate(&mut command, &dir);
    let output = command
        .arg("--null")
        .arg("/ADD")
        .output()
        .expect("spawn rustamp");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stderr.contains("/ADD without a path"), "stderr:\n{stderr}");
    assert!(!stdout.contains("playing"), "stdout:\n{stdout}");
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
        fn GetPropW(hwnd: *mut c_void, name: *const u16) -> *mut c_void;
    }

    fn try_query(hwnd: *mut c_void, id: isize, argument: i32) -> std::io::Result<u32> {
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
        if ok == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(result as u32)
        }
    }

    fn query(hwnd: *mut c_void, id: isize, argument: i32) -> u32 {
        try_query(hwnd, id, argument)
            .unwrap_or_else(|error| panic!("IPC {id} timed out or failed: {error}"))
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

    struct ChildGuard {
        child: std::process::Child,
        stdout: PathBuf,
        stderr: PathBuf,
    }
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            if std::thread::panicking() {
                eprintln!(
                    "player stdout:\n{}\nplayer stderr:\n{}",
                    std::fs::read_to_string(&self.stdout).unwrap_or_default(),
                    std::fs::read_to_string(&self.stderr).unwrap_or_default()
                );
            }
        }
    }

    rustamp_app::window::require_test_vm();
    let dir = tempfile::tempdir().unwrap();
    let mut command = binary(); // Always CREATE_NO_WINDOW, even in the guest.
    isolate(&mut command, &dir);
    command.env_remove(rustamp_app::window::NO_GUI_ENV);
    let stdout = dir.path().join("player.stdout");
    let stderr = dir.path().join("player.stderr");
    let mut child = ChildGuard {
        child: command
            .args(["--no-audio", "--volume", "128"])
            .stdout(std::fs::File::create(&stdout).unwrap())
            .stderr(std::fs::File::create(&stderr).unwrap())
            .spawn()
            .expect("spawn player"),
        stdout,
        stderr,
    };
    let hwnd = child_windows::find_window(child.child.id(), "Rustamp", Duration::from_secs(15));
    assert_eq!(child_windows::window_class(hwnd), "Winamp v1.x");
    // A version reply may arrive reentrantly during startup and does not prove
    // that startup finished. Wait without sending for the installed handler's
    // first complete UI tick. EVERY IPC send, including the first version
    // query, retains its strict 1 s timeout with no failed-send retry.
    let ready_property: Vec<u16> = ipc_readiness::READY_PROPERTY
        .encode_utf16()
        .chain([0])
        .collect();
    let started = Instant::now();
    let deadline = started + Duration::from_secs(10);
    let mut retries = 0;
    loop {
        assert!(
            child.child.try_wait().unwrap().is_none(),
            "player exited before IPC readiness"
        );
        // SAFETY: this is our child's HWND and a live NUL-terminated name.
        // GetPropW reads window-manager data without dispatching into the UI
        // thread; a stuck player therefore cannot stall this bounded barrier.
        let ready = unsafe { GetPropW(hwnd, ready_property.as_ptr()) } as usize;
        if ready == 1 {
            eprintln!(
                "player IPC ready after {:?}; startup retries={retries}",
                started.elapsed()
            );
            break;
        }
        retries += 1;
        assert!(
            Instant::now() < deadline,
            "player handler never completed its first UI tick: readiness marker={ready}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(query(hwnd, 0, 0), 0x2905, "version after readiness");
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

/// Structural startup/lifetime regression, without creating an HWND. Actual
/// Win32 scheduling and cross-process marker visibility still need the VM.
#[test]
fn ipc_handler_precedes_visibility_and_readiness_follows_the_full_ui_tick() {
    let source = include_str!("../src/window.rs");
    let resumed = source
        .split("fn resumed(")
        .nth(1)
        .unwrap()
        .split("fn window_event(")
        .next()
        .unwrap();
    let mut previous = 0;
    for step in [
        ".with_visible(false)",
        "event_loop.create_window(attributes)",
        "ExternalControl::attach(",
        "softbuffer::Context::new(",
        "self.applied_main_visible = false",
        "self.attach_tray()",
        "self.sync_window_state()",
    ] {
        let position = resumed
            .find(step)
            .unwrap_or_else(|| panic!("missing startup step: {step}"));
        assert!(position > previous, "startup step is out of order: {step}");
        previous = position;
    }
    assert!(
        !resumed.contains(".mark_ready()"),
        "resumed is still allowed to dispatch reentrantly"
    );
    let tick = source
        .split("fn about_to_wait(")
        .nth(1)
        .unwrap()
        .split("fn exiting(")
        .next()
        .unwrap();
    let ready = tick.find("control.mark_ready()").unwrap();
    for step in [
        "control.publish(",
        "self.present_frames()",
        "self.sync_title()",
        "event_loop.set_control_flow(",
    ] {
        assert!(
            tick.find(step).unwrap() < ready,
            "readiness precedes full tick: {step}"
        );
    }
    let exiting = source.split("fn exiting(").nth(1).unwrap();
    assert!(
        exiting.contains("self.external_control = None"),
        "exiting must remove handler/readiness"
    );
}

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
