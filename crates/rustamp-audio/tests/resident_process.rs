//! The engine adapter against a **real second process** (RFC 0006 §5).
//!
//! Everything else in this crate's resident suite drives the adapter over an
//! in-process control pair; this file drives the production path: a helper
//! process in its own address space, the real named PCM ring, the real
//! control channel, and the engine's own [`Decoder`] seam in front of it.
//!
//! # VM only
//!
//! Every test that spawns a helper is `#[ignore]`d: the default
//! `cargo test --workspace` must not start a second process on the host. The
//! Hyper-V test VM runs them with `--ignored`.
//!
//! Two helpers are used, and both are staged by the VM runner:
//!
//! * `RUSTAMP_DECODER_FIXTURE` — the mediahost **stub** resident
//!   (`cargo build -p rustamp-mediahost --example decoder_fixture`). It
//!   speaks the real wire and serves a deterministic pattern, so the adapter
//!   can be pinned exactly without a codec.
//! * `RUSTAMP_DECHOST_BINARY` — the **real** decoder resident
//!   (`rustamp-dechost-symphonia`), which decodes actual files through the
//!   decoder registry. A test that needs it skips with a note when the
//!   runner did not stage it; it never pretends to have run.
//!
//! # What is pinned
//!
//! * the handshake is replayed: format, duration, seekability, metadata;
//! * the PCM that crosses the ring is bit-identical to the same decoder
//!   running in-process;
//! * a seek runs the real handshake (new ring, new generation) and lands;
//! * the end of the stream is sticky, and only after the ring is drained;
//! * a helper that dies mid-stream is a **loud** decode error, never an
//!   end-of-stream and never silence;
//! * a missing helper binary degrades to "format unsupported".

#![cfg(windows)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustamp_audio::resident::{ResidentConfig, ResidentFactory};
use rustamp_audio::{Engine, EngineConfig, TrackSource};
use rustamp_core::audio::{AudioSpec, Decoder, DecoderFactory, DecoderRegistry, MediaSource};
use rustamp_core::bus::EventBus;
use rustamp_core::error::Error as CoreError;
use rustamp_mediahost::control::Liveness;
use rustamp_output::NullOutput;

/// Stall escape for every wait in this file (the M10 rule): a bound so
/// generous that only a real stall reaches it, with the message naming the
/// step. Never a wall-clock bound a healthy-but-slow VM schedule can trip.
const WAIT: Duration = Duration::from_secs(30);

/// Liveness the engine supervises the helper with. The framework's 5 s
/// default is the wall-clock trip wire this guest proved flakey (M24 §1.3);
/// the engine's own default is a stall escape (see
/// `rustamp_audio::resident::DEFAULT_RESIDENT_LIVENESS`).
const LIVENESS: Liveness = Liveness {
    interval: Duration::from_millis(100),
    timeout: Duration::from_secs(30),
};

/// The stub resident's format and length, as the fixture serves them.
const STUB_SPEC: AudioSpec = AudioSpec::new(1000, 2);
/// Frames the stub resident serves.
const STUB_FRAMES: u64 = 4099;

/// The stub resident's telemetry (`DecoderStats`), as the fixture reports it.
const STUB_SKIPPED: u64 = 7;
const STUB_BITRATE: u32 = 128;

/// The stub resident's mode: serve the pattern.
const STUB_PLAIN: &str = "fixture:plain";
/// The stub resident's mode: die in the middle of the stream.
const STUB_EXIT: &str = "fixture:exit";

/// A file of the repository's decoder fixtures.
fn repo_fixture(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rustamp-decoders/tests/fixtures")
        .join(name);
    assert!(path.is_file(), "repo fixture missing: {}", path.display());
    path
}

/// The stub resident's binary, staged by the VM runner.
fn stub_helper() -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("RUSTAMP_DECODER_FIXTURE")?);
    if path.is_file() {
        Some(path)
    } else {
        eprintln!(
            "SKIP: RUSTAMP_DECODER_FIXTURE points at {}, which is not a file",
            path.display()
        );
        None
    }
}

