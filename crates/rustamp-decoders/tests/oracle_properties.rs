//! Seeded property tests for the truncation oracles.
//!
//! Every container this crate reads states, in one way or another, what its
//! payload is. The decoders compare what they delivered with what the file
//! claims and call a file **truncated** when the two disagree. That verdict is
//! what a half-downloaded track needs, and it is the verdict that must never
//! be wrong in the other direction: a file that only *looks* damaged has to
//! keep playing.
//!
//! The properties below state the rule and check it over many files and many
//! truncation points:
//!
//! * a file is called truncated **exactly** when the conditions of its oracle
//!   hold - never more often, never less often,
//! * a corrupt but complete file is never called truncated (the "keeps
//!   playing" contract of `CONTRACT_NOTES.md`),
//! * a header that lies is never called truncated,
//! * a file that is shorter than its own header is a loud error of some kind,
//!   never a silent clean end,
//! * no prefix of a fixture panics.
//!
//! The files are built here, byte by byte, out of a seeded generator: the same
//! seed builds the same files on every run and on every machine, so a failure
//! is replayable. Nothing is downloaded, no test opens a window and no test
//! runs an example: the whole file is headless.

use std::io::Cursor;
use std::path::PathBuf;

use rustamp_core::audio::{Decoder, MediaSource};
use rustamp_decoders::{SymphoniaDecoder, WaveDecoder};
use symphonia::core::checksum::Crc32;
use symphonia::core::io::Monitor;

/// The sample rate of the synthetic files.
const RATE: u32 = 44_100;

// ---------------------------------------------------------------------------
// The seeded generator
// ---------------------------------------------------------------------------

/// The seed of the properties.
///
/// A fixed seed keeps the suite reproducible: the same files on every run and
/// on every machine, so a failure can be replayed from the seed alone.
const SEED: u64 = 0x5EED_0FC0_FFEE_0002;

/// How many random points every property walks.
const ROUNDS: usize = 64;

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

    /// A number in `0..n`.
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// A number in `low..=high`.
    fn between(&mut self, low: u64, high: u64) -> u64 {
        low + self.below(high - low + 1)
    }

    /// The next byte.
    fn byte(&mut self) -> u8 {
        (self.next_u64() >> 24) as u8
    }
}

/// A number of a name, so that every name gets its own part of the seed.
fn hash(name: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in name.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

// ---------------------------------------------------------------------------
// The verdict
// ---------------------------------------------------------------------------

/// What the decoder did with a file.
#[derive(Debug)]
enum Verdict {
    /// The stream ended cleanly: the file was not called truncated.
    Clean { frames: u64 },
    /// The file was refused when it was opened.
    Refused { message: String },
    /// The stream failed while it was read.
    Failed { frames: u64, message: String },
}

impl Verdict {
    /// Whether the file was called truncated.
    ///
    /// The decoders name a cut in their message, so the message is the
    /// verdict: a file is called truncated when the decoder says so, whether
    /// it said so when it was opened (an MP4 file) or at the end of the
    /// stream (every other format).
    fn is_truncated(&self) -> bool {
        match self {
            Verdict::Clean { .. } => false,
            Verdict::Refused { message } | Verdict::Failed { message, .. } => {
                message.contains("truncated")
            }
        }
    }

    /// Whether the stream ended cleanly.
    fn is_clean(&self) -> bool {
        matches!(self, Verdict::Clean { .. })
    }

    /// The frames the decoder delivered, when it delivered any.
    fn frames(&self) -> Option<u64> {
        match self {
            Verdict::Clean { frames } | Verdict::Failed { frames, .. } => Some(*frames),
            Verdict::Refused { .. } => None,
        }
    }
}

/// The number of `read` calls a drain may make before it is called a hang.
///
/// A decoder that never reaches an end of stream within this many calls is a
/// possible hang, and a hang inside a single `read` cannot be turned into a
/// failure from within the process.
const MAX_READS: usize = 100_000;

/// Reads a decoder to its end.
///
/// The frames are counted, so a property can tell a file that delivered
/// nothing from a file that delivered its audio.
fn drain(d: &mut dyn Decoder) -> Verdict {
    let ch = usize::from(d.spec().channels.max(1));
    let mut buf = vec![0.0f32; 4096 * ch];
    let mut frames = 0u64;
    for _ in 0..MAX_READS {
        match d.read(&mut buf) {
            Ok(0) => return Verdict::Clean { frames },
            Ok(n) => frames += n as u64,
            Err(e) => {
                return Verdict::Failed {
                    frames,
                    message: e.to_string(),
                };
            }
        }
    }
    panic!("no end of stream after {MAX_READS} reads: possible hang");
}

/// The verdict of the symphonia decoder about a file.
fn symphonia_verdict(bytes: Vec<u8>, ext: &str) -> Verdict {
    let source = MediaSource::Reader {
        reader: Box::new(Cursor::new(bytes)),
        extension: Some(ext.to_string()),
    };
    match SymphoniaDecoder::open(source) {
        Ok(mut d) => drain(&mut d),
        Err(e) => Verdict::Refused {
            message: e.to_string(),
        },
    }
}

/// The verdict of the wave decoder about a file.
fn wave_verdict(bytes: Vec<u8>) -> Verdict {
    let source = MediaSource::Reader {
        reader: Box::new(Cursor::new(bytes)),
        extension: Some("wav".to_string()),
    };
    match WaveDecoder::open(source) {
        Ok(mut d) => drain(&mut d),
        Err(e) => Verdict::Refused {
            message: e.to_string(),
        },
    }
}

/// Asserts that a file is called truncated exactly when the oracle says so.
#[track_caller]
fn assert_truncation(label: &str, verdict: &Verdict, expected: bool) {
    assert_eq!(
        verdict.is_truncated(),
        expected,
        "{label}: the verdict must match the conditions of the oracle ({verdict:?})"
    );
}
// ---------------------------------------------------------------------------
// The synthetic files
// ---------------------------------------------------------------------------

/// A file of the repository, read from the crate.
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// A RIFF/WAVE file with 16 bit stereo PCM.
///
/// The `data` chunk states the number of bytes of sample data, so the file
/// states its payload exactly. The samples are a quiet sine, so the file holds
/// real audio and not a run of zeros.
fn wav(frames: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0u32.to_le_bytes()); // the size of the form, patched below
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // the format tag: PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // channels
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 4).to_le_bytes()); // the byte rate
    out.extend_from_slice(&4u16.to_le_bytes()); // the block align
    out.extend_from_slice(&16u16.to_le_bytes()); // the bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(frames * 4).to_le_bytes());
    for i in 0..frames {
        let value = (i as i32 % 64) as i16 * 64;
        out.extend_from_slice(&value.to_le_bytes());
        out.extend_from_slice(&(-value).to_le_bytes());
    }
    let size = u32::try_from(out.len() - 8).expect("the form size fits");
    out[4..8].copy_from_slice(&size.to_be_bytes());
    out
}

