//! The `https://` path, exercised through the **real binary**.
//!
//! What these tests prove, and what they deliberately do not:
//!
//! * an `ftp://` argument is refused at parse time with the command line's own
//!   wording, before anything is opened;
//! * the binary really takes the TLS route: pointed at a plain TCP peer, the
//!   first bytes on the wire are a TLS handshake record, never a plaintext
//!   request - there is no fallback to `http://`;
//! * an untrusted certificate ends the run loudly, naming the certificate, and
//!   never reports the track as playing.
//!
//! **Not proven here: a successful transfer.** The player trusts the operating
//! system's store (`TlsAnchors::system()`), so a headless test cannot make its
//! own self-signed certificate trusted without writing into the user's root
//! store. The positive path - an `https://` URL that is trusted and produces
//! frames - therefore belongs to the disposable VM acceptance, where a test CA
//! can be installed in the guest's own store. Claiming it here would be a test
//! that passes by accident.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::sync::Arc;
use std::thread;

use rustamp_app::cli::{self, CliError, parse as cli_parse};

/// A self-signed certificate of the authority `subject` for the given names.
fn self_signed(subject: &str, names: &[&str]) -> (Vec<u8>, Vec<u8>) {
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
    (certificate.der().to_vec(), key.serialize_der())
}

/// A local TLS server that answers one request.
struct TlsServer {
    /// The port it listens on.
    port: u16,
}

impl TlsServer {
    /// A server that responds with `response` and presents the given cert/key.
    fn answering(der: Vec<u8>, key: Vec<u8>, response: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
        let port = listener.local_addr().expect("the address").port();
        thread::spawn(move || {
            let (socket, _) = listener.accept().expect("a connection");
            let mut stream = tls_stream(socket, &der, key);
            read_request(&mut stream);
            let _ = stream.write_all(&response);
            let _ = stream.flush();
        });
        TlsServer { port }
    }

    /// The URL of a path on this server.
    fn url(&self, path: &str) -> String {
        format!("https://127.0.0.1:{}{path}", self.port)
    }
}

/// The TLS stream of the server: the handshake runs while it is read.
fn tls_stream(
    socket: TcpStream,
    der: &[u8],
    key: Vec<u8>,
) -> rustls::StreamOwned<rustls::ServerConnection, TcpStream> {
    use rustls::ServerConfig;
    use rustls::crypto::ring::default_provider;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    let chain = vec![CertificateDer::from(der.to_vec())];
    let private_key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key));
    let config = ServerConfig::builder_with_provider(Arc::new(default_provider()))
        .with_safe_default_protocol_versions()
        .expect("the protocol versions of the server")
        .with_no_client_auth()
        .with_single_cert(chain, private_key)
        .expect("the configuration of the server");
    let connection = rustls::ServerConnection::new(Arc::new(config)).expect("the connection");
    rustls::StreamOwned::new(connection, socket)
}

/// Reads the request head of a connection.
fn read_request(stream: &mut rustls::StreamOwned<rustls::ServerConnection, TcpStream>) {
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

/// Reads the repository fixture.
fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("rustamp-decoders")
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read(path).expect("the fixture")
}

/// Runs the `rustamp` binary headless with the given URL and `--null --seconds 2`,
/// capturing stdout and stderr for assertion.
fn run_rustamp_headless(url: &str) -> (std::process::ExitStatus, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_rustamp"));
    // No window: on Windows suppress the console window
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
    let output = command
        .arg("--null")
        .arg("--seconds")
        .arg("2")
        .arg(url)
        .env("RUSTAMP_CONFIG", dir.path().join("rustamp.toml"))
        .env("RUSTAMP_NO_GUI", "1")
        .output()
        .expect("spawn rustamp");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status, stdout, stderr)
}

/// Parses command line args using the app's CLI parser.
fn parse_cli_args(args: &[&str]) -> Result<cli::Args, CliError> {
    cli_parse(args.iter().map(|s| (*s).to_owned()))
}

#[test]
fn an_ftp_url_is_refused_at_parse_time() {
    let args = parse_cli_args(&["ftp://example.test/stream.mp3"]).unwrap_err();
    match args {
        CliError::UnsupportedUrl { url, scheme } => {
            assert_eq!(scheme, "ftp");
            assert!(url.contains("ftp"));
        }
        other => panic!("expected UnsupportedUrl, got {other:?}"),
    }
}

#[test]
fn an_https_url_with_untrusted_certificate_fails_loudly() {
    let body = fixture("sine.mp3");
    // Generate a cert signed by a different authority
    let (cert_der, _key) = self_signed("another authority", &["localhost", "127.0.0.1"]);
    let server = TlsServer::answering(cert_der.clone(), _key, ok(&body));
    let url = server.url("/sine.mp3");

    // Run the real binary headless with the https URL
    let (status, stdout, stderr) = run_rustamp_headless(&url);

    // The run must not succeed (TLS failure is loud, no fallback to HTTP)
    assert!(
        !status.success(),
        "expected rustamp to fail with an untrusted certificate, but it succeeded:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );

    // The error must mention the certificate
    assert!(
        stderr.contains("certificate") || stdout.contains("certificate"),
        "expected the error to mention the certificate, stderr:\n{stderr}\nstdout:\n{stdout}"
    );

    // It must not report "playing" (since the connection failed)
    assert!(
        !stdout.contains("playing"),
        "expected no 'playing' when cert is untrusted, got:\n{stdout}"
    );
}

#[test]
fn an_https_url_never_falls_back_to_plain_http() {
    // The sharp property: an `https://` URL must put a TLS ClientHello on the
    // wire, never a plaintext request. The first version of this test asked for
    // "playing" and a certificate error in the same run, which is a
    // contradiction - it could only pass by accident.
    //
    // A plain TCP listener plays the part of a server that is not speaking TLS:
    // it records the first bytes of the connection and closes. Whatever the
    // client does next, those bytes are the evidence.
    use std::io::Read;
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback listener");
    let port = listener.local_addr().expect("an address").port();
    let recorder = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one connection");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .ok();
        let mut buffer = vec![0u8; 16];
        let read = stream.read(&mut buffer).unwrap_or(0);
        buffer.truncate(read);
        buffer
    });

    let url = format!("https://127.0.0.1:{port}/sine.mp3");
    let (status, stdout, stderr) = run_rustamp_headless(&url);
    let first_bytes = recorder.join().expect("the recorder thread");

    assert!(
        !status.success(),
        "a peer that does not speak TLS must not produce a successful run:\n{stdout}\n{stderr}"
    );
    assert!(
        !first_bytes.starts_with(b"GET ") && !first_bytes.starts_with(b"GET\t"),
        "the client sent a plaintext request to an https:// URL: {:?}",
        String::from_utf8_lossy(&first_bytes)
    );
    assert!(
        first_bytes.first().copied() == Some(0x16),
        "expected a TLS handshake record (0x16) as the first byte, got {:?}",
        first_bytes
    );
}
