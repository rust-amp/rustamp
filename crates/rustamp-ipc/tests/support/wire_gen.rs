//! The seeded generator behind the wire property tests.
//!
//! The generator is this file's own SplitMix64: no new dependency, and the
//! published crates keep their dependency list (and its license closure)
//! untouched. Every case is reproducible from the seed the failure message
//! prints.
//!
//! The two `*_variant` functions are the **contract table** of the wire: they
//! map a declaration index to the variant that carries it and to its name.
//! The property test checks every arm against the bytes the encoder produces,
//! so a variant inserted into the middle of an enum moves every later index
//! and fails loudly instead of silently changing the wire.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rustamp_ipc::Message;
use rustamp_ipc::VisFrameWire;
use rustamp_ipc::shm::{RingDirection, RingKind, RingRefuseReason};
use rustamp_plugin_abi::PluginKind;
use rustamp_proto::command::{Command, JumpTarget, OpenMediaKind, PrefsPage, SelectOp};
use rustamp_proto::host::{DspStageDesc, MediaLocationWire, PcmSpecWire};
use rustamp_proto::meta::TrackMetadata;
use rustamp_proto::playlist::{EntryId, MediaLocation, SortKey};
use rustamp_proto::state::{EqState, PlayerState, StreamInfo, TransportState, WindowId};

/// The number of [`Message`] variants (declaration indices `0..=27`).
pub const MESSAGE_VARIANT_COUNT: u8 = 28;

/// The number of [`Command`] variants (declaration indices `0..=65`).
pub const COMMAND_VARIANT_COUNT: u8 = 66;

/// A SplitMix64 generator: this file's own seeded source of randomness.
pub struct Rng(u64);

impl Rng {
    /// The characters a generated string is built from.
    ///
    /// Multi-byte characters are in the mix on purpose: a `postcard` string
    /// length counts **bytes**, not characters, so the generated lengths must
    /// not be taken from the character count.
    const ALPHABET: [&'static str; 8] = ["a", "b", "Z", "0", " ", "ä", "€", "漢"];

    /// A generator for `seed`.
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    /// The next 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..bound`; `bound` must not be zero.
    pub fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "a bound of zero has no value");
        self.next_u64() % bound
    }

    /// A boolean.
    pub fn flag(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    /// An arbitrary byte.
    pub fn u8(&mut self) -> u8 {
        self.next_u64() as u8
    }

    /// An arbitrary signed byte.
    pub fn i8(&mut self) -> i8 {
        self.next_u64() as i8
    }

    /// An arbitrary 16-bit value.
    pub fn u16(&mut self) -> u16 {
        self.next_u64() as u16
    }

    /// An arbitrary 32-bit value.
    pub fn u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    /// An arbitrary signed 32-bit value.
    pub fn i32(&mut self) -> i32 {
        self.next_u64() as i32
    }

    /// An arbitrary 64-bit value.
    pub fn u64(&mut self) -> u64 {
        self.next_u64()
    }

    /// An arbitrary signed 64-bit value.
    pub fn i64(&mut self) -> i64 {
        self.next_u64() as i64
    }

    /// A finite `f32`, or one of the special values the wire has to carry
    /// unchanged.
    ///
    /// `NaN` is deliberately left out: it is never equal to itself, so it
    /// cannot take part in an equality-based round-trip property.
    pub fn float(&mut self) -> f32 {
        match self.below(16) {
            0 => 0.0,
            1 => -0.0,
            2 => f32::INFINITY,
            3 => f32::NEG_INFINITY,
            _ => (self.below(2_000_001) as f32) / 1000.0 - 1000.0,
        }
    }

    /// A string of `0..=7` characters, or - now and then - of `130..=137`
    /// characters, which need a two-byte length varint on the wire.
    pub fn text(&mut self) -> String {
        let chars = if self.below(8) == 0 {
            130 + self.below(8) as usize
        } else {
            self.below(8) as usize
        };
        let mut out = String::new();
        for _ in 0..chars {
            out.push_str(Self::ALPHABET[self.below(Self::ALPHABET.len() as u64) as usize]);
        }
        out
    }

    /// A file path.
    pub fn path(&mut self) -> PathBuf {
        let name = self.below(1000);
        PathBuf::from(format!("C:\\music\\{name:03} track.mp3"))
    }

    /// A vector of `0..=max` random items.
    pub fn vec_of<T>(&mut self, max: usize, mut item: impl FnMut(&mut Rng) -> T) -> Vec<T> {
        let len = self.below(max as u64 + 1) as usize;
        let mut out = Vec::with_capacity(len);
        for _ in 0..len {
            out.push(item(self));
        }
        out
    }

    /// `Some` of a random value or `None`.
    pub fn option<T>(&mut self, value: impl FnOnce(&mut Rng) -> T) -> Option<T> {
        if self.flag() { Some(value(self)) } else { None }
    }
}

