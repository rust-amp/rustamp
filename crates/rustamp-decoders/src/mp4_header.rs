//! The header of an ISO base media file (MP4/M4A): what it states about its payload.
//!
//! An MP4 file is a tree of boxes. The boxes that matter here are:
//!
//! * `mdat`: the **media data** box. It states how many bytes of media data
//!   the file holds. A file that is shorter than its media data box was cut.
//! * `moov` → `trak` → `mdia` → `mdhd`: the **media header** states the
//!   duration of a track in units of the media timescale.
//! * `moov` → `trak` → `mdia` → `hdlr`: the **handler reference** names what
//!   the track holds (`soun` for a sound track).
//! * `moov` → `trak` → `mdia` → `minf` → `stbl` → `stsz`: the **sample size**
//!   box states the number of samples of a track and the size of each.
//! * `moov` → `trak` → `mdia` → `minf` → `stbl` → `stsc`: the **sample to
//!   chunk** box states how many samples each chunk holds.
//! * `moov` → `trak` → `mdia` → `minf` → `stbl` → `stco` (or `co64`): the
//!   **chunk offset** box states where the chunks of samples start.
//!
//! The sample tables together say exactly where the audio of a track is and
//! how many bytes it occupies: the first chunk offset is where the audio
//! starts, and the last chunk offset plus the sizes of the samples in that
//! chunk is where it ends. A file that is shorter than that was cut.
//!
//! The layout is the public ISO base media file format layout (the box tree,
//! the media header and the sample tables). No Winamp source, SDK header or
//! disassembly was consulted and nothing was copied.

use std::io::SeekFrom;

use rustamp_core::audio::ReadSeek;

/// What an MP4 file states about the payload of one of its sound tracks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Mp4Track {
    /// The number of frames the media header declares.
    frames: u64,
    /// The media timescale the frame count is measured in.
    timescale: u32,
    /// The offset of the first byte of the track's sample data.
    data_start: u64,
    /// The offset behind the last byte of the track's sample data.
    data_end: u64,
}

impl Mp4Track {
    /// The number of frames the media header declares.
    pub(crate) fn frames(&self) -> u64 {
        self.frames
    }

    /// The media timescale the frame count is measured in.
    pub(crate) fn timescale(&self) -> u32 {
        self.timescale
    }

    /// The offset of the first byte of the track's sample data.
    pub(crate) fn data_start(&self) -> u64 {
        self.data_start
    }

    /// The number of bytes the track's sample data occupies.
    pub(crate) fn data_bytes(&self) -> u64 {
        self.data_end.saturating_sub(self.data_start)
    }
}

/// What an MP4 file states about its payload.
#[derive(Debug, Default)]
pub(crate) struct Mp4Header {
    /// The extent of the last media data box: where its payload starts and
    /// where it ends.
    ///
    /// A file may hold more than one media data box; the file has to hold all
    /// of them, so the last one is the claim.
    media_data: Option<(u64, u64)>,
    /// The sound tracks the file describes.
    tracks: Vec<Mp4Track>,
}

impl Mp4Header {
    /// The extent of the last media data box: the offset of its first payload
    /// byte and the offset behind its last.
    pub(crate) fn media_data(&self) -> Option<(u64, u64)> {
        self.media_data
    }

    /// The sound tracks the file describes.
    pub(crate) fn tracks(&self) -> &[Mp4Track] {
        &self.tracks
    }
}

/// Whether the bytes look like the start of an ISO base media file.
///
/// The check is cheap on purpose: the header parse reads the whole box tree
/// of the file, and that is only worth doing for a file that can be one.
pub(crate) fn looks_like_mp4(window: &[u8]) -> bool {
    if window.len() < 8 {
        return false;
    }
    let kind = &window[4..8];
    kind == b"ftyp" || kind == b"moov" || kind == b"mdat" || kind == b"styp"
}

/// A box of an ISO base media file: its type and the range of its payload.
#[derive(Debug, Clone, Copy)]
struct Mp4Box {
    /// The type of the box.
    kind: [u8; 4],
    /// The offset of the first payload byte.
    start: u64,
    /// The offset behind the last payload byte.
    end: u64,
}

impl Mp4Box {
    /// Whether the box is of the given type.
    fn is(&self, kind: &[u8; 4]) -> bool {
        self.kind == *kind
    }
}

