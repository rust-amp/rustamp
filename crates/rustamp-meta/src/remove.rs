//! Removing tag fields.
//!
//! A [`TrackMetadata`](rustamp_core::meta::TrackMetadata) field that is
//! `None` means "leave this tag as it is", and it keeps that meaning. The
//! writer had no way to say "this value goes away" — this module is that
//! way: a list of [`TagField`]s the caller wants gone from the file.
//!
//! ## The fields
//!
//! The list is the one the writer supports: the title, the artist, the
//! album, the album artist, the year, the genre, the comment, the track
//! and the disc. A field is a *whole* field: removing the track takes the
//! track number and the total number of tracks with it, because half the
//! formats keep the two in one place (the ID3v2 `TRCK` frame, the MP4
//! `trkn` atom) and the other half keep them as two comments that belong
//! together.
//!
//! ## The comment
//!
//! The comment is the `COMMENT` item, and the `DESCRIPTION` item with it:
//! the reader takes a description as the comment when a file has no
//! comment of its own, so a removal that left the description behind
//! would not clear the comment.
//!
//! ## Where a removal lands
//!
//! The removal is applied to the *primary* tag of the file — the ID3v2
//! tag, the Vorbis comments, the MP4 `ilst`, the APEv2 tag. A value in a
//! second tag (an ID3v1 tag next to an ID3v2 tag, the `RIFF INFO` of a
//! WAV, the text chunks of an AIFF) is not written and stays in the file;
//! the write report names every removal that is in that state.

use lofty::ogg::tag::VorbisComments;
use lofty::tag::{ItemKey, Tag, TagType};

/// A tag field the writer can remove.
///
/// The list is the one the writer supports: the title, the artist, the
/// album, the album artist, the year, the genre, the comment, the track
/// and the disc. See [`crate::write_tags_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TagField {
    /// The title.
    Title,
    /// The artist.
    Artist,
    /// The album.
    Album,
    /// The album artist.
    AlbumArtist,
    /// The release year.
    Year,
    /// The genre.
    Genre,
    /// The comment.
    ///
    /// The comment is the `COMMENT` item, and the `DESCRIPTION` item with
    /// it: the reader takes a description as the comment when a file has
    /// no comment of its own.
    Comment,
    /// The track number and the total number of tracks.
    Track,
    /// The disc number and the total number of discs.
    Disc,
}

impl TagField {
    /// The tag items the field is stored in.
    ///
    /// A field can live in more than one item: the year is a recording
    /// date or a plain year, the track is a number and a total, and the
    /// comment is a comment or a description.
    pub(crate) fn item_keys(self) -> &'static [ItemKey] {
        match self {
            Self::Title => &[ItemKey::TrackTitle],
            Self::Artist => &[ItemKey::TrackArtist],
            Self::Album => &[ItemKey::AlbumTitle],
            Self::AlbumArtist => &[ItemKey::AlbumArtist],
            Self::Year => &[ItemKey::RecordingDate, ItemKey::Year],
            Self::Genre => &[ItemKey::Genre],
            Self::Comment => &[ItemKey::Comment, ItemKey::Description],
            Self::Track => &[ItemKey::TrackNumber, ItemKey::TrackTotal],
            Self::Disc => &[ItemKey::DiscNumber, ItemKey::DiscTotal],
        }
    }

    /// Whether the tag holds a value for the field.
    pub(crate) fn is_held_by(self, tag: &Tag) -> bool {
        self.item_keys().iter().any(|key| tag.get(*key).is_some())
    }
}

/// Removes the fields of `removals` from the generic tag.
pub(crate) fn remove_from_tag(tag: &mut Tag, removals: &[TagField]) {
    for field in removals {
        for key in field.item_keys() {
            tag.remove_key(*key);
        }
    }
}

/// Removes the fields of `removals` from concrete Vorbis comments.
///
/// The comments hold the keys of the format, not the items of the generic
/// model, and a key can be spelled in more than one way (`ALBUMARTIST`
/// and `ALBUM ARTIST`, `TRACKTOTAL` and `TOTALTRACKS`). Every comment
/// whose key belongs to a removed field goes, whatever it is spelled.
pub(crate) fn remove_from_vorbis(comments: &mut VorbisComments, removals: &[TagField]) {
    if removals.is_empty() {
        return;
    }
    let keys: Vec<String> = comments.items().map(|(key, _)| key.to_owned()).collect();
    for key in keys {
        let Some(item_key) = ItemKey::from_key(TagType::VorbisComments, &key) else {
            continue;
        };
        if removals
            .iter()
            .any(|field| field.item_keys().contains(&item_key))
        {
            let _ = comments.remove(&key);
        }
    }
}

/// The removals the file still carries: the value of the field is in a tag
/// the writer does not write.
pub(crate) fn unremoved_in(others: &[Tag], removals: &[TagField]) -> Vec<TagField> {
    let mut unremoved = Vec::new();
    for field in removals {
        let held = others.iter().any(|tag| field.is_held_by(tag));
        if held && !unremoved.contains(field) {
            unremoved.push(*field);
        }
    }
    unremoved
}
