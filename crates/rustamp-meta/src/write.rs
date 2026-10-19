//! Tag writing.
//!
//! Writing is the mirror image of reading: the file is opened, its primary
//! tag is read, the fields the caller sets are applied and the tag is
//! written back. Three rules hold for every format:
//!
//! * **Atomic.** The new file is built next to the original and renamed
//!   over it (see [`crate::atomic`]). A failed write never leaves a
//!   half-written media file behind.
//! * **The audio is never rewritten.** Only the tag container changes; the
//!   audio frames are copied byte for byte.
//! * **What the caller does not set stays.** A field that is `None` in the
//!   [`TrackMetadata`] keeps the value the file already had.
//!
//! ## Formats
//!
//! Which tag a file gets, and which unknown frames or keys survive, is
//! documented at [`write_tags_report`].
//!
//! ## The R128 keys of an ID3v2 tag
//!
//! `lofty` 0.25.4 has no `ItemKey` for the `R128_TRACK_GAIN` /
//! `R128_ALBUM_GAIN` keys in ID3v2, so the generic tag model cannot hold
//! them. The concrete ID3v2 tag can: a `TXXX` frame is told apart by its
//! description, and the description of these frames is the key. The writer
//! therefore adds them to the concrete tag and hands the tag back to the
//! generic model, which keeps such a frame next to the frames it models
//! itself. A file without R128 values is written exactly as before.
//!
//! ## Symlinks
//!
//! A symlinked file is written *through*: the link stays a link and the file
//! it points at is replaced.

use std::fs::File;
use std::path::Path;

use lofty::config::{GlobalOptions, ParseOptions, WriteOptions};
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::flac::FlacFile;
use lofty::id3::v2::Id3v2Tag;
use lofty::ogg::tag::VorbisComments;
use lofty::ogg::{OpusFile, SpeexFile, VorbisFile};
use lofty::probe::Probe;
use lofty::tag::items::Timestamp;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};

use rustamp_core::error::{Error, Result};
use rustamp_core::meta::TrackMetadata;

use crate::atomic::replace_atomically;
use crate::read::map_lofty;
use crate::replaygain::{R128_ALBUM_GAIN_KEY, R128_TRACK_GAIN_KEY, item_key_for_extra};

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
    /// no field for them (for example `R128_TRACK_GAIN` on an MP4 file, or
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
/// The write is atomic: the new file is built next to the original and
/// renamed over it, so a failure leaves the original untouched (the
/// `atomic` module of this crate).
///
/// ## Formats
///
/// | File | Tag written | Unknown items in it |
/// |---|---|---|
/// | MP3, AAC, WAV, AIFF | ID3v2 | kept |
/// | FLAC | Vorbis comments | kept |
/// | Ogg Vorbis, Opus, Speex | Vorbis comments | kept |
/// | MP4 (M4A, ALAC) | iTunes `ilst` | kept |
/// | APE, MPC, WavPack | APEv2 | dropped |
///
/// "Unknown items" are the frames, atoms and comment keys the tag format
/// allows but this crate has no field for: a `TXXX` frame with a private
/// description, a FLAC `CUSTOM_KEY`, a freeform MP4 atom. ID3v2 and MP4
/// keep them — `lofty` holds them next to the generic tag — and the Vorbis
/// family keeps them because the *concrete* tag is written. APEv2 has no
/// such place in `lofty`'s generic model, so its unknown items are dropped.
///
/// Only the primary tag of a file is written. A second tag beside it — an
/// ID3v1 next to an ID3v2, the `RIFF INFO` of a WAV, the text chunks of an
/// AIFF — is left byte for byte as it was.
///
/// ReplayGain keys are looked up in a canonical table (see `replaygain`),
/// not by the format's short key name, so MP4 — whose ilst map only knows
/// the freeform `----:com.apple.iTunes:replaygain_*` form — round-trips the
/// four classic keys instead of dropping them. The R128 keys are written
/// as `TXXX` frames in an ID3v2 tag and as plain comments in a Vorbis one;
/// every other format refuses them, and the refusal is reported.
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

    // A symlink is written through: the link stays a link and the file it
    // points at is the one that is replaced.
    let path = std::fs::canonicalize(path)?;

    // The unknown ID3v2 frames and MP4 atoms live next to the generic tag
    // and are only carried along when this option is on. It is on by
    // default, but the write path sets it explicitly so that a write keeps
    // them whatever the rest of the process did to the global options.
    lofty::config::apply_global_options(GlobalOptions::new().preserve_format_specific_items(true));

    let mut target = Target::read(&path)?;
    let unwritten_extra = target.apply(metadata);
    replace_atomically(&path, |file| target.save(file))?;
    Ok(WriteReport { unwritten_extra })
}

