//! The protocol message enum.

use rustamp_plugin_abi::PluginKind;
use rustamp_proto::command::Command;
use rustamp_proto::host::{DspStageDesc, MediaLocationWire, PcmSpecWire};
use rustamp_proto::meta::TrackMetadata;
use rustamp_proto::state::PlayerState;
use rustamp_proto::vis::{VIS_CHANNELS, VIS_SAMPLES, VisFrame};
use serde::{Deserialize, Serialize};

use crate::shm::{RingDirection, RingExpect, RingKind, RingRefuseReason};

/// Protocol version spoken by this build. Bumped on any incompatible change
/// to [`Message`]; peers compare it in `Hello`.
///
/// * `0`: `Hello`, `Ping`/`Pong`, `LoadPlugin`, `PluginLoaded`/`PluginError`,
///   `PlayerStateUpdate`, `CommandFromPlugin`, `Vis`, `Shutdown`, `WinampIpc`.
/// * `1`: adds `RingOffer`, `RingReady`, `RingRefused` (RFC 0003).
/// * `2`: adds the RFC 0005 app-level `Command` variants, `WindowId::Video`,
///   the `PlayerState::stop_after_current` field and [`Message::CommandToHost`]
///   (RFC 0005). The command surface is part of the protocol: a v1-only peer
///   cannot decode a `CommandFromPlugin` carrying a new variant, so the bump
///   turns a mixed-version pair into a clean `Hello` rejection instead of a
///   postcard decode error (RFC 0003 §9's reasoning for the 0 -> 1 bump).
/// * `3`: adds the RFC 0006 hosting messages (`DecoderOpen`…`DspError`) and
///   the RFC 0007 wake events (`rustamp-ipc`-local, no `Message` change).
///   RFC 0005 shipped v2 first; RFC 0006/0007 therefore take v3 to avoid
///   breaking existing v2 peers that would mis-decode the new variants.
/// * `4`: adds [`Message::DspResetAck`], the acknowledgement that turns
///   `DspReset` into an ordering fence (RFC 0015). Postcard encodes an enum
///   by declaration index, so a v3 peer would mis-decode the new variant —
///   and `DspError`, whose index moves from 26 to 27 — for the same reason
///   the 0 → 1 and 2 → 3 bumps exist: the mismatch becomes a clean `Hello`
///   rejection instead of a decode error.
/// * `5`: adds the RFC 0020 plug-in request `Command` variants
///   (`ConfigurePlugin`, `SeekPlugin`, `PausePlugin`, `UnpausePlugin`,
///   `StopPlugin`), which travel in the existing `Message::CommandToHost`
///   carrier — `Message` itself is unchanged. A v4 peer cannot decode the
///   appended variants, so the same rule applies: a clean `Hello` rejection
///   instead of a postcard decode error.
/// * `6`: adds [`Message::NowPlaying`], the track plug-ins read from the main
///   window.
/// * `7`: adds [`Message::EmbedSkin`], the sheet the host's embed frames draw
///   from, and [`Message::OutputClock`], what an output plug-in has played
///   (RFC 0024).
pub const PROTOCOL_VERSION: u32 = 7;

/// Wire form of [`VisFrame`] (flat vectors, `channel-major`: all samples of
/// channel 0, then channel 1). Each vector has `VIS_CHANNELS * VIS_SAMPLES` values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisFrameWire {
    /// See [`VisFrame::timestamp_us`].
    pub timestamp_us: u64,
    /// Flattened waveform.
    pub waveform: Vec<f32>,
    /// Flattened spectrum.
    pub spectrum: Vec<f32>,
}

impl From<&VisFrame> for VisFrameWire {
    fn from(f: &VisFrame) -> Self {
        VisFrameWire {
            timestamp_us: f.timestamp_us,
            waveform: f.waveform.iter().flatten().copied().collect(),
            spectrum: f.spectrum.iter().flatten().copied().collect(),
        }
    }
}

impl VisFrameWire {
    /// Convert back; `None` if the vectors have the wrong length.
    pub fn to_frame(&self) -> Option<VisFrame> {
        let n = VIS_CHANNELS * VIS_SAMPLES;
        if self.waveform.len() != n || self.spectrum.len() != n {
            return None;
        }
        let mut f = VisFrame {
            timestamp_us: self.timestamp_us,
            ..VisFrame::default()
        };
        for c in 0..VIS_CHANNELS {
            let r = c * VIS_SAMPLES..(c + 1) * VIS_SAMPLES;
            f.waveform[c].copy_from_slice(&self.waveform[r.clone()]);
            f.spectrum[c].copy_from_slice(&self.spectrum[r]);
        }
        Some(f)
    }
}

