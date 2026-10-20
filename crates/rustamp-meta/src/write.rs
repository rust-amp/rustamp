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
//!   [`TrackMetadata`] keeps the value the file already had. A field the
//!   caller wants *gone* is named in the removal list of
//!   [`write_tags_with`]; the two never mean the same thing.
//!
//! ## Formats
//!
//! Which tag a file gets, and which unknown frames or keys survive, is
//! documented at [`write_tags_report`].
//!
//! ## Removing a value
//!
//! The removal list ([`TagField`]) takes a field out of the file: the ID3v2
//! frame is deleted, the Vorbis comment is deleted, the MP4 atom is
//! deleted. A value in a second tag of the file is not written and stays
//! where it is; the write report names every removal that is in that state
//! ([`WriteReport::unremoved`]).
//!
//! ## A write that changes nothing
//!
//! A write that would not change the tags the file has does not touch the
//! file: its bytes and its modification time stay the ones it had, and the
//! report says so ([`WriteReport::unchanged`]).
//!
//! The question is worth asking, because the bytes of a tag are not
//! deterministic: `lofty` keeps the text frames of an ID3v2 tag in a hash
//! map, so two writes of the same metadata produce the same frames in a
//! different order. Writing the same metadata twice would otherwise
//! rewrite the file with another frame order and move its modification time
//! — a change the user never asked for, and one every tool that watches
//! the file (a library scanner, a file index) sees.
//!
//! The comparison is made on the **decoded values** — the values a reader
//! would see — and only on the fields the request names:
//!
//! * A field that is `None` in the metadata means "keep", so it cannot
//!   decide anything. A removal of a field the primary tag does not hold
//!   changes nothing either.
//! * The comparison is made against the **primary tag**, the tag the
//!   writer writes. A value that only a second tag holds (an ID3v1 tag next
//!   to an ID3v2 tag) is not in the primary tag, so a request for it is a
//!   change: the write would put the value into the tag the reader
//!   prefers.
//! * An `extra` entry the target format cannot store is not a change: it
//!   cannot be in the file and cannot be put there. It is reported
//!   ([`WriteReport::unwritten_extra`]), not written.
//!
//! The rule is in the `unchanged` module of this crate.
//!
//! ## What the format cannot store
//!
//! A value the target format cannot hold is not in the file after the
//! write, and the write report names it. There are two kinds:
//!
//! * An `extra` entry the format has no field for: the report names the
//!   key ([`WriteReport::unwritten_extra`]).
//! * A value the format has a field for, but no room: the report names the
//!   field ([`WriteReport::unstoreable`]).
//!
//! | File | Tag written | The numbers it can carry |
//! |---|---|---|
//! | MP3, AAC, WAV, AIFF | ID3v2 | any `u32` |
//! | FLAC, Ogg Vorbis, Opus, Speex | Vorbis comments | any `u32` |
//! | MP4 (M4A, ALAC) | iTunes `ilst` | 1..=65535 |
//! | APE, MPC, WavPack | APEv2 | any `u32` |
//!
//! The MP4 `trkn` and `disk` atoms hold a pair of 16-bit numbers, and a zero
//! means "no number at all". A track or disc number of zero, or one above
//! 65535, is therefore not in the file after the write — and because the
//! writer *replaces* the number, the number the file had before is gone with
//! it. The report says so; the file does not.
//!
//! The check is a range check per format, not a read-back of the written
//! file: the writer knows what each format can hold, the check costs
//! nothing, and it has no error path and no false positives. A read-back
//! would have to know the reader's trimming rules, would have to look at
//! the primary tag alone so that a second tag cannot hide a loss, and
//! could fail after a write that succeeded.
//!
//! Two things are not in that list, because nothing is lost with them:
//!
//! * The year. A year above 9999 is refused with an error before the file
//!   is opened (see `MAX_YEAR`), and a year below is written as a
//!   four-digit year, which every format reads back.
//! * An empty text value. The reader drops an empty value in every format,
//!   so no format loses anything the caller could read back.
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