/// Writes `metadata` into the tags of `path`.
///
/// Fields that are `None` are left untouched, so a read → edit → write cycle
/// keeps every value the caller does not set. If the file has no tag of its
/// format's primary type yet, one is created. Writing rewrites tag blocks
/// only; the audio is never decoded or re-encoded, and the file itself is
/// replaced atomically (see [`write_tags_report`]).
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

/// The tag a write targets.
///
/// Most formats go through `lofty`'s generic [`Tag`] model. The Vorbis
/// family gets the *concrete* tag instead: the generic model cannot hold a
/// key it does not know, so a FLAC file's own `CUSTOM_KEY` would be dropped
/// on every write.
enum Target {
    /// ID3v2 (MP3, AAC, WAV, AIFF), the MP4 `ilst` and APEv2.
    Generic(Tag),
    /// A FLAC file: the concrete Vorbis comments plus the picture blocks
    /// that live beside them (the concrete comments alone would drop the
    /// cover art).
    Flac(FlacFile),
    /// Ogg Vorbis, Opus and Speex: the concrete Vorbis comments, pictures
    /// included.
    Vorbis(VorbisComments),
}

impl Target {
    /// Reads the tag to be written from `path`.
    fn read(path: &Path) -> Result<Self> {
        let file_type = file_type_of(path)?;
        match file_type {
            // The Vorbis family: the concrete tag, so that the keys the
            // generic model does not know survive the write.
            FileType::Flac => Ok(Self::Flac(read_flac(path)?)),
            FileType::Vorbis | FileType::Opus | FileType::Speex => {
                Ok(Self::Vorbis(read_vorbis_comments(file_type, path)?))
            }
            // Everything else: the generic tag model.
            _ => {
                let primary = file_type.primary_tag_type();
                let mut file = Probe::open(path)
                    .map_err(map_lofty)?
                    .guess_file_type()
                    .map_err(map_lofty)?
                    .options(ParseOptions::new().read_properties(false))
                    .read()
                    .map_err(map_lofty)?;
                let tag = file.remove(primary).unwrap_or_else(|| Tag::new(primary));
                Ok(Self::Generic(tag))
            }
        }
    }

    /// Applies `metadata` to the tag, returning the `extra` keys the format
    /// could not store.
    fn apply(&mut self, metadata: &TrackMetadata) -> Vec<String> {
        match self {
            Self::Generic(tag) => {
                let mut unwritten = apply_to_tag(tag, metadata);
                // ID3v2 has no `ItemKey` for the R128 keys, but it can carry
                // them as `TXXX` frames (see `write_id3v2_r128`).
                if tag.tag_type() == TagType::Id3v2 && write_id3v2_r128(tag, metadata) {
                    unwritten.retain(|key| !is_r128_key(key));
                }
                unwritten
            }
            Self::Flac(flac) => {
                if flac.vorbis_comments().is_none() {
                    flac.set_vorbis_comments(VorbisComments::new());
                }
                match flac.vorbis_comments_mut() {
                    Some(comments) => apply_to_vorbis(comments, metadata),
                    None => Vec::new(),
                }
            }
            Self::Vorbis(comments) => apply_to_vorbis(comments, metadata),
        }
    }

    /// Writes the tag into `file`.
    fn save(&self, file: &mut File) -> Result<()> {
        let options = WriteOptions::default();
        match self {
            Self::Generic(tag) => tag.save_to(file, options).map_err(map_lofty),
            Self::Flac(flac) => flac.save_to(file, options).map_err(map_lofty),
            Self::Vorbis(comments) => comments.save_to(file, options).map_err(map_lofty),
        }
    }
}

/// The file type of `path`, guessed from its extension and its content.
fn file_type_of(path: &Path) -> Result<FileType> {
    let probe = Probe::open(path)
        .map_err(map_lofty)?
        .guess_file_type()
        .map_err(map_lofty)?;
    probe
        .file_type()
        .ok_or_else(|| Error::UnsupportedFormat(format!("{} is not a media file", path.display())))
}

