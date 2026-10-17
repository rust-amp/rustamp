//! Tag writing.

use std::path::Path;

use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::items::Timestamp;
use lofty::tag::{Accessor, ItemKey, Tag};

use rustamp_core::error::{Error, Result};
use rustamp_core::meta::TrackMetadata;

use crate::read::map_lofty;
use crate::replaygain::item_key_for_extra;

/// The largest year a tag can hold: the readers only accept a four-digit year
/// (`read::parse_year`), so anything larger could not be read back, and
/// `lofty`'s timestamp year is a `u16`, which would silently wrap.
const MAX_YEAR: u32 = 9999;

/// Outcome of [`write_tags_report`]: which `extra` entries the target tag
/// format could not store.
///
/// Standard fields are always written (they map on every primary tag type);
/// only `extra` entries can be refused, and only when the target format has
/// no field for them. Nothing is dropped silently: the refused key names are
/// the report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WriteReport {
    /// `extra` keys that were not written because the target tag format has
    /// no field for them (for example `R128_TRACK_GAIN` on MP3, or
    /// `REPLAYGAIN_REFERENCE_LOUDNESS` everywhere — `lofty` 0.25.4 has no
    /// key for it). Everything else in `extra` was written.
    pub unwritten_extra: Vec<String>,
}

impl WriteReport {
    /// Returns `true` when every requested `extra` entry was written.
    pub fn is_complete(&self) -> bool {
        self.unwritten_extra.is_empty()
    }
}

/// Writes `metadata` into the tags of `path` and reports what could not be
/// stored.
///
/// This is the honest form of [`write_tags`]: fields that are `None` are
/// left untouched, so a read → edit → write cycle keeps every value the
/// caller does not set, and the returned [`WriteReport`] names every `extra`
/// entry the target format refused. A caller that needs to know whether a
/// write happened uses this; [`write_tags`] keeps the old best-effort
/// signature for existing callers.
///
/// ReplayGain keys are looked up in a canonical table (see `replaygain`),
/// not by the format's short key name, so MP4 — whose ilst map only knows
/// the freeform `----:com.apple.iTunes:replaygain_*` form — round-trips the
/// four classic keys instead of dropping them.
///
/// # Errors
///
/// * [`Error::Io`] if the file cannot be opened or written.
/// * [`Error::UnsupportedFormat`] if the file is not a media file whose tags
///   [`lofty`] understands.
/// * [`Error::Other`] if the tag format cannot hold the requested fields.
/// * [`Error::InvalidArgument`] if `year` is above 9999 (before anything is
///   opened; the old behaviour was to wrap it into a `u16`).
pub fn write_tags_report(path: impl AsRef<Path>, metadata: &TrackMetadata) -> Result<WriteReport> {
    let path = path.as_ref();
    // Checked before the file is opened, so a refused call touches nothing.
    if let Some(year) = metadata.year
        && year > MAX_YEAR
    {
        return Err(Error::InvalidArgument(format!(
            "year {year} cannot be written: tags store a four-digit year (at most {MAX_YEAR})"
        )));
    }
    let mut file = Probe::open(path)
        .map_err(map_lofty)?
        .guess_file_type()?
        .options(ParseOptions::new().read_properties(false))
        .read()
        .map_err(map_lofty)?;

    let primary = file.primary_tag_type();
    let mut tag = file.remove(primary).unwrap_or_else(|| Tag::new(primary));
    let unwritten_extra = apply(&mut tag, metadata);
    file.insert_tag(tag);

    file.save_to_path(path, WriteOptions::default())
        .map_err(map_lofty)?;
    Ok(WriteReport { unwritten_extra })
}

/// Writes `metadata` into the tags of `path`.
///
/// Fields that are `None` are left untouched, so a read → edit → write cycle
/// keeps every value the caller does not set. If the file has no tag of its
/// format's primary type yet, one is created. Writing rewrites tag blocks
/// only; the audio is never decoded or re-encoded.
///
/// The `year` is written as a recording date (ID3v2 `TDRC`, Vorbis `DATE`,
/// MP4 `©day`). Items in [`TrackMetadata::extra`] are written best-effort:
/// entries whose key maps to a field of the target tag format are set, the
/// rest are ignored — silently. Callers that need to know what was skipped
/// should use [`write_tags_report`] instead.
///
/// # Errors
///
/// * [`Error::Io`] if the file cannot be opened or written.
/// * [`Error::UnsupportedFormat`] if the file is not a media file whose tags
///   [`lofty`] understands.
/// * [`Error::Other`] if the tag format cannot hold the requested fields.
/// * [`Error::InvalidArgument`] if `year` is above 9999 (before anything is
///   opened; the old behaviour was to wrap it into a `u16`).
pub fn write_tags(path: impl AsRef<Path>, metadata: &TrackMetadata) -> Result<()> {
    write_tags_report(path, metadata).map(|_| ())
}

/// Copies the set fields of `metadata` onto `tag`, returning the `extra`
/// keys the tag could not store.
fn apply(tag: &mut Tag, metadata: &TrackMetadata) -> Vec<String> {
    if let Some(value) = &metadata.title {
        tag.set_title(value.clone());
    }
    if let Some(value) = &metadata.artist {
        tag.set_artist(value.clone());
    }
    if let Some(value) = &metadata.album {
        tag.set_album(value.clone());
    }
    if let Some(value) = &metadata.album_artist {
        tag.insert_text(ItemKey::AlbumArtist, value.clone());
    }
    if let Some(value) = &metadata.genre {
        tag.set_genre(value.clone());
    }
    if let Some(value) = &metadata.comment {
        tag.set_comment(value.clone());
    }
    if let Some(year) = metadata.year {
        tag.set_date(Timestamp {
            year: year as u16,
            ..Timestamp::default()
        });
    }
    if let Some(number) = metadata.track_no {
        tag.set_track(number);
    }
    if let Some(total) = metadata.track_total {
        tag.set_track_total(total);
    }
    if let Some(number) = metadata.disc_no {
        tag.set_disk(number);
    }

    let mut unwritten = Vec::new();
    for (key, value) in &metadata.extra {
        // The canonical table first: it carries the ReplayGain keys even
        // where the format's short key name does not resolve (MP4 ilst).
        // Anything else falls back to the format's own key table.
        let item_key = item_key_for_extra(key).or_else(|| ItemKey::from_key(tag.tag_type(), key));
        match item_key {
            Some(item_key) if tag.insert_text(item_key, value.clone()) => {}
            _ => unwritten.push(key.clone()),
        }
    }
    unwritten
}
