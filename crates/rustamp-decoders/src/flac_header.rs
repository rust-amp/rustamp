//! The metadata of a FLAC file: the chain of blocks that precedes the audio.
//!
//! A FLAC file is the `fLaC` marker, then a chain of metadata blocks, then
//! the audio frames. Every block starts with a header of four bytes: one type
//! byte - bit 7 marks the last block of the chain, bits 0 to 6 are the type -
//! and a 24 bit big endian length of the payload.
//!
//! The `STREAMINFO` block (type 0) is the one that states the audio
//! parameters: the sample rate, the channel count, the bit depth and the
//! number of samples. A file without it states no parameters at all, so a
//! reader cannot recognise a frame and runs into the end of the file - it
//! reports an end of stream for a file that is not at its end. The probe
//! names that case.
//!
//! The chain states the length of every block, so a file that ends inside one
//! was cut: it is not at its end, and an end of stream would be a lie. That
//! is the one thing the chain is an oracle for.
//!
//! What the chain does **not** state is the length of the audio: the
//! `STREAMINFO` block counts the samples, not the bytes, and a damaged FLAC
//! frame is dropped by the reader, so a corrupt file is short of samples
//! exactly like a cut one. The audio data therefore has no oracle at all
//! (see `CONTRACT_NOTES.md`).
//!
//! The layout is the public FLAC metadata layout (the `fLaC` marker and the
//! block headers). No Winamp source, SDK header or disassembly was consulted
//! and nothing was copied.

use std::io::{self, SeekFrom};

use rustamp_core::audio::ReadSeek;

/// The type of the `STREAMINFO` metadata block.
const STREAMINFO: u8 = 0;

/// The length of the `fLaC` marker.
const MAGIC_LEN: u64 = 4;

/// The length of a metadata block header: the type byte and the 24 bit
/// length of the payload.
const BLOCK_HEADER_LEN: u64 = 4;

/// What the metadata chain of a FLAC file says.
#[derive(Debug, Clone, Copy)]
pub(crate) enum FlacMetadata {
    /// The chain is complete. It holds a `STREAMINFO` block or it does not:
    /// that block is the one that states the audio parameters.
    Complete {
        /// Whether the chain holds a `STREAMINFO` block.
        stream_info: bool,
    },
    /// The file ends inside the chain: the metadata was cut.
    Cut {
        /// The offset of the block that the file cuts.
        offset: u64,
        /// The number of bytes that block claims, when its header was read.
        claimed: Option<u64>,
        /// The number of bytes of that block that are there, when the length
        /// of the file is known.
        available: Option<u64>,
    },
}

/// Whether the file holds a metadata block in full.
enum BlockState {
    /// The file holds the whole block.
    Complete,
    /// The file ends inside the block.
    Cut {
        /// The number of bytes of the block that are there, when the length
        /// of the file is known.
        available: Option<u64>,
    },
}

/// Whether the bytes look like the start of a FLAC file.
///
/// The check is cheap on purpose: the parse reads the block headers of the
/// file, and that is only worth doing for a file that can be one.
pub(crate) fn looks_like_flac(window: &[u8]) -> bool {
    window.starts_with(b"fLaC")
}

/// Reads the metadata chain of the FLAC file that starts at `from`.
///
/// `from` is the offset of the `fLaC` marker: a file may carry an ID3v2 tag
/// in front of it. `len` is the length of the file when it is known; a chain
/// that reaches beyond it was cut.
///
/// `None` when there is no FLAC file at `from`.
pub(crate) fn parse(
    reader: &mut dyn ReadSeek,
    from: u64,
    len: Option<u64>,
) -> Option<FlacMetadata> {
    reader.seek(SeekFrom::Start(from)).ok()?;
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic).ok()?;
    if &magic != b"fLaC" {
        return None;
    }
    let mut pos = from + MAGIC_LEN;
    let mut stream_info = false;
    loop {
        reader.seek(SeekFrom::Start(pos)).ok()?;
        let mut head = [0u8; 4];
        match reader.read_exact(&mut head) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                // The file ends before the next block header: the chain says
                // there is more of the file than there is.
                return Some(FlacMetadata::Cut {
                    offset: pos,
                    claimed: None,
                    available: None,
                });
            }
            // A real I/O failure is not a cut file: the decoder's own reader
            // reports it.
            Err(_) => return None,
        }
        let last = head[0] & 0x80 != 0;
        if head[0] & 0x7F == STREAMINFO {
            stream_info = true;
        }
        let claimed = u64::from(u32::from_be_bytes([0, head[1], head[2], head[3]]));
        let end = pos.saturating_add(BLOCK_HEADER_LEN).saturating_add(claimed);
        match block_state(reader, pos, end, len) {
            Some(BlockState::Cut { available }) => {
                return Some(FlacMetadata::Cut {
                    offset: pos,
                    claimed: Some(claimed),
                    available,
                });
            }
            Some(BlockState::Complete) => {}
            // The file cannot be judged: the decoder's own reader reports
            // the failure.
            None => return None,
        }
        pos = end;
        if last {
            return Some(FlacMetadata::Complete { stream_info });
        }
    }
}

