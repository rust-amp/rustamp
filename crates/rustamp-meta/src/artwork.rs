//! Embedded pictures (cover art).

/// An embedded picture read from a tag.
///
/// `rustamp-meta` exposes the stored bytes verbatim; it does not decode,
/// resize or otherwise process the image, and it does not know whether the
/// bytes form a valid picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artwork {
    /// The picture type as stored in the ID3v2 APIC frame (`0..=20`; larger
    /// values are undefined types, see the ID3v2 APIC table).
    pub kind: u8,
    /// The MIME type stored in the tag (for example `"image/jpeg"`), if the
    /// format carries one.
    pub mime_type: Option<String>,
    /// The picture description, if the format carries one.
    pub description: Option<String>,
    /// The picture bytes exactly as stored in the tag.
    pub data: Vec<u8>,
}
