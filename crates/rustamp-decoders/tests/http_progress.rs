//! Integration tests for the observation of an HTTP transfer (S19 §2).
//!
//! Every test runs against a local `TcpListener` on `127.0.0.1`: no test
//! reaches into the network, no test opens a window and no test starts a
//! process.
//!
//! The media is the repository fixture `tests/fixtures/sine.mp3` (1.5 s,
//! 440 Hz, 44.1 kHz stereo sine wave), served over the socket.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use rustamp_core::audio::Decoder;
use rustamp_decoders::{HttpDecoder, HttpError, HttpPhase, HttpProgress};

/// The timeout the tests use for the exchange.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// The repository fixture.
fn fixture(name: &str) -> Vec<u8> {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(path).expect("the fixture")
}

/// A local server that answers one request in steps.
///
/// The test drives the steps: the server waits for a signal before it sends
/// the head and for another one before it sends the rest of the body. That
/// is what makes the phases observable - the transfer waits for the server
/// at every step, so the phase at that moment is the phase of the step.
struct Server {
    /// The port it listens on.
    port: u16,
    /// The signal that lets the server send the head.
    send_head: Sender<()>,
    /// The signal that lets the server send the rest of the body.
    send_body: Sender<()>,
    /// The signal that says the request arrived.
    request_seen: Receiver<()>,
}

impl Server {
    /// A server that declares `body.len()` bytes and sends the body in two
    /// steps: the head with the first half, then the rest.
    fn stepping(body: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
        let port = listener.local_addr().expect("the address").port();
        let (request_tx, request_seen) = channel();
        let (head_tx, head_rx) = channel();
        let (body_tx, body_rx) = channel();
        thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("a connection");
            read_request(&mut socket);
            let _ = request_tx.send(());
            let _ = head_rx.recv();
            let half = body.len() / 2;
            let head = format!("HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
            let _ = socket.write_all(head.as_bytes());
            let _ = socket.write_all(&body[..half]);
            let _ = socket.flush();
            let _ = body_rx.recv();
            let _ = socket.write_all(&body[half..]);
            let _ = socket.flush();
        });
        Server {
            port,
            send_head: head_tx,
            send_body: body_tx,
            request_seen,
        }
    }

    /// A server that declares `declared` bytes and sends `body`: the
    /// connection dies before the declared end.
    fn cutting(body: Vec<u8>, declared: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
        let port = listener.local_addr().expect("the address").port();
        thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("a connection");
            read_request(&mut socket);
            let head = format!("HTTP/1.0 200 OK\r\nContent-Length: {declared}\r\n\r\n");
            let _ = socket.write_all(head.as_bytes());
            let _ = socket.write_all(&body);
            let _ = socket.flush();
            // The socket is dropped here: the connection closes with the body
            // half delivered.
        });
        Server {
            port,
            send_head: channel().0,
            send_body: channel().0,
            request_seen: channel().1,
        }
    }

    /// A server that answers with `response`: the raw bytes of the whole
    /// answer, head and body.
    fn answering(response: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
        let port = listener.local_addr().expect("the address").port();
        thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("a connection");
            read_request(&mut socket);
            let _ = socket.write_all(&response);
            let _ = socket.flush();
        });
        Server {
            port,
            send_head: channel().0,
            send_body: channel().0,
            request_seen: channel().1,
        }
    }

    /// The URL of a path on this server.
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

/// Reads the request head of a connection and drops it.
fn read_request(socket: &mut TcpStream) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let Ok(n) = socket.read(&mut chunk) else {
            return;
        };
        if n == 0 {
            return;
        }
        buffer.extend_from_slice(&chunk[..n]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
            return;
        }
    }
}

/// Reads a whole stream and counts its frames.
fn read_frames(decoder: &mut dyn Decoder) -> u64 {
    let channels = usize::from(decoder.spec().channels.max(1));
    let mut buffer = vec![0.0f32; 1024 * channels];
    let mut frames = 0u64;
    loop {
        let n = decoder.read(&mut buffer).expect("the stream reads");
        if n == 0 {
            break;
        }
        frames += n as u64;
    }
    frames
}

