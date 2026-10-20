//! The malformed-input suite of the header probes.
//!
//! The probes of this crate read bytes that arrive from outside: a file that
//! was half downloaded, a stream that was cut, a file that lies about its
//! length. Every one of them has to answer such bytes with a decision or
//! with the absence of one - never with a panic and never with a read
//! outside the buffer it was given.
//!
//! The corpus is generated here and seeded: the same seed builds the same
//! corpus on every run and on every machine, so a failure is replayable and
//! the suite does not change from run to run. No dependency is added for it.
//!
//! The probes are crate-internal (`pub(crate)`), so the suite lives inside
//! the crate: it is a unit test, not an integration test.
//!
//! What the suite feeds the probes:
//!
//! * random bytes of many lengths (0, 1, 3, 10, 512, ...),
//! * truncated prefixes of every fixture of the crate, walked over the length
//!   of the file,
//! * every capture pattern of the formats the probes read, followed by
//!   random bytes,
//! * files of the right shape with wrong numbers in them: a chunk, a
//!   metadata block, a box, a page, a frame header and an identification
//!   header, each with random lengths and random fields,
//! * fixtures with a few bytes damaged, which is what a lying header looks
//!   like.

use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;

use symphonia::core::codecs::{CODEC_TYPE_OPUS, CodecParameters, Decoder, DecoderOptions};
use symphonia::core::formats::Packet;

use crate::aiff_header;
use crate::flac_header;
use crate::mp4_header;
use crate::mpeg_header;
use crate::ogg_header;
use crate::opus_dec;
use crate::symphonia_dec::header_probe;

// ---------------------------------------------------------------------------
// The seeded generator
// ---------------------------------------------------------------------------

/// The seed of the corpus.
///
/// A fixed seed keeps the suite reproducible: the same corpus on every run
/// and on every machine, so a failure can be replayed from the seed alone.
const SEED: u64 = 0x5EED_0FC0_FFEE_0001;

/// A deterministic pseudo-random generator (xorshift64).
///
/// The suite must be reproducible without a dependency: a generator that
/// carries its own state and says nothing about the platform is enough.
struct Rng(u64);

impl Rng {
    /// A generator with the given seed.
    ///
    /// A seed of zero would leave the generator in a state it never leaves,
    /// so the seed is forced to be odd.
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    /// The next 64 bits.
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// The next byte.
    fn byte(&mut self) -> u8 {
        (self.next_u64() >> 24) as u8
    }

    /// Fills the buffer with random bytes.
    fn fill(&mut self, buf: &mut [u8]) {
        for b in buf {
            *b = self.byte();
        }
    }
}

// ---------------------------------------------------------------------------
// The corpus
// ---------------------------------------------------------------------------

/// One input of the corpus.
struct Input {
    /// What the input is, for the failure message.
    label: String,
    /// The bytes of the input.
    bytes: Vec<u8>,
}

/// The lengths the random inputs have.
///
/// The short ones matter most: a probe that reads a length out of the bytes
/// it was given must not read past them. The long ones matter too: a probe
/// that walks a container must not walk forever.
const RANDOM_LENGTHS: &[usize] = &[
    0, 1, 2, 3, 4, 7, 10, 16, 27, 31, 32, 63, 64, 100, 128, 255, 256, 512, 1024, 4096,
];

/// The capture patterns of the formats the probes read.
///
/// A pattern is the beginning of a file that the format in question would
/// accept. The bytes behind it are random: the probe has to decide what it
/// is looking at without trusting anything else.
const PATTERNS: &[(&str, &[u8])] = &[
    ("ID3v2", b"ID3\x04\x00\x00"),
    ("fLaC", b"fLaC"),
    ("OggS", b"OggS"),
    ("FORM/AIFF", b"FORM\x00\x00\x00\x00AIFF"),
    ("FORM/AIFC", b"FORM\x00\x00\x00\x00AIFC"),
    ("RIFF/WAVE", b"RIFF\x00\x00\x00\x00WAVE"),
    ("ftyp", b"\x00\x00\x00\x18ftyp"),
    ("moov", b"\x00\x00\x00\x08moov"),
    ("mdat", b"\x00\x00\x00\x08mdat"),
    ("styp", b"\x00\x00\x00\x08styp"),
    ("OpusHead", b"OpusHead"),
    ("EBML", b"\x1A\x45\xDF\xA3"),
    ("caff", b"caff"),
    ("an MPEG frame", b"\xFF\xFB\x90\x00"),
];

