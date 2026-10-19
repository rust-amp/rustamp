//! The private trace protocol between the input fixture and the acceptance
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
//! (`examples/in_rustamp_fixture.rs`) and the test
//! (`tests/in_acceptance.rs`) — so the two cannot drift apart silently.
//!
//! Every item here is used by **both** sides: the fixture writes the lines
//! and the test reads them, and each side is its own compilation unit, so an
//! item only one side used would be dead code there.

/// File name prefix of the fixture's own trace file.
pub const TRACE_FILE_PREFIX: &str = "rustamp-in-fixture-";

/// Extension of the fixture's trace file.
pub const TRACE_FILE_EXTENSION: &str = "log";

/// The description the fixture publishes (S09 §3.1 offset 0x04).
///
/// NUL-terminated, because the ABI member is a pointer to an ANSI string.
pub const FIXTURE_DESCRIPTION: &[u8] = b"Rustamp in fixture\0";

/// The track length the fixture reports (S09 §3.1 offset 0x4C).
pub const FIXTURE_LENGTH_MS: i32 = 30_000;

/// The sample rate the fixture announces (S09 §3.1 offset 0x60, §3.8).
pub const FIXTURE_SAMPLE_RATE: i32 = 44_100;

/// The channel count the fixture announces (S09 §3.1 offset 0x80, §3.8).
pub const FIXTURE_CHANNELS: i32 = 2;

/// The sample rate in kHz the fixture reports through `set_info`
/// (S09 §3.1 offset 0x90: the sample rate is in kHz).
pub const FIXTURE_SAMPLE_RATE_KHZ: i32 = 44;

/// The bitrate the fixture reports through `set_info` (S09 §3.1 offset 0x90).
pub const FIXTURE_BITRATE_KBPS: i32 = 128;

/// The latency the fixture reports to `analyzer_init` (S09 §3.1 offset 0x60).
///
/// Zero: the fixture has flag bit 0 clear, so it drives no output plugin and
/// has no output latency to hand over (S09 §3.3, §3.7).
pub const FIXTURE_LATENCY_MS: i32 = 0;

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
/// `init` (S09 §3.5 line 154), and once from `play`, where the host must
/// have filled it (S09 §3.5 line 155).
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

/// Trace prefix of the line the fixture writes when it pauses or resumes.
pub const EVENT_PAUSED: &str = "playback paused=";

/// The trace line the fixture writes when it pauses or resumes.
pub fn paused_line(paused: bool) -> String {
    format!("{EVENT_PAUSED}{}", u8::from(paused))
}

/// Trace prefix of the `stop` callback.
pub const EVENT_STOP: &str = "playback stopped";

/// Trace prefix of the `analyzer_init` host callback.
pub const EVENT_ANALYZER_INIT: &str = "analyzer_init ";

/// The trace line the fixture writes when it calls the host's `analyzer_init`
/// (S09 §3.1 offset 0x60).
pub fn analyzer_init_line(latency_ms: i32, sample_rate: i32) -> String {
    format!("{EVENT_ANALYZER_INIT}latency={latency_ms} rate={sample_rate}")
}

/// Trace prefix of the `analyzer_deinit` host callback.
pub const EVENT_ANALYZER_DEINIT: &str = "analyzer_deinit";

/// Trace prefix of the `vis_set_format` host callback.
pub const EVENT_VIS_SET_FORMAT: &str = "vis_set_format ";

/// The trace line the fixture writes when it calls the host's `vis_set_format`
/// (S09 §3.1 offset 0x80).
pub fn vis_set_format_line(sample_rate: i32, channels: i32) -> String {
    format!("{EVENT_VIS_SET_FORMAT}rate={sample_rate} channels={channels}")
}

/// Trace prefix of the `set_info` host callback.
pub const EVENT_SET_INFO: &str = "set_info ";

/// The trace line the fixture writes when it calls the host's `set_info`
/// (S09 §3.1 offset 0x90).
pub fn set_info_line(
    bitrate_kbps: i32,
    sample_rate_khz: i32,
    channels: i32,
    synched: i32,
) -> String {
    format!(
        "{EVENT_SET_INFO}bitrate={bitrate_kbps} rate={sample_rate_khz} channels={channels} \
         synched={synched}"
    )
}

/// Trace prefix of the `config` callback.
pub const EVENT_CONFIG: &str = "config ";

/// The trace line the fixture writes when `config` runs.
pub fn config_line(parent: usize) -> String {
    format!("{EVENT_CONFIG}parent={parent:#x}")
}

/// Trace prefix of the `quit` callback.
pub const EVENT_QUIT: &str = "quit";

/// Trace prefix of the `about` callback. The host has no route to it.
pub const EVENT_ABOUT: &str = "about ";

/// Trace prefix of the `set_volume` callback.
///
/// The host has no path from the player's volume to a classic plugin yet, so
/// the slot stays untouched (S09 §3.1 offset 0x58).
pub const EVENT_SET_VOLUME: &str = "set_volume ";

/// Trace prefix of the `set_pan` callback.
///
/// The host has no path from the player's balance to a classic plugin yet, so
/// the slot stays untouched (S09 §3.1 offset 0x5C).
pub const EVENT_SET_PAN: &str = "set_pan ";

/// Trace prefix of the `eq_set` callback.
///
/// The host has no path from the player's equalizer to a classic plugin yet,
/// so the slot stays untouched (S09 §3.1 offset 0x8C).
pub const EVENT_EQ_SET: &str = "eq_set ";

/// Trace prefix of the `seek` callback.
///
/// The host has no seek path into a classic plugin yet, so the slot stays
/// untouched (S09 §3.1 offset 0x54).
pub const EVENT_SEEK: &str = "seek ";

/// Trace prefix of the `get_file_info` callback.
///
/// The host has no module-selection path yet, so the slot stays untouched
/// (S09 §3.1 offset 0x2C, §3.5 step 1).
pub const EVENT_FILE_INFO: &str = "get_file_info ";

/// Trace prefix of the `info_box` callback.
///
/// The host has no file-info path yet, so the slot stays untouched
/// (S09 §3.1 offset 0x30).
pub const EVENT_INFO_BOX: &str = "info_box ";

/// Trace prefix of the `is_our_file` callback.
///
/// The host has no module-selection path yet, so the slot stays untouched
/// (S09 §3.1 offset 0x34, §3.5 step 1).
pub const EVENT_IS_OUR_FILE: &str = "is_our_file ";