/// The offset of the sample data of a RIFF/WAVE file of this crate.
const WAV_DATA_AT: usize = 44;

/// The number of bytes the `data` chunk of a RIFF/WAVE file states.
///
/// The chunk is located instead of computed, so the helper cannot be used on
/// a file that has no data chunk.
fn wav_data_size(bytes: &mut [u8], size: u32) {
    let at = bytes
        .windows(4)
        .position(|w| w == b"data")
        .expect("the data chunk")
        + 4;
    bytes[at..at + 4].copy_from_slice(&size.to_le_bytes());
}

/// The 80-bit IEEE 754 extended float an AIFF file stores a sample rate in.
///
/// The value is `2^(exponent - 16383) * mantissa / 2^63`, with the mantissa's
/// integer bit set.
fn extended_rate(rate: u32) -> [u8; 10] {
    let rate = rate.max(1);
    let exponent = 16383 + 31 - rate.leading_zeros();
    let mantissa = u64::from(rate) << (63 - (31 - rate.leading_zeros()));
    let mut out = [0u8; 10];
    out[..2].copy_from_slice(&(exponent as u16).to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// An AIFF file: `FORM`/`AIFF` with a `COMM` chunk and an `SSND` chunk.
///
/// `frames` is what the `COMM` chunk states, `sound_bytes` what the `SSND`
/// chunk states and `held` how many bytes of sound data the file really
/// holds. The three are independent on purpose: the oracle compares the
/// frames the file delivers with the first and the bytes it holds with the
/// second, so a file has to be tested in all four combinations of the two.
fn aiff(frames: u32, sound_bytes: u64, held: usize) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"FORM");
    out.extend_from_slice(&0u32.to_be_bytes()); // the size of the form, patched below
    out.extend_from_slice(b"AIFF");
    out.extend_from_slice(b"COMM");
    out.extend_from_slice(&18u32.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes()); // channels
    out.extend_from_slice(&frames.to_be_bytes()); // sample frames
    out.extend_from_slice(&16u16.to_be_bytes()); // bits per sample
    out.extend_from_slice(&extended_rate(RATE));
    out.extend_from_slice(b"SSND");
    // The size of the chunk covers its own eight byte header next to the
    // sound data.
    let ssnd_len = u32::try_from(sound_bytes + 8).expect("the chunk size fits");
    out.extend_from_slice(&ssnd_len.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes()); // the offset
    out.extend_from_slice(&0u32.to_be_bytes()); // the block size
    out.resize(out.len() + held, 0);
    // The size of the form states the form as it was written, not as it is
    // now: a file that was cut states more than it holds, and that is the
    // claim the reader has to keep.
    let size = u32::try_from(46 + sound_bytes).expect("the form size fits");
    out[4..8].copy_from_slice(&size.to_be_bytes());
    out
}

/// The offset of the sound data of an AIFF file of this crate.
///
/// The sound data starts behind the header of the `SSND` chunk, which holds
/// the offset and the block size and is not audio.
const AIFF_DATA_AT: usize = 54;

/// A box of an ISO base media file: its type and its payload.
fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

/// A box that starts with a version and flags.
fn mp4_full_box(kind: &[u8; 4], flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.push(0u8);
    body.extend_from_slice(&flags.to_be_bytes()[1..]);
    body.extend_from_slice(payload);
    mp4_box(kind, &body)
}

