//! Tag writing.

use std::path::Path;

use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::items::Timestamp;
use lofty::tag::{Accessor, ItemKey, Tag};

use rustamp_core::error::Result;
use rustamp_core::meta::TrackMetadata;

use crate::read::map_lofty;

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
/// rest are ignored.
///
/// # Errors
///
/// * [`Error::Io`] if the file cannot be opened or written.
/// * [`Error::UnsupportedFormat`] if the file is not a media file whose tags
///   [`lofty`] understands.
/// * [`Error::Other`] if the tag format cannot hold the requested fields.
pub fn write_tags(path: impl AsRef<Path>, metadata: &TrackMetadata) -> Result<()> {
    let path = path.as_ref();
    let mut file = Probe::open(path)
        .map_err(map_lofty)?
        .guess_file_type()?
        .options(ParseOptions::new().read_properties(false))
        .read()
        .map_err(map_lofty)?;

    let primary = file.primary_tag_type();
    let mut tag = file.remove(primary).unwrap_or_else(|| Tag::new(primary));
    apply(&mut tag, metadata);
    file.insert_tag(tag);

    file.save_to_path(path, WriteOptions::default())
        .map_err(map_lofty)
}

/// Copies the set fields of `metadata` onto `tag`.
fn apply(tag: &mut Tag, metadata: &TrackMetadata) {
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

    for (key, value) in &metadata.extra {
        if let Some(item_key) = ItemKey::from_key(tag.tag_type(), key) {
            tag.insert_text(item_key, value.clone());
        }
    }
}
