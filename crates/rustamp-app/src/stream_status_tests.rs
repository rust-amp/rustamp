// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! What the app shows about the stream that is playing (S19 §2).
//!
//! The texts are the app's (S19 §2 names them), the facts are the engine's:
//! the engine publishes what the transfer is doing, and the app turns a phase
//! change into a note and the counters into a transient status.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rustamp_decoders::{HttpPhase, HttpProgressSnapshot};

use super::*;
use rustamp_core::playlist::MediaLocation;

/// The status texts of S19 §2, as the app spells them.
///
/// The transfer display has two variants, so its head is matched by its
/// beginning: `Retrieving file...` and `Retrieving file (42% done)` are the
/// same head with and without the share.
const PHASE_TEXTS: [&str; 7] = [
    "Connecting to host...",
    "Sending HTTP request",
    "Reading HTTP response",
    "Retrieving file",
    "Connection to host lost",
    "\nConnection closed.",
    "Error connecting to host",
];

/// Whether the note is one of the status texts.
fn is_phase_text(note: &str) -> bool {
    PHASE_TEXTS.iter().any(|text| note.starts_with(text))
}

/// A snapshot of a transfer in the given phase.
fn snapshot(phase: HttpPhase, bytes_read: u64, total: Option<u64>) -> HttpProgressSnapshot {
    HttpProgressSnapshot {
        phase,
        bytes_read,
        total,
    }
}

/// A player with the shipped configuration and a null device.
fn player(media: Option<MediaLocation>) -> App {
    App::with_registry(
        AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            media,
            seconds: None,
        },
        rustamp_decoders::default_registry(),
    )
    .expect("a player")
}

/// A tiny WAV file: a header and a tenth of a second of silence.
fn wav(samples: usize) -> Vec<u8> {
    let data_len = (samples * 2) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&44100u32.to_le_bytes());
    out.extend_from_slice(&88200u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.resize(out.len() + data_len as usize, 0);
    out
}

/// A server that answers one stream request after a pause.
///
/// The pause keeps the phase of the exchange readable from the outside: the
/// client waits for the head of the answer, and the phase says that it is
/// reading the response.
fn stream_server(hold: Duration, body: Vec<u8>) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
    let port = listener.local_addr().expect("the address").port();
    let handle = thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("a connection");
        let mut request = [0u8; 1024];
        let _ = socket.read(&mut request);
        thread::sleep(hold);
        let head = format!("HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
        let _ = socket.write_all(head.as_bytes());
        let _ = socket.write_all(&body);
        let _ = socket.flush();
    });
    (format!("http://127.0.0.1:{port}/stream.wav"), handle)
}

/// A port that nothing listens on: the connect is refused at once.
fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
    let port = listener.local_addr().expect("the address").port();
    drop(listener);
    port
}

/// Pumps the player until `ready` is happy, and returns the notes the user
/// read in the meantime.
///
/// The null device is driven along: without it the device never pulls, and
/// the run never ends.
fn pump_until(app: &mut App, ready: impl Fn(&[String]) -> bool) -> Vec<String> {
    let mut notes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        app.drive_null_clock();
        app.pump().expect("the pump");
        notes.extend(app.drain_notes());
        if ready(&notes) {
            return notes;
        }
        thread::sleep(Duration::from_millis(2));
    }
    notes
}

/// A phase change is a note, and it is one note per change.
#[test]
fn a_phase_change_makes_one_note_with_the_spec_text() {
    let mut app = player(None);
    let now = Instant::now();

    app.apply_stream_snapshot(snapshot(HttpPhase::Connecting, 0, None), now);
    assert_eq!(
        app.drain_notes(),
        vec!["Connecting to host...".to_string()],
        "the text of the phase is the spec's own"
    );

    // The same phase again: nothing new to say.
    app.apply_stream_snapshot(snapshot(HttpPhase::Connecting, 0, None), now);
    assert!(app.drain_notes().is_empty(), "the phase is reported once");

    // The next phase is a new text and a new note.
    app.apply_stream_snapshot(snapshot(HttpPhase::SendingRequest, 0, None), now);
    assert_eq!(
        app.drain_notes(),
        vec!["Sending HTTP request".to_string()],
        "the next phase is reported"
    );

    app.apply_stream_snapshot(snapshot(HttpPhase::ReadingResponse, 0, None), now);
    assert_eq!(
        app.drain_notes(),
        vec!["Reading HTTP response".to_string()],
        "the phase that reads the answer is reported"
    );
}

