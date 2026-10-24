//! Tag reading.
//!
//! Everything here parses tag containers only; no audio is decoded. See the
//! module docs of the crate and `CONTRACT_NOTES.md` for the merge and
//! encoding rules.
//!
//! ## The R128 keys of an ID3v2 tag
//!
//! `lofty` 0.25.4 has no `ItemKey` for the `R128_TRACK_GAIN` /
//! `R128_ALBUM_GAIN` keys in ID3v2: a `TXXX` frame is told apart by its
//! description, and only the descriptions `lofty` knows have an `ItemKey`.
//! The generic tag model therefore cannot see such a frame, so the reader
//! looks for it in the concrete ID3v2 tag of the file — which means the
//! tag container of a file with an ID3v2 tag is parsed twice. No other tag
//! is affected, and the R128 values of a file with an ID3v2 tag are the
//! ones its ID3v2 tag holds.

use std::path::{Path, PathBuf};

use lofty::aac::AacFile;
use lofty::config::{ParseOptions, ParsingMode};
use lofty::file::{AudioFile, FileType, TaggedFile, TaggedFileExt};
use lofty::id3::v2::Id3v2Tag;
use lofty::iff::aiff::AiffFile;
use lofty::iff::wav::WavFile;
use lofty::mpeg::MpegFile;
use lofty::picture::Picture;
use lofty::probe::Probe;
use lofty::tag::{ItemKey, Tag, TagType};

use rustamp_core::error::{Error, Result};
use rustamp_core::meta::TrackMetadata;

use crate::artwork::{Artwork, PictureKind};
use crate::replaygain::{R128_ALBUM_GAIN_KEY, R128_TRACK_GAIN_KEY};

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
    // The Opus/R128 loudness variants: carried by the Vorbis family only
    // (see `replaygain` for the per-format table). Reading them here keeps
    // them in `extra`, so they survive a read-write round trip there.
    (ItemKey::R128TrackGain, "R128_TRACK_GAIN"),
    (ItemKey::R128AlbumGain, "R128_ALBUM_GAIN"),
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
/// The `R128_TRACK_GAIN` / `R128_ALBUM_GAIN` values of an ID3v2 tag are read
/// from the concrete tag, because the generic tag model cannot see them (see
/// the module docs).
///
/// # Errors
///
/// * [`Error::Io`] if the file cannot be opened or read.
/// * [`Error::UnsupportedFormat`] if the file is not a media file whose tags
///   [`lofty`] understands.
pub fn read_tags(path: impl AsRef<Path>) -> Result<TrackMetadata> {
    let path = path.as_ref();
    let file = load(path, tag_options())?;
    let mut metadata = metadata_from(&file);

    // The R128 keys of an ID3v2 tag live in `TXXX` frames that the generic
    // tag model cannot see, so they are read from the concrete tag.
    for (key, value) in id3v2_r128(path, file.file_type()) {
        metadata.extra.insert(key.to_owned(), value);
    }

    Ok(metadata)
}

/// The R128 values of the file's ID3v2 tag.
///
/// `lofty` 0.25.4 has no `ItemKey` for the R128 keys in ID3v2: a `TXXX`
/// frame is told apart by its description, and only the descriptions
/// `lofty` knows have an `ItemKey`. The generic tag model therefore cannot
/// see a `R128_TRACK_GAIN` frame — the concrete tag can.
fn id3v2_r128(path: &Path, file_type: FileType) -> Vec<(&'static str, String)> {
    let Some(tag) = concrete_id3v2(path, file_type) else {
        return Vec::new();
    };
    [R128_TRACK_GAIN_KEY, R128_ALBUM_GAIN_KEY]
        .into_iter()
        .filter_map(|key| tag.get_user_text(key).map(|value| (key, value.to_owned())))
        .collect()
}

/// The ID3v2 tag of `path`, read with the concrete reader of its format.
///
/// The ID3v2 tag of an MP3 or an AAC file sits at the front of the file; in
/// a WAV or an AIFF file it sits in a chunk. `lofty` has a reader for each.
fn concrete_id3v2(path: &Path, file_type: FileType) -> Option<Id3v2Tag> {
    let options = ParseOptions::new().read_properties(false);
    let mut file = std::fs::File::open(path).ok()?;
    match file_type {
        FileType::Mpeg => {
            let mpeg = MpegFile::read_from(&mut file, options).ok()?;
            mpeg.id3v2().cloned()
        }
        FileType::Aac => {
            let aac = AacFile::read_from(&mut file, options).ok()?;
            aac.id3v2().cloned()
        }
        FileType::Wav => {
            let wav = WavFile::read_from(&mut file, options).ok()?;
            wav.id3v2().cloned()
        }
        FileType::Aiff => {
            let aiff = AiffFile::read_from(&mut file, options).ok()?;
            aiff.id3v2().cloned()
        }
        _ => None,
    }
}

