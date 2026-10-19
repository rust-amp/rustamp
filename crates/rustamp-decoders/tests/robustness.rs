//! Robustness tests: hostile input, resource release and probe/registry policy.
//!
//! Every hostile case must end in a defined error or a defined degradation —
//! never a panic, never a hang. The inputs are generated here (in memory) or are
//! a truncated/corrupted copy of a generated repository fixture (see
//! `tests/fixtures/FIXTURES.txt`); nothing is downloaded. No test opens a window
//! and none runs an example: the whole file is headless.

use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rustamp_core::audio::{
    AudioSpec, Confidence, Decoder, DecoderFactory, DecoderRegistry, MediaSource, ProbeHint,
};
use rustamp_core::error::{Error, Result};
use rustamp_core::mock::MockDecoder;
use rustamp_decoders::{SymphoniaDecoder, SymphoniaDecoderFactory, default_registry};

const RATE: u32 = 44_100;
const FIXTURE_FRAMES: u64 = RATE as u64 * 3 / 2;

/// Upper bound on `read` calls per drain. A decoder that never reaches an end of
/// stream within this many calls is reported as a possible hang instead of
/// stalling the suite forever. A hang *inside* a single `read` cannot be turned
/// into a failure from within the process, so the bound is on the loop: it only
/// advances because the previous call returned.
const MAX_READS: usize = 200_000;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn reader_source(bytes: Vec<u8>, extension: Option<&str>) -> MediaSource {
    MediaSource::Reader {
        reader: Box::new(Cursor::new(bytes)),
        extension: extension.map(str::to_owned),
    }
}

fn open_bytes(bytes: Vec<u8>, ext: Option<&str>) -> Box<dyn Decoder> {
    default_registry()
        .open(reader_source(bytes, ext))
        .expect("open")
}

// ---------------------------------------------------------------------------
// Generated hostile input
// ---------------------------------------------------------------------------

/// Deterministic pseudo-random bytes (xorshift64), so a failure is reproducible.
/// It can contain any byte value, including `0xFF` (a false MPEG/ADTS sync).
fn noise_bytes(n: usize) -> Vec<u8> {
    let mut s = 0x2545_F491_4F6C_DD1Du64;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            (s >> 24) as u8
        })
        .collect()
}

/// A byte pattern that provably carries no format signature: `i * 37 % 251` is
/// never `0xFF`, so no MPEG/ADTS sync can be found, and the first bytes cannot
/// match `ID3`, `fLaC`, `OggS`, `caff`, `RIFF`/`FORM` or `ftyp`.
fn pattern_bytes(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i * 37 % 251) as u8).collect()
}

fn wav_bytes(spec: hound::WavSpec, frames: usize, f: impl Fn(usize, u16) -> f64) -> Vec<u8> {
    let mut cur = Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut cur, spec).unwrap();
        for i in 0..frames {
            for c in 0..spec.channels {
                let v = f(i, c);
                match (spec.sample_format, spec.bits_per_sample) {
                    (hound::SampleFormat::Float, _) => w.write_sample(v as f32).unwrap(),
                    (_, bits) => {
                        let max = (1i64 << (bits - 1)) - 1;
                        w.write_sample((v * max as f64).round() as i32).unwrap();
                    }
                }
            }
        }
        w.finalize().unwrap();
    }
    cur.into_inner()
}

fn sine(i: usize, c: u16) -> f64 {
    let t = i as f64 / f64::from(RATE);
    0.8 * (std::f64::consts::TAU * (440.0 + 110.0 * f64::from(c)) * t).sin()
}

// ---------------------------------------------------------------------------
// Bounded drain and outcome classification
// ---------------------------------------------------------------------------

/// How a bounded drain ended.
#[derive(Debug)]
enum Drained {
    /// `read` returned `Ok(0)`: a defined end of stream.
    End,
    /// `read` returned an error before the end of stream.
    Failed(Error),
}

fn drain_bounded(d: &mut dyn Decoder) -> (u64, Drained) {
    let ch = usize::from(d.spec().channels.max(1));
    let mut buf = vec![0.0f32; 4096 * ch];
    let mut frames = 0u64;
    for _ in 0..MAX_READS {
        match d.read(&mut buf) {
            Ok(0) => return (frames, Drained::End),
            Ok(n) => frames += n as u64,
            Err(e) => return (frames, Drained::Failed(e)),
        }
    }
    panic!("no end of stream after {MAX_READS} reads: possible hang");
}

/// A failure while *opening*: nothing was decoded yet, so all three kinds are
/// honest — "not my format", "mine but damaged" or "I/O".
fn assert_defined_open_failure(e: &Error, label: &str) {
    match e {
        Error::Io(_) | Error::Decode(_) | Error::UnsupportedFormat(_) => {}
        other => panic!("{label}: unexpected error kind at open: {other:?}"),
    }
}

/// A failure *mid-stream*. `UnsupportedFormat` must not appear here: once the
/// container was accepted, "this is not my format" would be a lie — the collapse
/// fixed by `stream_err` (see `CONTRACT_NOTES.md`).
fn assert_defined_stream_failure(e: &Error, label: &str) {
    match e {
        Error::Io(_) | Error::Decode(_) | Error::FormatChanged { .. } => {}
        Error::UnsupportedFormat(m) => {
            panic!("{label}: a mid-stream failure was called 'not my format': {m}")
        }
        other => panic!("{label}: unexpected error kind while reading: {other:?}"),
    }
}

