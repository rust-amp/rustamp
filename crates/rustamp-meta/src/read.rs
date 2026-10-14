//! Tag reading.
//!
//! Everything here parses tag containers only; no audio is decoded. See the
//! module docs of the crate and `CONTRACT_NOTES.md` for the merge and
//! encoding rules.

use std::path::{Path, PathBuf};

use lofty::config::ParseOptions;
use lofty::error::{ErrorKind as LoftyErrorKind, LoftyError};
use lofty::file::{TaggedFile, TaggedFileExt};
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag, TagType};

use rustamp_core::error::{Error, Result};
use rustamp_core::meta::TrackMetadata;

use crate::artwork::Artwork;

/// Tag types in descending precedence when a file carries more than one tag.
///
/// The file format's *primary* tag always comes first (see
/// [`FileType::primary_tag_type`](lofty::file::FileType::primary_tag_type));
/// the remaining tags follow in this order. ID3v2 therefore beats ID3v1, and
/// ID3v1 only fills the fields the other tags leave empty.
const TAG_PRECEDENCE: &[TagType] = &[
    TagType::Id3v2,
    TagType::VorbisComments,
    TagType::Mp4Ilst,
    TagType::Ape,
    TagType::Id3v1,
    TagType::RiffInfo,
    TagType::AiffText,
];

/// Additional items copied verbatim into [`TrackMetadata::extra`], keyed by
/// their conventional upper-case name.
const EXTRA_ITEMS: &[(ItemKey, &str)] = &[
    (ItemKey::ReplayGainTrackGain, "REPLAYGAIN_TRACK_GAIN"),
    (ItemKey::ReplayGainTrackPeak, "REPLAYGAIN_TRACK_PEAK"),
    (ItemKey::ReplayGainAlbumGain, "REPLAYGAIN_ALBUM_GAIN"),
    (ItemKey::ReplayGainAlbumPeak, "REPLAYGAIN_ALBUM_PEAK"),
    (ItemKey::Isrc, "ISRC"),
    (ItemKey::Barcode, "BARCODE"),
    (ItemKey::CatalogNumber, "CATALOGNUMBER"),
    (ItemKey::Bpm, "BPM"),
    (ItemKey::IntegerBpm, "BPM"),
    (ItemKey::InitialKey, "INITIALKEY"),
    (ItemKey::Label, "LABEL"),
    (ItemKey::Composer, "COMPOSER"),
    (ItemKey::Conductor, "CONDUCTOR"),
    (ItemKey::Lyricist, "LYRICIST"),
    (ItemKey::Language, "LANGUAGE"),
    (ItemKey::CopyrightMessage, "COPYRIGHT"),
    (ItemKey::EncodedBy, "ENCODEDBY"),
    (ItemKey::Mood, "MOOD"),
    (ItemKey::MusicBrainzRecordingId, "MUSICBRAINZ_TRACKID"),
    (ItemKey::MusicBrainzReleaseId, "MUSICBRAINZ_ALBUMID"),
    (ItemKey::MusicBrainzArtistId, "MUSICBRAINZ_ARTISTID"),
    (
        ItemKey::MusicBrainzReleaseArtistId,
        "MUSICBRAINZ_ALBUMARTISTID",
    ),
    (
        ItemKey::MusicBrainzReleaseGroupId,
        "MUSICBRAINZ_RELEASEGROUPID",
    ),
    (ItemKey::MusicBrainzWorkId, "MUSICBRAINZ_WORKID"),
];

/// Reads the tags of `path` into [`TrackMetadata`].
///
/// Only tag data is read: no audio is decoded and no audio properties are
/// computed, so [`TrackMetadata::duration_ms`] and
/// [`TrackMetadata::bitrate_kbps`] always stay `None` here (the decoder fills
/// them during playback). A file without tags yields
/// [`TrackMetadata::default()`]; a file that is not media yields an error.
///
/// When a file carries more than one tag, they are merged field by field: the
/// format's primary tag wins, then the tags follow
/// [`TAG_PRECEDENCE`] (ID3v2, Vorbis comments, MP4, APE, ID3v1, RIFF INFO,
/// AIFF text). The first tag that supplies a non-empty value for a field wins.
///
/// # Errors
///
/// * [`Error::Io`] if the file cannot be opened or read.
/// * [`Error::UnsupportedFormat`] if the file is not a media file whose tags
///   [`lofty`] understands.
pub fn read_tags(path: impl AsRef<Path>) -> Result<TrackMetadata> {
    let file = load(path.as_ref(), false)?;
    Ok(metadata_from(&file))
}