/// The lengths of the random tail behind a capture pattern.
const TAIL_LENGTHS: &[usize] = &[0, 1, 8, 64, 512];

/// The fixtures of the crate: one file per format the probes read.
const FIXTURES: &[&str] = &[
    "sine.mp3",
    "sine.flac",
    "sine.ogg",
    "sine.opus",
    "sine_alac.m4a",
    "sine_aac.m4a",
    "sine_opus.mkv",
];

/// The corpus of the suite.
///
/// It is built once and shared by the probes: the tests run in parallel and
/// every one of them walks the same bytes.
fn corpus() -> &'static [Input] {
    static CORPUS: OnceLock<Vec<Input>> = OnceLock::new();
    CORPUS.get_or_init(build_corpus)
}

/// Builds the corpus.
fn build_corpus() -> Vec<Input> {
    let mut out = Vec::new();
    random_inputs(&mut out);
    pattern_inputs(&mut out);
    structured_inputs(&mut out);
    for (name, bytes) in fixtures() {
        prefix_inputs(&mut out, &name, &bytes);
        damaged_inputs(&mut out, &name, &bytes);
    }
    out
}

/// The random inputs: bytes of many lengths.
fn random_inputs(out: &mut Vec<Input>) {
    for (i, &len) in RANDOM_LENGTHS.iter().enumerate() {
        for round in 0..4 {
            let mut rng = Rng::new(SEED ^ ((i as u64) << 8) ^ round as u64);
            let mut bytes = vec![0u8; len];
            rng.fill(&mut bytes);
            out.push(Input {
                label: format!("{len} random bytes #{round}"),
                bytes,
            });
        }
    }
}

/// The capture patterns followed by random bytes.
fn pattern_inputs(out: &mut Vec<Input>) {
    for (name, pattern) in PATTERNS {
        for &tail in TAIL_LENGTHS {
            for round in 0..4 {
                let mut rng = Rng::new(SEED ^ hash(name) ^ ((tail as u64) << 8) ^ round as u64);
                let mut bytes = pattern.to_vec();
                bytes.resize(pattern.len() + tail, 0);
                rng.fill(&mut bytes[pattern.len()..]);
                out.push(Input {
                    label: format!("{name} followed by {tail} random bytes #{round}"),
                    bytes,
                });
            }
        }
    }
}

/// The malformed files of the formats the probes read.
///
/// Random bytes rarely reach the interesting part of a probe: a probe reads
/// a length out of the file and then walks the file by that length. These
/// inputs are files of the right shape with wrong numbers in them, so the
/// walk goes where it should not.
fn structured_inputs(out: &mut Vec<Input>) {
    aiff_inputs(out);
    flac_inputs(out);
    mp4_inputs(out);
    ogg_inputs(out);
    mpeg_inputs(out);
    opus_inputs(out);
}

/// The number of malformed files of every format.
const STRUCTURED_ROUNDS: usize = 64;

/// A malformed AIFF file: the chunks and the fields in them are random.
fn aiff_inputs(out: &mut Vec<Input>) {
    for round in 0..STRUCTURED_ROUNDS {
        let mut rng = Rng::new(SEED ^ hash("aiff") ^ round as u64);
        let compressed = rng.byte().is_multiple_of(2);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"FORM");
        // The length of the form: the walk of the chunks ignores it.
        bytes.extend_from_slice(&0u32.to_be_bytes());
        bytes.extend_from_slice(if compressed { b"AIFC" } else { b"AIFF" });
        // The chunks before the sound data.
        for tag in [b"FVER".as_slice(), b"COMM".as_slice()] {
            let payload = aiff_payload(tag, &mut rng, compressed);
            bytes.extend_from_slice(tag);
            bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            bytes.extend_from_slice(&payload);
        }
        // The sound data chunk.
        let payload = aiff_payload(b"SSND", &mut rng, compressed);
        bytes.extend_from_slice(b"SSND");
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&payload);
        out.push(Input {
            label: format!("an AIFF file with random chunks #{round}"),
            bytes,
        });
    }
}