/// A random plugin kind.
pub fn plugin_kind(rng: &mut Rng) -> PluginKind {
    PluginKind::ALL[rng.below(PluginKind::ALL.len() as u64) as usize]
}

/// A random ring kind.
pub fn ring_kind(rng: &mut Rng) -> RingKind {
    if rng.flag() {
        RingKind::Pcm
    } else {
        RingKind::Vis
    }
}

/// A random ring direction.
pub fn ring_direction(rng: &mut Rng) -> RingDirection {
    if rng.flag() {
        RingDirection::MainToHost
    } else {
        RingDirection::HostToMain
    }
}

/// A random ring refusal reason.
pub fn ring_refuse_reason(rng: &mut Rng) -> RingRefuseReason {
    match rng.below(11) {
        0 => RingRefuseReason::MappingUnavailable,
        1 => RingRefuseReason::BadMagic,
        2 => RingRefuseReason::BadVersion,
        3 => RingRefuseReason::BadHeaderSize,
        4 => RingRefuseReason::WrongKind,
        5 => RingRefuseReason::FrameSizeMismatch,
        6 => RingRefuseReason::CapacityMismatch,
        7 => RingRefuseReason::Stale,
        8 => RingRefuseReason::RegionTooSmall,
        9 => RingRefuseReason::InvalidLayout,
        _ => RingRefuseReason::WakeUnavailable,
    }
}

/// A random window.
pub fn window_id(rng: &mut Rng) -> WindowId {
    match rng.below(6) {
        0 => WindowId::Main,
        1 => WindowId::Equalizer,
        2 => WindowId::Playlist,
        3 => WindowId::Vis,
        4 => WindowId::Gen(rng.u32()),
        _ => WindowId::Video,
    }
}

/// A random sort key.
pub fn sort_key(rng: &mut Rng) -> SortKey {
    match rng.below(4) {
        0 => SortKey::Title,
        1 => SortKey::FileName,
        2 => SortKey::PathAndFileName,
        _ => SortKey::Duration,
    }
}

/// A random selection operation.
pub fn select_op(rng: &mut Rng) -> SelectOp {
    match rng.below(5) {
        0 => {
            let count = rng.below(4);
            let mut ids = Vec::new();
            for _ in 0..count {
                ids.push(entry_id(rng));
            }
            SelectOp::Set(ids)
        }
        1 => SelectOp::Toggle(entry_id(rng)),
        2 => SelectOp::All,
        3 => SelectOp::None,
        _ => SelectOp::Invert,
    }
}

/// A random entry id.
pub fn entry_id(rng: &mut Rng) -> EntryId {
    EntryId(rng.u64())
}

/// A random media location.
pub fn media_location(rng: &mut Rng) -> MediaLocation {
    if rng.flag() {
        MediaLocation::File(rng.path())
    } else {
        MediaLocation::Url(format!("http://example.com/{}", rng.text()))
    }
}

/// A random media location in its wire form.
pub fn media_location_wire(rng: &mut Rng) -> MediaLocationWire {
    if rng.flag() {
        MediaLocationWire::File(rng.path().to_string_lossy().into_owned())
    } else {
        MediaLocationWire::Url(format!("http://example.com/{}", rng.text()))
    }
}

/// A random open-media kind.
pub fn open_media_kind(rng: &mut Rng) -> OpenMediaKind {
    match rng.below(3) {
        0 => OpenMediaKind::Files,
        1 => OpenMediaKind::Location,
        _ => OpenMediaKind::Directory,
    }
}

/// A random jump target.
pub fn jump_target(rng: &mut Rng) -> JumpTarget {
    if rng.flag() {
        JumpTarget::File
    } else {
        JumpTarget::Time
    }
}

/// A random preferences page.
pub fn prefs_page(rng: &mut Rng) -> PrefsPage {
    if rng.flag() {
        PrefsPage::VisPlugin
    } else {
        PrefsPage::VisOptions
    }
}