/// A minimal MP4 file with uncompressed 16 bit stereo PCM in a media data
/// box.
///
/// The movie box comes first, so the file is the "fast start" layout: the
/// sample table is at the front and the media data behind it. The sample
/// tables state one sample per audio frame, so the media header states the
/// number of frames the file holds.
///
/// `frames` is the number of samples the sample tables state (and the number
/// the container reader reports), `mdhd_frames` what the media header states
/// and `mdat_bytes` what the media data box states. `held` is how many bytes
/// of media data the file really holds.
fn mp4(frames: u32, mdhd_frames: u32, mdat_bytes: u64, held: usize) -> Vec<u8> {
    let movie_timescale = 1000u32;
    let movie_duration = frames * movie_timescale / RATE;

    // The movie header.
    let mut mvhd = Vec::new();
    mvhd.extend_from_slice(&0u32.to_be_bytes()); // creation time
    mvhd.extend_from_slice(&0u32.to_be_bytes()); // modification time
    mvhd.extend_from_slice(&movie_timescale.to_be_bytes());
    mvhd.extend_from_slice(&movie_duration.to_be_bytes());
    mvhd.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // preferred rate
    mvhd.extend_from_slice(&0x0100u16.to_be_bytes()); // preferred volume
    mvhd.extend_from_slice(&[0u8; 2]); // reserved
    mvhd.extend_from_slice(&[0u8; 8]); // reserved
    mvhd.extend_from_slice(&[0u8; 36]); // transformation matrix
    mvhd.extend_from_slice(&[0u8; 24]); // predefined
    mvhd.extend_from_slice(&2u32.to_be_bytes()); // next track id
    let mvhd = mp4_full_box(b"mvhd", 0, &mvhd);

    // The track header.
    let mut tkhd = Vec::new();
    tkhd.extend_from_slice(&0u32.to_be_bytes()); // creation time
    tkhd.extend_from_slice(&0u32.to_be_bytes()); // modification time
    tkhd.extend_from_slice(&1u32.to_be_bytes()); // track id
    tkhd.extend_from_slice(&0u32.to_be_bytes()); // reserved
    tkhd.extend_from_slice(&movie_duration.to_be_bytes());
    tkhd.extend_from_slice(&[0u8; 8]); // reserved
    tkhd.extend_from_slice(&0u16.to_be_bytes()); // layer
    tkhd.extend_from_slice(&0u16.to_be_bytes()); // alternate group
    tkhd.extend_from_slice(&0x0100u16.to_be_bytes()); // volume
    tkhd.extend_from_slice(&[0u8; 2]); // reserved
    tkhd.extend_from_slice(&[0u8; 36]); // transformation matrix
    tkhd.extend_from_slice(&0u32.to_be_bytes()); // width
    tkhd.extend_from_slice(&0u32.to_be_bytes()); // height
    let tkhd = mp4_full_box(b"tkhd", 3, &tkhd);

    // The media header: the timescale is the sample rate, so the duration is
    // the number of audio frames.
    let mut mdhd = Vec::new();
    mdhd.extend_from_slice(&0u32.to_be_bytes()); // creation time
    mdhd.extend_from_slice(&0u32.to_be_bytes()); // modification time
    mdhd.extend_from_slice(&RATE.to_be_bytes()); // timescale
    mdhd.extend_from_slice(&mdhd_frames.to_be_bytes()); // duration
    mdhd.extend_from_slice(&0x55C4u16.to_be_bytes()); // language
    mdhd.extend_from_slice(&0u16.to_be_bytes()); // quality
    let mdhd = mp4_full_box(b"mdhd", 0, &mdhd);

    // The handler reference: the track holds sound.
    let mut hdlr = Vec::new();
    hdlr.extend_from_slice(&0u32.to_be_bytes()); // predefined
    hdlr.extend_from_slice(b"soun");
    hdlr.extend_from_slice(&[0u8; 12]); // reserved
    hdlr.extend_from_slice(b"\0"); // name
    let hdlr = mp4_full_box(b"hdlr", 0, &hdlr);

    // The sound media header.
    let smhd = mp4_full_box(b"smhd", 0, &[0u8; 4]);

    // The data information: the samples are in the file itself.
    let url = mp4_full_box(b"url ", 1, &[]);
    let mut dref = Vec::new();
    dref.extend_from_slice(&1u32.to_be_bytes());
    dref.extend_from_slice(&url);
    let dinf = mp4_box(b"dinf", &mp4_full_box(b"dref", 0, &dref));

    // The sample description: a `sowt` entry, 16 bit little endian PCM.
    let mut sowt = Vec::new();
    sowt.extend_from_slice(&[0u8; 6]); // reserved
    sowt.extend_from_slice(&1u16.to_le_bytes()); // data reference index
    sowt.extend_from_slice(&0u16.to_le_bytes()); // version
    sowt.extend_from_slice(&0u16.to_le_bytes()); // revision
    sowt.extend_from_slice(&0u32.to_le_bytes()); // vendor
    sowt.extend_from_slice(&2u16.to_be_bytes()); // channels
    sowt.extend_from_slice(&16u16.to_be_bytes()); // sample size
    sowt.extend_from_slice(&0u16.to_le_bytes()); // compression id
    sowt.extend_from_slice(&0u16.to_le_bytes()); // packet size
    sowt.extend_from_slice(&(RATE << 16).to_be_bytes()); // sample rate (16.16)
    let mut stsd = Vec::new();
    stsd.extend_from_slice(&1u32.to_be_bytes()); // entry count
    stsd.extend_from_slice(&mp4_box(b"sowt", &sowt));
    let stsd = mp4_full_box(b"stsd", 0, &stsd);

    // The time to sample table: one sample per frame.
    let mut stts = Vec::new();
    stts.extend_from_slice(&1u32.to_be_bytes()); // entry count
    stts.extend_from_slice(&frames.to_be_bytes()); // sample count
    stts.extend_from_slice(&1u32.to_be_bytes()); // sample delta
    let stts = mp4_full_box(b"stts", 0, &stts);

    // The sample to chunk table: all samples in one chunk.
    let mut stsc = Vec::new();
    stsc.extend_from_slice(&1u32.to_be_bytes()); // entry count
    stsc.extend_from_slice(&1u32.to_be_bytes()); // first chunk
    stsc.extend_from_slice(&frames.to_be_bytes()); // samples per chunk
    stsc.extend_from_slice(&1u32.to_be_bytes()); // sample description index
    let stsc = mp4_full_box(b"stsc", 0, &stsc);

    // The sample size table: every sample is one stereo frame.
    let mut stsz = Vec::new();
    stsz.extend_from_slice(&4u32.to_be_bytes()); // sample size
    stsz.extend_from_slice(&frames.to_be_bytes()); // sample count
    let stsz = mp4_full_box(b"stsz", 0, &stsz);

    // The chunk offset table: the offset is patched once the file is laid out.
    let mut stco = Vec::new();
    stco.extend_from_slice(&1u32.to_be_bytes()); // entry count
    stco.extend_from_slice(&0u32.to_be_bytes()); // chunk offset
    let stco = mp4_full_box(b"stco", 0, &stco);

    let stbl = mp4_box(b"stbl", &[stsd, stts, stsc, stsz, stco].concat());
    let minf = mp4_box(b"minf", &[smhd, dinf, stbl].concat());
    let mdia = mp4_box(b"mdia", &[mdhd, hdlr, minf].concat());
    let trak = mp4_box(b"trak", &[tkhd, mdia].concat());
    let moov = mp4_box(b"moov", &[mvhd, trak].concat());

    let mut ftyp = Vec::new();
    ftyp.extend_from_slice(b"M4A ");
    ftyp.extend_from_slice(&0x200u32.to_be_bytes()); // minor version
    ftyp.extend_from_slice(b"M4A ");
    ftyp.extend_from_slice(b"isom");
    let ftyp = mp4_box(b"ftyp", &ftyp);

    // The media data: one stereo frame per sample.
    let mut mdat = Vec::new();
    for i in 0..frames {
        let v = (i as i16).to_le_bytes();
        mdat.extend_from_slice(&v);
        mdat.extend_from_slice(&v);
    }
    let mdat = mp4_box(b"mdat", &mdat);

    let mut file = Vec::new();
    file.extend_from_slice(&ftyp);
    file.extend_from_slice(&moov);
    let mdat_at = file.len();
    // The media data box claims more than the file holds when the file was
    // cut: the box is written with the claim and the payload is cut short.
    let mut mdat = mdat;
    let claimed = u32::try_from(mdat_bytes + 8).expect("the box size fits");
    mdat[0..4].copy_from_slice(&claimed.to_be_bytes());
    file.extend_from_slice(&mdat[..8]);
    file.resize(file.len() + held, 0);
    // The first sample starts behind the header of the media data box.
    let offset = (mdat_at + 8) as u32;
    let at = mp4_offset_box(&file, b"stco");
    file[at..at + 4].copy_from_slice(&offset.to_be_bytes());
    file
}

/// The offset of the first payload byte of the first box of the given type.
fn mp4_offset_box(bytes: &[u8], kind: &[u8; 4]) -> usize {
    let at = bytes
        .windows(4)
        .position(|w| w == kind)
        .unwrap_or_else(|| panic!("the {kind:?} box"));
    // The box header is eight bytes, the version and flags four: the payload
    // starts behind them.
    at + 8 + 4
}

