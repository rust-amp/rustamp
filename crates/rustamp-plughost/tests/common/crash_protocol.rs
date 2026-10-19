//! The private trace protocol between the crashing plugin fixture and the
//! acceptance test that loads it.
//!
//! Like `gen_protocol.rs`, this is **not** a Winamp ABI and not a channel of
//! the host: it is the file name the fixture writes its own trace to, so the
//! fixture and the test name the same file without a handshake.
//!
//! The trace is written **from inside the plugin**, in the host process, and
//! named after that process id — so it is evidence the host or the test
//! runner cannot forge.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

/// File name prefix of the fixture's own trace file.
pub const TRACE_FILE_PREFIX: &str = "rustamp-crash-fixture-";

/// Extension of the fixture's trace file.
pub const TRACE_FILE_EXTENSION: &str = "log";

/// The file name of the crashing fixture DLL.
pub const FIXTURE_FILE_NAME: &str = "crash_rustamp_fixture.dll";