/// Reads the box at `pos`.
///
/// `file_len` is the length of the file: a box that claims to reach beyond it
/// is still reported, because its claim is what tells a cut file from a
/// complete one.
fn read_box(reader: &mut dyn ReadSeek, pos: u64, file_len: u64) -> Option<Mp4Box> {
    reader.seek(SeekFrom::Start(pos)).ok()?;
    let mut head = [0u8; 8];
    reader.read_exact(&mut head).ok()?;
    let kind = [head[4], head[5], head[6], head[7]];
    let size32 = u32::from_be_bytes([head[0], head[1], head[2], head[3]]);
    let (header_len, size) = match size32 {
        // A size of zero: the box reaches to the end of the file.
        0 => (8u64, file_len.checked_sub(pos)?),
        // A size of one: the real size follows as a 64 bit number.
        1 => {
            let mut ext = [0u8; 8];
            reader.read_exact(&mut ext).ok()?;
            (16u64, u64::from_be_bytes(ext))
        }
        n => (8u64, u64::from(n)),
    };
    if size < header_len {
        return None;
    }
    let end = pos.checked_add(size)?;
    Some(Mp4Box {
        kind,
        start: pos + header_len,
        end,
    })
}

/// The boxes in the range `[from, to)` of the file.
///
/// A box that reaches beyond `to` ends the walk: the file is broken there.
fn boxes(reader: &mut dyn ReadSeek, from: u64, to: u64, file_len: u64) -> Vec<Mp4Box> {
    let mut out = Vec::new();
    let mut pos = from;
    while pos + 8 <= to {
        let Some(b) = read_box(reader, pos, file_len) else {
            break;
        };
        if b.end > to || b.end <= pos {
            break;
        }
        out.push(b);
        pos = b.end;
    }
    out
}

/// The boxes at the top level of the file.
///
/// A box that reaches beyond the end of the file is reported (its claim is
/// what tells a cut file from a complete one) and ends the walk.
fn top_level_boxes(reader: &mut dyn ReadSeek, from: u64, file_len: u64) -> Vec<Mp4Box> {
    let mut out = Vec::new();
    let mut pos = from;
    while pos + 8 <= file_len {
        let Some(b) = read_box(reader, pos, file_len) else {
            break;
        };
        let complete = b.end <= file_len;
        out.push(b);
        if !complete || b.end <= pos {
            break;
        }
        pos = b.end;
    }
    out
}

/// Reads `len` bytes at `pos`.
fn read_at(reader: &mut dyn ReadSeek, pos: u64, len: usize) -> Option<Vec<u8>> {
    reader.seek(SeekFrom::Start(pos)).ok()?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).ok()?;
    Some(buf)
}

/// Reads the header of the sound tracks an MP4 file describes.
///
/// `None` when the file is not an MP4 file or when its length is unknown: the
/// byte claim needs the length of the file.
pub(crate) fn parse(reader: &mut dyn ReadSeek, len: Option<u64>) -> Option<Mp4Header> {
    let file_len = len?;
    // A file may start with an ID3v2 tag (some taggers write one into an MP4
    // file); the boxes start behind it.
    reader.seek(SeekFrom::Start(0)).ok()?;
    let mut head = [0u8; 10];
    let from = if reader.read_exact(&mut head).is_ok() {
        crate::mpeg_header::id3v2_len(&head) as u64
    } else {
        0
    };
    let mut header = Mp4Header::default();
    for b in top_level_boxes(reader, from, file_len) {
        if b.is(b"mdat") {
            // A file may hold more than one media data box; the file has to
            // hold all of them, so the last one is the claim.
            if header.media_data.is_none_or(|(_, end)| b.end > end) {
                header.media_data = Some((b.start, b.end));
            }
        } else if b.is(b"moov") {
            header.tracks = moov_tracks(reader, &b, file_len);
        }
    }
    Some(header)
}

/// The sound tracks the movie box describes.
fn moov_tracks(reader: &mut dyn ReadSeek, moov: &Mp4Box, file_len: u64) -> Vec<Mp4Track> {
    let mut tracks = Vec::new();
    for b in boxes(reader, moov.start, moov.end, file_len) {
        if b.is(b"trak")
            && let Some(track) = trak(reader, &b, file_len)
        {
            tracks.push(track);
        }
    }
    tracks
}

