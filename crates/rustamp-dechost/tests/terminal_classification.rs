//! Terminal classification of every truncation oracle, end to end.
//!
//! A decoder that cannot tell a cut file from a finished one plays a
//! half-downloaded track as if it were complete. The place where that verdict
//! becomes a user-visible outcome is the **resident**: the decoder adapter
//! behind the media host (`rustamp-dechost`), where a clean end of stream and
//! a loud failure are told apart. This file pins that decision for every
//! format family the decoder registry reads.
//!
//! # What is pinned
//!
//! For every family, three shapes of file:
//!
//! | shape | what it is | what must happen |
//! |---|---|---|
//! | **cut** | the file was cut in the middle of its payload | a **loud** failure that names the truncation |
//! | **corrupt** | the file is damaged but holds every byte it declares | a **clean** end of stream (the "keeps playing" contract) |
//! | **lying header** | the header claims more than the file holds | the verdict the format allows (see the families) |
//!
//! FLAC is the family without an oracle: a cut **audio** payload ends
//! cleanly, while a cut **metadata chain** is loud. The two are different
//! verdicts and are pinned apart.
//!
//! # Where the decision is made
//!
//! The resident is `rustamp_dechost::RegistryResident` - the one the crate
//! ships - and it runs **in this process**: no resident binary is spawned, no
//! window is opened, no file is downloaded. The tests below therefore run on
//! the host, not only in the test VM.
//!
//! The second half of the file walks the same files through the framework's
//! **in-process host** (`ControlPeer` + `serve_decoder` + `DecoderSession`,
//! the pair `tests/real_decode.rs` already uses) to show what the player
//! sees: an end of stream or a terminal decoder error. That half needs the
//! shared-memory PCM ring of the framework and is therefore Windows-only,
//! like every other ring test in the repository.
//!
//! # The counts
//!
//! The delivered frame count is checked in every case: a file never delivers
//! more frames than it holds, and a cut file delivers exactly the frames it
//! still has - no frame is invented at the tail to reach the count the
//! header declares.

use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rustamp_dechost::RegistryResident;
use rustamp_mediahost::decoder::DecoderResident;
use rustamp_proto::host::MediaLocationWire;

// ---------------------------------------------------------------------------
// The fixtures
// ---------------------------------------------------------------------------

/// The sample rate of the generated WAVE and AIFF files.
const RATE: u32 = 8_000;

/// The number of frames of the generated WAVE and AIFF files.
const FRAMES: u32 = 4_000;

/// The number of frames of the repository fixtures: 1.5 s at 44.1 kHz.
const FIXTURE_FRAMES: u64 = 44_100 * 3 / 2;

/// A repository fixture of the decoder crate.
fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rustamp-decoders/tests/fixtures")
        .join(name);
    std::fs::read(&path)
        .unwrap_or_else(|error| panic!("the fixture {} is missing: {error}", path.display()))
}

