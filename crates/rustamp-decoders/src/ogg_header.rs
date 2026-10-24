//! The pages of an Ogg file: the end-of-stream marker and the page walk.
//!
//! An Ogg file is a sequence of **pages**. Every page starts with a header of
//! 27 bytes, followed by a segment table and the body of the page:
//!
//! ```text
//! "OggS"          the capture pattern
//! version         0
//! flags           bit 0: the page continues a packet of the page before
//!                 bit 1: the page is the first of its logical bitstream
//!                 bit 2: the page is the last of its logical bitstream
//! granule         the absolute granule position (8 bytes)
//! serial          the serial number of the logical bitstream (4 bytes)
//! sequence        the page sequence number (4 bytes)
//! checksum        the checksum of the page (4 bytes)
//! segments        the number of entries in the segment table (1 byte)
//! segment table   `segments` bytes, each one a lacing value
//! ```
//!
//! Every count of the header is little endian. The segment table states the
//! length of the body: the sum of its lacing values.
//!
//! **The end-of-stream flag is the oracle.** The last page of a logical
//! bitstream carries it, so an intact file ends with a page that has it. A
//! file whose last page is intact and does not have it was cut: the bitstream
//! itself says that more is to come. A file that ends *inside* a page was cut
//! as well - an intact file always ends at the boundary of a page.
//!
//! The checksum is the format's own integrity check: it covers the whole
//! page - the header with its checksum field zeroed, the segment table and
//! the body - with the CRC-32 of the format (the polynomial `0x04c11db7`,
//! neither reflected in nor out, and no final inversion). It is what tells a
//! damaged page from a cut file: the page that would say "the stream ends
//! here" cannot be read when it is damaged, so a damaged file is not a cut
//! file and keeps playing.
//!
//! The page sequence numbers count the pages of a logical bitstream. A page
//! that does not follow its predecessor means that pages are missing: that
//! is damage too, and not a cut.
//!
//! The layout is the public Ogg container layout (the capture pattern, the
//! page header, the segment table and the checksum). No Winamp source, SDK
//! header or disassembly was consulted and nothing was copied.

use std::io::{self, SeekFrom};

use rustamp_core::audio::ReadSeek;
use symphonia::core::checksum::Crc32;
use symphonia::core::io::Monitor;

/// The capture pattern at the start of every page.
const CAPTURE: [u8; 4] = *b"OggS";

/// The length of the page header.
const HEADER_LEN: usize = 27;

/// The offset of the checksum field in the page header.
const CHECKSUM_AT: usize = 22;

/// The end-of-stream flag: the page is the last of its logical bitstream.
const FLAG_END_OF_STREAM: u8 = 0x04;

/// The first-page flag: the page is the first of its logical bitstream.
const FLAG_FIRST_PAGE: u8 = 0x02;

/// The flag bits the format defines.
const FLAG_KNOWN: u8 = 0x07;

/// What the walk of the pages of an Ogg file found.
#[derive(Debug, Clone, Copy)]
pub(crate) enum OggPages {
    /// The file ends at the end of a page, and that page is intact.
    ///
    /// `end_of_stream` tells whether the page carries the end-of-stream
    /// flag: an intact file ends with such a page.
    Complete {
        /// Whether the last page of the file carries the end-of-stream flag.
        end_of_stream: bool,
        /// The absolute granule position of the last page: the number of
        /// samples the file holds up to that page.
        granule: u64,
    },
    /// The file ends inside a page: the file was cut.
    Cut {
        /// The offset of the page the file ends in.
        offset: u64,
    },
    /// The pages of the file show damage: nothing can be said about the file.
    Damaged,
}

/// The header of an Ogg page.
#[derive(Debug, Clone, Copy)]
struct PageHeader {
    /// The header type flags.
    flags: u8,
    /// The absolute granule position.
    granule: u64,
    /// The serial number of the logical bitstream the page belongs to.
    serial: u32,
    /// The page sequence number.
    sequence: u32,
    /// The checksum of the page.
    checksum: u32,
    /// The number of entries in the segment table.
    segments: u8,
}

