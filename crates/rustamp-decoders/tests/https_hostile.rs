//! Hostile input against the `https://` transport.
//!
//! Every test in this file is a server that does not behave, and the question
//! is the same for all of them: does the client answer with a typed failure
//! and leave the transfer at `ConnectFailed`, or does it panic, hang, or
//! accept a stream it should refuse?
//!
//! The properties pinned here are the ones that carry the security of the
//! secure stream, so they are pinned at the level the user sees them:
//!
//! * an **empty anchor set** trusts nothing, and the trust store of the
//!   machine is a real verifier and not "accept anything";
//! * an **address literal** is its own name: nothing is announced, and only an
//!   address in the certificate authorises it - a host name in the SAN, a
//!   wildcard or the common name does not;
//! * a certificate whose **names say nothing usable** (no SAN, not yet valid,
//!   not meant for a server) is refused by the name of the reason;
//! * a connection that **cannot speak TLS** - a plain HTTP answer, garbage, a
//!   cut answer, an oversized record, a chain of many certificates, a host
//!   that talks without finishing the handshake - fails typed instead of
//!   panicking or reading forever;
//! * every one of those failures leaves the transfer at `ConnectFailed` with
//!   **nothing retrieved**: no body bytes, no declared length.
//!
//! Every test runs against a loopback server on this machine. No test reaches
//! into the network, opens a window or starts a process.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rustamp_core::audio::Decoder;
use rustamp_decoders::{
    CertificateFailure, HttpDecoder, HttpError, HttpPhase, HttpProgress, TlsAnchors,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use rustls::{ServerConfig, ServerConnection, StreamOwned};

/// The timeout the tests give the exchange.
///
/// The default of the source is ten seconds; a test that waited for it would
/// be slow, so every test that needs a failure in time asks for a short one.
const TEST_TIMEOUT: Duration = Duration::from_secs(2);

// ---------------------------------------------------------------------------
// Ceremony: the certificates of the test servers
// ---------------------------------------------------------------------------

/// A certificate of a test server, with the key that signs for it.
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
fn certificate(names: &[&str], subject: &str) -> TestCertificate {
    certificate_with(names, subject, |_| {})
}

/// The same, with the parameters of the certificate changed by `tweak`.
fn certificate_with(
    names: &[&str],
    subject: &str,
    tweak: impl FnOnce(&mut rcgen::CertificateParams),
) -> TestCertificate {
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
    tweak(&mut params);
    let key = rcgen::KeyPair::generate().expect("the key of the certificate");
    let certificate = params.self_signed(&key).expect("the certificate");
    TestCertificate {
        der: certificate.der().to_vec(),
        key: key.serialize_der(),
    }
}

/// The repository fixture.
fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(path).expect("the fixture")
}

