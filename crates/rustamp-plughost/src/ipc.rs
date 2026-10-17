//! Table-driven answering surface for the classic `WM_WA_IPC` (`WM_USER`)
//! messages measured by spec S10 (draft `S10-ipc.md`, section 3.7).
//!
//! One row lives in [`IPC_TABLE`] for each id S10 measured. The reply and the
//! evidence (run and job step) travel together, so the surface can be audited
//! against the measured table row by row. An id that is not in the table is not
//! silently answered: it falls through to the S10 convention of `1`.
//!
//! # Conventions implemented (S10 section 1.2, 3.8, 3.9)
//!
//! * `IPC_GETVERSION` (0) returns [`WINAMP_VERSION_295`] = `0x2905`
//!   (RFC 0009), not the SDK-derived `0x2095`.
//! * An unknown or unsupported id returns `1`.
//! * A reply is a 32-bit value. A negative 32-bit reply is carried as its
//!   low 32 bits (see [`u32_from_i32`]); the harness records exactly that form.
//! * Volume (`IPC_SETVOLUME`, 122) is unsigned `0..=255`.
//! * Pan (`IPC_SETPANNING`, 123) is signed; setting 255 reads back `-1`,
//!   i.e. `0xFFFFFFFF` as an unsigned 32-bit value.
//! * The equalizer read/write pair (127, 128) is stateful: the write lands on
//!   the band/preamp/enabled/autoload selector last chosen by a 127 query.
//! * Shuffle (250/252) and repeat (251/253) echo `1`/`0`.
//! * Ids whose reply depends on a playing reference are flagged
//!   ([`IpcRow::requires_playing_reference`]) and never get an invented value.

use rustamp_plugin_abi::common::WINAMP_VERSION_295;

/// Convenience: interpret a signed `i32` reply as the unsigned 32-bit value the
/// `WM_WA_IPC` result carries (zero extension, same shape as the measured
/// reply strings).
pub const fn u32_from_i32(value: i32) -> u32 {
    value as u32
}

/// Convenience: interpret the unsigned 32-bit value as a signed `i32`.
pub const fn i32_from_u32(value: u32) -> i32 {
    value as i32
}

/// Bands `0..=9`, index 10 = preamp, 11 = enabled, 12 = autoload.
pub const EQ_SLOT_COUNT: usize = 13;
pub const EQ_BAND_COUNT: usize = 10;

/// S10 convention for any id the surface does not recognise or support.
pub const UNKNOWN_REPLY: u32 = 1;

/// Which of the measured shapes the reply has. This is documentation for the
/// row; it is not the answer logic (see [`IpcSurface`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasuredReply {
    /// Single numeric value measured by S10.
    Fixed(u32),
    /// Depends on the `wParam` mode selector (output time 105, stream info 126,
    /// EQ data 127).
    ModeDependent,
    /// A window handle supplied by the host at runtime; measured proof only
    /// says the value is nonzero and distinct (260).
    RuntimeHandle,
    /// A function/data pointer measured as stable across launches but whose
    /// contents need an in-process caller (800).
    PluginPointer,
    /// State is carried inside [`IpcSurface`] (volume 122).
    VolumeUnsigned,
    /// State is carried inside [`IpcSurface`] (pan 123).
    PanSigned,
    /// Stateful equalizer read/write pair.
    StatefulEq,
    /// Toggle read-back (shuffle 250/252, repeat 251/253).
    StatefulToggle,
    /// Posted command, no reply read by the harness.
    PostedNoReply,
    /// Needs a playing reference before it can be answered (105, 106, 126,
    /// 501); the row carries no invented value.
    PlayingReference,
    /// Matches the S10 "unsupported id -> 1" convention.
    UnsupportedConvention,
}

/// One row of the measured S10 id table. The `evidence_run` and `evidence_step`
/// fields name the run directory and the job step the value rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IpcRow {
    /// IPC number (the `LPARAM`/second parameter of `WM_WA_IPC`).
    pub id: u32,
    /// Public name from the S10 draft.
    pub name: &'static str,
    /// Shape recorded by S10.
    pub measured: MeasuredReply,
    /// Measured numeric value, where stating one is honest. `None` for
    /// per-boot handles, for stateful results and for ids that need a playing
    /// reference.
    pub measured_value: Option<u32>,
    /// Whether the value only becomes known when a track is playing.
    pub requires_playing_reference: bool,
    /// Whether confirming the value needs a plugin DLL in process.
    pub requires_plugin_dll: bool,
    /// Whether the value lives in [`IpcSurface`] state rather than in the table.
    pub stateful: bool,
    /// Run tag: R1, R2, R3, B1, B2, B3.
    pub evidence_run: &'static str,
    /// Job step name the reply was read from.
    pub evidence_step: &'static str,
    /// Short note, including open points from S10 section 5.
    pub note: &'static str,
}