/// Open a hostile input and, when it opened, drain it under the read bound.
/// Returns the decoded frame count, or `None` when the input was refused at open
/// time (also a defined outcome, checked by [`assert_defined_open_failure`]).
fn decode_hostile(bytes: Vec<u8>, ext: Option<&str>, label: &str) -> Option<u64> {
    match SymphoniaDecoder::open(reader_source(bytes, ext)) {
        Ok(mut d) => {
            let (frames, outcome) = drain_bounded(&mut d);
            if let Drained::Failed(e) = &outcome {
                assert_defined_stream_failure(e, label);
            }
            Some(frames)
        }
        Err(e) => {
            assert_defined_open_failure(&e, label);
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Readers that make resource release observable
// ---------------------------------------------------------------------------

/// Reader that counts its own drops, so a test can tell "released exactly once"
/// from "leaked" and from "released twice".
struct DropCountingReader {
    inner: Cursor<Vec<u8>>,
    drops: Arc<AtomicUsize>,
}

impl Read for DropCountingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for DropCountingReader {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl Drop for DropCountingReader {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}

fn counting_source(bytes: Vec<u8>, ext: Option<&str>) -> (MediaSource, Arc<AtomicUsize>) {
    let drops = Arc::new(AtomicUsize::new(0));
    let reader = DropCountingReader {
        inner: Cursor::new(bytes),
        drops: Arc::clone(&drops),
    };
    let source = MediaSource::Reader {
        reader: Box::new(reader),
        extension: ext.map(str::to_owned),
    };
    (source, drops)
}

/// Reader that refuses `SeekFrom::End`, so the decoder cannot learn the total
/// length and a container that reports none (MP3 without an estimate) has no
/// duration: the unclamped seek path.
struct NoEndSeek {
    inner: Cursor<Vec<u8>>,
}

impl Read for NoEndSeek {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for NoEndSeek {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        match pos {
            SeekFrom::End(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "this stream cannot report its end",
            )),
            other => self.inner.seek(other),
        }
    }
}

// ---------------------------------------------------------------------------
// Zero-length input
// ---------------------------------------------------------------------------

#[test]
fn a_zero_length_reader_is_named_as_empty_before_probing() {
    // Before B2 this was the probe's "no suitable format reader found", which
    // names the wrong problem: the file is empty, not foreign.
    let err = SymphoniaDecoder::open(reader_source(Vec::new(), Some("mp3")))
        .err()
        .expect("0 bytes cannot hold a container");
    assert!(
        matches!(&err, Error::Decode(m) if m.contains("empty source")),
        "{err:?}"
    );

    // The same through the registry, where a factory is asked first.
    let err = default_registry()
        .open(reader_source(Vec::new(), Some("mp3")))
        .err()
        .expect("0 bytes cannot hold a container");
    assert!(
        matches!(&err, Error::Decode(m) if m.contains("empty source")),
        "{err:?}"
    );
}

#[test]
fn a_zero_length_file_is_named_as_empty() {
    let file = tempfile::NamedTempFile::new().expect("temp file");
    let err = SymphoniaDecoder::open(MediaSource::Path(file.path().to_path_buf()))
        .err()
        .expect("an empty file cannot hold a container");
    assert!(
        matches!(&err, Error::Decode(m) if m.contains("empty source")),
        "{err:?}"
    );
}

// ---------------------------------------------------------------------------
// Truncated input
// ---------------------------------------------------------------------------

#[test]
fn a_truncated_wav_never_reports_more_audio_than_its_bytes_hold() {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let full = wav_bytes(spec, 8000, sine);
    let cut = full[..full.len() * 2 / 5].to_vec();
    assert!(cut.len() < full.len());
    // The header (44 bytes) is intact and the data chunk claims 8000 frames, but
    // only about a third of the bytes are present.
    let frames = decode_hostile(cut, Some("wav"), "truncated wav");
    assert!(
        frames.is_none_or(|f| f < 8000),
        "a truncated file must not yield the claimed 8000 frames: {frames:?}"
    );
}

#[test]
fn a_truncated_flac_never_reports_more_audio_than_its_bytes_hold() {
    let full = std::fs::read(fixture("sine.flac")).expect("fixture");
    let cut = full[..full.len() / 3].to_vec();
    // STREAMINFO still claims FIXTURE_FRAMES samples; only a third of the audio
    // is there. The damage may show as a defined error or as a short stream.
    let frames = decode_hostile(cut, Some("flac"), "truncated flac");
    assert!(
        frames.is_none_or(|f| f < FIXTURE_FRAMES),
        "a truncated file must not yield the full {FIXTURE_FRAMES} frames: {frames:?}"
    );
}

/// A cut file is a loud decode failure, not an end of stream.
///
/// The RIFF/WAVE `data` chunk states the number of samples the file holds.
/// A file that ends before that count was delivered was cut, and saying so is
/// the whole point: a half-downloaded track must not play short as if it were
/// complete.
#[test]
fn a_truncated_wav_is_a_loud_failure_and_not_an_end_of_stream() {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let full = wav_bytes(spec, 8000, sine);
    // Cut inside the sample data, in the middle of a frame.
    let cut = full[..full.len() * 2 / 5].to_vec();
    assert!(cut.len() < full.len());
    let mut d = SymphoniaDecoder::open(reader_source(cut, Some("wav")))
        .expect("a truncated wav still opens");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        frames < 8000,
        "a truncated file must not yield the claimed 8000 frames: {frames}"
    );
    match outcome {
        Drained::Failed(Error::Decode(message)) => assert!(
            message.contains("truncated"),
            "the failure must name the truncation: {message}"
        ),
        other => panic!("a cut file must fail loudly, not end the stream: {other:?}"),
    }
}

/// The limit of the rule, named so it is not mistaken for a fix.
///
/// FLAC states its sample count in the `STREAMINFO` block, but a damaged FLAC
/// frame is resynchronised by the container reader, so a corrupt file
/// delivers fewer frames than it declares too. A frame count cannot tell the
/// two apart, so a truncated FLAC is not called truncated.
#[test]
fn a_truncated_flac_is_not_mistaken_for_a_cut_file() {
    let full = std::fs::read(fixture("sine.flac")).expect("fixture");
    let cut = full[..full.len() / 3].to_vec();
    let mut d = SymphoniaDecoder::open(reader_source(cut, Some("flac")))
        .expect("a truncated flac still opens");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        frames < FIXTURE_FRAMES,
        "a truncated file must not yield the full {FIXTURE_FRAMES} frames: {frames}"
    );
    if let Drained::Failed(Error::Decode(message)) = &outcome {
        assert!(
            !message.contains("truncated"),
            "a FLAC has no exact oracle, so it must not be called truncated: {message}"
        );
    }
}

// ---------------------------------------------------------------------------
// FLAC: the shape of its header, and why it has no truncation oracle
// ---------------------------------------------------------------------------

/// The metadata blocks of a FLAC file: their type, where their payload starts
/// and how long it is.
///
/// A FLAC file is the `fLaC` marker, then a chain of metadata blocks (a type
/// byte, a 24 bit length and the payload), then the audio frames. The tests
/// below need the `STREAMINFO` block: it is the one that states what the file
/// holds.
fn flac_metadata(bytes: &[u8]) -> Vec<(u8, usize, usize)> {
    assert_eq!(&bytes[..4], b"fLaC", "the fixture is a FLAC file");
    let mut blocks = Vec::new();
    let mut at = 4;
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

/// The offset of the first audio frame: behind every metadata block.
fn flac_audio_start(bytes: &[u8]) -> usize {
    let (_, at, len) = *flac_metadata(bytes)
        .last()
        .expect("the fixture carries metadata blocks");
    at + len
}

/// The number of samples the `STREAMINFO` block declares.
///
/// The block holds the sample rate (20 bits), the channel count (3 bits) and
/// the bit depth (5 bits) in the first 28 bits of its last eight bytes, and
/// the total number of samples in the 36 bits behind them. A count of zero
/// means "length unknown" in the format.
fn flac_declared_samples(payload: &[u8; 34]) -> u64 {
    let bits = u64::from_be_bytes(payload[10..18].try_into().expect("eight bytes"));
    bits & ((1 << 36) - 1)
}

/// Writes the number of samples the `STREAMINFO` block declares.
fn flac_set_declared_samples(payload: &mut [u8; 34], samples: u64) {
    let mut bits = u64::from_be_bytes(payload[10..18].try_into().expect("eight bytes"));
    bits = (bits & !((1 << 36) - 1)) | (samples & ((1 << 36) - 1));
    payload[10..18].copy_from_slice(&bits.to_be_bytes());
}

/// The largest number of samples one frame of the stream carries.
///
/// The block states the minimum and the maximum block size in samples.
fn flac_max_block(payload: &[u8; 34]) -> u64 {
    u64::from(u16::from_be_bytes(
        payload[2..4].try_into().expect("two bytes"),
    ))
}

/// The smallest number of bytes one frame of the stream occupies.
///
/// The block states the minimum and the maximum frame size in bytes. Zero
/// means "unknown" in the format.
fn flac_min_frame(payload: &[u8; 34]) -> u64 {
    u64::from(payload[4]) << 16 | u64::from(payload[5]) << 8 | u64::from(payload[6])
}

/// Changes the type of a metadata block, keeping its "last block" flag.
///
/// Type 126 is unknown to the container reader: it skips a block of that
/// type like any other block it does not know.
fn flac_set_block_type(bytes: &mut [u8], payload_at: usize, kind: u8) {
    let header = payload_at - 4;
    bytes[header] = (bytes[header] & 0x80) | kind;
}

/// The reason a FLAC file has no truncation oracle.
///
/// The `STREAMINFO` block states the number of samples exactly, so the file
/// says how much audio it holds. A damaged FLAC frame is dropped by the
/// reader - its CRC-16 does not match - and the reader resynchronises at the
/// next frame. A corrupt file is therefore short of samples exactly like a
/// cut one, while it holds all of its bytes: the sample count cannot tell
/// the two apart, and FLAC states no byte count that could.
#[test]
fn a_corrupt_flac_is_short_of_the_samples_it_declares_and_still_ends_cleanly() {
    let mut bytes = std::fs::read(fixture("sine.flac")).expect("fixture");
    let (_, payload) = flac_stream_info(&bytes);
    let declared = flac_declared_samples(&payload);
    assert_eq!(
        declared, FIXTURE_FRAMES,
        "the fixture states its sample count"
    );
    // Damage inside the audio: the bytes stay, the frame does not.
    let middle = bytes.len() / 2;
    for b in &mut bytes[middle..middle + 16] {
        *b ^= 0xA5;
    }
    let mut d = SymphoniaDecoder::open(reader_source(bytes, Some("flac"))).expect("open");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        matches!(outcome, Drained::End),
        "a damaged file is not a cut file: {outcome:?}"
    );
    assert!(
        frames < declared,
        "the damaged frame is lost: {frames} of the {declared} samples"
    );
    assert!(
        frames > declared / 2,
        "only the damaged frame is lost: {frames} of {declared}"
    );
}

/// A FLAC file that was cut in the middle of the audio ends cleanly.
///
/// The file is short of the samples its `STREAMINFO` block declares - the
/// first half of the condition the MPEG audio rule uses. The second half is
/// missing: FLAC states no byte count, so nothing says how many bytes the
/// file should hold. A damaged FLAC file is short of samples in the same way
/// (see the corrupt file above), so the shortfall alone is not evidence of a
/// cut file.
#[test]
fn a_flac_cut_in_the_middle_of_the_audio_ends_cleanly_and_is_not_called_truncated() {
    let full = std::fs::read(fixture("sine.flac")).expect("fixture");
    let cut = full[..full.len() / 2].to_vec();
    let mut d = SymphoniaDecoder::open(reader_source(cut, Some("flac")))
        .expect("a truncated flac still opens");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(frames > 0, "the file holds whole frames: {frames}");
    assert!(
        frames < FIXTURE_FRAMES,
        "the file is short of the samples it declares: {frames}"
    );
    if let Drained::Failed(e) = &outcome {
        panic!("a FLAC file states no byte count, so it must not be called truncated: {e:?}");
    }
}

/// An intact FLAC file delivers exactly the samples its `STREAMINFO` block
/// declares.
///
/// The container reader takes its frame count from that block, so the two
/// agree without a correction - unlike MPEG audio, where the reader has to
/// subtract the encoder delay and padding of the LAME extension.
#[test]
fn an_intact_flac_delivers_exactly_the_samples_its_stream_info_declares() {
    let bytes = std::fs::read(fixture("sine.flac")).expect("fixture");
    let (_, payload) = flac_stream_info(&bytes);
    let declared = flac_declared_samples(&payload);
    assert_eq!(
        declared, FIXTURE_FRAMES,
        "the fixture states its sample count"
    );
    let mut d = SymphoniaDecoder::open(reader_source(bytes, Some("flac"))).expect("open");
    assert_eq!(
        d.duration(),
        Some(Duration::from_millis(1500)),
        "the declared sample count is the duration"
    );
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(matches!(outcome, Drained::End), "{outcome:?}");
    assert_eq!(
        frames, declared,
        "the file holds every sample its STREAMINFO block declares"
    );
}

/// A FLAC file whose `STREAMINFO` block states no sample count has no
/// duration and nothing to compare against.
///
/// A total sample count of zero means "length unknown" in the format. The
/// reader then reports no frame count at all, so there is no claim to keep:
/// the file plays and ends as it always did.
#[test]
fn a_flac_that_states_no_sample_count_has_no_duration_and_no_claim() {
    let mut bytes = std::fs::read(fixture("sine.flac")).expect("fixture");
    let (at, mut payload) = flac_stream_info(&bytes);
    flac_set_declared_samples(&mut payload, 0);
    bytes[at..at + 34].copy_from_slice(&payload);
    let mut d = SymphoniaDecoder::open(reader_source(bytes, Some("flac"))).expect("open");
    assert_eq!(
        d.duration(),
        None,
        "a file of unknown length has no duration"
    );
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(matches!(outcome, Drained::End), "{outcome:?}");
    assert_eq!(frames, FIXTURE_FRAMES, "the audio is all there");
}

/// A header that lies is not a cut file.
///
/// The `STREAMINFO` block claims twice the samples the file holds. The file
/// holds all of its bytes, so it was not cut: it ends cleanly, like the
/// damaged file it resembles.
#[test]
fn a_flac_whose_stream_info_lies_about_the_sample_count_is_not_a_cut_file() {
    let mut bytes = std::fs::read(fixture("sine.flac")).expect("fixture");
    let (at, mut payload) = flac_stream_info(&bytes);
    flac_set_declared_samples(&mut payload, FIXTURE_FRAMES * 2);
    bytes[at..at + 34].copy_from_slice(&payload);
    let mut d = SymphoniaDecoder::open(reader_source(bytes, Some("flac"))).expect("open");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        matches!(outcome, Drained::End),
        "a wrong header is not a cut file: {outcome:?}"
    );
    assert_eq!(frames, FIXTURE_FRAMES, "the file holds all of its frames");
}

