// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-meta
//!
//! Tag reading and writing for Rustamp, mapped onto the frozen
//! [`rustamp_core::TrackMetadata`] contract.
//!
//! The crate is deliberately *tags only*:
//!
//! * [`read_tags`] reads the tag fields of a file without decoding any audio.
//! * [`read_artwork`] exposes embedded pictures as raw bytes (no image
//!   decoding, no resizing).
//! * [`scan_paths`] reads many files at once, returning one [`Result`] per
//!   path so a single unreadable file never aborts a folder scan.
//! * [`write_tags`] writes the common fields back, preserving whatever the
//!   caller does not set.
//! * [`write_tags_report`] is the honest form of [`write_tags`]: it names
//!   every `extra` entry the target format could not store.
//! * The [`replaygain`] module holds the ReplayGain keys, the
//!   [`replaygain::ReplayGainTags`] value type, the pure headless analysis
//!   ([`replaygain::analyze_replaygain`]) and an honest ReplayGain writer
//!   ([`replaygain::write_replaygain`]).
//! * [`supported_extensions`] and [`supports_extension`] describe the formats
//!   [`lofty`] handles, next to the decoder registry in `rustamp-decoders`.
//!
//! Reading is done through [`lofty`], which parses the tag containers
//! (ID3v1/v2, Vorbis comments, MP4/ILST, APE, RIFF INFO, AIFF text) but never
//! runs an audio codec. When several tags are present they are merged with a
//! fixed precedence (see [`read_tags`]).
//!
//! ## Out of scope
//!
//! Winamp's title formatting (`%1`–`%7`, spec **S14**) does not exist yet and
//! is **not** implemented here: this crate exposes the raw fields only. The
//! fallback `"Artist - Title"` string lives in
//! [`TrackMetadata::display_title`](rustamp_core::meta::TrackMetadata::display_title)
//! and will be replaced by S14. Whether a playlist entry shows the raw tag
//! title or a formatted string is an S14 decision; see `CONTRACT_NOTES.md`.
//!
//! ## Example
//!
//! ```no_run
//! # fn main() -> rustamp_core::Result<()> {
//! use rustamp_meta::{read_tags, write_tags};
//!
//! let mut metadata = read_tags("song.flac")?;
//! metadata.title = Some("A new title".to_owned());
//! write_tags("song.flac", &metadata)?;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

mod artwork;
mod read;
pub mod replaygain;
mod write;

pub use artwork::Artwork;
pub use read::{read_artwork, read_tags, scan_paths};
pub use replaygain::{
    ALBUM_GAIN_KEY, ALBUM_PEAK_KEY, MAX_GAIN_DB, R128_ALBUM_GAIN_KEY, R128_STEPS_PER_DB,
    R128_TRACK_GAIN_KEY, REFERENCE_LOUDNESS_KEY, REFERENCE_RMS, REPLAYGAIN_KEYS,
    ReplayGainAnalysis, ReplayGainHistogram, ReplayGainTags, ReplayGainWriteReport, TRACK_GAIN_KEY,
    TRACK_PEAK_KEY, album_gain_db, album_gain_from_blocks, album_peak, analyze_replaygain,
    db_to_r128_q8, format_gain_db, format_peak, parse_gain_db, parse_peak, r128_q8_to_db,
    r128_supported_for_extension, replaygain_supported_for_extension,
    replaygain_supported_for_tag_type, write_replaygain,
};
pub use write::{WriteReport, write_tags, write_tags_report};

/// File extensions whose tags [`lofty`] can read.
///
/// This mirrors [`lofty::file::EXTENSIONS`] and is intended to sit next to the
/// decoder registry's own extension list. [`supports_extension`] is the
/// authoritative check; a handful of extra aliases (for example `wave`) are
/// accepted by [`lofty`] without appearing here.
pub fn supported_extensions() -> &'static [&'static str] {
    lofty::file::EXTENSIONS
}

/// Returns whether `ext` (without a leading dot, case-insensitive) is an
/// extension [`lofty`] recognizes.
pub fn supports_extension(ext: &str) -> bool {
    lofty::file::FileType::from_ext(ext).is_some()
}
