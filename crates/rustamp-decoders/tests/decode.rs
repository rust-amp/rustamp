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

/// The rate an Opus stream is decoded at.
///
/// Opus always runs at 48 kHz: the rate of the codec is fixed and the
/// container states it.
const OPUS_RATE: u32 = 48_000;

/// The number of frames the Opus fixture holds: 1.5 s at 48 kHz.
const OPUS_FRAMES: u64 = OPUS_RATE as u64 * 3 / 2;

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

/// An Opus file holds exactly the samples of its audio.
///
/// The count is the point: the samples at the start of an Opus stream are the
/// encoder's look-ahead and the samples at its end are padding, and neither is
/// audio. A decoder that delivers them would play 312 samples too much and
/// would place the start of the file 6.5 ms late.
#[test]
fn opus_decodes_exact_length() {
    let mut d = open_path("sine.opus");
    assert_eq!(d.spec(), AudioSpec::new(OPUS_RATE, 2));
    assert_eq!(d.duration(), Some(Duration::from_millis(1500)));
    let out = read_all(d.as_mut());
    let frames = (out.len() / 2) as u64;
    assert_eq!(
        frames, OPUS_FRAMES,
        "the file holds every sample of its audio, and nothing else"
    );
    let mid = &out[out.len() / 2..out.len() / 2 + 2000];
    let peak = mid.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    assert!(peak > 0.05 && peak <= 1.05, "opus: peak {peak}");
    let info = d.stream_info();
    assert_eq!(info.sample_rate_hz, OPUS_RATE);
    assert_eq!(info.channels, 2);
    assert!(info.bitrate_kbps.is_some_and(|b| b > 0), "opus bitrate");
    assert!(d.is_seekable());
}

/// The decoded audio is the sine the file holds, not noise.
///
/// The fixture is a 440 Hz sine wave, so the decoded signal crosses zero
/// twice per period and the number of crossings names the frequency. A
/// decoder that produced noise or silence would not cross zero at that rate.
#[test]
fn the_opus_audio_is_the_sine_the_file_holds() {
    let mut d = open_path("sine.opus");
    let out = read_all(d.as_mut());
    // The left channel of the second half of the file.
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let mid = &left[OPUS_RATE as usize / 2..OPUS_RATE as usize];
    let crossings = mid
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    let hz = crossings as f64 * f64::from(OPUS_RATE) / 2.0 / mid.len() as f64;
    assert!(
        (hz - 440.0).abs() < 2.0,
        "the decoded signal is not a 440 Hz sine: {hz} Hz"
    );
}

/// An Opus stream in a Matroska file decodes like the same stream in an Ogg
/// file.
///
/// The Matroska mapping states the channel count of an Opus track as a
/// channel layout and not as a count, so the decoder used to refuse the
/// stream by name ("the stream states no channel count"). The
/// identification header the track carries states the count, and the two
/// files hold the same encoded stream, so the decoded samples have to be the
/// same.
#[test]
fn opus_in_matroska_decodes_the_same_audio_as_the_ogg_file() {
    let mut mkv = open_path("sine_opus.mkv");
    assert_eq!(mkv.spec(), AudioSpec::new(OPUS_RATE, 2));
    // The container states 1508 ms for the encoded stream. The encoder's
    // look-ahead is not audio, so the duration is that much shorter - and
    // the container states its duration with a resolution of a millisecond.
    let dur = mkv.duration().expect("duration known");
    assert!(
        dur.abs_diff(Duration::from_millis(1500)) < Duration::from_millis(2),
        "matroska duration {dur:?}"
    );
    let out = read_all(mkv.as_mut());
    let ogg = read_all(open_path("sine.opus").as_mut());
    // The Ogg file holds exactly the samples of its audio.
    assert_eq!(ogg.len() / 2, OPUS_FRAMES as usize);
    // The Matroska file holds the same samples and the padding at its end.
    // The Matroska reader does not state that padding (it ignores the
    // `DiscardPadding` of the last block), so the decoder cannot drop it -
    // see `CONTRACT_NOTES.md`.
    assert!(out.len() > ogg.len(), "the padding at the end is missing");
    assert_eq!(
        &out[..ogg.len()],
        &ogg[..],
        "the two files hold the same audio"
    );
    let tail = &out[ogg.len()..];
    let peak = tail.iter().fold(0.0f32, |a, b| a.max(b.abs()));
    assert!(peak < 0.01, "the tail is the padding: peak {peak}");
}

/// A seek in a Matroska file lands on the position the container states.
///
/// The Matroska reader counts its timestamps in the units of the timestamp
/// scale of the file - a millisecond by default - so a seek cannot be
/// sample exact there: the file states the position of a block with a
/// resolution of a millisecond.
#[test]
fn opus_in_matroska_seeks_to_the_position_the_container_states() {
    let mut d = open_path("sine_opus.mkv");
    let ch = usize::from(d.spec().channels);
    let full = read_all(d.as_mut());
    for ms in [300u64, 1000, 1400] {
        let at = Duration::from_millis(ms);
        d.seek(at).unwrap();
        let mut buf = vec![0.0f32; 400 * ch];
        let n = d.read(&mut buf).unwrap();
        assert!(n > 0, "matroska {ms} ms: nothing after the seek");
        let expect = (at.as_secs_f64() * f64::from(OPUS_RATE)).round() as usize;
        // The window is wider than the error the test is about, so that a
        // seek that lands further away does not find its best match at the
        // edge of the window.
        let landed = best_match(&full, &buf[..n * ch], ch, expect, 512);
        // One millisecond is 48 frames at 48 kHz; the container states the
        // position of a block with that resolution.
        let off = landed.abs_diff(expect);
        assert!(
            off * 1000 <= 2 * OPUS_RATE as usize,
            "matroska {ms} ms: the seek landed {off} frames off"
        );
    }
}

