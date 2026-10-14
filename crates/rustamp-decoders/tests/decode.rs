//! Integration tests for the symphonia decoder.
//!
//! WAV/AIFF inputs are generated in memory. The compressed fixtures in
//! `tests/fixtures/` are 1.5 s, 440 Hz, 44.1 kHz stereo sine waves made with
//! ffmpeg (see `tests/fixtures/FIXTURES.txt` for the exact commands).

use std::io::Cursor;
use std::path::PathBuf;
use std::time::Duration;

use rustamp_core::audio::{AudioSpec, Confidence, Decoder, DecoderFactory, MediaSource, ProbeHint};
use rustamp_core::error::Error;
use rustamp_decoders::{SymphoniaDecoder, SymphoniaDecoderFactory, default_registry};

const RATE: u32 = 44_100;
const FIXTURE_FRAMES: u64 = RATE as u64 * 3 / 2;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn open_path(name: &str) -> Box<dyn Decoder> {
    default_registry()
        .open(MediaSource::Path(fixture(name)))
        .unwrap_or_else(|e| panic!("open {name}: {e}"))
}

fn open_bytes(bytes: Vec<u8>, ext: Option<&str>) -> Box<dyn Decoder> {
    default_registry()
        .open(MediaSource::Reader {
            reader: Box::new(Cursor::new(bytes)),
            extension: ext.map(str::to_owned),
        })
        .expect("open")
}

fn read_all(d: &mut dyn Decoder) -> Vec<f32> {
    let ch = usize::from(d.spec().channels);
    let mut out = Vec::new();
    let mut buf = vec![0.0f32; 1000 * ch];
    loop {
        let n = d.read(&mut buf).expect("read");
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n * ch]);
    }
    out
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

fn int_wav_roundtrip(bits: u16) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: RATE,
        bits_per_sample: bits,
        sample_format: hound::SampleFormat::Int,
    };
    let n = 5000;
    let bytes = wav_bytes(spec, n, sine);
    let mut d = open_bytes(bytes, Some("wav"));
    assert_eq!(d.spec(), AudioSpec::new(RATE, 2));
    assert_eq!(d.duration(), Some(d.spec().duration_of(n as u64)));
    let out = read_all(d.as_mut());
    assert_eq!(out.len(), n * 2);
    let max = ((1i64 << (bits - 1)) - 1) as f64;
    let scale = (1i64 << (bits - 1)) as f64;
    for i in 0..n {
        for c in 0..2u16 {
            let q = (sine(i, c) * max).round();
            let expect = (q / scale) as f32;
            assert_eq!(out[i * 2 + usize::from(c)], expect, "frame {i} ch {c}");
        }
    }
}

#[test]
fn wav_16bit_bit_exact() {
    int_wav_roundtrip(16);
}

#[test]
fn wav_24bit_bit_exact() {
    int_wav_roundtrip(24);
}

#[test]
fn wav_32bit_int_bit_exact() {
    int_wav_roundtrip(32);
}

#[test]
fn wav_float_bit_exact() {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: RATE,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let n = 5000;
    let mut d = open_bytes(wav_bytes(spec, n, sine), Some("wav"));
    let out = read_all(d.as_mut());
    assert_eq!(out.len(), n * 2);
    for i in 0..n {
        for c in 0..2u16 {
            assert_eq!(out[i * 2 + usize::from(c)], sine(i, c) as f32);
        }
    }
}

#[test]
fn wav_8bit_and_mono() {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8000,
        bits_per_sample: 8,
        sample_format: hound::SampleFormat::Int,
    };
    let mut d = open_bytes(wav_bytes(spec, 800, sine), Some("wav"));
    assert_eq!(d.spec(), AudioSpec::new(8000, 1));
    let out = read_all(d.as_mut());
    assert_eq!(out.len(), 800);
    for (i, v) in out.iter().enumerate() {
        let q = (sine(i, 0) * 127.0).round();
        assert!((f64::from(*v) - q / 128.0).abs() < 1.0 / 128.0);
    }
}