/// The single place that maps a measured IPC id to its reply and its evidence.
pub const IPC_TABLE: &[IpcRow] = &[
    IpcRow {
        id: 0,
        name: "IPC_GETVERSION",
        measured: MeasuredReply::Fixed(WINAMP_VERSION_295),
        measured_value: Some(WINAMP_VERSION_295),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "R1/R2/R3",
        evidence_step: "s10_000_getversion",
        note: "Measured 0x2905 in three runs; contradicts S09's 0x2095 (RFC 0009).",
    },
    IpcRow {
        id: 1,
        name: "IPC? (undefined probe)",
        measured: MeasuredReply::UnsupportedConvention,
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "R2/R3",
        evidence_step: "s10_unknown_1",
        note: "Undefined id answers 1, not 0 (refutes S09 U-6).",
    },
    IpcRow {
        id: 101,
        name: "IPC_DELETE",
        measured: MeasuredReply::PostedNoReply,
        measured_value: None,
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "B3",
        evidence_step: "del_action",
        note: "Main-window artefact; effect unmeasured (S10 5.15).",
    },
    IpcRow {
        id: 102,
        name: "IPC_STARTPLAY",
        measured: MeasuredReply::PostedNoReply,
        measured_value: None,
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "B3",
        evidence_step: "play_action",
        note: "Effect measured (+359 px); reply unread because step posted. Host reports CommandEffect without triggering engine playback (missing reverse channel, S10 2, 11).",
    },
    IpcRow {
        id: 104,
        name: "IPC_ISPLAYING",
        measured: MeasuredReply::Fixed(0),
        measured_value: Some(0),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3/B2",
        evidence_step: "s10_104_isplaying",
        note: "0 stopped, 1 playing, 3 paused (S09). Argument ignored.",
    },
    IpcRow {
        id: 105,
        name: "IPC_GETOUTPUTTIME",
        measured: MeasuredReply::ModeDependent,
        measured_value: Some(4_294_967_295),
        requires_playing_reference: true,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_105_time_mode0",
        note: "Mode 0/1 -> 0xFFFFFFFF (-1), mode 2 -> 0 while stopped.",
    },
    IpcRow {
        id: 106,
        name: "IPC_JUMPTOTIME",
        measured: MeasuredReply::PlayingReference,
        measured_value: Some(4_294_967_295),
        requires_playing_reference: true,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "B3",
        evidence_step: "seek_action",
        note: "Reply -1 while stopped; other returns need playing (S10 5.17).",
    },
    IpcRow {
        id: 109,
        name: "IPC_GETMODULENAME",
        measured: MeasuredReply::Fixed(0),
        measured_value: Some(0),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "R2/R3",
        evidence_step: "s10_109_getmodulename",
        note: "Unsupported or no module loaded; pointer contents open (S10 5.13).",
    },
    IpcRow {
        id: 121,
        name: "IPC_SETPLAYLISTPOS",
        measured: MeasuredReply::PostedNoReply,
        measured_value: None,
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "B3",
        evidence_step: "pos_action",
        note: "Ambiguous main-window artefact (S10 5.16).",
    },
    IpcRow {
        id: 122,
        name: "IPC_SETVOLUME",
        measured: MeasuredReply::VolumeUnsigned,
        measured_value: Some(0),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_122_volume_query",
        note: "Query -666 -> 0; read-backs 64, 0, 255. Volume is unsigned 0..=255 (S10 3.9f). Sent setter return value unmeasured (S10 0.2 E4, 13.11).",
    },
    IpcRow {
        id: 123,
        name: "IPC_SETPANNING",
        measured: MeasuredReply::PanSigned,
        measured_value: Some(0),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_123_pan_query",
        note: "Query -666 -> 0; setting 255 reads back -1 (0xFFFFFFFF). Pan is signed (S10 3.9f). Sent setter return value unmeasured (S10 0.2 E4, 13.11).",
    },
    IpcRow {
        id: 124,
        name: "IPC_GETLISTLENGTH",
        measured: MeasuredReply::Fixed(0),
        measured_value: Some(0),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3/B2",
        evidence_step: "s10_124_getlistlength",
        note: "0 fresh, 60 with the loaded list (B2).",
    },
    IpcRow {
        id: 125,
        name: "IPC_GETLISTPOS",
        measured: MeasuredReply::Fixed(0),
        measured_value: Some(0),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_125_getlistpos",
        note: "Moves by one on next/prev (S02).",
    },
    IpcRow {
        id: 126,
        name: "IPC_GETINFO",
        measured: MeasuredReply::ModeDependent,
        measured_value: Some(0),
        requires_playing_reference: true,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_126_info_mode0",
        note: "Modes 0/1/2 all 0 while stopped. Playing sample rate unit (raw Hz vs kHz) unmeasured (S10 13.2).",
    },
    IpcRow {
        id: 127,
        name: "IPC_GETEQDATA",
        measured: MeasuredReply::ModeDependent,
        measured_value: Some(31),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_127_eq_band0",
        note: "Bands/preamp 31, enabled 1, autoload 1, selector 99 -> 0 (stateful read/write with 128).",
    },
    IpcRow {
        id: 128,
        name: "IPC_SETEQDATA",
        measured: MeasuredReply::StatefulEq,
        measured_value: None,
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R3/B1",
        evidence_step: "s10_eq_write_band0_0",
        note: "127 selects, 128 writes, read back 0; +431 px redraw (S10 3.9g).",
    },
    IpcRow {
        id: 135,
        name: "IPC_RESTARTWINAMP",
        measured: MeasuredReply::PostedNoReply,
        measured_value: None,
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "B3",
        evidence_step: "restart_action",
        note: "Capture failed while the process restarted (exit 3). Host process does not restart player (missing protocol channel, S10 2, 11).",
    },
    IpcRow {
        id: 242,
        name: "IPC_INETAVAILABLE",
        measured: MeasuredReply::Fixed(1),
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_242_inetavailable",
        note: "Provisional: sandbox-networking-dependent; not a fixture property (S10 3, 13.7).",
    },
    IpcRow {
        id: 250,
        name: "IPC_GET_SHUFFLE",
        measured: MeasuredReply::StatefulToggle,
        measured_value: Some(0),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_250_get_shuffle",
        note: "Read-backs 1 then 0 after setting.",
    },
    IpcRow {
        id: 251,
        name: "IPC_GET_REPEAT",
        measured: MeasuredReply::StatefulToggle,
        measured_value: Some(0),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_251_get_repeat",
        note: "Read-backs 1 then 0 after setting.",
    },
    IpcRow {
        id: 252,
        name: "IPC_SET_SHUFFLE",
        measured: MeasuredReply::StatefulToggle,
        measured_value: None,
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_setshuffle_1",
        note: "Posted; effect observed via 250. -666 query and sent setter return are unmeasured (S10 3, 6).",
    },
    IpcRow {
        id: 253,
        name: "IPC_SET_REPEAT",
        measured: MeasuredReply::StatefulToggle,
        measured_value: None,
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_setrepeat_1",
        note: "Posted; effect observed via 251. -666 query and sent setter return are unmeasured (S10 3, 6).",
    },
    IpcRow {
        id: 260,
        name: "IPC_GETWND",
        measured: MeasuredReply::RuntimeHandle,
        measured_value: None,
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_260_getwnd_eq",
        note: "Real distinct per-boot handles (EQ 66262, PE 66264, video 66266, MB 66268/66270). Host returns 0 pending M3 handle ownership decision (S10 13.8).",
    },
    IpcRow {
        id: 261,
        name: "IPC_ISWNDVISIBLE",
        measured: MeasuredReply::Fixed(1),
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3/B2",
        evidence_step: "s10_261_iswndvisible_eq",
        note: "1 for EQ, playlist and main.",
    },
    IpcRow {
        id: 281,
        name: "IPC_GET_HMENU",
        measured: MeasuredReply::RuntimeHandle,
        measured_value: None,
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_281_get_hmenu",
        note: "Non-null menu handle per boot (262789, 328057). Host returns 0 pending M3 menu ownership decision (S10 13.8).",
    },
    IpcRow {
        id: 400,
        name: "IPC_ISFULLSTOP",
        measured: MeasuredReply::Fixed(0),
        measured_value: Some(0),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_400_isfullstop",
        note: "2.05+ id.",
    },
    IpcRow {
        id: 501,
        name: "IPC_IS_PLAYING_VIDEO",
        measured: MeasuredReply::PlayingReference,
        measured_value: Some(1),
        requires_playing_reference: true,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_501_is_playing_video",
        note: "Measured 1 with no video (\"old host\" value).",
    },
    IpcRow {
        id: 502,
        name: "IPC_HAS_VIDEO_SUPPORT",
        measured: MeasuredReply::Fixed(1),
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "R2/R3",
        evidence_step: "s10_502_has_video_support",
        note: "Job name [JOB]; settled as 1.",
    },
    IpcRow {
        id: 666,
        name: "IPC_EX_ISRIGHTEXE",
        measured: MeasuredReply::Fixed(1),
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "R2/R3",
        evidence_step: "s10_666_isrightexe",
        note: "1.666+ id.",
    },
    IpcRow {
        id: 800,
        name: "IPC_GETSADATAFUNC",
        measured: MeasuredReply::PluginPointer,
        measured_value: Some(4_407_808),
        requires_playing_reference: false,
        requires_plugin_dll: true,
        stateful: false,
        evidence_run: "R2/R3/B2",
        evidence_step: "s10_800_sadatafunc",
        note: "Reference-only address 0x00434200 recorded across three launches; never a host runtime pointer. Runtime unsupported (reply 1) until a host-owned implementation is audited (S10 5.3, 8.3, 13.3).",
    },
    IpcRow {
        id: 801,
        name: "IPC_GETVUDATAFUNC",
        measured: MeasuredReply::Fixed(1),
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "R2/R3",
        evidence_step: "s10_801_vudatafunc",
        note: "Unsupported (5.0+ id).",
    },
    IpcRow {
        id: 900,
        name: "IPC_ISMAINWNDVISIBLE",
        measured: MeasuredReply::Fixed(1),
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: true,
        evidence_run: "R2/R3",
        evidence_step: "s10_900_ismainwndvisible",
        note: "Job name [JOB].",
    },
    IpcRow {
        id: 999,
        name: "IPC_IS_SAFEMODE",
        measured: MeasuredReply::Fixed(1),
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "R2/R3",
        evidence_step: "s10_999_is_safemode",
        note: "Job name [JOB].",
    },
    IpcRow {
        id: 3015,
        name: "IPC_GET_RANDFUNC",
        measured: MeasuredReply::Fixed(1),
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: true,
        stateful: false,
        evidence_run: "R2/R3",
        evidence_step: "s10_3015_randfunc",
        note: "Unsupported (5.1+ id).",
    },
    IpcRow {
        id: 3025,
        name: "IPC_GET_API_SERVICE",
        measured: MeasuredReply::Fixed(1),
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: true,
        stateful: false,
        evidence_run: "R2/R3",
        evidence_step: "s10_3025_api_service",
        note: "Unsupported matches the 1 = unsupported convention.",
    },
    IpcRow {
        id: 65536,
        name: "IPC_REGISTER_WINAMP_IPCMESSAGE",
        measured: MeasuredReply::Fixed(1),
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "R2/R3",
        evidence_step: "s10_65536_register",
        note: "Unsupported (5.0+ id).",
    },
    IpcRow {
        id: 9999,
        name: "(undefined probe)",
        measured: MeasuredReply::UnsupportedConvention,
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "R2/R3",
        evidence_step: "s10_unknown_9999",
        note: "Undefined id answers 1.",
    },
    IpcRow {
        id: 2_147_483_647,
        name: "(out-of-range probe)",
        measured: MeasuredReply::UnsupportedConvention,
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "R2/R3",
        evidence_step: "s10_unknown_max",
        note: "Out-of-range id answers 1.",
    },
    IpcRow {
        id: u32::MAX,
        name: "(negative id probe)",
        measured: MeasuredReply::UnsupportedConvention,
        measured_value: Some(1),
        requires_playing_reference: false,
        requires_plugin_dll: false,
        stateful: false,
        evidence_run: "R2/R3",
        evidence_step: "s10_unknown_neg",
        note: "Negative id answers 1 (id sent as 0xFFFFFFFF).",
    },
];

