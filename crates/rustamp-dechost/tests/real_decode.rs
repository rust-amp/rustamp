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
    let client = ControlPeer::start(ar, aw, Liveness::default(), false).unwrap();
    let host = ControlPeer::start(br, bw, Liveness::default(), true).unwrap();
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
fn read_pcm(
    session: &mut DecoderSession<NamedPcmRings>,
    buf: &mut [f32],
    step: &str,
) -> Result<usize> {
    match session.read(buf, Some(STALL_WAIT)) {
        Err(MediaHostError::TimedOut) => panic!(
            "stall: {step} received no PCM within {STALL_WAIT:?} \
             (the resident stopped delivering without EOS or an error)"
        ),
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
        let client = ControlPeer::start(ar, aw, Liveness::default(), false).unwrap();
        let host = ControlPeer::start(br, bw, Liveness::default(), true).unwrap();
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

#[test]
fn truncated_wav_preserves_backend_short_eos_not_an_integrity_guarantee() {
    let full: Vec<[f32; 2]> = (0..4000)
        .map(|i| {
            let t = i as f32 / 8000.0;
            [(t * 440.0).sin() * 0.5, (t * 550.0).sin() * 0.5]
        })
        .collect();
    let mut bytes = wav_bytes(8000, 1, &full);
    bytes.truncate(bytes.len() / 2);
    let path = temp_file("truncated.wav", &bytes);
    // Reference is the same in-process registry decoder, not Winamp. Its
    // UnexpectedEof -> EOS conversion loses the distinction before the adapter.
    // Pin that exact terminal here; do not accept an arbitrary error instead.
    let expected = direct_decode(&path);
    assert!(!expected.is_empty() && expected.len() < full.len());

    let (mut session, done) = start(file_location(&path), 7);
    let mut received = Vec::new();
    let mut buf = [0.0f32; 128];
    loop {
        match read_pcm(&mut session, &mut buf, "the truncated-file transfer")
            .expect("backend short EOS")
        {
            0 => break,
            frames => received.extend_from_slice(&buf[..frames]),
        }
    }
    assert_eq!(
        received
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        expected
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        read_pcm(&mut session, &mut buf, "the sticky EOS read").unwrap(),
        0
    );
    assert_eq!(session.frames_dropped(), 0);
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