/// A FLAC file without a `STREAMINFO` block cannot be decoded.
///
/// The block is the one that states the audio parameters, so a file without
/// it has nothing to decode: the reader cannot recognise a frame without them
/// and runs into the end of the file. The file is refused with a defined
/// error - never a panic, never a hang.
///
/// What the refusal *says* is the container reader's business: it reports the
/// end of the stream, which for a file that is not at its end is a poor
/// diagnosis. The refusal itself is the documented behaviour.
#[test]
fn a_flac_without_a_stream_info_block_is_refused() {
    let mut bytes = std::fs::read(fixture("sine.flac")).expect("fixture");
    let (at, _) = flac_stream_info(&bytes);
    flac_set_block_type(&mut bytes, at, 126);
    let err = SymphoniaDecoder::open(reader_source(bytes, Some("flac")))
        .err()
        .expect("a FLAC file without a STREAMINFO block cannot be decoded");
    assert_defined_open_failure(&err, "a FLAC file without a STREAMINFO block");
}

/// A metadata block the reader does not know is skipped.
///
/// The padding block of the fixture becomes an unknown block. The reader
/// skips what it does not know, so the audio is unaffected and the file still
/// holds every sample it declares.
#[test]
fn a_flac_with_an_unknown_metadata_block_still_decodes() {
    let mut bytes = std::fs::read(fixture("sine.flac")).expect("fixture");
    let padding = flac_metadata(&bytes)
        .into_iter()
        .find(|(kind, _, _)| *kind == 1)
        .expect("the fixture carries a padding block");
    flac_set_block_type(&mut bytes, padding.1, 126);
    let mut d = SymphoniaDecoder::open(reader_source(bytes, Some("flac"))).expect("open");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(matches!(outcome, Drained::End), "{outcome:?}");
    assert_eq!(frames, FIXTURE_FRAMES, "the audio is unaffected");
}

/// What the format does offer, and why it is not an oracle.
///
/// The `STREAMINFO` block declares the minimum frame size in bytes and the
/// maximum block size in samples. Together with the sample count they give a
/// *proven lower bound* on the number of audio bytes a complete file must
/// hold: the number of frames the declared samples need at the declared
/// maximum block size, times the declared minimum frame size. A file below
/// that bound cannot hold the samples it declares, so it was cut.
///
/// The bound is far below the size of a real file, and it depends on the
/// content: the smallest frame of a stream is usually its last one, and a
/// file that ends in silence has a very small last frame. So the bound fires
/// for some cuts and not for others - a loud failure on it would be an
/// unstable promise, which is why the decoder does not make one.
#[test]
fn the_lower_bound_the_format_offers_is_too_loose_to_be_an_oracle() {
    let bytes = std::fs::read(fixture("sine.flac")).expect("fixture");
    let (_, payload) = flac_stream_info(&bytes);
    let samples = flac_declared_samples(&payload);
    let max_block = flac_max_block(&payload);
    let min_frame = flac_min_frame(&payload);
    assert!(
        max_block > 0 && min_frame > 0,
        "the fixture states its frame bounds"
    );
    // The fewest frames that can carry the declared samples.
    let frames = samples.div_ceil(max_block);
    let bound = frames * min_frame;
    let audio = (bytes.len() - flac_audio_start(&bytes)) as u64;
    assert!(
        bound < audio,
        "a complete file holds more than the bound: {bound} of {audio} bytes"
    );
    assert!(
        bound * 2 < audio,
        "the bound is far below the file: {bound} of {audio} bytes - it only catches deep cuts"
    );
}

/// A file that holds everything its container declares ends with a plain end
/// of stream, and the frame count is the one the container states.
#[test]
fn a_complete_file_ends_with_an_end_of_stream() {
    for name in ["sine.flac", "sine.mp3", "sine.ogg", "sine_alac.m4a"] {
        let bytes = std::fs::read(fixture(name)).expect("fixture");
        let extension = name.rsplit('.').next().map(str::to_owned);
        let mut d = SymphoniaDecoder::open(reader_source(bytes, extension.as_deref()))
            .expect("a complete fixture opens");
        let (frames, outcome) = drain_bounded(&mut d);
        assert!(matches!(outcome, Drained::End), "{name}: {outcome:?}");
        assert_eq!(frames, FIXTURE_FRAMES, "{name}: the whole file was decoded");
    }
}

// ---------------------------------------------------------------------------
// MPEG audio: the Xing/Info header as a truncation oracle
// ---------------------------------------------------------------------------

/// The position of the Xing/Info tag in the MPEG fixture.
///
/// The first MPEG frame follows the ID3v2 tag and carries the tag behind its
/// side information.
fn xing_tag(bytes: &[u8]) -> usize {
    let mut found = None;
    for (i, window) in bytes.windows(4).enumerate() {
        if window == b"Xing" || window == b"Info" {
            assert!(found.is_none(), "the fixture carries more than one tag");
            found = Some(i);
        }
    }
    found.expect("the fixture carries a Xing/Info tag")
}

/// A cut MPEG file is a loud decode failure when its Xing/Info header says
/// what the file should hold.
///
/// The header states the number of MPEG frames and the number of bytes the
/// audio occupies. A file that ends before both are there was cut, and
/// saying so is the whole point: a half-downloaded track must not play short
/// as if it were complete.
#[test]
fn a_truncated_mp3_with_a_xing_header_is_a_loud_failure() {
    let full = std::fs::read(fixture("sine.mp3")).expect("fixture");
    // Cut in the middle of the audio data.
    let cut = full[..full.len() * 3 / 5].to_vec();
    assert!(cut.len() < full.len());
    let mut d = SymphoniaDecoder::open(reader_source(cut, Some("mp3")))
        .expect("a truncated mp3 still opens");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        frames < FIXTURE_FRAMES,
        "a truncated file must not yield the claimed {FIXTURE_FRAMES} frames: {frames}"
    );
    match outcome {
        Drained::Failed(Error::Decode(message)) => {
            assert!(
                message.contains("truncated"),
                "the failure must name the truncation: {message}"
            );
            assert!(
                message.contains(&format!("of the {FIXTURE_FRAMES} frames")),
                "the failure must name the count the header declares: {message}"
            );
        }
        other => panic!("a cut file must fail loudly, not end the stream: {other:?}"),
    }
}

