//! The Xing/Info header of an MPEG audio stream.
//!
//! An MPEG audio file may carry a **Xing** (variable bitrate) or **Info**
//! (constant bitrate) tag in its first frame. The tag states how many MPEG
//! frames and how many bytes the audio data holds, and that is the only
//! exact oracle an MPEG audio file has: without the tag a reader can only
//! *estimate* the frame count from the bitrate, and an estimate is not
//! evidence of a cut file.
//!
//! The tag sits behind the side information of the first frame, so its
//! position depends on the frame header: the MPEG version, the channel mode
//! and the CRC bit decide it. The layout is:
//!
//! ```text
//! "Xing" | "Info"   the tag id ("Info" for a constant bitrate file)
//! flags             bit 0: a frame count follows
//!                   bit 1: a byte count follows
//!                   bit 2: a seek table follows
//!                   bit 3: a quality indicator follows
//! frames            the number of MPEG frames (only with bit 0)
//! bytes             the number of bytes (only with bit 1)
//! ```
//!
//! All counts are big endian.
//!
//! Two conventions differ between encoders and are handled here:
//!
//! * The **frame count** does not include the frame that carries the tag:
//!   that frame holds the tag, not audio. Every encoder measured (LAME,
//!   ffmpeg) counts the audio frames only.
//! * The **byte count** covers the audio data, but some encoders count the
//!   tag frame with it and some do not. It is therefore never used as an
//!   exact length, only as evidence that bytes are missing.

/// The payload an MPEG audio file declares in its Xing/Info tag.
#[derive(Debug, Clone, Copy)]
pub(crate) struct XingHeader {
    /// The number of MPEG frames the file holds. The frame that carries the
    /// tag is not counted: it holds the tag, not audio.
    pub(crate) frames: u32,
    /// The number of bytes the audio data occupies.
    pub(crate) bytes: u32,
    /// The number of audio samples one MPEG frame carries.
    pub(crate) samples_per_frame: u64,
    /// The offset of the first MPEG frame in the file.
    pub(crate) start: u64,
}

/// The number of bytes the ID3v2 tag at the start of `head` occupies.
///
/// The size in an ID3v2 header covers everything behind the header itself
/// (the frames, the padding and, for ID3v2.4, the footer), so the audio
/// starts `10 + size` bytes into the file. A file without an ID3v2 tag
/// starts its audio at 0.
pub(crate) fn id3v2_len(head: &[u8]) -> usize {
    if head.len() < 10 || &head[0..3] != b"ID3" {
        return 0;
    }
    // The size is stored "syncsafe": seven bits per byte, the most
    // significant byte first.
    let size = (u32::from(head[6] & 0x7F) << 21)
        | (u32::from(head[7] & 0x7F) << 14)
        | (u32::from(head[8] & 0x7F) << 7)
        | u32::from(head[9] & 0x7F);
    10 + size as usize
}

/// Parses the Xing/Info tag of the first MPEG frame.
///
/// `id3v2` is the length of the ID3v2 tag at the start of the file (0 when
/// there is none) and `window` the bytes that follow it. `None` when the
/// file carries no such tag, or a tag that states neither a frame count nor
/// a byte count: both are needed to tell a cut file from a damaged one.
pub(crate) fn parse(id3v2: usize, window: &[u8]) -> Option<XingHeader> {
    let (offset, header) = first_frame(window)?;
    let at = offset + tag_offset(&header);
    let id = window.get(at..at + 4)?;
    if id != b"Xing" && id != b"Info" {
        return None;
    }
    let flags = u32::from_be_bytes(four(window, at + 4)?);
    // The frame count (bit 0) and the byte count (bit 1) are the two numbers
    // the tag states about the payload. Without both of them there is no
    // oracle: a frame count alone cannot tell a cut file from a damaged one,
    // because a damaged MPEG frame costs frames without costing bytes.
    if flags & 0x01 == 0 || flags & 0x02 == 0 {
        return None;
    }
    let frames = u32::from_be_bytes(four(window, at + 8)?);
    let bytes = u32::from_be_bytes(four(window, at + 12)?);
    Some(XingHeader {
        frames,
        bytes,
        samples_per_frame: header.samples_per_frame,
        start: id3v2 as u64 + offset as u64,
    })
}

/// The first MPEG frame in `window`, with the offset it starts at.
fn first_frame(window: &[u8]) -> Option<(usize, FrameHeader)> {
    for offset in 0..window.len().saturating_sub(3) {
        if let Some(header) = FrameHeader::parse(&window[offset..]) {
            return Some((offset, header));
        }
    }
    None
}

/// The offset of the Xing/Info tag inside a frame, behind its side
/// information.
fn tag_offset(header: &FrameHeader) -> usize {
    // The frame header, the CRC it may carry and the side information.
    4 + usize::from(header.crc) * 2 + header.side_info_len()
}

/// Four bytes of `window` at `at`.
fn four(window: &[u8], at: usize) -> Option<[u8; 4]> {
    window.get(at..at + 4)?.try_into().ok()
}

/// The MPEG audio version of a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Version {
    Mpeg1,
    Mpeg2,
    Mpeg2p5,
}

/// The fields of an MPEG audio frame header the Xing/Info tag needs.
#[derive(Debug, Clone, Copy)]
struct FrameHeader {
    version: Version,
    mono: bool,
    /// Whether the frame carries a 16 bit CRC behind its header.
    crc: bool,
    /// The number of audio samples the frame carries.
    samples_per_frame: u64,
}