/// The payload of a chunk of a malformed AIFF file.
fn aiff_payload(tag: &[u8], rng: &mut Rng, compressed: bool) -> Vec<u8> {
    match tag {
        b"COMM" => {
            // The channel count, the number of sample frames and the number of
            // bits per sample.
            let mut payload = vec![0u8; 18];
            payload[0..2].copy_from_slice(&(u16::from(1 + rng.byte() % 4)).to_be_bytes());
            payload[2..6].copy_from_slice(&(rng.next_u64() as u32).to_be_bytes());
            payload[6..8]
                .copy_from_slice(&[8u16, 16, 24, 32][(rng.byte() % 4) as usize].to_be_bytes());
            if compressed {
                // The name of the compression.
                let name: &[u8; 4] = match rng.byte() % 6 {
                    0 => b"none",
                    1 => b"NONE",
                    2 => b"sowt",
                    3 => b"ulaw",
                    4 => b"fl32",
                    _ => b"fl64",
                };
                payload.extend_from_slice(name);
            }
            payload
        }
        b"SSND" => {
            // The offset and the block size: both have to be zero for the
            // reader to accept the file.
            let mut payload = vec![0u8; 8];
            if rng.byte().is_multiple_of(4) {
                payload[0..4].copy_from_slice(&4u32.to_be_bytes());
            }
            if rng.byte().is_multiple_of(4) {
                payload[4..8].copy_from_slice(&4u32.to_be_bytes());
            }
            payload
        }
        _ => {
            let mut payload = vec![0u8; 4];
            rng.fill(&mut payload);
            payload
        }
    }
}

/// A malformed FLAC file: the chain of metadata blocks and their lengths.
fn flac_inputs(out: &mut Vec<Input>) {
    for round in 0..STRUCTURED_ROUNDS {
        let mut rng = Rng::new(SEED ^ hash("flac") ^ round as u64);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"fLaC");
        for _ in 0..1 + rng.byte() % 4 {
            // The type of the block and its "last block" flag.
            let kind = rng.byte() % 0x80;
            let last = if rng.byte().is_multiple_of(2) {
                0x80
            } else {
                0
            };
            bytes.push(kind | last);
            // The length of the payload.
            let len = rng.next_u64() as u32 % 4096;
            bytes.extend_from_slice(&len.to_be_bytes()[1..]);
            // The payload: as many bytes as the block claims, or fewer.
            let hold = if rng.byte().is_multiple_of(2) {
                len as usize
            } else {
                (len as usize) / 2
            };
            for _ in 0..hold {
                bytes.push(rng.byte());
            }
        }
        out.push(Input {
            label: format!("a FLAC file with random metadata blocks #{round}"),
            bytes,
        });
    }
}

/// A malformed MP4 file: a tree of boxes with random types and sizes.
fn mp4_inputs(out: &mut Vec<Input>) {
    for round in 0..STRUCTURED_ROUNDS {
        let mut rng = Rng::new(SEED ^ hash("mp4") ^ round as u64);
        let mut bytes = Vec::new();
        // The file type box: the file starts like an MP4 file.
        bytes.extend_from_slice(&16u32.to_be_bytes());
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(b"M4A ");
        bytes.extend_from_slice(&[0u8; 4]);
        bytes.extend_from_slice(&mp4_boxes(&mut rng, 3));
        out.push(Input {
            label: format!("an MP4 file with a random box tree #{round}"),
            bytes,
        });
    }
}

