//! The private challenge protocol between the P3 general-purpose fixture and
//! the acceptance test that drives it.
//!
//! This is **not** a Winamp ABI, not an IPC channel of the host and not a
//! number any specification assigns:
//!
//! * the transport is an **OS-registered message**
//!   (`RegisterWindowMessageW`), so both sides derive the same message id from
//!   the same name without anybody handing out a numeric id;
//! * the command ids below are private to *this* fixture and *this* test. They
//!   are never sent to a real host and are not Winamp IPC numbers.
//!
//! P3 §5.5 asks for exactly this: a private test transport chosen by the
//! implementation, with the fixture's window identity and its dialog
//! automation defined here, and no numeric ABI invented anywhere.
//!
//! Both sides include this file — the fixture
//! (`examples/gen_rustamp_fixture.rs`) and the test
//! (`tests/gen_acceptance.rs`) — so the two cannot drift apart silently.

/// Name of the OS-registered message that carries a challenge to the fixture.
///
/// `RegisterWindowMessageW` maps this name to the same id for every process in
/// the session, so the fixture and the test agree without a shared constant
/// value and without a spec-assigned number.
pub const CHALLENGE_MESSAGE_NAME: &str = "RustampGenFixtureChallenge";

/// Window class of the fixture's own window, created in `init`.
///
/// The test finds it by class name and process id, which is the "independent
/// guest observation" P3 §5.5 asks for: the test does not need the host to
/// hand it a handle.
pub const FIXTURE_WINDOW_CLASS: &str = "Rustamp Gen Fixture";

/// Window class of the fixture's configuration window, created in `config`.
pub const FIXTURE_CONFIG_CLASS: &str = "Rustamp Gen Fixture Config";

/// File name prefix of the fixture's own trace file.
///
/// The trace lives in the process temp directory and is named after the
/// process id, so the test and the fixture name the same file without a
/// handshake. It is written **from inside the plugin**, so it is
/// plugin-side evidence that the host or the runner cannot forge.
pub const TRACE_FILE_PREFIX: &str = "rustamp-gen-fixture-";

/// Extension of the fixture's trace file.
pub const TRACE_FILE_EXTENSION: &str = "log";

/// The IPC id the fixture uses for the "unknown id" probe (P3 §4 step 7).
///
/// Nothing assigns this number: it is simply an id no host answers, so the
/// reply shows the host's convention for unsupported ids.
pub const UNKNOWN_IPC_ID: u32 = 9999;

/// Challenge: echo the argument back unchanged.
///
/// The answer proves that the message reached the *plugin's own* window
/// procedure (P3 §4 step 8) and not the host's.
pub const CMD_ECHO: usize = 1;

/// Challenge: the parent window handle the host filled in before `init`.
pub const CMD_PARENT_HWND: usize = 2;

/// Challenge: the fixture's own window handle.
pub const CMD_OWN_HWND: usize = 3;

/// Challenge: the configuration window handle, 0 while none is open.
pub const CMD_CONFIG_HWND: usize = 4;

/// Challenge: run the version query **from the plugin's own code** and report
/// the raw 32-bit reply.
pub const CMD_IPC_VERSION_REPLY: usize = 5;

/// Challenge: run the unknown-id query **from the plugin's own code** and
/// report the raw 32-bit reply.
pub const CMD_IPC_UNKNOWN_REPLY: usize = 6;

/// Challenge: the raw `int` the fixture's `init` returned.
pub const CMD_INIT_RESULT: usize = 7;

/// Challenge: the subclass state — 0 never installed, 1 installed, 2 restored.
pub const CMD_SUBCLASS_STATE: usize = 8;

/// Challenge: how many messages the subclass forwarded to the host's original
/// window procedure.
pub const CMD_SUBCLASS_ENTRIES: usize = 9;

/// Challenge: 1 once the fixture's `quit` ran.
pub const CMD_QUIT_CALLED: usize = 10;

/// Challenge: 1 when the parent window's class was `Winamp v1.x`.
pub const CMD_PARENT_CLASS_OK: usize = 11;
