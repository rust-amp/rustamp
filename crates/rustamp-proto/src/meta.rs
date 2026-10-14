//! Track metadata.
//!
//! [`TrackMetadata`] travels on the wire inside `Event::TrackChanged`. Reading
//! and writing tags, and Winamp's title formatting, live in `rustamp-meta`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Metadata of a track, as read by `rustamp-meta` or a decoder.
///
/// All fields are optional; absent means unknown.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TrackMetadata {
    /// Track title.
    pub title: Option<String>,
    /// Artist.
    pub artist: Option<String>,
    /// Album.
    pub album: Option<String>,
    /// Album artist.
    pub album_artist: Option<String>,
    /// Release year.
    pub year: Option<u32>,
    /// Genre (free text).
    pub genre: Option<String>,
    /// Comment.
    pub comment: Option<String>,
    /// Track number.
    pub track_no: Option<u32>,
    /// Total tracks.
    pub track_total: Option<u32>,
    /// Disc number.
    pub disc_no: Option<u32>,
    /// Duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Bitrate in kbps.
    pub bitrate_kbps: Option<u32>,
    /// Additional tag fields (key upper-case by convention), e.g. `REPLAYGAIN_TRACK_GAIN`.
    pub extra: BTreeMap<String, String>,
}

impl TrackMetadata {
    /// Returns `true` if no field is set.
    pub fn is_empty(&self) -> bool {
        *self == TrackMetadata::default()
    }

    /// Simple `"Artist - Title"` display string. Winamp's full title format
    /// (spec S14) lives in `rustamp-meta`; this is only a fallback.
    pub fn display_title(&self) -> Option<String> {
        match (&self.artist, &self.title) {
            (Some(a), Some(t)) => Some(format!("{a} - {t}")),
            (None, Some(t)) => Some(t.clone()),
            _ => None,
        }
    }
}