/// A tree of boxes with random types and sizes.
///
/// The size of a box is the real one, zero (the box reaches to the end of
/// the file) or a number the file does not hold: every one of them is a
/// length the walk has to believe.
fn mp4_boxes(rng: &mut Rng, depth: u32) -> Vec<u8> {
    const KINDS: [&[u8; 4]; 12] = [
        b"moov", b"trak", b"mdia", b"minf", b"stbl", b"stsz", b"stsc", b"stco", b"co64", b"hdlr",
        b"mdhd", b"free",
    ];
    let mut payload = Vec::new();
    if depth > 0 {
        for _ in 0..rng.byte() % 3 {
            payload.extend_from_slice(&mp4_boxes(rng, depth - 1));
        }
    }
    let size = match rng.byte() % 3 {
        // A size of zero: the box reaches to the end of the file.
        0 => 0u32,
        // A size the file does not hold.
        1 => rng.next_u64() as u32,
        // The real size of the box.
        _ => 8 + payload.len() as u32,
    };
    let mut out = Vec::new();
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(KINDS[(rng.byte() as usize) % KINDS.len()]);
    out.extend_from_slice(&payload);
    out
}

/// A malformed Ogg file: the pages of the file and their segment tables.
fn ogg_inputs(out: &mut Vec<Input>) {
    for round in 0..STRUCTURED_ROUNDS {
        let mut rng = Rng::new(SEED ^ hash("ogg") ^ round as u64);
        let mut bytes = Vec::new();
        for _ in 0..1 + rng.byte() % 2 {
            bytes.extend_from_slice(b"OggS");
            bytes.push(0); // the version
            bytes.push(rng.byte() % 8); // the flags
            bytes.extend_from_slice(&rng.next_u64().to_le_bytes()); // the granule
            bytes.extend_from_slice(&(rng.next_u64() as u32).to_le_bytes()); // the serial
            bytes.extend_from_slice(&(rng.next_u64() as u32).to_le_bytes()); // the sequence
            bytes.extend_from_slice(&0u32.to_le_bytes()); // the checksum: never right
            // The segment table and the length of the body it states.
            let segments = rng.byte();
            bytes.push(segments);
            let mut lacing = 0usize;
            for _ in 0..segments {
                let lacing_value = rng.byte();
                lacing += lacing_value as usize;
                bytes.push(lacing_value);
            }
            // The body: as many bytes as the table claims, or fewer.
            let hold = if rng.byte().is_multiple_of(2) {
                lacing
            } else {
                lacing / 2
            };
            for _ in 0..hold {
                bytes.push(rng.byte());
            }
        }
        out.push(Input {
            label: format!("an Ogg file with random pages #{round}"),
            bytes,
        });
    }
}

/// A malformed MPEG audio file: a frame header and a Xing/Info tag.
fn mpeg_inputs(out: &mut Vec<Input>) {
    for round in 0..STRUCTURED_ROUNDS {
        let mut rng = Rng::new(SEED ^ hash("mpeg") ^ round as u64);
        // The frame header: the version, the layer and the bitrate are
        // random, so the header is a frame or not.
        let mut bytes = vec![0xFF, 0xE0 | (rng.byte() & 0x1F), rng.byte(), rng.byte()];
        // The side information and the tag behind it.
        for _ in 0..rng.byte() as usize % 40 {
            bytes.push(rng.byte());
        }
        let tag: &[u8; 4] = if rng.byte().is_multiple_of(2) {
            b"Xing"
        } else {
            b"Info"
        };
        bytes.extend_from_slice(tag);
        bytes.extend_from_slice(&(rng.next_u64() as u32).to_be_bytes()); // the flags
        bytes.extend_from_slice(&(rng.next_u64() as u32).to_be_bytes()); // the frames
        bytes.extend_from_slice(&(rng.next_u64() as u32).to_be_bytes()); // the bytes
        out.push(Input {
            label: format!("an MPEG file with a random Xing tag #{round}"),
            bytes,
        });
    }
}