use std::borrow::Cow;
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
use crate::remove::{TagField, remove_from_tag, remove_from_vorbis, unremoved_in};
use crate::replaygain::{canonical_r128_key, is_r128_key, item_key_for_extra};
use crate::unchanged::Before;

/// The largest year a tag can hold: the readers only accept a four-digit year
/// (`read::parse_year`), so anything larger could not be read back, and
/// `lofty`'s timestamp year is a `u16`, which would silently wrap.
const MAX_YEAR: u32 = 9999;

/// Outcome of [`write_tags_report`]: which `extra` entries the target tag
/// format could not store, which removals the file still carries, and which
/// fields the target format could not store at all.
///
/// A standard field always has a place in every primary tag type, so it is
/// always written; only `extra` entries can be refused, and only when the
/// target format has no field for them. A standard field can still be
/// *too big* for the format — see [`WriteReport::unstoreable`]. Nothing is
/// dropped silently: the report names every one of them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WriteReport {
    /// `extra` keys that were not written because the target tag format has
    /// no field for them (for example `R128_TRACK_GAIN` on an MP4 file, or
    /// `REPLAYGAIN_REFERENCE_LOUDNESS` everywhere — `lofty` 0.25.4 has no
    /// key for it). Everything else in `extra` was written.
    pub unwritten_extra: Vec<String>,
    /// The removals the file still carries: the value of the field is in a
    /// tag the writer does not write (an ID3v1 tag next to an ID3v2 tag,
    /// the `RIFF INFO` of a WAV, the text chunks of an AIFF, the ID3v2 tag
    /// of a FLAC file).
    ///
    /// The removal was carried out in the primary tag of the file; the
    /// value is still in the file, in the second tag. A caller that needs
    /// to know whether the value is gone uses this.
    pub unremoved: Vec<TagField>,
    /// The fields whose value the target format could not store.
    ///
    /// The MP4 `trkn` and `disk` atoms hold a pair of 16-bit numbers, and a
    /// zero means "no number at all": the MP4 family carries neither a zero
    /// nor a number above 65535. Every other format the writer targets
    /// stores the number as text:
    ///
    /// | File | Tag written | The numbers it can carry |
    /// |---|---|---|
    /// | MP3, AAC, WAV, AIFF | ID3v2 | any `u32` |
    /// | FLAC, Ogg Vorbis, Opus, Speex | Vorbis comments | any `u32` |
    /// | MP4 (M4A, ALAC) | iTunes `ilst` | 1..=65535 |
    /// | APE, MPC, WavPack | APEv2 | any `u32` |
    ///
    /// The value the caller asked for is gone, and so is the value the file
    /// had before: the writer replaces the number, so a number the format
    /// cannot hold takes the old one with it. A field that was removed on
    /// request is not in this list — the caller asked for it to be gone.
    ///
    /// Two values are never in this list, because nothing is lost with them:
    /// a year above 9999 is refused with an error before the file is opened,
    /// and an empty text value is read back as "no value" in every format.
    pub unstoreable: Vec<TagField>,
    /// The request would not have changed the file, so the file was not
    /// touched: its bytes and its modification time are the ones it had.
    ///
    /// The comparison is made on the decoded values — the values a reader
    /// would see — and only on the fields the request names; see the
    /// `unchanged` module for the rule. A write that changes nothing is a
    /// no-op: the byte layout of an ID3v2 tag is not deterministic (`lofty`
    /// keeps the text frames in a hash map), so writing the same metadata
    /// twice would otherwise rewrite the file with the frames in another
    /// order and move its modification time.
    ///
    /// This says nothing about the other fields of the report: an `extra`
    /// entry the format refused is still refused, and a removal a second tag
    /// still carries is still not carried out.
    pub unchanged: bool,
}

