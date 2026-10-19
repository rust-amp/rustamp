//! Album grouping: which tracks belong to one album, and the album-level
//! ReplayGain values of that album.
//!
//! The crate reads tags one file at a time; an album value, though, is a
//! property of a *group* of files. This module draws that group and
//! computes the two album values from it:
//!
//! * [`group_albums`] groups the tracks and computes
//!   [`AlbumValues`] for every group.
//! * [`AlbumGrouping::values_for`] answers the album values that apply to
//!   one track — the mapping a tag writer needs.
//!
//! ## The grouping key
//!
//! Two tracks are in the same album when their [`AlbumKey`]s are equal:
//!
//! * A track **with** an album tag is keyed by the album title and the
//!   *effective album artist*: the `album_artist` tag, or — when that tag
//!   is missing — the `artist` tag. Both are trimmed and lower-cased, so
//!   `"Abbey Road"` and `" abbey road "` are one album and a differently
//!   cased album artist does not split an album.
//! * A track **without** an album tag is **its own album**, keyed by its
//!   path. Two untagged tracks never share an album: nothing says they
//!   belong together, and merging them would give one of them the wrong
//!   album gain.
//!
//! The fallback is per track, and that is deliberate: a track that names
//! the album artist and a track that names only the artist end up in the
//! same album when that artist *is* the album artist, but a track whose
//! artist differs from the album's artist starts its own group instead of
//! dragging the album's gain onto it.
//!
//! ## The album values
//!
//! * **Album gain**: the negative of the 95th percentile of the pooled
//!   per-block loudness values of the album's tracks, clamped to ±51 dB.
//!   The blocks are pooled through their loudness histograms, so the
//!   percentile is resolved on the 0.01 dB histogram grid and can differ
//!   from the exact percentile by at most 0.005 dB. An album whose blocks
//!   all sit at or below -99 dB is silent and measures as the neutral
//!   0.0 dB.
//! * **Album peak**: the largest track peak of the album.
//!
//! A one-track album therefore reproduces that track's gain (to within the
//! histogram's bin width) and its peak.
//!
//! ## What the caller does
//!
//! The grouping is pure: it reads no file and touches no audio. The caller
//! reads the tags, analyzes the audio and hands the result in. Writing the
//! album values back is the caller's job as well — the crate has
//! [`write_replaygain`](crate::replaygain::write_replaygain) for it.
//!
//! ```no_run
//! # fn main() -> rustamp_core::Result<()> {
//! # let metadata = rustamp_core::meta::TrackMetadata::default();
//! # let samples = vec![0.0f32; 4_410];
//! use rustamp_meta::album::{AnalyzedTrack, group_albums};
//! use rustamp_meta::replaygain::{ReplayGainTags, analyze_replaygain, write_replaygain};
//!
//! // The caller knows the tags of the file and has analyzed its audio.
//! let track = AnalyzedTrack::new(
//!     "song.flac",
//!     metadata,
//!     analyze_replaygain(&samples, 2, 44_100)?,
//! );
//!
//! // The grouping and the album values of every album.
//! let grouping = group_albums(vec![track]);
//! for album in grouping.albums() {
//!     for track in &album.tracks {
//!         let tags = ReplayGainTags {
//!             album_gain_db: Some(album.values.album_gain_db),
//!             album_peak: Some(album.values.album_peak),
//!             ..ReplayGainTags::default()
//!         };
//!         write_replaygain(&track.path, &tags)?;
//!     }
//! }
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rustamp_core::meta::TrackMetadata;

use crate::replaygain::{ReplayGainAnalysis, album_gain_db, album_peak};

/// A track together with the loudness analysis of its audio.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalyzedTrack {
    /// The track's path: its identity in the grouping and the key of a
    /// track without an album tag.
    pub path: PathBuf,
    /// The track's tags.
    pub metadata: TrackMetadata,
    /// The analysis of the track's audio.
    pub analysis: ReplayGainAnalysis,
}

impl AnalyzedTrack {
    /// Builds an analyzed track from its path, its tags and its analysis.
    pub fn new(
        path: impl Into<PathBuf>,
        metadata: TrackMetadata,
        analysis: ReplayGainAnalysis,
    ) -> Self {
        Self {
            path: path.into(),
            metadata,
            analysis,
        }
    }
}