/// A malformed Opus identification header.
fn opus_inputs(out: &mut Vec<Input>) {
    for round in 0..STRUCTURED_ROUNDS {
        let mut rng = Rng::new(SEED ^ hash("opus") ^ round as u64);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"OpusHead");
        bytes.push(1); // the version
        bytes.push(rng.byte()); // the channel count
        bytes.extend_from_slice(&(rng.next_u64() as u16).to_le_bytes()); // the pre-skip
        bytes.extend_from_slice(&(rng.next_u64() as u32).to_le_bytes()); // the rate
        bytes.extend_from_slice(&(rng.next_u64() as u16).to_le_bytes()); // the gain
        bytes.push(rng.byte()); // the mapping family
        // The channel mapping table.
        for _ in 0..rng.byte() as usize % 8 {
            bytes.push(rng.byte());
        }
        out.push(Input {
            label: format!("an Opus identification header with random fields #{round}"),
            bytes,
        });
    }
}

/// The truncated prefixes of a fixture.
///
/// The prefix length is walked: every length at the front of the file, where
/// the headers and their length fields are, then a growing stride, so that
/// the whole file is covered without walking every byte of a 27 kB file.
fn prefix_inputs(out: &mut Vec<Input>, name: &str, bytes: &[u8]) {
    for len in prefix_lengths(bytes.len()) {
        out.push(Input {
            label: format!("{name} cut at {len} bytes"),
            bytes: bytes[..len].to_vec(),
        });
    }
}

/// The prefix lengths of a file that the suite walks.
fn prefix_lengths(len: usize) -> Vec<usize> {
    /// The number of lengths that are walked one by one.
    const HEAD: usize = 256;
    let mut out: Vec<usize> = (0..=len.min(HEAD)).collect();
    let mut at = HEAD;
    let mut stride = 1usize;
    while at < len {
        at += stride;
        stride += stride / 8 + 1;
        out.push(at.min(len));
    }
    out.push(len);
    out.dedup();
    out
}

/// The fixtures with a few bytes damaged.
///
/// A damaged byte is what a lying header looks like: the structure of the
/// file is intact, but one of its numbers states something else.
fn damaged_inputs(out: &mut Vec<Input>, name: &str, bytes: &[u8]) {
    for round in 0..32 {
        let mut rng = Rng::new(SEED ^ hash(name) ^ (round as u64) << 16);
        let mut damaged = bytes.to_vec();
        for _ in 0..4 {
            let at = (rng.next_u64() % damaged.len().max(1) as u64) as usize;
            if let Some(byte) = damaged.get_mut(at) {
                *byte = rng.byte();
            }
        }
        out.push(Input {
            label: format!("{name} with four damaged bytes #{round}"),
            bytes: damaged,
        });
    }
}

/// The fixtures of the crate, read from the repository.
fn fixtures() -> Vec<(String, Vec<u8>)> {
    FIXTURES
        .iter()
        .map(|name| {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name);
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("the fixture {name}: {e}"));
            ((*name).to_string(), bytes)
        })
        .collect()
}