/// The phases of a transfer run in the order of the spec, and the counters
/// follow the bytes.
#[test]
fn the_phases_run_from_connecting_over_the_response_to_the_body() {
    let body = fixture("sine.mp3");
    let server = Server::stepping(body.clone());
    let progress = HttpProgress::new();

    // Before the transfer starts: the host is being reached.
    assert_eq!(progress.snapshot().phase, HttpPhase::Connecting);

    // The open runs on its own thread, so the test can read the snapshot
    // while the transfer waits for the server.
    let url = server.url("/sine.mp3");
    let observer = progress.clone();
    let opened =
        thread::spawn(move || HttpDecoder::open_with_progress(&url, TEST_TIMEOUT, &observer));

    // The server holds the answer: the transfer is reading the response.
    server
        .request_seen
        .recv_timeout(TEST_TIMEOUT)
        .expect("the request arrived");
    thread::sleep(Duration::from_millis(50));
    assert_eq!(progress.snapshot().phase, HttpPhase::ReadingResponse);

    // The head and the first half of the body arrive: the body is read.
    server.send_head.send(()).expect("the head is sent");
    let mut decoder = opened
        .join()
        .expect("the open thread")
        .expect("the stream opens");
    assert_eq!(progress.snapshot().phase, HttpPhase::Retrieving);
    assert_eq!(
        progress.snapshot().total,
        Some(body.len() as u64),
        "the total is the length the server declared"
    );
    assert!(
        progress.snapshot().bytes_read > 0,
        "the probe read the head of the body"
    );

    // The rest of the body arrives: the counters follow the bytes.
    server.send_body.send(()).expect("the body is sent");
    read_frames(&mut decoder);
    assert_eq!(
        progress.snapshot().bytes_read,
        body.len() as u64,
        "every byte of the body was counted"
    );
    assert_eq!(
        progress.snapshot().phase,
        HttpPhase::Closed,
        "the server closed the connection at the end of the body"
    );
}

/// A response that declares no length has no total: a live stream has no
/// total, and that is what the two transfer texts of S19 §2 differ in.
#[test]
fn a_response_without_a_length_has_no_total() {
    let body = fixture("sine.mp3");
    let mut response = b"HTTP/1.0 200 OK\r\n\r\n".to_vec();
    response.extend_from_slice(&body);
    let server = Server::answering(response);
    let progress = HttpProgress::new();

    let mut decoder =
        HttpDecoder::open_with_progress(&server.url("/live"), TEST_TIMEOUT, &progress)
            .expect("the stream opens");
    assert_eq!(progress.snapshot().total, None, "there is no total");
    assert_eq!(progress.snapshot().phase, HttpPhase::Retrieving);
    read_frames(&mut decoder);
    assert!(
        progress.snapshot().bytes_read > 0,
        "the bytes are counted all the same"
    );
}

/// A connection that dies before the declared end is a lost connection, and
/// the phase says so.
#[test]
fn a_connection_that_dies_before_the_declared_end_is_lost() {
    let body = fixture("sine.mp3");
    let declared = body.len();
    let half = body.len() / 2;
    let server = Server::cutting(body[..half].to_vec(), declared);
    let progress = HttpProgress::new();

    let mut decoder =
        HttpDecoder::open_with_progress(&server.url("/sine.mp3"), TEST_TIMEOUT, &progress)
            .expect("the stream opens");
    assert_eq!(progress.snapshot().phase, HttpPhase::Retrieving);

    let channels = usize::from(decoder.spec().channels.max(1));
    let mut buffer = vec![0.0f32; 1024 * channels];
    let err = loop {
        match decoder.read(&mut buffer) {
            Ok(0) => panic!("the cut stream ended cleanly"),
            Ok(_) => {}
            Err(e) => break e,
        }
    };
    assert!(
        err.to_string().contains("truncated stream"),
        "the failure names the cut: {err}"
    );
    assert_eq!(
        progress.snapshot().phase,
        HttpPhase::ConnectionLost,
        "the host vanished before the body was complete"
    );
    assert!(
        progress.snapshot().bytes_read < declared as u64,
        "not every byte arrived"
    );
}

/// A host that cannot be reached reports the connect phase, and the failure
/// names the host.
#[test]
fn a_host_that_cannot_be_reached_reports_the_connect_phase() {
    // Port 1 on the loopback interface: nothing listens there.
    let progress = HttpProgress::new();
    let err = HttpDecoder::open_with_progress("http://127.0.0.1:1/a.mp3", TEST_TIMEOUT, &progress)
        .expect_err("the stream does not open");
    assert!(
        matches!(err, HttpError::Connect { .. }),
        "unexpected {err:?}"
    );
    assert_eq!(
        progress.snapshot().phase,
        HttpPhase::ConnectFailed,
        "the host could not be reached"
    );
    assert_eq!(progress.snapshot().bytes_read, 0, "no byte came in");
    assert_eq!(progress.snapshot().total, None, "there is no total");
}

