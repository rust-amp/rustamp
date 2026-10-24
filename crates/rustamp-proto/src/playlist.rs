//! Playlist wire types.
//!
//! These are the subset of the playlist model that the protocol refers to:
//! entries are named by a stable [`EntryId`], a location is an [`MediaLocation`]
//! and commands/events carry a [`SortKey`] or a [`PlaylistChangeKind`]. The
//! `Playlist` model itself stays in `rustamp-core`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Stable, never-reused identifier of a playlist entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntryId(pub u64);

/// Where a piece of media lives.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaLocation {
    /// A local file.
    File(PathBuf),
    /// A network stream or other URL.
    Url(String),
}

impl MediaLocation {
    /// Final path component / URL tail, for display and sorting.
    pub fn file_name(&self) -> String {
        match self {
            MediaLocation::File(p) => p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            MediaLocation::Url(u) => u.rsplit('/').next().unwrap_or(u).to_owned(),
        }
    }

    /// Lossy string form of the full location.
    pub fn to_display_string(&self) -> String {
        match self {
            MediaLocation::File(p) => p.to_string_lossy().into_owned(),
            MediaLocation::Url(u) => u.clone(),
        }
    }
}

/// Sort keys for `Command::Sort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortKey {
    /// By displayed title (case-insensitive).
    Title,
    /// By file name only.
    FileName,
    /// By full path then file name.
    PathAndFileName,
    /// By duration (unknown first).
    Duration,
}

/// What changed in a playlist, carried by `Event::PlaylistChanged`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaylistChangeKind {
    /// Entries were inserted.
    Inserted,
    /// Entries were removed.
    Removed,
    /// Entries were moved.
    Moved,
    /// The playlist was cleared.
    Cleared,
    /// The whole order changed (sort, reverse, randomize).
    Reordered,
    /// The selection changed.
    SelectionChanged,
    /// Cached title/duration of an entry was updated.
    EntryUpdated,
}