/// A number of a name, so that every name gets its own part of the corpus.
fn hash(name: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in name.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

// ---------------------------------------------------------------------------
// The probes
// ---------------------------------------------------------------------------

/// The MPEG audio probe: the length of the ID3v2 tag and the Xing/Info tag.
fn probe_mpeg(bytes: &[u8]) {
    let id3v2 = mpeg_header::id3v2_len(bytes);
    // A tag that claims more than the file holds leaves nothing behind it.
    let window = bytes.get(id3v2..).unwrap_or_default();
    let _ = mpeg_header::parse(id3v2, window);
}

/// The AIFF probe: the `COMM` and `SSND` chunks.
fn probe_aiff(bytes: &[u8]) {
    let len = bytes.len() as u64;
    let _ = aiff_header::parse(&mut Cursor::new(bytes), Some(len));
    let _ = aiff_header::parse(&mut Cursor::new(bytes), None);
}

/// The FLAC probe: the marker and the chain of metadata blocks.
fn probe_flac(bytes: &[u8]) {
    let len = bytes.len() as u64;
    let id3v2 = mpeg_header::id3v2_len(bytes) as u64;
    let _ = flac_header::looks_like_flac(bytes);
    let _ = flac_header::parse(&mut Cursor::new(bytes), 0, Some(len));
    let _ = flac_header::parse(&mut Cursor::new(bytes), 0, None);
    // A FLAC file may carry an ID3v2 tag in front of the marker.
    let _ = flac_header::parse(&mut Cursor::new(bytes), id3v2, Some(len));
    let _ = flac_header::parse(&mut Cursor::new(bytes), id3v2, None);
}

/// The MP4 probe: the box tree of the file.
fn probe_mp4(bytes: &[u8]) {
    let len = bytes.len() as u64;
    let _ = mp4_header::looks_like_mp4(bytes);
    let _ = mp4_header::parse(&mut Cursor::new(bytes), Some(len));
}

/// The Ogg probe: the pages of the file.
fn probe_ogg(bytes: &[u8]) {
    let _ = ogg_header::looks_like_ogg(bytes);
    let _ = ogg_header::parse(&mut Cursor::new(bytes));
}

/// The Opus probe: the identification header of the stream.
fn probe_opus_header(bytes: &[u8]) {
    let _ = opus_dec::pre_skip(Some(bytes));
    let _ = opus_dec::pre_skip(None);
    let mut params = CodecParameters::new();
    params.for_codec(CODEC_TYPE_OPUS);
    params.with_extra_data(bytes.to_vec().into_boxed_slice());
    opus_dec::complete_parameters(&mut params);
    let _ = opus_dec::audio_frames(&params, Some(bytes.len() as u64));
}

/// The Opus probe: the table of contents of a packet and the packet itself.
///
/// The table of contents states how many frames the packet holds; the codec
/// reads the rest of the packet.
fn probe_opus_packet(bytes: &[u8]) {
    let _ = opus_dec::packet_frames(bytes);
}

/// The header probe of the symphonia decoder: it ties the probes above
/// together, so it sees every one of them.
fn probe_header(bytes: &[u8]) {
    let len = bytes.len() as u64;
    let _ = header_probe(&mut Cursor::new(bytes), Some(len));
    let _ = header_probe(&mut Cursor::new(bytes), None);
}

// ---------------------------------------------------------------------------
// The sweep
// ---------------------------------------------------------------------------

/// Runs one probe over the whole corpus.
///
/// A panic is caught and named: the input that caused it is part of the
/// message, so a failure can be replayed. The suite keeps going after a
/// panic, so one run names every input that breaks a probe.
fn sweep(name: &str, mut probe: impl FnMut(&[u8])) {
    let mut failures: Vec<String> = Vec::new();
    for input in corpus() {
        let result = catch_unwind(AssertUnwindSafe(|| probe(&input.bytes)));
        if let Err(payload) = result {
            failures.push(format!(
                "  {}\n    {}\n    bytes: {}",
                input.label,
                panic_message(&payload),
                hex(&input.bytes),
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{name} panicked on {} input(s) of the corpus:\n{}",
        failures.len(),
        failures.join("\n"),
    );
}

/// The message of a caught panic.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "a panic of an unknown type".to_string()
    }
}

/// The first bytes of an input, in hex, for a failure message.
fn hex(bytes: &[u8]) -> String {
    const SHOWN: usize = 48;
    let mut out = String::new();
    for (i, b) in bytes.iter().take(SHOWN).enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{b:02X}"));
    }
    if bytes.len() > SHOWN {
        out.push_str(" ...");
    }
    out
}

/// The corpus has to reach the interesting part of every probe.
///
/// A corpus that never gets past the first bytes of a file would prove
/// nothing: the counts below say how many inputs a probe has a decision
/// about, and they are the lower bound the suite keeps.
#[test]
fn the_corpus_reaches_the_decisions_of_every_probe() {
    let mut flac = 0usize;
    let mut ogg = 0usize;
    let mut mp4 = 0usize;
    let mut mpeg = 0usize;
    let mut aiff = 0usize;
    let mut opus = 0usize;
    for input in corpus() {
        let bytes = &input.bytes;
        let len = bytes.len() as u64;
        if flac_header::parse(&mut Cursor::new(bytes), 0, Some(len)).is_some() {
            flac += 1;
        }
        if ogg_header::parse(&mut Cursor::new(bytes)).is_some() {
            ogg += 1;
        }
        if mp4_header::parse(&mut Cursor::new(bytes), Some(len))
            .is_some_and(|h| h.media_data().is_some())
        {
            mp4 += 1;
        }
        let id3v2 = mpeg_header::id3v2_len(bytes);
        if mpeg_header::parse(id3v2, bytes.get(id3v2..).unwrap_or_default()).is_some() {
            mpeg += 1;
        }
        if aiff_header::parse(&mut Cursor::new(bytes), Some(len)).is_some() {
            aiff += 1;
        }
        if opus_dec::pre_skip(Some(bytes)) > 0 {
            opus += 1;
        }
    }
    assert!(
        flac >= 200,
        "the FLAC probe has a decision about {flac} inputs"
    );
    assert!(
        ogg >= 300,
        "the Ogg probe has a decision about {ogg} inputs"
    );
    assert!(
        mp4 >= 200,
        "the MP4 probe finds a media data box in {mp4} inputs"
    );
    assert!(mpeg >= 50, "the MPEG probe finds a tag in {mpeg} inputs");
    assert!(
        aiff >= 20,
        "the AIFF probe has a decision about {aiff} inputs"
    );
    assert!(opus >= 30, "the Opus probe reads a header in {opus} inputs");
}

// ---------------------------------------------------------------------------
// A box that claims more than the file holds
// ---------------------------------------------------------------------------

/// A box of a hostile MP4 file.
///
/// `size` is the size the box claims; the file holds only its header and the
/// payload that is written down.
fn hostile_box(kind: &[u8; 4], size: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

/// An MP4 file whose boxes claim far more than the file holds.
///
/// The sample size box claims a hundred million samples and the sample table
/// claims to hold it: a probe that reads the table would allocate 64 MB for a
/// file of 140 bytes. The boxes are nested so that the walk reaches the
/// sample table: every box claims more than the one in it.
fn a_hostile_mp4() -> Vec<u8> {
    // The sample size box: the version, the size of every sample (zero: they
    // differ) and the number of samples.
    let mut stsz = [0u8; 12];
    stsz[8..12].copy_from_slice(&0x0100_0000u32.to_be_bytes());
    let stbl = hostile_box(
        b"stbl",
        0xFFFF_F800,
        &hostile_box(b"stsz", 0x0800_0000, &stsz),
    );
    let minf = hostile_box(b"minf", 0xFFFF_FC00, &stbl);
    // The handler reference: the track holds sound. The name of the handler
    // stands behind the version, the flags and the predefined field.
    let mut hdlr = [0u8; 24];
    hdlr[8..12].copy_from_slice(b"soun");
    // The media header: the timescale is the sample rate, the duration the
    // number of frames.
    let mut mdhd = [0u8; 24];
    mdhd[12..16].copy_from_slice(&44_100u32.to_be_bytes());
    mdhd[16..20].copy_from_slice(&66_150u32.to_be_bytes());
    let mut mdia = hostile_box(b"hdlr", 32, &hdlr);
    mdia.extend_from_slice(&hostile_box(b"mdhd", 32, &mdhd));
    mdia.extend_from_slice(&minf);
    // The track box and the movie box around them.
    let trak = hostile_box(
        b"trak",
        0xFFFF_FF00,
        &hostile_box(b"mdia", 0xFFFF_FE00, &mdia),
    );
    let moov = hostile_box(b"moov", 0xFFFF_FFF0, &trak);

    let mut out = Vec::new();
    // The file type box: the file starts like an MP4 file.
    out.extend_from_slice(&16u32.to_be_bytes());
    out.extend_from_slice(b"ftyp");
    out.extend_from_slice(b"M4A ");
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&moov);
    out
}

/// A reader that records the largest read it was asked for.
///
/// A probe that asks for more bytes than the file holds wants to allocate
/// them: the request is the allocation.
struct CountingReader {
    /// The bytes of the file.
    bytes: Vec<u8>,
    /// The offset the reader is at.
    pos: usize,
    /// The largest read the reader was asked for.
    largest: usize,
}

impl CountingReader {
    /// A reader over the given bytes.
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            pos: 0,
            largest: 0,
        }
    }
}