#[test]
fn wav_seek_is_sample_accurate() {
    // Ramp so every frame has a unique value.
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 8000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let n = 20_000usize;
    let bytes = wav_bytes(spec, n, |i, c| (i as f64 + f64::from(c)) / 32767.0);
    let mut d = open_bytes(bytes, Some("wav"));
    let at = Duration::from_millis(1250); // frame 10_000
    let got = d.seek(at).unwrap();
    assert_eq!(got, at);
    let mut buf = [0.0f32; 8];
    assert_eq!(d.read(&mut buf).unwrap(), 4);
    assert_eq!(buf[0], 10_000.0 / 32768.0);
    assert_eq!(buf[1], 10_001.0 / 32768.0);
    assert_eq!(buf[6], 10_003.0 / 32768.0);
    // Backwards to the start, then past the end (clamped).
    d.seek(Duration::ZERO).unwrap();
    d.read(&mut buf).unwrap();
    assert_eq!(buf[0], 0.0);
    let end = d.seek(Duration::from_secs(60)).unwrap();
    assert_eq!(end, d.duration().unwrap());
    assert_eq!(d.read(&mut buf).unwrap(), 0);
}

#[test]
fn aiff_in_memory() {
    // 16-bit big-endian mono AIFF at 44.1 kHz, 1000 frames.
    let n = 1000u32;
    let mut comm = Vec::new();
    comm.extend_from_slice(&1u16.to_be_bytes());
    comm.extend_from_slice(&n.to_be_bytes());
    comm.extend_from_slice(&16u16.to_be_bytes());
    comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
    let mut ssnd = Vec::new();
    ssnd.extend_from_slice(&0u32.to_be_bytes());
    ssnd.extend_from_slice(&0u32.to_be_bytes());
    for i in 0..n {
        ssnd.extend_from_slice(&((i as i16) * 10).to_be_bytes());
    }
    let mut body = b"AIFF".to_vec();
    body.extend_from_slice(b"COMM");
    body.extend_from_slice(&(comm.len() as u32).to_be_bytes());
    body.extend_from_slice(&comm);
    body.extend_from_slice(b"SSND");
    body.extend_from_slice(&(ssnd.len() as u32).to_be_bytes());
    body.extend_from_slice(&ssnd);
    let mut file = b"FORM".to_vec();
    file.extend_from_slice(&(body.len() as u32).to_be_bytes());
    file.extend_from_slice(&body);

    let mut d = open_bytes(file, None); // content sniffing only
    assert_eq!(d.spec(), AudioSpec::new(44100, 1));
    let out = read_all(d.as_mut());
    assert_eq!(out.len(), 1000);
    assert_eq!(out[7], 70.0 / 32768.0);
}

fn check_fixture(name: &str, tol: u64) {
    let mut d = open_path(name);
    assert_eq!(d.spec(), AudioSpec::new(RATE, 2), "{name}");
    let dur = d.duration().expect("duration known");
    let dframes = (dur.as_secs_f64() * f64::from(RATE)).round() as u64;
    assert!(
        dframes.abs_diff(FIXTURE_FRAMES) <= tol,
        "{name}: duration frames {dframes} vs {FIXTURE_FRAMES}"
    );
    let out = read_all(d.as_mut());
    let frames = (out.len() / 2) as u64;
    assert!(
        frames.abs_diff(FIXTURE_FRAMES) <= tol,
        "{name}: decoded {frames} frames, expected about {FIXTURE_FRAMES}"
    );
    let mid = &out[out.len() / 2..out.len() / 2 + 2000];
    let peak = mid.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    assert!(peak > 0.05 && peak <= 1.05, "{name}: peak {peak}");
    let info = d.stream_info();
    assert_eq!(info.sample_rate_hz, RATE);
    assert_eq!(info.channels, 2);
    assert!(info.bitrate_kbps.is_some_and(|b| b > 0), "{name} bitrate");
    assert!(d.is_seekable());
}

#[test]
fn flac_decodes_exact_length() {
    check_fixture("sine.flac", 0);
}

#[test]
fn mp3_decodes_gapless_length() {
    check_fixture("sine.mp3", 1152);
}

#[test]
fn vorbis_decodes_gapless_length() {
    check_fixture("sine.ogg", 1);
}

