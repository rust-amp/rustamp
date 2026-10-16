// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The playlist editor window, opened for real (VM-only).
//!
//! GUI must never run on the host, so this test is `#[ignore]`d **and** gated
//! on the headless test VM, exactly like
//! `headless::the_windowed_player_opens_a_window_inside_the_test_vm`. It
//! spawns the real binary in the silent windowed mode (`--no-audio`, GUI rail
//! off), which opens both the main window and the playlist editor (`Winamp PE`,
//! 275 x 116, docked below the main window) on the null output — so it needs
//! **no audio endpoint**. A console window counts as a window on the
//! host, so the child goes through the same `CREATE_NO_WINDOW` guard.
//!
//! Run inside the headless test VM; no audio endpoint is needed:
//!
//! ```text
//! cargo test -p rustamp-app -- --ignored playlist_window
//! ```

use std::process::{Command, Stdio};
use std::time::Duration;

// OS geometry of a spawned player's windows (VM-only window tests).
#[cfg(windows)]
#[path = "child_windows/mod.rs"]
mod child_windows;

/// `CreateProcess` flag: run the child with a console but **no console window**
/// (`CREATE_NO_WINDOW`), mirroring `headless.rs`.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// The shared test-VM gate lives in `rustamp_app::window::require_test_vm`:
// this file keeps no copy of it. There is deliberately **no** override:
// this test opens the app's windows.

fn fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("rustamp-decoders")
        .join("tests")
        .join("fixtures")
        .join("sine.flac")
}

/// Opens the real windows. **VM-only, `#[ignore]`d by default, and gated
/// on the test VM.**
///
/// Besides staying up (the windows opened and the event loop is live), the
/// test reads back the OS geometry: the main client is 275 x 116 (S01 §4),
/// the playlist client is 275 x 116 (S06 scope, S01 §1), and the playlist
/// outer origin sits exactly one docked step below the main outer origin —
/// the S05 §3.1/§4.2 neighbour relation, independent of where the OS placed
/// the main window (the product sets no absolute main position).
#[test]
#[ignore = "VM-only: opens the GUI windows; run inside the headless test VM"]
fn the_playlist_window_opens_docked_below_the_main_window() {
    // The gate, before any process exists.
    rustamp_app::window::require_test_vm();

    // The creation parameters the playlist window must carry (S06 scope,
    // S01 §1; S05 §3.1 for the docked origin): checked here so the manual run
    // has a pinned contract even before the windows are eyeballed.
    assert_eq!(rustamp_ui::PLAYLIST_WINDOW_SIZE, (275, 116));
    assert_eq!(
        (
            rustamp_ui::DEFAULT_PLAYLIST_POSITION.x,
            rustamp_ui::DEFAULT_PLAYLIST_POSITION.y
        ),
        (26, 261)
    );

    let dir = tempfile::tempdir().expect("tempdir");
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustamp"));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    // Deliberately no `RUSTAMP_NO_GUI`: the windows are the point of this
    // test. The config is isolated so the test never reads real user state.
    command.env("RUSTAMP_CONFIG", dir.path().join("rustamp.toml"));
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    // The silent windowed mode: the windows open, but playback runs on the
    // null output, so the test needs no audio endpoint. The subject is the
    // docked geometry, not the device.
    let mut child = command
        .arg("--no-audio")
        .arg(fixture())
        .spawn()
        .expect("spawn rustamp");
    let pid = child.id();

    // If it is still running after three seconds, the windows opened and the
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
    // The spawned child builds its loop on its own main thread, so it needs
    // no opt-in (the off-main-thread mode is only for in-process probes).
    // Windows-only: the headless test VM is Windows.
    #[cfg(windows)]
    {
        let main = child_windows::find_window(pid, "Rustamp", Duration::from_secs(10));
        let playlist = child_windows::find_window(pid, "Rustamp Playlist", Duration::from_secs(10));
        assert_eq!(
            child_windows::client_size(main),
            (275, 116),
            "the main window client area must be the S01 rest size"
        );
        assert_eq!(
            child_windows::client_size(playlist),
            (275, 116),
            "the playlist client area must be the spec'd 275 x 116"
        );
        let (main_x, main_y, _, _) = child_windows::outer_rect(main);
        let (pl_x, pl_y, _, _) = child_windows::outer_rect(playlist);
        let dx = rustamp_ui::DEFAULT_PLAYLIST_POSITION.x - rustamp_ui::DEFAULT_MAIN_POSITION.x;
        let dy = rustamp_ui::DEFAULT_PLAYLIST_POSITION.y - rustamp_ui::DEFAULT_MAIN_POSITION.y;
        assert_eq!(
            (pl_x, pl_y),
            (main_x + dx, main_y + dy),
            "the playlist must sit docked below the main window (S05 §3.1/§4.2)"
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