/// Look up a measured IPC row by id.
pub fn lookup_table(id: u32) -> Option<&'static IpcRow> {
    IPC_TABLE.iter().find(|row| row.id == id)
}

/// Sorted iterator over the table.
pub fn table() -> &'static [IpcRow] {
    IPC_TABLE
}

/// Reply returned by [`IpcSurface::answer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcAnswer {
    /// The surface produced a raw 32-bit reply.
    Value(u32),
    /// Id is not in the table (or has no fixed reply); S10 convention => 1.
    Unknown(u32),
    /// The reply needs a playing reference and was not invented.
    NeedsPlayingReference {
        /// The id that was queried.
        id: u32,
    },
    /// The reply needs a plugin DLL in process.
    NeedsPluginDll {
        /// The id that was queried.
        id: u32,
    },
    /// The query landed as a posted command with no reply read.
    CommandEffect {
        /// The id that was queried.
        id: u32,
    },
    /// A host-supplied runtime handle was returned.
    RuntimeHandle {
        /// The id that was queried.
        id: u32,
        /// The handle value, `0` if the host never supplied one.
        handle: u32,
    },
}

impl IpcAnswer {
    /// The raw 32-bit value an out-of-process sender would observe.
    pub fn raw(&self) -> u32 {
        match *self {
            IpcAnswer::Value(v) | IpcAnswer::Unknown(v) => v,
            IpcAnswer::NeedsPlayingReference { .. }
            | IpcAnswer::NeedsPluginDll { .. }
            | IpcAnswer::CommandEffect { .. } => UNKNOWN_REPLY,
            IpcAnswer::RuntimeHandle { handle, .. } => handle,
        }
    }