/// The real decoder resident's binary.
///
/// The VM runner stages it and names it in `RUSTAMP_DECHOST_BINARY`; on a
/// development machine the workspace build leaves it next to the test
/// binaries, which is where the fallback looks.
fn real_helper() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("RUSTAMP_DECHOST_BINARY") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
        eprintln!(
            "SKIP: RUSTAMP_DECHOST_BINARY points at {}, which is not a file",
            path.display()
        );
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    // <target>/debug/deps/<test>.exe -> <target>/debug/<helper>.exe
    let candidate = exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(|debug| debug.join("rustamp-dechost-symphonia.exe"))?;
    if candidate.is_file() {
        Some(candidate)
    } else {
        eprintln!(
            "SKIP: no real decoder resident staged (looked for {}); \
             the VM runner must stage rustamp-dechost-symphonia.exe",
            candidate.display()
        );
        None
    }
}

/// The engine's resident configuration for one helper.
fn config(helper: PathBuf) -> ResidentConfig {
    ResidentConfig::new(helper)
        .with_timeouts(WAIT, WAIT)
        .with_liveness(LIVENESS)
}

/// Decode a file in-process, for the bit-exact comparison.
fn direct_decode(path: &Path) -> (AudioSpec, Vec<f32>) {
    let mut decoder = rustamp_decoders::default_registry()
        .open(MediaSource::Path(path.to_path_buf()))
        .expect("the in-process decoder opens the file");
    let spec = decoder.spec();
    let channels = usize::from(spec.channels);
    let mut out = Vec::new();
    let mut buf = vec![0.0f32; 2048 * channels];
    loop {
        match decoder
            .read(&mut buf)
            .expect("the in-process decoder reads")
        {
            0 => break,
            frames => out.extend_from_slice(&buf[..frames * channels]),
        }
    }
    (spec, out)
}

/// The raw bits of every sample: bit-identity, not float equality.
fn bits(samples: &[f32]) -> Vec<u32> {
    samples.iter().map(|s| s.to_bits()).collect()
}

/// Read a decoder to its end and return every interleaved sample.
fn read_all(decoder: &mut dyn Decoder, spec: AudioSpec) -> Vec<f32> {
    let channels = usize::from(spec.channels);
    let mut out = Vec::new();
    let mut buf = vec![0.0f32; 2048 * channels];
    loop {
        match decoder.read(&mut buf).expect("the resident delivers") {
            0 => break,
            frames => out.extend_from_slice(&buf[..frames * channels]),
        }
    }
    out
}

#[test]
fn a_missing_helper_binary_degrades_to_an_unsupported_format() {
    // No process is started: the spawn fails before a child exists, so this
    // runs on the host too. RFC 0006 §8: "if a helper is missing, its
    // features degrade to 'format unsupported', never to a crash".
    let missing = std::env::temp_dir().join("rustamp-no-such-resident-helper.exe");
    let _ = std::fs::remove_file(&missing);
    let factory = ResidentFactory::new(config(missing));
    let outcome = factory.open(MediaSource::Path(PathBuf::from("track.mp3")));
    match outcome {
        Err(CoreError::UnsupportedFormat(message)) => {
            assert!(
                message.contains("resident helper"),
                "the degradation must name the missing helper: {message}"
            );
        }
        Err(other) => panic!("a missing helper must degrade, got {other}"),
        Ok(_) => panic!("a missing helper must not produce a decoder"),
    }
}

#[test]
#[ignore = "two processes; Hyper-V VM only; requires RUSTAMP_DECODER_FIXTURE"]
fn the_stub_resident_decodes_through_the_engine_adapter() {
    let Some(helper) = stub_helper() else {
        return;
    };
    let factory = ResidentFactory::new(config(helper));
    let mut decoder = factory
        .open(MediaSource::Url(STUB_PLAIN.into()))
        .expect("the stub resident opens");

    // The handshake is replayed: the format, the length, the seekability.
    assert_eq!(decoder.spec(), STUB_SPEC);
    assert_eq!(decoder.duration(), Some(Duration::from_millis(STUB_FRAMES)));
    assert!(decoder.is_seekable());

    // The first frames carry the stub's pattern.
    let mut buf = vec![0.0f32; 2 * 8];
    let frames = decoder.read(&mut buf).expect("the stub delivers");
    assert!(frames > 0, "the stub must deliver frames");
    for index in 0..frames {
        let value = index as f32 / 8.0;
        assert_eq!(buf[index * 2], value);
        assert_eq!(buf[index * 2 + 1], -value);
    }

    // The seek handshake runs for real: the host closes the ring, offers a
    // fresh one with a new generation and answers with the reached position.
    let reached = decoder
        .seek(Duration::from_millis(100))
        .expect("the stub seeks");
    assert_eq!(reached, Duration::from_millis(100));

    // After the seek the pattern continues at the reached position.
    let frames = decoder
        .read(&mut buf)
        .expect("the stub delivers after the seek");
    assert!(
        frames > 0,
        "a seek inside the stream must keep yielding audio"
    );
    assert_eq!(buf[0], 100.0 / 8.0);
    assert_eq!(buf[1], -100.0 / 8.0);

    // The telemetry the host reported is replayed on the engine's side.
    assert_eq!(decoder.skipped_packets(), STUB_SKIPPED);
    assert_eq!(decoder.stream_info().bitrate_kbps, Some(STUB_BITRATE));

    // The end of the stream is sticky, and it comes only after the ring is
    // drained: the stub's own frame count is delivered first.
    let mut total = 0u64;
    let mut buf = vec![0.0f32; 2 * 257];
    loop {
        match decoder.read(&mut buf).expect("the stub delivers") {
            0 => break,
            frames => total += frames as u64,
        }
    }
    assert_eq!(
        total,
        STUB_FRAMES - 100,
        "every frame up to the end must arrive, none invented"
    );
    assert_eq!(
        decoder
            .read(&mut buf)
            .expect("the sticky end of the stream"),
        0
    );
}

