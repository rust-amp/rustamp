//! The protocol message enum.

use rustamp_plugin_abi::PluginKind;
use rustamp_proto::command::Command;
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
///   RFC 0006 takes this to 3 when it lands (sequential bumps).
pub const PROTOCOL_VERSION: u32 = 2;

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
    /// Appended at the end of the enum so every earlier `Message` keeps its
    /// variant index (and its bytes). **Placeholder overlap:** how this relates
    /// to `WinampIpc` is S10's to resolve (RFC 0005 O13); the handling lands
    /// with M3.
    CommandToHost(Command),
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