/// Reads the front cover of `path`.
///
/// The cover is the embedded picture a library view shows: the first picture
/// of the file's primary tag that claims to be the front cover
/// ([`PictureKind::FrontCover`]). A picture that names no role at all
/// ([`PictureKind::Other`]) is the cover only when no picture claims the
/// role — that is what a tagger writes when it does not know the picture
/// types. A file whose pictures all name another role (a back cover, the
/// media label) has no front cover, and the answer is `None`.
///
/// The picture bytes are returned verbatim; see [`Artwork`]. Nothing is
/// decoded, resized or validated: the caller hands the bytes and the MIME
/// type to an image decoder.
///
/// **Embedded pictures only.** A cover image that sits next to the file
/// (`cover.jpg`, `folder.jpg`) is *not* looked for; this function never
/// touches the network and never reads a file other than `path` itself.
///
/// # Errors
///
/// * [`Error::Io`] if the file cannot be opened or read.
/// * [`Error::UnsupportedFormat`] if the file is not a media file whose tags
///   [`lofty`] understands.
/// * [`Error::Other`] if the tag container cannot be parsed. Unlike
///   [`read_tags`], the artwork reader parses strictly, so a corrupt picture
///   is reported instead of being silently dropped. The price is that a
///   file whose tag is malformed in another way is an error here as well,
///   even when [`read_tags`] still reads its text fields.
pub fn read_artwork(path: impl AsRef<Path>) -> Result<Option<Artwork>> {
    let file = load(path.as_ref(), artwork_options())?;
    Ok(front_cover(&file))
}

/// Reads every embedded picture of `path`.
///
/// The pictures are returned in the same precedence order as [`read_tags`]:
/// the primary tag first, then the other tags of the file. The bytes are
/// returned verbatim; see [`Artwork`].
///
/// This is the form to use when a caller wants more than the album art — the
/// back cover or the media label, for example. A library view that shows the
/// album art wants [`read_artwork`].
///
/// # Errors
///
/// See [`read_artwork`].
pub fn read_artworks(path: impl AsRef<Path>) -> Result<Vec<Artwork>> {
    let file = load(path.as_ref(), artwork_options())?;
    Ok(artworks(&file))
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
fn load(path: &Path, options: ParseOptions) -> Result<TaggedFile> {
    Probe::open(path)
        .map_err(map_lofty)?
        .guess_file_type()?
        .options(options)
        .read()
        .map_err(map_lofty)
}

/// The parse options of the tag reader.
///
/// No audio properties (the decoder fills those) and no pictures (the tag
/// reader is about the text fields). Malformed tag bytes are tolerated:
/// a broken frame is dropped, the rest of the file still reads.
fn tag_options() -> ParseOptions {
    ParseOptions::new()
        .read_properties(false)
        .read_cover_art(false)
}

/// The parse options of the artwork reader.
///
/// The pictures are read, the audio properties are not. The parsing is
/// strict: a picture that cannot be parsed is an error, not a picture that
/// is silently dropped — a cover art that is half there is worse than none.
fn artwork_options() -> ParseOptions {
    ParseOptions::new()
        .read_properties(false)
        .read_cover_art(true)
        .parsing_mode(ParsingMode::Strict)
}

/// Maps a `lofty` error onto the shared [`Error`] type.
///
/// Generic over the error type because `lofty` 0.25 replaced its single error
/// enum with per-operation types (`FileParseError`, `TagEncodingError`, ...).
/// The unknown-format marker and any I/O error are looked up in the error's
/// source chain, which is where `lofty` carries them.
pub(crate) fn map_lofty<E>(error: E) -> Error
where
    E: std::error::Error + 'static,
{
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    let mut unknown_format = false;
    let mut io = None;
    while let Some(current) = source {
        unknown_format |= current.is::<lofty::error::UnknownFormatError>();
        if io.is_none()
            && let Some(inner) = current.downcast_ref::<std::io::Error>()
        {
            io = Some(std::io::Error::new(inner.kind(), inner.to_string()));
        }
        source = current.source();
    }
    if unknown_format {
        return Error::UnsupportedFormat(error.to_string());
    }
    if let Some(io) = io {
        return Error::Io(io);
    }
    Error::Other(error.to_string())
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
    metadata_from_tags(&ordered_tags(file))
}

/// Maps the given tags onto [`TrackMetadata`], in precedence order.
///
/// The first tag that supplies a non-empty value for a field wins, so the
/// tags are given in the order they are consulted (see
/// [`TAG_PRECEDENCE`]). A single tag is the case of one tag: the writer
/// uses it to decode the primary tag of a file on its own (see the
/// `unchanged` module).
pub(crate) fn metadata_from_tags(tags: &[&Tag]) -> TrackMetadata {
    let mut metadata = TrackMetadata {
        title: text(tags, ItemKey::TrackTitle),
        artist: text(tags, ItemKey::TrackArtist),
        album: text(tags, ItemKey::AlbumTitle),
        album_artist: text(tags, ItemKey::AlbumArtist),
        genre: text(tags, ItemKey::Genre),
        // Some taggers (for example ffmpeg's FLAC muxer) write the comment as
        // the Vorbis `DESCRIPTION` item, so fall back to it.
        comment: text(tags, ItemKey::Comment).or_else(|| text(tags, ItemKey::Description)),
        ..TrackMetadata::default()
    };

    metadata.year = date(tags).and_then(|value| parse_year(&value));

    let (track_no, track_total) = number_pair(tags, ItemKey::TrackNumber, ItemKey::TrackTotal);
    metadata.track_no = track_no;
    metadata.track_total = track_total;

    // `TrackMetadata` has no `disc_total` field yet; keep it in `extra` so it
    // is not lost. See `CONTRACT_NOTES.md` for the proposed core change.
    let (disc_no, disc_total) = number_pair(tags, ItemKey::DiscNumber, ItemKey::DiscTotal);
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
        if let Some(value) = text(tags, key) {
            metadata.extra.insert(name.to_owned(), value);
        }
    }

    metadata
}