/// What one track of an MP4 file states about its payload.
fn trak(reader: &mut dyn ReadSeek, trak: &Mp4Box, file_len: u64) -> Option<Mp4Track> {
    let mdia = boxes(reader, trak.start, trak.end, file_len)
        .into_iter()
        .find(|b| b.is(b"mdia"))?;
    let mdia_boxes = boxes(reader, mdia.start, mdia.end, file_len);

    // The handler reference names what the track holds.
    let hdlr = mdia_boxes.iter().find(|b| b.is(b"hdlr"))?;
    if read_at(reader, hdlr.start + 8, 4)? != *b"soun" {
        return None;
    }

    // The media header states the duration of the track.
    let mdhd = mdia_boxes.iter().find(|b| b.is(b"mdhd"))?;
    let (timescale, frames) = media_header(reader, mdhd)?;

    // The sample tables state where the samples are and how big they are.
    let minf = mdia_boxes.iter().find(|b| b.is(b"minf"))?;
    let stbl = boxes(reader, minf.start, minf.end, file_len)
        .into_iter()
        .find(|b| b.is(b"stbl"))?;
    let stbl_boxes = boxes(reader, stbl.start, stbl.end, file_len);
    let sizes = sample_sizes(reader, stbl_boxes.iter().find(|b| b.is(b"stsz"))?)?;
    let mapping = sample_to_chunk(reader, stbl_boxes.iter().find(|b| b.is(b"stsc"))?)?;
    let chunks = chunk_offsets(
        reader,
        stbl_boxes.iter().find(|b| b.is(b"stco")),
        stbl_boxes.iter().find(|b| b.is(b"co64")),
    )?;
    let (data_start, data_end) = sample_extent(&sizes, &mapping, &chunks)?;

    Some(Mp4Track {
        frames,
        timescale,
        data_start,
        data_end,
    })
}

/// The timescale and the duration of a track, as the media header states them.
fn media_header(reader: &mut dyn ReadSeek, b: &Mp4Box) -> Option<(u32, u64)> {
    let version = read_at(reader, b.start, 1)?[0];
    let (timescale, duration) = match version {
        0 => {
            let raw = read_at(reader, b.start + 4, 16)?;
            (
                u32::from_be_bytes(raw[8..12].try_into().ok()?),
                u64::from(u32::from_be_bytes(raw[12..16].try_into().ok()?)),
            )
        }
        1 => {
            let raw = read_at(reader, b.start + 4, 28)?;
            (
                u32::from_be_bytes(raw[16..20].try_into().ok()?),
                u64::from_be_bytes(raw[20..28].try_into().ok()?),
            )
        }
        _ => return None,
    };
    // A duration of zero means "no duration", and the all ones value of the
    // 32 bit form means "unknown". Neither is a claim.
    if timescale == 0 || duration == 0 || duration == u64::MAX || duration == u32::MAX as u64 {
        return None;
    }
    Some((timescale, duration))
}

/// The sizes of the samples of a track.
struct SampleSizes {
    /// The size of every sample, when they are all the same.
    uniform: Option<u32>,
    /// The size of every sample, when they differ.
    sizes: Vec<u32>,
    /// The number of samples.
    count: u64,
}

impl SampleSizes {
    /// The size of the sample at `index`.
    fn get(&self, index: u64) -> Option<u32> {
        match self.uniform {
            Some(size) => Some(size),
            None => self.sizes.get(index as usize).copied(),
        }
    }
}

/// Reads the sample size box: the number of samples and the size of each.
fn sample_sizes(reader: &mut dyn ReadSeek, b: &Mp4Box) -> Option<SampleSizes> {
    let head = read_at(reader, b.start, 12)?;
    let uniform = u32::from_be_bytes(head[4..8].try_into().ok()?);
    let count = u32::from_be_bytes(head[8..12].try_into().ok()?);
    if uniform != 0 {
        return Some(SampleSizes {
            uniform: Some(uniform),
            sizes: Vec::new(),
            count: u64::from(count),
        });
    }
    // The sizes follow the header, one per sample. The box has to hold them
    // all: a box that claims more samples than it can hold is broken.
    let room = b.end.saturating_sub(b.start + 12) / 4;
    if u64::from(count) > room {
        return None;
    }
    let raw = read_at(reader, b.start + 12, count as usize * 4)?;
    let sizes = raw
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect();
    Some(SampleSizes {
        uniform: None,
        sizes,
        count: u64::from(count),
    })
}