/// The same rule for a file on disk: the header is read from the file, not
/// from a stream.
#[test]
fn a_truncated_mp3_file_with_a_xing_header_is_a_loud_failure() {
    let full = std::fs::read(fixture("sine.mp3")).expect("fixture");
    let cut = full[..full.len() * 3 / 5].to_vec();
    let file = tempfile::NamedTempFile::new().expect("temp file");
    std::fs::write(file.path(), &cut).expect("write");
    let mut d = SymphoniaDecoder::open(MediaSource::Path(file.path().to_path_buf()))
        .expect("a truncated mp3 still opens");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(frames < FIXTURE_FRAMES, "only {frames} frames");
    match outcome {
        Drained::Failed(Error::Decode(message)) => assert!(
            message.contains("truncated"),
            "the failure must name the truncation: {message}"
        ),
        other => panic!("a cut file must fail loudly, not end the stream: {other:?}"),
    }
}

/// The same file, intact: it holds everything its header declares, so it ends
/// with a plain end of stream and the declared number of frames.
#[test]
fn an_intact_mp3_with_a_xing_header_ends_with_the_declared_count() {
    let full = std::fs::read(fixture("sine.mp3")).expect("fixture");
    let mut d = SymphoniaDecoder::open(reader_source(full, Some("mp3"))).expect("open");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(matches!(outcome, Drained::End), "{outcome:?}");
    assert_eq!(
        frames, FIXTURE_FRAMES,
        "the file holds every frame its header declares"
    );
}

/// The limit of the rule, named so it is not mistaken for a fix.
///
/// Without a Xing/Info header there is no oracle: a reader can only estimate
/// the frame count from the bitrate, and an estimate is not evidence of a cut
/// file. A truncated MPEG file without the header therefore still ends with
/// an end of stream, exactly as before.
#[test]
fn a_truncated_mp3_without_a_xing_header_still_ends_cleanly() {
    let mut full = std::fs::read(fixture("sine.mp3")).expect("fixture");
    // Take the tag out of the first frame: the file states nothing about
    // what it holds any more, so the reader falls back to its estimate.
    let tag = xing_tag(&full);
    full[tag..tag + 4].copy_from_slice(b"XXXX");
    let cut = full[..full.len() * 3 / 5].to_vec();
    let mut d = SymphoniaDecoder::open(reader_source(cut, Some("mp3"))).expect("open");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        matches!(outcome, Drained::End),
        "without a header there is no oracle: {outcome:?}"
    );
    assert!(
        frames < FIXTURE_FRAMES,
        "the file is cut, but nothing states what it should hold: {frames}"
    );
}

/// A header that lies is not a cut file.
///
/// The frame count in the tag is larger than the file could ever hold, but
/// the file holds every byte the tag declares: the header is wrong, the file
/// is not cut. The byte count is what tells the two apart, so this file ends
/// cleanly - calling it truncated would be crying wolf.
#[test]
fn a_xing_header_that_lies_about_the_frame_count_is_not_a_truncation() {
    let mut full = std::fs::read(fixture("sine.mp3")).expect("fixture");
    let tag = xing_tag(&full);
    // The frame count follows the tag id and its flags.
    full[tag + 8..tag + 12].copy_from_slice(&u32::MAX.to_be_bytes());
    let mut d = SymphoniaDecoder::open(reader_source(full, Some("mp3"))).expect("open");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        matches!(outcome, Drained::End),
        "a wrong header is not a cut file: {outcome:?}"
    );
    assert!(
        frames >= FIXTURE_FRAMES,
        "the file holds all of its frames: {frames}"
    );
}

/// The other side of the same coin: damage is not a cut file.
///
/// The header is intact and states the whole file, but the damage in the
/// middle costs frames - not bytes. A damaged MPEG file therefore still
/// ends cleanly, as it always did.
#[test]
fn a_damaged_mp3_with_a_xing_header_is_not_called_truncated() {
    let mut bytes = std::fs::read(fixture("sine.mp3")).expect("fixture");
    let len = bytes.len();
    for b in &mut bytes[len / 2..len / 2 + 300] {
        *b ^= 0xA5;
    }
    let mut d = SymphoniaDecoder::open(reader_source(bytes, Some("mp3"))).expect("open");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        matches!(outcome, Drained::End),
        "a damaged file is not a cut file: {outcome:?}"
    );
    assert!(frames > FIXTURE_FRAMES / 2, "only {frames} frames");
}

/// A seek must not turn a complete file into a truncated one.
///
/// After a seek the stream is read from the target, so the frames still to
/// come are counted from there and the declared count is what is left of it.
#[test]
fn a_seek_in_an_intact_mp3_does_not_turn_it_into_a_truncation() {
    let full = std::fs::read(fixture("sine.mp3")).expect("fixture");
    let mut d = SymphoniaDecoder::open(reader_source(full, Some("mp3"))).expect("open");
    d.seek(Duration::from_millis(500)).expect("seek");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        matches!(outcome, Drained::End),
        "a seek is not a cut file: {outcome:?}"
    );
    assert!(frames > 0, "the rest of the file is still there");
}

// ---------------------------------------------------------------------------
// AIFF: the COMM and SSND chunks as a truncation oracle
// ---------------------------------------------------------------------------

/// The 80-bit IEEE 754 extended float an AIFF file stores a sample rate in.
///
/// The value is `2^(exponent - 16383) * mantissa / 2^63`, with the
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
/// chunk that holds `frames` frames of 16 bit mono audio.
///
/// The `COMM` chunk states the number of sample frames, the `SSND` chunk the
/// number of bytes of sound data: the two numbers the truncation oracle
/// compares the file against.
fn aiff_bytes(frames: u32) -> Vec<u8> {
    let block_align = 2u32; // 16 bit mono
    let data_len = u64::from(frames) * u64::from(block_align);
    // The `SSND` chunk holds its own eight byte header next to the sound
    // data.
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
    out.extend(std::iter::repeat_n(0u8, data_len as usize));
    out
}

/// A cut AIFF file is a loud decode failure.
///
/// The `COMM` chunk states the number of sample frames and the `SSND` chunk
/// the number of bytes of sound data. A file that is short of frames *and* of
/// bytes was cut, and saying so is the whole point: a half-downloaded track
/// must not play short as if it were complete.
#[test]
fn a_truncated_aiff_is_a_loud_failure_and_not_an_end_of_stream() {
    let full = aiff_bytes(1000);
    // Cut inside the sound data, in the middle of a frame.
    let cut = full[..full.len() - 3].to_vec();
    assert!(cut.len() < full.len());
    let mut d = SymphoniaDecoder::open(reader_source(cut, Some("aiff")))
        .expect("a truncated aiff still opens");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        frames < 1000,
        "a truncated file must not yield the claimed 1000 frames: {frames}"
    );
    match outcome {
        Drained::Failed(Error::Decode(message)) => {
            assert!(
                message.contains("truncated"),
                "the failure must name the truncation: {message}"
            );
            assert!(
                message.contains("of the 1000 frames"),
                "the failure must name the count the file declares: {message}"
            );
        }
        other => panic!("a cut file must fail loudly, not end the stream: {other:?}"),
    }
}

/// The same file, intact: it holds everything its chunks declare, so it ends
/// with a plain end of stream and exactly the declared number of frames.
///
/// The count is the one the `COMM` chunk states, not the one the reader
/// reports: the reader counts the eight bytes of the `SSND` header as audio
/// and would say 1004 frames for 1000 frames of sound data.
#[test]
fn an_intact_aiff_ends_with_the_declared_count() {
    let full = aiff_bytes(1000);
    let mut d = SymphoniaDecoder::open(reader_source(full, Some("aiff"))).expect("open");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(matches!(outcome, Drained::End), "{outcome:?}");
    assert_eq!(
        frames, 1000,
        "the file holds every frame its COMM chunk declares"
    );
}

/// Damage is not a cut file.
///
/// The samples in the middle of the file are damaged, but the file holds
/// every byte its `SSND` chunk declares: the header and the file agree, so
/// the file keeps playing and ends cleanly.
#[test]
fn a_damaged_aiff_that_holds_all_its_bytes_is_not_called_truncated() {
    let mut bytes = aiff_bytes(1000);
    let len = bytes.len();
    for b in &mut bytes[len / 2..len / 2 + 300] {
        *b ^= 0xA5;
    }
    let mut d = SymphoniaDecoder::open(reader_source(bytes, Some("aiff"))).expect("open");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        matches!(outcome, Drained::End),
        "a damaged file is not a cut file: {outcome:?}"
    );
    assert_eq!(frames, 1000, "damage costs samples, not frames");
}

/// A header that lies is not a cut file.
///
/// The `COMM` chunk claims far more frames than the file holds, but the file
/// holds every byte its `SSND` chunk declares: the header is wrong, the file
/// is not cut. The byte count is what tells the two apart, so this file ends
/// cleanly - calling it truncated would be crying wolf.
#[test]
fn an_aiff_header_that_lies_about_the_frame_count_is_not_a_truncation() {
    let mut bytes = aiff_bytes(1000);
    // The sample frame count of the `COMM` chunk: the chunk header is 8
    // bytes behind the `FORM` header, the channels take two more.
    let at = 12 + 8 + 2;
    bytes[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    let mut d = SymphoniaDecoder::open(reader_source(bytes, Some("aiff"))).expect("open");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        matches!(outcome, Drained::End),
        "a wrong header is not a cut file: {outcome:?}"
    );
    assert_eq!(frames, 1000, "the file holds all of its frames");
}