/// The FLAC file at `path`, with its tags and its picture blocks.
fn read_flac(path: &Path) -> Result<FlacFile> {
    let mut file = File::open(path)?;
    let mut flac = FlacFile::read_from(&mut file, ParseOptions::new().read_properties(false))
        .map_err(map_lofty)?;
    // The ID3v2 tag of a FLAC file is read-only (the format does not allow
    // it), and the write path would rewrite it. It is taken out of the
    // file, so the copy in the file stays as it is.
    let _ = flac.remove_id3v2();
    Ok(flac)
}

/// The concrete Vorbis comments of an Ogg Vorbis, Opus or Speex file.
fn read_vorbis_comments(file_type: FileType, path: &Path) -> Result<VorbisComments> {
    let options = ParseOptions::new().read_properties(false);
    let mut file = File::open(path)?;
    let comments = match file_type {
        FileType::Vorbis => VorbisFile::read_from(&mut file, options)
            .map_err(map_lofty)?
            .vorbis_comments()
            .clone(),
        FileType::Opus => OpusFile::read_from(&mut file, options)
            .map_err(map_lofty)?
            .vorbis_comments()
            .clone(),
        FileType::Speex => SpeexFile::read_from(&mut file, options)
            .map_err(map_lofty)?
            .vorbis_comments()
            .clone(),
        other => {
            return Err(Error::UnsupportedFormat(format!(
                "{other:?} files do not carry Vorbis comments"
            )));
        }
    };
    Ok(comments)
}

/// Copies the set fields of `metadata` onto `tag`, returning the `extra`
/// keys the tag could not store.
fn apply_to_tag(tag: &mut Tag, metadata: &TrackMetadata) -> Vec<String> {
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

/// Applies `metadata` to concrete Vorbis comments.
///
/// The field mapping is the generic one (see [`apply_to_tag`]), and the
/// result is merged into the concrete comments. The comments keep every key
/// the generic model cannot represent, so a FLAC file's own `CUSTOM_KEY`
/// survives a write.
fn apply_to_vorbis(comments: &mut VorbisComments, metadata: &TrackMetadata) -> Vec<String> {
    let mut tag = Tag::new(TagType::VorbisComments);
    let unwritten = apply_to_tag(&mut tag, metadata);
    for item in tag.items() {
        let Some(key) = item.key().map_key(TagType::VorbisComments) else {
            continue;
        };
        let Some(value) = item.value().text() else {
            continue;
        };
        comments.insert(key.to_owned(), value.to_owned());
    }
    unwritten
}

/// Whether `key` names one of the two R128 keys.
fn is_r128_key(key: &str) -> bool {
    key.eq_ignore_ascii_case(R128_TRACK_GAIN_KEY) || key.eq_ignore_ascii_case(R128_ALBUM_GAIN_KEY)
}

/// Writes the R128 values of `metadata` into an ID3v2 tag as `TXXX` frames.
///
/// `lofty` 0.25.4 has no `ItemKey` for the R128 keys in ID3v2, so the
/// generic tag model cannot hold them: a `TXXX` frame is told apart by its
/// description, and only the descriptions `lofty` knows have an `ItemKey`.
/// The concrete tag can hold such a frame. The round trip through it is
/// what puts the frame into the generic tag's companion — the place where
/// the writer keeps the frames it does not model itself — so the frame
/// reaches the file next to the mapped items.
///
/// Returns `true` when at least one frame was written.
fn write_id3v2_r128(tag: &mut Tag, metadata: &TrackMetadata) -> bool {
    let mut frames: Vec<(&str, &str)> = Vec::new();
    for (key, value) in &metadata.extra {
        let description = if key.eq_ignore_ascii_case(R128_TRACK_GAIN_KEY) {
            R128_TRACK_GAIN_KEY
        } else if key.eq_ignore_ascii_case(R128_ALBUM_GAIN_KEY) {
            R128_ALBUM_GAIN_KEY
        } else {
            continue;
        };
        frames.push((description, value.as_str()));
    }
    if frames.is_empty() {
        return false;
    }

    // The concrete tag is built once, from the tag as it stands, and the
    // frames are added to it.
    let mut concrete: Id3v2Tag = tag.clone().into();
    for (description, value) in frames {
        concrete.insert_user_text(description.to_owned(), value.to_owned());
    }
    *tag = concrete.into();
    true
}