/// The pictures of `file`, in tag precedence order.
///
/// The pictures are borrowed, so a caller that wants one of them does not
/// pay for the bytes of the others.
fn pictures(file: &TaggedFile) -> impl Iterator<Item = &Picture> {
    ordered_tags(file)
        .into_iter()
        .flat_map(|tag| tag.pictures().iter())
}

/// Collects the pictures of every tag, highest precedence first.
fn artworks(file: &TaggedFile) -> Vec<Artwork> {
    let file_type = file.file_type();
    pictures(file)
        .map(|picture| artwork(picture, picture_kind(picture, file_type)))
        .collect()
}

/// The artwork value of `picture`.
fn artwork(picture: &Picture, kind: PictureKind) -> Artwork {
    Artwork {
        kind,
        mime_type: picture.mime_type().map(|mime| mime.as_str().to_owned()),
        description: picture.description().map(str::to_owned),
        data: picture.data().to_vec(),
    }
}

/// The front cover of `file`.
///
/// The first picture that claims to be the front cover wins. A picture that
/// names no role ([`PictureKind::Other`]) is the cover only when no picture
/// claims the role: it is what a tagger writes when it does not know the
/// picture types, but a real front cover is always the better answer.
fn front_cover(file: &TaggedFile) -> Option<Artwork> {
    let file_type = file.file_type();
    let mut without_a_role = None;
    for picture in pictures(file) {
        let kind = picture_kind(picture, file_type);
        if kind.is_front_cover() {
            return Some(artwork(picture, kind));
        }
        if kind == PictureKind::Other && without_a_role.is_none() {
            without_a_role = Some(artwork(picture, kind));
        }
    }
    without_a_role
}

/// The role of `picture`.
///
/// MP4 has no picture type: the `covr` atom holds the album art, and
/// `lofty` reports every MP4 picture as [`PictureKind::Other`]. It is
/// reported as the front cover here, because that is what the atom means.
fn picture_kind(picture: &Picture, file_type: FileType) -> PictureKind {
    let kind = PictureKind::from_u8(picture.pic_type().as_u8());
    if file_type == FileType::Mp4 && kind == PictureKind::Other {
        PictureKind::FrontCover
    } else {
        kind
    }
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