/// A file on disk behaves like a stream: the header is read from the file,
/// not from a stream.
#[test]
fn a_truncated_aiff_file_on_disk_is_a_loud_failure() {
    let full = aiff_bytes(1000);
    let cut = full[..full.len() - 3].to_vec();
    let file = tempfile::NamedTempFile::new().expect("temp file");
    std::fs::write(file.path(), &cut).expect("write");
    let mut d = SymphoniaDecoder::open(MediaSource::Path(file.path().to_path_buf()))
        .expect("a truncated aiff still opens");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(frames < 1000, "only {frames} frames");
    match outcome {
        Drained::Failed(Error::Decode(message)) => assert!(
            message.contains("truncated"),
            "the failure must name the truncation: {message}"
        ),
        other => panic!("a cut file must fail loudly, not end the stream: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// MP4/M4A: the media data box and the sample tables as a truncation oracle
// ---------------------------------------------------------------------------

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
fn mp4_pcm_bytes(frames: u32, rate: u32) -> Vec<u8> {
    let movie_timescale = 1000u32;
    let movie_duration = frames * movie_timescale / rate;

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
    mdhd.extend_from_slice(&rate.to_be_bytes()); // timescale
    mdhd.extend_from_slice(&frames.to_be_bytes()); // duration
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
    sowt.extend_from_slice(&1u16.to_be_bytes()); // data reference index
    sowt.extend_from_slice(&0u16.to_be_bytes()); // version
    sowt.extend_from_slice(&0u16.to_be_bytes()); // revision
    sowt.extend_from_slice(&0u32.to_be_bytes()); // vendor
    sowt.extend_from_slice(&2u16.to_be_bytes()); // channels
    sowt.extend_from_slice(&16u16.to_be_bytes()); // sample size
    sowt.extend_from_slice(&0u16.to_be_bytes()); // compression id
    sowt.extend_from_slice(&0u16.to_be_bytes()); // packet size
    sowt.extend_from_slice(&(rate << 16).to_be_bytes()); // sample rate (16.16)
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
    file.extend_from_slice(&mdat);
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

/// A cut MP4 file is a loud failure, not a short play.
///
/// The media data box states how many bytes of media data the file holds. A
/// file that is shorter than that was cut, and the container reader - which
/// scans the whole file before it plays anything - refuses such a file. The
/// decoder names the cut instead of letting the bare "end of stream" of the
/// reader stand.
#[test]
fn a_truncated_mp4_is_a_loud_failure() {
    let full = mp4_pcm_bytes(1000, RATE);
    // Cut inside the media data, in the middle of a frame.
    let cut = full[..full.len() - 1000].to_vec();
    assert!(cut.len() < full.len());
    let err = SymphoniaDecoder::open(reader_source(cut, Some("m4a")))
        .err()
        .expect("a truncated mp4 must not open");
    match err {
        Error::Decode(message) => assert!(
            message.contains("truncated"),
            "the failure must name the truncation: {message}"
        ),
        other => panic!("a cut file must fail loudly: {other:?}"),
    }
}

/// The same file on disk: the header is read from the file, not from a
/// stream.
#[test]
fn a_truncated_mp4_file_on_disk_is_a_loud_failure() {
    let full = mp4_pcm_bytes(1000, RATE);
    let cut = full[..full.len() - 1000].to_vec();
    let file = tempfile::NamedTempFile::new().expect("temp file");
    std::fs::write(file.path(), &cut).expect("write");
    let err = SymphoniaDecoder::open(MediaSource::Path(file.path().to_path_buf()))
        .err()
        .expect("a truncated mp4 must not open");
    assert!(
        matches!(&err, Error::Decode(message) if message.contains("truncated")),
        "{err:?}"
    );
}

/// The same file, intact: it holds everything its boxes declare, so it ends
/// with a plain end of stream and the number of frames the media header
/// states.
#[test]
fn an_intact_mp4_ends_with_the_declared_count() {
    let full = mp4_pcm_bytes(1000, RATE);
    let mut d = SymphoniaDecoder::open(reader_source(full, Some("m4a"))).expect("open");
    assert_eq!(
        d.duration().map(|d| d.as_millis()),
        Some(22),
        "the media header states the duration"
    );
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(matches!(outcome, Drained::End), "{outcome:?}");
    assert_eq!(
        frames, 1000,
        "the file holds every frame its media header declares"
    );
}

/// Damage is not a cut file.
///
/// The samples in the middle of the file are damaged, but the file holds
/// every byte its media data box declares: the boxes and the file agree, so
/// the file keeps playing and ends cleanly.
#[test]
fn a_damaged_mp4_that_holds_all_its_bytes_is_not_called_truncated() {
    let mut bytes = std::fs::read(fixture("sine_alac.m4a")).expect("fixture");
    let len = bytes.len();
    for b in &mut bytes[len / 2..len / 2 + 300] {
        *b ^= 0xA5;
    }
    let mut d = SymphoniaDecoder::open(reader_source(bytes, Some("m4a"))).expect("open");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        matches!(outcome, Drained::End),
        "a damaged file is not a cut file: {outcome:?}"
    );
    assert!(
        frames < FIXTURE_FRAMES,
        "the damaged frame is lost: {frames} of the {FIXTURE_FRAMES} frames"
    );
}

/// A cut MP4 file with the movie box behind the media data.
///
/// The file is the other way round: the media data comes first and the movie
/// box behind it. A cut file then has no movie box at all - but the media
/// data box still states the bytes that are missing, so the cut is named all
/// the same.
#[test]
fn a_truncated_mp4_with_the_movie_box_at_the_end_is_a_loud_failure() {
    let bytes = std::fs::read(fixture("sine_alac.m4a")).expect("fixture");
    // Cut the file inside the media data: the movie box is behind it and is
    // gone with it.
    let cut = bytes[..bytes.len() / 2].to_vec();
    assert!(
        !cut.windows(4).any(|w| w == b"moov"),
        "the cut file has no movie box left"
    );
    let err = SymphoniaDecoder::open(reader_source(cut, Some("m4a")))
        .err()
        .expect("a truncated mp4 must not open");
    assert!(
        matches!(&err, Error::Decode(message) if message.contains("truncated")),
        "{err:?}"
    );
}

/// A file whose sample table points beyond the end of the file.
///
/// The media data box is intact, so the file opens - but the samples it
/// names are not there. The stream ends before the media header's count is
/// reached, and the file is shorter than the sample table says: it was cut.
#[test]
fn an_mp4_whose_sample_table_points_beyond_the_file_is_a_loud_failure() {
    let mut bytes = mp4_pcm_bytes(1000, RATE);
    // The chunk offset table: move the first chunk behind the end of the
    // file.
    let at = mp4_offset_box(&bytes, b"stco");
    let offset = u32::from_be_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
    bytes[at..at + 4].copy_from_slice(&(offset + 100_000).to_be_bytes());
    let mut d = SymphoniaDecoder::open(reader_source(bytes, Some("m4a"))).expect("open");
    let (frames, outcome) = drain_bounded(&mut d);
    assert_eq!(frames, 0, "the file holds no sample that is there");
    match outcome {
        Drained::Failed(Error::Decode(message)) => assert!(
            message.contains("truncated"),
            "the failure must name the truncation: {message}"
        ),
        other => panic!("a cut file must fail loudly, not end the stream: {other:?}"),
    }
}

/// A file without the sample tables states nothing about its payload.
///
/// The sample size box is what states the number of samples and their sizes;
/// without it the container reader refuses the file. The refusal is a
/// defined error - never a panic, never a hang - and the decoder adds no
/// claim of its own to it.
#[test]
fn an_mp4_without_the_sample_size_box_is_refused() {
    let mut bytes = std::fs::read(fixture("sine_alac.m4a")).expect("fixture");
    let at = bytes
        .windows(4)
        .position(|w| w == b"stsz")
        .expect("the sample size box");
    bytes[at..at + 4].copy_from_slice(b"stzX");
    let err = SymphoniaDecoder::open(reader_source(bytes, Some("m4a")))
        .err()
        .expect("a file without the sample tables cannot be decoded");
    assert_defined_open_failure(&err, "an mp4 without the sample size box");
}

/// A file without the chunk offsets states nothing about where its samples
/// are.
#[test]
fn an_mp4_without_the_chunk_offsets_is_refused() {
    let mut bytes = std::fs::read(fixture("sine_alac.m4a")).expect("fixture");
    let at = bytes
        .windows(4)
        .position(|w| w == b"stco")
        .expect("the chunk offset box");
    bytes[at..at + 4].copy_from_slice(b"stcX");
    let err = SymphoniaDecoder::open(reader_source(bytes, Some("m4a")))
        .err()
        .expect("a file without the chunk offsets cannot be decoded");
    assert_defined_open_failure(&err, "an mp4 without the chunk offsets");
}

/// The repository fixtures: an intact MP4 file ends with the number of
/// frames its media header states, and a cut one fails loudly.
#[test]
fn the_mp4_fixtures_are_read_to_their_end_and_a_cut_one_is_named() {
    for (name, frames) in [("sine_alac.m4a", 66_150u64), ("sine_aac.m4a", 67_584)] {
        let bytes = std::fs::read(fixture(name)).expect("fixture");
        let mut d = SymphoniaDecoder::open(reader_source(bytes.clone(), Some("m4a")))
            .expect("a complete fixture opens");
        let (got, outcome) = drain_bounded(&mut d);
        assert!(matches!(outcome, Drained::End), "{name}: {outcome:?}");
        assert_eq!(got, frames, "{name}: the whole file was decoded");

        let cut = bytes[..bytes.len() / 2].to_vec();
        let err = SymphoniaDecoder::open(reader_source(cut, Some("m4a")))
            .err()
            .expect("a truncated fixture must not open");
        assert!(
            matches!(&err, Error::Decode(message) if message.contains("truncated")),
            "{name}: {err:?}"
        );
    }
}

/// A seek must not turn a complete file into a truncated one.
///
/// After a seek the stream is read from the target, so the frames still to
/// come are counted from there and the declared count is what is left of it.
#[test]
fn a_seek_in_an_intact_mp4_does_not_turn_it_into_a_truncation() {
    let bytes = std::fs::read(fixture("sine_alac.m4a")).expect("fixture");
    let mut d = SymphoniaDecoder::open(reader_source(bytes, Some("m4a"))).expect("open");
    d.seek(Duration::from_millis(500)).expect("seek");
    let (frames, outcome) = drain_bounded(&mut d);
    assert!(
        matches!(outcome, Drained::End),
        "a seek is not a cut file: {outcome:?}"
    );
    assert!(frames > 0, "the rest of the file is still there");
}

// ---------------------------------------------------------------------------
// Headers that lie about their format
// ---------------------------------------------------------------------------

#[test]
fn a_content_signature_over_garbage_is_a_defined_outcome() {
    let noise = noise_bytes(4096);

    let mut id3 = b"ID3\x04\x00\x00\x00\x00\x02\x00".to_vec(); // tag says 256 bytes
    id3.extend_from_slice(&noise);
    let mut riff = b"RIFF\xff\xff\xff\xffWAVEfmt ".to_vec(); // chunk says 4 GiB
    riff.extend_from_slice(&noise);
    let mut mp4 = vec![0, 0, 0, 0x20]; // box says 32 bytes
    mp4.extend_from_slice(b"ftypM4A ");
    mp4.extend_from_slice(&noise);
    let mut flac = b"fLaC".to_vec();
    flac.extend_from_slice(&noise);
    let mut ogg = b"OggS\x00\x02".to_vec();
    ogg.extend_from_slice(&noise);
    let mut adts = vec![0xFF, 0xF1, 0x50, 0x80, 0x00, 0x1F, 0xFC];
    adts.extend_from_slice(&noise);
    let mut mpeg = vec![0xFF, 0xFB, 0x90, 0x00];
    mpeg.extend_from_slice(&noise);

    let cases: &[(&str, Vec<u8>, Option<&str>)] = &[
        ("id3 with a bogus tag size", id3, Some("mp3")),
        ("riff claiming 4 GiB", riff, Some("wav")),
        ("mp4 box with garbage inside", mp4, Some("m4a")),
        ("flac magic over noise", flac, Some("flac")),
        ("ogg page over noise", ogg, Some("ogg")),
        ("adts sync over noise", adts, Some("aac")),
        ("mpeg sync over noise", mpeg, Some("mp3")),
    ];
    for (label, bytes, ext) in cases {
        // `decode_hostile` panics on a hang and on an undefined error kind; a
        // decoder that opens must still end (or fail) in a defined way.
        let _ = decode_hostile(bytes.clone(), *ext, label);
    }
}

#[test]
fn pure_noise_is_not_claimed_as_content_and_stays_defined() {
    // A signature-less pattern: the factory must not claim it as content...
    let pattern = pattern_bytes(4096);
    let f = SymphoniaDecoderFactory::new();
    assert_eq!(
        f.probe(&ProbeHint {
            extension: None,
            header: &pattern,
        }),
        Confidence::NONE
    );

    // ...while random bytes wearing an `.mp3` extension still take the normal
    // path: a false MPEG sync is possible, a panic is not.
    let _ = decode_hostile(
        noise_bytes(8192),
        Some("mp3"),
        "noise with a lying extension",
    );
}

#[test]
fn an_unclaimable_input_makes_the_registry_refuse_instead_of_guessing() {
    // No extension and no signature: nothing claims it, so the registry refuses.
    let err = default_registry()
        .open(reader_source(pattern_bytes(4096), None))
        .err()
        .expect("nothing may claim an unrecognized file");
    assert!(matches!(err, Error::UnsupportedFormat(_)), "{err:?}");
}

// ---------------------------------------------------------------------------
// Probe and registry policy
// ---------------------------------------------------------------------------

#[test]
fn the_registry_decodes_by_content_even_when_the_extension_lies() {
    // FLAC bytes wearing `.mp3`: only the bytes decide, so the whole file
    // decodes. The score itself says so first.
    let flac = std::fs::read(fixture("sine.flac")).expect("fixture");
    let f = SymphoniaDecoderFactory::new();
    assert_eq!(
        f.probe(&ProbeHint {
            extension: Some("mp3"),
            header: &flac[..64],
        }),
        Confidence::CONTENT
    );
    let mut d = open_bytes(flac, Some("mp3"));
    assert_eq!(d.spec(), AudioSpec::new(RATE, 2));
    let (frames, outcome) = drain_bounded(d.as_mut());
    assert!(
        matches!(outcome, Drained::End),
        "a clean stream must end, not fail"
    );
    assert_eq!(frames, FIXTURE_FRAMES);

    // And the other way round: MP3 bytes wearing `.flac`.
    let mp3 = std::fs::read(fixture("sine.mp3")).expect("fixture");
    let mut d = open_bytes(mp3, Some("flac"));
    assert_eq!(d.spec(), AudioSpec::new(RATE, 2));
    let (frames, outcome) = drain_bounded(d.as_mut());
    assert!(matches!(outcome, Drained::End));
    assert!(
        frames.abs_diff(FIXTURE_FRAMES) <= 1152,
        "mp3 decoded {frames} frames"
    );
}

#[test]
fn a_clean_fixture_reaches_a_defined_end_of_stream() {
    let mut d = default_registry()
        .open(MediaSource::Path(fixture("sine.flac")))
        .expect("open");
    let (frames, outcome) = drain_bounded(d.as_mut());
    assert!(
        matches!(outcome, Drained::End),
        "a clean stream must end, not fail"
    );
    assert_eq!(frames, FIXTURE_FRAMES);
}

#[test]
fn the_default_registry_has_two_real_claimants_so_the_tie_rule_is_reachable() {
    let reg = default_registry();
    let names: Vec<&str> = reg.factories().iter().map(|f| f.name()).collect();
    assert_eq!(
        names,
        vec!["symphonia", "wave"],
        "registration order is the RFC 0013 tie-break"
    );
    // The tie-break is no longer dead policy: both real factories claim a
    // `RIFF`/`WAVE` signature at `Confidence::CONTENT` and both claim `.wav` by
    // extension, so the fake factories below only pin the *rule*, not its
    // reachability. `tests/selection.rs` exercises it with the real pair.
    let header = b"RIFF\x24\x00\x00\x00WAVEfmt ";
    let hint = ProbeHint {
        extension: Some("wav"),
        header,
    };
    assert_eq!(reg.factories()[0].probe(&hint), Confidence::CONTENT);
    assert_eq!(reg.factories()[1].probe(&hint), Confidence::CONTENT);
    assert_eq!(
        reg.probe(&hint).expect("both claim it").0.name(),
        "symphonia"
    );
}

/// Factory that claims every input at a fixed confidence and opens a short
/// silent decoder. Two of these make the registry's tie-break observable.
struct ClaimingFactory {
    name: &'static str,
    confidence: Confidence,
}

const NO_EXTENSIONS: &[&str] = &[];

impl DecoderFactory for ClaimingFactory {
    fn name(&self) -> &str {
        self.name
    }
    fn extensions(&self) -> &[&str] {
        NO_EXTENSIONS
    }
    fn probe(&self, _hint: &ProbeHint<'_>) -> Confidence {
        self.confidence
    }
    fn open(&self, _source: MediaSource) -> Result<Box<dyn Decoder>> {
        Ok(Box::new(MockDecoder::silence(AudioSpec::new(8_000, 1), 10)))
    }
}

/// A fake factory that claims specific extensions at a designated confidence
/// without inspecting content (RFC 0013 test helper).
struct ExtensionClaimingFactory {
    name: &'static str,
    extensions: &'static [&'static str],
    confidence: Confidence,
}

impl DecoderFactory for ExtensionClaimingFactory {
    fn name(&self) -> &str {
        self.name
    }
    fn extensions(&self) -> &[&str] {
        self.extensions
    }
    fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
        match hint.extension {
            Some(ext) if self.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)) => {
                self.confidence
            }
            _ => Confidence::NONE,
        }
    }
    fn open(&self, _source: MediaSource) -> Result<Box<dyn Decoder>> {
        // Returns a distinct short mock decoder (10 frames) to verify that real
        // decoders (which produce thousands of frames) are what actually decode.
        Ok(Box::new(MockDecoder::silence(
            AudioSpec::new(44_100, 2),
            10,
        )))
    }
}