impl FrameHeader {
    /// Parses the frame header at the start of `bytes`.
    ///
    /// `None` when the bytes are not a frame header. The checks are the ones
    /// a reader has to make to find a frame in a stream: the sync word, the
    /// reserved version and layer values, the "free" bitrate and the reserved
    /// sample rate.
    fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 || bytes[0] != 0xFF || (bytes[1] & 0xE0) != 0xE0 {
            return None;
        }
        let version = match (bytes[1] >> 3) & 0x03 {
            0b00 => Version::Mpeg2p5,
            0b10 => Version::Mpeg2,
            0b11 => Version::Mpeg1,
            // 0b01 is reserved.
            _ => return None,
        };
        let layer = match (bytes[1] >> 1) & 0x03 {
            0b01 => 3,
            0b10 => 2,
            0b11 => 1,
            // 0b00 is reserved.
            _ => return None,
        };
        let bitrate = (bytes[2] >> 4) & 0x0F;
        // 0 is the "free" bitrate, 15 is invalid.
        if bitrate == 0 || bitrate == 15 {
            return None;
        }
        // 3 is the reserved sample rate.
        if (bytes[2] >> 2) & 0x03 == 3 {
            return None;
        }
        let mono = (bytes[3] >> 6) & 0x03 == 3;
        // The protection bit is 0 when a CRC follows the header.
        let crc = bytes[1] & 0x01 == 0;
        let samples_per_frame = match layer {
            1 => 384,
            2 => 1152,
            // Layer III carries two granules in MPEG 1 and one in MPEG 2/2.5.
            _ if version == Version::Mpeg1 => 1152,
            _ => 576,
        };
        Some(Self {
            version,
            mono,
            crc,
            samples_per_frame,
        })
    }

    /// The length of the side information behind the frame header.
    fn side_info_len(&self) -> usize {
        match (self.version, self.mono) {
            (Version::Mpeg1, true) => 17,
            (Version::Mpeg1, false) => 32,
            (_, true) => 9,
            (_, false) => 17,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A MPEG 1 layer III stereo frame header: 128 kbit/s at 44.1 kHz.
    const FRAME_HEADER: [u8; 4] = [0xFF, 0xFB, 0x90, 0x00];

    /// Builds a frame with a Xing/Info tag behind its side information.
    ///
    /// `flags` selects the fields the tag carries, `frames` and `bytes` are
    /// the counts it states.
    fn xing_frame(flags: u32, frames: u32, bytes: u32) -> Vec<u8> {
        let mut frame = FRAME_HEADER.to_vec();
        // 32 bytes of side information (MPEG 1, stereo).
        frame.extend_from_slice(&[0u8; 32]);
        frame.extend_from_slice(b"Xing");
        frame.extend_from_slice(&flags.to_be_bytes());
        if flags & 0x01 != 0 {
            frame.extend_from_slice(&frames.to_be_bytes());
        }
        if flags & 0x02 != 0 {
            frame.extend_from_slice(&bytes.to_be_bytes());
        }
        frame
    }

    #[test]
    fn the_tag_is_found_behind_the_side_information() {
        let frame = xing_frame(0x03, 59, 12537);
        let tag = parse(0, &frame).expect("the tag is there");
        assert_eq!(tag.frames, 59);
        assert_eq!(tag.bytes, 12537);
        assert_eq!(tag.samples_per_frame, 1152);
        assert_eq!(tag.start, 0);
    }

    #[test]
    fn an_id3v2_tag_is_skipped() {
        // A tag whose size is 16 bytes: the syncsafe size puts the low bits
        // in the last byte, so a plain big endian reading would be wrong.
        let mut file = b"ID3\x04\x00\x00\x00\x00\x00\x10".to_vec();
        // The syncsafe size says 16 bytes of tag behind the header.
        file.extend_from_slice(&[0u8; 16]);
        let frame = xing_frame(0x03, 59, 12537);
        file.extend_from_slice(&frame);
        assert_eq!(id3v2_len(&file), 26);
        let tag = parse(id3v2_len(&file), &file[26..]).expect("the tag is there");
        assert_eq!(tag.frames, 59);
        // The frame starts behind the tag.
        assert_eq!(tag.start, 26);
    }

    #[test]
    fn the_id3v2_size_is_read_syncsafe() {
        // 0x00 0x00 0x01 0x14 is 148 bytes syncsafe, not 276.
        let head = b"ID3\x04\x00\x00\x00\x00\x01\x14";
        assert_eq!(id3v2_len(head), 158);
        // A file without a tag starts its audio at 0.
        assert_eq!(id3v2_len(b"RIFF\0\0\0\0WAVE"), 0);
        assert_eq!(id3v2_len(b"ID3"), 0);
    }

    #[test]
    fn a_tag_without_both_counts_is_no_oracle() {
        // A frame count alone cannot tell a cut file from a damaged one.
        assert!(parse(0, &xing_frame(0x01, 59, 0)).is_none());
        // A byte count alone cannot either.
        assert!(parse(0, &xing_frame(0x02, 0, 12537)).is_none());
    }

    #[test]
    fn a_file_without_a_tag_is_no_oracle() {
        assert!(parse(0, &FRAME_HEADER).is_none());
        // A frame header that is not one.
        assert!(parse(0, &[0x00, 0x01, 0x02, 0x03]).is_none());
        // The reserved version and the "free" bitrate are not frames.
        assert!(parse(0, &[0xFF, 0xF3, 0x90, 0x00]).is_none());
        assert!(parse(0, &[0xFF, 0xFB, 0x00, 0x00]).is_none());
    }
}