impl Read for CountingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.largest = self.largest.max(buf.len());
        let left = self.bytes.len().saturating_sub(self.pos);
        let n = left.min(buf.len());
        buf[..n].copy_from_slice(&self.bytes[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

impl Seek for CountingReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let at = match pos {
            SeekFrom::Start(at) => at,
            SeekFrom::End(end) => (self.bytes.len() as u64).saturating_add_signed(end),
            SeekFrom::Current(delta) => (self.pos as u64).saturating_add_signed(delta),
        };
        self.pos = at.min(self.bytes.len() as u64) as usize;
        Ok(self.pos as u64)
    }
}

/// A box that claims more than the file holds must not make the probe
/// allocate it.
///
/// The sample size box of the file claims a hundred million samples, and the
/// sample table claims to hold it. A probe that reads the table would
/// allocate 64 MB for a file of 140 bytes - and a box that claims four
/// gigabytes would make it allocate four gigabytes. The length of the file
/// bounds the read: a box that claims more than the file holds states
/// nothing, and the probe must not spend the memory of the player on the
/// claim.
#[test]
fn a_box_that_claims_more_than_the_file_holds_does_not_make_the_probe_allocate_it() {
    let bytes = a_hostile_mp4();
    let mut reader = CountingReader::new(bytes.clone());
    let len = bytes.len() as u64;
    let header = mp4_header::parse(&mut reader, Some(len)).expect("a header");
    assert!(
        reader.largest <= bytes.len(),
        "the probe asked for {} bytes of a file that holds {}",
        reader.largest,
        bytes.len()
    );
    assert!(
        header.tracks().is_empty(),
        "a file that does not hold its sample table states no track"
    );
}