#[test]
fn aac_m4a_decodes() {
    check_fixture("sine_aac.m4a", 2048);
}

#[test]
fn alac_m4a_decodes() {
    check_fixture("sine_alac.m4a", 0);
}

fn check_seek_matches_full_decode(name: &str, tol: f32) {
    let mut d = open_path(name);
    let full = read_all(d.as_mut());
    for ms in [0u64, 300, 1000, 1400] {
        let at = Duration::from_millis(ms);
        let got = d.seek(at).unwrap();
        let f0 = (at.as_secs_f64() * f64::from(RATE)).round() as usize;
        assert!(
            got.abs_diff(at) < Duration::from_millis(1),
            "{name} {ms}: {got:?}"
        );
        let mut buf = vec![0.0f32; 200];
        let n = d.read(&mut buf).unwrap();
        assert!(n > 0, "{name} {ms}: nothing after seek");
        for i in 0..n * 2 {
            let want = full[f0 * 2 + i];
            assert!(
                (buf[i] - want).abs() <= tol,
                "{name} seek {ms} ms sample {i}: got {} want {want}",
                buf[i]
            );
        }
    }
}

#[test]
fn flac_seek_accurate() {
    check_seek_matches_full_decode("sine.flac", 0.0);
}

#[test]
fn alac_seek_accurate() {
    check_seek_matches_full_decode("sine_alac.m4a", 0.0);
}

#[test]
fn mp3_seek_accurate() {
    check_seek_matches_full_decode("sine.mp3", 0.02);
}

#[test]
fn vorbis_seek_accurate() {
    check_seek_matches_full_decode("sine.ogg", 0.02);
}

#[test]
fn aac_seek_accurate() {
    check_seek_matches_full_decode("sine_aac.m4a", 0.05);
}

#[test]
fn metadata_roundtrip() {
    let m = open_path("sine.flac").metadata();
    assert_eq!(m.title.as_deref(), Some("Sine Title"));
    assert_eq!(m.artist.as_deref(), Some("Test Artist"));
    assert_eq!(m.album.as_deref(), Some("Test Album"));
    assert_eq!(m.year, Some(2001));
    assert_eq!(m.genre.as_deref(), Some("Test"));
    assert_eq!(m.comment.as_deref(), Some("hello"));
    assert_eq!(m.track_no, Some(3));
    assert_eq!(m.duration_ms, Some(1500));
    assert!(m.bitrate_kbps.is_some());

    let m = open_path("sine.mp3").metadata();
    assert_eq!(m.title.as_deref(), Some("Sine Title"));
    assert_eq!(m.artist.as_deref(), Some("Test Artist"));
    assert_eq!(m.album.as_deref(), Some("Test Album"));
    assert_eq!(m.year, Some(2001));
    assert_eq!(m.genre.as_deref(), Some("Test"));
    assert_eq!(m.track_no, Some(3));
    assert_eq!(m.track_total, Some(9));

    let m = open_path("sine.ogg").metadata();
    assert_eq!(m.title.as_deref(), Some("Sine Title"));
    assert_eq!(m.artist.as_deref(), Some("Test Artist"));
}

#[test]
fn probe_scores() {
    let f = SymphoniaDecoderFactory::new();
    let p = |ext: Option<&str>, header: &[u8]| {
        f.probe(&ProbeHint {
            extension: ext,
            header,
        })
    };
    assert_eq!(p(Some("mp3"), b""), Confidence::EXTENSION);
    assert_eq!(p(Some("FLAC"), b""), Confidence::EXTENSION);
    assert_eq!(p(Some("txt"), b""), Confidence::NONE);
    assert_eq!(p(None, b"hello world, not audio"), Confidence::NONE);
    assert_eq!(p(None, b"ID3\x04\0\0\0\0\0\0"), Confidence::CONTENT);
    assert_eq!(p(None, &[0xFF, 0xFB, 0x90, 0x00]), Confidence::CONTENT);
    assert_eq!(p(None, &[0xFF, 0xFF, 0xFF, 0xFF]), Confidence::NONE);
    assert_eq!(p(None, b"fLaC\0\0\0\x22"), Confidence::CONTENT);
    assert_eq!(p(None, b"OggS\0\x02"), Confidence::CONTENT);
    assert_eq!(p(None, b"RIFF\0\0\0\0WAVEfmt "), Confidence::CONTENT);
    assert_eq!(p(None, b"RIFF\0\0\0\0AVI LIST"), Confidence::NONE);
    assert_eq!(p(None, b"FORM\0\0\0\0AIFFCOMM"), Confidence::CONTENT);
    assert_eq!(p(None, b"\0\0\0\x20ftypM4A "), Confidence::CONTENT);
    // Content beats a wrong extension.
    assert_eq!(p(Some("txt"), b"fLaC\0\0\0\x22"), Confidence::CONTENT);
    // Real headers of the fixtures.
    for name in [
        "sine.flac",
        "sine.mp3",
        "sine.ogg",
        "sine_aac.m4a",
        "sine_alac.m4a",
    ] {
        let mut src = MediaSource::Path(fixture(name));
        let header = src.sniff(4096).unwrap();
        assert_eq!(p(None, &header), Confidence::CONTENT, "{name}");
    }
}