#[test]
fn a_tie_between_two_claimants_is_broken_by_registration_order_not_refused() {
    let hint = ProbeHint {
        extension: None,
        header: b"whatever",
    };
    let mut reg = DecoderRegistry::new();
    reg.register(Arc::new(ClaimingFactory {
        name: "first",
        confidence: Confidence::CONTENT,
    }));
    reg.register(Arc::new(ClaimingFactory {
        name: "second",
        confidence: Confidence::CONTENT,
    }));
    // Two claimants at equal confidence: `DecoderRegistry::probe` deterministically
    // selects the earlier registration without refusing (RFC 0013 Guarantee 2).
    let (factory, confidence) = reg.probe(&hint).expect("both claimants answered");
    assert_eq!(factory.name(), "first");
    assert_eq!(confidence, Confidence::CONTENT);

    // `open` does not refuse either: the first claimant's decoder comes back.
    let opened = reg.open(reader_source(b"whatever".to_vec(), None));
    assert!(
        opened.is_ok(),
        "a tie must still open a decoder, not refuse: {:?}",
        opened.err()
    );

    // When the scores differ, the higher score wins regardless of order.
    let mut strong = DecoderRegistry::new();
    strong.register(Arc::new(ClaimingFactory {
        name: "low",
        confidence: Confidence::EXTENSION,
    }));
    strong.register(Arc::new(ClaimingFactory {
        name: "high",
        confidence: Confidence::CONTENT,
    }));
    assert_eq!(strong.probe(&hint).expect("a claim").0.name(), "high");
}

