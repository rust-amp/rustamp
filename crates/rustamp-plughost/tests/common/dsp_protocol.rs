//! The private trace protocol between the DSP fixture and the acceptance test
//! that drives it.
//!
//! This is **not** a Winamp ABI, not an IPC channel of the host and not a
//! number any specification assigns: it is the name of the fixture's own trace
//! file and the wording of the lines the fixture writes into it. The file name
//! carries the process id, so the fixture and the test agree on it without a
//! handshake, and the trace is written **from inside the plugin**, so it is
//! plugin-side evidence that neither the host nor the runner can forge.
//!
//! Both sides include this file — the fixture
//! (`examples/dsp_rustamp_fixture.rs`) and the test
//! (`tests/dsp_fixture_acceptance.rs`) — so the two cannot drift apart
//! silently.

/// File name prefix of the fixture's own trace file.
pub const TRACE_FILE_PREFIX: &str = "rustamp-dsp-fixture-";

/// Extension of the fixture's trace file.
pub const TRACE_FILE_EXTENSION: &str = "log";

/// The library name the fixture publishes (S09 §6.1, offset 0x04).
///
/// NUL-terminated, because the ABI member is a pointer to an ANSI string.
pub const FIXTURE_LIBRARY_DESCRIPTION: &[u8] = b"Rustamp dsp fixture\0";

/// The name of the fixture's first module (S09 §6.2, offset 0x00).
pub const FIXTURE_MODULE_A_DESCRIPTION: &[u8] = b"Rustamp dsp fixture A\0";

/// The name of the fixture's second module (S09 §6.2, offset 0x00).
pub const FIXTURE_MODULE_B_DESCRIPTION: &[u8] = b"Rustamp dsp fixture B\0";

/// The code the fixture's second module's `init` returns (S09 §6.2, offset
/// 0x10: "0 on success"; the host treats a nonzero return as a refusal).
pub const FIXTURE_INIT_REFUSAL_CODE: i32 = 1;

/// The factor the fixture's `process` applies to every sample, so the effect
/// is measurable and exactly predictable.
pub const FIXTURE_GAIN: i16 = 2;

/// The trace line the fixture writes when a module's `init` runs.
///
/// `module` is the index of the module in the library's module list.
pub fn init_line(module: usize, parent: usize, instance: usize) -> String {
    format!("init module={module} parent={parent:#x} instance={instance:#x}")
}

/// The trace line the fixture writes when a block was processed.
///
/// `frames` is the number of frames the host passed in, `out` the number the
/// fixture returned, and `first` the first sample after the gain.
pub fn process_line(module: usize, frames: i32, out: i32, first: i16) -> String {
    format!("process module={module} frames={frames} out={out} first={first}")
}

/// The trace line the fixture writes when a module is torn down.
pub fn quit_line(module: usize) -> String {
    format!("quit module={module}")
}

/// The trace prefix of the `config` callback.
///
/// A DSP `config` opens the plugin's configuration dialog (S09 §6.2, offset
/// 0x0C). This fixture's `config` opens **no window**: it only writes this
/// line, so the acceptance test can prove the route without a dialog.
pub const EVENT_CONFIG: &str = "config ";
