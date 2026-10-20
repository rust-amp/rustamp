//! Integration tests for the HTTPS source.
//!
//! Every test runs against a local TLS server on `127.0.0.1`: the server
//! presents a certificate that the test itself generates, and the client is
//! configured with that certificate as its **only** anchor. No test reaches
//! into the network, and no test opens a window or starts a process.
//!
//! The media is the repository fixture `tests/fixtures/sine.mp3` (1.5 s,
//! 440 Hz, 44.1 kHz stereo sine wave), served over the TLS connection.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rustamp_core::audio::{Decoder, MediaSource};
use rustamp_decoders::{
    CertificateFailure, HttpDecoder, HttpError, HttpProgress, SymphoniaDecoder, TlsAnchors,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection, StreamOwned};

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

/// A certificate of the test server, with the key that signs it.
#[derive(Clone)]
struct TestCertificate {
    /// The certificate, in DER.
    der: Vec<u8>,
    /// The key of the certificate, in PKCS#8 DER.
    key: Vec<u8>,
}

/// A self-signed certificate of the authority `subject` for the given names.
///
/// A name that is an address becomes an address of the certificate, and any
/// other name becomes a host name: that is what the certificate of a server
/// carries.
fn self_signed(subject: &str, names: &[&str]) -> TestCertificate {
    let mut params = rcgen::CertificateParams::new(
        names
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<String>>(),
    )
    .expect("the parameters of the certificate");
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, subject);
    let key = rcgen::KeyPair::generate().expect("the key of the certificate");
    let certificate = params.self_signed(&key).expect("the certificate");
    TestCertificate {
        der: certificate.der().to_vec(),
        key: key.serialize_der(),
    }
}

/// A self-signed certificate of the authority `subject` whose validity ended
/// in the past.
fn expired(subject: &str, names: &[&str]) -> TestCertificate {
    let mut params = rcgen::CertificateParams::new(
        names
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<String>>(),
    )
    .expect("the parameters of the certificate");
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, subject);
    params.not_before = rcgen::date_time_ymd(2000, 1, 1);
    params.not_after = rcgen::date_time_ymd(2001, 1, 1);
    let key = rcgen::KeyPair::generate().expect("the key of the certificate");
    let certificate = params.self_signed(&key).expect("the certificate");
    TestCertificate {
        der: certificate.der().to_vec(),
        key: key.serialize_der(),
    }
}

/// The authority of the certificate of the test server.
const SERVER: &str = "rustamp test server";

/// A local TLS server that answers one request.
struct TlsServer {
    /// The port it listens on.
    port: u16,
    /// The request it received.
    request: Arc<Mutex<Vec<u8>>>,
    /// The name the client announced in the handshake.
    name: Arc<Mutex<Option<String>>>,
}

impl TlsServer {
    /// A server that answers with `response` and presents `certificate`.
    fn answering(certificate: TestCertificate, response: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
        let port = listener.local_addr().expect("the address").port();
        let request = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&request);
        let name = Arc::new(Mutex::new(None));
        let announced = Arc::clone(&name);
        thread::spawn(move || {
            let (socket, _) = listener.accept().expect("a connection");
            let mut stream = tls_stream(socket, &certificate);
            read_request(&mut stream, &recorded);
            // The hello of the client carries the name of the host, and the
            // hello is read while the handshake runs.
            *announced.lock().expect("the lock") = stream.conn.server_name().map(str::to_owned);
            let _ = stream.write_all(&response);
            let _ = stream.flush();
        });
        TlsServer {
            port,
            request,
            name,
        }
    }

    /// The URL of a path on this server.
    fn url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{path}", self.port)
    }

    /// The request the server received.
    fn request(&self) -> Vec<u8> {
        self.request.lock().expect("the lock").clone()
    }

    /// The name the client announced in the handshake.
    fn announced(&self) -> Option<String> {
        self.name.lock().expect("the lock").clone()
    }
}

/// The TLS stream of the server: the handshake runs while it is read.
fn tls_stream(
    socket: TcpStream,
    certificate: &TestCertificate,
) -> StreamOwned<ServerConnection, TcpStream> {
    let chain = vec![CertificateDer::from(certificate.der.clone())];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certificate.key.clone()));
    let config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("the protocol versions of the server")
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .expect("the configuration of the server");
    let connection = ServerConnection::new(Arc::new(config)).expect("the connection");
    StreamOwned::new(connection, socket)
}

