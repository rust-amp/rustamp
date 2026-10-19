//! Real-decode resident tests: the run loop against byte-stream control pairs
//! and real fixture files. Headless; one Windows process, no window, no audio.
//! Run in the Hyper-V test VM, never on the host.
//!
//! Waiting rule (see the M10 flakiness note): PCM reads wait on the condition
//! (data, EOS or a terminal failure wakes them), never on a wall-clock bound
//! that a healthy-but-slow schedule could trip. Every such wait still carries
//! a generous stall escape (`STALL_WAIT`): only a wedged or dead resident
//! reaches it, and reaching it fails loudly with the step named — a stall is
//! never reported as success. Only joining the host thread uses the same
//! generous bound (`JOIN_WAIT`), which a healthy host reaches in milliseconds;
//! the message on expiry names the stall. `DecoderSession::open` and `seek`
//! inherit the framework's 5 s `CONTROL_WAIT` for a single handshake exchange.
//!
//! What the escape used to hide (docs/tasks/W19-twoproc-stall.md): the
//! framework spent the whole 30 s inside **one** wait on the ring's wake
//! event, so a signal the ring skipped cost the entire bound instead of being
//! noticed at the next look at the counters.
//! `end_of_stream_comes_only_after_the_ring_is_drained` is the most exposed
//! test in this file — three-frame ring, 32-frame reads, so the most
//! handoffs — and that is the stall it hit. The framework now slices every
//! PCM wait (`rustamp-mediahost`'s `pcm::RING_POLL`), and the escape below
//! prints the ring counters so a future red says which of the two causes it
//! was.
//!
//! Terminal classification (docs/tasks/M14-host-framework-review.md, F2): a
//! stream that was cut short is a **loud failure**, never a clean end of
//! stream. The tests below pin both terminals separately: a truncated file
//! fails with a diagnostic that names the truncation, an intact file ends
//! with an end of stream and the frame count its container declares, and a
//! stream that holds no frames at all ends with an end of stream too (there
//! is nothing missing). A container whose reader scans the whole file before
//! it plays anything (MP4) refuses a cut file when it is opened instead:
//! that terminal is a refusal with the same diagnostic, and it is pinned
//! separately.

#![cfg(windows)]

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rustamp_core::audio::MediaSource;
use rustamp_dechost::RegistryResident;
use rustamp_mediahost::control::{ControlPeer, Liveness};
use rustamp_mediahost::decoder::{DecoderSession, serve_decoder};
use rustamp_mediahost::pcm::NamedPcmRings;
use rustamp_mediahost::{MediaHostError, Result};
use rustamp_proto::host::MediaLocationWire;

const JOIN_WAIT: Duration = Duration::from_secs(30);
const SEEK_WAIT: Duration = Duration::from_secs(30);
/// Stall bound for every PCM read (see the file docs): generous enough that
/// only a real stall reaches it, and a timeout is a loud failure that names
/// the step, never a pass.
const STALL_WAIT: Duration = Duration::from_secs(30);
/// Liveness policy for the in-process control peers. The timeout is a stall
/// escape, never a schedule bound (the M10 flake class): a dead resident
/// never answers a Ping, so only a real death reaches it, and matching
/// STALL_WAIT keeps a healthy-but-slow schedule from ever tripping it -
/// exactly what the waiting rule above forbids. No test in this file
/// depends on a fast liveness expiry: the only dead-host case here is the
/// player vanishing, which the control EOF (not the Ping deadline) detects.
const LIVENESS: Liveness = Liveness {
    interval: Duration::from_millis(100),
    timeout: STALL_WAIT,
};

fn repo_fixture(name: &str) -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rustamp-decoders/tests/fixtures")
        .join(name);
    assert!(path.is_file(), "repo fixture missing: {}", path.display());
    path
}

fn file_location(path: &Path) -> MediaLocationWire {
    MediaLocationWire::File(path.to_string_lossy().into_owned())
}

struct ByteReader {
    rx: mpsc::Receiver<Vec<u8>>,
    bytes: Cursor<Vec<u8>>,
}

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

/// Serve a real file on a host thread; return the player session plus the
/// host outcome. The ring is deliberately small so every run wraps it.
fn start(
    location: MediaLocationWire,
    capacity: u32,
) -> (DecoderSession<NamedPcmRings>, mpsc::Receiver<Result<()>>) {
    let ((ar, aw), (br, bw)) = streams();
    let client = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let outcome = serve_decoder(&host, &NamedPcmRings, &RegistryResident);
        let _ = done.send(outcome);
    });
    let session = DecoderSession::open(client, NamedPcmRings, location, capacity).unwrap();
    (session, result)
}

