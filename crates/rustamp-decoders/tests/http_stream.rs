//! Integration tests for the HTTP source (S19).
//!
//! Every test runs against a local `TcpListener` on `127.0.0.1`: no test
//! reaches into the network, and no test opens a window or starts a process.
//!
//! The media is the repository fixture `tests/fixtures/sine.mp3` (1.5 s,
//! 440 Hz, 44.1 kHz stereo sine wave), served over the socket.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rustamp_core::audio::{Decoder, MediaSource};
use rustamp_decoders::{HttpDecoder, HttpError, SymphoniaDecoder};

/// The timeout the tests use for the exchange.
///
/// The default of the source is ten seconds; a test that waits for it would
/// be slow, so the tests that need a failure in time ask for a short one.
const TEST_TIMEOUT: Duration = Duration::from_secs(5);

/// The repository fixture.
fn fixture(name: &str) -> Vec<u8> {
    let path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(path).expect("the fixture")
}

/// The path of the repository fixture.
fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// A local server that answers one request.
struct Server {
    /// The port it listens on.
    port: u16,
    /// The request it received.
    request: Arc<Mutex<Vec<u8>>>,
}

impl Server {
    /// A server that answers with `response`: the raw bytes of the whole
    /// answer, head and body.
    fn answering(response: Vec<u8>) -> Self {
        Self::serving(move |socket| {
            let _ = socket.write_all(&response);
            let _ = socket.flush();
        })
    }

    /// A server that accepts the connection and never answers.
    fn silent() -> Self {
        Self::serving(|socket| {
            // Read until the client gives up: the read ends when the client
            // drops the connection.
            let mut sink = [0u8; 1024];
            while matches!(socket.read(&mut sink), Ok(n) if n > 0) {}
        })
    }

    /// A server that runs `respond` for the accepted connection.
    fn serving<F>(respond: F) -> Self
    where
        F: FnOnce(&mut TcpStream) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
        let port = listener.local_addr().expect("the address").port();
        let request = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&request);
        thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("a connection");
            read_request(&mut socket, &recorded);
            respond(&mut socket);
        });
        Server { port, request }
    }

    /// The URL of a path on this server.
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    /// The request the server received.
    fn request(&self) -> Vec<u8> {
        self.request.lock().expect("the lock").clone()
    }
}

/// Reads the request head of a connection and records it.
fn read_request(socket: &mut TcpStream, recorded: &Arc<Mutex<Vec<u8>>>) {
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
            *recorded.lock().expect("the lock") = buffer;
            return;
        }
    }
}