#[test]
fn content_beats_extension_even_when_extension_factory_registered_first() {
    // Register an extension-only claimant FIRST: claims "flac" and "mp3" at EXTENSION (40).
    let mut reg = DecoderRegistry::new();
    reg.register(Arc::new(ExtensionClaimingFactory {
        name: "fake_extension_claimant",
        extensions: &["flac", "mp3"],
        confidence: Confidence::EXTENSION,
    }));
    // Register Symphonia SECOND: verifies content signatures at CONTENT (90).
    reg.register(Arc::new(SymphoniaDecoderFactory::new()));

    // Case 1: Real FLAC fixture wearing its honest extension ".flac".
    // Even though the fake factory was registered first, Symphonia's CONTENT score (90)
    // strictly defeats EXTENSION score (40) per RFC 0013 Guarantee 1.
    let flac_bytes = std::fs::read(fixture("sine.flac")).expect("fixture");
    let hint_honest = ProbeHint {
        extension: Some("flac"),
        header: &flac_bytes[..64],
    };
    let (winner, conf) = reg
        .probe(&hint_honest)
        .expect("a factory must claim the flac");
    assert_eq!(winner.name(), "symphonia");
    assert_eq!(conf, Confidence::CONTENT);

    let mut decoder = reg
        .open(reader_source(flac_bytes.clone(), Some("flac")))
        .expect("open must succeed with content winner");
    let (frames, outcome) = drain_bounded(decoder.as_mut());
    assert!(matches!(outcome, Drained::End));
    assert_eq!(
        frames, FIXTURE_FRAMES,
        "must decode the real fixture to the end (not the 10 mock frames)"
    );

    // Case 2: Real FLAC fixture wearing a lying extension ".mp3".
    // The fake factory claims "mp3" via extension, but Symphonia inspects the header,
    // recognizes the FLAC magic bytes ("fLaC"), and scores Confidence::CONTENT.
    let hint_lying = ProbeHint {
        extension: Some("mp3"),
        header: &flac_bytes[..64],
    };
    let (winner_lying, conf_lying) = reg.probe(&hint_lying).expect("probe lying flac");
    assert_eq!(winner_lying.name(), "symphonia");
    assert_eq!(conf_lying, Confidence::CONTENT);

    let mut decoder_lying = reg
        .open(reader_source(flac_bytes.clone(), Some("mp3")))
        .expect("open lying flac");
    let (frames_lying, outcome_lying) = drain_bounded(decoder_lying.as_mut());
    assert!(matches!(outcome_lying, Drained::End));
    assert_eq!(frames_lying, FIXTURE_FRAMES);

    // Case 3: Registration order reversed (Symphonia first, fake second).
    // Symphonia must still win with CONTENT confidence.
    let mut reg_rev = DecoderRegistry::new();
    reg_rev.register(Arc::new(SymphoniaDecoderFactory::new()));
    reg_rev.register(Arc::new(ExtensionClaimingFactory {
        name: "fake_extension_claimant",
        extensions: &["flac", "mp3"],
        confidence: Confidence::EXTENSION,
    }));
    let (winner_rev, conf_rev) = reg_rev.probe(&hint_lying).expect("probe reverse");
    assert_eq!(winner_rev.name(), "symphonia");
    assert_eq!(conf_rev, Confidence::CONTENT);
}

#[test]
fn tie_break_is_deterministic_by_registration_order_for_both_content_and_extension() {
    let hint = ProbeHint {
        extension: Some("test"),
        header: b"mock_header_bytes",
    };

    // --- Part A: Tie at Confidence::CONTENT (90 vs 90) ---
    // Order A -> B: A must win.
    let mut reg_ab_content = DecoderRegistry::new();
    reg_ab_content.register(Arc::new(ClaimingFactory {
        name: "content_factory_a",
        confidence: Confidence::CONTENT,
    }));
    reg_ab_content.register(Arc::new(ClaimingFactory {
        name: "content_factory_b",
        confidence: Confidence::CONTENT,
    }));
    let (winner_ab_c, conf_ab_c) = reg_ab_content.probe(&hint).expect("probe");
    assert_eq!(winner_ab_c.name(), "content_factory_a");
    assert_eq!(conf_ab_c, Confidence::CONTENT);
    let dec_ab_c = reg_ab_content.open(reader_source(b"test".to_vec(), Some("test")));
    assert!(dec_ab_c.is_ok(), "content tie must open without refusal");

    // Order B -> A: B must win.
    let mut reg_ba_content = DecoderRegistry::new();
    reg_ba_content.register(Arc::new(ClaimingFactory {
        name: "content_factory_b",
        confidence: Confidence::CONTENT,
    }));
    reg_ba_content.register(Arc::new(ClaimingFactory {
        name: "content_factory_a",
        confidence: Confidence::CONTENT,
    }));
    let (winner_ba_c, conf_ba_c) = reg_ba_content.probe(&hint).expect("probe");
    assert_eq!(winner_ba_c.name(), "content_factory_b");
    assert_eq!(conf_ba_c, Confidence::CONTENT);
    let dec_ba_c = reg_ba_content.open(reader_source(b"test".to_vec(), Some("test")));
    assert!(dec_ba_c.is_ok(), "content tie must open without refusal");

    // --- Part B: Tie at Confidence::EXTENSION (40 vs 40) ---
    // Order A -> B: A must win.
    let mut reg_ab_ext = DecoderRegistry::new();
    reg_ab_ext.register(Arc::new(ClaimingFactory {
        name: "ext_factory_a",
        confidence: Confidence::EXTENSION,
    }));
    reg_ab_ext.register(Arc::new(ClaimingFactory {
        name: "ext_factory_b",
        confidence: Confidence::EXTENSION,
    }));
    let (winner_ab_e, conf_ab_e) = reg_ab_ext.probe(&hint).expect("probe");
    assert_eq!(winner_ab_e.name(), "ext_factory_a");
    assert_eq!(conf_ab_e, Confidence::EXTENSION);
    let dec_ab_e = reg_ab_ext.open(reader_source(b"test".to_vec(), Some("test")));
    assert!(dec_ab_e.is_ok(), "extension tie must open without refusal");

    // Order B -> A: B must win.
    let mut reg_ba_ext = DecoderRegistry::new();
    reg_ba_ext.register(Arc::new(ClaimingFactory {
        name: "ext_factory_b",
        confidence: Confidence::EXTENSION,
    }));
    reg_ba_ext.register(Arc::new(ClaimingFactory {
        name: "ext_factory_a",
        confidence: Confidence::EXTENSION,
    }));
    let (winner_ba_e, conf_ba_e) = reg_ba_ext.probe(&hint).expect("probe");
    assert_eq!(winner_ba_e.name(), "ext_factory_b");
    assert_eq!(conf_ba_e, Confidence::EXTENSION);
    let dec_ba_e = reg_ba_ext.open(reader_source(b"test".to_vec(), Some("test")));
    assert!(dec_ba_e.is_ok(), "extension tie must open without refusal");
}

