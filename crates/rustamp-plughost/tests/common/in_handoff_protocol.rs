//! The private trace protocol between the **hand-over** input fixture and the
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
//! (`examples/in_handoff_rustamp_fixture.rs`) and the test
//! (`tests/handoff_acceptance.rs`) — so the two cannot drift apart silently.
//!
//! Every item here is used by **both** sides: the fixture writes the lines
//! and the test reads them, and each side is its own compilation unit, so an
//! item only one side used would be dead code there.
//!
//! # What the protocol fixes and what it does not
//!
//! The lines that report what the **input fixture chose** — the stream
//! parameters, the frame size, the frame count — are fixed in full, because
//! the test can predict them. The lines that report what the **output
//! plug-in answered** — the free space, the playing state, the output's own
//! clock — carry only a prefix: those values belong to the output plug-in,
//! not to this fixture, so the protocol fixes the wording and the test
//! asserts that the call happened, not what the output said.

/// File name prefix of the fixture's own trace file.
pub const TRACE_FILE_PREFIX: &str = "rustamp-in-handoff-fixture-";

/// Extension of the fixture's trace file.
pub const TRACE_FILE_EXTENSION: &str = "log";

/// The description the fixture publishes (S09 §3.1 offset 0x04).
///
/// NUL-terminated, because the ABI member is a pointer to an ANSI string.
pub const FIXTURE_DESCRIPTION: &[u8] = b"Rustamp in handoff fixture\0";

/// The sample rate the fixture announces to the output (S09 §3.7).
pub const FIXTURE_SAMPLE_RATE: i32 = 44_100;

/// The channel count the fixture announces to the output (S09 §3.7).
pub const FIXTURE_CHANNELS: i32 = 2;

/// The bit depth the fixture announces to the output (S09 §3.7).
pub const FIXTURE_BITS: i32 = 16;

/// The buffer size the fixture asks the output for; 0 means the output's own
/// default (S09 §3.7).
pub const FIXTURE_BUFFER_MS: i32 = 0;

/// The prebuffer size the fixture asks the output for; 0 means the output's
/// own default (S09 §3.7).
pub const FIXTURE_PREBUFFER_MS: i32 = 0;

/// The bytes of one silent frame: 16-bit stereo (S09 §3.7).
pub const FIXTURE_FRAME_BYTES: usize = 4;

/// The number of silent frames the fixture writes into the output.
pub const FIXTURE_FRAMES: usize = 16;

/// Trace prefix of the `init` callback.
pub const EVENT_INIT: &str = "init ";

/// The trace line the fixture writes when `init` runs.
pub fn init_line(parent: usize, instance: usize) -> String {
    format!("{EVENT_INIT}parent={parent:#x} instance={instance:#x}")
}

/// The trace line the fixture writes about the class of the window the host
/// handed it (S09 §1).
pub fn parent_class_line(ok: bool) -> String {
    format!("parent class ok={}", u8::from(ok))
}

/// Trace prefix of the host-slots line.
pub const EVENT_HOST_SLOTS: &str = "host slots ";

/// The trace line the fixture writes about the host members it found.
///
/// `analyzer`, `vis` and `dsp` are the three host callback groups
/// (S09 §3.1 offsets 0x60–0x70, 0x74–0x80 and 0x84–0x88), `set_info` the
/// stream-info callback (offset 0x90) and `out_module` the selected output
/// (offset 0x94). The fixture writes this line twice: once from `init`,
/// where `set_info` must still be empty because the host clears it before
/// `init` (S09 §3.5 line 154) and where `out_module` is still empty because
/// the hand-over happens later, and once from `play`, where the host must
/// have filled `set_info` (S09 §3.5 line 155) and written the selected
/// output into `out_module` (S09 §3.5 step 2).
pub fn host_slots_line(
    analyzer: bool,
    vis: bool,
    dsp: bool,
    set_info: bool,
    out_module: bool,
) -> String {
    format!(
        "{EVENT_HOST_SLOTS}analyzer={} vis={} dsp={} set_info={} out_module={}",
        u8::from(analyzer),
        u8::from(vis),
        u8::from(dsp),
        u8::from(set_info),
        u8::from(out_module),
    )
}

/// Trace prefix of the `play` callback.
pub const EVENT_PLAY: &str = "play file=";

/// The trace line the fixture writes when `play` runs.
pub fn play_line(file: &str) -> String {
    format!("{EVENT_PLAY}{file}")
}

/// Trace prefix of the `config` callback.
pub const EVENT_CONFIG: &str = "config ";

/// The trace line the fixture writes when `config` runs.
pub fn config_line(parent: usize) -> String {
    format!("{EVENT_CONFIG}parent={parent:#x}")
}

/// Trace prefix of the `quit` callback.
pub const EVENT_QUIT: &str = "quit";

/// Trace prefix of the line the fixture writes when it opens the output
/// plug-in (S09 §3.7).
pub const EVENT_OUT_OPEN: &str = "out open ";

/// The trace line the fixture writes when it opens the output plug-in.
pub fn out_open_line(
    rate: i32,
    channels: i32,
    bits: i32,
    buffer_ms: i32,
    prebuffer_ms: i32,
) -> String {
    format!(
        "{EVENT_OUT_OPEN}rate={rate} channels={channels} bits={bits} buffer={buffer_ms} \
         prebuffer={prebuffer_ms}"
    )
}

/// Trace prefix of the line the fixture writes when it asks the output plug-in
/// how much room it has (S09 §3.5 step 4).
///
/// Only the prefix is fixed: the free space is the output plug-in's answer.
pub const EVENT_OUT_CAN_WRITE: &str = "out can_write ";

/// Trace prefix of the line the fixture writes when it writes a frame into the
/// output plug-in (S09 §3.5 step 4).
pub const EVENT_OUT_WRITE: &str = "out write ";

/// The trace line the fixture writes when it writes a frame into the output
/// plug-in.
pub fn out_write_line(len: i32) -> String {
    format!("{EVENT_OUT_WRITE}len={len}")
}

/// Trace prefix of the line the fixture writes when it asks the output plug-in
/// whether it still plays (S09 §3.5 step 5).
///
/// Only the prefix is fixed: the answer is the output plug-in's.
pub const EVENT_OUT_IS_PLAYING: &str = "out is_playing ";

/// Trace prefix of the line the fixture writes when it reads the output
/// plug-in's clock (S09 §3.7).
///
/// Only the prefix is fixed: the times are the output plug-in's.
pub const EVENT_OUT_TIME: &str = "out time ";

/// Trace prefix of the line the fixture writes when it closes the output
/// plug-in.
pub const EVENT_OUT_CLOSE: &str = "out close";

/// Trace prefix of the line the fixture writes about the end-of-track
/// notification (S09 §3.6).
pub const EVENT_END_OF_TRACK: &str = "end of track ";

/// The trace line the fixture writes about the end-of-track notification.
///
/// `posted` says whether the notification really reached the host's message
/// queue (S09 §3.6).
pub fn end_of_track_line(posted: bool) -> String {
    format!("{EVENT_END_OF_TRACK}posted={}", u8::from(posted))
}