/// Reads the embedded pictures of `path`.
///
/// The picture bytes are returned verbatim; see [`Artwork`]. Pictures from
/// every tag are returned in the same precedence order as [`read_tags`].
///
/// # Errors
///
/// See [`read_tags`].
pub fn read_artwork(path: impl AsRef<Path>) -> Result<Vec<Artwork>> {
    let file = load(path.as_ref(), true)?;
    Ok(artwork_from(&file))
}

/// Reads the tags of many paths, returning one result per path.
///
/// This is the batch entry point for folder scans: an unreadable or non-media
/// file yields an `Err` for its own entry and does not abort the scan. The
/// function keeps no global state and never panics, so it is safe to call from
/// a background thread.
pub fn scan_paths<I, P>(paths: I) -> Vec<(PathBuf, Result<TrackMetadata>)>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    paths
        .into_iter()
        .map(|path| {
            let path = path.as_ref().to_path_buf();
            let metadata = read_tags(&path);
            (path, metadata)
        })
        .collect()
}

/// Opens `path`, guesses its format and parses its tags.
fn load(path: &Path, read_cover_art: bool) -> Result<TaggedFile> {
    let options = ParseOptions::new()
        .read_properties(false)
        .read_cover_art(read_cover_art);

    Probe::open(path)
        .map_err(map_lofty)?
        .guess_file_type()?
        .options(options)
        .read()
        .map_err(map_lofty)
}

/// Maps a [`LoftyError`] onto the shared [`Error`] type.
pub(crate) fn map_lofty(error: LoftyError) -> Error {
    match error.kind() {
        LoftyErrorKind::UnknownFormat => Error::UnsupportedFormat(error.to_string()),
        LoftyErrorKind::Io(io) => Error::Io(std::io::Error::new(io.kind(), io.to_string())),
        _ => Error::Other(error.to_string()),
    }
}

/// The file's tags, highest precedence first.
fn ordered_tags(file: &TaggedFile) -> Vec<&Tag> {
    let mut ordered: Vec<&Tag> = Vec::with_capacity(file.tags().len());
    add_tag(&mut ordered, file, file.primary_tag_type());
    for &tag_type in TAG_PRECEDENCE {
        add_tag(&mut ordered, file, tag_type);
    }
    // Custom or otherwise unknown tag types come last, in file order.
    for tag in file.tags() {
        if !ordered.iter().any(|t| t.tag_type() == tag.tag_type()) {
            ordered.push(tag);
        }
    }
    ordered
}

fn add_tag<'a>(ordered: &mut Vec<&'a Tag>, file: &'a TaggedFile, tag_type: TagType) {
    if ordered.iter().any(|t| t.tag_type() == tag_type) {
        return;
    }
    if let Some(tag) = file.tag(tag_type) {
        ordered.push(tag);
    }
}

