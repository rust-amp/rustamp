// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The playlist editor window, opened for real (sandbox-only).
//!
//! GUI must never run on the host, so this test is `#[ignore]`d **and** gated
//! on the sandbox user, exactly like
//! `headless::the_windowed_player_opens_a_window_inside_the_sandbox`. It
//! spawns the real binary in windowed mode (no `--null`, GUI rail off), which
//! opens both the main window and the playlist editor (`Winamp PE`, 275 x 116,
//! docked below the main window). A console window counts as a window on the
//! host, so the child goes through the same `CREATE_NO_WINDOW` guard.
//!
//! Run inside the Windows Sandbox with an audio endpoint available:
//!
//! ```text
//! cargo test -p rustamp-app -- --ignored playlist_window
//! ```

use std::process::{Command, Stdio};
use std::time::Duration;

/// `CreateProcess` flag: run the child with a console but **no console window**
/// (`CREATE_NO_WINDOW`), mirroring `headless.rs`.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// The Windows Sandbox account (the marker that proves a run is not on the
/// host), mirroring `headless.rs`.
const SANDBOX_USER: &str = "WDAGUtilityAccount";

/// Refuses unless this process provably runs inside the Windows Sandbox.
///
/// There is deliberately **no** override: this test opens the app's windows.
fn require_windows_sandbox() {
    let user = std::env::var("USERNAME").unwrap_or_default();
    assert!(
        user.eq_ignore_ascii_case(SANDBOX_USER),
        "refusing to open GUI windows: this test is sandbox-only, but the user is {user:?} \
         rather than {SANDBOX_USER:?}. GUI must never run on the host; run it inside the \
         Windows Sandbox (see crates/rustamp-app/README.md)."
    );
}

fn fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("rustamp-decoders")
        .join("tests")
        .join("fixtures")
        .join("sine.flac")
}

/// Opens the real windows. **Sandbox-only, `#[ignore]`d by default, and gated
/// on the sandbox user.**
#[test]
#[ignore = "sandbox-only: opens the GUI windows; run inside the Windows Sandbox"]
fn the_playlist_window_opens_docked_below_the_main_window() {
    // The gate, before any process exists.
    require_windows_sandbox();

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
    let mut child = command.arg(fixture()).spawn().expect("spawn rustamp");

    // If it is still running after three seconds, the windows opened and the
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
    child.kill().expect("kill the windowed player");
    let output = child.wait_with_output().expect("wait_with_output");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("output "),
        "the player never reported its device; stdout:\n{stdout}"
    );
}
