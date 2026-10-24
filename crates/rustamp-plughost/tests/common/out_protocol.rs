//! The private trace protocol between the P3 output fixture and the acceptance
//! test that drives it.
//!
//! This is **not** a Winamp ABI, not an IPC channel of the host and not a
//! number any specification assigns: it is the name of the fixture's own trace
//! file and the wording of the lines the fixture writes into it. The file name
//! carries the process id, so the fixture and the test agree on it without a
//! handshake, and the trace is written **from inside the plugin**, so it is
//! plugin-side evidence that neither the host nor the runner can forge.
//!
//! Both sides include this file — the fixture
//! (`examples/out_rustamp_fixture.rs`) and the test
//! (`tests/out_acceptance.rs`) — so the two cannot drift apart silently.

/// File name prefix of the fixture's own trace file.
pub const TRACE_FILE_PREFIX: &str = "rustamp-out-fixture-";

/// Extension of the fixture's trace file.
pub const TRACE_FILE_EXTENSION: &str = "log";

/// The module id the fixture publishes.
///
/// S09 §4.1 line 231: third-party modules should use values >= 65536. The
/// host must leave the id alone (the 5.x host replaces it with the DLL file
/// name; whether 2.95 does is open, S09 U-4), so the value is also the
/// evidence that the host did not touch it.
pub const FIXTURE_MODULE_ID: i32 = 0x0001_0000;

/// The description the fixture publishes (S09 §4.1 offset 0x04).
///
/// NUL-terminated, because the ABI member is a pointer to an ANSI string.
pub const FIXTURE_DESCRIPTION: &[u8] = b"Rustamp out fixture\0";

/// The trace line the fixture writes when `init` runs.
pub fn init_line(parent: usize, instance: usize, id: i32) -> String {
    format!("init parent={parent:#x} instance={instance:#x} id={id}")
}

/// The trace line the fixture writes when the host's main window carries the
/// class the ABI fixes (S09 §1).
pub fn parent_class_line(ok: bool) -> String {
    format!("parent class ok={}", u8::from(ok))
}

/// The trace line the fixture writes when `quit` runs.
pub const QUIT_LINE: &str = "quit";

/// Trace prefix of the `open` callback.
///
/// `open` is driven by the **input** plugin from its decode thread
/// (S09 §4.2 step 3), never by the host, so the acceptance test asserts that
/// no such line is in the trace.
pub const EVENT_OPEN: &str = "open ";

/// Trace prefix of the `can_write` callback.
///
/// `can_write` belongs to the same decode-thread path as `open` (S09 §4.2
/// step 3), so the host never calls it either.
pub const EVENT_CAN_WRITE: &str = "can_write ";

/// Trace prefix of the `write` callback. Never called by the host.
pub const EVENT_WRITE: &str = "write ";

/// Trace prefix of the `is_playing` callback.
///
/// `is_playing` belongs to the same decode-thread path as `open`
/// (S09 §4.2 step 3), so the host never calls it either.
pub const EVENT_IS_PLAYING: &str = "is_playing ";

/// Trace prefix of the `close` callback.
///
/// `close` belongs to the same decode-thread path as `open` (S09 §4.2
/// step 3), so the host never calls it either.
pub const EVENT_CLOSE: &str = "close";

/// Trace prefix of the `set_volume` callback.
///
/// The two callbacks the host itself may call are `set_volume` and
/// `set_pan`: S09 §3.5 step 2 has it apply the player's levels to the
/// output before `play` (the hand-over, see `src/handoff.rs`). Without a
/// hand-over the host calls neither, so the acceptance test asserts that no
/// such line is in the trace.
pub const EVENT_SET_VOLUME: &str = "set_volume ";

/// Trace prefix of the `set_pan` callback. See [`EVENT_SET_VOLUME`].
pub const EVENT_SET_PAN: &str = "set_pan ";

/// Trace prefix of the `config` callback.
///
/// The output's `config` takes the parent window and opens a dialog
/// (S09 §4.1 offset 0x14). The host routes it when the player asks for the
/// plug-in's configuration; this fixture's `config` opens **no window**: it
/// only writes this trace line, so the route can be proven headlessly.
pub const EVENT_CONFIG: &str = "config ";

/// The trace line the fixture writes when `config` runs.
pub fn config_line(parent: usize) -> String {
    format!("{EVENT_CONFIG}parent={parent:#x}")
}

/// Trace prefix of the `about` callback. Never called by the host.
pub const EVENT_ABOUT: &str = "about ";