/// One entry of the sample to chunk table.
#[derive(Debug, Clone, Copy)]
struct StscEntry {
    /// The first chunk the entry describes.
    first_chunk: u64,
    /// The number of samples each of those chunks holds.
    samples_per_chunk: u64,
}

/// Reads the sample to chunk table: how many samples each chunk holds.
fn sample_to_chunk(reader: &mut dyn ReadSeek, b: &Mp4Box) -> Option<Vec<StscEntry>> {
    let head = read_at(reader, b.start, 8)?;
    let count = u32::from_be_bytes(head[4..8].try_into().ok()?);
    let room = b.end.saturating_sub(b.start + 8) / 12;
    if u64::from(count) > room {
        return None;
    }
    let raw = read_at(reader, b.start + 8, count as usize * 12)?;
    let mut out = Vec::with_capacity(count as usize);
    for e in raw.as_chunks::<12>().0 {
        out.push(StscEntry {
            first_chunk: u64::from(u32::from_be_bytes([e[0], e[1], e[2], e[3]])),
            samples_per_chunk: u64::from(u32::from_be_bytes([e[4], e[5], e[6], e[7]])),
        });
    }
    Some(out)
}

/// Reads the chunk offset table: where the chunks of samples start.
fn chunk_offsets(
    reader: &mut dyn ReadSeek,
    stco: Option<&Mp4Box>,
    co64: Option<&Mp4Box>,
) -> Option<Vec<u64>> {
    let (b, wide) = match (stco, co64) {
        (Some(b), _) => (b, false),
        (None, Some(b)) => (b, true),
        // A track without chunk offsets states nothing about where its
        // samples are.
        (None, None) => return None,
    };
    let head = read_at(reader, b.start, 8)?;
    let count = u32::from_be_bytes(head[4..8].try_into().ok()?);
    let width = if wide { 8usize } else { 4usize };
    let room = b.end.saturating_sub(b.start + 8) / width as u64;
    if u64::from(count) > room {
        return None;
    }
    let raw = read_at(reader, b.start + 8, count as usize * width)?;
    let mut out = Vec::with_capacity(count as usize);
    for e in raw.chunks_exact(width) {
        let offset = if wide {
            u64::from_be_bytes([e[0], e[1], e[2], e[3], e[4], e[5], e[6], e[7]])
        } else {
            u64::from(u32::from_be_bytes([e[0], e[1], e[2], e[3]]))
        };
        out.push(offset);
    }
    Some(out)
}

/// The number of samples the chunk `chunk` holds.
fn samples_per_chunk(mapping: &[StscEntry], chunk: u64) -> Option<u64> {
    let mut found = None;
    for e in mapping {
        if e.first_chunk > chunk {
            break;
        }
        found = Some(e.samples_per_chunk);
    }
    found
}