impl PageHeader {
    /// Parses the page header at the start of `bytes`.
    ///
    /// `None` when the bytes are not a page header: the capture pattern is
    /// missing, the stream structure version is not the one the format
    /// defines, or a flag bit is set that the format does not define.
    fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < HEADER_LEN || bytes[..4] != CAPTURE {
            return None;
        }
        // The stream structure version: 0 is the only one there is.
        if bytes[4] != 0 {
            return None;
        }
        let flags = bytes[5];
        if flags & !FLAG_KNOWN != 0 {
            return None;
        }
        Some(Self {
            flags,
            granule: u64::from_le_bytes(bytes[6..14].try_into().ok()?),
            serial: u32::from_le_bytes(bytes[14..18].try_into().ok()?),
            sequence: u32::from_le_bytes(bytes[18..22].try_into().ok()?),
            checksum: u32::from_le_bytes(bytes[22..26].try_into().ok()?),
            segments: bytes[26],
        })
    }

    /// Whether this page carries the end-of-stream flag: it is the last page
    /// of its logical bitstream.
    fn end_of_stream(&self) -> bool {
        self.flags & FLAG_END_OF_STREAM != 0
    }

    /// Whether this page is the first page of its logical bitstream.
    fn first_page(&self) -> bool {
        self.flags & FLAG_FIRST_PAGE != 0
    }
}

/// Whether the bytes look like the start of an Ogg file.
///
/// The check is cheap on purpose: the walk reads every page of the file, and
/// that is only worth doing for a file that can be one.
pub(crate) fn looks_like_ogg(window: &[u8]) -> bool {
    window.starts_with(&CAPTURE)
}

/// Walks the pages of the Ogg file that `reader` holds.
///
/// The walk reads every page and checks its checksum. A page whose checksum
/// does not match is damaged, and the extent of a damaged page cannot be
/// trusted (the checksum covers the segment table, which states the extent),
/// so the walk stops there and says nothing about the file.
///
/// `None` when the file holds no page at all: it is not an Ogg file.
pub(crate) fn parse(reader: &mut dyn ReadSeek) -> Option<OggPages> {
    reader.seek(SeekFrom::Start(0)).ok()?;
    // The capture pattern of the first page: a file that does not start with
    // it is not an Ogg file.
    let mut capture = [0u8; 4];
    if read_up_to(reader, &mut capture)? < capture.len() || capture != CAPTURE {
        return None;
    }
    // The offset of the page the walk is at.
    let mut offset = 0u64;
    // The sequence number of the last page of every logical bitstream.
    let mut sequences: Vec<(u32, u32)> = Vec::new();
    // Whether the pages of the file show damage.
    let mut damaged = false;
    // The body of the page, reused from page to page.
    let mut body: Vec<u8> = Vec::new();

    loop {
        // The page header behind the capture pattern.
        let mut head = [0u8; HEADER_LEN];
        head[..4].copy_from_slice(&CAPTURE);
        if read_up_to(reader, &mut head[4..])? < HEADER_LEN - 4 {
            // The file ends inside the page header: it was cut.
            return Some(OggPages::Cut { offset });
        }
        let Some(header) = PageHeader::parse(&head) else {
            // The bytes are not a page header: the structure is broken.
            return Some(OggPages::Damaged);
        };
        // The segment table: it states the length of the body.
        let mut table = vec![0u8; usize::from(header.segments)];
        if read_up_to(reader, &mut table)? < table.len() {
            // The file ends inside the segment table: it was cut.
            return Some(OggPages::Cut { offset });
        }
        let body_len: usize = table.iter().map(|&lacing| usize::from(lacing)).sum();
        body.resize(body_len, 0);
        if read_up_to(reader, &mut body)? < body_len {
            // The file ends inside the body: it was cut.
            return Some(OggPages::Cut { offset });
        }
        // The checksum of the page: the header (with its checksum field
        // zeroed), the segment table and the body.
        if page_checksum(&head, &table, &body) != header.checksum {
            // The page is damaged, so its extent cannot be trusted.
            return Some(OggPages::Damaged);
        }
        // The sequence numbers of the pages: a gap means that pages are
        // missing, and that is damage.
        if !sequence_is_continuous(&mut sequences, &header) {
            damaged = true;
        }
        offset += (HEADER_LEN + table.len() + body_len) as u64;

        // The capture pattern of the next page.
        let got = read_up_to(reader, &mut capture)?;
        if got == 0 {
            // The file ends here: the page before is the last one.
            return Some(complete_or_damaged(&header, damaged));
        }
        if got < capture.len() {
            // The file ends inside the capture pattern of a page: it was cut.
            // Anything else behind the last page is trailing data (a tag, a
            // padding), and then the last page is the last one.
            return Some(if capture[..got] == CAPTURE[..got] {
                OggPages::Cut { offset }
            } else {
                complete_or_damaged(&header, damaged)
            });
        }
        if capture != CAPTURE {
            // The bytes behind the last page are not a page: they are
            // trailing data, and the last page is the last one.
            return Some(complete_or_damaged(&header, damaged));
        }
        // The next page follows.
    }
}