/// The frame of the full decode that matches the audio after a seek best.
///
/// A seek that lands on the wrong sample finds its best match a few samples
/// away from the target, so the offset of the best match is the sample
/// accuracy of the seek. The codec rebuilds its state from the lead-in of the
/// seek, so the two signals are not identical - but they are the same signal.
///
/// `window` is the number of frames the search looks at on either side of
/// the expected frame. A seek that lands further away than the window finds
/// its best match at the edge of it, so the window has to be wider than the
/// error the test is about.
fn best_match(
    full: &[f32],
    seeked: &[f32],
    channels: usize,
    expect: usize,
    window: usize,
) -> usize {
    let start = expect.saturating_sub(window);
    let end = (expect + window).min(full.len() / channels);
    (start..end)
        .map(|frame| {
            let mut sum = 0.0f64;
            for i in 0..seeked.len() {
                let d = f64::from(full[frame * channels + i]) - f64::from(seeked[i]);
                sum += d * d;
            }
            (sum, frame)
        })
        .min_by(|a, b| a.0.partial_cmp(&b.0).expect("no NaN in the signal"))
        .expect("the window holds frames")
        .1
}

/// A seek lands on the exact sample it asked for.
///
/// The Opus codec rebuilds its state from the lead-in of the seek, so the
/// first samples after the seek differ a little from the ones of a decode
/// that ran from the beginning - but the seek lands on the right sample.
#[test]
fn opus_seek_is_sample_accurate() {
    let mut d = open_path("sine.opus");
    let ch = usize::from(d.spec().channels);
    let full = read_all(d.as_mut());
    for ms in [0u64, 300, 1000, 1400] {
        let at = Duration::from_millis(ms);
        let got = d.seek(at).unwrap();
        assert!(
            got.abs_diff(at) < Duration::from_millis(1),
            "opus {ms} ms: {got:?}"
        );
        let mut buf = vec![0.0f32; 400 * ch];
        let n = d.read(&mut buf).unwrap();
        assert!(n > 0, "opus {ms} ms: nothing after the seek");
        let expect = (at.as_secs_f64() * f64::from(OPUS_RATE)).round() as usize;
        assert_eq!(
            best_match(&full, &buf[..n * ch], ch, expect, 64),
            expect,
            "opus {ms} ms: the seek landed on the wrong sample"
        );
    }
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

    // An Opus file carries its tags in the Opus comment header, which holds
    // the same Vorbis comments as an Ogg Vorbis file.
    let m = open_path("sine.opus").metadata();
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
    assert_eq!(p(Some("opus"), b""), Confidence::EXTENSION);
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
        "sine.opus",
        "sine_opus.mkv",
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
    for e in ["mp3", "flac", "ogg", "opus", "wav", "aiff", "m4a"] {
        assert!(exts.iter().any(|x| x == e), "{e}");
    }
}

#[test]
fn url_is_rejected_with_clear_error() {
    // The symphonia factory reads files and readers. An `http://` stream is
    // opened with `rustamp_decoders::HttpDecoder` (see `tests/http_stream.rs`),
    // so the refusal names the way that works.
    let err = SymphoniaDecoderFactory::new()
        .open(MediaSource::Url("http://example.com/a.mp3".into()))
        .err()
        .expect("must fail");
    match err {
        Error::Unsupported(m) => assert!(m.contains("HttpDecoder"), "{m}"),
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

/// The count must also be visible through the trait object the engine holds.
/// An inherent method alone silently returned the trait default of zero.
#[test]
fn skipped_packets_are_visible_through_the_decoder_trait() {
    let (_, d) = corrupt_decode("sine_alac.m4a", "m4a");
    let expected = d.skipped_packets();
    assert!(expected > 0, "fixture did not produce a skip");
    let boxed: Box<dyn Decoder> = Box::new(d);
    assert_eq!(boxed.skipped_packets(), expected);
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

/// The engine opens a source on one thread and decodes on another, so a decoder
/// built from `MediaSource::Reader` — and with it the reader wrapper inside —
/// must be `Send`. The decoder is opened here and only then moved to the
/// worker, so this stops compiling if that chain loses `Send` (RFC 0002, C2).
#[test]
fn reader_source_decoder_decodes_on_another_thread() {
    let mut d = open_bytes(std::fs::read(fixture("sine.flac")).unwrap(), Some("flac"));
    let decoded: u64 = std::thread::spawn(move || (read_all(d.as_mut()).len() / 2) as u64)
        .join()
        .expect("decode thread");
    assert_eq!(decoded, FIXTURE_FRAMES);
}