/// The counters are a transient status, not a note per sample.
#[test]
fn the_counter_line_updates_without_spamming() {
    let mut app = player(None);
    let start = Instant::now();

    app.apply_stream_snapshot(snapshot(HttpPhase::Retrieving, 0, None), start);
    assert_eq!(
        app.drain_notes(),
        vec!["Retrieving file...".to_string()],
        "the phase is a note"
    );

    // The body arrives: the counters change with every sample.
    for bytes in [1024u64, 2048, 4096, 8192] {
        app.apply_stream_snapshot(
            snapshot(HttpPhase::Retrieving, bytes, None),
            start + Duration::from_millis(10),
        );
        assert!(app.drain_notes().is_empty(), "the counters are not notes");
    }

    let status = app.stream_status().expect("the status line").to_string();
    assert!(
        status.starts_with("Retrieving file...\n"),
        "the status is the transfer display: {status}"
    );
    assert!(
        status.contains("8192 bytes read @"),
        "the status carries the byte count: {status}"
    );
    assert!(
        status.contains("KB/sec"),
        "the status carries the rate: {status}"
    );
}

/// The share is there only when the server declared a length.
#[test]
fn the_percentage_appears_only_when_the_total_is_known() {
    let start = Instant::now();

    // No total: the plain text and no share.
    let mut app = player(None);
    app.apply_stream_snapshot(snapshot(HttpPhase::Retrieving, 500, None), start);
    app.apply_stream_snapshot(
        snapshot(HttpPhase::Retrieving, 1000, None),
        start + Duration::from_millis(10),
    );
    let status = app.stream_status().expect("the status line").to_string();
    assert!(
        status.starts_with("Retrieving file...\n"),
        "the plain text: {status}"
    );
    assert!(
        !status.contains("done"),
        "no share without a total: {status}"
    );

    // A total: the variant with the share, in the note and in the status.
    let mut app = player(None);
    app.apply_stream_snapshot(snapshot(HttpPhase::Retrieving, 500, Some(1000)), start);
    assert_eq!(
        app.drain_notes(),
        vec!["Retrieving file (50% done)".to_string()],
        "the note names the share"
    );
    app.apply_stream_snapshot(
        snapshot(HttpPhase::Retrieving, 1000, Some(1000)),
        start + Duration::from_millis(10),
    );
    let status = app.stream_status().expect("the status line").to_string();
    assert!(
        status.starts_with("Retrieving file (100% done)\n"),
        "the status names the share: {status}"
    );
    assert!(
        status.contains("1000/1000 bytes read @"),
        "the status names the ratio: {status}"
    );
}

/// The observer survives a failed open: the phase that says why reaches the
/// user.
#[test]
fn a_host_that_cannot_be_reached_is_reported() {
    let url = format!("http://127.0.0.1:{}/stream.wav", closed_port());
    let mut app = player(Some(MediaLocation::Url(url)));
    app.submit(Command::Play).expect("the play");

    let notes = pump_until(&mut app, |notes| {
        notes.iter().any(|note| note == "Error connecting to host")
    });
    assert!(
        notes.iter().any(|note| note == "Error connecting to host"),
        "the phase of the failed connect reaches the user: {notes:?}"
    );
}

/// The phases of a running stream reach the user, and each of them once.
#[test]
fn the_phases_of_a_running_stream_reach_the_user() {
    let (url, server) = stream_server(Duration::from_millis(300), wav(4410));
    let mut app = player(Some(MediaLocation::Url(url)));
    app.submit(Command::Play).expect("the play");

    let mut notes = pump_until(&mut app, |notes| {
        notes.iter().any(|note| is_phase_text(note))
    });
    // The rest of the transfer: the phases keep coming, and each of them
    // comes once.
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        app.drive_null_clock();
        app.pump().expect("the pump");
        notes.extend(app.drain_notes());
        if app.engine().transport() == TransportState::Stopped {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
    let _ = server.join();

    let phases: Vec<&String> = notes.iter().filter(|note| is_phase_text(note)).collect();
    assert!(
        !phases.is_empty(),
        "a phase of the transfer reaches the user: {notes:?}"
    );
    // The same phase is never reported twice.
    for phase in &phases {
        assert_eq!(
            phases.iter().filter(|other| *other == phase).count(),
            1,
            "the phase is reported once: {notes:?}"
        );
    }
}

/// A file run shows no stream phase at all.
#[test]
fn a_file_run_never_shows_a_stream_phase() {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let path = dir.path().join("track.wav");
    std::fs::write(&path, wav(44100)).expect("the file");
    let mut app = player(Some(MediaLocation::File(path)));
    app.submit(Command::Play).expect("the play");

    for _ in 0..50 {
        app.pump().expect("the pump");
    }
    let notes = app.drain_notes();
    assert!(
        !notes.iter().any(|note| is_phase_text(note)),
        "a file has no stream phase: {notes:?}"
    );
    assert_eq!(app.stream_status(), None, "a file has no stream status");
}
