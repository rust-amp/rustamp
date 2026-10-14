//! Player state snapshot types.

use serde::{Deserialize, Serialize};

use crate::playlist::EntryId;

/// Number of equalizer bands (60, 170, 310, 600, 1k, 3k, 6k, 12k, 14k, 16k Hz;
/// exact centers are defined by spec S07).
pub const EQ_BANDS: usize = 10;

/// Identifies one of the player's windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WindowId {
    /// Main player window.
    Main,
    /// Equalizer window.
    Equalizer,
    /// Playlist editor.
    Playlist,
    /// Visualization window.
    Vis,
    /// Generic / plugin window (`gen_*` plugins, library, ...), by number.
    Gen(u32),
}

/// Transport state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TransportState {
    /// Nothing playing.
    #[default]
    Stopped,
    /// Playing.
    Playing,
    /// Paused.
    Paused,
}

/// Technical info about the current stream, shown in the main window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct StreamInfo {
    /// Bitrate in kbps, if known.
    pub bitrate_kbps: Option<u32>,
    /// Sample rate in Hz (the UI shows kHz).
    pub sample_rate_hz: u32,
    /// Channel count (1 = mono, 2 = stereo).
    pub channels: u16,
}

/// Equalizer state. Gains are in dB, nominally `-12.0..=12.0`; mapping to the
/// slider scale is a UI concern (spec S07).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EqState {
    /// Per-band gain in dB.
    pub bands: [f32; EQ_BANDS],
    /// Preamp gain in dB.
    pub preamp: f32,
    /// EQ processing enabled ("ON" button).
    pub enabled: bool,
    /// Auto-load presets per track ("AUTO" button).
    pub auto: bool,
}

impl Default for EqState {
    fn default() -> Self {
        EqState {
            bands: [0.0; EQ_BANDS],
            preamp: 0.0,
            enabled: false,
            auto: false,
        }
    }
}

impl EqState {
    /// Gain limit in dB (absolute value) accepted by the engine.
    pub const MAX_GAIN_DB: f32 = 12.0;
}

/// Snapshot of the whole player; the app state store publishes these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerState {
    /// Transport state.
    pub transport: TransportState,
    /// Current position in milliseconds.
    pub position_ms: u64,
    /// Duration of the current track in ms, if known.
    pub duration_ms: Option<u64>,
    /// Volume, `0..=255` (Winamp style).
    pub volume: u8,
    /// Balance, `-127..=127` (negative = left).
    pub balance: i8,
    /// Shuffle enabled.
    pub shuffle: bool,
    /// Repeat enabled.
    pub repeat: bool,
    /// Index of the current playlist entry.
    pub current_index: Option<u32>,
    /// Stable id of the current playlist entry.
    pub current_id: Option<EntryId>,
    /// Number of playlist entries.
    pub playlist_len: u32,
    /// Info about the current stream.
    pub stream: Option<StreamInfo>,
    /// Equalizer state.
    pub eq: EqState,
}

impl Default for PlayerState {
    fn default() -> Self {
        PlayerState {
            transport: TransportState::Stopped,
            position_ms: 0,
            duration_ms: None,
            volume: 255,
            balance: 0,
            shuffle: false,
            repeat: false,
            current_index: None,
            current_id: None,
            playlist_len: 0,
            stream: None,
            eq: EqState::default(),
        }
    }
}