/// The key an album group is formed by.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AlbumKey {
    /// A tagged album: the normalized album title and the normalized
    /// effective album artist.
    Tagged {
        /// The album title, trimmed and lower-cased.
        title: String,
        /// The effective album artist, trimmed and lower-cased: the
        /// `album_artist` tag, or the `artist` tag when the album artist is
        /// missing.
        artist: String,
    },
    /// A track without an album tag is its own album, keyed by its path.
    Untagged {
        /// The path of the track.
        path: PathBuf,
    },
}

/// The album-level values of one album.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AlbumValues {
    /// The album gain in dB: the negative of the 95th percentile of the
    /// pooled per-block loudness values of the album's tracks.
    pub album_gain_db: f32,
    /// The album peak, linear: the largest track peak of the album.
    pub album_peak: f32,
}

/// One album: the tracks that belong to it and the values computed for it.
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumGroup {
    /// The key the group was formed by.
    pub key: AlbumKey,
    /// The tracks of the album, in the order they were given.
    pub tracks: Vec<AnalyzedTrack>,
    /// The album values, computed from the tracks of this group.
    pub values: AlbumValues,
}

/// The result of a grouping run: the album groups and the per-track mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct AlbumGrouping {
    albums: Vec<AlbumGroup>,
    by_path: HashMap<PathBuf, usize>,
}

impl AlbumGrouping {
    /// The album groups, in the order their first track appeared.
    pub fn albums(&self) -> &[AlbumGroup] {
        &self.albums
    }

    /// The number of album groups.
    pub fn len(&self) -> usize {
        self.albums.len()
    }

    /// Returns `true` when there is no album group.
    pub fn is_empty(&self) -> bool {
        self.albums.is_empty()
    }

    /// The album values that apply to `path`.
    ///
    /// `None` when the path is not one of the grouped tracks. When the same
    /// path was grouped twice, the values of the group it landed in last
    /// are answered.
    pub fn values_for(&self, path: &Path) -> Option<&AlbumValues> {
        let index = *self.by_path.get(path)?;
        self.albums.get(index).map(|album| &album.values)
    }
}

/// Groups `tracks` into albums and computes the album values of every group.
///
/// The groups appear in the order their first track was given, and the
/// tracks of a group keep the order they were given in. The groups own the
/// tracks they hold, so the caller gets the analyses back with them.
///
/// The function is pure: it reads no file, opens no audio and needs no
/// audio hardware — the caller hands in the tags and the analysis of every
/// track.
///
/// See the [module docs](self) for the grouping key and the aggregation
/// rule.
pub fn group_albums(tracks: Vec<AnalyzedTrack>) -> AlbumGrouping {
    let mut albums: Vec<AlbumGroup> = Vec::new();
    let mut index_of: HashMap<AlbumKey, usize> = HashMap::new();

    for track in tracks {
        let key = album_key(&track.path, &track.metadata);
        match index_of.get(&key) {
            Some(&index) => albums[index].tracks.push(track),
            None => {
                index_of.insert(key.clone(), albums.len());
                albums.push(AlbumGroup {
                    key,
                    tracks: vec![track],
                    values: AlbumValues {
                        album_gain_db: 0.0,
                        album_peak: 0.0,
                    },
                });
            }
        }
    }

    for album in &mut albums {
        album.values = album_values(&album.tracks);
    }

    let mut by_path = HashMap::with_capacity(albums.len());
    for (index, album) in albums.iter().enumerate() {
        for track in &album.tracks {
            by_path.insert(track.path.clone(), index);
        }
    }

    AlbumGrouping { albums, by_path }
}

/// The album key of a track.
///
/// The key of a track with an album tag is the normalized album title plus
/// the effective album artist — the `album_artist` tag, or the `artist`
/// tag when the album artist is missing. A track without an album tag is
/// keyed by its path and is its own album.
pub fn album_key(path: &Path, metadata: &TrackMetadata) -> AlbumKey {
    match non_empty(metadata.album.as_deref()) {
        None => AlbumKey::Untagged {
            path: path.to_path_buf(),
        },
        Some(title) => AlbumKey::Tagged {
            title: normalize(title),
            artist: normalize(effective_album_artist(metadata)),
        },
    }
}

