//! The [`Event`] enum: notifications published by the player core.
//!
//! Events describe *what happened*. UI, platform integrations and plugin
//! bridges subscribe via [`EventBus`](crate::bus::EventBus).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::ErrorInfo;
use crate::meta::TrackMetadata;
use crate::playlist::{EntryId, PlaylistChangeKind};
use crate::state::{EqState, StreamInfo, TransportState, WindowId};

/// Something that happened in the player.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Event {
    /// Transport state changed.
    StateChanged(TransportState),
    /// The current track changed (or was cleared).
    TrackChanged {
        /// Entry id of the new current track.
        id: Option<EntryId>,
        /// Its index in the playlist.
        index: Option<u32>,
        /// Metadata, if already known.
        metadata: Option<Box<TrackMetadata>>,
        /// Duration in ms, if known.
        duration_ms: Option<u64>,
    },
    /// Playback position update (rate-limited by the publisher). Also sent after seeks.
    PositionChanged {
        /// Position in ms.
        position_ms: u64,
        /// Duration in ms, if known.
        duration_ms: Option<u64>,
    },
    /// Stream technical info changed (bitrate, rate, channels).
    StreamInfoChanged(Option<StreamInfo>),
    /// Volume and/or balance changed.
    VolumeChanged {
        /// Volume `0..=255`.
        volume: u8,
        /// Balance `-127..=127`.
        balance: i8,
    },
    /// Shuffle/repeat flags changed.
    ModesChanged {
        /// Shuffle.
        shuffle: bool,
        /// Repeat.
        repeat: bool,
    },
    /// The playlist changed.
    PlaylistChanged {
        /// What changed.
        kind: PlaylistChangeKind,
        /// New length.
        len: u32,
    },
    /// The equalizer state changed.
    EqChanged(EqState),
    /// A skin was loaded (`None` = built-in default).
    SkinChanged(Option<PathBuf>),
    /// A window was shown/hidden.
    WindowVisibilityChanged {
        /// Which window.
        window: WindowId,
        /// New visibility.
        visible: bool,
    },
    /// Playback reached the end of the playlist.
    PlaylistEnded,
    /// An error occurred.
    Error(ErrorInfo),
    /// The application is quitting.
    Quit,
}
