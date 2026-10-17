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
