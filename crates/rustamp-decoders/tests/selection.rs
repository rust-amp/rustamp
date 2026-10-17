//! RFC 0013 selection with two **real** decoder factories.
//!
//! `SymphoniaDecoderFactory` and `WaveDecoderFactory` genuinely overlap: both
//! claim `.wav`/`.wave` and both recognise the `RIFF`/`WAVE` signature at
//! `Confidence::CONTENT`. That makes every guarantee of RFC 0013 reachable in
//! `default_registry` with real implementations instead of the fake claimants
//! used in `tests/robustness.rs`:
//!
//! * a real content match beats a real extension claim,
//! * a real equal-confidence tie is broken by registration order,
//! * a container neither factory claims is refused.
//!
//! The inputs are the repository fixtures (see `tests/fixtures/FIXTURES.txt`) or
//! WAV bytes written by `hound` into a temp file; nothing is downloaded. No test
//! opens a window or plays audio.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use rustamp_core::audio::{
    AudioSpec, Confidence, Decoder, DecoderFactory, DecoderRegistry, MediaSource, ProbeHint,
};
use rustamp_core::error::Error;
use rustamp_decoders::{SymphoniaDecoderFactory, WaveDecoderFactory, default_registry};

const RATE: u32 = 44_100;
const FIXTURE_FRAMES: u64 = RATE as u64 * 3 / 2;

/// Upper bound on `read` calls per drain, so a hang fails instead of stalling.
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

/// A byte pattern that carries no format signature: `i * 37 % 251` is never
/// `0xFF`, and the first bytes cannot be `RIFF`/`WAVE` (see `robustness.rs`).
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

/// A real 16-bit stereo WAV file, 2000 frames.
fn real_wav() -> Vec<u8> {
    wav_bytes(
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
        2000,
        sine,
    )
}

/// Drains a decoder and returns the frame count, failing on an error or a hang.
fn drain_to_end(d: &mut dyn Decoder) -> u64 {
    let ch = usize::from(d.spec().channels.max(1));
    let mut buf = vec![0.0f32; 4096 * ch];
    let mut frames = 0u64;
    for _ in 0..MAX_READS {
        match d.read(&mut buf) {
            Ok(0) => return frames,
            Ok(n) => frames += n as u64,
            Err(e) => panic!("a clean fixture must not fail: {e:?}"),
        }
    }
    panic!("no end of stream after {MAX_READS} reads: possible hang");
}

// ---------------------------------------------------------------------------
// Two real factories, one registry
// ---------------------------------------------------------------------------

#[test]
fn the_default_registry_has_two_real_factories() {
    let reg = default_registry();
    let names: Vec<&str> = reg.factories().iter().map(|f| f.name()).collect();
    assert_eq!(
        names,
        vec!["symphonia", "wave"],
        "registration order is the RFC 0013 tie-break and part of the contract"
    );
    // Both claim `.wav`, so the registry's extension list is unchanged by the
    // second factory - only the number of real claimants grew.
    assert!(reg.extensions().contains(&"wav".to_string()));
}

#[test]
fn a_real_wav_file_is_a_genuine_tie_resolved_by_registration_order() {
    let wav = real_wav();
    let symphonia = SymphoniaDecoderFactory::new();
    let wave = WaveDecoderFactory::new();

    // Both real factories score the same, from the signature and from the
    // extension: this is a tie between two real decoders, not a test double.
    let content = ProbeHint {
        extension: Some("wav"),
        header: &wav,
    };
    assert_eq!(symphonia.probe(&content), Confidence::CONTENT);
    assert_eq!(wave.probe(&content), Confidence::CONTENT);
    // The extension-only case (no signature in the header) is a real tie too.
    let pattern = pattern_bytes(64);
    let extension = ProbeHint {
        extension: Some("wav"),
        header: &pattern,
    };
    assert_eq!(symphonia.probe(&extension), Confidence::EXTENSION);
    assert_eq!(wave.probe(&extension), Confidence::EXTENSION);

    for hint in [&content, &extension] {
        let mut wave_first = DecoderRegistry::new();
        wave_first.register(Arc::new(WaveDecoderFactory::new()));
        wave_first.register(Arc::new(SymphoniaDecoderFactory::new()));
        assert_eq!(wave_first.probe(hint).expect("a claim").0.name(), "wave");

        let mut symphonia_first = DecoderRegistry::new();
        symphonia_first.register(Arc::new(SymphoniaDecoderFactory::new()));
        symphonia_first.register(Arc::new(WaveDecoderFactory::new()));
        assert_eq!(
            symphonia_first.probe(hint).expect("a claim").0.name(),
            "symphonia"
        );

        // The default registry keeps symphonia first, so a real WAV is
        // unchanged by the second factory even though the tie is real.
        assert_eq!(
            default_registry().probe(hint).expect("a claim").0.name(),
            "symphonia"
        );
    }

    // Neither order refuses, and whichever factory wins decodes the same real
    // file to the same number of frames.
    for wave_first in [true, false] {
        let mut reg = DecoderRegistry::new();
        if wave_first {
            reg.register(Arc::new(WaveDecoderFactory::new()));
            reg.register(Arc::new(SymphoniaDecoderFactory::new()));
        } else {
            reg.register(Arc::new(SymphoniaDecoderFactory::new()));
            reg.register(Arc::new(WaveDecoderFactory::new()));
        }
        let mut d = reg
            .open(reader_source(wav.clone(), Some("wav")))
            .expect("a tie must open a decoder, not refuse");
        assert_eq!(d.spec(), AudioSpec::new(RATE, 2));
        assert_eq!(drain_to_end(d.as_mut()), 2000);
    }
}