/// Whether the file holds the block that starts at `start` in full.
///
/// The block ends at `end`: the header plus the payload it claims.
///
/// `None` when that cannot be told (a read failure that is not the end of
/// the file): the caller then says nothing about the file.
fn block_state(
    reader: &mut dyn ReadSeek,
    start: u64,
    end: u64,
    len: Option<u64>,
) -> Option<BlockState> {
    if let Some(len) = len {
        if end <= len {
            return Some(BlockState::Complete);
        }
        return Some(BlockState::Cut {
            available: Some(len.saturating_sub(start.saturating_add(BLOCK_HEADER_LEN))),
        });
    }
    // Without the length of the file: ask for the last byte of the block.
    let last = end.checked_sub(1)?;
    reader.seek(SeekFrom::Start(last)).ok()?;
    let mut byte = [0u8; 1];
    match reader.read_exact(&mut byte) {
        Ok(()) => Some(BlockState::Complete),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
            Some(BlockState::Cut { available: None })
        }
        // A real I/O failure is not a cut file.
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// The repository fixture: a FLAC file with a `STREAMINFO` block, a
    /// comment block and a padding block.
    fn fixture() -> Vec<u8> {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sine.flac");
        std::fs::read(path).expect("the FLAC fixture")
    }

    /// The chain of the fixture: the `STREAMINFO` block, a comment block and
    /// a padding block that ends the chain.
    #[test]
    fn the_chain_of_the_fixture_is_complete_and_holds_a_stream_info_block() {
        let bytes = fixture();
        let len = bytes.len() as u64;
        let metadata = parse(&mut Cursor::new(bytes), 0, Some(len)).expect("a FLAC file");
        assert!(
            matches!(metadata, FlacMetadata::Complete { stream_info: true }),
            "{metadata:?}"
        );
    }

    /// A file whose `STREAMINFO` block carries another type states no audio
    /// parameters: the chain is complete, the parameters are not there.
    #[test]
    fn a_file_without_a_stream_info_block_is_a_complete_chain() {
        let mut bytes = fixture();
        // The type byte of the first block: the last-block flag stays.
        bytes[4] = 126;
        let len = bytes.len() as u64;
        let metadata = parse(&mut Cursor::new(bytes), 0, Some(len)).expect("a FLAC file");
        assert!(
            matches!(metadata, FlacMetadata::Complete { stream_info: false }),
            "{metadata:?}"
        );
    }

    /// A file that ends inside a block was cut: the block claims more bytes
    /// than the file holds.
    #[test]
    fn a_file_that_ends_inside_a_block_is_a_cut() {
        let bytes = fixture();
        // The first block claims 34 bytes; the file holds 12 of them.
        let cut = &bytes[..20];
        let metadata =
            parse(&mut Cursor::new(cut.to_vec()), 0, Some(cut.len() as u64)).expect("a FLAC file");
        match metadata {
            FlacMetadata::Cut {
                offset,
                claimed,
                available,
            } => {
                assert_eq!(offset, 4, "the first block starts behind the marker");
                assert_eq!(claimed, Some(34), "the block states its own length");
                assert_eq!(available, Some(12), "the file holds 12 of them");
            }
            other => panic!("a cut chain must be named: {other:?}"),
        }
    }

    /// A file that ends between two blocks was cut as well: the chain says
    /// there is another block and there is none.
    #[test]
    fn a_file_that_ends_between_two_blocks_is_a_cut() {
        let bytes = fixture();
        // The first block ends at 42; the header of the next one is gone.
        let cut = &bytes[..42];
        let metadata =
            parse(&mut Cursor::new(cut.to_vec()), 0, Some(cut.len() as u64)).expect("a FLAC file");
        match metadata {
            FlacMetadata::Cut {
                offset,
                claimed,
                available,
            } => {
                assert_eq!(offset, 42, "the next block would start there");
                assert_eq!(claimed, None, "no block header was read");
                assert_eq!(available, None);
            }
            other => panic!("a cut chain must be named: {other:?}"),
        }
    }

    /// A file of unknown length is judged by its last byte: a block that is
    /// there in full is complete, one that is not is cut.
    #[test]
    fn a_file_of_unknown_length_is_judged_by_its_last_byte() {
        let bytes = fixture();
        let complete = parse(&mut Cursor::new(bytes.clone()), 0, None).expect("a FLAC file");
        assert!(
            matches!(complete, FlacMetadata::Complete { stream_info: true }),
            "{complete:?}"
        );

        let cut = &bytes[..20];
        let metadata = parse(&mut Cursor::new(cut.to_vec()), 0, None).expect("a FLAC file");
        assert!(matches!(metadata, FlacMetadata::Cut { .. }), "{metadata:?}");
    }

    /// Bytes that are not a FLAC file are not a FLAC file.
    #[test]
    fn other_files_are_not_parsed_as_flac() {
        assert!(parse(&mut Cursor::new(b"RIFF....WAVE".to_vec()), 0, None).is_none());
        assert!(parse(&mut Cursor::new(Vec::new()), 0, None).is_none());
        assert!(!looks_like_flac(b"RIFF....WAVE"));
        assert!(!looks_like_flac(b""));
        assert!(looks_like_flac(b"fLaC\0\0\0\0"));
    }
}