fn finish_ok(session: DecoderSession<NamedPcmRings>, done: mpsc::Receiver<Result<()>>) {
    drop(session);
    done.recv_timeout(JOIN_WAIT)
        .expect("host must stop after Shutdown instead of stalling")
        .unwrap();
}

/// Read PCM with a stall escape (see the file docs): a healthy resident
/// delivers within milliseconds, so a timeout means it stopped answering
/// without EOS or an error — a stall that fails loudly, never a pass.
///
/// The escape reports the ring counters, because red on its own cannot tell
/// the two causes apart (docs/tasks/W19-twoproc-stall.md): a nonzero
/// `available` means the frames were already published and this wait slept
/// on them (a skipped wake), a zero one means the resident published nothing
/// for thirty seconds (a wedged or starved host).
fn read_pcm(
    session: &mut DecoderSession<NamedPcmRings>,
    buf: &mut [f32],
    step: &str,
) -> Result<usize> {
    match session.read(buf, Some(STALL_WAIT)) {
        Err(MediaHostError::TimedOut) => {
            let ring = session.ring_counters();
            panic!(
                "stall: {step} received no PCM within {STALL_WAIT:?} \
                 (the resident stopped delivering without EOS or an error); \
                 ring at the escape: published {}, read {}, available {}, closed {}",
                ring.published,
                ring.read,
                ring.available(),
                ring.closed
            )
        }
        other => other,
    }
}

/// Decode the same file in-process for the bit-exact comparison.
fn direct_decode(path: &Path) -> Vec<f32> {
    let mut decoder = rustamp_decoders::default_registry()
        .open(MediaSource::Path(path.to_path_buf()))
        .expect("direct open");
    let channels = usize::from(decoder.spec().channels);
    let mut out = Vec::new();
    let mut buf = vec![0.0f32; 2048 * channels];
    loop {
        match decoder.read(&mut buf).expect("direct read") {
            0 => return out,
            frames => out.extend_from_slice(&buf[..frames * channels]),
        }
    }
}

/// Decode a file in-process and report how the decode ended.
///
/// The end of a stream is one of two things: the file held everything its
/// container declared and the decode ends with an end of stream, or the file
/// was cut and the decode fails. The second one is a failure, not a short
/// end of stream, so the two are kept apart here.
fn direct_decode_terminal(path: &Path) -> (Vec<f32>, Option<String>) {
    let mut decoder = rustamp_decoders::default_registry()
        .open(MediaSource::Path(path.to_path_buf()))
        .expect("direct open");
    let channels = usize::from(decoder.spec().channels);
    let mut out = Vec::new();
    let mut buf = vec![0.0f32; 2048 * channels];
    loop {
        match decoder.read(&mut buf) {
            Ok(0) => return (out, None),
            Ok(frames) => out.extend_from_slice(&buf[..frames * channels]),
            Err(error) => return (out, Some(error.to_string())),
        }
    }
}

fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let path = std::env::temp_dir().join(format!(
        "rustamp-dechost-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, bytes).expect("temp fixture");
    path
}

/// Minimal 16-bit PCM WAV writer (mono/stereo); std only, no new dependency.
fn wav_bytes(sample_rate: u32, channels: u16, frames: &[[f32; 2]]) -> Vec<u8> {
    let data_len = frames.len() * usize::from(channels) * 2;
    let mut out = Vec::with_capacity(44 + data_len);
    let put = |v: &mut Vec<u8>, b: &[u8]| v.extend_from_slice(b);
    put(&mut out, b"RIFF");
    put(&mut out, &(36 + data_len as u32).to_le_bytes());
    put(&mut out, b"WAVEfmt ");
    put(&mut out, &16u32.to_le_bytes());
    put(&mut out, &1u16.to_le_bytes());
    put(&mut out, &channels.to_le_bytes());
    put(&mut out, &sample_rate.to_le_bytes());
    put(
        &mut out,
        &(sample_rate * u32::from(channels) * 2).to_le_bytes(),
    );
    put(&mut out, &(channels * 2).to_le_bytes());
    put(&mut out, &16u16.to_le_bytes());
    put(&mut out, b"data");
    put(&mut out, &(data_len as u32).to_le_bytes());
    for frame in frames {
        for c in 0..channels {
            let sample = (frame[usize::from(c)].clamp(-1.0, 1.0) * 32767.0).round() as i16;
            put(&mut out, &sample.to_le_bytes());
        }
    }
    out
}

#[test]
fn real_file_opens_with_format_handshake_and_decodes_bit_exact() {
    let path = repo_fixture("sine.flac");
    let (mut session, done) = start(file_location(&path), 7);
    let info = session.info();
    assert_eq!(info.spec.sample_rate_hz, 44_100);
    assert_eq!(info.spec.channels, 2);
    assert_eq!(info.duration_ms, Some(1500));
    assert!(info.seekable);
    assert_eq!(info.metadata.title.as_deref(), Some("Sine Title"));

    let mut received = Vec::new();
    let mut buf = [0.0f32; 2048];
    loop {
        match read_pcm(&mut session, &mut buf, "the full-file transfer").unwrap() {
            0 => break,
            frames => received.extend_from_slice(&buf[..frames * 2]),
        }
    }
    let expected = direct_decode(&path);
    assert_eq!(received.len(), expected.len());
    assert_eq!(
        received.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        expected.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        "ring delivery must match the same decoder in-process"
    );
    assert_eq!(session.frames_dropped(), 0);
    // EOS is sticky: a second read stays at zero.
    assert_eq!(
        read_pcm(&mut session, &mut buf, "the sticky EOS read").unwrap(),
        0
    );
    finish_ok(session, done);
}

#[test]
fn end_of_stream_comes_only_after_the_ring_is_drained() {
    let path = repo_fixture("sine.flac");
    let expected = direct_decode(&path);
    let (mut session, done) = start(file_location(&path), 3);
    let mut received = Vec::new();
    let mut buf = [0.0f32; 64];
    loop {
        match read_pcm(&mut session, &mut buf, "the drained transfer").unwrap() {
            0 => break,
            frames => received.extend_from_slice(&buf[..frames * 2]),
        }
    }
    assert!(
        !received.is_empty(),
        "the stream must deliver frames before it terminates"
    );
    assert_eq!(
        received.len(),
        expected.len(),
        "no frame may be cut before EOS and none invented after it"
    );
    assert_eq!(
        read_pcm(&mut session, &mut buf, "the sticky EOS read").unwrap(),
        0
    );
    finish_ok(session, done);
}

#[test]
fn seek_past_the_end_clamps_and_terminates() {
    let path = repo_fixture("sine.flac");
    let (mut session, done) = start(file_location(&path), 7);
    let reached = session.seek(60_000, SEEK_WAIT).unwrap();
    assert!(
        reached <= 1500,
        "seek past the end must clamp to the duration, got {reached}"
    );
    let mut buf = [0.0f32; 64];
    assert_eq!(
        read_pcm(&mut session, &mut buf, "the clamped-seek termination read").unwrap(),
        0,
        "a clamped seek to the end must terminate, not hang"
    );
    finish_ok(session, done);
}

#[test]
fn seek_inside_matches_direct_decode() {
    let path = repo_fixture("sine.flac");
    let (mut session, done) = start(file_location(&path), 7);
    let reached = session.seek(1000, SEEK_WAIT).unwrap();
    assert_eq!(reached, 1000);
    let mut buf = [0.0f32; 200];
    // The ring is deliberately tiny (7 frames), so one read returns what is
    // available and never the whole request: loop to the same 100 frames the
    // direct decode gathers below.
    let mut got: Vec<f32> = Vec::new();
    while got.len() < 200 {
        let frames = read_pcm(&mut session, &mut buf, "the post-seek read").unwrap();
        assert!(
            frames > 0,
            "a seek inside the file must keep yielding audio"
        );
        got.extend_from_slice(&buf[..frames * 2]);
    }
    got.truncate(200);

    let mut direct = rustamp_decoders::default_registry()
        .open(MediaSource::Path(path))
        .expect("direct open");
    direct
        .seek(Duration::from_millis(1000))
        .expect("direct seek");
    // Collect the same 100 frames the hosted side gathered: a backend may
    // return short reads, so loop instead of assuming one call fills the
    // request. The count and the bit-exact comparison below still pin the
    // contract.
    let mut want: Vec<f32> = Vec::new();
    let mut direct_buf = vec![0.0f32; 200];
    while want.len() < 200 {
        let frames = direct.read(&mut direct_buf).expect("direct read");
        assert!(
            frames > 0,
            "the direct decode must keep yielding audio after the seek"
        );
        want.extend_from_slice(&direct_buf[..frames * 2]);
    }
    want.truncate(200);
    assert_eq!(want.len(), 200, "the direct read fills the whole request");
    assert_eq!(got, want);
    finish_ok(session, done);
}

#[test]
fn garbage_and_empty_files_are_clean_errors_never_eos() {
    for (name, bytes) in [("garbage.bin", vec![7u8; 256]), ("empty.wav", Vec::new())] {
        let path = temp_file(name, &bytes);
        let ((ar, aw), (br, bw)) = streams();
        let client = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
        let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
        let (done, result) = mpsc::channel();
        thread::spawn(move || {
            let outcome = serve_decoder(&host, &NamedPcmRings, &RegistryResident);
            let _ = done.send(outcome);
        });
        let outcome = DecoderSession::open(client, NamedPcmRings, file_location(&path), 3);
        // The honest contract is a loud refusal that carries the resident's
        // reason — never an accepted session, never EOS. The error variant is
        // deliberately not pinned: the framework's control monitor consumes
        // the resident's terminal DecoderError and fails the peer sticky
        // (rustamp-mediahost/src/control.rs), so the player sees
        // Disconnected("resident decoder failed: operation failed: ..."),
        // and the OperationFailed arm in DecoderSession::open is unreachable
        // (the monitor never forwards DecoderError to the session). The
        // framework's own resident-failure test pins the reason, not the
        // variant; this one does the same and keeps the reason.
        let described = match &outcome {
            Ok(_) => "Ok(session): the undecodable file was accepted".to_string(),
            Err(error) => format!("Err({error:?})"),
        };
        let Err(error) = outcome else {
            panic!("{name}: undecodable input must be a loud error, never EOS (got {described})");
        };
        assert!(
            error.to_string().contains("operation failed"),
            "{name}: the loud error must carry the resident's refusal reason (got {described})"
        );
        result
            .recv_timeout(JOIN_WAIT)
            .expect("host must end after a failed open instead of stalling")
            .expect_err("a failed open must end the host with an error");
        let _ = std::fs::remove_file(&path);
    }
}

/// Read the whole stream and report how it ended.
///
/// `None` is a clean end of stream; `Some(message)` is the failure that
/// ended the transfer. A stall is a panic (see [`read_pcm`]), never a
/// terminal of its own.
fn drain_pcm(session: &mut DecoderSession<NamedPcmRings>, step: &str) -> (usize, Option<String>) {
    let mut buf = [0.0f32; 128];
    let mut frames = 0usize;
    loop {
        match read_pcm(session, &mut buf, step) {
            Ok(0) => return (frames, None),
            Ok(n) => frames += n,
            Err(error) => return (frames, Some(error.to_string())),
        }
    }
}

#[test]
fn a_truncated_file_is_a_loud_decode_failure_and_not_an_end_of_stream() {
    let full: Vec<[f32; 2]> = (0..4000)
        .map(|i| {
            let t = i as f32 / 8000.0;
            [(t * 440.0).sin() * 0.5, (t * 550.0).sin() * 0.5]
        })
        .collect();
    let mut bytes = wav_bytes(8000, 1, &full);
    // Cut the file inside the sample data, in the middle of a frame: the
    // header still claims 4000 frames, the file holds 2999 of them.
    bytes.truncate(bytes.len() - 2001);
    let path = temp_file("truncated.wav", &bytes);

    // The same decoder in-process: the frames the file holds, then the
    // failure. A truncated file must not look like a finished one.
    let (expected, failure) = direct_decode_terminal(&path);
    let failure = failure.expect("a truncated file must fail, not end the stream");
    assert!(
        failure.contains("truncated"),
        "the failure must name the truncation: {failure}"
    );
    assert_eq!(expected.len(), 2999, "the frames the file holds");

    let (mut session, done) = start(file_location(&path), 7);
    let (frames, failure) = drain_pcm(&mut session, "the truncated-file transfer");
    let failure =
        failure.expect("a truncated file must be a loud failure, never a clean end of stream");
    assert!(
        failure.contains("truncated"),
        "the failure must name the truncation: {failure}"
    );
    assert!(
        frames > 0 && frames <= expected.len(),
        "the frames the file holds are delivered before the failure, and no more: {frames}"
    );
    // The host ends with the resident's failure, not with a clean shutdown.
    let error = done
        .recv_timeout(JOIN_WAIT)
        .expect("the host must end after a failed decode instead of stalling")
        .expect_err("a failed decode must end the host with an error");
    assert!(
        error.to_string().contains("truncated"),
        "the host error must name the truncation: {error}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_intact_file_ends_with_a_clean_end_of_stream() {
    let full: Vec<[f32; 2]> = (0..4000)
        .map(|i| {
            let t = i as f32 / 8000.0;
            [(t * 440.0).sin() * 0.5, (t * 550.0).sin() * 0.5]
        })
        .collect();
    let bytes = wav_bytes(8000, 1, &full);
    let path = temp_file("intact.wav", &bytes);

    // The in-process reference: the whole file, then an end of stream.
    let (expected, failure) = direct_decode_terminal(&path);
    assert!(
        failure.is_none(),
        "a complete file must not fail: {failure:?}"
    );
    assert_eq!(expected.len(), 4000, "the whole file is decoded");

    let (mut session, done) = start(file_location(&path), 7);
    let (frames, failure) = drain_pcm(&mut session, "the intact-file transfer");
    assert!(
        failure.is_none(),
        "a complete file must end with an end of stream, not a failure: {failure:?}"
    );
    assert_eq!(
        frames,
        expected.len(),
        "every frame of the file is delivered"
    );
    assert_eq!(
        read_pcm(&mut session, &mut [0.0f32; 128], "the sticky EOS read").unwrap(),
        0
    );
    assert_eq!(session.frames_dropped(), 0);
    finish_ok(session, done);
    let _ = std::fs::remove_file(&path);
}

/// A truncated MP4 file is refused when it is opened, and the refusal names
/// the truncation.
///
/// The MP4 container reader scans the whole file before it plays anything, so
/// a cut MP4 file never reaches the transfer: it is refused at the open.
/// The terminal shape is therefore the one of an unreadable file, not the one
/// of a failed transfer - and the reason that reaches the player names the
/// cut, so a half-downloaded track is not mistaken for a finished one.
#[test]
fn a_truncated_mp4_is_refused_at_the_open_and_never_ends_the_stream() {
    let bytes = std::fs::read(repo_fixture("sine_alac.m4a")).expect("fixture");
    // Cut the file inside the media data: the movie box is behind it and is
    // gone with it.
    let cut = bytes[..bytes.len() / 2].to_vec();
    let path = temp_file("truncated.m4a", &cut);

    // The in-process reference: the file is refused when it is opened, and
    // the refusal names the cut.
    let error = rustamp_decoders::default_registry()
        .open(MediaSource::Path(path.clone()))
        .err()
        .expect("a truncated mp4 must not open");
    assert!(
        error.to_string().contains("truncated"),
        "the refusal must name the truncation: {error}"
    );

    // The hosted path: the session never opens, so the player sees a loud
    // refusal and never an end of stream.
    let ((ar, aw), (br, bw)) = streams();
    let client = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let outcome = serve_decoder(&host, &NamedPcmRings, &RegistryResident);
        let _ = done.send(outcome);
    });
    let outcome = DecoderSession::open(client, NamedPcmRings, file_location(&path), 3);
    let Err(error) = outcome else {
        panic!("a truncated mp4 must be a loud refusal, never an accepted session");
    };
    assert!(
        error.to_string().contains("truncated"),
        "the refusal must carry the resident's reason: {error}"
    );
    result
        .recv_timeout(JOIN_WAIT)
        .expect("host must end after a failed open instead of stalling")
        .expect_err("a failed open must end the host with an error");
    let _ = std::fs::remove_file(&path);
}