/// A random PCM specification.
pub fn pcm_spec(rng: &mut Rng) -> PcmSpecWire {
    PcmSpecWire {
        sample_rate_hz: rng.u32(),
        channels: rng.u8(),
    }
}

/// A random DSP stage.
pub fn dsp_stage(rng: &mut Rng) -> DspStageDesc {
    DspStageDesc {
        id: rng.text(),
        params: rng.vec_of(4, |r| r.u8()),
    }
}

/// A random track metadata block.
pub fn track_metadata(rng: &mut Rng) -> TrackMetadata {
    let mut extra = BTreeMap::new();
    for index in 0..rng.below(3) {
        extra.insert(format!("KEY_{index}"), rng.text());
    }
    TrackMetadata {
        title: rng.option(|r| r.text()),
        artist: rng.option(|r| r.text()),
        album: rng.option(|r| r.text()),
        album_artist: rng.option(|r| r.text()),
        year: rng.option(|r| r.u32()),
        genre: rng.option(|r| r.text()),
        comment: rng.option(|r| r.text()),
        track_no: rng.option(|r| r.u32()),
        track_total: rng.option(|r| r.u32()),
        disc_no: rng.option(|r| r.u32()),
        duration_ms: rng.option(|r| r.u64()),
        bitrate_kbps: rng.option(|r| r.u32()),
        extra,
    }
}

/// A random player state snapshot.
pub fn player_state(rng: &mut Rng) -> PlayerState {
    let mut bands = [0.0f32; 10];
    for band in bands.iter_mut() {
        *band = rng.float();
    }
    PlayerState {
        transport: match rng.below(3) {
            0 => TransportState::Stopped,
            1 => TransportState::Playing,
            _ => TransportState::Paused,
        },
        position_ms: rng.u64(),
        duration_ms: rng.option(|r| r.u64()),
        volume: rng.u8(),
        balance: rng.i8(),
        shuffle: rng.flag(),
        repeat: rng.flag(),
        current_index: rng.option(|r| r.u32()),
        current_id: rng.option(entry_id),
        playlist_len: rng.u32(),
        stream: rng.option(|r| StreamInfo {
            bitrate_kbps: r.option(|r| r.u32()),
            sample_rate_hz: r.u32(),
            channels: r.u16(),
        }),
        eq: EqState {
            bands,
            preamp: rng.float(),
            enabled: rng.flag(),
            auto: rng.flag(),
        },
        stop_after_current: rng.flag(),
    }
}

/// A random visualization frame in its wire form.
pub fn vis_frame_wire(rng: &mut Rng) -> VisFrameWire {
    VisFrameWire {
        timestamp_us: rng.u64(),
        waveform: rng.vec_of(8, |r| r.float()),
        spectrum: rng.vec_of(8, |r| r.float()),
    }
}