#[test]
fn a_real_content_signature_beats_a_real_extension_claim_in_both_orders() {
    let flac = std::fs::read(fixture("sine.flac")).expect("fixture");
    let wave = WaveDecoderFactory::new();
    let symphonia = SymphoniaDecoderFactory::new();

    // FLAC bytes wearing `.wav`: the wave factory claims the extension, the
    // symphonia factory recognises the `fLaC` signature.
    let hint = ProbeHint {
        extension: Some("wav"),
        header: &flac,
    };
    assert_eq!(wave.probe(&hint), Confidence::EXTENSION);
    assert_eq!(symphonia.probe(&hint), Confidence::CONTENT);

    // Even with the extension claimant registered first (RFC 0013, G1).
    let mut reg = DecoderRegistry::new();
    reg.register(Arc::new(WaveDecoderFactory::new()));
    reg.register(Arc::new(SymphoniaDecoderFactory::new()));
    let (winner, confidence) = reg.probe(&hint).expect("the content claimant answers");
    assert_eq!(winner.name(), "symphonia");
    assert_eq!(confidence, Confidence::CONTENT);

    // And the content winner is the one that really decodes the file.
    let mut d = reg
        .open(reader_source(flac.clone(), Some("wav")))
        .expect("open");
    assert_eq!(d.spec(), AudioSpec::new(RATE, 2));
    assert_eq!(drain_to_end(d.as_mut()), FIXTURE_FRAMES);

    // The mirror image: MP3 bytes wearing `.wav` are recognised by content
    // (an ID3 tag or an MPEG sync) and still beat the extension claim.
    let mp3 = std::fs::read(fixture("sine.mp3")).expect("fixture");
    let hint = ProbeHint {
        extension: Some("wav"),
        header: &mp3,
    };
    assert_eq!(wave.probe(&hint), Confidence::EXTENSION);
    assert_eq!(symphonia.probe(&hint), Confidence::CONTENT);
    assert_eq!(reg.probe(&hint).expect("a claim").0.name(), "symphonia");
    let mut d = reg.open(reader_source(mp3, Some("wav"))).expect("open");
    assert!(drain_to_end(d.as_mut()) >= 1000);
}

#[test]
fn two_real_factories_refuse_media_that_neither_claims() {
    let reg = default_registry();

    // A real RIFF container that is not WAVE (AVI): both real factories look at
    // the signature, neither claims it, and the registry names the extension.
    let mut avi = b"RIFF".to_vec();
    avi.extend_from_slice(&64u32.to_le_bytes());
    avi.extend_from_slice(b"AVI LIST");
    avi.extend_from_slice(&pattern_bytes(64));
    let hint = ProbeHint {
        extension: Some("avi"),
        header: &avi,
    };
    assert_eq!(
        SymphoniaDecoderFactory::new().probe(&hint),
        Confidence::NONE
    );
    assert_eq!(WaveDecoderFactory::new().probe(&hint), Confidence::NONE);
    assert!(reg.probe(&hint).is_none());
    let err = reg
        .open(reader_source(avi, Some("avi")))
        .err()
        .expect("nothing may claim AVI");
    match err {
        Error::UnsupportedFormat(ext) => assert_eq!(ext, "avi"),
        other => panic!("expected Error::UnsupportedFormat, got {other:?}"),
    }

    // No signature and no extension: refused as "unknown".
    let hint = ProbeHint {
        extension: None,
        header: &pattern_bytes(4096),
    };
    assert!(reg.probe(&hint).is_none());
    let err = reg
        .open(reader_source(pattern_bytes(4096), None))
        .err()
        .expect("nothing may claim an unrecognized file");
    match err {
        Error::UnsupportedFormat(ext) => assert_eq!(ext, "unknown"),
        other => panic!("expected Error::UnsupportedFormat(\"unknown\"), got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Each factory decodes a real file on its own
// ---------------------------------------------------------------------------

#[test]
fn each_real_factory_decodes_a_real_file_on_its_own() {
    // Symphonia alone: a repository fixture.
    let mut reg = DecoderRegistry::new();
    reg.register(Arc::new(SymphoniaDecoderFactory::new()));
    let mut d = reg
        .open(MediaSource::Path(fixture("sine.flac")))
        .expect("symphonia opens the flac");
    assert_eq!(d.spec(), AudioSpec::new(RATE, 2));
    assert_eq!(drain_to_end(d.as_mut()), FIXTURE_FRAMES);

    // The wave factory alone: a real WAV file on disk, decoded bit-exactly.
    let frames = 2000usize;
    let bytes = real_wav();
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("sine.wav");
    std::fs::write(&path, &bytes).expect("write");
    let mut reg = DecoderRegistry::new();
    reg.register(Arc::new(WaveDecoderFactory::new()));
    let mut d = reg
        .open(MediaSource::Path(path))
        .expect("wave opens the wav");
    assert_eq!(d.spec(), AudioSpec::new(RATE, 2));
    assert_eq!(d.duration(), Some(d.spec().duration_of(frames as u64)));

    let ch = 2usize;
    let mut buf = vec![0.0f32; 512 * ch];
    let mut got: Vec<f32> = Vec::new();
    loop {
        let n = d.read(&mut buf).expect("read");
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n * ch]);
    }
    assert_eq!(got.len(), frames * ch);
    for i in 0..frames {
        for c in 0..2u16 {
            let q = (sine(i, c) * 32_767.0).round();
            assert_eq!(
                got[i * 2 + usize::from(c)],
                (q / 32_768.0) as f32,
                "frame {i} channel {c}"
            );
        }
    }
}