/// The bitrates of an MPEG 1 layer III frame, in kbit/s.
///
/// The index is the bitrate field of the frame header; the first and the last
/// value are the "free" and the invalid one.
const MP3_BITRATES: [u32; 16] = [
    0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
];

/// The sample rates of an MPEG 1 frame, in Hz.
const MP3_RATES: [u32; 4] = [44100, 48000, 32000, 0];

/// The length of the MPEG frame at `at`.
///
/// `None` when the bytes at `at` are not the header of an MPEG 1 layer III
/// frame.
fn mp3_frame_len(bytes: &[u8], at: usize) -> Option<usize> {
    let head = bytes.get(at..at + 4)?;
    if head[0] != 0xFF || (head[1] & 0xE0) != 0xE0 {
        return None;
    }
    let version = (head[1] >> 3) & 0x03;
    let layer = (head[1] >> 1) & 0x03;
    if version != 0b11 || layer != 0b01 {
        return None;
    }
    let bitrate = MP3_BITRATES[usize::from((head[2] >> 4) & 0x0F)] * 1000;
    let rate = MP3_RATES[usize::from((head[2] >> 2) & 0x03)];
    if bitrate == 0 || rate == 0 {
        return None;
    }
    let padding = u32::from((head[2] >> 1) & 0x01);
    Some((144 * bitrate / rate + padding) as usize)
}

/// The offset of the first MPEG frame of a file, behind its ID3v2 tag.
fn mp3_audio_at(bytes: &[u8]) -> usize {
    if !bytes.starts_with(b"ID3") {
        return 0;
    }
    // The size of an ID3v2 tag is stored "syncsafe": seven bits per byte.
    let size = (u32::from(bytes[6] & 0x7F) << 21)
        | (u32::from(bytes[7] & 0x7F) << 14)
        | (u32::from(bytes[8] & 0x7F) << 7)
        | u32::from(bytes[9] & 0x7F);
    10 + size as usize
}

/// The audio frames of the MPEG fixture: the frames behind the first one.
///
/// The first frame of the fixture carries the Xing/Info tag; the frames
/// behind it are the audio. They are the payload of the synthetic files the
/// properties build: real audio under a header this suite writes itself, so
/// that the claims of the header are the claims of the test.
fn mp3_audio_frames() -> Vec<u8> {
    let bytes = std::fs::read(fixture("sine.mp3")).expect("the MPEG fixture");
    let at = mp3_audio_at(&bytes);
    let first = mp3_frame_len(&bytes, at).expect("the fixture starts with a frame");
    bytes[at + first..].to_vec()
}

/// The number of MPEG frames the audio frames of the fixture hold.
fn mp3_audio_frame_count() -> u32 {
    let bytes = std::fs::read(fixture("sine.mp3")).expect("the MPEG fixture");
    let mut at = mp3_audio_at(&bytes);
    let first = mp3_frame_len(&bytes, at).expect("the fixture starts with a frame");
    at += first;
    let mut count = 0u32;
    while let Some(len) = mp3_frame_len(&bytes, at) {
        at += len;
        count += 1;
    }
    count
}

/// The length of the frame that carries the tag of a synthetic MPEG file.
///
/// The frame is an MPEG 1 layer III frame at 64 kbit/s and 44.1 kHz: 144
/// samples times the bit rate, divided by the sample rate.
const MP3_TAG_FRAME_LEN: usize = 144 * 64_000 / 44_100;

/// An MPEG audio file: a first frame with a Xing/Info tag and the audio
/// frames of the fixture.
///
/// `frames` is the number of MPEG frames the tag declares (the frame that
/// carries the tag is not counted: it holds the tag, not audio) and `bytes`
/// the number of bytes of audio the tag declares.
fn mpeg(frames: u32, bytes: u32) -> Vec<u8> {
    let mut out = Vec::new();
    // The frame header: MPEG 1 layer III, 64 kbit/s, 44.1 kHz, stereo, and
    // no CRC - the tag sits behind the side information either way.
    out.extend_from_slice(&[0xFF, 0xFB, 0x50, 0x00]);
    // The side information has to be zeroed for the tag to be found.
    out.extend_from_slice(&[0u8; 32]);
    out.extend_from_slice(b"Xing");
    // The flags: a frame count and a byte count. Both are needed - a frame
    // count alone cannot tell a cut file from a damaged one.
    out.extend_from_slice(&3u32.to_be_bytes());
    out.extend_from_slice(&frames.to_be_bytes());
    out.extend_from_slice(&bytes.to_be_bytes());
    // The frame is as long as its header says.
    out.resize(MP3_TAG_FRAME_LEN, 0);
    out.extend_from_slice(&mp3_audio_frames());
    out
}

/// One page of an Ogg file.
#[derive(Clone)]
struct Page {
    /// The header type flags.
    flags: u8,
    /// The absolute granule position.
    granule: u64,
    /// The serial number of the logical bitstream.
    serial: u32,
    /// The page sequence number.
    sequence: u32,
    /// The segment table.
    table: Vec<u8>,
    /// The body the segment table states.
    body: Vec<u8>,
}

/// The pages of the Ogg fixture.
///
/// The pages are read out of the file and re-emitted byte by byte, so a test
/// can change a flag or cut a page without touching the audio.
fn ogg_pages() -> Vec<Page> {
    let bytes = std::fs::read(fixture("sine.ogg")).expect("the Ogg fixture");
    let mut pages = Vec::new();
    let mut at = 0usize;
    while at + 27 <= bytes.len() {
        assert_eq!(&bytes[at..at + 4], b"OggS", "a page starts at {at}");
        let segments = usize::from(bytes[at + 26]);
        let table_at = at + 27;
        let body_at = table_at + segments;
        let body_len: usize = bytes[table_at..body_at]
            .iter()
            .map(|&lacing| usize::from(lacing))
            .sum();
        let end = body_at + body_len;
        pages.push(Page {
            flags: bytes[at + 5],
            granule: u64::from_le_bytes(bytes[at + 6..at + 14].try_into().expect("eight bytes")),
            serial: u32::from_le_bytes(bytes[at + 14..at + 18].try_into().expect("four bytes")),
            sequence: u32::from_le_bytes(bytes[at + 18..at + 22].try_into().expect("four bytes")),
            table: bytes[table_at..body_at].to_vec(),
            body: bytes[body_at..end].to_vec(),
        });
        at = end;
    }
    assert_eq!(at, bytes.len(), "the file ends at the end of a page");
    pages
}