/// The [`Command`] variant with the given declaration index, and its name.
///
/// The arms are the contract table: the arm's index is the variant's
/// **documented** number, and the property test checks it against the byte
/// the encoder produces. `None` means "no variant with that index".
pub fn command_variant(index: u8, rng: &mut Rng) -> Option<(&'static str, Command)> {
    Some(match index {
        0 => ("Play", Command::Play),
        1 => ("Pause", Command::Pause),
        2 => ("TogglePause", Command::TogglePause),
        3 => ("Stop", Command::Stop),
        4 => ("StopWithFadeout", Command::StopWithFadeout),
        5 => ("Next", Command::Next),
        6 => ("Prev", Command::Prev),
        7 => ("Seek", Command::Seek { to_ms: rng.u64() }),
        8 => (
            "SeekRelative",
            Command::SeekRelative {
                delta_ms: rng.i64(),
            },
        ),
        9 => ("SetVolume", Command::SetVolume(rng.u8())),
        10 => ("SetBalance", Command::SetBalance(rng.i8())),
        11 => ("ToggleShuffle", Command::ToggleShuffle),
        12 => ("ToggleRepeat", Command::ToggleRepeat),
        13 => ("SetShuffle", Command::SetShuffle(rng.flag())),
        14 => ("SetRepeat", Command::SetRepeat(rng.flag())),
        15 => (
            "AddItems",
            Command::AddItems {
                items: rng.vec_of(3, media_location),
                at: rng.option(|r| r.u32()),
                play: rng.flag(),
            },
        ),
        16 => ("Remove", Command::Remove(rng.vec_of(3, entry_id))),
        17 => ("Clear", Command::Clear),
        18 => (
            "MoveEntries",
            Command::MoveEntries {
                ids: rng.vec_of(3, entry_id),
                to: rng.u32(),
            },
        ),
        19 => ("Select", Command::Select(select_op(rng))),
        20 => ("PlayIndex", Command::PlayIndex(rng.u32())),
        21 => ("PlayEntry", Command::PlayEntry(entry_id(rng))),
        22 => ("Sort", Command::Sort(sort_key(rng))),
        23 => ("Randomize", Command::Randomize),
        24 => ("Reverse", Command::Reverse),
        25 => (
            "SetBand",
            Command::SetBand {
                band: rng.u8(),
                gain_db: rng.float(),
            },
        ),
        26 => ("SetPreamp", Command::SetPreamp(rng.float())),
        27 => ("SetEqEnabled", Command::SetEqEnabled(rng.flag())),
        28 => ("SetEqAuto", Command::SetEqAuto(rng.flag())),
        29 => ("ToggleWindow", Command::ToggleWindow(window_id(rng))),
        30 => ("ToggleDoubleSize", Command::ToggleDoubleSize),
        31 => (
            "ToggleWindowShade",
            Command::ToggleWindowShade(window_id(rng)),
        ),
        32 => ("ToggleAlwaysOnTop", Command::ToggleAlwaysOnTop),
        33 => ("LoadSkin", Command::LoadSkin(rng.path())),
        34 => ("Quit", Command::Quit),
        35 => ("OpenPrefs", Command::OpenPrefs),
        36 => ("SkipBy", Command::SkipBy { delta: rng.i32() }),
        37 => ("PlayLast", Command::PlayLast),
        38 => ("ToggleStopAfterCurrent", Command::ToggleStopAfterCurrent),
        39 => (
            "OpenMedia",
            Command::OpenMedia {
                kind: open_media_kind(rng),
                play: rng.flag(),
            },
        ),
        40 => (
            "OpenJump",
            Command::OpenJump {
                target: jump_target(rng),
            },
        ),
        41 => ("OpenFileInfo", Command::OpenFileInfo),
        42 => ("OpenPrefsPage", Command::OpenPrefsPage(prefs_page(rng))),
        43 => ("OpenAbout", Command::OpenAbout),
        44 => ("OpenSkinBrowser", Command::OpenSkinBrowser),
        45 => ("RefreshSkin", Command::RefreshSkin),
        46 => ("OpenBookmarkEditor", Command::OpenBookmarkEditor),
        47 => ("AddBookmark", Command::AddBookmark),
        48 => ("ToggleEasyMove", Command::ToggleEasyMove),
        49 => ("Minimize", Command::Minimize),
        50 => ("FocusNextWindow", Command::FocusNextWindow),
        51 => ("ShowMainWindow", Command::ShowMainWindow),
        52 => ("CloseFocusedWindow", Command::CloseFocusedWindow),
        53 => ("OpenMainMenu", Command::OpenMainMenu),
        54 => ("ConfigureVisPlugin", Command::ConfigureVisPlugin),
        55 => ("ToggleVisPlugin", Command::ToggleVisPlugin),
        56 => ("OpenPlaylist", Command::OpenPlaylist),
        57 => ("OpenOptionsMenu", Command::OpenOptionsMenu),
        58 => ("OpenVisualizationMenu", Command::OpenVisualizationMenu),
        59 => ("SavePlaylist", Command::SavePlaylist(rng.path())),
        60 => ("RemoveDeadFiles", Command::RemoveDeadFiles),
        // RFC 0020's plug-in requests, appended after `RemoveDeadFiles`.
        61 => (
            "ConfigurePlugin",
            Command::ConfigurePlugin {
                plugin_id: rng.u32(),
                kind: plugin_kind(rng),
            },
        ),
        62 => (
            "SeekPlugin",
            Command::SeekPlugin {
                plugin_id: rng.u32(),
                to_ms: rng.i32(),
            },
        ),
        63 => (
            "PausePlugin",
            Command::PausePlugin {
                plugin_id: rng.u32(),
            },
        ),
        64 => (
            "UnpausePlugin",
            Command::UnpausePlugin {
                plugin_id: rng.u32(),
            },
        ),
        65 => (
            "StopPlugin",
            Command::StopPlugin {
                plugin_id: rng.u32(),
            },
        ),
        _ => return None,
    })
}

