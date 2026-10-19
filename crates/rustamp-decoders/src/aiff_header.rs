//! The header of an AIFF file: what it states about its payload.
//!
//! An AIFF file is a `FORM` chunk whose form type is `AIFF` (uncompressed
//! PCM) or `AIFC` (the same, with the compression named in the `COMM`
//! chunk). Two of its sub-chunks matter here:
//!
//! * `COMM` states the number of sample frames the file holds, together with
//!   the sample format.
//! * `SSND` holds the sound data. Its size field covers the eight bytes of
//!   its own header - the `offset` and the `blockSize` field - plus the
//!   sound data.
//!
//! The container reader counts the whole `SSND` chunk as audio: the frame
//! count it reports is the chunk size divided by the block align, which is up
//! to `8 / block_align` frames more than the file holds. The count the file
//! states is the one in the `COMM` chunk; the correction that makes the two
//! comparable is the `SSND` header (see [`AiffHeader::reader_frames`] and
//! [`AiffHeader::sound_frames`]).
//!
//! The layout is the public AIFF container layout (the `FORM`/`COMM`/`SSND`
//! chunked form and the 80-bit extended float the sample rate is stored in).
//! No Winamp source, SDK header or disassembly was consulted and nothing was
//! copied.

use std::io::SeekFrom;

use rustamp_core::audio::ReadSeek;

/// The bytes the `SSND` chunk spends on its own header: the `offset` and the
/// `blockSize` field. They are part of the chunk's size, but they are not
/// audio.
const SSND_HEADER_LEN: u64 = 8;

/// What an AIFF file states about its payload.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AiffHeader {
    /// The number of audio frames the `COMM` chunk declares.
    frames: u64,
    /// The size of the `SSND` chunk, its own header included.
    ssnd_len: u64,
    /// The offset of the sound data in the file.
    data_start: u64,
    /// The number of bytes one audio frame occupies, as the reader counts it.
    block_align: u64,
}

impl AiffHeader {
    /// The number of bytes of sound data the `SSND` chunk declares: its size
    /// less the eight bytes of its own header.
    pub(crate) fn sound_bytes(&self) -> u64 {
        self.ssnd_len.saturating_sub(SSND_HEADER_LEN)
    }

    /// The number of frames the sound data holds.
    pub(crate) fn sound_frames(&self) -> u64 {
        self.sound_bytes() / self.block_align
    }

    /// The number of frames the container reader reports for this file.
    ///
    /// The reader divides the whole `SSND` chunk by the block align, so the
    /// eight bytes of the chunk's own header count as audio: the count is
    /// [`AiffHeader::sound_frames`] plus up to `8 / block_align` frames.
    pub(crate) fn reader_frames(&self) -> u64 {
        self.ssnd_len / self.block_align
    }

    /// The number of audio frames the file declares.
    ///
    /// The `COMM` chunk states it. A file that states none (0) falls back to
    /// the frames its sound data holds.
    pub(crate) fn declared_frames(&self) -> u64 {
        if self.frames > 0 {
            self.frames
        } else {
            self.sound_frames()
        }
    }

    /// The offset of the sound data in the file.
    pub(crate) fn data_start(&self) -> u64 {
        self.data_start
    }
}

/// The `COMM` chunk of an AIFF file.
struct Comm {
    /// The number of channels.
    channels: u16,
    /// The number of sample frames the file holds.
    frames: u64,
    /// The number of bits per sample.
    sample_size: u16,
    /// The compression type of an AIFC file; `None` for a plain AIFF file.
    compression: Option<[u8; 4]>,
}