/// The verdict for a file that ends at the end of a page.
///
/// The pages of the file show damage: then nothing can be said about it.
fn complete_or_damaged(header: &PageHeader, damaged: bool) -> OggPages {
    if damaged {
        return OggPages::Damaged;
    }
    OggPages::Complete {
        end_of_stream: header.end_of_stream(),
        granule: header.granule,
    }
}

/// The checksum of an Ogg page.
///
/// The checksum covers the whole page: the header with its checksum field
/// zeroed, the segment table and the body.
fn page_checksum(header: &[u8; HEADER_LEN], table: &[u8], body: &[u8]) -> u32 {
    let mut zeroed = *header;
    zeroed[CHECKSUM_AT..CHECKSUM_AT + 4].fill(0);
    let mut crc = Crc32::new(0);
    crc.process_buf_bytes(&zeroed);
    crc.process_buf_bytes(table);
    crc.process_buf_bytes(body);
    crc.crc()
}

/// Records the sequence number of a page and tells whether the page follows
/// its predecessor.
///
/// The sequence numbers count the pages of a logical bitstream, so a page
/// that does not follow its predecessor means that pages are missing. That is
/// damage, not a cut file.
fn sequence_is_continuous(sequences: &mut Vec<(u32, u32)>, header: &PageHeader) -> bool {
    match sequences
        .iter()
        .position(|(serial, _)| *serial == header.serial)
    {
        // The first page of this logical bitstream: it starts the numbering.
        None => {
            sequences.push((header.serial, header.sequence));
            true
        }
        Some(at) => {
            let previous = sequences[at].1;
            sequences[at].1 = header.sequence;
            // The first page of a logical bitstream starts its own numbering,
            // so it does not follow the page before it.
            if header.first_page() {
                return true;
            }
            header.sequence == previous.wrapping_add(1)
        }
    }
}