/// A random [`Command`] variant.
pub fn random_command(rng: &mut Rng) -> Command {
    let index = rng.below(u64::from(COMMAND_VARIANT_COUNT)) as u8;
    command_variant(index, rng)
        .map(|(_, command)| command)
        .expect("a Command variant")
}

/// The [`Message`] variant with the given declaration index, and its name.
///
/// The arms are the contract table, like [`command_variant`]'s.
pub fn message_variant(index: u8, rng: &mut Rng) -> Option<(&'static str, Message)> {
    Some(match index {
        0 => (
            "Hello",
            Message::Hello {
                protocol_version: rng.u32(),
                pid: rng.u32(),
            },
        ),
        1 => ("Ping", Message::Ping(rng.u64())),
        2 => ("Pong", Message::Pong(rng.u64())),
        3 => (
            "LoadPlugin",
            Message::LoadPlugin {
                path: rng.text(),
                kind: plugin_kind(rng),
            },
        ),
        4 => (
            "PluginLoaded",
            Message::PluginLoaded {
                plugin_id: rng.u32(),
                name: rng.text(),
                kind: plugin_kind(rng),
            },
        ),
        5 => (
            "PluginError",
            Message::PluginError {
                path: rng.text(),
                message: rng.text(),
            },
        ),
        6 => (
            "PlayerStateUpdate",
            Message::PlayerStateUpdate(Box::new(player_state(rng))),
        ),
        7 => (
            "CommandFromPlugin",
            Message::CommandFromPlugin(random_command(rng)),
        ),
        8 => ("Vis", Message::Vis(Box::new(vis_frame_wire(rng)))),
        9 => (
            "RingOffer",
            Message::RingOffer {
                name: rng.text(),
                kind: ring_kind(rng),
                frame_size: rng.u32(),
                capacity: rng.u32(),
                size: rng.u64(),
                generation: rng.u64(),
                direction: ring_direction(rng),
            },
        ),
        10 => (
            "RingReady",
            Message::RingReady {
                kind: ring_kind(rng),
                generation: rng.u64(),
            },
        ),
        11 => (
            "RingRefused",
            Message::RingRefused {
                kind: ring_kind(rng),
                generation: rng.u64(),
                reason: ring_refuse_reason(rng),
            },
        ),
        12 => ("Shutdown", Message::Shutdown),
        13 => (
            "WinampIpc",
            Message::WinampIpc {
                msg_id: rng.u32(),
                wparam: rng.u64(),
                lparam: rng.u64(),
            },
        ),
        14 => ("CommandToHost", Message::CommandToHost(random_command(rng))),
        15 => (
            "DecoderOpen",
            Message::DecoderOpen {
                location: media_location_wire(rng),
                prefetch_frames: rng.u32(),
            },
        ),
        16 => (
            "DecoderOpened",
            Message::DecoderOpened {
                spec: pcm_spec(rng),
                duration_ms: rng.option(|r| r.u64()),
                seekable: rng.flag(),
                metadata: track_metadata(rng),
            },
        ),
        17 => ("DecoderSeek", Message::DecoderSeek { to_ms: rng.u64() }),
        18 => (
            "DecoderSeeked",
            Message::DecoderSeeked {
                reached_ms: rng.u64(),
            },
        ),
        19 => (
            "DecoderFormat",
            Message::DecoderFormat {
                spec: pcm_spec(rng),
            },
        ),
        20 => (
            "DecoderStats",
            Message::DecoderStats {
                skipped_packets: rng.u64(),
                bitrate_kbps: rng.option(|r| r.u32()),
            },
        ),
        21 => ("DecoderEos", Message::DecoderEos),
        22 => (
            "DecoderError",
            Message::DecoderError {
                message: rng.text(),
            },
        ),
        23 => (
            "DspConfigure",
            Message::DspConfigure {
                spec: pcm_spec(rng),
                stages: rng.vec_of(3, dsp_stage),
            },
        ),
        24 => (
            "DspReady",
            Message::DspReady {
                latency_frames: rng.u32(),
            },
        ),
        25 => ("DspReset", Message::DspReset),
        26 => ("DspResetAck", Message::DspResetAck),
        27 => (
            "DspError",
            Message::DspError {
                message: rng.text(),
            },
        ),
        _ => return None,
    })
}
