//! Property tests for the tag writer and the tag reader.
//!
//! The cases are generated from fixed seeds, so every failure can be
//! replayed: the seed and the case number are part of the message.
//!
//! # The formats
//!
//! The suite runs over every format the writer supports:
//!
//! | File | Tag the writer writes | Numbers it can carry | `extra` keys it carries |
//! |---|---|---|---|
//! | MP3 | ID3v2 | any `u32` | `REPLAYGAIN_TRACK_GAIN`, `R128_TRACK_GAIN` |
//! | AAC (ADTS) | ID3v2 | any `u32` | `REPLAYGAIN_TRACK_GAIN`, `R128_TRACK_GAIN` |
//! | WAV | ID3v2 | any `u32` | `REPLAYGAIN_TRACK_GAIN`, `R128_TRACK_GAIN` |
//! | AIFF | ID3v2 | any `u32` | `REPLAYGAIN_TRACK_GAIN`, `R128_TRACK_GAIN` |
//! | FLAC | Vorbis comments | any `u32` | `REPLAYGAIN_TRACK_GAIN`, `R128_TRACK_GAIN`, `DISCTOTAL` |
//! | Ogg Vorbis | Vorbis comments | any `u32` | `REPLAYGAIN_TRACK_GAIN`, `R128_TRACK_GAIN`, `DISCTOTAL` |
//! | Opus | Vorbis comments | any `u32` | `REPLAYGAIN_TRACK_GAIN`, `R128_TRACK_GAIN`, `DISCTOTAL` |
//! | MP4 (AAC) | `ilst` | 1..=65535 | `REPLAYGAIN_TRACK_GAIN` |
//! | MP4 (ALAC) | `ilst` | 1..=65535 | `REPLAYGAIN_TRACK_GAIN` |
//!
//! # What every format does the same way
//!
//! * A field that is `None` in the metadata keeps the value the file has.
//! * The reader trims the value it reads and drops it when nothing is left:
//!   an empty string and a string of blanks are both read back as "no
//!   value".
//! * The audio of the file is never rewritten.
//!
//! # What the formats do differently
//!
//! * The MP4 `trkn` and `disk` atoms hold a pair of 16-bit numbers, and a
//!   zero is "no number at all". The MP4 family therefore carries neither a
//!   zero nor a number above 65535; the other formats carry every `u32`.
//! * The Vorbis family does not store an empty value at all: the key is
//!   gone. ID3v2 and MP4 store it as an empty item. The reader cannot tell
//!   the two apart, because it drops an empty value either way.
//! * The Vorbis family stores the disc total under a key of its own, so it
//!   can carry `DISCTOTAL` in `extra`; ID3v2 and MP4 have no key for it.
//! * ID3v2 keeps the track number and the track total in one frame
//!   (`TRCK`). A total without a number is written as the pair `0/total`,
//!   so the file gains a track number of zero. The Vorbis family keeps the
//!   two in comments of their own, and MP4 in one atom.

use std::collections::BTreeMap;
use std::io::Seek;
use std::path::{Path, PathBuf};

use rustamp_core::meta::TrackMetadata;
use rustamp_meta::{TagField, read_tags, write_tags, write_tags_report, write_tags_with_report};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// The generator
// ---------------------------------------------------------------------------

/// A seeded pseudo-random generator (SplitMix64).
///
/// The suite needs no dependency and no randomness at run time: every case
/// comes from a fixed seed, so a failure can be replayed.
struct Rng(u64);

impl Rng {
    /// A generator for the seed.
    fn new(seed: u64) -> Self {
        Rng(seed)
    }