#[test]
#[ignore = "two processes; Hyper-V VM only; requires RUSTAMP_DECODER_FIXTURE"]
fn a_helper_that_dies_mid_stream_is_a_loud_decode_error() {
    let Some(helper) = stub_helper() else {
        return;
    };
    let factory = ResidentFactory::new(config(helper));
    let mut decoder = factory
        .open(MediaSource::Url(STUB_EXIT.into()))
        .expect("the stub resident opens");

    // The first block arrives; the helper dies right after it.
    let mut buf = vec![0.0f32; 2 * 64];
    match decoder.read(&mut buf) {
        Ok(frames) => {
            assert!(frames > 0, "the stub must deliver one block before it dies");
            // The next read must be a loud failure. It must never look like
            // the end of the stream, and it must never hang: the control EOF
            // wakes the blocked read through the framework's close-only ring
            // view.
            let error = decoder
                .read(&mut buf)
                .expect_err("a dead helper must not look like the end of the stream");
            assert!(
                matches!(&error, CoreError::Decode(message) if !message.is_empty()),
                "the death must surface as a loud decode error, got {error:?}"
            );
            // RFC 0006 §8: the engine restarts the helper once before it gives
            // up on the track, and the second death is what ends it.
            assert!(
                error.to_string().contains("resident restart"),
                "the track must have been restarted once, got {error}"
            );
        }
        Err(error) => {
            // The helper died before its first block could be read (a heavily
            // loaded guest). The contract still holds: the death is loud and
            // never an end of stream.
            assert!(
                matches!(&error, CoreError::Decode(message) if !message.is_empty()),
                "the death must surface as a loud decode error, got {error:?}"
            );
            assert!(
                error.to_string().contains("resident restart"),
                "the track must have been restarted once, got {error}"
            );
        }
    }
}

#[test]
#[ignore = "two processes; Hyper-V VM only; requires the staged real resident"]
fn the_real_resident_decodes_a_real_file_bit_exactly() {
    let Some(helper) = real_helper() else {
        return;
    };
    let path = repo_fixture("sine.flac");
    let (spec, expected) = direct_decode(&path);

    let factory = ResidentFactory::new(config(helper));
    let mut decoder = factory
        .open(MediaSource::Path(path.clone()))
        .expect("the real resident opens the file");

    // The handshake is replayed: format, length and the tags the file
    // carries.
    assert_eq!(decoder.spec(), spec);
    assert_eq!(decoder.duration(), Some(Duration::from_millis(1500)));
    assert_eq!(decoder.metadata().title.as_deref(), Some("Sine Title"));
    assert!(decoder.is_seekable());

    // The PCM that crossed the process boundary is bit-identical to the same
    // decoder running in-process.
    let received = read_all(decoder.as_mut(), spec);
    assert_eq!(received.len(), expected.len());
    assert_eq!(bits(&received), bits(&expected));
    // The end of the stream is sticky.
    let mut buf = vec![0.0f32; 2 * 64];
    assert_eq!(
        decoder
            .read(&mut buf)
            .expect("the sticky end of the stream"),
        0
    );
}