/// A temporary file with the given bytes; the name is unique per process.
fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let path = std::env::temp_dir().join(format!(
        "rustamp-dechost-terminal-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, bytes).expect("the temporary fixture is written");
    path
}

/// A minimal 16 bit mono WAVE file: the header states the frame count, the
/// payload holds `frames` frames of it.
fn wave(frames: u32) -> Vec<u8> {
    let data_len = frames as usize * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for index in 0..frames {
        let sample = ((index as f32 * 0.1).sin() * 1000.0) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// The 80 bit IEEE 754 extended float an AIFF file stores a sample rate in.
fn extended_rate(rate: u32) -> [u8; 10] {
    let exponent = 16383 + 31 - rate.leading_zeros();
    let mantissa = u64::from(rate) << (63 - (31 - rate.leading_zeros()));
    let mut out = [0u8; 10];
    out[..2].copy_from_slice(&(exponent as u16).to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// A minimal AIFF file: the `COMM` chunk states the frame count, the `SSND`
/// chunk the number of bytes of sound data.
fn aiff(frames: u32) -> Vec<u8> {
    let data_len = u64::from(frames) * 2;
    let ssnd_len = 8 + data_len;
    let mut out = Vec::new();
    out.extend_from_slice(b"FORM");
    out.extend_from_slice(&((4 + 26 + 8 + ssnd_len) as u32).to_be_bytes());
    out.extend_from_slice(b"AIFF");
    out.extend_from_slice(b"COMM");
    out.extend_from_slice(&18u32.to_be_bytes());
    out.extend_from_slice(&1u16.to_be_bytes()); // channels
    out.extend_from_slice(&frames.to_be_bytes()); // sample frames
    out.extend_from_slice(&16u16.to_be_bytes()); // bits per sample
    out.extend_from_slice(&extended_rate(RATE));
    out.extend_from_slice(b"SSND");
    out.extend_from_slice(&(ssnd_len as u32).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes()); // offset
    out.extend_from_slice(&0u32.to_be_bytes()); // block size
    for index in 0..frames {
        let sample = ((index as f32 * 0.1).sin() * 1000.0) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

/// The offset of the Xing/Info tag of an MPEG audio file.
fn xing_tag(bytes: &[u8]) -> usize {
    bytes
        .windows(4)
        .position(|window| window == b"Xing" || window == b"Info")
        .expect("the fixture carries a Xing/Info tag")
}

/// The MPEG frames of a file: how many there are and how many samples they
/// carry.
///
/// The walk needs the frame header of every frame: the bitrate and the
/// sample rate state the length of a frame. The samples are the upper bound
/// of what the file can deliver - a decoder never delivers more frames than
/// the file holds.
fn mpeg_frames(bytes: &[u8]) -> (usize, u64) {
    const BITRATES: [u64; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    const RATES: [u64; 4] = [44100, 48000, 32000, 0];
    // The ID3v2 tag in front of the audio.
    let mut at = 0usize;
    if bytes.starts_with(b"ID3") {
        let size = (u64::from(bytes[6] & 0x7F) << 21)
            | (u64::from(bytes[7] & 0x7F) << 14)
            | (u64::from(bytes[8] & 0x7F) << 7)
            | u64::from(bytes[9] & 0x7F);
        at = 10 + size as usize;
    }
    let mut frames = 0usize;
    let mut samples = 0u64;
    while at + 4 <= bytes.len() {
        if bytes[at] != 0xFF || (bytes[at + 1] & 0xE0) != 0xE0 {
            break;
        }
        let bitrate = BITRATES[usize::from(bytes[at + 2] >> 4)] * 1000;
        let rate = RATES[usize::from((bytes[at + 2] >> 2) & 3)];
        if bitrate == 0 || rate == 0 {
            break;
        }
        let padding = u64::from(bytes[at + 2] & 0x02 != 0);
        let samples_per_frame = 1152u64;
        let size = (samples_per_frame / 8 * bitrate) / rate + padding;
        if size == 0 {
            break;
        }
        frames += 1;
        samples += samples_per_frame;
        at += size as usize;
    }
    (frames, samples)
}

/// The offset of the duration field of the media header of an MP4 file.
///
/// The media header is a full box: the version and the flags, then the
/// creation and the modification time, then the timescale and the duration.
fn mdhd_duration(bytes: &[u8]) -> usize {
    let at = bytes
        .windows(4)
        .position(|window| window == b"mdhd")
        .expect("the fixture carries a media header");
    assert_eq!(bytes[at + 4], 0, "the media header is a version 0 box");
    at + 4 + 16
}

/// The pages of an Ogg file: where every page starts and where it ends.
///
/// The segment table of a page states the length of its body, so the pages of
/// a file can be walked without decoding anything.
fn ogg_pages(bytes: &[u8]) -> Vec<(usize, usize)> {
    let mut pages = Vec::new();
    let mut at = 0usize;
    while at + 27 <= bytes.len() {
        assert_eq!(&bytes[at..at + 4], b"OggS", "a page starts at {at}");
        let segments = usize::from(bytes[at + 26]);
        let table_at = at + 27;
        let body: usize = bytes[table_at..table_at + segments]
            .iter()
            .map(|&lacing| usize::from(lacing))
            .sum();
        let end = table_at + segments + body;
        pages.push((at, end));
        at = end;
    }
    pages
}

/// The CRC-32 of the Ogg format: the polynomial `0x04c11db7`, neither
/// reflected in nor out and without a final inversion.
fn ogg_crc32(crc: u32, bytes: &[u8]) -> u32 {
    let mut crc = crc;
    for &byte in bytes {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Writes the checksum of the Ogg page at `at` into its header.
///
/// The checksum covers the header with its checksum field zeroed, the segment
/// table and the body of the page.
fn ogg_write_checksum(bytes: &mut [u8], at: usize, end: usize) {
    let mut head = bytes[at..at + 27].to_vec();
    head[22..26].fill(0);
    let crc = ogg_crc32(ogg_crc32(0, &head), &bytes[at + 27..end]);
    bytes[at + 22..at + 26].copy_from_slice(&crc.to_le_bytes());
}

/// The metadata blocks of a FLAC file: their type, where their payload starts
/// and how long it is.
fn flac_metadata(bytes: &[u8]) -> Vec<(u8, usize, usize)> {
    assert_eq!(&bytes[..4], b"fLaC", "the fixture is a FLAC file");
    let mut blocks = Vec::new();
    let mut at = 4usize;
    loop {
        let header = *bytes.get(at).expect("a metadata block header");
        let len = (usize::from(bytes[at + 1]) << 16)
            | (usize::from(bytes[at + 2]) << 8)
            | usize::from(bytes[at + 3]);
        blocks.push((header & 0x7F, at + 4, len));
        at += 4 + len;
        if header & 0x80 != 0 {
            return blocks;
        }
    }
}

/// The `STREAMINFO` block of a FLAC file: where its payload starts and the
/// payload itself (34 bytes).
fn flac_stream_info(bytes: &[u8]) -> (usize, [u8; 34]) {
    let (_, at, _) = flac_metadata(bytes)
        .into_iter()
        .find(|(kind, _, _)| *kind == 0)
        .expect("the fixture carries a STREAMINFO block");
    let mut payload = [0u8; 34];
    payload.copy_from_slice(&bytes[at..at + 34]);
    (at, payload)
}

/// Writes the number of samples the `STREAMINFO` block of a FLAC file
/// declares.
///
/// The block holds the sample rate, the channel count and the bit depth in
/// the first 28 bits of its last eight bytes, and the total number of
/// samples in the 36 bits behind them.
fn flac_set_declared_samples(payload: &mut [u8; 34], samples: u64) {
    let mut bits = u64::from_be_bytes(payload[10..18].try_into().expect("eight bytes"));
    bits = (bits & !((1 << 36) - 1)) | (samples & ((1 << 36) - 1));
    payload[10..18].copy_from_slice(&bits.to_be_bytes());
}

// ---------------------------------------------------------------------------
// The classification
// ---------------------------------------------------------------------------

/// How a stream ended, from the resident's point of view.
#[derive(Debug)]
enum Terminal {
    /// The file held everything its container declares: a clean end of stream.
    Clean,
    /// The file was cut or could not be read: a loud failure.
    Loud {
        /// Whether the failure happened when the file was opened.
        at_open: bool,
        /// The reason the player sees.
        reason: String,
    },
}

/// What a file did at the end of its stream.
#[derive(Debug)]
struct Outcome {
    /// The terminal the stream reached.
    terminal: Terminal,
    /// The frames the file delivered.
    frames: u64,
}

/// Walk a file through the resident and report how the stream ended.
///
/// The resident is the one the crate ships (`RegistryResident`); it runs in
/// this process, so nothing is spawned.
fn walk(path: &Path) -> Outcome {
    let location = MediaLocationWire::File(path.to_string_lossy().into_owned());
    let mut decoder = match RegistryResident.open(location) {
        Ok(decoder) => decoder,
        Err(error) => {
            return Outcome {
                terminal: Terminal::Loud {
                    at_open: true,
                    reason: error.to_string(),
                },
                frames: 0,
            };
        }
    };
    let channels = usize::from(decoder.info().spec.channels.max(1));
    let mut buffer = vec![0.0f32; 4096 * channels];
    let mut frames = 0u64;
    loop {
        match decoder.read(&mut buffer) {
            Ok(0) => {
                return Outcome {
                    terminal: Terminal::Clean,
                    frames,
                };
            }
            Ok(read) => frames += read as u64,
            Err(error) => {
                return Outcome {
                    terminal: Terminal::Loud {
                        at_open: false,
                        reason: error.to_string(),
                    },
                    frames,
                };
            }
        }
    }
}

/// The frames a file must deliver.
type Frames = RangeInclusive<u64>;

/// Assert that a file ends cleanly and delivers the frames it holds.
fn assert_clean(name: &str, bytes: &[u8], frames: Frames) {
    let path = temp_file(name, bytes);
    let outcome = walk(&path);
    let _ = std::fs::remove_file(&path);
    match &outcome.terminal {
        Terminal::Clean => {}
        Terminal::Loud { at_open, reason } => {
            panic!("{name}: the file must end cleanly, but it failed ({at_open}): {reason}");
        }
    }
    assert!(
        outcome.frames >= *frames.start() && outcome.frames <= *frames.end(),
        "{name}: the file delivered {} frames, but it holds {}..={}",
        outcome.frames,
        frames.start(),
        frames.end()
    );
}

/// Assert that a file fails loudly, that the failure happens where the
/// format says it does, that the reason names the truncation in the words of
/// the format and that it delivers the frames it holds.
///
/// `at_open` says where the failure happens: the MP4 container reader scans
/// the whole file before it plays anything, so a cut MP4 file is refused
/// when it is opened; every other family fails at the end of the stream.
///
/// Every needle must be in the reason: the first one names the truncation
/// itself, the second one names the claim of the format that proves it.
fn assert_loud(name: &str, bytes: &[u8], at_open: bool, needles: &[&str], frames: Frames) {
    let path = temp_file(name, bytes);
    let outcome = walk(&path);
    let _ = std::fs::remove_file(&path);
    let reason = match &outcome.terminal {
        Terminal::Loud {
            at_open: failed_at,
            reason,
        } => {
            assert!(
                *failed_at == at_open,
                "{name}: the failure must happen {}, but it happened at the other place: {reason}",
                if at_open {
                    "at the open"
                } else {
                    "at the end of the stream"
                }
            );
            reason.clone()
        }
        Terminal::Clean => panic!(
            "{name}: a cut file must fail loudly, but it ended cleanly after {} frames",
            outcome.frames
        ),
    };
    for needle in needles {
        assert!(
            reason.contains(needle),
            "{name}: the failure must name the {needle}: {reason}"
        );
    }
    assert!(
        outcome.frames >= *frames.start() && outcome.frames <= *frames.end(),
        "{name}: the file delivered {} frames, but it holds {}..={}",
        outcome.frames,
        frames.start(),
        frames.end()
    );
}

/// Assert that a file fails loudly and that the reason must **not** name the
/// truncation: damage is not a cut file.
fn assert_loud_without(name: &str, bytes: &[u8], forbidden: &str) {
    let path = temp_file(name, bytes);
    let outcome = walk(&path);
    let _ = std::fs::remove_file(&path);
    let reason = match &outcome.terminal {
        Terminal::Loud { reason, .. } => reason.clone(),
        Terminal::Clean => panic!("{name}: the file must not end cleanly"),
    };
    assert!(
        !reason.contains(forbidden),
        "{name}: a damaged file is not a cut file, but the reason says: {reason}"
    );
}

// ---------------------------------------------------------------------------
// WAVE
// ---------------------------------------------------------------------------

/// The WAVE family.
///
/// The `data` chunk is the only claim WAVE makes: it states the number of
/// bytes, and the frame count follows from it. A damaged sample is still a
/// sample, so a shortfall of frames can only mean that the file was cut.
#[test]
fn wave_is_loud_on_a_cut_file_and_clean_on_a_damaged_one() {
    // The file holds 4000 frames; the cut leaves 2999 whole ones.
    let mut cut = wave(FRAMES);
    cut.truncate(cut.len() - 2001);
    assert_loud(
        "wave-cut.wav",
        &cut,
        false,
        &["truncated", "its container declares"],
        2999..=2999,
    );

    // The same file with damaged samples: it holds every byte its `data`
    // chunk declares, so it keeps playing and ends cleanly.
    let mut corrupt = wave(FRAMES);
    for byte in &mut corrupt[1000..1300] {
        *byte ^= 0xA5;
    }
    assert_clean("wave-corrupt.wav", &corrupt, FRAMES as u64..=FRAMES as u64);

    // The RIFF size field claims four gigabytes while the file holds eight
    // kilobytes. The chunk walk is authoritative - the `data` chunk states
    // the payload - so the claim of the file header is not an oracle: the
    // file plays to its end and ends cleanly.
    let mut lying = wave(FRAMES);
    lying[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_clean("wave-lying.wav", &lying, FRAMES as u64..=FRAMES as u64);

    // The limit of the rule: the `data` chunk is WAVE's **only** claim, and
    // it states the number of bytes - so a count larger than the file is not
    // a lying header but a cut file, and the two cannot be told apart. Such
    // a file fails loudly: a player must not play it short as if it were
    // complete.
    let mut short = wave(FRAMES);
    short[4..8].copy_from_slice(&(36 + FRAMES * 4).to_le_bytes());
    short[40..44].copy_from_slice(&(FRAMES * 4).to_le_bytes());
    assert_loud(
        "wave-lying-data.wav",
        &short,
        false,
        &["truncated", "its container declares"],
        FRAMES as u64..=FRAMES as u64,
    );
}

// ---------------------------------------------------------------------------
// MPEG audio
// ---------------------------------------------------------------------------

/// The MPEG audio family.
///
/// The Xing/Info tag states the number of MPEG frames **and** the number of
/// bytes the audio occupies. Both are needed: a damaged MPEG frame is
/// resynchronised by the reader, so a damaged file delivers fewer frames
/// than it declares as well - but it holds all of its bytes.
#[test]
fn mpeg_is_loud_on_a_cut_file_and_clean_on_a_damaged_one() {
    let full = fixture("sine.mp3");

    // The file holds 60 MPEG frames; the cut leaves the ones that are whole.
    let mut cut = full.clone();
    cut.truncate(cut.len() * 3 / 5);
    let (_, samples) = mpeg_frames(&cut);
    assert_loud(
        "mpeg-cut.mp3",
        &cut,
        false,
        &["truncated", "its Xing/Info header declares"],
        1..=samples,
    );

    // The header is intact and states the whole file, but the damage in the
    // middle costs frames - not bytes. The file keeps playing.
    let mut corrupt = full.clone();
    let len = corrupt.len();
    for byte in &mut corrupt[len / 2..len / 2 + 300] {
        *byte ^= 0xA5;
    }
    assert_clean("mpeg-corrupt.mp3", &corrupt, 1..=FIXTURE_FRAMES);

    // The frame count of the tag is a lie: it claims four billion frames
    // while the file holds sixty. The file holds every byte the tag
    // declares, so it is not a cut file - and the frames it delivers are the
    // frames of the sixty MPEG frames it holds.
    let tag = xing_tag(&full);
    let mut lying = full.clone();
    lying[tag + 8..tag + 12].copy_from_slice(&u32::MAX.to_be_bytes());
    let (frames, samples) = mpeg_frames(&full);
    assert!(
        samples >= FIXTURE_FRAMES,
        "the {frames} frames of the fixture carry {samples} samples"
    );
    assert_clean("mpeg-lying.mp3", &lying, FIXTURE_FRAMES..=samples);
}

// ---------------------------------------------------------------------------
// AIFF
// ---------------------------------------------------------------------------

/// The AIFF family.
///
/// The `COMM` chunk states the number of sample frames and the `SSND` chunk
/// the number of bytes of sound data. The two together tell a cut file from
/// a damaged one.
#[test]
fn aiff_is_loud_on_a_cut_file_and_clean_on_a_damaged_one() {
    // The file holds 4000 frames; the cut leaves 3998 whole ones.
    let mut cut = aiff(FRAMES);
    cut.truncate(cut.len() - 3);
    assert_loud(
        "aiff-cut.aiff",
        &cut,
        false,
        &["truncated", "its COMM chunk declares"],
        3998..=3998,
    );

    // The samples in the middle are damaged, but the file holds every byte
    // its `SSND` chunk declares: it keeps playing and ends cleanly.
    let mut corrupt = aiff(FRAMES);
    for byte in &mut corrupt[1000..1300] {
        *byte ^= 0xA5;
    }
    assert_clean("aiff-corrupt.aiff", &corrupt, FRAMES as u64..=FRAMES as u64);

    // The `COMM` chunk claims four billion frames while the file holds
    // four thousand. The file holds every byte its `SSND` chunk declares, so
    // the wrong header is not a cut file.
    let mut lying = aiff(FRAMES);
    let at = 12 + 8 + 2;
    lying[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_clean("aiff-lying.aiff", &lying, FRAMES as u64..=FRAMES as u64);
}

// ---------------------------------------------------------------------------
// MP4
// ---------------------------------------------------------------------------

/// The MP4 family.
///
/// The media data box states the bytes of media data the file holds, and the
/// media header together with the sample tables state the frames. The
/// container reader scans the whole file before it plays anything, so a cut
/// MP4 file is refused when it is **opened** - the terminal shape of an
/// unreadable file, with the same diagnostic.
#[test]
fn mp4_is_loud_on_a_cut_file_and_clean_on_a_damaged_one() {
    let full = fixture("sine_alac.m4a");

    // The media data box is behind the cut: the file is refused when it is
    // opened, and the refusal names the cut.
    let mut cut = full.clone();
    cut.truncate(cut.len() / 2);
    assert_loud(
        "mp4-cut.m4a",
        &cut,
        true,
        &["truncated", "its media data box declares"],
        0..=0,
    );

    // The samples in the middle are damaged, but the file holds every byte
    // its media data box declares: it keeps playing and ends cleanly.
    let mut corrupt = full.clone();
    let len = corrupt.len();
    for byte in &mut corrupt[len / 2..len / 2 + 300] {
        *byte ^= 0xA5;
    }
    assert_clean("mp4-corrupt.m4a", &corrupt, 1..=FIXTURE_FRAMES);

    // The media header claims twice the frames the file holds. The file
    // holds every byte its media data box declares, so the wrong header is
    // not a cut file - and the frames it delivers are the frames the file
    // holds.
    let mut lying = full.clone();
    let at = mdhd_duration(&lying);
    lying[at..at + 4].copy_from_slice(&(FIXTURE_FRAMES as u32 * 2).to_be_bytes());
    assert_clean("mp4-lying.m4a", &lying, FIXTURE_FRAMES..=FIXTURE_FRAMES);
}

// ---------------------------------------------------------------------------
// Ogg
// ---------------------------------------------------------------------------

/// The Ogg family.
///
/// Ogg states the end of its stream with a **flag** on the last page, not
/// with a count. The oracle is that flag: a file whose last page is intact
/// and does not carry it says that more is to come, so it was cut - even
/// when every byte of its audio is there.
#[test]
fn ogg_is_loud_on_a_cut_file_and_clean_on_a_damaged_one() {
    let full = fixture("sine.ogg");
    let pages = ogg_pages(&full);
    let (last_at, last_end) = *pages.last().expect("the fixture carries pages");

    // The file ends where the end-of-stream page starts: the pages it holds
    // are intact and the file is named as a cut file.
    let cut = full[..last_at].to_vec();
    assert_loud(
        "ogg-cut.ogg",
        &cut,
        false,
        &["truncated", "end-of-stream flag"],
        1..=FIXTURE_FRAMES,
    );

    // The audio of a page is damaged and the page's checksum is repaired, so
    // the container stays consistent: the file keeps playing and ends
    // cleanly. The frames the damage costs are gone, and the file is not
    // called a cut file.
    let (at, end) = pages[2];
    let mut corrupt = full.clone();
    let segments = usize::from(corrupt[at + 26]);
    for byte in &mut corrupt[at + 27 + segments..end] {
        *byte ^= 0xA5;
    }
    ogg_write_checksum(&mut corrupt, at, end);
    assert_clean("ogg-corrupt.ogg", &corrupt, 1..=FIXTURE_FRAMES);

    // A page whose checksum does not match cannot be read, so the page that
    // would say "the stream ends here" says nothing: the container reader
    // refuses the file, but the oracle must not call it a cut file.
    let (at, end) = pages[2];
    let mut damaged = full.clone();
    let segments = usize::from(damaged[at + 26]);
    for byte in &mut damaged[at + 27 + segments..end] {
        *byte ^= 0xA5;
    }
    assert_loud_without("ogg-damaged.ogg", &damaged, "truncated");

    // The granule position of the last page claims twice the samples the
    // file holds. The Ogg oracle is the end-of-stream flag and not the
    // count: the count is the reader's business, and the file ends cleanly.
    let mut lying = full.clone();
    lying[last_at + 6..last_at + 14].copy_from_slice(&(FIXTURE_FRAMES * 2).to_le_bytes());
    ogg_write_checksum(&mut lying, last_at, last_end);
    assert_clean(
        "ogg-lying.ogg",
        &lying,
        FIXTURE_FRAMES..=FIXTURE_FRAMES + 1152,
    );

    // The end-of-stream flag is gone: the file is complete in every other
    // respect and holds every byte of its audio, and it says that more is to
    // come. That is the Ogg oracle, and it is loud.
    //
    // The frames: the granule position of the last page states the 66150
    // samples of the audio, but a file whose end-of-stream flag is missing
    // is not trimmed at the end - the last Vorbis packet is delivered in
    // full, and it carries 986 samples beyond the granule position. One
    // packet (1152 samples) is the bound for that.
    let mut no_flag = full.clone();
    no_flag[last_at + 5] &= !0x04;
    ogg_write_checksum(&mut no_flag, last_at, last_end);
    assert_loud(
        "ogg-no-flag.ogg",
        &no_flag,
        false,
        &["truncated", "end-of-stream flag"],
        FIXTURE_FRAMES..=FIXTURE_FRAMES + 1152,
    );
}

// ---------------------------------------------------------------------------
// FLAC
// ---------------------------------------------------------------------------

/// The FLAC family: no oracle for the audio, one for the metadata.
///
/// FLAC states its sample count in the `STREAMINFO` block, but the format
/// states no byte count, and a damaged FLAC frame is dropped by the reader
/// while the reader resynchronises at the next frame. A corrupt file is
/// therefore short of samples exactly like a cut one, and it holds all of
/// its bytes - the shortfall proves nothing. The *metadata* is a different
/// matter: the chain states the length of every block, so a file that ends
/// inside the chain was cut.
#[test]
fn flac_ends_cleanly_on_a_cut_audio_payload_and_is_loud_on_a_cut_metadata_chain() {
    let full = fixture("sine.flac");

    // The audio is cut: the file holds whole frames and ends cleanly.
    let cut = full[..full.len() / 2].to_vec();
    assert_clean("flac-cut.flac", &cut, 1..=FIXTURE_FRAMES);

    // The metadata chain is cut: the file is refused when it is opened, and
    // the refusal names the cut.
    let (_, at, len) = flac_metadata(&full)[0];
    let cut = full[..at + len / 2].to_vec();
    assert_loud(
        "flac-cut-metadata.flac",
        &cut,
        true,
        &["truncated", "metadata blocks"],
        0..=0,
    );

    // A damaged frame is dropped by the reader: the file is short of the
    // samples it declares, but it holds all of its bytes and keeps playing.
    let mut corrupt = full.clone();
    let len = corrupt.len();
    for byte in &mut corrupt[len / 2..len / 2 + 16] {
        *byte ^= 0xA5;
    }
    assert_clean("flac-corrupt.flac", &corrupt, 1..=FIXTURE_FRAMES);

    // The `STREAMINFO` block claims twice the samples the file holds. The
    // file holds all of its bytes, so the wrong header is not a cut file.
    let (at, mut payload) = flac_stream_info(&full);
    flac_set_declared_samples(&mut payload, FIXTURE_FRAMES * 2);
    let mut lying = full.clone();
    lying[at..at + 34].copy_from_slice(&payload);
    assert_clean("flac-lying.flac", &lying, FIXTURE_FRAMES..=FIXTURE_FRAMES);
}

// ---------------------------------------------------------------------------
// The framework: what the player sees
// ---------------------------------------------------------------------------

/// The framework's in-process host: the player side of the media host.
///
/// The two peers talk over in-memory channels: no process is spawned, no
/// window is opened. The ring is the shared memory of the framework, which
/// is a Windows facility.
#[cfg(windows)]
mod framework {
    use std::io::{Cursor, Read, Write};
    use std::path::Path;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use rustamp_dechost::RegistryResident;
    use rustamp_mediahost::Result;
    use rustamp_mediahost::control::{ControlPeer, Liveness};
    use rustamp_mediahost::decoder::{DecoderSession, serve_decoder};
    use rustamp_mediahost::pcm::NamedPcmRings;
    use rustamp_proto::host::MediaLocationWire;

    /// The liveness policy of the in-process peers: a stall escape, never a
    /// schedule bound.
    const LIVENESS: Liveness = Liveness {
        interval: Duration::from_millis(100),
        timeout: Duration::from_secs(30),
    };

    /// The reader end of the control channel.
    struct ByteReader {
        rx: mpsc::Receiver<Vec<u8>>,
        bytes: Cursor<Vec<u8>>,
    }

    /// The writer end of the control channel.
    struct ByteWriter(mpsc::Sender<Vec<u8>>);

    impl Read for ByteReader {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if out.is_empty() {
                return Ok(0);
            }
            loop {
                let count = self.bytes.read(out)?;
                if count > 0 {
                    return Ok(count);
                }
                match self.rx.recv() {
                    Ok(bytes) => self.bytes = Cursor::new(bytes),
                    Err(_) => return Ok(0),
                }
            }
        }
    }

    impl Write for ByteWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .send(bytes.to_vec())
                .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// The two ends of the control channel.
    fn streams() -> ((ByteReader, ByteWriter), (ByteReader, ByteWriter)) {
        let (a_tx, b_rx) = mpsc::channel();
        let (b_tx, a_rx) = mpsc::channel();
        (
            (
                ByteReader {
                    rx: a_rx,
                    bytes: Cursor::new(Vec::new()),
                },
                ByteWriter(a_tx),
            ),
            (
                ByteReader {
                    rx: b_rx,
                    bytes: Cursor::new(Vec::new()),
                },
                ByteWriter(b_tx),
            ),
        )
    }

    /// What the player sees at the end of a stream.
    #[derive(Debug)]
    pub(super) enum Seen {
        /// The player saw an end of stream.
        EndOfStream,
        /// The player saw a terminal failure with this reason.
        Failure(String),
    }

    /// Read the whole stream and report how it ended.
    ///
    /// `None` is a clean end of stream; `Some(message)` is the failure that
    /// ended the transfer.
    fn drain(session: &mut DecoderSession<NamedPcmRings>) -> (usize, Option<String>) {
        let mut buffer = [0.0f32; 128];
        let mut frames = 0usize;
        loop {
            match session.read(&mut buffer, Some(Duration::from_secs(30))) {
                Ok(0) => return (frames, None),
                Ok(read) => frames += read,
                Err(error) => return (frames, Some(error.to_string())),
            }
        }
    }

    /// Walk a file through the framework and report what the player sees.
    pub(super) fn play(path: &Path) -> Seen {
        let ((ar, aw), (br, bw)) = streams();
        let client = ControlPeer::start(ar, aw, LIVENESS, false).expect("the client peer starts");
        let host = ControlPeer::start(br, bw, LIVENESS, true).expect("the host peer starts");
        let (done, result) = mpsc::channel();
        let location = MediaLocationWire::File(path.to_string_lossy().into_owned());
        thread::spawn(move || {
            let outcome: Result<()> = serve_decoder(&host, &NamedPcmRings, &RegistryResident);
            let _ = done.send(outcome);
        });
        let session = DecoderSession::open(client, NamedPcmRings, location, 7);
        let (frames, failure) = match session {
            Ok(mut session) => drain(&mut session),
            // The resident refused the file when it was opened: the player
            // never gets a session, and the reason names the refusal.
            Err(error) => (0, Some(error.to_string())),
        };
        let host = result
            .recv_timeout(Duration::from_secs(30))
            .expect("the host must end instead of stalling");
        match failure {
            Some(message) => {
                assert!(
                    host.is_err(),
                    "the host must end with the failure of the resident: {host:?}"
                );
                Seen::Failure(message)
            }
            None => {
                assert!(
                    host.is_ok(),
                    "the host must end cleanly after the end of the stream: {host:?}"
                );
                assert!(frames > 0, "the stream must deliver frames");
                Seen::EndOfStream
            }
        }
    }
}

#[cfg(windows)]
use framework::{Seen, play};

/// The player sees a terminal failure for every cut file and an end of
/// stream for every complete one.
///
/// This is the user-visible end of the decision the cases above pin: a cut
/// file must not look like a finished one at the player's screen.
#[cfg(windows)]
#[test]
fn the_player_sees_a_terminal_failure_for_a_cut_file_and_an_end_of_stream_for_a_complete_one() {
    let mut wave_cut = wave(FRAMES);
    wave_cut.truncate(wave_cut.len() - 2001);
    let mut mpeg_cut = fixture("sine.mp3");
    mpeg_cut.truncate(mpeg_cut.len() * 3 / 5);
    let mut aiff_cut = aiff(FRAMES);
    aiff_cut.truncate(aiff_cut.len() - 3);
    let mut mp4_cut = fixture("sine_alac.m4a");
    mp4_cut.truncate(mp4_cut.len() / 2);
    let mut ogg_cut = fixture("sine.ogg");
    let pages = ogg_pages(&ogg_cut);
    let (last_at, _) = *pages.last().expect("the fixture carries pages");
    ogg_cut.truncate(last_at);

    let cut_files: [(&str, Vec<u8>); 5] = [
        ("wave-cut.wav", wave_cut),
        ("mpeg-cut.mp3", mpeg_cut),
        ("aiff-cut.aiff", aiff_cut),
        ("mp4-cut.m4a", mp4_cut),
        ("ogg-cut.ogg", ogg_cut),
    ];
    for (name, bytes) in cut_files {
        let path = temp_file(name, &bytes);
        let seen = play(&path);
        let _ = std::fs::remove_file(&path);
        match seen {
            Seen::Failure(reason) => assert!(
                reason.contains("truncated"),
                "{name}: the player must see the truncation: {reason}"
            ),
            Seen::EndOfStream => panic!("{name}: a cut file must not end the stream"),
        }
    }

    let complete_files: [(&str, Vec<u8>); 5] = [
        ("wave.wav", wave(FRAMES)),
        ("mpeg.mp3", fixture("sine.mp3")),
        ("aiff.aiff", aiff(FRAMES)),
        ("mp4.m4a", fixture("sine_alac.m4a")),
        ("ogg.ogg", fixture("sine.ogg")),
    ];
    for (name, bytes) in complete_files {
        let path = temp_file(name, &bytes);
        let seen = play(&path);
        let _ = std::fs::remove_file(&path);
        match seen {
            Seen::EndOfStream => {}
            Seen::Failure(reason) => {
                panic!("{name}: a complete file must end the stream, not fail: {reason}")
            }
        }
    }
}