impl WriteReport {
    /// Returns `true` when every requested `extra` entry was written, every
    /// requested removal was carried out, and no requested value was too big
    /// for the target format.
    ///
    /// A write that changed nothing ([`WriteReport::unchanged`]) is
    /// complete: the values the caller asked for are in the file, they were
    /// there already.
    pub fn is_complete(&self) -> bool {
        self.unwritten_extra.is_empty() && self.unremoved.is_empty() && self.unstoreable.is_empty()
    }
}

/// Writes `metadata` into the tags of `path` and reports what could not be
/// stored.
///
/// This is the honest form of [`write_tags`]: fields that are `None` are
/// left untouched, so a read → edit → write cycle keeps every value the
/// caller does not set, and the returned [`WriteReport`] names every `extra`
/// entry the target format refused and every field the target format could
/// not store ([`WriteReport::unstoreable`]). A caller that needs to know
/// whether a write happened uses this; [`write_tags`] keeps the old
/// best-effort signature for existing callers.
///
/// The write is atomic: the new file is built next to the original and
/// renamed over it, so a failure leaves the original untouched (the
/// `atomic` module of this crate).
///
/// A write that would not change the tags of the file does not touch it at
/// all — the bytes and the modification time stay the ones the file had —
/// and the report says so ([`WriteReport::unchanged`]).
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
    write_tags_with_report(path, metadata, &[])
}