    /// `true` when the reply follows the unsupported-id convention.
    pub fn is_unknown(&self) -> bool {
        matches!(self, IpcAnswer::Unknown(_))
    }
}

/// Call shape used by the rest of the crate.
pub type Answer = IpcAnswer;

/// State held by the emulated Winamp main window. Defaults match the fresh
/// reference state (no playlist, stopped).
#[derive(Debug, Clone)]
pub struct IpcSurface {
    /// 0 stopped, 1 playing, 3 paused (S10 / IPC_ISPLAYING).
    pub is_playing: u32,
    /// Playlist length (`IPC_GETLISTLENGTH`).
    pub playlist_len: u32,
    /// Playlist position (`IPC_GETLISTPOS`).
    pub playlist_pos: u32,
    /// Last output position in milliseconds (`IPC_GETOUTPUTTIME` mode 0).
    pub output_position_ms: u32,
    /// Stream length in milliseconds (`IPC_GETOUTPUTTIME` mode 1/2).
    pub output_length_ms: u32,
    /// Stream info sample rate (`IPC_GETINFO` mode 0).
    pub sample_rate: u32,
    /// Stream info bitrate in kbps (`IPC_GETINFO` mode 1).
    pub bitrate: u32,
    /// Stream info channel count (`IPC_GETINFO` mode 2).
    pub channels: u32,
    /// Volume (`IPC_SETVOLUME`), unsigned, 0..=255.
    pub volume: u8,
    /// Pan stored as a signed value (-127..=127 class, here full i32 for the
    /// measured -1 case).
    pub pan: i32,
    /// Shuffle (`IPC_GET_SHUFFLE` / `IPC_SET_SHUFFLE`).
    pub shuffle: bool,
    /// Repeat (`IPC_GET_REPEAT` / `IPC_SET_REPEAT`).
    pub repeat: bool,
    /// Executed `WM_USER` state for the equalizer.
    pub eq_selected: Option<u32>,
    pub eq_bands: [i32; EQ_BAND_COUNT],
    pub eq_preamp: i32,
    pub eq_enabled: bool,
    pub eq_autoload: bool,
    pub visible_eq: bool,
    pub visible_playlist: bool,
    pub visible_main: bool,
    pub visible_minibrowser: bool,
    pub visible_video: bool,
    pub eq_hwnd: u32,
    pub playlist_hwnd: u32,
    pub minibrowser_hwnd: u32,
    pub video_hwnd: u32,
    pub main_hwnd: u32,
    pub menu_handle: u32,
    pub inet_available: bool,
}

