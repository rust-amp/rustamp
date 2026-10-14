//! The [`Command`] enum: every user or plugin intent that mutates the player.
//!
//! Commands are *requests*; the app/engine applies them and answers with
//! [`Event`](crate::event::Event)s. They are `serde`-serializable so that the
//! plugin host can forward them over IPC. Indices are `u32`; entries are
//! referenced by stable [`EntryId`] where possible.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::playlist::{EntryId, MediaLocation, SortKey};
use crate::state::WindowId;

/// Selection operations on the playlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelectOp {
    /// Replace the selection with exactly these entries.
    Set(Vec<EntryId>),
    /// Toggle one entry.
    Toggle(EntryId),
    /// Select all.
    All,
    /// Select none.
    None,
    /// Invert.
    Invert,
}

/// A command to the player.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    // ----- transport -----
    /// Play (resume if paused, restart current if playing).
    Play,
    /// Pause (no-op unless playing).
    Pause,
    /// Toggle between playing and paused.
    TogglePause,
    /// Stop.
    Stop,
    /// Stop with a fade-out (duration from config).
    StopWithFadeout,
    /// Next track.
    Next,
    /// Previous track.
    Prev,
    /// Seek to an absolute position in ms.
    Seek {
        /// Target position in ms.
        to_ms: u64,
    },
    /// Seek relative to the current position (negative = backwards).
    SeekRelative {
        /// Delta in ms.
        delta_ms: i64,
    },
    /// Set volume, `0..=255` (Winamp style).
    SetVolume(u8),
    /// Set balance, `-127..=127`; values outside are clamped.
    SetBalance(i8),
    /// Toggle shuffle.
    ToggleShuffle,
    /// Toggle repeat.
    ToggleRepeat,
    /// Set shuffle explicitly.
    SetShuffle(bool),
    /// Set repeat explicitly.
    SetRepeat(bool),

    // ----- playlist -----
    /// Add files/URLs. `at = None` appends. `play` starts the first added entry.
    AddItems {
        /// Locations to add.
        items: Vec<MediaLocation>,
        /// Insert position (clamped); `None` = end.
        at: Option<u32>,
        /// Start playing the first new entry.
        play: bool,
    },
    /// Remove entries.
    Remove(Vec<EntryId>),
    /// Remove all entries.
    Clear,
    /// Move entries as a block; the first lands at `to` in the list without the moved entries.
    MoveEntries {
        /// Entries to move.
        ids: Vec<EntryId>,
        /// Target index.
        to: u32,
    },
    /// Selection change.
    Select(SelectOp),
    /// Play the entry at an index.
    PlayIndex(u32),
    /// Play a specific entry.
    PlayEntry(EntryId),
    /// Sort the playlist.
    Sort(SortKey),
    /// Randomize the order.
    Randomize,
    /// Reverse the order.
    Reverse,

    // ----- equalizer -----
    /// Set one band (`band < EQ_BANDS`) gain in dB (clamped to +-12).
    SetBand {
        /// Band index `0..10`.
        band: u8,
        /// Gain in dB.
        gain_db: f32,
    },
    /// Set preamp gain in dB.
    SetPreamp(f32),
    /// Enable/disable EQ.
    SetEqEnabled(bool),
    /// Enable/disable EQ auto-load.
    SetEqAuto(bool),

    // ----- windows / UI -----
    /// Show/hide a window.
    ToggleWindow(WindowId),
    /// Toggle double-size mode.
    ToggleDoubleSize,
    /// Toggle windowshade for a window.
    ToggleWindowShade(WindowId),
    /// Toggle always-on-top.
    ToggleAlwaysOnTop,
    /// Load a skin from a `.wsz`/`.zip` or folder.
    LoadSkin(PathBuf),

    // ----- app -----
    /// Quit the application.
    Quit,
    /// Open the preferences dialog.
    OpenPrefs,
}