/// Reads the request head of a connection and records it.
fn read_request(
    stream: &mut StreamOwned<ServerConnection, TcpStream>,
    recorded: &Arc<Mutex<Vec<u8>>>,
) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let Ok(n) = stream.read(&mut chunk) else {
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

/// The client of a test: the anchors of the server's certificate.
fn anchors_of(certificate: &TestCertificate) -> TlsAnchors {
    TlsAnchors::only(vec![certificate.der.clone()])
}

/// An `https://` stream decodes to the frames the file holds.
#[test]
fn an_https_stream_decodes_to_the_frames_the_file_holds() {
    let body = fixture("sine.mp3");
    let certificate = self_signed(SERVER, &["localhost", "127.0.0.1"]);
    let server = TlsServer::answering(certificate.clone(), ok(&body));
    let anchors = anchors_of(&certificate);
    let mut stream = HttpDecoder::open_with_anchors(
        &server.url("/sine.mp3"),
        TEST_TIMEOUT,
        &HttpProgress::new(),
        &anchors,
    )
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

/// The request of an `https://` stream is the origin form, and the `Host:`
/// line names the port of the URL.
#[test]
fn the_request_of_an_https_stream_is_the_origin_form() {
    let certificate = self_signed(SERVER, &["localhost", "127.0.0.1"]);
    let server = TlsServer::answering(certificate.clone(), ok(&fixture("sine.mp3")));
    let anchors = anchors_of(&certificate);
    let mut stream = HttpDecoder::open_with_anchors(
        &server.url("/music/sine.mp3?x=1"),
        TEST_TIMEOUT,
        &HttpProgress::new(),
        &anchors,
    )
    .expect("the stream opens");
    read_frames(&mut stream);

    let expected = format!(
        "GET /music/sine.mp3?x=1 HTTP/1.0\r\n\
         Host: 127.0.0.1:{}\r\n\
         User-Agent: Winamp/2.95\r\n\
         Accept: */*\r\n\
         \r\n",
        server.port
    );
    assert_eq!(String::from_utf8_lossy(&server.request()), expected);
}

/// An `https://` URL with a port of its own works.
#[test]
fn an_https_url_with_a_port_of_its_own_works() {
    let certificate = self_signed(SERVER, &["localhost", "127.0.0.1"]);
    let server = TlsServer::answering(certificate.clone(), ok(&fixture("sine.mp3")));
    let anchors = anchors_of(&certificate);
    let url = format!("https://127.0.0.1:{}/sine.mp3", server.port);
    let mut stream =
        HttpDecoder::open_with_anchors(&url, TEST_TIMEOUT, &HttpProgress::new(), &anchors)
            .expect("the stream opens");
    assert!(read_frames(&mut stream) > 60_000);
    let request = String::from_utf8_lossy(&server.request()).to_string();
    assert!(
        request.contains(&format!("Host: 127.0.0.1:{}", server.port)),
        "the `Host:` line names the port: {request}"
    );
}

/// The name of the host is announced in the handshake.
#[test]
fn the_name_of_the_host_is_announced_in_the_handshake() {
    let certificate = self_signed(SERVER, &["localhost", "127.0.0.1"]);
    let server = TlsServer::answering(certificate.clone(), ok(&fixture("sine.mp3")));
    let anchors = anchors_of(&certificate);
    // `localhost` names the loopback interface of the machine.
    let url = format!("https://localhost:{}/sine.mp3", server.port);
    let mut stream =
        HttpDecoder::open_with_anchors(&url, TEST_TIMEOUT, &HttpProgress::new(), &anchors)
            .expect("the stream opens");
    read_frames(&mut stream);
    assert_eq!(server.announced().as_deref(), Some("localhost"));
}

/// A certificate the client does not trust is refused by name.
#[test]
fn a_certificate_the_client_does_not_trust_is_refused_by_name() {
    let certificate = self_signed(SERVER, &["localhost", "127.0.0.1"]);
    let server = TlsServer::answering(certificate.clone(), ok(&fixture("sine.mp3")));
    // The client trusts another authority: the one of the server is signed
    // by nobody the client knows.
    let other = self_signed("another authority", &["localhost", "127.0.0.1"]);
    let anchors = TlsAnchors::only(vec![other.der]);
    let err = HttpDecoder::open_with_anchors(
        &server.url("/sine.mp3"),
        TEST_TIMEOUT,
        &HttpProgress::new(),
        &anchors,
    )
    .expect_err("the stream does not open");
    match &err {
        HttpError::Certificate { reason } => {
            assert_eq!(reason, &CertificateFailure::UnknownIssuer)
        }
        other => panic!("unexpected {other:?}"),
    }
    let message = err.to_string();
    assert!(
        message.contains("certificate"),
        "the message names the certificate: {message}"
    );
}

/// A certificate that is not signed by the key of its authority is refused by
/// name.
#[test]
fn a_certificate_that_is_not_signed_by_its_authority_is_refused_by_name() {
    let certificate = self_signed(SERVER, &["localhost", "127.0.0.1"]);
    let server = TlsServer::answering(certificate.clone(), ok(&fixture("sine.mp3")));
    // The client trusts a certificate of the same authority, but with
    // another key: the signature of the server does not fit it.
    let other = self_signed(SERVER, &["localhost", "127.0.0.1"]);
    let anchors = TlsAnchors::only(vec![other.der]);
    let err = HttpDecoder::open_with_anchors(
        &server.url("/sine.mp3"),
        TEST_TIMEOUT,
        &HttpProgress::new(),
        &anchors,
    )
    .expect_err("the stream does not open");
    match &err {
        HttpError::Certificate { reason } => {
            assert_eq!(reason, &CertificateFailure::BadSignature)
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// An expired certificate is refused by name.
#[test]
fn an_expired_certificate_is_refused_by_name() {
    let certificate = expired(SERVER, &["localhost", "127.0.0.1"]);
    let server = TlsServer::answering(certificate.clone(), ok(&fixture("sine.mp3")));
    // The client trusts the certificate, but the certificate is past the end
    // of its validity.
    let anchors = anchors_of(&certificate);
    let err = HttpDecoder::open_with_anchors(
        &server.url("/sine.mp3"),
        TEST_TIMEOUT,
        &HttpProgress::new(),
        &anchors,
    )
    .expect_err("the stream does not open");
    match &err {
        HttpError::Certificate { reason } => assert_eq!(reason, &CertificateFailure::Expired),
        other => panic!("unexpected {other:?}"),
    }
    let message = err.to_string();
    assert!(
        message.contains("certificate"),
        "the message names the certificate: {message}"
    );
}

/// A certificate that names another host is refused by name.
#[test]
fn a_certificate_for_another_host_is_refused_by_name() {
    let certificate = self_signed(SERVER, &["other.example"]);
    let server = TlsServer::answering(certificate.clone(), ok(&fixture("sine.mp3")));
    // The client trusts the certificate, but the certificate is not valid
    // for the host that was asked for.
    let anchors = anchors_of(&certificate);
    let err = HttpDecoder::open_with_anchors(
        &server.url("/sine.mp3"),
        TEST_TIMEOUT,
        &HttpProgress::new(),
        &anchors,
    )
    .expect_err("the stream does not open");
    match &err {
        HttpError::Certificate { reason } => {
            assert_eq!(reason, &CertificateFailure::NotValidForName)
        }
        other => panic!("unexpected {other:?}"),
    }
    let message = err.to_string();
    assert!(
        message.contains("certificate"),
        "the message names the certificate: {message}"
    );
}

/// A refused certificate is a failure of the connection and not a stream
/// without a certificate: the phase says that the host was not reached.
#[test]
fn a_refused_certificate_fails_the_connection() {
    let certificate = self_signed(SERVER, &["localhost", "127.0.0.1"]);
    let server = TlsServer::answering(certificate.clone(), ok(&fixture("sine.mp3")));
    let other = self_signed("another authority", &["localhost", "127.0.0.1"]);
    let anchors = TlsAnchors::only(vec![other.der]);
    let progress = HttpProgress::new();
    let err =
        HttpDecoder::open_with_anchors(&server.url("/sine.mp3"), TEST_TIMEOUT, &progress, &anchors)
            .expect_err("the stream does not open");
    assert!(matches!(err, HttpError::Certificate { .. }));
    assert_eq!(
        progress.snapshot().phase,
        rustamp_decoders::HttpPhase::ConnectFailed,
        "the host was not reached"
    );
}

/// A host that accepts the connection and never answers the handshake runs
/// into the timeout of the source.
///
/// The handshake belongs to the connection, so the phase says that the host
/// is being reached while it waits.
#[test]
fn a_host_that_never_answers_the_handshake_runs_into_the_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
    let port = listener.local_addr().expect("the address").port();
    thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("a connection");
        // The hello of the client is read and never answered: the client
        // waits for the hello of the host until its timeout closes the
        // connection. The second read waits for that.
        let mut sink = [0u8; 1024];
        let _ = socket.read(&mut sink);
        let _ = socket.read(&mut sink);
    });
    let progress = HttpProgress::new();
    let err = HttpDecoder::open_with_anchors(
        &format!("https://127.0.0.1:{port}/sine.mp3"),
        Duration::from_millis(300),
        &progress,
        &TlsAnchors::system(),
    )
    .expect_err("the stream does not open");
    match err {
        HttpError::Timeout { phase } => assert_eq!(phase, "the TLS handshake"),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        progress.snapshot().phase,
        rustamp_decoders::HttpPhase::ConnectFailed,
        "the host was not reached"
    );
}