/// Parses the header of an AIFF or AIFC file.
///
/// `None` when the file is not an AIFF file, when it states no frame count,
/// or when its `SSND` chunk is one the reader refuses (a block-aligned file).
/// `len` is the length of the file when it is known; the walk stops at the
/// end of the file either way.
pub(crate) fn parse(reader: &mut dyn ReadSeek, len: Option<u64>) -> Option<AiffHeader> {
    reader.seek(SeekFrom::Start(0)).ok()?;
    let mut head = [0u8; 12];
    reader.read_exact(&mut head).ok()?;
    if &head[0..4] != b"FORM" {
        return None;
    }
    // The plain `AIFF` form is uncompressed PCM; the `AIFC` form names its
    // compression in the `COMM` chunk.
    let compressed = match &head[8..12] {
        b"AIFF" => false,
        b"AIFC" => true,
        _ => return None,
    };
    let mut pos = 12u64;
    let mut comm: Option<Comm> = None;
    loop {
        if let Some(len) = len
            && pos + 8 > len
        {
            return None;
        }
        reader.seek(SeekFrom::Start(pos)).ok()?;
        let mut chunk = [0u8; 8];
        reader.read_exact(&mut chunk).ok()?;
        let tag = [chunk[0], chunk[1], chunk[2], chunk[3]];
        let size = u64::from(u32::from_be_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]));
        let body = pos + 8;
        match &tag {
            b"COMM" => {
                let mut fields = [0u8; 18];
                reader.read_exact(&mut fields).ok()?;
                let mut compression = None;
                if compressed {
                    let mut tag = [0u8; 4];
                    reader.read_exact(&mut tag).ok()?;
                    compression = Some(tag);
                }
                comm = Some(Comm {
                    channels: u16::from_be_bytes([fields[0], fields[1]]),
                    frames: u64::from(u32::from_be_bytes([
                        fields[2], fields[3], fields[4], fields[5],
                    ])),
                    sample_size: u16::from_be_bytes([fields[6], fields[7]]),
                    compression,
                });
            }
            b"SSND" => {
                let comm = comm?;
                let block_align = block_align(&comm)?;
                // The sound data starts behind the chunk's own header, which
                // holds the offset and the block size.
                let mut fields = [0u8; 8];
                reader.read_exact(&mut fields).ok()?;
                let offset = u32::from_be_bytes([fields[0], fields[1], fields[2], fields[3]]);
                let block_size = u32::from_be_bytes([fields[4], fields[5], fields[6], fields[7]]);
                if offset != 0 || block_size != 0 {
                    // A block-aligned file: the reader refuses it, so there is
                    // no count to compare with.
                    return None;
                }
                return Some(AiffHeader {
                    frames: comm.frames,
                    ssnd_len: size,
                    data_start: body + SSND_HEADER_LEN,
                    block_align,
                });
            }
            _ => {}
        }
        // The next chunk starts behind this one; an odd size is padded to an
        // even one.
        pos = body + size + (size & 1);
    }
}

