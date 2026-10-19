//! Embedded pictures (cover art).

/// The role an embedded picture claims in the tag.
///
/// The variants are the ID3v2 `APIC` picture types. Every tag format that
/// carries a picture uses that table: the FLAC `PICTURE` block and the Vorbis
/// `METADATA_BLOCK_PICTURE` store the same number. MP4 has no picture type
/// at all — its `covr` atom *is* the album art — so an MP4 picture is
/// reported as [`PictureKind::FrontCover`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PictureKind {
    /// The picture names no role (`0`).
    Other,
    /// A 32×32 pixel file icon, PNG only (`1`).
    Icon,
    /// Another file icon (`2`).
    OtherIcon,
    /// The front cover, the album art (`3`).
    FrontCover,
    /// The back cover (`4`).
    BackCover,
    /// A leaflet page (`5`).
    Leaflet,
    /// The media, for example the label side of a CD (`6`).
    Media,
    /// The lead artist or lead performer (`7`).
    LeadArtist,
    /// The artist or performer (`8`).
    Artist,
    /// The conductor (`9`).
    Conductor,
    /// The band or orchestra (`10`).
    Band,
    /// The composer (`11`).
    Composer,
    /// The lyricist or text writer (`12`).
    Lyricist,
    /// The recording location (`13`).
    RecordingLocation,
    /// During the recording (`14`).
    DuringRecording,
    /// During the performance (`15`).
    DuringPerformance,
    /// A movie or video screen capture (`16`).
    ScreenCapture,
    /// A bright coloured fish (`17`).
    BrightFish,
    /// An illustration (`18`).
    Illustration,
    /// The band or artist logotype (`19`).
    BandLogo,
    /// The publisher or studio logotype (`20`).
    PublisherLogo,
    /// A picture type outside the table (`21` and above).
    Undefined(u8),
}

impl PictureKind {
    /// The picture type as the tag stores it.
    ///
    /// The value is the ID3v2 `APIC` picture type, so it can be written back
    /// into a tag of the same family.
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Other => 0,
            Self::Icon => 1,
            Self::OtherIcon => 2,
            Self::FrontCover => 3,
            Self::BackCover => 4,
            Self::Leaflet => 5,
            Self::Media => 6,
            Self::LeadArtist => 7,
            Self::Artist => 8,
            Self::Conductor => 9,
            Self::Band => 10,
            Self::Composer => 11,
            Self::Lyricist => 12,
            Self::RecordingLocation => 13,
            Self::DuringRecording => 14,
            Self::DuringPerformance => 15,
            Self::ScreenCapture => 16,
            Self::BrightFish => 17,
            Self::Illustration => 18,
            Self::BandLogo => 19,
            Self::PublisherLogo => 20,
            Self::Undefined(value) => value,
        }
    }

    /// The picture type of a stored number.
    ///
    /// Numbers outside the table become [`PictureKind::Undefined`].
    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Other,
            1 => Self::Icon,
            2 => Self::OtherIcon,
            3 => Self::FrontCover,
            4 => Self::BackCover,
            5 => Self::Leaflet,
            6 => Self::Media,
            7 => Self::LeadArtist,
            8 => Self::Artist,
            9 => Self::Conductor,
            10 => Self::Band,
            11 => Self::Composer,
            12 => Self::Lyricist,
            13 => Self::RecordingLocation,
            14 => Self::DuringRecording,
            15 => Self::DuringPerformance,
            16 => Self::ScreenCapture,
            17 => Self::BrightFish,
            18 => Self::Illustration,
            19 => Self::BandLogo,
            20 => Self::PublisherLogo,
            value => Self::Undefined(value),
        }
    }

    /// Whether this picture is the front cover.
    pub fn is_front_cover(self) -> bool {
        matches!(self, Self::FrontCover)
    }
}

/// An embedded picture read from a tag.
///
/// `rustamp-meta` exposes the stored bytes verbatim; it does not decode,
/// resize or otherwise process the image, and it does not know whether the
/// bytes form a valid picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artwork {
    /// The role the picture claims in the tag.
    pub kind: PictureKind,
    /// The MIME type stored in the tag (for example `"image/jpeg"`), if the
    /// format carries one.
    pub mime_type: Option<String>,
    /// The picture description, if the format carries one.
    pub description: Option<String>,
    /// The picture bytes exactly as stored in the tag.
    pub data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_type_table_round_trips() {
        for value in 0u8..=20 {
            let kind = PictureKind::from_u8(value);
            assert_eq!(kind.as_u8(), value, "picture type {value}");
        }
    }

    #[test]
    fn a_picture_type_outside_the_table_is_undefined() {
        let kind = PictureKind::from_u8(21);
        assert_eq!(kind, PictureKind::Undefined(21));
        assert_eq!(kind.as_u8(), 21);
        assert!(!kind.is_front_cover());
    }

    #[test]
    fn only_the_front_cover_is_the_front_cover() {
        assert!(PictureKind::FrontCover.is_front_cover());
        assert!(!PictureKind::BackCover.is_front_cover());
        assert!(!PictureKind::Other.is_front_cover());
    }
}