/// Reads up to `buf.len()` bytes into `buf`.
///
/// The number of bytes that are there, or `None` when the read failed for a
/// reason other than the end of the file - the decoder's own reader reports
/// that.
fn read_up_to(reader: &mut dyn ReadSeek, buf: &mut [u8]) -> Option<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
    Some(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// The repository fixture: an Ogg Vorbis file that ends with a page that
    /// carries the end-of-stream flag.
    fn fixture() -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sine.ogg");
        std::fs::read(path).expect("the Ogg fixture")
    }

    /// Builds a page with the given flags, sequence number and body.
    ///
    /// The segment table and the checksum are the ones the format asks for,
    /// so the page is a valid one.
    fn page(flags: u8, sequence: u32, body: &[u8]) -> Vec<u8> {
        // The lacing values of the segment table: a packet longer than 255
        // bytes needs one value of 255 per 255 bytes and a last value for
        // the rest.
        let mut table = Vec::new();
        let mut left = body.len();
        while left >= 255 {
            table.push(255u8);
            left -= 255;
        }
        table.push(left as u8);
        let mut head = [0u8; HEADER_LEN];
        head[..4].copy_from_slice(&CAPTURE);
        head[4] = 0;
        head[5] = flags;
        head[6..14].copy_from_slice(&0u64.to_le_bytes());
        head[14..18].copy_from_slice(&1u32.to_le_bytes());
        head[18..22].copy_from_slice(&sequence.to_le_bytes());
        head[26] = table.len() as u8;
        let checksum = page_checksum(&head, &table, body);
        head[CHECKSUM_AT..CHECKSUM_AT + 4].copy_from_slice(&checksum.to_le_bytes());
        let mut out = head.to_vec();
        out.extend_from_slice(&table);
        out.extend_from_slice(body);
        out
    }

    /// The pages of the fixture, with the offset of every page.
    fn fixture_pages(bytes: &[u8]) -> Vec<(usize, usize, u8)> {
        let mut pages = Vec::new();
        let mut at = 0usize;
        while at + HEADER_LEN <= bytes.len() {
            assert_eq!(&bytes[at..at + 4], &CAPTURE, "a page starts at {at}");
            let segments = usize::from(bytes[at + 26]);
            let table_at = at + HEADER_LEN;
            let body_len: usize = bytes[table_at..table_at + segments]
                .iter()
                .map(|&lacing| usize::from(lacing))
                .sum();
            let end = table_at + segments + body_len;
            pages.push((at, end, bytes[at + 5]));
            at = end;
        }
        assert_eq!(at, bytes.len(), "the file ends at the end of a page");
        pages
    }

    /// The intact fixture ends with a page that carries the end-of-stream
    /// flag, and every page of it has a correct checksum.
    #[test]
    fn the_fixture_ends_with_an_intact_end_of_stream_page() {
        let bytes = fixture();
        let pages = parse(&mut Cursor::new(bytes)).expect("an Ogg file");
        assert!(
            matches!(
                pages,
                OggPages::Complete {
                    end_of_stream: true,
                    ..
                }
            ),
            "{pages:?}"
        );
    }

    /// The last page of the fixture carries the flag, and it is the only page
    /// that carries it.
    #[test]
    fn the_last_page_of_the_fixture_carries_the_end_of_stream_flag() {
        let bytes = fixture();
        let pages = fixture_pages(&bytes);
        let (at, _, flags) = *pages.last().expect("the fixture has pages");
        assert_ne!(flags & FLAG_END_OF_STREAM, 0, "the last page at {at}");
        assert_eq!(
            pages
                .iter()
                .filter(|(_, _, flags)| flags & FLAG_END_OF_STREAM != 0)
                .count(),
            1,
            "only the last page carries the flag"
        );
    }

    /// The granule position of the last page is the number of samples the
    /// file holds: the reader's own count is derived from it.
    #[test]
    fn the_granule_position_of_the_last_page_is_the_sample_count() {
        let bytes = fixture();
        let pages = parse(&mut Cursor::new(bytes)).expect("an Ogg file");
        match pages {
            OggPages::Complete { granule, .. } => {
                // The fixture is 1.5 s of 44.1 kHz audio.
                assert_eq!(granule, 66_150, "the last page states the samples");
            }
            other => panic!("the fixture is complete: {other:?}"),
        }
    }

    /// A file whose last page is intact and does not carry the flag was cut:
    /// the bitstream says that more is to come.
    #[test]
    fn a_file_without_an_end_of_stream_flag_is_not_complete() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&page(FLAG_FIRST_PAGE, 0, b"first"));
        bytes.extend_from_slice(&page(0, 1, b"second"));
        let pages = parse(&mut Cursor::new(bytes)).expect("an Ogg file");
        assert!(
            matches!(
                pages,
                OggPages::Complete {
                    end_of_stream: false,
                    ..
                }
            ),
            "{pages:?}"
        );
    }

    /// A file that ends inside a page was cut.
    #[test]
    fn a_file_that_ends_inside_a_page_is_a_cut() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&page(FLAG_FIRST_PAGE, 0, b"first"));
        bytes.extend_from_slice(&page(0, 1, b"second"));
        // Cut in the middle of the body of the second page.
        bytes.truncate(bytes.len() - 3);
        let pages = parse(&mut Cursor::new(bytes)).expect("an Ogg file");
        match pages {
            OggPages::Cut { offset } => assert!(offset > 0, "the cut is in a page"),
            other => panic!("a file that ends inside a page must be named a cut: {other:?}"),
        }
    }

    /// A file that ends in the middle of a page header was cut as well.
    #[test]
    fn a_file_that_ends_inside_a_page_header_is_a_cut() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&page(FLAG_FIRST_PAGE, 0, b"first"));
        bytes.extend_from_slice(&page(0, 1, b"second"));
        // Cut in the middle of the header of the second page.
        bytes.truncate(bytes.len() - 3 - 5);
        let pages = parse(&mut Cursor::new(bytes)).expect("an Ogg file");
        assert!(matches!(pages, OggPages::Cut { .. }), "{pages:?}");
    }

    /// A page whose checksum does not match is damaged: the walk cannot trust
    /// its extent, so it says nothing about the file.
    #[test]
    fn a_damaged_page_stops_the_walk() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&page(FLAG_FIRST_PAGE, 0, b"first"));
        let mut second = page(0, 1, b"second");
        // Damage the body of the second page: its checksum does not match any
        // more.
        let body_at = second.len() - 6;
        second[body_at] ^= 0xA5;
        bytes.extend_from_slice(&second);
        let pages = parse(&mut Cursor::new(bytes)).expect("an Ogg file");
        assert!(matches!(pages, OggPages::Damaged), "{pages:?}");
    }

    /// A gap in the page sequence numbers means that pages are missing: that
    /// is damage, not a cut.
    #[test]
    fn a_gap_in_the_page_sequence_numbers_is_damage() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&page(FLAG_FIRST_PAGE, 0, b"first"));
        // The third page is missing: the sequence jumps from 1 to 3.
        bytes.extend_from_slice(&page(0, 3, b"third"));
        let pages = parse(&mut Cursor::new(bytes)).expect("an Ogg file");
        assert!(matches!(pages, OggPages::Damaged), "{pages:?}");
    }

    /// A page that does not start with the capture pattern is not a page: the
    /// bytes behind the last page are trailing data, and the last page is the
    /// last one.
    #[test]
    fn trailing_data_behind_the_last_page_is_not_a_page() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&page(FLAG_FIRST_PAGE, 0, b"first"));
        bytes.extend_from_slice(b"not a page");
        let pages = parse(&mut Cursor::new(bytes)).expect("an Ogg file");
        match pages {
            OggPages::Complete { end_of_stream, .. } => {
                assert!(!end_of_stream, "the last page has no end-of-stream flag");
            }
            other => panic!("the file ends at the end of its last page: {other:?}"),
        }
    }

    /// A file that is not an Ogg file is not an Ogg file.
    #[test]
    fn other_files_are_not_parsed_as_ogg() {
        assert!(!looks_like_ogg(b"RIFF....WAVE"));
        assert!(!looks_like_ogg(b""));
        assert!(looks_like_ogg(b"OggS\0\0\0\0"));
        assert!(parse(&mut Cursor::new(b"RIFF....WAVE".to_vec())).is_none());
        assert!(parse(&mut Cursor::new(Vec::new())).is_none());
    }
}