/// The file the pages make.
///
/// Every page is written byte by byte and gets the checksum of its own
/// content, so the file is a valid Ogg file whatever the flags say.
fn ogg_file(pages: &[Page], end_of_stream: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        let last = i + 1 == pages.len();
        let flags = if last {
            if end_of_stream {
                page.flags | 0x04
            } else {
                page.flags & !0x04
            }
        } else {
            page.flags
        };
        let mut head = [0u8; 27];
        head[..4].copy_from_slice(b"OggS");
        head[4] = 0; // the stream structure version
        head[5] = flags;
        head[6..14].copy_from_slice(&page.granule.to_le_bytes());
        head[14..18].copy_from_slice(&page.serial.to_le_bytes());
        head[18..22].copy_from_slice(&page.sequence.to_le_bytes());
        head[26] = u8::try_from(page.table.len()).expect("the segment table fits");
        // The checksum covers the header with its own field zeroed, the
        // segment table and the body.
        let mut crc = Crc32::new(0);
        crc.process_buf_bytes(&head);
        crc.process_buf_bytes(&page.table);
        crc.process_buf_bytes(&page.body);
        head[22..26].copy_from_slice(&crc.crc().to_le_bytes());
        out.extend_from_slice(&head);
        out.extend_from_slice(&page.table);
        out.extend_from_slice(&page.body);
    }
    out
}

// ---------------------------------------------------------------------------
// The cases of an oracle
// ---------------------------------------------------------------------------

/// A file and what the oracle has to say about it.
struct Case {
    /// What the case is, for the failure message.
    label: &'static str,
    /// The file.
    bytes: Vec<u8>,
    /// The extension the file is opened with.
    ext: &'static str,
    /// Whether the file is to be called truncated.
    truncated: bool,
}

/// Checks the cases of one oracle.
///
/// The cases are the truth table of the oracle: every combination of its
/// conditions has to land on the right side of it.
#[track_caller]
fn assert_cases(cases: &[Case]) {
    for case in cases {
        let verdict = symphonia_verdict(case.bytes.clone(), case.ext);
        assert_eq!(
            verdict.is_truncated(),
            case.truncated,
            "{}: the verdict must match the conditions of the oracle ({verdict:?})",
            case.label
        );
    }
}

/// Damages bytes in a range, so that the file keeps its length.
///
/// A damaged byte is what damage looks like: the file is complete, but its
/// content is not what it was.
fn damage(bytes: &mut [u8], from: usize, to: usize, rng: &mut Rng) {
    for b in &mut bytes[from..to] {
        *b = rng.byte();
    }
}

/// The offset of the payload of the media data box of an MP4 file.
fn mp4_media_at(bytes: &[u8]) -> usize {
    let at = bytes
        .windows(4)
        .position(|w| w == b"mdat")
        .expect("the media data box");
    // The type of the box is four bytes; the payload starts behind it.
    at + 4
}

// ---------------------------------------------------------------------------
// RIFF/WAVE: the `data` chunk is the oracle
// ---------------------------------------------------------------------------

/// The WAVE oracle: the `data` chunk states the number of bytes of sample
/// data, and a damaged sample is still a sample.
///
/// A file that holds fewer bytes than the chunk declares was cut; a file that
/// holds them all was not, no matter how its samples look. There is one
/// condition here - the frames of a PCM file follow from its bytes - so the
/// file is called truncated exactly when it is short of bytes.
#[test]
fn a_cut_wav_is_called_truncated_exactly_when_the_data_chunk_is_short() {
    const FRAMES: u32 = 4000;
    let full = wav(FRAMES);
    let declared = FRAMES as usize * 4;

    // The complete file holds everything its chunk declares, and it delivers
    // every frame it holds.
    let verdict = symphonia_verdict(full.clone(), "wav");
    assert!(!verdict.is_truncated());
    assert_eq!(verdict.frames(), Some(u64::from(FRAMES)));
    let verdict = wave_verdict(full.clone());
    assert!(!verdict.is_truncated());
    assert_eq!(verdict.frames(), Some(u64::from(FRAMES)));

    // A chunk that claims less than the file holds: the file holds more than
    // it claims, and the extra bytes are not audio the file promised. It is
    // not a cut file.
    let mut lying = full.clone();
    wav_data_size(&mut lying, 1000);
    let verdict = symphonia_verdict(lying.clone(), "wav");
    assert!(
        !verdict.is_truncated(),
        "a wav that holds more than it claims: {verdict:?}"
    );
    assert_eq!(
        verdict.frames(),
        Some(250),
        "only the claimed bytes are audio"
    );
    let verdict = wave_verdict(lying);
    assert!(!verdict.is_truncated());
    assert_eq!(verdict.frames(), Some(250));

    let mut rng = Rng::new(SEED ^ hash("wav"));
    for _ in 0..ROUNDS {
        let cut = rng.between(WAV_DATA_AT as u64, full.len() as u64) as usize;
        let bytes = full[..cut].to_vec();
        let expected = cut - WAV_DATA_AT < declared;
        assert_truncation(
            &format!("the symphonia decoder, a wav cut at {cut} bytes"),
            &symphonia_verdict(bytes.clone(), "wav"),
            expected,
        );
        assert_truncation(
            &format!("the wave decoder, a wav cut at {cut} bytes"),
            &wave_verdict(bytes),
            expected,
        );
    }
}

/// A WAVE file whose samples are damaged is not a cut file.
///
/// The bytes are all there: the file holds everything its chunk declares, so
/// it keeps playing. This is the "keeps playing" contract of
/// `CONTRACT_NOTES.md` as a property.
#[test]
fn a_damaged_wav_is_never_called_truncated() {
    let mut rng = Rng::new(SEED ^ hash("wav-damage"));
    for _ in 0..ROUNDS {
        let mut bytes = wav(4000);
        let from = WAV_DATA_AT + rng.below(4000) as usize;
        let to = (from + rng.between(1, 400) as usize).min(WAV_DATA_AT + 16_000);
        damage(&mut bytes, from, to, &mut rng);
        assert_truncation(
            "the symphonia decoder",
            &symphonia_verdict(bytes.clone(), "wav"),
            false,
        );
        assert_truncation("the wave decoder", &wave_verdict(bytes), false);
    }
}

// ---------------------------------------------------------------------------
// AIFF: the `COMM` and `SSND` chunks are the oracle
// ---------------------------------------------------------------------------