/// The byte extent of the samples of a track: where they start and where they
/// end.
///
/// The samples of a chunk are contiguous in the file, so the chunk holds the
/// sum of the sizes of its samples.
fn sample_extent(sizes: &SampleSizes, mapping: &[StscEntry], chunks: &[u64]) -> Option<(u64, u64)> {
    let total = sizes.count;
    if total == 0 || chunks.is_empty() || mapping.is_empty() {
        return None;
    }
    let mut index = 0u64;
    let mut start = None;
    for (n, offset) in chunks.iter().enumerate() {
        let per_chunk = samples_per_chunk(mapping, n as u64 + 1)?;
        if per_chunk == 0 {
            return None;
        }
        // The last chunk may hold fewer samples than the table says.
        let in_chunk = per_chunk.min(total - index);
        let mut length = 0u64;
        for i in 0..in_chunk {
            length += u64::from(sizes.get(index + i)?);
        }
        start.get_or_insert(*offset);
        index += in_chunk;
        if index == total {
            return Some((start?, *offset + length));
        }
    }
    // The chunk table does not cover all the samples: the file states
    // nothing about where the rest is.
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// The repository fixture: an ALAC file with one sound track.
    fn fixture() -> Vec<u8> {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sine_alac.m4a");
        std::fs::read(path).expect("the ALAC fixture")
    }

    /// The fixture states what its boxes say: one sound track of 66150 frames
    /// at 44100 Hz, its samples in one chunk behind the media data box.
    #[test]
    fn the_media_header_and_the_sample_tables_are_read() {
        let bytes = fixture();
        let len = bytes.len() as u64;
        let header = parse(&mut Cursor::new(bytes), Some(len)).expect("an MP4 header");
        let (start, end) = header.media_data().expect("the media data box");
        assert_eq!(start, 44, "the media data starts behind the file type box");
        assert_eq!(end, 25642, "the media data box states its own length");

        let tracks = header.tracks();
        assert_eq!(tracks.len(), 1, "the fixture carries one sound track");
        let track = &tracks[0];
        assert_eq!(track.frames(), 66150, "the media header states the frames");
        assert_eq!(track.timescale(), 44100, "the media timescale");
        assert_eq!(track.data_start(), 44, "the first sample");
        assert_eq!(track.data_bytes(), 25598, "the samples fill the media data");
    }

    /// A file without the sample tables states nothing about where its
    /// samples are: it is no oracle.
    #[test]
    fn a_file_without_the_sample_size_box_states_nothing() {
        let mut bytes = fixture();
        let at = bytes
            .windows(4)
            .position(|w| w == b"stsz")
            .expect("the sample size box");
        bytes[at..at + 4].copy_from_slice(b"stzX");
        let len = bytes.len() as u64;
        let header = parse(&mut Cursor::new(bytes), Some(len)).expect("an MP4 header");
        assert!(
            header.tracks().is_empty(),
            "a track without sample sizes is not a track this crate can use"
        );
    }

    /// A file without the chunk offsets states nothing about where its
    /// samples are.
    #[test]
    fn a_file_without_the_chunk_offsets_states_nothing() {
        let mut bytes = fixture();
        let at = bytes
            .windows(4)
            .position(|w| w == b"stco")
            .expect("the chunk offset box");
        bytes[at..at + 4].copy_from_slice(b"stcX");
        let len = bytes.len() as u64;
        let header = parse(&mut Cursor::new(bytes), Some(len)).expect("an MP4 header");
        assert!(header.tracks().is_empty());
    }

    /// A file whose media data box reaches beyond the end of the file was
    /// cut: the box states the claim that says so.
    #[test]
    fn a_media_data_box_beyond_the_end_of_the_file_is_still_reported() {
        let bytes = fixture();
        let cut = &bytes[..bytes.len() - 1000];
        let header =
            parse(&mut Cursor::new(cut.to_vec()), Some(cut.len() as u64)).expect("an MP4 header");
        let (start, end) = header.media_data().expect("the media data box");
        assert_eq!(start, 44);
        assert!(
            end > cut.len() as u64,
            "the media data box claims {} bytes, the file holds {}",
            end - start,
            cut.len() as u64 - start
        );
    }

    /// A file that starts with an ID3v2 tag: the boxes start behind it.
    ///
    /// Some taggers write an ID3v2 tag in front of an MP4 file. The tag is
    /// not a box, so the walk has to start behind it.
    #[test]
    fn an_id3v2_tag_in_front_of_the_boxes_is_skipped() {
        let bytes = fixture();
        // An ID3v2 header of ten bytes with an empty tag.
        let mut tagged = b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec();
        tagged.extend_from_slice(&bytes);
        let len = tagged.len() as u64;
        let header = parse(&mut Cursor::new(tagged), Some(len)).expect("an MP4 header");
        let (start, end) = header.media_data().expect("the media data box");
        assert_eq!(start, 44 + 10, "the media data starts behind the tag");
        assert_eq!(end, 25642 + 10, "the media data box states its own length");
        assert_eq!(header.tracks().len(), 1, "the sound track is still found");
    }

    /// Bytes that are not an MP4 file are not an MP4 file.
    #[test]
    fn other_files_are_not_parsed_as_mp4() {
        assert!(!looks_like_mp4(b"RIFF....WAVE"));
        assert!(!looks_like_mp4(b"fLaC"));
        assert!(!looks_like_mp4(b""));
        assert!(looks_like_mp4(b"\0\0\0\x18ftypM4A "));
    }
}