#[test]
#[ignore = "two processes; Hyper-V VM only; requires the staged real resident"]
fn the_real_resident_seeks_and_lands_on_the_same_audio() {
    let Some(helper) = real_helper() else {
        return;
    };
    let path = repo_fixture("sine.flac");
    let (spec, _) = direct_decode(&path);

    let factory = ResidentFactory::new(config(helper));
    let mut decoder = factory
        .open(MediaSource::Path(path.clone()))
        .expect("the real resident opens the file");

    // The seek handshake reaches the position the host reports.
    let reached = decoder
        .seek(Duration::from_millis(1000))
        .expect("the real resident seeks");
    assert_eq!(reached, Duration::from_millis(1000));

    // And the audio after the seek is the audio the same decoder produces
    // in-process after the same seek.
    let mut in_process = rustamp_decoders::default_registry()
        .open(MediaSource::Path(path))
        .expect("the in-process decoder opens the file");
    in_process
        .seek(Duration::from_millis(1000))
        .expect("the in-process decoder seeks");

    let channels = usize::from(spec.channels);
    let mut want = Vec::new();
    let mut buf = vec![0.0f32; 512 * channels];
    while want.len() < 512 * channels {
        let frames = in_process.read(&mut buf).expect("the in-process read");
        assert!(
            frames > 0,
            "the in-process decoder must keep yielding audio"
        );
        want.extend_from_slice(&buf[..frames * channels]);
    }
    want.truncate(512 * channels);

    let mut got = Vec::new();
    let mut buf = vec![0.0f32; 512 * channels];
    while got.len() < 512 * channels {
        let frames = decoder.read(&mut buf).expect("the resident delivers");
        assert!(frames > 0, "the resident must keep yielding audio");
        got.extend_from_slice(&buf[..frames * channels]);
    }
    got.truncate(512 * channels);
    assert_eq!(bits(&got), bits(&want));
}

#[test]
#[ignore = "two processes; Hyper-V VM only; requires the staged real resident"]
fn the_engine_plays_a_track_through_the_real_resident() {
    let Some(helper) = real_helper() else {
        return;
    };
    let path = repo_fixture("sine.flac");
    let (spec, expected) = direct_decode(&path);

    // The production path: the registry holds the resident factory, so the
    // engine opens the track through the helper process and knows nothing
    // about it.
    let mut registry = DecoderRegistry::new();
    registry.register(std::sync::Arc::new(ResidentFactory::new(config(helper))));
    let (output, clock) = NullOutput::pair();
    let bus = EventBus::new();
    let mut engine = Engine::new(registry, Box::new(output), bus, EngineConfig::default())
        .expect("engine construction");
    // Bit-exact comparisons: the limiter would clamp the fixture's peaks.
    engine.set_limiter_enabled(false);
    engine.set_queue(vec![TrackSource::path(path.display().to_string())]);
    engine.play_index(0).expect("the track starts");

    // The track opens through the helper and reports its length.
    let deadline = Instant::now() + WAIT;
    loop {
        engine.poll().expect("poll");
        if engine.current_index() == Some(0) && engine.duration_frames().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "the resident track must open");
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(engine.source_spec(), Some(spec));

    // Render the track. The clock is only pumped by as many frames as the
    // engine's ring actually holds, so the recording never contains the
    // silence of an underrun: every rendered frame is real audio.
    let channels = usize::from(spec.channels);
    let expected_frames = (expected.len() / channels) as u64;
    let mut recorded = Vec::new();
    let mut pumped = 0u64;
    let deadline = Instant::now() + WAIT;
    while pumped < expected_frames {
        engine.poll().expect("poll");
        let buffered = engine.buffered_frames();
        if buffered == 0 {
            assert!(
                !engine.is_drain_pending(),
                "a fresh run must not wait for a drain"
            );
            if engine.transport() == rustamp_core::state::TransportState::Stopped {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the resident must keep feeding the engine"
            );
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        let step = buffered.min(1024).min(expected_frames - pumped);
        let rendered = clock.pump_record(step, &mut recorded);
        if rendered == 0 {
            // The device stopped: the run is over.
            break;
        }
        pumped += rendered;
    }
    assert_eq!(
        pumped, expected_frames,
        "the whole track must be rendered through the resident"
    );
    recorded.truncate(expected.len());

    let left: Vec<f32> = recorded.iter().step_by(channels).copied().collect();
    let want: Vec<f32> = expected.iter().step_by(channels).copied().collect();
    assert_eq!(left.len(), want.len(), "the whole track must be rendered");
    assert_eq!(
        bits(&left),
        bits(&want),
        "the engine must render exactly what the in-process decoder produces"
    );
}
