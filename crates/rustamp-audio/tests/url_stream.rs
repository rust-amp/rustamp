//! Integration tests for a URL track: the engine plays an `http://` stream.
//!
//! Every test runs against a local `TcpListener` on `127.0.0.1`: no test
//! reaches into the network, no test opens a window, no test starts a
//! process.
//!
//! The media is the repository fixture `sine.mp3` (1.5 s, 440 Hz, 44.1 kHz
//! stereo sine wave) and `sine.ogg`, served over the socket; see
//! `crates/rustamp-decoders/tests/fixtures/FIXTURES.txt` for how they were
//! made.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use rustamp_audio::{Engine, EngineConfig, TrackSource};
use rustamp_core::audio::AudioSpec;
use rustamp_core::bus::{EventBus, EventReceiver};
use rustamp_core::event::Event;
use rustamp_core::state::TransportState;
use rustamp_output::{NullClock, NullOutput};

/// The format of the fixtures: 44.1 kHz stereo.
const SPEC: AudioSpec = AudioSpec::new(44_100, 2);

/// The frames the `sine.mp3` fixture holds: 1.5 s at 44.1 kHz.
const MP3_FRAMES: u64 = 66_150;

/// The frames the `sine.ogg` fixture holds: 1.5 s at 44.1 kHz, plus the
/// samples the Vorbis encoder left in the stream.
const OGG_FRAMES: u64 = 67_136;

/// Stall escape for every wait: a bound so generous that only a real stall
/// reaches it (the rule the other engine tests use).
const WAIT: Duration = Duration::from_secs(30);

/// The bytes of a repository fixture.
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name)).expect("the fixture")
}

/// The path of a repository fixture.
fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../rustamp-decoders/tests/fixtures")
        .join(name)
}

/// A local server that answers one request.
struct Server {
    /// The port it listens on.
    port: u16,
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

    /// A server that declares `declared` bytes and sends `body`: the
    /// connection dies before the declared end.
    fn cutting(body: &[u8], declared: usize) -> Self {
        let body = body.to_vec();
        Self::serving(move |socket| {
            let head = format!("HTTP/1.0 200 OK\r\nContent-Length: {declared}\r\n\r\n");
            let _ = socket.write_all(head.as_bytes());
            let _ = socket.write_all(&body);
            let _ = socket.flush();
            // The socket is dropped here: the connection closes with the body
            // half delivered.
        })
    }

    /// A server that runs `respond` for the accepted connection.
    fn serving<F>(respond: F) -> Self
    where
        F: FnOnce(&mut std::net::TcpStream) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
        let port = listener.local_addr().expect("the address").port();
        thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("a connection");
            read_request(&mut socket);
            respond(&mut socket);
        });
        Server { port }
    }

    /// The URL of a path on this server.
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
}