/// The album values of a group of tracks.
fn album_values(tracks: &[AnalyzedTrack]) -> AlbumValues {
    let analyses: Vec<&ReplayGainAnalysis> = tracks.iter().map(|track| &track.analysis).collect();
    AlbumValues {
        album_gain_db: album_gain_db(&analyses),
        album_peak: album_peak(&analyses),
    }
}

/// The album artist of a track: the `album_artist` tag, or the `artist`
/// tag when the album artist is missing.
fn effective_album_artist(metadata: &TrackMetadata) -> &str {
    non_empty(metadata.album_artist.as_deref())
        .or_else(|| non_empty(metadata.artist.as_deref()))
        .unwrap_or("")
}

/// The trimmed text of a tag, or `None` when the tag is missing or empty.
fn non_empty(text: Option<&str>) -> Option<&str> {
    let trimmed = text?.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// The comparison form of a tag value: trimmed and lower-cased.
fn normalize(text: &str) -> String {
    text.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::replaygain::analyze_replaygain;

    const SAMPLE_RATE: u32 = 44_100;

    /// A sine at 440 Hz, `amplitude` peak, with whole cycles over the
    /// buffer, so its RMS is `amplitude / sqrt(2)`.
    fn sine(amplitude: f32) -> Vec<f32> {
        (0..SAMPLE_RATE as usize)
            .map(|i| {
                amplitude
                    * (2.0 * core::f32::consts::PI * 440.0 * i as f32 / SAMPLE_RATE as f32).sin()
            })
            .collect()
    }

    /// An analyzed track with the given tags and a sine of `amplitude`.
    fn track(path: &str, amplitude: f32, metadata: TrackMetadata) -> AnalyzedTrack {
        let analysis = analyze_replaygain(&sine(amplitude), 1, SAMPLE_RATE).unwrap();
        AnalyzedTrack::new(path, metadata, analysis)
    }

    /// Tags of a track of an album.
    fn album_tags(album: &str, album_artist: Option<&str>, artist: Option<&str>) -> TrackMetadata {
        TrackMetadata {
            album: Some(album.to_owned()),
            album_artist: album_artist.map(str::to_owned),
            artist: artist.map(str::to_owned),
            ..TrackMetadata::default()
        }
    }

    #[test]
    fn two_albums_get_their_own_gain() {
        let tracks = vec![
            track("a1.flac", 0.5, album_tags("First", Some("Artist"), None)),
            track("a2.flac", 0.5, album_tags("First", Some("Artist"), None)),
            track("b1.flac", 0.1, album_tags("Second", Some("Artist"), None)),
        ];
        let grouping = group_albums(tracks);
        assert_eq!(grouping.len(), 2);

        let first = &grouping.albums()[0];
        assert_eq!(first.tracks.len(), 2);
        assert_eq!(
            first.key,
            AlbumKey::Tagged {
                title: "first".to_owned(),
                artist: "artist".to_owned(),
            }
        );
        // A half-scale sine asks for about +6 dB.
        assert!(
            (first.values.album_gain_db - 6.02).abs() < 0.05,
            "album gain = {}",
            first.values.album_gain_db
        );

        let second = &grouping.albums()[1];
        assert_eq!(second.tracks.len(), 1);
        // A tenth-scale sine asks for about +20 dB.
        assert!(
            (second.values.album_gain_db - 20.0).abs() < 0.05,
            "album gain = {}",
            second.values.album_gain_db
        );
        assert!(second.values.album_gain_db > first.values.album_gain_db);
    }

    #[test]
    fn a_one_track_album_reproduces_the_track_gain() {
        let track = track("a1.flac", 0.5, album_tags("First", Some("Artist"), None));
        let track_gain = track.analysis.track_gain_db;
        let track_peak = track.analysis.track_peak;
        let grouping = group_albums(vec![track]);
        let album = &grouping.albums()[0];
        // The album percentile is resolved on the 0.01 dB histogram grid,
        // so the two can differ by one bin.
        assert!(
            (album.values.album_gain_db - track_gain).abs() <= 0.01,
            "album gain = {}, track gain = {}",
            album.values.album_gain_db,
            track_gain
        );
        assert_eq!(album.values.album_peak, track_peak);
    }

    #[test]
    fn a_track_without_an_album_tag_is_its_own_album() {
        let tracks = vec![
            track("loose1.flac", 0.5, TrackMetadata::default()),
            track("loose2.flac", 0.5, TrackMetadata::default()),
        ];
        let grouping = group_albums(tracks);
        assert_eq!(grouping.len(), 2);
        for album in grouping.albums() {
            assert_eq!(album.tracks.len(), 1);
            assert!(matches!(album.key, AlbumKey::Untagged { .. }));
        }
        assert_eq!(
            grouping.albums()[0].key,
            AlbumKey::Untagged {
                path: PathBuf::from("loose1.flac")
            }
        );
        assert_eq!(
            grouping.albums()[1].key,
            AlbumKey::Untagged {
                path: PathBuf::from("loose2.flac")
            }
        );
    }

    #[test]
    fn the_album_artist_falls_back_to_the_artist() {
        let tracks = vec![
            // Names the album artist.
            track(
                "a1.flac",
                0.5,
                album_tags("First", Some("Artist"), Some("Guest")),
            ),
            // Names only the artist, which is the album artist here.
            track("a2.flac", 0.5, album_tags("First", None, Some("Artist"))),
            // A different artist is a different album.
            track("a3.flac", 0.5, album_tags("First", None, Some("Other"))),
        ];
        let grouping = group_albums(tracks);
        assert_eq!(grouping.len(), 2);
        assert_eq!(grouping.albums()[0].tracks.len(), 2);
        assert_eq!(grouping.albums()[1].tracks.len(), 1);
        assert_eq!(
            grouping.albums()[0].key,
            AlbumKey::Tagged {
                title: "first".to_owned(),
                artist: "artist".to_owned(),
            }
        );
        assert_eq!(
            grouping.albums()[1].key,
            AlbumKey::Tagged {
                title: "first".to_owned(),
                artist: "other".to_owned(),
            }
        );
    }

    #[test]
    fn the_album_peak_is_the_largest_track_peak() {
        let tracks = vec![
            track("a1.flac", 0.5, album_tags("First", Some("Artist"), None)),
            track("a2.flac", 0.9, album_tags("First", Some("Artist"), None)),
        ];
        let grouping = group_albums(tracks);
        let album = &grouping.albums()[0];
        assert!((album.values.album_peak - 0.9).abs() < 0.001);
        assert!(
            (album.values.album_peak - album.tracks[1].analysis.track_peak).abs() < f32::EPSILON
        );
    }

    #[test]
    fn the_values_of_a_track_are_the_values_of_its_album() {
        let tracks = vec![
            track("a1.flac", 0.5, album_tags("First", Some("Artist"), None)),
            track("b1.flac", 0.1, album_tags("Second", Some("Artist"), None)),
        ];
        let grouping = group_albums(tracks);
        let first = &grouping.albums()[0].values;
        let second = &grouping.albums()[1].values;
        assert_eq!(grouping.values_for(Path::new("a1.flac")), Some(first));
        assert_eq!(grouping.values_for(Path::new("b1.flac")), Some(second));
        assert_eq!(grouping.values_for(Path::new("missing.flac")), None);
        assert!(first.album_gain_db < second.album_gain_db);
    }

    #[test]
    fn the_album_title_is_compared_without_case_and_padding() {
        let tracks = vec![
            track(
                "a1.flac",
                0.5,
                album_tags("Abbey Road", Some("Artist"), None),
            ),
            track(
                "a2.flac",
                0.5,
                album_tags(" abbey road ", Some("artist"), None),
            ),
        ];
        let grouping = group_albums(tracks);
        assert_eq!(grouping.len(), 1);
        assert_eq!(grouping.albums()[0].tracks.len(), 2);
    }

    #[test]
    fn an_empty_input_has_no_album() {
        let grouping = group_albums(Vec::new());
        assert!(grouping.is_empty());
        assert_eq!(grouping.len(), 0);
        assert!(grouping.albums().is_empty());
        assert_eq!(grouping.values_for(Path::new("a1.flac")), None);
    }
}