/// The AIFF oracle: the `COMM` chunk states the number of sample frames and
/// the `SSND` chunk the number of bytes of sound data.
///
/// A file that is short of frames *and* of bytes was cut. A file that is short
/// of one of them only is not: a damaged sample is still a sample, so a
/// shortfall of frames alone is damage, and a byte count that is too large is
/// a header that lies.
#[test]
fn an_aiff_is_called_truncated_exactly_when_its_frames_and_its_bytes_are_short() {
    const FRAMES: u32 = 1000;
    const SOUND: u64 = 2000;
    let complete = aiff(FRAMES, SOUND, 2000);

    assert_cases(&[
        // Both short: the file was cut.
        Case {
            label: "an aiff that is short of frames and of bytes",
            bytes: aiff(FRAMES, SOUND, 1000),
            ext: "aiff",
            truncated: true,
        },
        // The frames are short, the bytes are all there: the file holds
        // everything it claims of its bytes, so the `COMM` chunk is the one
        // that lies.
        Case {
            label: "an aiff whose COMM chunk claims too many frames",
            bytes: aiff(1500, SOUND, 2000),
            ext: "aiff",
            truncated: false,
        },
        // The frames are all there, the bytes are short: the audio is
        // complete, so the `SSND` chunk is the one that lies.
        Case {
            label: "an aiff whose SSND chunk claims too many bytes",
            bytes: aiff(500, SOUND, 1000),
            ext: "aiff",
            truncated: false,
        },
        // Neither is short: the file is complete.
        Case {
            label: "a complete aiff",
            bytes: complete.clone(),
            ext: "aiff",
            truncated: false,
        },
    ]);

    // A cut at any point of the sound data is a cut of both.
    let mut rng = Rng::new(SEED ^ hash("aiff"));
    for _ in 0..ROUNDS {
        let at = rng.between(AIFF_DATA_AT as u64, complete.len() as u64) as usize;
        assert_truncation(
            "a cut aiff",
            &symphonia_verdict(complete[..at].to_vec(), "aiff"),
            true,
        );
    }

    // The complete file delivers every frame its `COMM` chunk declares.
    let verdict = symphonia_verdict(complete, "aiff");
    assert_eq!(
        verdict.frames(),
        Some(u64::from(FRAMES)),
        "the file holds every frame it declares"
    );
}

/// An AIFF file whose samples are damaged is not a cut file.
///
/// The bytes are all there, so the file keeps playing.
#[test]
fn a_damaged_aiff_is_never_called_truncated() {
    let mut rng = Rng::new(SEED ^ hash("aiff-damage"));
    for _ in 0..ROUNDS {
        let mut bytes = aiff(1000, 2000, 2000);
        let from = AIFF_DATA_AT + rng.below(2000) as usize;
        let to = (from + rng.between(1, 300) as usize).min(AIFF_DATA_AT + 2000);
        damage(&mut bytes, from, to, &mut rng);
        assert_truncation("a damaged aiff", &symphonia_verdict(bytes, "aiff"), false);
    }
}

// ---------------------------------------------------------------------------
// MP4/M4A: the boxes are the oracle
// ---------------------------------------------------------------------------

/// The MP4 oracle: the media data box states the number of bytes of media
/// data, and the media header together with the sample tables state the
/// number of frames and the bytes of the audio.
///
/// The claim of the media data box survives the loss of the movie box, so it
/// is the one that names a cut file - and it names it when the file is
/// opened, because the container reader scans the whole file before it plays
/// anything.
#[test]
fn an_mp4_is_called_truncated_exactly_when_the_file_is_short_of_the_bytes_its_boxes_claim() {
    const FRAMES: u32 = 1000;
    const MEDIA: u64 = 4000;
    let complete = mp4(FRAMES, FRAMES, MEDIA, 4000);

    assert_cases(&[
        // The file is short of the media data box: it was cut.
        Case {
            label: "an mp4 that is short of its media data box",
            bytes: mp4(FRAMES, FRAMES, MEDIA, 2000),
            ext: "m4a",
            truncated: true,
        },
        // The media header claims more frames than the file delivers, but the
        // file holds every byte its boxes claim: the header lies, the file
        // is not cut.
        Case {
            label: "an mp4 whose media header claims too many frames",
            bytes: mp4(FRAMES, 2 * FRAMES, MEDIA, 4000),
            ext: "m4a",
            truncated: false,
        },
        // The complete file.
        Case {
            label: "a complete mp4",
            bytes: complete.clone(),
            ext: "m4a",
            truncated: false,
        },
    ]);

    // A sample table that points behind the end of the file: the samples are
    // not there, so the file was cut even though its boxes are all there.
    let mut beyond = mp4(FRAMES, FRAMES, MEDIA, 4000);
    let at = mp4_offset_box(&beyond, b"stco");
    let offset = u32::from_be_bytes(beyond[at..at + 4].try_into().expect("four bytes"));
    beyond[at..at + 4].copy_from_slice(&(offset + 100_000).to_be_bytes());
    assert_truncation(
        "an mp4 whose sample table points beyond the file",
        &symphonia_verdict(beyond, "m4a"),
        true,
    );

    // A cut at any point of the media data is a cut of the media data box.
    let media_at = mp4_media_at(&complete);
    let mut rng = Rng::new(SEED ^ hash("mp4"));
    for _ in 0..ROUNDS {
        let at = rng.between(media_at as u64, complete.len() as u64) as usize;
        assert_truncation(
            "a cut mp4",
            &symphonia_verdict(complete[..at].to_vec(), "m4a"),
            true,
        );
    }

    // The complete file delivers every frame its media header declares.
    let verdict = symphonia_verdict(complete, "m4a");
    assert_eq!(
        verdict.frames(),
        Some(u64::from(FRAMES)),
        "the file holds every frame it declares"
    );
}

/// An MP4 file whose samples are damaged is not a cut file.
///
/// The bytes are all there, so the file keeps playing.
#[test]
fn a_damaged_mp4_is_never_called_truncated() {
    let mut rng = Rng::new(SEED ^ hash("mp4-damage"));
    for _ in 0..ROUNDS {
        let mut bytes = mp4(1000, 1000, 4000, 4000);
        let media_at = mp4_media_at(&bytes);
        let from = media_at + rng.below(4000) as usize;
        let to = (from + rng.between(1, 300) as usize).min(media_at + 4000);
        damage(&mut bytes, from, to, &mut rng);
        assert_truncation("a damaged mp4", &symphonia_verdict(bytes, "m4a"), false);
    }
}