/// Reads the request head of a connection and drops it.
fn read_request(socket: &mut std::net::TcpStream) {
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

/// An engine on a virtual device, with the queue the test wants.
struct Stream {
    engine: Engine,
    clock: NullClock,
    events: EventReceiver,
    /// The events published so far.
    seen: Vec<Event>,
}

impl Stream {
    /// An engine with `queue` as its play queue.
    fn with_queue(queue: Vec<TrackSource>) -> Self {
        let (output, clock) = NullOutput::pair();
        let bus = EventBus::new();
        let events = bus.subscribe();
        let mut engine = Engine::new(
            rustamp_decoders::default_registry(),
            Box::new(output),
            bus,
            EngineConfig::default(),
        )
        .expect("engine construction");
        engine.set_queue(queue);
        Stream {
            engine,
            clock,
            events,
            seen: Vec::new(),
        }
    }

    /// An engine that plays the URL.
    fn url(url: &str) -> Self {
        Self::with_queue(vec![TrackSource::Url(url.to_string())])
    }

    /// Poll and pump until `cond` holds or the wait runs out.
    ///
    /// The pump advances the virtual device, so the ring drains and the
    /// decode thread can go on; the events are collected on the way.
    fn wait(&mut self, mut cond: impl FnMut(&Engine, &[Event]) -> bool) -> bool {
        let deadline = Instant::now() + WAIT;
        loop {
            self.engine.poll().expect("poll");
            self.seen.extend(self.events.try_iter());
            if cond(&self.engine, &self.seen) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            self.clock.pump(256);
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// The messages of the errors published so far.
    fn error_messages(&self) -> Vec<String> {
        self.seen
            .iter()
            .filter_map(|event| match event {
                Event::Error(info) => Some(info.message.clone()),
                _ => None,
            })
            .collect()
    }
}

/// A URL track plays the body of the stream.
#[test]
fn a_url_track_plays_the_body_of_the_stream() {
    let body = fixture("sine.mp3");
    let server = Server::answering(ok(&body));
    let mut h = Stream::url(&server.url("/sine.mp3"));
    h.engine.play_index(0).expect("play");

    assert!(
        h.wait(|engine, _| engine.source_spec().is_some()),
        "the stream must start"
    );
    assert_eq!(h.engine.source_spec(), Some(SPEC));
    // The stream cannot be seeked, and the engine says so.
    assert!(!h.engine.is_seekable());
    // The container of the fixture states its length, so the length is shown.
    assert_eq!(h.engine.duration_frames(), Some(MP3_FRAMES));
    assert_eq!(h.engine.state().duration_ms, Some(1_500));
    assert!(
        h.wait(|engine, _| engine.transport() == TransportState::Stopped),
        "the track must end"
    );
    assert_eq!(
        h.engine.last_track_frames(),
        MP3_FRAMES,
        "the stream must hold what the file holds"
    );
    assert!(
        h.error_messages().is_empty(),
        "a clean stream reports no error: {:?}",
        h.error_messages()
    );
}

/// The URL plays what the file plays, and the file still goes through the
/// registry.
#[test]
fn a_url_track_plays_what_the_file_plays() {
    // The file: the registry opens it, exactly as before.
    let mut file = Stream::with_queue(vec![TrackSource::path(fixture_path("sine.mp3"))]);
    file.engine.play_index(0).expect("play");
    assert!(
        file.wait(|engine, _| engine.transport() == TransportState::Stopped),
        "the file must end"
    );
    let from_file = file.engine.last_track_frames();
    assert!(from_file > 60_000, "only {from_file} frames");

    // The same bytes, over the socket.
    let body = fixture("sine.mp3");
    let server = Server::answering(ok(&body));
    let mut stream = Stream::url(&server.url("/sine.mp3"));
    stream.engine.play_index(0).expect("play");
    assert!(
        stream.wait(|engine, _| engine.transport() == TransportState::Stopped),
        "the stream must end"
    );
    assert_eq!(
        stream.engine.last_track_frames(),
        from_file,
        "the stream must hold what the file holds"
    );
}

/// A connection that dies in the middle of the body ends the track loudly.
#[test]
fn a_cut_stream_ends_the_track_loudly() {
    let body = fixture("sine.mp3");
    let declared = body.len();
    let half = body.len() / 2;
    let server = Server::cutting(&body[..half], declared);
    let mut h = Stream::url(&server.url("/sine.mp3"));
    h.engine.play_index(0).expect("play");

    assert!(
        h.wait(|_, events| events.iter().any(|event| matches!(
            event,
            Event::Error(info) if info.message.contains("truncated stream")
        ))),
        "the cut must be reported: {:?}",
        h.error_messages()
    );
    let message = h.error_messages().join("; ");
    assert!(
        message.contains(&format!("of the {declared} bytes")),
        "the failure names the length the server declared: {message}"
    );
    assert!(
        h.wait(|engine, _| engine.transport() == TransportState::Stopped),
        "the run must end"
    );
}

/// A seek on a stream is refused, reported, and moves nothing.
#[test]
fn a_seek_on_a_stream_is_refused_and_reported() {
    let body = fixture("sine.mp3");
    let server = Server::answering(ok(&body));
    let mut h = Stream::url(&server.url("/sine.mp3"));
    h.engine.play_index(0).expect("play");
    assert!(
        h.wait(|engine, _| engine.source_spec().is_some()),
        "the stream must start"
    );

    // A target far beyond the position: a seek that was honoured would move
    // the position there.
    h.engine.seek_frames(60_000).expect("seek");
    assert!(
        h.wait(|_, events| events.iter().any(|event| matches!(
            event,
            Event::Error(info) if info.message.contains("seeking in an HTTP stream")
        ))),
        "the refusal must be reported: {:?}",
        h.error_messages()
    );
    assert!(
        h.engine.position_frames() < 60_000,
        "the position must not jump to the target: {}",
        h.engine.position_frames()
    );

    // The refusal does not end the track: the stream plays on.
    assert!(
        h.wait(|engine, _| engine.transport() == TransportState::Stopped),
        "the track must end"
    );
    assert_eq!(
        h.engine.last_track_frames(),
        MP3_FRAMES,
        "the stream must play to its end"
    );
}

/// A stream whose container states no length has no length to show.
#[test]
fn the_length_of_a_stream_without_one_is_unknown() {
    // The Ogg container states the end of its stream with a flag, not with a
    // count, so the length of the stream is unknown while it plays.
    let body = fixture("sine.ogg");
    let server = Server::answering(ok(&body));
    let mut h = Stream::url(&server.url("/sine.ogg"));
    h.engine.play_index(0).expect("play");

    assert!(
        h.wait(|engine, _| engine.source_spec().is_some()),
        "the stream must start"
    );
    assert_eq!(
        h.engine.duration_frames(),
        None,
        "the length of the track must be unknown"
    );
    assert_eq!(
        h.engine.state().duration_ms,
        None,
        "the window must show no total time"
    );
    assert!(
        h.wait(|engine, _| engine.transport() == TransportState::Stopped),
        "the track must end"
    );
    // The position counted the whole stream: an unknown length does not
    // clamp the position to a wrong number.
    assert_eq!(
        h.engine.position_frames(),
        OGG_FRAMES,
        "the position must count the frames the stream held"
    );
}

/// A seek on a stream that is not playing yet is undone when the run starts.
///
/// The engine remembers the target of a seek on a stopped transport and
/// applies it when playback starts. A stream cannot be seeked, so the
/// decoder refuses the target: the engine publishes the refusal and puts the
/// position back where the stream really is.
#[test]
fn a_seek_on_a_stopped_stream_is_undone_when_the_run_starts() {
    let body = fixture("sine.mp3");
    let server = Server::answering(ok(&body));
    let mut h = Stream::url(&server.url("/sine.mp3"));

    h.engine.seek_frames(60_000).expect("seek");
    assert_eq!(
        h.engine.position_frames(),
        60_000,
        "the engine remembers the target of a stopped seek"
    );

    h.engine.play().expect("play");
    assert!(
        h.wait(|_, events| events.iter().any(|event| matches!(
            event,
            Event::Error(info) if info.message.contains("seeking in an HTTP stream")
        ))),
        "the refusal must be reported: {:?}",
        h.error_messages()
    );
    assert!(
        h.engine.position_frames() < 60_000,
        "the position must not stay at the target: {}",
        h.engine.position_frames()
    );
    // The refusal does not end the track: the stream plays on from its start.
    assert!(
        h.wait(|engine, _| engine.transport() == TransportState::Stopped),
        "the track must end"
    );
    assert_eq!(
        h.engine.last_track_frames(),
        MP3_FRAMES,
        "the stream must play to its end"
    );
}

/// A stream the server refuses is a reported failure, not a silent stop.
#[test]
fn a_url_the_server_refuses_is_a_reported_failure() {
    let server = Server::answering(not_found());
    let mut h = Stream::url(&server.url("/missing.mp3"));
    h.engine.play_index(0).expect("play");

    assert!(
        h.wait(|_, events| events
            .iter()
            .any(|event| matches!(event, Event::Error(info) if info.message.contains("404")))),
        "the status must be reported: {:?}",
        h.error_messages()
    );
}
