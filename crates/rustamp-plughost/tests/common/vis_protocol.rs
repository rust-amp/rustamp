//! The private trace protocol between the visualization fixture and the
//! acceptance test that drives it.
//!
//! This is **not** a Winamp ABI, not an IPC channel of the host and not a
//! number any specification assigns: it is the name of the fixture's own trace
//! file and the wording of the lines the fixture writes into it. The file name
//! carries the process id, so the fixture and the test agree on it without a
//! handshake, and the trace is written **from inside the plugin**, so it is
//! plugin-side evidence that neither the host nor the runner can forge.
//!
//! Both sides include this file — the fixture
//! (`examples/vis_rustamp_fixture.rs`) and the test
//! (`tests/vis_acceptance.rs`) — so the two cannot drift apart silently.

/// File name prefix of the fixture's own trace file.
pub const TRACE_FILE_PREFIX: &str = "rustamp-vis-fixture-";

/// Extension of the fixture's trace file.
pub const TRACE_FILE_EXTENSION: &str = "log";

/// The library name the fixture publishes (S09 §5.1, offset 0x04).
///
/// NUL-terminated, because the ABI member is a pointer to an ANSI string.
pub const FIXTURE_LIBRARY_DESCRIPTION: &[u8] = b"Rustamp vis fixture\0";

/// The name of the fixture's first module (S09 §5.2, offset 0x000).
pub const FIXTURE_MODULE_A_DESCRIPTION: &[u8] = b"Rustamp vis fixture A\0";

/// The name of the fixture's second module (S09 §5.2, offset 0x000).
pub const FIXTURE_MODULE_B_DESCRIPTION: &[u8] = b"Rustamp vis fixture B\0";

/// The latency the fixture's first module publishes (S09 §5.2, offset 0x014).
///
/// 14 ms, the value S09 §5.2 records for the 2.95 AVS plugin.
pub const FIXTURE_LATENCY_MS: i32 = 14;

/// The delay the fixture's first module publishes (S09 §5.2, offset 0x018).
///
/// 14 ms, the value S09 §5.2 records for the 2.95 AVS plugin.
pub const FIXTURE_DELAY_MS: i32 = 14;

/// The spectrum rows the fixture's first module asks for (S09 §5.2, offset
/// 0x01C).
pub const FIXTURE_SPECTRUM_ROWS: i32 = 2;

/// The waveform rows the fixture's first module asks for (S09 §5.2, offset
/// 0x020).
pub const FIXTURE_WAVEFORM_ROWS: i32 = 2;

/// The code the fixture's second module's `init` returns (S09 §5.2, offset
/// 0x928: a nonzero return aborts the start).
pub const FIXTURE_INIT_REFUSAL_CODE: i32 = 1;

/// The trace line the fixture writes when a module's `init` runs.
///
/// `module` is the index of the module in the library's module list.
pub fn init_line(
    module: usize,
    parent: usize,
    instance: usize,
    rate: i32,
    channels: i32,
) -> String {
    format!(
        "init module={module} parent={parent:#x} instance={instance:#x} rate={rate} channels={channels}"
    )
}

/// The trace line the fixture writes when a frame is drawn.
///
/// The two values are the first bytes of the spectrum and the waveform row
/// the host filled before the call.
pub fn render_line(module: usize, spectrum_first: u8, waveform_first: u8) -> String {
    format!("render module={module} spectrum={spectrum_first} waveform={waveform_first}")
}

/// The trace line the fixture writes when a module is torn down.
pub fn quit_line(module: usize) -> String {
    format!("quit module={module}")
}

/// The trace prefix of the `config` callback.
///
/// The host routes it to the running module when the player asks for the
/// visualization plug-in's configuration (S09 §5.2 offset 0x924). This
/// fixture's `config` opens **no window**: it only writes this trace line,
/// so the route can be proven headlessly.
pub const EVENT_CONFIG: &str = "config ";

/// The trace line the fixture writes when a module's `config` runs.
pub fn config_line(module: usize) -> String {
    format!("{EVENT_CONFIG}module={module}")
}
