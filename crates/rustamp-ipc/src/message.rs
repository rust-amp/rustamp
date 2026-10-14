//! The protocol message enum.

use rustamp_core::command::Command;
use rustamp_core::state::PlayerState;
use rustamp_core::vis::{VIS_CHANNELS, VIS_SAMPLES, VisFrame};
use rustamp_plugin_abi::PluginKind;
use serde::{Deserialize, Serialize};

/// Protocol version spoken by this build. Bumped on any incompatible change
/// to [`Message`]; peers compare it in `Hello`.
pub const PROTOCOL_VERSION: u32 = 0;

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
}

impl Message {
    /// `Hello` for this process.
    pub fn hello() -> Message {
        Message::Hello {
            protocol_version: PROTOCOL_VERSION,
            pid: std::process::id(),
        }
    }
}