impl Default for IpcSurface {
    fn default() -> Self {
        Self {
            is_playing: 0,
            playlist_len: 0,
            playlist_pos: 0,
            output_position_ms: 0,
            output_length_ms: 0,
            sample_rate: 0,
            bitrate: 0,
            channels: 0,
            volume: 0,
            pan: 0,
            shuffle: false,
            repeat: false,
            eq_selected: None,
            eq_bands: [31; EQ_BAND_COUNT],
            eq_preamp: 31,
            eq_enabled: true,
            eq_autoload: true,
            visible_eq: true,
            visible_playlist: true,
            visible_main: true,
            visible_minibrowser: false,
            visible_video: false,
            eq_hwnd: 0,
            playlist_hwnd: 0,
            minibrowser_hwnd: 0,
            video_hwnd: 0,
            main_hwnd: 0,
            menu_handle: 0,
            inet_available: true,
        }
    }
}

impl IpcSurface {
    /// Fresh reference state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Volume query or set, returning the raw 32-bit reply.
    pub fn volume_unsigned(&self) -> u32 {
        self.volume as u32
    }

    /// Pan query, as the signed 32-bit value carried unsigned.
    pub fn pan_unsigned(&self) -> u32 {
        self.pan as u32
    }

    /// Look up a measured row without touching state.
    pub fn lookup(&self, id: u32) -> Option<&'static IpcRow> {
        lookup_table(id)
    }

    /// Answer one `WM_WA_IPC` query against the emulated window state.
    pub fn answer(&mut self, id: u32, wparam: i64) -> IpcAnswer {
        match id {
            0 => IpcAnswer::Value(WINAMP_VERSION_295),
            // Mutating IPCs (102 IPC_STARTPLAY, 135 IPC_RESTARTWINAMP) and playlist actions (101, 121).
            // Finding 3: The host returns CommandEffect { id } (raw() == 1), but causes no effect
            // in the player engine and no restart of the process because no reverse command channel
            // is ratified across rustamp-ipc (S10 §2, §11, RFC 0005 O13).
            101 | 121 | 102 | 135 => IpcAnswer::CommandEffect { id },
            104 => IpcAnswer::Value(self.is_playing),
            105 => {
                let mode = wparam as i32;
                if self.is_playing == 0 {
                    match mode {
                        0 | 1 => IpcAnswer::Value(4_294_967_295),
                        2 => IpcAnswer::Value(0),
                        _ => IpcAnswer::Value(0),
                    }
                } else {
                    match mode {
                        0 => IpcAnswer::Value(self.output_position_ms),
                        1 => IpcAnswer::Value(self.output_length_ms / 1000),
                        2 => IpcAnswer::Value(self.output_length_ms),
                        _ => IpcAnswer::Value(0),
                    }
                }
            }
            106 => {
                if self.is_playing == 0 {
                    IpcAnswer::Value(4_294_967_295)
                } else {
                    IpcAnswer::NeedsPlayingReference { id }
                }
            }
            109 => IpcAnswer::Value(0),
            122 => {
                if wparam == -666 {
                    IpcAnswer::Value(self.volume as u32)
                } else {
                    let v = wparam.clamp(0, 255) as u8;
                    // Finding 3: Mutates local surface only; overwritten by next PlayerStateUpdate snapshot.
                    self.volume = v;
                    // Finding 4: S10 §0.2 E4 and §13.11 note that setter steps were posted, so
                    // what a sent setter returns was never measured. Returning the new value is an
                    // unmeasured fallback convention ([IDX] states "old or new value").
                    IpcAnswer::Value(v as u32)
                }
            }
            123 => {
                if wparam == -666 {
                    IpcAnswer::Value(self.pan as u32)
                } else {
                    let b = (wparam as u32) & 0xff;
                    let signed = if b >= 128 { b as i32 - 256 } else { b as i32 };
                    // Finding 3: Mutates local surface only; overwritten by next PlayerStateUpdate snapshot.
                    self.pan = signed;
                    // Finding 4: Sent setter return value is unmeasured (S10 §0.2 E4, §13.11).
                    // Returning signed pan is an unmeasured fallback convention.
                    IpcAnswer::Value(signed as u32)
                }
            }
            124 => IpcAnswer::Value(self.playlist_len),
            125 => IpcAnswer::Value(self.playlist_pos),
            126 => {
                let mode = wparam as i32;
                if self.is_playing == 0 {
                    IpcAnswer::Value(0)
                } else {
                    match mode {
                        // Finding 4: S10 §13.2 leaves stream info while playing open;
                        // whether mode 0 returns raw Hz (self.sample_rate) or kHz is unmeasured.
                        0 => IpcAnswer::Value(self.sample_rate),
                        1 => IpcAnswer::Value(self.bitrate),
                        2 => IpcAnswer::Value(self.channels),
                        _ => IpcAnswer::Value(0),
                    }
                }
            }
            127 => {
                let selector = wparam as i32;
                let reply = match selector {
                    0..=9 => self.eq_bands[selector as usize],
                    10 => self.eq_preamp,
                    11 => i32::from(self.eq_enabled),
                    12 => i32::from(self.eq_autoload),
                    _ => 0,
                };
                self.eq_selected = Some(selector as u32);
                IpcAnswer::Value(reply as u32)
            }
            128 => {
                let value = wparam as i32;
                match self.eq_selected {
                    Some(sel) if sel < 10 => {
                        self.eq_bands[sel as usize] = value;
                    }
                    Some(10) => self.eq_preamp = value,
                    Some(11) => self.eq_enabled = value != 0,
                    Some(12) => self.eq_autoload = value != 0,
                    _ => {}
                }
                // Finding 3: Local surface is mutated, but not forwarded to engine;
                // next PlayerStateUpdate snapshot will overwrite these values.
                IpcAnswer::CommandEffect { id }
            }
            250 => IpcAnswer::Value(u32::from(self.shuffle)),
            251 => IpcAnswer::Value(u32::from(self.repeat)),
            252 => {
                // Finding 4: -666 is measured only for 122/123. S10 defines dedicated getter 250;
                // -666 query handling on 252 is an unmeasured speculative convenience.
                if wparam == -666 {
                    IpcAnswer::Value(u32::from(self.shuffle))
                } else {
                    // Finding 3: Mutates local surface only; overwritten by next engine snapshot.
                    self.shuffle = wparam != 0;
                    // Finding 4: Sent return value is unmeasured (step was posted in S10).
                    IpcAnswer::Value(u32::from(self.shuffle))
                }
            }
            253 => {
                // Finding 4: -666 is measured only for 122/123. S10 defines dedicated getter 251;
                // -666 query handling on 253 is an unmeasured speculative convenience.
                if wparam == -666 {
                    IpcAnswer::Value(u32::from(self.repeat))
                } else {
                    // Finding 3: Mutates local surface only; overwritten by next engine snapshot.
                    self.repeat = wparam != 0;
                    // Finding 4: Sent return value is unmeasured (step was posted in S10).
                    IpcAnswer::Value(u32::from(self.repeat))
                }
            }
            260 => {
                // Finding 5: S10 row 260 measured distinct nonzero handles. These default to 0
                // on a fresh surface pending an M3 architecture decision on handle ownership (S10 §13.8).
                let handle = match wparam {
                    0 => self.eq_hwnd,
                    1 => self.playlist_hwnd,
                    2 => self.minibrowser_hwnd,
                    3 => self.video_hwnd,
                    _ => 0,
                };
                IpcAnswer::RuntimeHandle { id, handle }
            }
            261 => {
                let v = match wparam {
                    0 => self.visible_eq,
                    1 => self.visible_playlist,
                    2 => self.visible_minibrowser,
                    3 => self.visible_video,
                    _ => self.visible_main,
                };
                IpcAnswer::Value(u32::from(v))
            }
            281 => {
                // Finding 5: S10 row 281 measured a nonzero menu handle. Defaults to 0
                // pending M3 architecture decision on menu ownership (S10 §13.8).
                IpcAnswer::RuntimeHandle {
                    id,
                    handle: self.menu_handle,
                }
            }
            400 => IpcAnswer::Value(0),
            501 => {
                if self.is_playing == 0 {
                    IpcAnswer::Value(1)
                } else {
                    IpcAnswer::NeedsPlayingReference { id }
                }
            }
            502 => IpcAnswer::Value(1),
            666 => IpcAnswer::Value(1),
            // Recorded address belongs to the reference process; its meaning is
            // unaudited. Never return it as a pointer into this host process.
            800 => IpcAnswer::NeedsPluginDll { id },
            801 => IpcAnswer::Value(1),
            900 => IpcAnswer::Value(1),
            999 => IpcAnswer::Value(1),
            3015 => IpcAnswer::Value(1),
            3025 => IpcAnswer::Value(1),
            65536 => IpcAnswer::Value(1),
            // Finding 4: S10 row 242 explicitly states: "provisional — sandbox-networking-dependent;
            // not a fixture property, do not export it as one (§13.7)". Defaulting to true (1) is provisional.
            242 => IpcAnswer::Value(u32::from(self.inet_available)),
            _ => {
                // Known measured ids that intentionally carry no runtime value.
                if let Some(row) = lookup_table(id) {
                    if row.requires_playing_reference {
                        return IpcAnswer::NeedsPlayingReference { id };
                    }
                    if row.requires_plugin_dll {
                        return IpcAnswer::NeedsPluginDll { id };
                    }
                    if let Some(v) = row.measured_value {
                        return IpcAnswer::Value(v);
                    }
                }
                IpcAnswer::Unknown(UNKNOWN_REPLY)
            }
        }
    }

    /// Equality-friendly answer used in tests.
    pub fn reply(&mut self, id: u32, wparam: i64) -> IpcAnswer {
        self.answer(id, wparam)
    }
}