#[test]
fn an_empty_stream_is_a_clean_zero_length_end_of_stream() {
    // A complete WAV file whose `data` chunk is empty: nothing to play, but
    // nothing missing either. That is an end of stream, not a failure.
    let mut bytes = wav_bytes(8000, 1, &[]);
    bytes.truncate(36);
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&0u32.to_le_bytes());
    let path = temp_file("empty-data.wav", &bytes);

    let (expected, failure) = direct_decode_terminal(&path);
    assert!(
        failure.is_none(),
        "an empty stream must not be a failure: {failure:?}"
    );
    assert!(expected.is_empty(), "an empty stream decodes to no frames");

    let (mut session, done) = start(file_location(&path), 7);
    let (frames, failure) = drain_pcm(&mut session, "the empty-stream transfer");
    assert!(
        failure.is_none(),
        "an empty stream must end with an end of stream, not a failure: {failure:?}"
    );
    assert_eq!(frames, 0, "an empty stream delivers no frames");
    assert_eq!(
        read_pcm(&mut session, &mut [0.0f32; 128], "the sticky EOS read").unwrap(),
        0
    );
    finish_ok(session, done);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn dropping_the_player_stops_the_host_cleanly() {
    let path = repo_fixture("sine.flac");
    let (mut session, done) = start(file_location(&path), 3);
    let mut buf = [0.0f32; 2];
    assert_eq!(
        read_pcm(&mut session, &mut buf, "the first frame").unwrap(),
        1
    );
    // Player vanish modelled as session drop: the framework closes the ring
    // and delivers Shutdown, which must end the host rather than hang it.
    drop(session);
    done.recv_timeout(JOIN_WAIT)
        .expect("host must stop after the player goes away instead of hanging")
        .unwrap();
}