/// The observer derives the rate from two samples of the byte count and its
/// own clock.
#[test]
fn the_rate_is_derived_from_two_samples() {
    let body = fixture("sine.mp3");
    let server = Server::stepping(body.clone());
    let progress = HttpProgress::new();

    let url = server.url("/sine.mp3");
    let observer = progress.clone();
    let opened =
        thread::spawn(move || HttpDecoder::open_with_progress(&url, TEST_TIMEOUT, &observer));
    server
        .request_seen
        .recv_timeout(TEST_TIMEOUT)
        .expect("the request arrived");
    server.send_head.send(()).expect("the head is sent");
    let mut decoder = opened
        .join()
        .expect("the open thread")
        .expect("the stream opens");

    // The first sample: the half of the body that arrived.
    let first = progress.snapshot();
    let started = Instant::now();
    server.send_body.send(()).expect("the body is sent");
    read_frames(&mut decoder);
    let second = progress.snapshot();
    let elapsed = started.elapsed();

    assert!(
        second.bytes_read > first.bytes_read,
        "the byte count advanced: {} then {}",
        first.bytes_read,
        second.bytes_read
    );
    let rate = (second.bytes_read - first.bytes_read) as f64 / elapsed.as_secs_f64();
    assert!(
        rate > 0.0,
        "the rate the observer derives is {rate} bytes/s"
    );
}

/// The order of the phases, as S19 §2 lists them.
const PHASE_ORDER: [HttpPhase; 7] = [
    HttpPhase::Connecting,
    HttpPhase::SendingRequest,
    HttpPhase::ReadingResponse,
    HttpPhase::Retrieving,
    HttpPhase::ConnectionLost,
    HttpPhase::Closed,
    HttpPhase::ConnectFailed,
];

/// The phases of a transfer run in the order of the spec.
///
/// A poller reads the phase while the transfer runs. The phases it sees
/// must not go backwards: the transfer walks the list of the spec from the
/// first phase to the last one that happens.
#[test]
fn the_phases_run_in_the_order_of_the_spec() {
    let body = fixture("sine.mp3");
    let server = Server::stepping(body.clone());
    let progress = HttpProgress::new();

    let url = server.url("/sine.mp3");
    let observer = progress.clone();
    let opened =
        thread::spawn(move || HttpDecoder::open_with_progress(&url, TEST_TIMEOUT, &observer));

    // The poller samples the phase until the transfer is over.
    let poller = {
        let progress = progress.clone();
        thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            let mut seen: Vec<HttpPhase> = Vec::new();
            while Instant::now() < deadline {
                let phase = progress.snapshot().phase;
                if seen.last() != Some(&phase) {
                    seen.push(phase);
                }
                if matches!(
                    phase,
                    HttpPhase::Closed | HttpPhase::ConnectionLost | HttpPhase::ConnectFailed
                ) {
                    break;
                }
                thread::sleep(Duration::from_millis(1));
            }
            seen
        })
    };

    server
        .request_seen
        .recv_timeout(TEST_TIMEOUT)
        .expect("the request arrived");
    server.send_head.send(()).expect("the head is sent");
    let mut decoder = opened
        .join()
        .expect("the open thread")
        .expect("the stream opens");
    // The body waits, so the phase of the transfer is long enough to be
    // seen.
    thread::sleep(Duration::from_millis(50));
    server.send_body.send(()).expect("the body is sent");
    read_frames(&mut decoder);

    let seen = poller.join().expect("the poller");
    let mut last = 0;
    for phase in &seen {
        let index = PHASE_ORDER
            .iter()
            .position(|known| known == phase)
            .expect("a known phase");
        assert!(
            index >= last,
            "the phase went backwards: {seen:?} (the order of the spec is {PHASE_ORDER:?})"
        );
        last = index;
    }
    assert!(
        seen.contains(&HttpPhase::Retrieving),
        "the transfer was seen while it read the body: {seen:?}"
    );
    assert_eq!(
        seen.last(),
        Some(&HttpPhase::Closed),
        "the transfer ended in the closed phase: {seen:?}"
    );
}
