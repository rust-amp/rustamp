//! Deciding whether a write would change the file.
//!
//! A tag write replaces the file: the new file is built next to the old one
//! and renamed over it. The bytes of the new file are not always the bytes
//! of the old one, even when the metadata is the same — `lofty` keeps the
//! text frames of an ID3v2 tag in a hash map, so two writes of the same tag
//! produce the same frames in a different order. A save that changes nothing
//! therefore still changes the file: its modification time moves, and every
//! tool that watches the file (a library scanner, a file index) sees a new
//! file.
//!
//! This module is the rule that stops that. Before the writer touches the
//! file, it asks whether the request would change the tags the file has. If
//! it would not, the file is left alone — its bytes and its modification
//! time — and the report says so ([`WriteReport::unchanged`]).
//!
//! ## The comparison
//!
//! The comparison is made on the **decoded values**, the values a reader
//! would see, and never on the bytes: the order of the frames of an ID3v2
//! tag is not ours to compare, and two tags that hold the same values are
//! the same tag.
//!
//! * **Only the fields the request names are compared.** A field that is
//!   `None` in the metadata means "keep", so it cannot decide anything. A
//!   field of the removal list is compared as well: a removal of a field the
//!   primary tag does not hold changes nothing.
//! * **The comparison is made against the primary tag**, the tag the writer
//!   writes. A value that only a second tag holds (an ID3v1 tag next to an
//!   ID3v2 tag) is not in the primary tag, so a request for it is a change:
//!   the write would put the value into the tag the reader prefers.
//! * **An `extra` entry the target format cannot store is not a change.** It
//!   cannot be in the file and cannot be put there; it is reported
//!   ([`WriteReport::unwritten_extra`]), not written.
//! * **The values are the reader's.** A text value is trimmed and an empty
//!   one is "no value", a date is its year and a number pair is split on the
//!   `/` — the rules the tag reader applies, so the value that is compared
//!   is the value a reader would see.
//!
//! ## What the rule does not decide
//!
//! The rule looks at the values, not at the bytes: a write that would
//! rewrite the tag of the file in another shape (an ID3v2 tag is written as
//! ID3v2.4 with padding, whatever the file held before) is a no-op when the
//! values are the same. The file keeps the shape it had, which is the point:
//! a save that changes nothing must not change the file.

use std::collections::BTreeMap;

use lofty::id3::v2::Id3v2Tag;
use lofty::tag::{ItemKey, Tag, TagType};

use rustamp_core::meta::TrackMetadata;

use crate::read::metadata_from_tags;
use crate::remove::TagField;
use crate::replaygain::{canonical_r128_key, item_key_for_extra};

/// The state of the primary tag before the write.
///
/// Only the values a request can decide on are kept: the standard fields,
/// the `extra` entries the request names and the fields of the removal list
/// the tag holds.
pub(crate) struct Before {
    /// The standard fields of the tag, decoded.
    fields: TrackMetadata,
    /// The values of the `extra` entries the request names.
    extra: BTreeMap<String, Option<String>>,
    /// The fields of the removal list the tag holds.
    held: Vec<TagField>,
}

impl Before {
    /// The state of the primary tag `tag`, seen through the generic model.
    ///
    /// `tag` is the primary tag of the file on its own — the second tags of
    /// the file are not part of it, because the writer does not write them.
    pub(crate) fn of(tag: &Tag, metadata: &TrackMetadata, removals: &[TagField]) -> Self {
        let extra = metadata
            .extra
            .keys()
            .map(|key| (key.clone(), extra_value(tag, key)))
            .collect();
        let held = removals
            .iter()
            .copied()
            .filter(|field| field.is_held_by(tag))
            .collect();
        Self {
            fields: metadata_from_tags(&[tag]),
            extra,
            held,
        }
    }

    /// Whether the request would change the primary tag.
    ///
    /// `unwritten_extra` names the `extra` entries the target format could
    /// not store: they are not a change, because they cannot be in the file
    /// and cannot be put there.
    pub(crate) fn would_change(
        &self,
        metadata: &TrackMetadata,
        removals: &[TagField],
        unwritten_extra: &[String],
    ) -> bool {
        let current = &self.fields;

        // The named fields. A field that is `None` keeps the value the file
        // has, so it cannot decide anything.
        if text_changed(current.title.as_deref(), metadata.title.as_deref())
            || text_changed(current.artist.as_deref(), metadata.artist.as_deref())
            || text_changed(current.album.as_deref(), metadata.album.as_deref())
            || text_changed(
                current.album_artist.as_deref(),
                metadata.album_artist.as_deref(),
            )
            || text_changed(current.genre.as_deref(), metadata.genre.as_deref())
            || text_changed(current.comment.as_deref(), metadata.comment.as_deref())
        {
            return true;
        }
        if let Some(year) = metadata.year
            && current.year != Some(year)
        {
            return true;
        }
        if let Some(number) = metadata.track_no
            && current.track_no != Some(number)
        {
            return true;
        }
        if let Some(total) = metadata.track_total
            && current.track_total != Some(total)
        {
            return true;
        }
        if let Some(number) = metadata.disc_no
            && current.disc_no != Some(number)
        {
            return true;
        }

        // The `extra` entries the target format can store.
        for (key, value) in &metadata.extra {
            if unwritten_extra.iter().any(|unwritten| unwritten == key) {
                continue;
            }
            let held = self.extra.get(key).and_then(|value| value.as_deref());
            if text_changed(held, Some(value)) {
                return true;
            }
        }

        // The removals: a removal of a field the tag does not hold changes
        // nothing.
        removals.iter().any(|field| self.held.contains(field))
    }
}

/// Whether the requested text value differs from the one the tag holds.
///
/// A `None` request means "keep", so the field cannot decide anything.
fn text_changed(current: Option<&str>, requested: Option<&str>) -> bool {
    match requested {
        None => false,
        Some(value) => normalized(value) != current.and_then(normalized),
    }
}

/// The value a reader would see for a text value.
///
/// A value of blanks is no value, exactly as the reader sees it.
fn normalized(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// The value of an `extra` entry in the tag.
fn extra_value(tag: &Tag, key: &str) -> Option<String> {
    // The R128 keys of an ID3v2 tag live in `TXXX` frames the generic model
    // cannot see, so the concrete tag is asked for them (see the `read`
    // module). The frames carry the canonical name of the key, whatever
    // spelling the caller used.
    if tag.tag_type() == TagType::Id3v2
        && let Some(description) = canonical_r128_key(key)
    {
        let concrete: Id3v2Tag = tag.clone().into();
        return concrete
            .get_user_text(description)
            .and_then(normalized)
            .map(str::to_owned);
    }
    let item_key = item_key_for_extra(key).or_else(|| ItemKey::from_key(tag.tag_type(), key))?;
    tag.get_string(item_key)
        .and_then(normalized)
        .map(str::to_owned)
}