/// Maps the merged tags of `file` onto [`TrackMetadata`].
fn metadata_from(file: &TaggedFile) -> TrackMetadata {
    let tags = ordered_tags(file);
    let mut metadata = TrackMetadata {
        title: text(&tags, ItemKey::TrackTitle),
        artist: text(&tags, ItemKey::TrackArtist),
        album: text(&tags, ItemKey::AlbumTitle),
        album_artist: text(&tags, ItemKey::AlbumArtist),
        genre: text(&tags, ItemKey::Genre),
        // Some taggers (for example ffmpeg's FLAC muxer) write the comment as
        // the Vorbis `DESCRIPTION` item, so fall back to it.
        comment: text(&tags, ItemKey::Comment).or_else(|| text(&tags, ItemKey::Description)),
        ..TrackMetadata::default()
    };

    metadata.year = date(&tags).and_then(|value| parse_year(&value));

    let (track_no, track_total) = number_pair(&tags, ItemKey::TrackNumber, ItemKey::TrackTotal);
    metadata.track_no = track_no;
    metadata.track_total = track_total;

    // `TrackMetadata` has no `disc_total` field yet; keep it in `extra` so it
    // is not lost. See `CONTRACT_NOTES.md` for the proposed core change.
    let (disc_no, disc_total) = number_pair(&tags, ItemKey::DiscNumber, ItemKey::DiscTotal);
    metadata.disc_no = disc_no;
    if let Some(total) = disc_total {
        metadata
            .extra
            .insert("DISCTOTAL".to_owned(), total.to_string());
    }

    for &(key, name) in EXTRA_ITEMS {
        if metadata.extra.contains_key(name) {
            continue;
        }
        if let Some(value) = text(&tags, key) {
            metadata.extra.insert(name.to_owned(), value);
        }
    }

    metadata
}

/// Collects the pictures of every tag, highest precedence first.
fn artwork_from(file: &TaggedFile) -> Vec<Artwork> {
    let mut pictures = Vec::new();
    for tag in ordered_tags(file) {
        for picture in tag.pictures() {
            pictures.push(Artwork {
                kind: picture.pic_type().as_u8(),
                mime_type: picture.mime_type().map(|mime| mime.as_str().to_owned()),
                description: picture.description().map(str::to_owned),
                data: picture.data().to_vec(),
            });
        }
    }
    pictures
}

/// The first non-empty, trimmed string for `key`, in tag precedence order.
fn text(tags: &[&Tag], key: ItemKey) -> Option<String> {
    tags.iter().find_map(|tag| {
        tag.get_string(key)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

/// The recording date, falling back to the plain year item.
fn date(tags: &[&Tag]) -> Option<String> {
    text(tags, ItemKey::RecordingDate).or_else(|| text(tags, ItemKey::Year))
}

/// Extracts a four-digit year from the start of a date or year string.
fn parse_year(value: &str) -> Option<u32> {
    let value = value.trim();
    let digits: String = value.chars().take(4).collect();
    if digits.len() == 4 && digits.chars().all(|c| c.is_ascii_digit()) {
        digits.parse().ok()
    } else {
        None
    }
}

/// Reads a number/total pair, tolerating both the split (`TRACKNUMBER` +
/// `TRACKTOTAL`) and the combined `"3/9"` form in either item.
fn number_pair(
    tags: &[&Tag],
    number_key: ItemKey,
    total_key: ItemKey,
) -> (Option<u32>, Option<u32>) {
    let (number, pair_total) = match text(tags, number_key) {
        Some(value) => split_number(&value),
        None => (None, None),
    };
    let total = pair_total.or_else(|| text(tags, total_key).and_then(|v| split_number(&v).0));
    (number, total)
}

/// Splits a `"3/9"` pair into its parts, tolerating a bare `"3"`.
fn split_number(value: &str) -> (Option<u32>, Option<u32>) {
    let mut parts = value.split('/');
    let number = parts.next().and_then(|part| part.trim().parse().ok());
    let total = parts.next().and_then(|part| part.trim().parse().ok());
    (number, total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_number_handles_both_forms() {
        assert_eq!(split_number("3/9"), (Some(3), Some(9)));
        assert_eq!(split_number("3"), (Some(3), None));
        assert_eq!(split_number(" 7 / 12 "), (Some(7), Some(12)));
        assert_eq!(split_number("x/y"), (None, None));
        assert_eq!(split_number(""), (None, None));
    }

    #[test]
    fn parse_year_takes_the_leading_four_digits() {
        assert_eq!(parse_year("2001"), Some(2001));
        assert_eq!(parse_year("2001-05-03"), Some(2001));
        assert_eq!(parse_year("2001-05-03T14:30:00"), Some(2001));
        assert_eq!(parse_year(" 1999 "), Some(1999));
        assert_eq!(parse_year("99"), None);
        assert_eq!(parse_year("01.01.2001"), None);
        assert_eq!(parse_year(""), None);
    }
}