/// Alias for callers that prefer the query vocabulary.
pub fn ipc_answer(surface: &mut IpcSurface, id: u32, wparam: i64) -> IpcAnswer {
    surface.answer(id, wparam)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_query_answers_0x2905() {
        let mut s = IpcSurface::new();
        assert_eq!(s.answer(0, 0), IpcAnswer::Value(0x2905));
        assert_eq!(WINAMP_VERSION_295, 0x2905);
    }

    #[test]
    fn lookup_finds_the_version_row_with_evidence() {
        let row = lookup_table(0).expect("version row measured");
        assert_eq!(row.id, 0);
        assert_eq!(row.measured_value, Some(0x2905));
        assert!(!row.requires_playing_reference);
        assert_eq!(row.evidence_run, "R1/R2/R3");
        assert_eq!(row.evidence_step, "s10_000_getversion");
    }

    #[test]
    fn unknown_ids_are_not_silently_answered() {
        let mut s = IpcSurface::new();
        for id in [2u32, 42, 10_000, 2_147_483_646] {
            assert_eq!(s.answer(id, 0), IpcAnswer::Unknown(1));
            assert!(s.answer(id, 0).raw() == 1);
            assert!(lookup_table(id).is_none());
        }
        // The probes measured as unsupported also report 1.
        for id in [1u32, 9999, u32::MAX] {
            assert_eq!(lookup_table(id).unwrap().measured_value, Some(1));
        }
    }

    #[test]
    fn spectrum_entry_point_never_returns_the_reference_address() {
        let row = lookup_table(800).expect("measured reference row");
        assert_eq!(row.measured_value, Some(4_407_808));
        let mut surface = IpcSurface::new();
        for argument in [0, 1, -666, i64::MAX] {
            let answer = surface.answer(800, argument);
            assert_eq!(answer, IpcAnswer::NeedsPluginDll { id: 800 });
            assert_eq!(answer.raw(), UNKNOWN_REPLY);
            assert_ne!(Some(answer.raw()), row.measured_value);
        }
    }

    #[test]
    fn negative_reply_uses_low_32_bits() {
        assert_eq!(u32_from_i32(-1), 4_294_967_295);
        assert_eq!(i32_from_u32(4_294_967_295), -1);
    }

    #[test]
    fn volume_is_unsigned_and_pan_is_signed() {
        let mut s = IpcSurface::new();
        // Volume 0..=255 stays unsigned.
        assert_eq!(s.answer(122, 64), IpcAnswer::Value(64));
        assert_eq!(s.answer(122, -666), IpcAnswer::Value(64));
        assert_eq!(s.answer(122, 255), IpcAnswer::Value(255));
        assert_eq!(s.answer(122, 300), IpcAnswer::Value(255));
        assert_eq!(s.volume_unsigned(), 255);

        // Pan: setting 255 reads back -1 (0xFFFFFFFF).
        assert_eq!(s.answer(123, 255).raw(), 4_294_967_295);
        assert_eq!(s.answer(123, -666), IpcAnswer::Value(4_294_967_295));
        assert_eq!(s.pan_unsigned(), 4_294_967_295);
        assert_eq!(s.pan, -1);

        // A signed-ish input of 64 stays 64.
        assert_eq!(s.answer(123, 64).raw(), 64);
    }

    #[test]
    fn equalizer_read_write_is_stateful() {
        let mut s = IpcSurface::new();
        // Fresh flat value.
        assert_eq!(s.answer(127, 0), IpcAnswer::Value(31));
        // Write a different value through the stateful setter.
        assert!(matches!(
            s.answer(128, 0),
            IpcAnswer::CommandEffect { id: 128 }
        ));
        // Read-back proves the write landed, as in S10 3.9g.
        assert_eq!(s.answer(127, 0), IpcAnswer::Value(0));
        assert_eq!(s.eq_bands[0], 0);

        // Preamp slot (10) is stateful too.
        assert_eq!(s.answer(127, 10), IpcAnswer::Value(31));
        assert!(matches!(
            s.answer(128, 42),
            IpcAnswer::CommandEffect { id: 128 }
        ));
        assert_eq!(s.answer(127, 10), IpcAnswer::Value(42));
        assert_eq!(s.eq_preamp, 42);
    }

    #[test]
    fn shuffle_and_repeat_echo() {
        let mut s = IpcSurface::new();
        assert_eq!(s.answer(250, 0), IpcAnswer::Value(0));
        assert_eq!(s.answer(252, 1).raw(), 1);
        assert_eq!(s.answer(250, 0), IpcAnswer::Value(1));
        assert_eq!(s.answer(252, 0).raw(), 0);
        assert_eq!(s.answer(250, 0), IpcAnswer::Value(0));

        assert_eq!(s.answer(251, 0), IpcAnswer::Value(0));
        assert_eq!(s.answer(253, 1).raw(), 1);
        assert_eq!(s.answer(251, 0), IpcAnswer::Value(1));
    }

    #[test]
    fn playing_reference_rows_say_so() {
        for id in [105u32, 106, 126, 501] {
            let row = lookup_table(id).unwrap();
            assert!(
                row.requires_playing_reference,
                "id {id} must flag its reference"
            );
        }
    }

    #[test]
    fn all_table_ids_are_unique_and_have_evidence() {
        let mut seen = std::collections::BTreeSet::new();
        for row in IPC_TABLE {
            assert!(seen.insert(row.id), "duplicate table id {}", row.id);
            assert!(!row.evidence_run.is_empty());
            assert!(!row.evidence_step.is_empty());
            assert!(!row.name.is_empty());
        }
    }

    #[test]
    fn version_row_matches_rfc_0009() {
        let row = lookup_table(0).unwrap();
        assert_eq!(row.measured, MeasuredReply::Fixed(WINAMP_VERSION_295));
        assert_eq!(row.measured_value, Some(0x2905));
    }

    #[test]
    fn stateful_rows_cover_volume_pan_eq_shuffle_repeat() {
        for id in [122u32, 123, 127, 128, 250, 251, 252, 253] {
            let row = lookup_table(id).unwrap();
            assert!(row.stateful, "id {id} must be stateful");
        }
    }

    #[test]
    fn finding_3_mutating_ipcs_return_command_effect_without_engine_channel() {
        let mut s = IpcSurface::new();
        // 102 (IPC_STARTPLAY) and 135 (IPC_RESTARTWINAMP) return CommandEffect
        // with raw() == 1. In our architecture, no reverse IPC command channel
        // from plughost to rustamp.exe exists yet (S10 §2, §11, RFC 0005 O13).
        let ans_102 = s.answer(102, 0);
        assert_eq!(ans_102, IpcAnswer::CommandEffect { id: 102 });
        assert_eq!(ans_102.raw(), UNKNOWN_REPLY);

        let ans_135 = s.answer(135, 0);
        assert_eq!(ans_135, IpcAnswer::CommandEffect { id: 135 });
        assert_eq!(ans_135.raw(), UNKNOWN_REPLY);

        // Surface state remains stopped (0) because 102 has no engine link here.
        assert_eq!(s.is_playing, 0);
    }

    #[test]
    fn finding_4_sent_volume_and_pan_fallback_returns_new_value() {
        let mut s = IpcSurface::new();
        // S10 §0.2 E4 and §13.11 note that setter steps were posted, so what a
        // sent setter returns was never measured. The host currently returns the
        // new value as an unmeasured fallback convention.
        assert_eq!(s.answer(122, 100), IpcAnswer::Value(100));
        assert_eq!(s.volume, 100);
        assert_eq!(s.answer(122, -666), IpcAnswer::Value(100));

        assert_eq!(s.answer(123, 40), IpcAnswer::Value(40));
        assert_eq!(s.pan, 40);
        assert_eq!(s.answer(123, -666), IpcAnswer::Value(40));
    }

    #[test]
    fn finding_4_shuffle_and_repeat_unmeasured_minus_666_fallback() {
        let mut s = IpcSurface::new();
        // S10 rows 250/251 are the dedicated getters. -666 was measured only for
        // 122/123. The -666 query branch on 252/253 is unmeasured, and sent 252/253
        // return value is unmeasured (step posted in S10).
        assert_eq!(s.answer(252, -666), IpcAnswer::Value(0));
        assert_eq!(s.answer(253, -666), IpcAnswer::Value(0));

        // Setting non-zero updates local state.
        assert_eq!(s.answer(252, 1), IpcAnswer::Value(1));
        assert!(s.shuffle);
        assert_eq!(s.answer(252, -666), IpcAnswer::Value(1));

        assert_eq!(s.answer(253, 1), IpcAnswer::Value(1));
        assert!(s.repeat);
        assert_eq!(s.answer(253, -666), IpcAnswer::Value(1));
    }

    #[test]
    fn finding_4_stream_info_playing_sample_rate_fallback() {
        let mut s = IpcSurface::new();
        s.is_playing = 1;
        s.sample_rate = 44_100;
        s.bitrate = 192;
        s.channels = 2;

        // Mode 0 returning raw Hz (44100) vs kHz (44) while playing is unmeasured
        // in S10 (§13.2). Pinned here as the host's current fallback.
        assert_eq!(s.answer(126, 0), IpcAnswer::Value(44_100));
        assert_eq!(s.answer(126, 1), IpcAnswer::Value(192));
        assert_eq!(s.answer(126, 2), IpcAnswer::Value(2));
    }

    #[test]
    fn finding_4_inet_available_provisional_reply() {
        let mut s = IpcSurface::new();
        // S10 §3 row 242 and §13.7: provisional, sandbox-networking-dependent.
        // It must not be treated as an immutable fixture invariant.
        assert_eq!(s.answer(242, 0), IpcAnswer::Value(1));
        s.inet_available = false;
        assert_eq!(s.answer(242, 0), IpcAnswer::Value(0));
    }

    #[test]
    fn finding_5_window_and_menu_handles_default_to_zero_pending_m3_decision() {
        let mut s = IpcSurface::new();
        // S10 rows 260 and 281 measured distinct nonzero handles in 2.95.
        // In our out-of-process 32-bit host, handles default to 0 pending an M3
        // architectural decision on handle ownership (S10 §13.8).
        for selector in 0..=3 {
            let ans = s.answer(260, selector);
            assert_eq!(ans, IpcAnswer::RuntimeHandle { id: 260, handle: 0 });
            assert_eq!(ans.raw(), 0);
        }
        let ans_menu = s.answer(281, 0);
        assert_eq!(ans_menu, IpcAnswer::RuntimeHandle { id: 281, handle: 0 });
        assert_eq!(ans_menu.raw(), 0);
    }
}