/// A 200 answer with `body` as its body.
fn ok(body: &[u8]) -> Vec<u8> {
    let mut response =
        format!("HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    response.extend_from_slice(body);
    response
}

// ---------------------------------------------------------------------------
// Servers
// ---------------------------------------------------------------------------

/// A TLS server on `host` (an address literal) that answers one request.
struct Server {
    /// The host the URL names: an address literal, with the brackets of an
    /// IPv6 one.
    host: String,
    /// The port it listens on.
    port: u16,
    /// The name the client announced in the handshake, when it announced one.
    ///
    /// `None` is the point of the address tests: an address literal is not a
    /// name, so there is nothing to announce.
    announced: Arc<Mutex<Option<String>>>,
}

impl Server {
    /// The URL of a path on this server.
    fn url(&self, path: &str) -> String {
        format!("https://{}:{}{path}", self.host, self.port)
    }

    /// The name the client announced in the handshake.
    fn announced(&self) -> Option<String> {
        self.announced.lock().expect("the lock").clone()
    }
}

/// A TLS server that presents `certificate` and answers with `response`.
fn tls_server(host: &str, certificate: &TestCertificate, response: Vec<u8>) -> Server {
    tls_server_with_chain(
        host,
        vec![certificate.der.clone()],
        certificate.key.clone(),
        response,
    )
}

/// A TLS server that presents `chain` and answers with `response`.
fn tls_server_with_chain(
    host: &str,
    chain: Vec<Vec<u8>>,
    key: Vec<u8>,
    response: Vec<u8>,
) -> Server {
    let listener = TcpListener::bind(format!("{host}:0")).expect("a local listener");
    let port = listener.local_addr().expect("the address").port();
    let announced = Arc::new(Mutex::new(None));
    let recorded = Arc::clone(&announced);
    thread::spawn(move || {
        let (socket, _) = listener.accept().expect("a connection");
        let _ = socket.set_read_timeout(Some(TEST_TIMEOUT));
        let _ = socket.set_write_timeout(Some(TEST_TIMEOUT));
        let chain = chain
            .into_iter()
            .map(CertificateDer::from)
            .collect::<Vec<_>>();
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key));
        let config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .expect("the protocol versions of the server")
                .with_no_client_auth()
                .with_single_cert(chain, key)
                .expect("the configuration of the server");
        let connection = ServerConnection::new(Arc::new(config)).expect("the connection");
        let mut stream = StreamOwned::new(connection, socket);
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    buffer.extend_from_slice(&chunk[..n]);
                    if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
            }
        }
        // The name of the client is announced inside the handshake, so it is
        // there once the request has been read.
        *recorded.lock().expect("the lock") = stream.conn.server_name().map(str::to_owned);
        let _ = stream.write_all(&response);
        let _ = stream.flush();
        // The answer is out before the connection ends.
        thread::sleep(Duration::from_millis(100));
    });
    Server {
        host: host.to_string(),
        port,
        announced,
    }
}

/// A server that does not speak TLS: it runs `behaviour` on the connection.
fn raw_server(behaviour: impl FnOnce(&mut TcpStream) + Send + 'static) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
    let port = listener.local_addr().expect("the address").port();
    thread::spawn(move || {
        let (mut socket, _) = listener.accept().expect("a connection");
        let _ = socket.set_read_timeout(Some(TEST_TIMEOUT));
        let _ = socket.set_write_timeout(Some(TEST_TIMEOUT));
        behaviour(&mut socket);
    });
    port
}

/// A TLS server that presents no certificate at all.
#[derive(Debug)]
struct NoCertificate;

impl ResolvesServerCert for NoCertificate {
    fn resolve(&self, _: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        None
    }
}

/// The port of a TLS server that presents no certificate.
fn server_without_a_certificate() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
    let port = listener.local_addr().expect("the address").port();
    thread::spawn(move || {
        let (socket, _) = listener.accept().expect("a connection");
        let _ = socket.set_read_timeout(Some(TEST_TIMEOUT));
        let config =
            ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .expect("the protocol versions of the server")
                .with_no_client_auth()
                .with_cert_resolver(Arc::new(NoCertificate));
        let connection = ServerConnection::new(Arc::new(config)).expect("the connection");
        let mut stream = StreamOwned::new(connection, socket);
        let mut sink = [0u8; 1024];
        let _ = stream.read(&mut sink);
    });
    port
}

// ---------------------------------------------------------------------------
// The failure the client must answer with
// ---------------------------------------------------------------------------