// ---------------------------------------------------------------------------
// The tests
// ---------------------------------------------------------------------------

#[test]
fn the_mpeg_header_probe_survives_the_malformed_corpus() {
    sweep("the MPEG header probe", probe_mpeg);
}

#[test]
fn the_aiff_header_probe_survives_the_malformed_corpus() {
    sweep("the AIFF header probe", probe_aiff);
}

#[test]
fn the_flac_header_probe_survives_the_malformed_corpus() {
    sweep("the FLAC header probe", probe_flac);
}

#[test]
fn the_mp4_header_probe_survives_the_malformed_corpus() {
    sweep("the MP4 header probe", probe_mp4);
}

#[test]
fn the_ogg_header_probe_survives_the_malformed_corpus() {
    sweep("the Ogg header probe", probe_ogg);
}

#[test]
fn the_opus_header_probe_survives_the_malformed_corpus() {
    sweep("the Opus header probe", probe_opus_header);
}

#[test]
fn the_opus_packet_probe_survives_the_malformed_corpus() {
    sweep("the Opus packet probe", probe_opus_packet);
}

#[test]
fn the_opus_decoder_survives_the_malformed_corpus() {
    // The codec reads the bytes of a packet that come from the file: a
    // malformed packet must be refused, not crash the decoder.
    let mut decoder = opus_decoder();
    sweep("the Opus decoder", |bytes| {
        let packet = Packet::new_from_slice(0, 0, 0, bytes);
        let _ = decoder.decode(&packet);
    });
}

#[test]
fn the_header_probe_survives_the_malformed_corpus() {
    sweep("the header probe", probe_header);
}

/// A decoder for a stereo Opus stream.
fn opus_decoder() -> opus_dec::OpusDecoder {
    let mut params = CodecParameters::new();
    params.for_codec(CODEC_TYPE_OPUS);
    params.with_sample_rate(48_000);
    params.with_channel_layout(symphonia::core::audio::Layout::Stereo);
    opus_dec::OpusDecoder::try_new(&params, &DecoderOptions::default())
        .expect("a decoder for a stereo Opus stream")
}