/// Writes `metadata` into the tags of `path`, removes the fields of
/// `removals` and reports what could not be done.
///
/// This is [`write_tags_report`] with the removals: a field of
/// [`TrackMetadata`](rustamp_core::meta::TrackMetadata) that is `None` still
/// means "leave this tag as it is", and a field of `removals` is taken out
/// of the file. A removal wins over a value: a field that is in both
/// `metadata` and `removals` is removed.
///
/// ## What a removal does
///
/// The removal is applied to the *primary* tag of the file, and it takes
/// the field out of it:
///
/// | File | The removal deletes |
/// |---|---|
/// | MP3, AAC, WAV, AIFF | the ID3v2 frame (`TIT2`, `TPE1`, …) |
/// | FLAC, Ogg Vorbis, Opus, Speex | the Vorbis comment (`TITLE`, `ARTIST`, …) |
/// | MP4 (M4A, ALAC) | the `ilst` atom (`©nam`, `©ART`, …) |
/// | APE, MPC, WavPack | the APEv2 item |
///
/// A value in a second tag — an ID3v1 next to an ID3v2, the `RIFF INFO` of
/// a WAV, the text chunks of an AIFF — is not written and stays in the
/// file. The report names every removal that is in that state
/// ([`WriteReport::unremoved`]), so a limitation is never silent.
///
/// The field is a whole field: removing the track takes the track number
/// and the total number of tracks with it, and removing the disc takes the
/// disc number and the total number of discs. The comment takes the
/// `DESCRIPTION` item with it, because the reader takes a description as
/// the comment when a file has no comment of its own.
///
/// A removed field is not in [`WriteReport::unstoreable`], even when the
/// metadata named a value the format could not store: the caller asked for
/// the field to be gone, so the format is not the reason it is not in the
/// file.
///
/// # Errors
///
/// The same errors as [`write_tags_report`].
pub fn write_tags_with_report(
    path: impl AsRef<Path>,
    metadata: &TrackMetadata,
    removals: &[TagField],
) -> Result<WriteReport> {
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
    let report = target.apply(metadata, removals);
    // A request that would not change the tags of the file does not touch
    // it: the bytes and the modification time stay the ones the file has
    // (see the `unchanged` module).
    if !report.unchanged {
        replace_atomically(&path, |file| target.save(file))?;
    }
    Ok(report)
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

/// Writes `metadata` into the tags of `path` and removes the fields of
/// `removals`.
///
/// This is [`write_tags_with_report`] without the report: the fields of
/// [`TrackMetadata`] that are `None` are left untouched, the fields of
/// `removals` are taken out of the file, and a removal wins over a value.
///
/// A removal that the file cannot carry out — a value in a second tag, an
/// ID3v1 tag next to an ID3v2 tag — is not reported here. A caller that
/// needs to know uses [`write_tags_with_report`].
///
/// # Errors
///
/// The same errors as [`write_tags`].
pub fn write_tags_with(
    path: impl AsRef<Path>,
    metadata: &TrackMetadata,
    removals: &[TagField],
) -> Result<()> {
    write_tags_with_report(path, metadata, removals).map(|_| ())
}

/// The tag a write targets.
///
/// Most formats go through `lofty`'s generic [`Tag`] model. The Vorbis
/// family gets the *concrete* tag instead: the generic model cannot hold a
/// key it does not know, so a FLAC file's own `CUSTOM_KEY` would be dropped
/// on every write.
enum Target {
    /// ID3v2 (MP3, AAC, WAV, AIFF), the MP4 `ilst` and APEv2, plus the
    /// other tags of the file: the writer does not touch them, so a value
    /// they hold survives a removal.
    Generic { tag: Tag, others: Vec<Tag> },
    /// A FLAC file: the concrete Vorbis comments plus the picture blocks
    /// that live beside them (the concrete comments alone would drop the
    /// cover art), plus the ID3v2 tag the writer leaves in the file.
    Flac { flac: FlacFile, others: Vec<Tag> },
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
            FileType::Flac => {
                let (flac, others) = read_flac(path)?;
                Ok(Self::Flac { flac, others })
            }
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
                // The other tags of the file are not written. A value they
                // hold survives a removal, and the report says so.
                let others = file.tags().to_vec();
                Ok(Self::Generic { tag, others })
            }
        }
    }

    /// The primary tag of the file, seen through the generic model.
    ///
    /// The Vorbis family is written as concrete comments, but the values of
    /// a comment are the values of the generic model — the ones the reader
    /// sees. The second tags of the file are not part of it: the writer
    /// does not write them.
    fn primary(&self) -> Cow<'_, Tag> {
        match self {
            Self::Generic { tag, .. } => Cow::Borrowed(tag),
            Self::Flac { flac, .. } => Cow::Owned(
                flac.vorbis_comments()
                    .map(|comments| Tag::from(comments.clone()))
                    .unwrap_or_else(|| Tag::new(TagType::VorbisComments)),
            ),
            Self::Vorbis(comments) => Cow::Owned(Tag::from(comments.clone())),
        }
    }

    /// Applies `metadata` to the tag and removes the fields of `removals`.
    fn apply(&mut self, metadata: &TrackMetadata, removals: &[TagField]) -> WriteReport {
        // The state of the file before the write: it decides whether the
        // write changes anything (see the `unchanged` module).
        let before = Before::of(&self.primary(), metadata, removals);
        match self {
            Self::Generic { tag, others } => {
                let mut unwritten_extra = apply_to_tag(tag, metadata);
                // ID3v2 has no `ItemKey` for the R128 keys, but it can carry
                // them as `TXXX` frames (see `write_id3v2_r128`).
                if tag.tag_type() == TagType::Id3v2 && write_id3v2_r128(tag, metadata) {
                    unwritten_extra.retain(|key| !is_r128_key(key));
                }
                // The removals come last: a removal wins over a value.
                remove_from_tag(tag, removals);
                let unchanged = !before.would_change(metadata, removals, &unwritten_extra);
                WriteReport {
                    unwritten_extra,
                    unremoved: unremoved_in(others, removals),
                    unstoreable: unstoreable_fields(tag.tag_type(), metadata, removals),
                    unchanged,
                }
            }
            Self::Flac { flac, others } => {
                if flac.vorbis_comments().is_none() {
                    flac.set_vorbis_comments(VorbisComments::new());
                }
                let mut unwritten_extra = Vec::new();
                if let Some(comments) = flac.vorbis_comments_mut() {
                    unwritten_extra = apply_to_vorbis(comments, metadata);
                    // The removals come last: a removal wins over a value.
                    remove_from_vorbis(comments, removals);
                }
                let unchanged = !before.would_change(metadata, removals, &unwritten_extra);
                WriteReport {
                    unwritten_extra,
                    unremoved: unremoved_in(others, removals),
                    unstoreable: unstoreable_fields(TagType::VorbisComments, metadata, removals),
                    unchanged,
                }
            }
            Self::Vorbis(comments) => {
                let unwritten_extra = apply_to_vorbis(comments, metadata);
                // The removals come last: a removal wins over a value.
                remove_from_vorbis(comments, removals);
                let unchanged = !before.would_change(metadata, removals, &unwritten_extra);
                WriteReport {
                    unwritten_extra,
                    unremoved: Vec::new(),
                    unstoreable: unstoreable_fields(TagType::VorbisComments, metadata, removals),
                    unchanged,
                }
            }
        }
    }

    /// Writes the tag into `file`.
    fn save(&self, file: &mut File) -> Result<()> {
        let options = WriteOptions::default();
        match self {
            Self::Generic { tag, .. } => tag.save_to(file, options).map_err(map_lofty),
            Self::Flac { flac, .. } => flac.save_to(file, options).map_err(map_lofty),
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

/// The FLAC file at `path`, with its tags and its picture blocks, and the
/// ID3v2 tag the writer leaves in the file.
///
/// The ID3v2 tag of a FLAC file is read-only (the format does not allow
/// it), and the write path would rewrite it. It is taken out of the file,
/// so the copy in the file stays as it is — and a value it holds survives
/// a removal, which the report says.
fn read_flac(path: &Path) -> Result<(FlacFile, Vec<Tag>)> {
    let mut file = File::open(path)?;
    let mut flac = FlacFile::read_from(&mut file, ParseOptions::new().read_properties(false))
        .map_err(map_lofty)?;
    let others = flac
        .remove_id3v2()
        .map(|tag| vec![Tag::from(tag)])
        .unwrap_or_default();
    Ok((flac, others))
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

/// The fields whose value the format of `tag_type` cannot store.
///
/// The value is the one the caller asked for; the format has no room for
/// it, so the file does not carry it after the write.
///
/// The check is a range check per format, not a read-back of the written
/// file: the writer knows what each format can hold, and the check costs
/// nothing. The table is in the module docs.
///
/// A field of `removals` is not in the list: the caller asked for it to be
/// gone, so the format is not the reason it is not in the file.
fn unstoreable_fields(
    tag_type: TagType,
    metadata: &TrackMetadata,
    removals: &[TagField],
) -> Vec<TagField> {
    let mut fields = Vec::new();

    // The track is a whole field: the number and the total belong together,
    // and half the formats keep the two in one place.
    let track = metadata
        .track_no
        .into_iter()
        .chain(metadata.track_total)
        .any(|number| !carries_number(tag_type, number));
    if track {
        fields.push(TagField::Track);
    }

    if metadata
        .disc_no
        .is_some_and(|number| !carries_number(tag_type, number))
    {
        fields.push(TagField::Disc);
    }

    fields.retain(|field| !removals.contains(field));
    fields
}

/// Whether the format of `tag_type` can carry `number`.
///
/// The MP4 `trkn` and `disk` atoms hold a pair of 16-bit numbers, and a
/// zero means "no number at all": the MP4 family carries neither a zero nor
/// a number above 65535. Every other format the writer targets stores the
/// number as text, so it carries every `u32`.
fn carries_number(tag_type: TagType, number: u32) -> bool {
    match tag_type {
        TagType::Mp4Ilst => (1..=u32::from(u16::MAX)).contains(&number),
        _ => true,
    }
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
        // The frame carries the canonical name of the key, whatever
        // spelling the caller used.
        let Some(description) = canonical_r128_key(key) else {
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