/// Opens `url` and returns the failure, with the transfer it left behind.
///
/// The three facts every hostile server has to share are checked here once:
/// the failure is a failure of the TLS layer and not a stream, the host was
/// not reached, and nothing of a body was retrieved.
fn failure(url: &str, anchors: &TlsAnchors) -> HttpError {
    let progress = HttpProgress::new();
    let error = HttpDecoder::open_with_anchors(url, TEST_TIMEOUT, &progress, anchors)
        .expect_err("the stream must not open");
    let snapshot = progress.snapshot();
    assert!(
        matches!(
            error,
            HttpError::Certificate { .. } | HttpError::Tls { .. } | HttpError::Timeout { .. }
        ),
        "not a failure of the TLS layer: {error:?}"
    );
    assert_eq!(
        snapshot.phase,
        HttpPhase::ConnectFailed,
        "the host was not reached: {error}"
    );
    assert_eq!(
        snapshot.bytes_read, 0,
        "a stream that never opened cannot have retrieved anything: {error}"
    );
    assert_eq!(
        snapshot.total, None,
        "a stream that never opened cannot know a length: {error}"
    );
    error
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

// ---------------------------------------------------------------------------
// The anchors
// ---------------------------------------------------------------------------

/// An anchor set that names nothing trusts nothing.
///
/// This is the security-critical half of [`TlsAnchors::only`]: an empty set is
/// not "the store of the machine" and it is certainly not "accept anything".
/// The server presents a certificate that is perfectly usable - the only
/// reason to refuse it is that nobody was named as an anchor.
#[test]
fn an_empty_anchor_set_trusts_nothing() {
    let certificate = certificate(&["127.0.0.1"], "the server");
    let server = tls_server("127.0.0.1", &certificate, ok(&fixture("sine.mp3")));
    let error = failure(&server.url("/sine.mp3"), &TlsAnchors::only(Vec::new()));
    match error {
        HttpError::Certificate { reason } => assert_eq!(
            reason,
            CertificateFailure::UnknownIssuer,
            "an unused authority is the only reason"
        ),
        other => panic!("unexpected {other:?}"),
    }
}

/// The trust store of the machine is a verifier and not a formality.
///
/// The certificate of the server is generated for this test, so no store of
/// any machine knows its authority. The default anchors must refuse it; a
/// client that accepted it would accept every certificate of the world. The
/// reason is the one of the verifier of the machine, which is not this
/// crate's to name - only the kind is checked.
#[test]
fn the_store_of_the_machine_refuses_an_authority_it_does_not_know() {
    let certificate = certificate(&["127.0.0.1"], "a stranger");
    let server = tls_server("127.0.0.1", &certificate, ok(&fixture("sine.mp3")));
    let error = failure(&server.url("/sine.mp3"), &TlsAnchors::system());
    match error {
        HttpError::Certificate { .. } => {}
        other => panic!("the store of the machine refused it as something else: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// An address literal is its own name
// ---------------------------------------------------------------------------

/// An address in the certificate authorises the address, and nothing is
/// announced for it.
///
/// The URL names an address literal, so the handshake has no name to
/// announce - the server name indication exists for names - and the
/// certificate authorises the connection with the address in its subject
/// alternative names.
#[test]
fn an_address_in_the_certificate_authorises_the_address() {
    let certificate = certificate(&["127.0.0.1"], "the server");
    let server = tls_server("127.0.0.1", &certificate, ok(&fixture("sine.mp3")));
    let anchors = TlsAnchors::only(vec![certificate.der.clone()]);
    let mut stream = HttpDecoder::open_with_anchors(
        &server.url("/sine.mp3"),
        TEST_TIMEOUT,
        &HttpProgress::new(),
        &anchors,
    )
    .expect("the stream opens");
    assert!(
        read_frames(&mut stream) > 60_000,
        "the stream holds the audio"
    );
    assert_eq!(
        server.announced(),
        None,
        "an address literal is not a name, so there is nothing to announce"
    );
}

/// A name in the certificate does not authorise an address.
#[test]
fn a_name_in_the_certificate_does_not_authorise_an_address() {
    let certificate = certificate(&["localhost"], "localhost");
    let server = tls_server("127.0.0.1", &certificate, ok(&fixture("sine.mp3")));
    let anchors = TlsAnchors::only(vec![certificate.der.clone()]);
    let error = failure(&server.url("/sine.mp3"), &anchors);
    match error {
        HttpError::Certificate { reason } => {
            assert_eq!(reason, CertificateFailure::NotValidForName)
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// A wildcard in the certificate does not authorise an address either.
///
/// The wildcard of a certificate covers names below a domain, and an address
/// is no such name: `*.0.0.1` is not a way to authorise `127.0.0.1`.
#[test]
fn a_wildcard_in_the_certificate_does_not_authorise_an_address() {
    let certificate = certificate(&["*.0.0.1"], "a wildcard");
    let server = tls_server("127.0.0.1", &certificate, ok(&fixture("sine.mp3")));
    let anchors = TlsAnchors::only(vec![certificate.der.clone()]);
    let error = failure(&server.url("/sine.mp3"), &anchors);
    match error {
        HttpError::Certificate { reason } => {
            assert_eq!(reason, CertificateFailure::NotValidForName)
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// An IPv6 address literal is checked the same way as an IPv4 one.
///
/// The URL carries the address in brackets, and the brackets belong to the
/// URL: the name of the connection is the bare address, and the certificate
/// authorises it with an address in its subject alternative names.
#[test]
fn an_ipv6_address_literal_is_checked_the_same_way() {
    let certificate = certificate(&["::1"], "the server");
    let server = tls_server("[::1]", &certificate, ok(&fixture("sine.mp3")));
    let anchors = TlsAnchors::only(vec![certificate.der.clone()]);
    let mut stream = HttpDecoder::open_with_anchors(
        &server.url("/sine.mp3"),
        TEST_TIMEOUT,
        &HttpProgress::new(),
        &anchors,
    )
    .expect("the stream opens");
    assert!(read_frames(&mut stream) > 60_000);
    assert_eq!(server.announced(), None, "an address is not a name");
}

// ---------------------------------------------------------------------------
// Certificates whose names say nothing usable
// ---------------------------------------------------------------------------

/// The common name of a certificate is not one of its names.
///
/// A certificate without a subject alternative name names nothing at all, and
/// its common name does not take the place of the missing names: a name that
/// only appears there authorises nothing.
#[test]
fn the_common_name_of_a_certificate_is_not_a_name() {
    let certificate = certificate(&[], "127.0.0.1");
    let server = tls_server("127.0.0.1", &certificate, ok(&fixture("sine.mp3")));
    let anchors = TlsAnchors::only(vec![certificate.der.clone()]);
    let error = failure(&server.url("/sine.mp3"), &anchors);
    match error {
        HttpError::Certificate { reason } => {
            assert_eq!(reason, CertificateFailure::NotValidForName)
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// A certificate that is not valid yet is refused by the name of the reason.
#[test]
fn a_certificate_that_is_not_valid_yet_is_refused_by_name() {
    let certificate = certificate_with(&["127.0.0.1"], "the server", |params| {
        params.not_before = rcgen::date_time_ymd(2030, 1, 1);
        params.not_after = rcgen::date_time_ymd(2031, 1, 1);
    });
    let server = tls_server("127.0.0.1", &certificate, ok(&fixture("sine.mp3")));
    let anchors = TlsAnchors::only(vec![certificate.der.clone()]);
    let error = failure(&server.url("/sine.mp3"), &anchors);
    match error {
        HttpError::Certificate { reason } => assert_eq!(reason, CertificateFailure::NotValidYet),
        other => panic!("unexpected {other:?}"),
    }
}

/// A certificate that is meant for a client and not for a server is refused
/// by the name of the reason.
#[test]
fn a_certificate_that_is_not_meant_for_a_server_is_refused_by_name() {
    let certificate = certificate_with(&["127.0.0.1"], "the server", |params| {
        params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ClientAuth];
    });
    let server = tls_server("127.0.0.1", &certificate, ok(&fixture("sine.mp3")));
    let anchors = TlsAnchors::only(vec![certificate.der.clone()]);
    let error = failure(&server.url("/sine.mp3"), &anchors);
    match error {
        HttpError::Certificate { reason } => assert_eq!(reason, CertificateFailure::InvalidPurpose),
        other => panic!("unexpected {other:?}"),
    }
}

/// A certificate with a very long subject and a long list of names is no
/// problem: the names are still compared, and the address among them
/// authorises the connection.
#[test]
fn a_certificate_with_a_very_long_subject_is_accepted() {
    let mut names = (0..200)
        .map(|index| format!("{index}.example.test"))
        .collect::<Vec<String>>();
    names.push("127.0.0.1".to_string());
    let names = names.iter().map(String::as_str).collect::<Vec<&str>>();
    let certificate = certificate(&names, &"a".repeat(2000));
    let server = tls_server("127.0.0.1", &certificate, ok(&fixture("sine.mp3")));
    let anchors = TlsAnchors::only(vec![certificate.der.clone()]);
    let mut stream = HttpDecoder::open_with_anchors(
        &server.url("/sine.mp3"),
        TEST_TIMEOUT,
        &HttpProgress::new(),
        &anchors,
    )
    .expect("the stream opens");
    assert!(read_frames(&mut stream) > 60_000);
}

/// A chain of many certificates is read and refused, not believed.
///
/// The host sends the same certificate thirty-two times, and none of them is
/// signed by an anchor of the client.
#[test]
fn a_chain_of_many_certificates_is_refused() {
    let presented = certificate(&["127.0.0.1"], "the server");
    let mut chain = Vec::new();
    for _ in 0..32 {
        chain.push(presented.der.clone());
    }
    let server = tls_server_with_chain(
        "127.0.0.1",
        chain,
        presented.key.clone(),
        ok(&fixture("sine.mp3")),
    );
    let authority = certificate(&["127.0.0.1"], "another authority");
    let anchors = TlsAnchors::only(vec![authority.der]);
    let error = failure(&server.url("/sine.mp3"), &anchors);
    match error {
        HttpError::Certificate { reason } => assert_eq!(reason, CertificateFailure::UnknownIssuer),
        other => panic!("unexpected {other:?}"),
    }
}

/// A chain that does not fit into a handshake message is refused, not read
/// into the memory of the player.
///
/// The host sends two hundred certificates: the message that carries them is
/// larger than the handshake message the TLS layer accepts, so the layer
/// refuses it instead of collecting it.
#[test]
fn a_chain_that_does_not_fit_into_a_handshake_message_is_refused() {
    let presented = certificate(&["127.0.0.1"], "the server");
    let mut chain = Vec::new();
    for _ in 0..200 {
        chain.push(presented.der.clone());
    }
    let server = tls_server_with_chain(
        "127.0.0.1",
        chain,
        presented.key.clone(),
        ok(&fixture("sine.mp3")),
    );
    let anchors = TlsAnchors::only(vec![presented.der.clone()]);
    let error = failure(&server.url("/sine.mp3"), &anchors);
    match error {
        HttpError::Tls { reason } => {
            // The TLS layer refuses the message that carries the chain.
            assert!(
                reason.contains("HandshakePayloadTooLarge"),
                "the layer names what it refused: {reason}"
            )
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// Any answer of the host is answered with a typed failure.
///
/// The bytes are the ones of a fixed pseudo random sequence, so a failure
/// can be looked at again: the client must not panic on any of them, must
/// not hang on any of them, and must not accept a stream on any of them.
#[test]
fn any_answer_of_the_host_is_answered_with_a_typed_failure() {
    let mut seed = 0x5eed_1234_u64;
    for round in 0..32 {
        let length = (round * 37 % 400) + 1;
        let mut bytes = Vec::with_capacity(length);
        for _ in 0..length {
            // A linear congruential generator: the same bytes every run.
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            bytes.push((seed >> 33) as u8);
        }
        let port = raw_server(move |socket| {
            let mut sink = [0u8; 4096];
            let _ = socket.read(&mut sink);
            let _ = socket.write_all(&bytes);
            thread::sleep(Duration::from_millis(50));
        });
        let error = failure(
            &format!("https://127.0.0.1:{port}/sine.mp3"),
            &TlsAnchors::only(Vec::new()),
        );
        assert!(
            matches!(
                error,
                HttpError::Certificate { .. } | HttpError::Tls { .. } | HttpError::Timeout { .. }
            ),
            "not a failure of the TLS layer: {error:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Connections that cannot speak TLS
// ---------------------------------------------------------------------------

/// A server that answers with plain HTTP never becomes a stream.
///
/// The answer is a complete `200` with a playable body: the only thing wrong
/// with it is that it is not TLS. A client that fell back to a plain
/// connection would play it, so the test fails if such a fallback ever
/// appears.
#[test]
fn a_plain_http_answer_never_becomes_a_stream() {
    let body = fixture("sine.mp3");
    let port = raw_server(move |socket| {
        let mut sink = [0u8; 4096];
        let _ = socket.read(&mut sink);
        let _ = socket.write_all(&ok(&body));
    });
    let error = failure(
        &format!("https://127.0.0.1:{port}/sine.mp3"),
        &TlsAnchors::system(),
    );
    match error {
        HttpError::Tls { reason } => {
            assert!(!reason.is_empty(), "the handshake names why it failed")
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// Bytes that are not a TLS record at all are refused.
#[test]
fn garbage_instead_of_a_hello_is_refused() {
    let port = raw_server(|socket| {
        let mut sink = [0u8; 4096];
        let _ = socket.read(&mut sink);
        let _ = socket.write_all(b"this is not a TLS record");
        thread::sleep(Duration::from_millis(100));
    });
    let error = failure(
        &format!("https://127.0.0.1:{port}/sine.mp3"),
        &TlsAnchors::only(Vec::new()),
    );
    match error {
        HttpError::Tls { reason } => {
            assert!(!reason.is_empty(), "the handshake names why it failed")
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// A record header that claims sixteen megabytes is refused, not allocated.
#[test]
fn a_record_that_claims_a_huge_length_is_refused() {
    let port = raw_server(|socket| {
        let mut sink = [0u8; 4096];
        let _ = socket.read(&mut sink);
        // Type 22 (handshake), version 3.3, a length of 65535 - the largest
        // the header can claim - and a single byte behind it.
        let _ = socket.write_all(&[0x16, 0x03, 0x03, 0xff, 0xff, 0x02]);
        thread::sleep(Duration::from_millis(100));
    });
    let error = failure(
        &format!("https://127.0.0.1:{port}/sine.mp3"),
        &TlsAnchors::only(Vec::new()),
    );
    match error {
        HttpError::Tls { reason } => {
            assert!(!reason.is_empty(), "the handshake names why it failed")
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// An answer that is cut in the middle of a record is a failure.
#[test]
fn a_handshake_answer_that_is_cut_short_is_refused() {
    let port = raw_server(|socket| {
        let mut sink = [0u8; 4096];
        let _ = socket.read(&mut sink);
        // The first eight bytes of a handshake record that claims 16 KiB, and
        // then the end of the connection.
        let _ = socket.write_all(&[0x16, 0x03, 0x03, 0x40, 0x00, 0x02, 0x03, 0x03]);
    });
    let error = failure(
        &format!("https://127.0.0.1:{port}/sine.mp3"),
        &TlsAnchors::only(Vec::new()),
    );
    match error {
        HttpError::Tls { reason } => {
            assert!(!reason.is_empty(), "the handshake names why it failed")
        }
        other => panic!("unexpected {other:?}"),
    }
}

/// A host that closes the connection in the middle of the handshake is a
/// failure, not a stream without a handshake.
///
/// The failure names what the host did: a text of the standard library
/// ("unexpected end of file") would not say that the handshake was the
/// thing that was cut short.
#[test]
fn a_silent_close_in_the_middle_of_the_handshake_is_refused() {
    let port = raw_server(|socket| {
        let mut sink = [0u8; 4096];
        let _ = socket.read(&mut sink);
    });
    let error = failure(
        &format!("https://127.0.0.1:{port}/sine.mp3"),
        &TlsAnchors::only(Vec::new()),
    );
    match error {
        HttpError::Tls { reason } => assert!(
            reason.contains("closed the connection"),
            "the failure names what the host did: {reason}"
        ),
        other => panic!("unexpected {other:?}"),
    }
}

/// A host that keeps talking without ever finishing the handshake does not
/// keep the client reading.
///
/// Each record is one the TLS layer consumes and rejects at once, so a client
/// that read for as long as the host kept writing would read forever: the
/// timeout of the socket bounds a read that returns nothing, not a host that
/// never stops sending. The failure has to arrive while the host is still
/// talking.
#[test]
fn a_host_that_talks_without_finishing_the_handshake_never_hangs() {
    let port = raw_server(|socket| {
        let mut sink = [0u8; 4096];
        let _ = socket.read(&mut sink);
        // A "change cipher spec" record with the one byte of the change: the
        // TLS layer rejects it in this state, and the host repeats it.
        let started = Instant::now();
        while started.elapsed() < 6 * TEST_TIMEOUT {
            if socket
                .write_all(&[0x14, 0x03, 0x03, 0x00, 0x01, 0x01])
                .is_err()
            {
                return;
            }
        }
    });
    let (sender, receiver) = mpsc::channel();
    let progress = HttpProgress::new();
    let reported = progress.clone();
    thread::spawn(move || {
        let error = HttpDecoder::open_with_anchors(
            &format!("https://127.0.0.1:{port}/sine.mp3"),
            TEST_TIMEOUT,
            &reported,
            &TlsAnchors::only(Vec::new()),
        )
        .expect_err("the stream must not open");
        let _ = sender.send(error);
    });
    // A second is a hundred times the time the failure needs, and still
    // shorter than the timeout of the source.
    let error = receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("the handshake ends instead of reading for as long as the host talks");
    assert!(
        matches!(
            error,
            HttpError::Certificate { .. } | HttpError::Tls { .. } | HttpError::Timeout { .. }
        ),
        "not a failure of the TLS layer: {error:?}"
    );
    assert_eq!(
        progress.snapshot().phase,
        HttpPhase::ConnectFailed,
        "the host was not reached: {error}"
    );
}

/// A host that trickles the handshake never ends it.
///
/// The host sends one byte every fifty milliseconds: every single read of the
/// client gets its byte before the timeout of the socket fires, so a timeout
/// that bounds the *gap* between two reads never fires. The handshake has to
/// end anyway: the client must not wait for as long as the host keeps
/// talking.
#[test]
fn a_host_that_trickles_the_handshake_does_not_keep_the_client_waiting() {
    let port = raw_server(|socket| {
        let mut sink = [0u8; 4096];
        let _ = socket.read(&mut sink);
        // The head of a handshake record of 16384 bytes: the TLS layer waits
        // for the rest of it and never sees a complete message.
        let _ = socket.write_all(&[0x16, 0x03, 0x03, 0x40, 0x00]);
        // One byte of the body every fifty milliseconds: every single read of
        // the client gets its byte before the timeout of the socket fires, so
        // a timeout that bounds the gap between two reads never fires.
        let started = Instant::now();
        while started.elapsed() < 30 * TEST_TIMEOUT {
            if socket.write_all(&[0x00]).is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
    });
    let (sender, receiver) = mpsc::channel();
    let progress = HttpProgress::new();
    let reported = progress.clone();
    thread::spawn(move || {
        let error = HttpDecoder::open_with_anchors(
            &format!("https://127.0.0.1:{port}/sine.mp3"),
            TEST_TIMEOUT,
            &reported,
            &TlsAnchors::only(Vec::new()),
        )
        .expect_err("the stream must not open");
        let _ = sender.send(error);
    });
    // Ten times the time the failure needs, and still far below the time the
    // host would keep talking.
    let error = receiver
        .recv_timeout(10 * TEST_TIMEOUT)
        .expect("the handshake ends instead of waiting for as long as the host talks");
    match error {
        HttpError::Timeout { phase } => assert_eq!(phase, "the TLS handshake"),
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        progress.snapshot().phase,
        HttpPhase::ConnectFailed,
        "the host was not reached"
    );
}

/// A host that presents no certificate is refused before it can be trusted.
#[test]
fn a_server_that_presents_no_certificate_is_refused() {
    let error = failure(
        &format!(
            "https://127.0.0.1:{}/sine.mp3",
            server_without_a_certificate()
        ),
        &TlsAnchors::only(Vec::new()),
    );
    match error {
        HttpError::Tls { reason } => {
            assert!(!reason.is_empty(), "the handshake names why it failed")
        }
        other => panic!("unexpected {other:?}"),
    }
}