    /// The next number.
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..bound`.
    fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }

    /// A number in `low..=high`.
    fn between(&mut self, low: u64, high: u64) -> u64 {
        low + self.below(high - low + 1)
    }

    /// Whether the next case happens (in percent).
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    /// One of the items.
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// The seeds every property runs with.
///
/// A failure names the seed, so it can be replayed.
const SEEDS: [u64; 3] = [0x5EED_0001, 0x5EED_0002, 0x5EED_0003];

/// The cases per seed and format.
const CASES: usize = 10;

// ---------------------------------------------------------------------------
// The formats
// ---------------------------------------------------------------------------

/// The tag format the writer writes for a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TagKind {
    /// ID3v2: MP3, AAC, WAV, AIFF.
    Id3v2,
    /// Vorbis comments: FLAC, Ogg Vorbis, Opus.
    Vorbis,
    /// The MP4 `ilst` atoms: M4A, ALAC.
    Ilst,
}

/// Where the audio of a file sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// The ID3v2 tag sits in front of the audio (MP3, AAC).
    Id3v2Front,
    /// The metadata blocks sit in front of the audio (FLAC).
    FlacFront,
    /// The audio is in the pages of an Ogg stream.
    Ogg,
    /// The audio is in the `mdat` atom (MP4).
    Mp4,
    /// The audio is in the `data` chunk (WAV).
    Wav,
    /// The audio is in the `SSND` chunk (AIFF).
    Aiff,
}

/// A file the writer supports.
struct Format {
    /// The name in the failure messages.
    name: &'static str,
    /// The tag the writer writes for the file.
    kind: TagKind,
    /// Where the audio of the file sits.
    layout: Layout,
    /// Makes the file the test works on.
    make: fn(&Path),
}

/// The formats the suite runs over.
const FORMATS: &[Format] = &[
    Format {
        name: "mp3",
        kind: TagKind::Id3v2,
        layout: Layout::Id3v2Front,
        make: make_mp3,
    },
    Format {
        name: "aac",
        kind: TagKind::Id3v2,
        layout: Layout::Id3v2Front,
        make: make_aac,
    },
    Format {
        name: "wav",
        kind: TagKind::Id3v2,
        layout: Layout::Wav,
        make: make_wav,
    },
    Format {
        name: "aiff",
        kind: TagKind::Id3v2,
        layout: Layout::Aiff,
        make: make_aiff,
    },
    Format {
        name: "flac",
        kind: TagKind::Vorbis,
        layout: Layout::FlacFront,
        make: make_flac,
    },
    Format {
        name: "ogg",
        kind: TagKind::Vorbis,
        layout: Layout::Ogg,
        make: make_ogg,
    },
    Format {
        name: "opus",
        kind: TagKind::Vorbis,
        layout: Layout::Ogg,
        make: make_opus,
    },
    Format {
        name: "m4a",
        kind: TagKind::Ilst,
        layout: Layout::Mp4,
        make: make_m4a,
    },
    Format {
        name: "alac",
        kind: TagKind::Ilst,
        layout: Layout::Mp4,
        make: make_alac,
    },
];

// ---------------------------------------------------------------------------
// The files
// ---------------------------------------------------------------------------

/// The value of the item the format has but this crate has no field for.
///
/// The item is in every file the suite makes, so every write has to keep it.
const UNKNOWN_MARKER: &str = "RUSTAMP-UNKNOWN-MARKER";

/// A fixture of the decoder crate.
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rustamp-decoders/tests/fixtures")
        .join(name)
}

/// The bytes of a file.
fn bytes_of(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("read the file")
}

/// The size of the ID3v2 tag at the front of `bytes`, header included.
fn id3v2_size(bytes: &[u8]) -> usize {
    if bytes.len() < 10 || &bytes[..3] != b"ID3" {
        return 0;
    }
    let size = (usize::from(bytes[6] & 0x7F) << 21)
        | (usize::from(bytes[7] & 0x7F) << 14)
        | (usize::from(bytes[8] & 0x7F) << 7)
        | usize::from(bytes[9] & 0x7F);
    10 + size
}

/// The size of a synchsafe number.
fn synchsafe(size: u32) -> [u8; 4] {
    [
        ((size >> 21) & 0x7F) as u8,
        ((size >> 14) & 0x7F) as u8,
        ((size >> 7) & 0x7F) as u8,
        (size & 0x7F) as u8,
    ]
}

/// An ID3v2.4 frame.
fn id3v2_frame(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.extend_from_slice(id);
    frame.extend_from_slice(&synchsafe(data.len() as u32));
    frame.extend_from_slice(&[0, 0]); // the flags
    frame.extend_from_slice(data);
    frame
}

/// An ID3v2.4 text frame.
fn id3v2_text_frame(id: &[u8; 4], text: &str) -> Vec<u8> {
    let mut data = vec![0x03]; // UTF-8
    data.extend_from_slice(text.as_bytes());
    id3v2_frame(id, &data)
}

/// An ID3v2.4 `TXXX` frame: a user text frame, told apart by its
/// description.
fn id3v2_txxx_frame(description: &str, text: &str) -> Vec<u8> {
    let mut data = vec![0x03]; // UTF-8
    data.extend_from_slice(description.as_bytes());
    data.push(0x00);
    data.extend_from_slice(text.as_bytes());
    id3v2_frame(b"TXXX", &data)
}

/// The ID3v2 tag the ID3v2 formats of the suite start with.
///
/// It holds a title, an artist and a `TXXX` frame this crate has no field
/// for: that frame is the "unknown item" of the ID3v2 formats.
fn id3v2_tag() -> Vec<u8> {
    let frames = [
        id3v2_text_frame(b"TIT2", "Sine Title"),
        id3v2_text_frame(b"TPE1", "Test Artist"),
        id3v2_txxx_frame("CUSTOM_TXXX", UNKNOWN_MARKER),
    ];
    let body: Vec<u8> = frames.concat();

    let mut tag = Vec::new();
    tag.extend_from_slice(b"ID3");
    tag.extend_from_slice(&[0x04, 0x00, 0x00]); // v2.4, no flags
    tag.extend_from_slice(&synchsafe(body.len() as u32));
    tag.extend_from_slice(&body);
    tag
}

/// The audio frames of the MP3 fixture: everything behind its ID3v2 tag.
fn mp3_audio() -> Vec<u8> {
    let bytes = bytes_of(&fixture("sine.mp3"));
    bytes[id3v2_size(&bytes)..].to_vec()
}

/// A RIFF chunk: the id, the size and the data.
fn riff_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// A chunk of an AIFF file: the same as a RIFF chunk, but big-endian.
fn aiff_chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(id);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// A minimal AAC (ADTS) file: a frame header and a little sound data.
fn aac_frames() -> Vec<u8> {
    const PAYLOAD: usize = 32;
    let frame_length = 7 + PAYLOAD;

    let header = [
        0xFFu8,
        0xF1, // MPEG-4, layer 0, no CRC
        0x50, // AAC LC, 44.1 kHz, mono
        0x40, // mono, no copyright, short frame
        ((frame_length >> 3) & 0xFF) as u8,
        (((frame_length & 7) << 5) | 0x1F) as u8,
        0xFC, // the buffer is full, one raw data block
    ];
    let mut frame = header.to_vec();
    frame.extend_from_slice(&[0u8; PAYLOAD]);
    frame
}

/// A minimal AIFF file: a `FORM` container with a `COMM` and an `SSND`
/// chunk.
fn aiff_comm() -> Vec<u8> {
    let mut comm = Vec::new();
    comm.extend_from_slice(&1u16.to_be_bytes()); // the channels
    comm.extend_from_slice(&44100u32.to_be_bytes()); // the sample frames
    comm.extend_from_slice(&16u16.to_be_bytes()); // the sample size
    // 44100 Hz as an 80-bit IEEE extended float.
    comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
    comm
}

/// A minimal AIFF sound chunk.
fn aiff_ssnd() -> Vec<u8> {
    let mut ssnd = Vec::new();
    ssnd.extend_from_slice(&0u32.to_be_bytes()); // the offset
    ssnd.extend_from_slice(&0u32.to_be_bytes()); // the block size
    ssnd.extend_from_slice(&[0u8; 64]); // the sound data
    ssnd
}

/// The `fmt ` chunk of the WAV file of the suite.
fn wav_fmt() -> Vec<u8> {
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes()); // PCM
    fmt.extend_from_slice(&1u16.to_le_bytes()); // mono
    fmt.extend_from_slice(&44100u32.to_le_bytes());
    fmt.extend_from_slice(&44100u32.to_le_bytes()); // the byte rate
    fmt.extend_from_slice(&1u16.to_le_bytes()); // the block align
    fmt.extend_from_slice(&8u16.to_le_bytes()); // the bits per sample
    fmt
}

/// A FLAC metadata block: a type byte, a 24-bit size and the content.
fn flac_block(ty: u8, last: bool, content: &[u8]) -> Vec<u8> {
    let mut block = Vec::new();
    block.push(if last { ty | 0x80 } else { ty });
    let size = (content.len() as u32).to_be_bytes();
    block.extend_from_slice(&size[1..]);
    block.extend_from_slice(content);
    block
}

/// The content of a `VORBIS_COMMENT` block.
fn flac_vorbis_comments(comments: &[(&str, &str)]) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend_from_slice(&0u32.to_le_bytes()); // the vendor length
    content.extend_from_slice(&(comments.len() as u32).to_le_bytes());
    for (key, value) in comments {
        let comment = format!("{key}={value}");
        content.extend_from_slice(&(comment.len() as u32).to_le_bytes());
        content.extend_from_slice(comment.as_bytes());
    }
    content
}

/// The offset where the audio frames of a FLAC file start.
fn flac_audio_start(bytes: &[u8]) -> usize {
    let mut offset = 4;
    loop {
        let header = bytes[offset];
        let size = u32::from_be_bytes([0, bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
            as usize;
        offset += 4 + size;
        if header & 0x80 != 0 {
            return offset;
        }
    }
}

/// The `STREAMINFO` block and the audio frames of the FLAC fixture.
fn flac_parts() -> (Vec<u8>, Vec<u8>) {
    let bytes = bytes_of(&fixture("sine.flac"));
    let size = u32::from_be_bytes([0, bytes[5], bytes[6], bytes[7]]) as usize;
    let streaminfo = bytes[4..4 + 4 + size].to_vec();
    let audio = bytes[flac_audio_start(&bytes)..].to_vec();
    (streaminfo, audio)
}

/// An MP3 file with the ID3v2 tag of the suite.
fn make_mp3(path: &Path) {
    let mut file = id3v2_tag();
    file.extend_from_slice(&mp3_audio());
    std::fs::write(path, file).expect("write the MP3 file");
}

/// An AAC file with the ID3v2 tag of the suite.
fn make_aac(path: &Path) {
    let mut file = id3v2_tag();
    file.extend_from_slice(&aac_frames());
    std::fs::write(path, file).expect("write the AAC file");
}

/// A WAV file with the ID3v2 chunk of the suite.
fn make_wav(path: &Path) {
    let mut body = b"WAVE".to_vec();
    body.extend_from_slice(&riff_chunk(b"fmt ", &wav_fmt()));
    body.extend_from_slice(&riff_chunk(b"data", &[0u8; 16]));
    body.extend_from_slice(&riff_chunk(b"ID3 ", &id3v2_tag()));

    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&(body.len() as u32).to_le_bytes());
    file.extend_from_slice(&body);
    std::fs::write(path, file).expect("write the WAV file");
}

/// An AIFF file with the ID3v2 chunk of the suite.
fn make_aiff(path: &Path) {
    let mut body = b"AIFF".to_vec();
    body.extend_from_slice(&aiff_chunk(b"COMM", &aiff_comm()));
    body.extend_from_slice(&aiff_chunk(b"SSND", &aiff_ssnd()));
    body.extend_from_slice(&aiff_chunk(b"ID3 ", &id3v2_tag()));

    let mut file = b"FORM".to_vec();
    file.extend_from_slice(&(body.len() as u32).to_be_bytes());
    file.extend_from_slice(&body);
    std::fs::write(path, file).expect("write the AIFF file");
}

/// A FLAC file with the Vorbis comments of the suite.
fn make_flac(path: &Path) {
    let (streaminfo, audio) = flac_parts();

    let mut file = b"fLaC".to_vec();
    file.extend_from_slice(&streaminfo);
    file.extend_from_slice(&flac_block(
        4,
        false,
        &flac_vorbis_comments(&[
            ("TITLE", "Sine Title"),
            ("ARTIST", "Test Artist"),
            ("CUSTOM_KEY", UNKNOWN_MARKER),
        ]),
    ));
    file.extend_from_slice(&flac_block(1, true, &[0u8; 1024])); // the padding block
    file.extend_from_slice(&audio);
    std::fs::write(path, file).expect("write the FLAC file");
}

/// An Ogg Vorbis file with the Vorbis comments of the suite.
fn make_ogg(path: &Path) {
    std::fs::copy(fixture("sine.ogg"), path).expect("copy the Ogg fixture");
    plant_vorbis_comment(path, false, "CUSTOM_KEY", UNKNOWN_MARKER);
}

/// An Opus file with the Vorbis comments of the suite.
fn make_opus(path: &Path) {
    std::fs::copy(fixture("sine.opus"), path).expect("copy the Opus fixture");
    plant_vorbis_comment(path, true, "CUSTOM_KEY", UNKNOWN_MARKER);
}

/// An MP4 file with the freeform atom of the suite.
fn make_m4a(path: &Path) {
    std::fs::copy(fixture("sine_aac.m4a"), path).expect("copy the MP4 fixture");
    plant_mp4_freeform(path, "CUSTOMKEY", UNKNOWN_MARKER);
}

/// An MP4 file with the freeform atom of the suite.
fn make_alac(path: &Path) {
    std::fs::copy(fixture("sine_alac.m4a"), path).expect("copy the MP4 fixture");
    plant_mp4_freeform(path, "CUSTOMKEY", UNKNOWN_MARKER);
}

/// Adds a comment this crate has no field for to the Ogg file at `path`.
///
/// The Opus file has its own reader, so the caller says which of the two it
/// is: the file of the suite has no extension to guess from.
fn plant_vorbis_comment(path: &Path, opus: bool, key: &str, value: &str) {
    use lofty::config::{ParseOptions, WriteOptions};
    use lofty::file::AudioFile;
    use lofty::ogg::{OpusFile, VorbisFile};
    use lofty::tag::TagExt;

    let options = ParseOptions::new().read_properties(false);
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open the Ogg file");

    let mut comments = if opus {
        OpusFile::read_from(&mut file, options)
            .expect("read the Opus file")
            .vorbis_comments()
            .clone()
    } else {
        VorbisFile::read_from(&mut file, options)
            .expect("read the Ogg file")
            .vorbis_comments()
            .clone()
    };
    comments.insert(key.to_owned(), value.to_owned());

    file.rewind().expect("rewind");
    comments
        .save_to(&mut file, WriteOptions::default())
        .expect("write the comment");
}

/// Writes a freeform MP4 atom — one this crate has no field for — into the
/// `ilst` of the file at `path`.
fn plant_mp4_freeform(path: &Path, name: &'static str, value: &str) {
    use lofty::config::{ParseOptions, WriteOptions};
    use lofty::file::AudioFile;
    use lofty::mp4::{Atom, AtomData, AtomIdent, Mp4File};
    use lofty::tag::TagExt;

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .expect("open the MP4 file");
    let mp4 = Mp4File::read_from(&mut file, ParseOptions::new().read_properties(false))
        .expect("read the MP4 file");
    let mut ilst = mp4.ilst().cloned().unwrap_or_default();
    ilst.insert(Atom::new(
        AtomIdent::Freeform {
            mean: "com.apple.iTunes".into(),
            name: name.into(),
        },
        AtomData::UTF8(value.into()),
    ));
    file.rewind().expect("rewind");
    ilst.save_to(&mut file, WriteOptions::default())
        .expect("write the atom");
}

// ---------------------------------------------------------------------------
// The audio of a file
// ---------------------------------------------------------------------------

/// The packets of an Ogg stream, in order.
fn ogg_packets(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut packets = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut pos = 0usize;
    while pos + 27 <= bytes.len() {
        assert_eq!(&bytes[pos..pos + 4], b"OggS", "the Ogg page header");
        let segments = usize::from(bytes[pos + 26]);
        let table = &bytes[pos + 27..pos + 27 + segments];
        let mut offset = pos + 27 + segments;
        for &segment in table {
            let length = usize::from(segment);
            current.extend_from_slice(&bytes[offset..offset + length]);
            offset += length;
            if length < 255 {
                packets.push(std::mem::take(&mut current));
            }
        }
        pos = offset;
    }
    packets
}

/// The audio packets of an Ogg stream: the packets that are not one of the
/// header packets.
///
/// The Ogg writer rewrites the page headers of the audio pages (the sequence
/// numbers and their checksums), so the audio is compared packet by packet
/// and not byte by byte.
fn ogg_audio_packets(bytes: &[u8]) -> Vec<Vec<u8>> {
    ogg_packets(bytes)
        .into_iter()
        .filter(|packet| {
            !(packet.starts_with(b"\x03vorbis")
                || packet.starts_with(b"\x05vorbis")
                || packet.starts_with(b"OpusHead")
                || packet.starts_with(b"OpusTags"))
        })
        .collect()
}

/// The payload of the `mdat` atom of an MP4 file.
fn mp4_mdat(bytes: &[u8]) -> Option<Vec<u8>> {
    let mut pos = 0usize;
    while pos + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[pos..pos + 4].try_into().ok()?) as u64;
        let kind = &bytes[pos + 4..pos + 8];
        let (header, size) = if size == 1 {
            (
                16usize,
                u64::from_be_bytes(bytes[pos + 8..pos + 16].try_into().ok()?),
            )
        } else {
            (8usize, size)
        };
        if kind == b"mdat" {
            let start = pos + header;
            let end = (pos + size as usize).min(bytes.len());
            return Some(bytes[start..end].to_vec());
        }
        if size == 0 {
            return None;
        }
        pos += size as usize;
    }
    None
}

/// The data of a chunk of a RIFF file.
fn riff_chunk_data(bytes: &[u8], want: &[u8; 4]) -> Option<Vec<u8>> {
    let mut pos = 12usize; // "RIFF", the size and "WAVE"
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        if id == want {
            return Some(bytes[pos + 8..pos + 8 + size].to_vec());
        }
        pos += 8 + size + (size % 2);
    }
    None
}

/// The data of a chunk of an AIFF file.
fn aiff_chunk_data(bytes: &[u8], want: &[u8; 4]) -> Option<Vec<u8>> {
    let mut pos = 12usize; // "FORM", the size and "AIFF"
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_be_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        if id == want {
            return Some(bytes[pos + 8..pos + 8 + size].to_vec());
        }
        pos += 8 + size + (size % 2);
    }
    None
}

/// The audio of the file: the bytes a write must not touch.
///
/// The tag container sits in front of the audio in every format, so the audio
/// is what is behind it.
fn audio_of(format: &Format, bytes: &[u8]) -> Vec<u8> {
    match format.layout {
        Layout::Id3v2Front => bytes[id3v2_size(bytes)..].to_vec(),
        Layout::FlacFront => bytes[flac_audio_start(bytes)..].to_vec(),
        Layout::Ogg => ogg_audio_packets(bytes).concat(),
        Layout::Mp4 => mp4_mdat(bytes).unwrap_or_default(),
        Layout::Wav => riff_chunk_data(bytes, b"data").unwrap_or_default(),
        Layout::Aiff => aiff_chunk_data(bytes, b"SSND").unwrap_or_default(),
    }
}

/// Asserts that the write did not touch the audio of the file.
fn assert_audio_unchanged(format: &Format, path: &Path, before: &[u8]) {
    let after = bytes_of(path);
    let audio = audio_of(format, &after);
    assert!(
        !audio.is_empty(),
        "{}: the audio of the file was not found",
        format.name
    );
    assert_eq!(
        audio_of(format, before),
        audio,
        "{}: the audio of the file changed",
        format.name
    );
}

// ---------------------------------------------------------------------------
// The cases
// ---------------------------------------------------------------------------

/// A text value: absent, empty, only blanks, padded or plain.
fn random_text(rng: &mut Rng) -> Option<String> {
    match rng.below(100) {
        0..=24 => None,
        25..=34 => Some(String::new()),
        35..=44 => Some("   ".to_owned()),
        45..=54 => Some(format!("  {}  ", random_word(rng))),
        _ => Some(random_word(rng)),
    }
}

/// A random text: plain ASCII, or characters from outside it.
///
/// The text is never empty: a patch has to name a field.
fn random_word(rng: &mut Rng) -> String {
    const ASCII: &[&str] = &["title", "Song", "track 1", "a-b_c", "0123", "MiXeD"];
    const UNICODE: &[&str] = &["Ünïcødé", "日本語", "🎵", "Ω≈ç√∫", "Ævar Þór", "naïve"];

    let mut word = if rng.chance(40) {
        rng.pick(UNICODE).to_string()
    } else {
        rng.pick(ASCII).to_string()
    };
    if rng.chance(50) {
        word.push('-');
        word.push_str(&rng.next_u64().to_string());
    }
    word
}

/// A year: absent, an edge of the range the tags can hold, or a random one.
fn random_year(rng: &mut Rng) -> Option<u32> {
    match rng.below(100) {
        0..=29 => None,
        30..=39 => Some(0),
        40..=49 => Some(9999),
        50..=59 => Some(1),
        _ => Some(rng.between(1000, 9999) as u32),
    }
}

/// A number: absent, an edge, or a random one.
fn random_number(rng: &mut Rng) -> Option<u32> {
    match rng.below(100) {
        0..=29 => None,
        30..=39 => Some(0),
        40..=49 => Some(u32::MAX),
        50..=59 => Some(65535),
        60..=69 => Some(65536),
        _ => Some(rng.between(1, 1000) as u32),
    }
}

/// The `extra` keys the suite writes.
const EXTRA_KEYS: [&str; 4] = [
    "REPLAYGAIN_TRACK_GAIN",
    "R128_TRACK_GAIN",
    "DISCTOTAL",
    "CUSTOM_UNKNOWN_KEY",
];

/// The value of an `extra` key.
///
/// The values are the ones a tagger writes: the reader parses some of them,
/// so they cannot be arbitrary text.
fn extra_value(key: &str) -> &'static str {
    match key {
        "REPLAYGAIN_TRACK_GAIN" => "-3.50 dB",
        "R128_TRACK_GAIN" => "-768",
        "DISCTOTAL" => "12",
        _ => "custom value",
    }
}

/// Whether the format can carry the `extra` key.
fn carries_extra(kind: TagKind, key: &str) -> bool {
    match key {
        "REPLAYGAIN_TRACK_GAIN" => true,
        "R128_TRACK_GAIN" => matches!(kind, TagKind::Id3v2 | TagKind::Vorbis),
        "DISCTOTAL" => matches!(kind, TagKind::Vorbis),
        _ => false,
    }
}

/// A random metadata set.
fn random_metadata(rng: &mut Rng) -> TrackMetadata {
    let mut metadata = TrackMetadata {
        title: random_text(rng),
        artist: random_text(rng),
        album: random_text(rng),
        album_artist: random_text(rng),
        year: random_year(rng),
        genre: random_text(rng),
        comment: random_text(rng),
        track_no: random_number(rng),
        track_total: random_number(rng),
        disc_no: random_number(rng),
        ..TrackMetadata::default()
    };
    for key in EXTRA_KEYS {
        if rng.chance(50) {
            metadata
                .extra
                .insert(key.to_owned(), extra_value(key).to_owned());
        }
    }
    metadata
}

/// A metadata with every field filled.
///
/// Every format carries every field of it, so a removal of one field can be
/// seen against the others.
fn full_metadata(rng: &mut Rng) -> TrackMetadata {
    let mut metadata = TrackMetadata {
        title: Some(random_word(rng)),
        artist: Some(random_word(rng)),
        album: Some(random_word(rng)),
        album_artist: Some(random_word(rng)),
        year: Some(random_patch_year(rng)),
        genre: Some(random_word(rng)),
        comment: Some(random_word(rng)),
        track_no: Some(rng.between(1, 1000) as u32),
        track_total: Some(rng.between(1, 1000) as u32),
        disc_no: Some(rng.between(1, 1000) as u32),
        ..TrackMetadata::default()
    };
    for key in EXTRA_KEYS {
        metadata
            .extra
            .insert(key.to_owned(), extra_value(key).to_owned());
    }
    metadata
}

// ---------------------------------------------------------------------------
// The expectations
// ---------------------------------------------------------------------------

/// What the reader makes of a text value.
///
/// The reader trims the value and drops it when nothing is left, so an empty
/// string and a string of blanks are both read back as "no value".
fn expected_text(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// Whether the format can carry the number.
///
/// The MP4 `trkn` and `disk` atoms hold a pair of 16-bit numbers, and a zero
/// is "no number at all": the MP4 family carries neither a zero nor a number
/// above 65535.
fn carries_number(kind: TagKind, value: u32) -> bool {
    match kind {
        TagKind::Id3v2 | TagKind::Vorbis => true,
        TagKind::Ilst => value > 0 && value <= u32::from(u16::MAX),
    }
}

/// The value the reader must see for a text field.
///
/// A field the metadata names is the value the writer was given; a field it
/// does not name keeps the value the file had.
fn expected_text_field(written: Option<&str>, before: Option<&str>) -> Option<String> {
    match written {
        Some(value) => expected_text(value),
        None => before.map(str::to_owned),
    }
}

/// The value the reader must see for the track number and the total.
///
/// The formats keep the pair in different ways:
///
/// * ID3v2 keeps the number and the total in one frame (`TRCK`). A total
///   without a number is written as the pair `0/total`, so the reader sees
///   a track number of zero.
/// * The Vorbis family keeps them in two comments.
/// * The MP4 `trkn` atom holds a pair of 16-bit numbers: a zero and a
///   number above 65535 are lost.
fn expected_track_pair(
    format: &Format,
    written: &TrackMetadata,
    before: &TrackMetadata,
) -> (Option<u32>, Option<u32>) {
    let number = written.track_no.or(before.track_no);
    let total = written.track_total.or(before.track_total);

    match format.kind {
        TagKind::Id3v2 => {
            let number = match (number, total) {
                (Some(number), _) => Some(number),
                // The pair `0/total` of the frame.
                (None, Some(_)) => Some(0),
                (None, None) => None,
            };
            (number, total)
        }
        TagKind::Vorbis => (number, total),
        TagKind::Ilst => (
            number.filter(|number| carries_number(format.kind, *number)),
            total.filter(|number| carries_number(format.kind, *number)),
        ),
    }
}

/// The value the reader must see for the disc number.
///
/// The disc number and the disc total share a frame in ID3v2 (`TPOS`) and an
/// atom in MP4 (`disk`), but the crate can only write the number: the total
/// of the file survives.
fn expected_disc_no(
    format: &Format,
    written: &TrackMetadata,
    before: &TrackMetadata,
) -> Option<u32> {
    let number = written.disc_no.or(before.disc_no);
    match format.kind {
        TagKind::Ilst => number.filter(|number| carries_number(format.kind, *number)),
        TagKind::Id3v2 | TagKind::Vorbis => number,
    }
}

/// The `extra` entries the reader must see.
fn expected_extra(
    format: &Format,
    written: &TrackMetadata,
    before: &TrackMetadata,
) -> BTreeMap<String, String> {
    let mut extra = before.extra.clone();
    for (key, value) in &written.extra {
        if carries_extra(format.kind, key) {
            extra.insert(key.clone(), value.clone());
        }
    }
    extra
}

/// The `extra` entries the writer must refuse.
///
/// The report names every entry the target tag format has no field for, and
/// nothing else.
fn refused_extra(format: &Format, written: &TrackMetadata) -> Vec<String> {
    written
        .extra
        .keys()
        .filter(|key| !carries_extra(format.kind, key))
        .cloned()
        .collect()
}

/// The metadata the reader must see after a write of `written` over a file
/// that held `before`.
fn expected_metadata(
    format: &Format,
    written: &TrackMetadata,
    before: &TrackMetadata,
) -> TrackMetadata {
    let (track_no, track_total) = expected_track_pair(format, written, before);

    TrackMetadata {
        title: expected_text_field(written.title.as_deref(), before.title.as_deref()),
        artist: expected_text_field(written.artist.as_deref(), before.artist.as_deref()),
        album: expected_text_field(written.album.as_deref(), before.album.as_deref()),
        album_artist: expected_text_field(
            written.album_artist.as_deref(),
            before.album_artist.as_deref(),
        ),
        year: written.year.or(before.year),
        genre: expected_text_field(written.genre.as_deref(), before.genre.as_deref()),
        comment: expected_text_field(written.comment.as_deref(), before.comment.as_deref()),
        track_no,
        track_total,
        disc_no: expected_disc_no(format, written, before),
        extra: expected_extra(format, written, before),
        ..TrackMetadata::default()
    }
}

// ---------------------------------------------------------------------------
// The fields
// ---------------------------------------------------------------------------

/// The fields the reader exposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Artist,
    Album,
    AlbumArtist,
    Year,
    Genre,
    Comment,
    TrackNo,
    TrackTotal,
    DiscNo,
}

/// Every field, in a fixed order.
const FIELDS: &[Field] = &[
    Field::Title,
    Field::Artist,
    Field::Album,
    Field::AlbumArtist,
    Field::Year,
    Field::Genre,
    Field::Comment,
    Field::TrackNo,
    Field::TrackTotal,
    Field::DiscNo,
];

impl Field {
    /// The value the reader sees for the field.
    fn value(self, metadata: &TrackMetadata) -> Option<String> {
        match self {
            Field::Title => metadata.title.clone(),
            Field::Artist => metadata.artist.clone(),
            Field::Album => metadata.album.clone(),
            Field::AlbumArtist => metadata.album_artist.clone(),
            Field::Year => metadata.year.map(|year| year.to_string()),
            Field::Genre => metadata.genre.clone(),
            Field::Comment => metadata.comment.clone(),
            Field::TrackNo => metadata.track_no.map(|number| number.to_string()),
            Field::TrackTotal => metadata.track_total.map(|number| number.to_string()),
            Field::DiscNo => metadata.disc_no.map(|number| number.to_string()),
        }
    }
}

/// Every removal the writer supports.
const REMOVALS: &[TagField] = &[
    TagField::Title,
    TagField::Artist,
    TagField::Album,
    TagField::AlbumArtist,
    TagField::Year,
    TagField::Genre,
    TagField::Comment,
    TagField::Track,
    TagField::Disc,
];

/// The fields a removal takes with it.
///
/// A field is a whole field: removing the track takes the track number and
/// the total number of tracks with it.
fn removed_fields(field: TagField) -> &'static [Field] {
    match field {
        TagField::Title => &[Field::Title],
        TagField::Artist => &[Field::Artist],
        TagField::Album => &[Field::Album],
        TagField::AlbumArtist => &[Field::AlbumArtist],
        TagField::Year => &[Field::Year],
        TagField::Genre => &[Field::Genre],
        TagField::Comment => &[Field::Comment],
        TagField::Track => &[Field::TrackNo, Field::TrackTotal],
        TagField::Disc => &[Field::DiscNo],
    }
}

/// A year for a patch: never absent and never above the largest year a tag
/// can hold.
fn random_patch_year(rng: &mut Rng) -> u32 {
    match rng.below(100) {
        0..=24 => 0,
        25..=49 => 9999,
        50..=74 => 1,
        _ => rng.between(1000, 9999) as u32,
    }
}

/// A number for a patch: never absent, sometimes an edge.
fn random_patch_number(rng: &mut Rng) -> u32 {
    match rng.below(100) {
        0..=24 => 0,
        25..=49 => u32::MAX,
        50..=69 => 65535,
        70..=79 => 65536,
        _ => rng.between(1, 1000) as u32,
    }
}

/// A metadata that names exactly one field.
fn patch(field: Field, rng: &mut Rng) -> TrackMetadata {
    let mut metadata = TrackMetadata::default();
    match field {
        Field::Title => metadata.title = Some(random_word(rng)),
        Field::Artist => metadata.artist = Some(random_word(rng)),
        Field::Album => metadata.album = Some(random_word(rng)),
        Field::AlbumArtist => metadata.album_artist = Some(random_word(rng)),
        Field::Genre => metadata.genre = Some(random_word(rng)),
        Field::Comment => metadata.comment = Some(random_word(rng)),
        Field::Year => metadata.year = Some(random_patch_year(rng)),
        Field::TrackNo => metadata.track_no = Some(random_patch_number(rng)),
        Field::TrackTotal => metadata.track_total = Some(random_patch_number(rng)),
        Field::DiscNo => metadata.disc_no = Some(random_patch_number(rng)),
    }
    metadata
}

// ---------------------------------------------------------------------------
// The properties
// ---------------------------------------------------------------------------

#[test]
fn a_write_round_trips_the_values_the_format_can_carry() {
    for format in FORMATS {
        for seed in SEEDS {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join(format.name);
            (format.make)(&path);

            let mut rng = Rng::new(seed);
            for case in 0..CASES {
                let written = random_metadata(&mut rng);
                let before = read_tags(&path).unwrap();
                let audio = bytes_of(&path);

                let report = write_tags_report(&path, &written).unwrap();

                let after = read_tags(&path).unwrap();
                assert_eq!(
                    after,
                    expected_metadata(format, &written, &before),
                    "{}: seed {seed:#x}, case {case}",
                    format.name
                );
                assert_eq!(
                    report.unwritten_extra,
                    refused_extra(format, &written),
                    "{}: seed {seed:#x}, case {case}: the report names the wrong entries",
                    format.name
                );
                assert_audio_unchanged(format, &path, &audio);
            }
        }
    }
}

#[test]
fn a_field_the_metadata_does_not_name_keeps_its_value() {
    for format in FORMATS {
        for seed in SEEDS {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join(format.name);
            (format.make)(&path);

            let mut rng = Rng::new(seed);
            for case in 0..CASES {
                let field = *rng.pick(FIELDS);
                let patch = patch(field, &mut rng);
                let before = read_tags(&path).unwrap();
                let audio = bytes_of(&path);

                write_tags(&path, &patch).unwrap();

                let after = read_tags(&path).unwrap();
                let expected = expected_metadata(format, &patch, &before);
                for other in FIELDS {
                    if *other == field {
                        assert_eq!(
                            (*other).value(&after),
                            (*other).value(&expected),
                            "{}: seed {seed:#x}, case {case}: {:?} did not take the new value",
                            format.name,
                            other
                        );
                        continue;
                    }
                    // The one exception: the ID3v2 `TRCK` frame is a pair, so
                    // a track total without a track number is written as the
                    // pair `0/total` and the file gains a track number of
                    // zero.
                    let gains_a_track_number = format.kind == TagKind::Id3v2
                        && field == Field::TrackTotal
                        && *other == Field::TrackNo
                        && before.track_no.is_none();
                    if gains_a_track_number {
                        assert_eq!(
                            (*other).value(&after),
                            Some(0.to_string()),
                            "{}: seed {seed:#x}, case {case}: the track number is not zero",
                            format.name
                        );
                        continue;
                    }
                    assert_eq!(
                        (*other).value(&after),
                        (*other).value(&before),
                        "{}: seed {seed:#x}, case {case}: {:?} changed",
                        format.name,
                        other
                    );
                }
                assert_eq!(
                    after.extra, before.extra,
                    "{}: seed {seed:#x}, case {case}: the extra entries changed",
                    format.name
                );
                assert_audio_unchanged(format, &path, &audio);
            }
        }
    }
}

#[test]
fn a_removal_takes_the_field_it_names_and_nothing_else() {
    for format in FORMATS {
        for seed in SEEDS {
            let dir = TempDir::new().unwrap();
            let path = dir.path().join(format.name);
            (format.make)(&path);

            let mut rng = Rng::new(seed);
            for case in 0..CASES {
                let removal = *rng.pick(REMOVALS);

                // A file with every field filled: the removal of one field
                // has to leave the others alone.
                let written = full_metadata(&mut rng);
                let before = read_tags(&path).unwrap();
                write_tags(&path, &written).unwrap();
                let filled = expected_metadata(format, &written, &before);
                let audio = bytes_of(&path);

                let report =
                    write_tags_with_report(&path, &TrackMetadata::default(), &[removal]).unwrap();
                assert!(
                    report.unremoved.is_empty(),
                    "{}: seed {seed:#x}, case {case}: {:?} was not removed",
                    format.name,
                    report.unremoved
                );

                let after = read_tags(&path).unwrap();
                for field in FIELDS {
                    if removed_fields(removal).contains(field) {
                        assert_eq!(
                            (*field).value(&after),
                            None,
                            "{}: seed {seed:#x}, case {case}: {:?} survived the removal",
                            format.name,
                            field
                        );
                    } else {
                        assert_eq!(
                            (*field).value(&after),
                            (*field).value(&filled),
                            "{}: seed {seed:#x}, case {case}: {:?} changed",
                            format.name,
                            field
                        );
                    }
                }
                // The disc total lives in `extra` on the Vorbis family, so a
                // removal of the disc has to take it with it.
                if removal == TagField::Disc {
                    assert!(
                        !after.extra.contains_key("DISCTOTAL"),
                        "{}: seed {seed:#x}, case {case}: the disc total survived",
                        format.name
                    );
                } else {
                    assert_eq!(
                        after.extra, filled.extra,
                        "{}: seed {seed:#x}, case {case}: the extra entries changed",
                        format.name
                    );
                }
                assert_audio_unchanged(format, &path, &audio);
            }
        }
    }
}

#[test]
fn the_item_the_crate_has_no_field_for_survives_every_write() {
    for format in FORMATS {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(format.name);
        (format.make)(&path);

        assert!(
            bytes_of(&path)
                .windows(UNKNOWN_MARKER.len())
                .any(|window| window == UNKNOWN_MARKER.as_bytes()),
            "{}: the unknown item is not in the file",
            format.name
        );

        write_tags(&path, &random_metadata(&mut Rng::new(0x5EED_0001))).unwrap();
        assert!(
            bytes_of(&path)
                .windows(UNKNOWN_MARKER.len())
                .any(|window| window == UNKNOWN_MARKER.as_bytes()),
            "{}: a write dropped the unknown item",
            format.name
        );

        write_tags_with_report(&path, &TrackMetadata::default(), &[TagField::Title]).unwrap();
        assert!(
            bytes_of(&path)
                .windows(UNKNOWN_MARKER.len())
                .any(|window| window == UNKNOWN_MARKER.as_bytes()),
            "{}: a removal dropped the unknown item",
            format.name
        );
    }
}

#[test]
fn the_cases_reach_the_edges_of_the_formats() {
    // The properties are only worth something when the cases reach the
    // edges: an empty value, a value of blanks, a year of zero, a number
    // above what the MP4 family can carry. A generator that stopped there
    // would make every property vacuous.
    let mut absent = 0;
    let mut empty = 0;
    let mut blanks = 0;
    let mut padded = 0;
    let mut plain = 0;
    let mut unicode = 0;
    let mut year_zero = 0;
    let mut year_last = 0;
    let mut number_zero = 0;
    let mut number_last = 0;
    let mut number_over_mp4 = 0;

    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for _ in 0..CASES {
            let metadata = random_metadata(&mut rng);
            for value in [
                &metadata.title,
                &metadata.artist,
                &metadata.album,
                &metadata.album_artist,
                &metadata.genre,
                &metadata.comment,
            ] {
                match value {
                    None => absent += 1,
                    Some(value) if value.is_empty() => empty += 1,
                    Some(value) if value.trim().is_empty() => blanks += 1,
                    Some(value) if value.trim() != value => padded += 1,
                    Some(value) if value.is_ascii() => plain += 1,
                    Some(_) => unicode += 1,
                }
            }
            match metadata.year {
                Some(0) => year_zero += 1,
                Some(9999) => year_last += 1,
                _ => {}
            }
            for number in [metadata.track_no, metadata.track_total, metadata.disc_no] {
                match number {
                    Some(0) => number_zero += 1,
                    Some(u32::MAX) => number_last += 1,
                    Some(number) if number > u32::from(u16::MAX) => number_over_mp4 += 1,
                    _ => {}
                }
            }
        }
    }

    for (what, count) in [
        ("a value that is not there", absent),
        ("an empty value", empty),
        ("a value of blanks", blanks),
        ("a padded value", padded),
        ("a plain value", plain),
        ("a value outside ASCII", unicode),
        ("a year of zero", year_zero),
        ("the last year a tag can hold", year_last),
        ("a number of zero", number_zero),
        ("the last number a tag can hold", number_last),
        ("a number above what MP4 can carry", number_over_mp4),
    ] {
        assert!(count > 0, "no case has {what}");
    }
}