// ---------------------------------------------------------------------------
// MPEG audio: the Xing/Info header is the oracle
// ---------------------------------------------------------------------------

/// The MPEG audio oracle: the Xing/Info tag states the number of MPEG frames
/// *and* the number of bytes the audio occupies.
///
/// Both are needed. A damaged MPEG frame is resynchronised by the reader, so
/// a damaged file delivers fewer frames than it declares as well - but it
/// holds all of its bytes. A file that is short of frames *and* of bytes was
/// cut; a file that is short of one of them only is not.
#[test]
fn an_mp3_is_called_truncated_exactly_when_its_frames_and_its_bytes_are_short() {
    let frames = mp3_audio_frame_count();
    let bytes = u32::try_from(MP3_TAG_FRAME_LEN + mp3_audio_frames().len()).expect("the size fits");
    let complete = mpeg(frames, bytes);

    assert_cases(&[
        // Both short: the file was cut.
        Case {
            label: "an mp3 that is short of frames and of bytes",
            bytes: complete[..complete.len() - 1000].to_vec(),
            ext: "mp3",
            truncated: true,
        },
        // The frames are short, the bytes are all there: the header lies.
        Case {
            label: "an mp3 whose Xing header claims too many frames",
            bytes: mpeg(frames + 10, bytes),
            ext: "mp3",
            truncated: false,
        },
        // The frames are all there, the bytes are short: the header lies.
        Case {
            label: "an mp3 whose Xing header claims too many bytes",
            bytes: mpeg(frames, bytes + 5000),
            ext: "mp3",
            truncated: false,
        },
        // The complete file.
        Case {
            label: "a complete mp3",
            bytes: complete.clone(),
            ext: "mp3",
            truncated: false,
        },
    ]);

    // A cut at any point of the audio is a cut of both.
    let mut rng = Rng::new(SEED ^ hash("mpeg"));
    for _ in 0..ROUNDS {
        let at = rng.between(MP3_TAG_FRAME_LEN as u64, complete.len() as u64) as usize;
        assert_truncation(
            "a cut mp3",
            &symphonia_verdict(complete[..at].to_vec(), "mp3"),
            true,
        );
    }

    // The complete file delivers every sample its tag declares.
    let verdict = symphonia_verdict(complete, "mp3");
    assert_eq!(
        verdict.frames(),
        Some(u64::from(frames) * 1152),
        "the file holds every frame it declares"
    );
}

/// An MP3 file whose audio is damaged is not a cut file.
///
/// The damaged frames cost frames, not bytes: the file holds every byte its
/// tag declares, so it keeps playing.
#[test]
fn a_damaged_mp3_is_never_called_truncated() {
    let mut rng = Rng::new(SEED ^ hash("mpeg-damage"));
    for _ in 0..ROUNDS {
        let mut bytes = mpeg(mp3_audio_frame_count(), honest_mp3_bytes());
        let from = MP3_TAG_FRAME_LEN + rng.below(8000) as usize;
        let to = (from + rng.between(1, 400) as usize).min(bytes.len());
        damage(&mut bytes, from, to, &mut rng);
        assert_truncation("a damaged mp3", &symphonia_verdict(bytes, "mp3"), false);
    }
}

/// The number of bytes the Xing/Info tag of the complete file states.
fn honest_mp3_bytes() -> u32 {
    u32::try_from(MP3_TAG_FRAME_LEN + mp3_audio_frames().len()).expect("the size fits")
}

// ---------------------------------------------------------------------------
// Ogg: the end-of-stream flag is the oracle
// ---------------------------------------------------------------------------

/// The Ogg oracle: the last page of a logical bitstream carries the
/// end-of-stream flag.
///
/// A file whose last page is intact and does not carry the flag was cut - the
/// bitstream itself says that more is to come. A file that ends inside a page
/// was cut as well. Damage is not a cut: a page whose checksum does not match
/// cannot be read, so nothing is claimed about the file.
#[test]
fn an_ogg_is_called_truncated_exactly_when_its_last_page_says_the_stream_goes_on() {
    let pages = ogg_pages();
    let complete = ogg_file(&pages, true);

    // The intact file: the last page carries the flag.
    assert_cases(&[Case {
        label: "a complete ogg",
        bytes: complete.clone(),
        ext: "ogg",
        truncated: false,
    }]);
    // It delivers every sample its end-of-stream page states.
    let verdict = symphonia_verdict(complete.clone(), "ogg");
    assert_eq!(
        verdict.frames(),
        Some(66_150),
        "the file holds every sample it declares"
    );

    // The same file without the flag: the bitstream says that more is to
    // come, so the file was cut.
    assert_cases(&[Case {
        label: "an ogg whose last page has no end-of-stream flag",
        bytes: ogg_file(&pages, false),
        ext: "ogg",
        truncated: true,
    }]);

    // A file that ends inside a page: an intact file always ends at the
    // boundary of a page, so it was cut.
    let cut = complete[..complete.len() - 30].to_vec();
    assert_cases(&[Case {
        label: "an ogg that ends inside a page",
        bytes: cut,
        ext: "ogg",
        truncated: true,
    }]);
}

/// An Ogg file with a damaged page is not a cut file.
///
/// The page that would say "the stream ends here" cannot be read, so nothing
/// is claimed about the file: it keeps playing.
#[test]
fn a_damaged_ogg_is_never_called_truncated() {
    let pages = ogg_pages();
    let mut rng = Rng::new(SEED ^ hash("ogg-damage"));
    for _ in 0..ROUNDS {
        let mut bytes = ogg_file(&pages, true);
        // A byte of the body of a page: the checksum of the page does not
        // match any more.
        let at = 27 + pages[0].table.len() + rng.below(64) as usize;
        bytes[at] ^= 0xA5;
        assert_truncation("a damaged ogg", &symphonia_verdict(bytes, "ogg"), false);
    }
}

/// The offset of the audio of a FLAC file: behind its metadata chain.
///
/// A FLAC file is the `fLaC` marker, then a chain of metadata blocks (a type
/// byte, a 24 bit length and the payload), then the audio frames. The chain
/// states the length of every block, so it says where the audio starts.
fn flac_audio_at(bytes: &[u8]) -> usize {
    let mut at = 4;
    loop {
        let header = *bytes.get(at).expect("a metadata block header");
        let len = (usize::from(bytes[at + 1]) << 16)
            | (usize::from(bytes[at + 2]) << 8)
            | usize::from(bytes[at + 3]);
        at += 4 + len;
        // The high bit of the type byte marks the last block of the chain.
        if header & 0x80 != 0 {
            return at;
        }
    }
}