#[test]
fn registry_lists_extensions() {
    let exts = default_registry().extensions();
    for e in ["mp3", "flac", "ogg", "wav", "aiff", "m4a"] {
        assert!(exts.iter().any(|x| x == e), "{e}");
    }
}

#[test]
fn url_is_rejected_with_clear_error() {
    let err = SymphoniaDecoderFactory::new()
        .open(MediaSource::Url("http://example.com/a.mp3".into()))
        .err()
        .expect("must fail");
    match err {
        Error::Unsupported(m) => assert!(m.contains("M4"), "{m}"),
        e => panic!("unexpected {e:?}"),
    }
}

#[test]
fn garbage_is_unsupported_not_panic() {
    let r = default_registry().open(MediaSource::Reader {
        reader: Box::new(Cursor::new(vec![7u8; 4096])),
        extension: Some("txt".into()),
    });
    assert!(r.is_err());
    let r = SymphoniaDecoder::open(MediaSource::Reader {
        reader: Box::new(Cursor::new(vec![7u8; 4096])),
        extension: Some("mp3".into()),
    });
    assert!(r.is_err());
}

fn corrupt_decode(name: &str, ext: &str) -> (u64, SymphoniaDecoder) {
    let mut bytes = std::fs::read(fixture(name)).unwrap();
    let len = bytes.len();
    for b in &mut bytes[len / 2..len / 2 + 300] {
        *b ^= 0xA5;
    }
    let mut d = SymphoniaDecoder::open(MediaSource::Reader {
        reader: Box::new(Cursor::new(bytes)),
        extension: Some(ext.into()),
    })
    .expect("open");
    let out = read_all(&mut d);
    ((out.len() / 2) as u64, d)
}

#[test]
fn corrupt_packets_are_skipped_and_counted() {
    // ALAC: the decoder rejects the damaged packet; playback continues.
    let (frames, d) = corrupt_decode("sine_alac.m4a", "m4a");
    assert!(d.skipped_packets() > 0, "no packet reported as skipped");
    assert!(frames > FIXTURE_FRAMES / 2, "only {frames} frames");
    // AAC: damage may surface as an "unsupported" decoder error; still no abort.
    let (frames, _) = corrupt_decode("sine_aac.m4a", "m4a");
    assert!(frames > FIXTURE_FRAMES / 4, "only {frames} frames");
}

#[test]
fn corrupt_flac_and_mp3_keep_playing() {
    // The container readers resynchronise by themselves; no error may escape.
    let (frames, _) = corrupt_decode("sine.flac", "flac");
    assert!(frames > FIXTURE_FRAMES / 2, "only {frames} frames");
    let (frames, _) = corrupt_decode("sine.mp3", "mp3");
    assert!(frames > FIXTURE_FRAMES / 2, "only {frames} frames");
}

#[test]
fn partial_reads_return_whole_frames() {
    let mut d = open_path("sine.flac");
    let mut buf = vec![0.0f32; 2 * 7];
    let mut total = 0;
    loop {
        let n = d.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        total += n as u64;
    }
    assert_eq!(total, FIXTURE_FRAMES);
}