/// A protocol message in either direction.
///
/// Direction notes: **M** = main process -> host, **H** = host -> main.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// Both: first message each side sends.
    Hello {
        /// Sender's [`PROTOCOL_VERSION`].
        protocol_version: u32,
        /// Sender's process id.
        pid: u32,
    },
    /// Both: liveness probe; the peer answers `Pong` with the same token.
    Ping(u64),
    /// Both: answer to `Ping`.
    Pong(u64),
    /// M: load a plugin DLL in the host.
    LoadPlugin {
        /// Path to the DLL (host-visible path).
        path: String,
        /// Expected kind.
        kind: PluginKind,
    },
    /// H: plugin loaded successfully.
    PluginLoaded {
        /// Host-assigned handle for later messages.
        plugin_id: u32,
        /// Display name reported by the plugin.
        name: String,
        /// Kind.
        kind: PluginKind,
    },
    /// H: loading or running a plugin failed.
    PluginError {
        /// Path of the plugin, if known.
        path: String,
        /// Description.
        message: String,
    },
    /// M: new player state snapshot for the host's fake Winamp window.
    PlayerStateUpdate(Box<PlayerState>),
    /// H: a plugin asked the player to do something.
    CommandFromPlugin(Command),
    /// M: latest visualization frame (fallback path; see crate docs).
    Vis(Box<VisFrameWire>),
    /// Both: the sender created a shared-memory ring and wants the peer to open
    /// it (RFC 0003). The peer answers `RingReady` or `RingRefused`.
    RingOffer {
        /// Mapping name, opaque to the transport (see `shm::ring_name`).
        name: String,
        /// Which payload the ring carries.
        kind: RingKind,
        /// Bytes per slot.
        frame_size: u32,
        /// Number of slots.
        capacity: u32,
        /// Total mapping size in bytes.
        size: u64,
        /// Generation (epoch) the creator wrote into the header.
        generation: u64,
        /// Which way data flows; tells the peer which end it is.
        direction: RingDirection,
    },
    /// Both: the ring is open and validated; data may flow.
    RingReady {
        /// Which ring this acknowledges.
        kind: RingKind,
        /// Generation that was validated.
        generation: u64,
    },
    /// Both: the ring could not be opened, or it is stale/foreign.
    RingRefused {
        /// Which ring this refuses.
        kind: RingKind,
        /// Generation that was offered.
        generation: u64,
        /// Why it was refused.
        reason: RingRefuseReason,
    },
    /// M: ask the host to exit cleanly.
    Shutdown,
    /// Both: opaque classic-Winamp window message
    /// (`SendMessage`-style). **Placeholder:** semantics (which ids, how
    /// replies are matched, pointer-argument marshalling) are defined by
    /// spec S10.
    WinampIpc {
        /// Message id (the `WM_USER`-range IPC number).
        msg_id: u32,
        /// First argument, zero-extended to 64 bits.
        wparam: u64,
        /// Second argument, zero-extended to 64 bits.
        lparam: u64,
    },
    /// M: the player asks the host to do something that only the host can do —
    /// the player -> host direction of RFC 0005 §5.4
    /// (`ConfigureVisPlugin`/`ToggleVisPlugin`, which the app applies *through*
    /// the host because the plug-in runs in `rustamp-plughost32.exe`).
    ///
    /// RFC 0020 appends the requests that **name** a plug-in — its
    /// host-assigned id plus, where the request has one, a small payload
    /// (`ConfigurePlugin`, `SeekPlugin`, `PausePlugin`, `UnpausePlugin`,
    /// `StopPlugin`) — without adding a `Message` variant: this carrier already
    /// reaches the host's router, so the whole wire change is five `Command`
    /// variants and one version number.
    ///
    /// Appended at the end of the enum so every earlier `Message` keeps its
    /// variant index (and its bytes). **Placeholder overlap:** how this relates
    /// to `WinampIpc` is S10's to resolve (RFC 0005 O13); the handling lands
    /// with M3.
    CommandToHost(Command),

    // ==== RFC 0006: out-of-process decoder hosting ====
    /// M: open a decoder in the helper. Answers: `DecoderOpened` | `DecoderError`.
    DecoderOpen {
        /// Media location (UTF-8 path or URL).
        location: MediaLocationWire,
        /// Ring capacity hint (frames).
        prefetch_frames: u32,
    },
    /// H: the decoder is open; a `RingOffer` for its PCM follows.
    DecoderOpened {
        /// Stream specification.
        spec: PcmSpecWire,
        /// Duration in milliseconds, if known.
        duration_ms: Option<u64>,
        /// Whether the stream is seekable.
        seekable: bool,
        /// Track metadata.
        metadata: TrackMetadata,
    },
    /// M: seek the open decoder. Answer: `DecoderSeeked` (after the new `RingOffer`).
    DecoderSeek {
        /// Target position in milliseconds.
        to_ms: u64,
    },
    /// H: seek done, `reached_ms` is what the decoder reports.
    DecoderSeeked {
        /// Actual position reached in milliseconds.
        reached_ms: u64,
    },
    /// H: the stream changed format; a new `RingOffer` follows (RFC 0002 C3).
    DecoderFormat {
        /// New stream specification.
        spec: PcmSpecWire,
    },
    /// H: damage/bitrate telemetry, on change only.
    DecoderStats {
        /// Cumulative skipped packets.
        skipped_packets: u64,
        /// Current bitrate in kbps, if known.
        bitrate_kbps: Option<u32>,
    },
    /// H: end of stream; the ring is drained, then closed.
    DecoderEos,
    /// Either: `DecoderOpen`/`DecoderSeek` failed, or the decoder died.
    DecoderError {
        /// Error description.
        message: String,
    },

    // ==== RFC 0006: out-of-process DSP hosting ====
    /// M: (re)configure the DSP chain. Answers: `DspReady` | `DspError`.
    DspConfigure {
        /// Stream specification.
        spec: PcmSpecWire,
        /// Ordered list of stages.
        stages: Vec<DspStageDesc>,
    },
    /// H: chain is live; both rings are offered; engine compensates this latency.
    DspReady {
        /// Total latency in frames reported by the chain.
        latency_frames: u32,
    },
    /// M: clear stage state; rings untouched.
    DspReset,
    /// H: answer to `DspReset` — the chain state is cleared (RFC 0015).
    /// This is the delivery fence: `DspSession::reset` returns only after
    /// the acknowledgement arrives.
    DspResetAck,
    /// Either: configure failed, a stage died, or the chain broke.
    DspError {
        /// Error description.
        message: String,
    },
    /// M: what plays now, for the plug-ins that ask the main window.
    NowPlaying {
        /// The track's place in the playlist, from 0.
        position: u32,
        /// The track's display title.
        title: String,
        /// The track's path or URL.
        file: String,
    },
    /// M: the skin's `gen.bmp` as pixels, for the embed frames plug-ins ask the
    /// host for. The player decodes skins, the host only draws (RFC 0024). Empty
    /// pixels: the skin has no such sheet, and the frames are plain windows.
    EmbedSkin {
        /// Width of the sheet in pixels.
        width: u32,
        /// Height of the sheet in pixels.
        height: u32,
        /// The pixels, 8-bit RGBA, top row first.
        rgba: Vec<u8>,
    },
    /// H: what an output plug-in has played so far, by its own clock: the
    /// player's clock for a device that buffers (RFC 0024). Never falls: the
    /// plug-in's own count starting over does not take it back.
    OutputClock {
        /// Milliseconds played since the helper started.
        played_ms: u64,
    },
}

impl Message {
    /// `Hello` for this process.
    pub fn hello() -> Message {
        Message::Hello {
            protocol_version: PROTOCOL_VERSION,
            pid: std::process::id(),
        }
    }

    /// If this is a [`Message::RingOffer`], what an opener validates the ring
    /// against (RFC 0003 section 9).
    pub fn ring_expect(&self) -> Option<RingExpect> {
        match self {
            Message::RingOffer {
                kind,
                frame_size,
                capacity,
                generation,
                ..
            } => Some(RingExpect::new(*kind, *frame_size, *capacity, *generation)),
            _ => None,
        }
    }

    /// A refusal for an offer that could not be satisfied.
    pub fn ring_refused(kind: RingKind, generation: u64, reason: RingRefuseReason) -> Message {
        Message::RingRefused {
            kind,
            generation,
            reason,
        }
    }
}