#[test]
fn nothing_claims_is_refused_with_clear_unsupported_format_error() {
    let mut reg = DecoderRegistry::new();
    // Register factory that only claims "known_ext"
    reg.register(Arc::new(ExtensionClaimingFactory {
        name: "known_only",
        extensions: &["known_ext"],
        confidence: Confidence::EXTENSION,
    }));

    // Subcase 1: Unknown extension provided -> refused naming the extension
    let hint_unknown = ProbeHint {
        extension: Some("unsupported_xyz"),
        header: &pattern_bytes(64),
    };
    assert!(reg.probe(&hint_unknown).is_none());
    let err_unknown = reg
        .open(reader_source(pattern_bytes(64), Some("unsupported_xyz")))
        .err()
        .expect("must refuse unclaimed media");
    match err_unknown {
        Error::UnsupportedFormat(ext) => assert_eq!(ext, "unsupported_xyz"),
        other => panic!("expected Error::UnsupportedFormat, got {other:?}"),
    }

    // Subcase 2: No extension (None) and no content signature -> refused with "unknown"
    let hint_none = ProbeHint {
        extension: None,
        header: &pattern_bytes(64),
    };
    assert!(reg.probe(&hint_none).is_none());
    let err_none = reg
        .open(reader_source(pattern_bytes(64), None))
        .err()
        .expect("must refuse media without extension or signature");
    match err_none {
        Error::UnsupportedFormat(ext) => assert_eq!(ext, "unknown"),
        other => panic!("expected Error::UnsupportedFormat(\"unknown\"), got {other:?}"),
    }
}

#[test]
fn real_symphonia_path_opens_every_repo_fixture_with_competing_extension_factory() {
    // Registry with a competing fake factory registered FIRST for every common extension.
    let mut reg = DecoderRegistry::new();
    reg.register(Arc::new(ExtensionClaimingFactory {
        name: "competing_stub_factory",
        extensions: &["flac", "mp3", "ogg", "m4a", "aac", "alac", "wav"],
        confidence: Confidence::EXTENSION,
    }));
    // Symphonia registered SECOND: must win on content for all fixtures.
    reg.register(Arc::new(SymphoniaDecoderFactory::new()));

    let fixtures = [
        "sine.flac",
        "sine.mp3",
        "sine.ogg",
        "sine_aac.m4a",
        "sine_alac.m4a",
    ];

    for fixture_name in fixtures {
        let path = fixture(fixture_name);
        let mut decoder = reg
            .open(MediaSource::Path(path.clone()))
            .unwrap_or_else(|e| panic!("failed to open fixture {fixture_name}: {e:?}"));

        let (frames, outcome) = drain_bounded(decoder.as_mut());
        assert!(
            matches!(outcome, Drained::End),
            "fixture {fixture_name} must reach Drained::End"
        );
        // Ensure that the real Symphonia decoder was chosen, not the 10-frame mock decoder:
        assert!(
            frames >= 1000,
            "fixture {fixture_name} decoded only {frames} frames; expected full fixture"
        );
    }
}

// ---------------------------------------------------------------------------
// Resource release
// ---------------------------------------------------------------------------

#[test]
fn an_input_refused_at_open_releases_its_reader() {
    let (source, drops) = counting_source(pattern_bytes(4096), Some("mp3"));
    let opened = SymphoniaDecoder::open(source);
    assert!(opened.is_err(), "signature-less bytes must not open");
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "the failed open must drop the reader exactly once"
    );
}

#[test]
fn a_decoder_dropped_mid_stream_releases_its_reader_once() {
    let full = std::fs::read(fixture("sine.flac")).expect("fixture");
    let (source, drops) = counting_source(full[..full.len() / 3].to_vec(), Some("flac"));
    let mut d = SymphoniaDecoder::open(source).expect("a truncated flac still opens");
    let _ = drain_bounded(&mut d);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "the reader must live as long as the decoder"
    );
    drop(d);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "dropping the decoder must release the reader exactly once"
    );
}

#[test]
fn repeated_open_and_drop_releases_exactly_one_reader_per_cycle() {
    let bytes = std::fs::read(fixture("sine.flac")).expect("fixture");
    let cycles = 200u64;
    let mut released = 0u64;
    for _ in 0..cycles {
        let (source, drops) = counting_source(bytes.clone(), Some("flac"));
        let mut d = SymphoniaDecoder::open(source).expect("open");
        let _ = drain_bounded(&mut d);
        drop(d);
        released += drops.load(Ordering::SeqCst) as u64;
    }
    assert_eq!(released, cycles, "one reader must be released per cycle");
}

// ---------------------------------------------------------------------------
// Seeking
// ---------------------------------------------------------------------------

#[test]
fn seeking_past_the_end_is_clamped_and_never_panics() {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut d = open_bytes(wav_bytes(spec, 4000, sine), Some("wav"));
    let duration = d.duration().expect("a wav knows its length");
    // A hostile `Duration` is clamped to the duration before it reaches the
    // frame conversion, so this cannot overflow (see `frames_at`).
    let reached = d.seek(Duration::MAX).expect("a clamped seek succeeds");
    assert_eq!(reached, duration);
    let mut buf = [0.0f32; 8];
    assert_eq!(d.read(&mut buf).expect("read at the end"), 0);
}

#[test]
fn a_huge_seek_on_a_stream_without_a_duration_is_a_defined_outcome() {
    // A reader whose `SeekFrom::End` fails leaves the decoder unable to learn
    // the byte length, so an MP3 (whose length would be estimated from it)
    // reports no duration. There is then no clamp and the hostile `Duration`
    // reaches the frame conversion directly: the path fixed by
    // `frames_at` + `MAX_SEEK_FRAMES`.
    let bytes = std::fs::read(fixture("sine.mp3")).expect("fixture");
    let source = MediaSource::Reader {
        reader: Box::new(NoEndSeek {
            inner: Cursor::new(bytes),
        }),
        extension: Some("mp3".into()),
    };
    match SymphoniaDecoder::open(source) {
        Err(e) => assert_defined_open_failure(&e, "mp3 without a reportable end"),
        Ok(mut d) => match d.duration() {
            // No duration: the unclamped path. Whatever `seek` answers, it must
            // be a position or a defined error — never an overflow panic.
            None => {
                if let Err(e) = d.seek(Duration::MAX) {
                    assert!(
                        matches!(e, Error::Unsupported(_) | Error::Decode(_) | Error::Io(_)),
                        "a huge seek must give a defined error: {e:?}"
                    );
                }
            }
            // Duration known after all: the seek is clamped, not overflowed.
            Some(duration) => {
                let reached = d.seek(Duration::MAX).expect("clamped seek");
                assert_eq!(reached, duration);
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Handle release (Windows only)
// ---------------------------------------------------------------------------

/// The reader tests above prove the *reader* is dropped. On Windows the
/// underlying file handle is the resource that really leaks if a decoder fails
/// half way, so this counts process-wide handles across many failing and
/// succeeding cycles. It is best effort: other threads of the test binary can
/// open or close handles concurrently, so it only fails on growth beyond a
/// generous tolerance. `kernel32` is linked at test-link time only.
#[cfg(windows)]
mod windows_handles {
    use super::*;
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetProcessHandleCount(process: *mut c_void, count: *mut u32) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
    }

    fn handle_count() -> Option<u32> {
        let mut count = 0u32;
        // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no
        // closing, and `count` is a valid out-pointer for the duration of the
        // call.
        let ok = unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) };
        (ok != 0).then_some(count)
    }

    #[test]
    fn failed_path_opens_do_not_accumulate_file_handles() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("garbage.mp3");
        std::fs::write(&path, pattern_bytes(4096)).expect("write");
        // One warm-up open, so lazily created handles are not counted as growth.
        let _ = SymphoniaDecoder::open(MediaSource::Path(path.clone()));
        let Some(before) = handle_count() else {
            return;
        };
        for _ in 0..200 {
            let _ = SymphoniaDecoder::open(MediaSource::Path(path.clone()));
        }
        let after = handle_count().expect("handle count");
        assert!(
            after <= before + 32,
            "200 failing opens grew the handle count from {before} to {after}"
        );
    }

    #[test]
    fn decoded_path_files_do_not_accumulate_file_handles() {
        let Some(before) = handle_count() else {
            return;
        };
        for _ in 0..100 {
            let mut d =
                SymphoniaDecoder::open(MediaSource::Path(fixture("sine.flac"))).expect("open");
            let _ = drain_bounded(&mut d);
            drop(d);
        }
        let after = handle_count().expect("handle count");
        assert!(
            after <= before + 16,
            "100 open/drain/drop cycles grew the handle count from {before} to {after}"
        );
    }
}