/// A 200 answer with `body` as its body.
fn ok(body: &[u8]) -> Vec<u8> {
    let mut response =
        format!("HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    response.extend_from_slice(body);
    response
}

/// A 404 answer.
fn not_found() -> Vec<u8> {
    b"HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()
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

/// The request is the one the template of S19 §1 describes.
#[test]
fn the_request_is_the_one_the_template_describes() {
    let body = fixture("sine.mp3");
    let server = Server::answering(ok(&body));
    let url = server.url("/music/sine.mp3");
    let mut decoder = HttpDecoder::open_with_timeout(&url, TEST_TIMEOUT).expect("the stream opens");
    read_frames(&mut decoder);

    let expected = format!(
        "GET {url} HTTP/1.0\r\n\
         Host: 127.0.0.1:{}\r\n\
         User-Agent: Rustamp/{}\r\n\
         Accept: */*\r\n\
         \r\n",
        server.port,
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(String::from_utf8_lossy(&server.request()), expected);
}

/// A stream decodes to the frames the file holds.
#[test]
fn a_stream_decodes_to_the_frames_the_file_holds() {
    let body = fixture("sine.mp3");
    let server = Server::answering(ok(&body));
    let mut stream = HttpDecoder::open_with_timeout(&server.url("/sine.mp3"), TEST_TIMEOUT)
        .expect("the stream opens");
    assert_eq!(stream.spec().sample_rate, 44_100);
    assert_eq!(stream.spec().channels, 2);
    let streamed = read_frames(&mut stream);

    let mut file = SymphoniaDecoder::open(MediaSource::Path(fixture_path("sine.mp3")))
        .expect("the file opens");
    let from_file = read_frames(&mut file);

    assert_eq!(streamed, from_file, "the stream holds what the file holds");
    assert!(streamed > 60_000, "only {streamed} frames");
}

/// A URL without an extension is sniffed by its content.
#[test]
fn a_url_without_an_extension_is_sniffed_by_its_content() {
    let body = fixture("sine.mp3");
    let server = Server::answering(ok(&body));
    let mut stream = HttpDecoder::open_with_timeout(&server.url("/live"), TEST_TIMEOUT)
        .expect("the stream opens");
    assert!(read_frames(&mut stream) > 60_000);
}

/// A stream cannot be seeked.
#[test]
fn a_stream_cannot_be_seeked() {
    let body = fixture("sine.mp3");
    let server = Server::answering(ok(&body));
    let mut stream = HttpDecoder::open_with_timeout(&server.url("/sine.mp3"), TEST_TIMEOUT)
        .expect("the stream opens");
    assert!(!stream.is_seekable());
    let err = stream.seek(Duration::from_secs(1)).expect_err("no seek");
    assert!(
        err.to_string().contains("seeking"),
        "the failure names the refusal: {err}"
    );
}

/// A status other than 200 is a typed failure that names the status.
#[test]
fn a_status_other_than_200_is_a_typed_failure_that_names_the_status() {
    let server = Server::answering(not_found());
    let err = HttpDecoder::open_with_timeout(&server.url("/missing.mp3"), TEST_TIMEOUT)
        .expect_err("the stream does not open");
    match &err {
        HttpError::Status { code, reason } => {
            assert_eq!(*code, 404);
            assert_eq!(reason, "Not Found");
        }
        other => panic!("unexpected {other:?}"),
    }
    let message = err.to_string();
    assert!(
        message.contains("404"),
        "the message names the status: {message}"
    );
}

/// A connection that dies in the middle of the body is a truncated stream,
/// not a clean end.
#[test]
fn a_connection_dropped_mid_body_is_a_loud_failure() {
    let body = fixture("sine.mp3");
    let declared = body.len();
    let half = body.len() / 2;
    let server = Server::serving(move |socket| {
        let head = format!("HTTP/1.0 200 OK\r\nContent-Length: {declared}\r\n\r\n");
        let _ = socket.write_all(head.as_bytes());
        let _ = socket.write_all(&body[..half]);
        let _ = socket.flush();
        // The socket is dropped here: the connection closes with the body
        // half delivered.
    });
    let mut stream = HttpDecoder::open_with_timeout(&server.url("/sine.mp3"), TEST_TIMEOUT)
        .expect("the stream opens");
    let channels = usize::from(stream.spec().channels.max(1));
    let mut buffer = vec![0.0f32; 1024 * channels];
    let err = loop {
        match stream.read(&mut buffer) {
            Ok(0) => panic!("the cut stream ended cleanly"),
            Ok(_) => {}
            Err(e) => break e,
        }
    };
    let message = err.to_string();
    assert!(
        message.contains("truncated stream"),
        "the failure names the cut: {message}"
    );
    assert!(
        message.contains(&format!("of the {declared} bytes")),
        "the failure names the length the server declared: {message}"
    );
}

/// `https` is refused by name: there is no TLS in this tree.
#[test]
fn https_is_refused_by_name() {
    let err = HttpDecoder::open("https://example.invalid/a.mp3").expect_err("https is not read");
    match err {
        HttpError::UnsupportedScheme { ref scheme } => assert_eq!(scheme, "https"),
        other => panic!("unexpected {other:?}"),
    }
    let message = HttpDecoder::open("https://example.invalid/a.mp3")
        .expect_err("https is not read")
        .to_string();
    assert!(
        message.contains("TLS"),
        "the message names the reason: {message}"
    );
}

/// A host that accepts the connection and never answers runs into the
/// timeout.
#[test]
fn the_timeout_fires_on_a_server_that_never_answers() {
    let server = Server::silent();
    let started = std::time::Instant::now();
    let err =
        HttpDecoder::open_with_timeout(&server.url("/stream.mp3"), Duration::from_millis(300))
            .expect_err("the stream does not open");
    match err {
        HttpError::Timeout { phase } => assert!(!phase.is_empty()),
        other => panic!("unexpected {other:?}"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the timeout fired in time"
    );
}

/// A host that cannot be reached is a typed failure, not a hang.
#[test]
fn a_host_that_cannot_be_reached_is_a_typed_failure() {
    // Port 1 on the loopback interface: nothing listens there.
    let err = HttpDecoder::open_with_timeout("http://127.0.0.1:1/a.mp3", TEST_TIMEOUT)
        .expect_err("the stream does not open");
    assert!(
        matches!(err, HttpError::Connect { .. } | HttpError::Timeout { .. }),
        "unexpected {err:?}"
    );
}

/// A body that is not media is refused when the decoder is opened.
#[test]
fn a_body_that_is_not_media_is_refused_by_name() {
    let server = Server::answering(ok(b"this is not audio at all"));
    let err = HttpDecoder::open_with_timeout(&server.url("/sine.mp3"), TEST_TIMEOUT)
        .expect_err("the stream does not open");
    match &err {
        HttpError::Media(_) => {}
        other => panic!("unexpected {other:?}"),
    }
    assert!(
        err.to_string().contains("unsupported format"),
        "the failure names the problem: {err}"
    );
}

/// The failure of the transport travels through the core error.
#[test]
fn a_failure_travels_through_the_core_error() {
    let server = Server::answering(not_found());
    let err: rustamp_core::error::Error =
        HttpDecoder::open_with_timeout(&server.url("/missing.mp3"), TEST_TIMEOUT)
            .expect_err("the stream does not open")
            .into();
    assert!(err.to_string().contains("404"), "{err}");
}