/// A cut FLAC file is not called truncated: the format has no oracle.
///
/// The `STREAMINFO` block states the number of samples, but the format states
/// no byte count of the audio, and a damaged FLAC frame is dropped by the
/// reader - so a corrupt file is short of samples exactly like a cut one.
/// The two cannot be told apart, so a cut file keeps the honest end of
/// stream. The *metadata* of a FLAC file is a different matter: the chain
/// states the length of every block, so a file that ends inside the chain is
/// named as a cut.
#[test]
fn a_cut_flac_is_not_called_truncated() {
    let bytes = std::fs::read(fixture("sine.flac")).expect("the fixture");
    let audio_at = flac_audio_at(&bytes);
    let mut rng = Rng::new(SEED ^ hash("flac"));
    for _ in 0..ROUNDS {
        let at = rng.between(audio_at as u64, bytes.len() as u64) as usize;
        let verdict = symphonia_verdict(bytes[..at].to_vec(), "flac");
        assert!(
            !verdict.is_truncated(),
            "a cut flac has no oracle and must not be called truncated: {verdict:?}"
        );
    }

    // The metadata chain is a different matter: it states the length of every
    // block, so a file that ends inside it was cut.
    let verdict = symphonia_verdict(bytes[..audio_at - 4].to_vec(), "flac");
    assert!(
        verdict.is_truncated(),
        "a flac whose metadata chain was cut is named as a cut: {verdict:?}"
    );
}

// ---------------------------------------------------------------------------
// The file that is shorter than its own header
// ---------------------------------------------------------------------------

/// A file that is shorter than its own header is a loud error, never a
/// silent clean end.
///
/// A file that cannot hold its own header states nothing about its payload,
/// so it is no oracle - and a file that says nothing and delivers nothing
/// looks like a file that ended cleanly. It must not: the reader has to say
/// that the file is broken.
#[test]
fn a_file_shorter_than_its_header_is_a_loud_error_and_not_a_silent_end() {
    let ogg = ogg_file(&ogg_pages(), true);
    let first_page = 27 + ogg_pages()[0].table.len() + ogg_pages()[0].body.len();
    let mp4_bytes = mp4(100, 100, 400, 400);
    let files: [(&str, Vec<u8>, &str, usize); 5] = [
        ("wav", wav(100), "wav", WAV_DATA_AT),
        ("aiff", aiff(100, 200, 200), "aiff", AIFF_DATA_AT),
        (
            "mp4",
            mp4_bytes.clone(),
            "m4a",
            mp4_media_at(&mp4_bytes) - 8,
        ),
        (
            "mpeg",
            mpeg(mp3_audio_frame_count(), honest_mp3_bytes()),
            "mp3",
            MP3_TAG_FRAME_LEN,
        ),
        ("ogg", ogg, "ogg", first_page),
    ];
    for (label, bytes, ext, header) in files {
        for cut in 0..header {
            let verdict = symphonia_verdict(bytes[..cut].to_vec(), ext);
            assert!(
                !verdict.is_clean(),
                "{label}: a file of {cut} bytes must not end cleanly ({verdict:?})"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The prefix sweep
// ---------------------------------------------------------------------------

/// The prefix lengths of a file that the sweep walks.
///
/// The head of the file is walked byte by byte, because that is where the
/// headers and their length fields are; the rest is walked with a fixed
/// stride, so that the whole file is covered without walking every byte of a
/// 27 kB file: a full walk of every prefix of every fixture would open a
/// hundred thousand decoders and take minutes.
fn prefix_lengths(len: usize) -> Vec<usize> {
    const HEAD: usize = 256;
    const STRIDE: usize = 64;
    let mut out: Vec<usize> = (0..=len.min(HEAD)).collect();
    let mut at = HEAD;
    while at < len {
        at += STRIDE;
        out.push(at.min(len));
    }
    out.push(len);
    out.dedup();
    out
}

/// The extension a fixture is opened with.
fn extension_of(name: &str) -> &'static str {
    match name {
        "sine.mp3" => "mp3",
        "sine.flac" => "flac",
        "sine.ogg" => "ogg",
        "sine.opus" => "opus",
        "sine_alac.m4a" | "sine_aac.m4a" => "m4a",
        "sine_opus.mkv" => "mkv",
        other => panic!("no extension for {other}"),
    }
}

/// No prefix of a fixture panics, and every prefix ends in a defined way.
///
/// A file that was half downloaded is the normal case for a player, and the
/// length of the prefix is the only thing that changes. Every one of them has
/// to be refused, decoded or named as a cut - never a panic, never a hang.
///
/// A panic of the third-party code is caught by the decoder and named as a
/// typed failure (see `symphonia_dec`): the counters below are the
/// measurement the contract notes carry - the panics of the Matroska reader
/// all happen while the file is *opened*, none of them while it is *read*.
#[test]
fn no_prefix_of_a_fixture_panics() {
    let mut panicked_on_open = 0usize;
    let mut panicked_on_read = 0usize;
    for name in [
        "sine.mp3",
        "sine.flac",
        "sine.ogg",
        "sine.opus",
        "sine_alac.m4a",
        "sine_aac.m4a",
        "sine_opus.mkv",
    ] {
        let bytes = std::fs::read(fixture(name)).expect("the fixture");
        let ext = extension_of(name);
        for cut in prefix_lengths(bytes.len()) {
            let prefix = bytes[..cut].to_vec();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                symphonia_verdict(prefix, ext)
            }));
            let verdict =
                result.unwrap_or_else(|_| panic!("{name} cut at {cut} bytes panicked the decoder"));
            let message = match &verdict {
                Verdict::Refused { message } => message,
                Verdict::Failed { message, .. } => message,
                Verdict::Clean { .. } => continue,
            };
            if !message.contains("panicked") {
                continue;
            }
            // A panic of the third-party code is named by the guard of the
            // decoder: a typed failure, not a crash.
            assert!(
                message.contains("it is broken"),
                "{name} cut at {cut} bytes: a panic must be named by the guard ({message})"
            );
            if message.contains("while the file was read") {
                panicked_on_read += 1;
            } else {
                panicked_on_open += 1;
            }
        }
    }
    assert_eq!(
        panicked_on_read, 0,
        "no prefix of a fixture makes the third-party code panic while it is read"
    );
    assert!(
        panicked_on_open > 0,
        "the corpus must exercise the guard: the Matroska reader panics on the prefixes of its fixture"
    );
}