/// The number of bytes one audio frame occupies, as the reader counts it.
///
/// `None` for a format the reader does not accept: then its frame count says
/// nothing about this file.
fn block_align(comm: &Comm) -> Option<u64> {
    if comm.channels == 0 {
        return None;
    }
    let channels = u64::from(comm.channels);
    let sample_size = u64::from(comm.sample_size);
    match comm.compression {
        // A plain AIFF file: uncompressed PCM of `sample_size` bits.
        None => matches!(comm.sample_size, 8 | 16 | 24 | 32).then_some(channels * sample_size / 8),
        Some(tag) => match &tag {
            b"none" | b"NONE" => {
                matches!(comm.sample_size, 8 | 16 | 24 | 32).then_some(channels * sample_size / 8)
            }
            // 16 bit little endian (`sowt`) and 16 bit big endian (`twos`).
            b"sowt" | b"SOWT" | b"twos" | b"TWOS" => Some(channels * 2),
            // A-law and mu-law: one byte per sample.
            b"alaw" | b"ALAW" | b"ulaw" | b"ULAW" => Some(channels),
            // IEEE float: 32 or 64 bits per sample.
            b"fl32" | b"FL32" => (comm.sample_size == 32).then_some(channels * 4),
            b"fl64" | b"FL64" => (comm.sample_size == 64).then_some(channels * 8),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// The 80-bit IEEE 754 extended float an AIFF file stores a sample rate
    /// in: the value is `2^(exponent - 16383) * mantissa / 2^63`, with the
    /// mantissa's integer bit set.
    fn extended_rate(rate: u32) -> [u8; 10] {
        let rate = rate.max(1);
        let exponent = 16383 + 31 - rate.leading_zeros();
        let mantissa = u64::from(rate) << (63 - (31 - rate.leading_zeros()));
        let mut out = [0u8; 10];
        out[..2].copy_from_slice(&(exponent as u16).to_be_bytes());
        out[2..].copy_from_slice(&mantissa.to_be_bytes());
        out
    }

    /// A minimal AIFF file: `FORM`/`AIFF` with a `COMM` chunk and an `SSND`
    /// chunk that holds `frames` frames of silence.
    fn aiff(channels: u16, bits: u16, frames: u32) -> Vec<u8> {
        let block_align = u32::from(channels) * u32::from(bits) / 8;
        let data_len = u64::from(frames) * u64::from(block_align);
        let ssnd_len = 8 + data_len;
        let mut out = Vec::new();
        out.extend_from_slice(b"FORM");
        out.extend_from_slice(&((4 + 26 + 8 + ssnd_len) as u32).to_be_bytes());
        out.extend_from_slice(b"AIFF");
        out.extend_from_slice(b"COMM");
        out.extend_from_slice(&18u32.to_be_bytes());
        out.extend_from_slice(&channels.to_be_bytes());
        out.extend_from_slice(&frames.to_be_bytes());
        out.extend_from_slice(&bits.to_be_bytes());
        out.extend_from_slice(&extended_rate(44_100));
        out.extend_from_slice(b"SSND");
        out.extend_from_slice(&(ssnd_len as u32).to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // offset
        out.extend_from_slice(&0u32.to_be_bytes()); // block size
        out.extend(std::iter::repeat_n(0u8, data_len as usize));
        out
    }

    /// The correction the whole oracle rests on: the reader counts the eight
    /// bytes of the `SSND` header as audio, so its frame count is the whole
    /// chunk divided by the block align.
    #[test]
    fn the_reader_counts_the_ssnd_header_as_audio() {
        let bytes = aiff(1, 16, 1000);
        let header = parse(&mut Cursor::new(bytes), None).expect("an AIFF header");
        assert_eq!(header.block_align, 2, "16 bit mono: two bytes per frame");
        // The file holds 1000 frames ...
        assert_eq!(header.sound_frames(), 1000);
        assert_eq!(header.declared_frames(), 1000);
        // ... the reader counts the eight bytes of the `SSND` header as
        // audio, so it reports four frames more (eight bytes / two bytes per
        // frame).
        assert_eq!(header.reader_frames(), 1004);
        // The sound data is what the `SSND` chunk holds without its header.
        assert_eq!(header.sound_bytes(), 2000);
    }

    /// The correction depends on the block align: the wider a frame, the
    /// fewer frames the eight header bytes are worth.
    #[test]
    fn the_correction_follows_the_block_align() {
        for (channels, bits, frames, correction) in [
            (1u16, 8u16, 1000u32, 8u64),
            (1, 16, 1000, 4),
            (2, 16, 1000, 2),
            (2, 24, 1000, 1),
        ] {
            let bytes = aiff(channels, bits, frames);
            let header = parse(&mut Cursor::new(bytes), None).expect("an AIFF header");
            assert_eq!(header.sound_frames(), u64::from(frames));
            assert_eq!(
                header.reader_frames() - header.sound_frames(),
                correction,
                "{channels} channels of {bits} bits"
            );
        }
    }

    /// A file that is not an AIFF file has no header to read.
    #[test]
    fn a_file_that_is_not_aiff_has_no_header() {
        assert!(parse(&mut Cursor::new(b"RIFF....WAVE".to_vec()), None).is_none());
        assert!(parse(&mut Cursor::new(b"FORM....AIFX".to_vec()), None).is_none());
        assert!(parse(&mut Cursor::new(Vec::new()), None).is_none());
    }

    /// A file without a `COMM` chunk states no frame count, and one without
    /// an `SSND` chunk holds no audio: neither is an oracle.
    #[test]
    fn an_incomplete_header_is_no_oracle() {
        // `FORM`/`AIFF` with an `SSND` chunk but no `COMM` chunk.
        let mut bytes = b"FORM".to_vec();
        bytes.extend_from_slice(&26u32.to_be_bytes());
        bytes.extend_from_slice(b"AIFF");
        bytes.extend_from_slice(b"SSND");
        bytes.extend_from_slice(&8u32.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        assert!(parse(&mut Cursor::new(bytes), None).is_none());

        // `FORM`/`AIFF` with a `COMM` chunk but no `SSND` chunk.
        let mut bytes = b"FORM".to_vec();
        bytes.extend_from_slice(&38u32.to_be_bytes());
        bytes.extend_from_slice(b"AIFF");
        bytes.extend_from_slice(b"COMM");
        bytes.extend_from_slice(&18u32.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&1000u32.to_be_bytes());
        bytes.extend_from_slice(&16u16.to_be_bytes());
        bytes.extend_from_slice(&extended_rate(44_100));
        assert!(parse(&mut Cursor::new(bytes), None).is_none());
    }

    /// A block-aligned file (a non-zero `offset` or `blockSize`) is one the
    /// reader refuses, so it is no oracle either.
    #[test]
    fn a_block_aligned_file_is_no_oracle() {
        let mut bytes = aiff(1, 16, 1000);
        // The `offset` field of the `SSND` chunk.
        let at = bytes
            .windows(4)
            .position(|w| w == b"SSND")
            .expect("the sound chunk")
            + 8;
        bytes[at..at + 4].copy_from_slice(&4u32.to_be_bytes());
        assert!(parse(&mut Cursor::new(bytes), None).is_none());
    }

    /// An AIFC file names its compression in the `COMM` chunk; the block
    /// align follows that name, not the sample size.
    #[test]
    fn an_aifc_file_states_its_compression() {
        let mut body = Vec::new();
        // The `FVER` chunk: the version of the AIFF-C format.
        body.extend_from_slice(b"FVER");
        body.extend_from_slice(&4u32.to_be_bytes());
        body.extend_from_slice(&0xA280_5140u32.to_be_bytes());
        // The `COMM` chunk: the usual fields, the compression type and the
        // compression name as a padded Pascal string.
        let mut comm = Vec::new();
        comm.extend_from_slice(&1u16.to_be_bytes());
        comm.extend_from_slice(&1000u32.to_be_bytes());
        comm.extend_from_slice(&16u16.to_be_bytes());
        comm.extend_from_slice(&extended_rate(44_100));
        comm.extend_from_slice(b"alaw");
        comm.extend_from_slice(&[4u8, b'n', b'o', b'n', b'e', 0]);
        body.extend_from_slice(b"COMM");
        body.extend_from_slice(&(comm.len() as u32).to_be_bytes());
        body.extend_from_slice(&comm);
        // The `SSND` chunk: one byte per sample, so 1000 frames.
        body.extend_from_slice(b"SSND");
        body.extend_from_slice(&(8 + 1000u32).to_be_bytes());
        body.extend_from_slice(&0u32.to_be_bytes()); // offset
        body.extend_from_slice(&0u32.to_be_bytes()); // block size
        body.extend(std::iter::repeat_n(0u8, 1000));

        let mut bytes = b"FORM".to_vec();
        bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
        bytes.extend_from_slice(b"AIFC");
        bytes.extend_from_slice(&body);

        let header = parse(&mut Cursor::new(bytes), None).expect("an AIFC header");
        // A-law stores one byte per sample, not the 16 bits the `COMM`
        // chunk names.
        assert_eq!(header.block_align, 1);
        assert_eq!(header.sound_frames(), 1000);
        assert_eq!(header.declared_frames(), 1000);
        assert_eq!(header.reader_frames(), 1008);
    }
}
