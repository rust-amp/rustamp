//! The TLS of an `https://` stream: the trust anchors, the handshake and the
//! failures of both.
//!
//! # What the spec settles
//!
//! Nothing. S19 §1 documents the request template of the reference player and
//! the template is plain `http://`; the spec establishes no evidence that the
//! reference reads a TLS stream at all. **HTTPS is an extension of this
//! implementation**, not a fact of the spec, and the module docs of
//! [`crate::http_source`] mark it as such where the request is built.
//!
//! # What this module decides
//!
//! * **The trust anchors**: the trust store of the operating system, or an
//!   explicit set of certificates ([`TlsAnchors`]).
//! * **The failure of a certificate**: a typed name for the reasons a
//!   certificate is refused ([`CertificateFailure`]). A refused certificate
//!   never becomes a plain connection: the failure names itself and the
//!   stream ends there.
//! * **The deadline of the handshake**: the timeout of the source bounds the
//!   handshake as a whole and not only the gap between two reads, so a host
//!   that trickles bytes into the connection cannot keep the client waiting
//!   ([`DeadlineSocket`]).
//! * **The cryptography**: TLS 1.3 and TLS 1.2 with the `ring` provider of
//!   the `rustls` crate. `rustls` is a blocking library, so this crate stays
//!   a decoder and pulls in no async runtime.
//!
//! # The trust store of the operating system
//!
//! The default is the store of the machine the player runs on, and the
//! **operating system itself** checks the certificate: on Windows the chain is
//! built and judged by the certificate engine of the system, with the stores
//! and the policies of the machine. A corporate authority that the machine
//! trusts is therefore trusted here too, and a certificate the machine
//! revokes is refused. A bundled root store would be a snapshot of the
//! Mozilla list: it would know none of the authorities of a corporate network
//! and would go stale between the releases of the player.
//!
//! The anchors can also be named outright ([`TlsAnchors::only`]): then the
//! named certificates are the **only** anchors, the store of the operating
//! system is not consulted, and the certificate is checked by `rustls`
//! itself. That is what the tests of this crate use, and it is what a player
//! needs when a stream is served under an authority the machine does not
//! know.
//!
//! Nothing in this module was taken from a Winamp source, from SDK material
//! or from a disassembly.

use std::fmt;
use std::io;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_platform_verifier::BuilderVerifierExt;

/// The stream of a TLS connection: the TLS layer over a socket.
pub(crate) type TlsStream = StreamOwned<ClientConnection, DeadlineSocket>;

/// The socket of a TLS connection, with the deadline of the handshake.
///
/// The timeout of the source bounds the **gap between two reads**, not the
/// handshake as a whole: a host that trickles a byte into the connection
/// every few milliseconds keeps every single read below that timeout and
/// would keep the client waiting for as long as it keeps talking. While the
/// handshake runs, every read of the socket therefore waits no longer than
/// the time the handshake has left, and the socket gets its own timeout back
/// when the handshake is over.
pub(crate) struct DeadlineSocket {
    /// The socket of the connection.
    socket: TcpStream,
    /// The moment the handshake has to be done by, while it runs.
    deadline: Option<Instant>,
    /// The read timeout of the socket: the budget of the source.
    budget: Option<Duration>,
}

impl DeadlineSocket {
    /// The socket of a connection, with the budget of the source.
    ///
    /// The budget is the read timeout the socket carries: the source sets it
    /// before the connection is made, and it is the time the handshake may
    /// take as a whole.
    fn new(socket: TcpStream) -> Self {
        let budget = socket.read_timeout().ok().flatten();
        DeadlineSocket {
            socket,
            deadline: None,
            budget,
        }
    }

    /// Gives the socket a deadline for the handshake.
    fn start_handshake(&mut self) {
        self.deadline = self
            .budget
            .and_then(|budget| Instant::now().checked_add(budget));
    }

    /// Takes the deadline away and gives the socket its own timeout back.
    fn end_handshake(&mut self) {
        self.deadline = None;
        if let Some(budget) = self.budget {
            let _ = self.socket.set_read_timeout(Some(budget));
        }
    }
}

impl io::Read for DeadlineSocket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(deadline) = self.deadline {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the TLS handshake ran out of time",
                ));
            }
            // The socket waits no longer than the time that is left. A
            // millisecond is the shortest wait a socket can be given.
            let _ = self
                .socket
                .set_read_timeout(Some(left.max(Duration::from_millis(1))));
        }
        self.socket.read(buf)
    }
}

impl io::Write for DeadlineSocket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.socket.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}

/// The trust anchors a TLS connection is checked against.
///
/// The default is the trust store of the operating system
/// ([`TlsAnchors::system`]); the anchors can also be named outright
/// ([`TlsAnchors::only`]).
#[derive(Debug, Clone)]
pub struct TlsAnchors {
    /// The certificates the client trusts, in DER.
    ///
    /// `None` means: the trust store of the operating system.
    certificates: Option<Vec<Vec<u8>>>,
}

impl Default for TlsAnchors {
    /// The trust store of the operating system.
    fn default() -> Self {
        Self::system()
    }
}

impl TlsAnchors {
    /// The trust store of the operating system.
    ///
    /// The operating system checks the certificate of the host: on Windows
    /// the certificate engine of the machine builds the chain and judges it
    /// with the stores and the policies of the machine, so an authority that
    /// the machine trusts - a corporate one included - is trusted here too.
    pub fn system() -> Self {
        TlsAnchors { certificates: None }
    }

    /// Only the given certificates, in DER.
    ///
    /// The certificates are the **only** anchors: the trust store of the
    /// operating system is not consulted, and a host whose certificate is
    /// not signed by one of them is refused. The certificate itself is
    /// checked by `rustls`.
    pub fn only(certificates: Vec<Vec<u8>>) -> Self {
        TlsAnchors {
            certificates: Some(certificates),
        }
    }

    /// The client configuration that checks a certificate against these
    /// anchors.
    fn client_config(&self) -> Result<ClientConfig, TlsError> {
        // The provider is named instead of taken from the process default:
        // the crate compiles exactly one, and the configuration does not
        // depend on what another part of the program installed.
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let builder = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| TlsError::Setup(e.to_string()))?;
        match &self.certificates {
            // The store of the operating system: the operating system checks
            // the certificate.
            None => Ok(builder
                .with_platform_verifier()
                .map_err(|e| TlsError::Setup(e.to_string()))?
                .with_no_client_auth()),
            // Only the named certificates: `rustls` checks the certificate.
            Some(certificates) => {
                let roots = root_store(certificates)?;
                Ok(builder.with_root_certificates(roots).with_no_client_auth())
            }
        }
    }
}

/// The trust store of the named anchors.
///
/// The named certificates are the **only** anchors: the store of the
/// operating system is not consulted, and an empty set of names is an empty
/// store - it trusts nothing at all.
fn root_store(certificates: &[Vec<u8>]) -> Result<RootCertStore, TlsError> {
    let mut roots = RootCertStore::empty();
    for certificate in certificates {
        roots
            .add(CertificateDer::from(certificate.clone()))
            .map_err(|e| TlsError::Setup(e.to_string()))?;
    }
    Ok(roots)
}

/// Why the certificate of a host was refused.
///
/// The names are the reasons the TLS layer reports. A certificate that is
/// refused is a failure of its own: the source does not fall back to a plain
/// connection and does not read the stream without a certificate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CertificateFailure {
    /// The certificate is not signed by an anchor the client trusts.
    UnknownIssuer,
    /// The certificate is not signed by the key of the authority that issued
    /// it.
    BadSignature,
    /// The certificate is past the end of its validity.
    Expired,
    /// The certificate is not valid yet.
    NotValidYet,
    /// The certificate is not valid for the host that was asked for.
    NotValidForName,
    /// The certificate was revoked.
    Revoked,
    /// The certificate is not meant for this use.
    InvalidPurpose,
    /// The TLS layer refused the certificate for another reason and named it.
    Other(String),
}

impl fmt::Display for CertificateFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::UnknownIssuer => "it is not signed by an authority the client trusts",
            Self::BadSignature => "it is not signed by the key of the authority that issued it",
            Self::Expired => "it is past the end of its validity",
            Self::NotValidYet => "it is not valid yet",
            Self::NotValidForName => "it is not valid for the host that was asked for",
            Self::Revoked => "it was revoked",
            Self::InvalidPurpose => "it is not meant for this use",
            Self::Other(reason) => return write!(f, "{reason}"),
        };
        write!(f, "{text}")
    }
}

/// The failure of a handshake that the host ended in the middle of it.
const CLOSED_IN_THE_MIDDLE: &str = "the host closed the connection before the handshake was done";

/// The failures of the TLS layer.
#[derive(Debug)]
pub(crate) enum TlsError {
    /// The certificate of the host was refused.
    Certificate(CertificateFailure),
    /// The host did not answer the handshake in time.
    Timeout,
    /// The handshake failed: the host and the client did not agree on a
    /// protocol, or the host sent an alert.
    Handshake(String),
    /// The connection could not be set up: the anchors or the name of the
    /// host are not usable.
    Setup(String),
}

/// The TLS stream of a client that talks to `host` over `socket`.
///
/// The name of the host is the name the certificate is checked against, and
/// it is the name the handshake announces (the server name indication). An
/// address literal is not a name: the handshake announces nothing for it, and
/// the certificate must carry the address in one of its subject alternative
/// names.
pub(crate) fn client_stream(
    host: &str,
    anchors: &TlsAnchors,
    socket: TcpStream,
) -> Result<TlsStream, TlsError> {
    let config = anchors.client_config()?;
    let name = ServerName::try_from(host.to_owned())
        .map_err(|_| TlsError::Setup(format!("{host:?} is not a host name")))?;
    let connection = ClientConnection::new(Arc::new(config), name)
        .map_err(|e| TlsError::Handshake(e.to_string()))?;
    Ok(StreamOwned::new(connection, DeadlineSocket::new(socket)))
}

/// The handshake of a TLS stream.
///
/// The handshake belongs to the connection: it happens while the source says
/// that it is reaching the host, and a failure of it is a failure to connect.
///
/// The handshake has a **deadline of its own**: the timeout of the socket
/// bounds the gap between two reads, and a host that trickles a byte into the
/// connection every few milliseconds keeps every single read below that
/// timeout. Without a deadline the client would wait for as long as the host
/// keeps talking, so the socket waits no longer than the time the handshake
/// has left. The deadline is the budget of the source, and the socket gets its
/// own timeout back when the handshake is over.
pub(crate) fn handshake(stream: &mut TlsStream) -> Result<(), TlsError> {
    stream.sock.start_handshake();
    let result = loop {
        let TlsStream { conn, sock } = stream;
        let (read, written) = conn.complete_io(sock).map_err(map_io_error)?;
        if !conn.is_handshaking() {
            break Ok(());
        }
        if read == 0 && written == 0 {
            // The host said nothing and the handshake is not done: the
            // connection ended in the middle of it. The TLS layer reports
            // that as a failure of its own, so this is the net for a layer
            // that reports no progress at all - without it the loop would
            // spin.
            break Err(TlsError::Handshake(CLOSED_IN_THE_MIDDLE.into()));
        }
    };
    stream.sock.end_handshake();
    result
}

/// The failure of a read or a write of the TLS layer.
fn map_io_error(e: io::Error) -> TlsError {
    // The TLS layer puts its own failure into the I/O failure it reports.
    if let Some(inner) = e
        .get_ref()
        .and_then(|inner| inner.downcast_ref::<rustls::Error>())
    {
        return map_error(inner.clone());
    }
    if matches!(
        e.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        return TlsError::Timeout;
    }
    if e.kind() == io::ErrorKind::UnexpectedEof {
        // The host closed the connection in the middle of the handshake. The
        // failure names what the host did: the text of the standard library
        // ("unexpected end of file") says nothing about the handshake.
        return TlsError::Handshake(CLOSED_IN_THE_MIDDLE.into());
    }
    TlsError::Handshake(e.to_string())
}

/// The failure of the TLS layer, named.
fn map_error(e: rustls::Error) -> TlsError {
    match e {
        rustls::Error::InvalidCertificate(certificate) => {
            TlsError::Certificate(map_certificate_error(certificate))
        }
        other => TlsError::Handshake(other.to_string()),
    }
}

/// The failure of a certificate, named.
fn map_certificate_error(e: rustls::CertificateError) -> CertificateFailure {
    match e {
        rustls::CertificateError::Expired | rustls::CertificateError::ExpiredContext { .. } => {
            CertificateFailure::Expired
        }
        rustls::CertificateError::NotValidYet
        | rustls::CertificateError::NotValidYetContext { .. } => CertificateFailure::NotValidYet,
        rustls::CertificateError::Revoked => CertificateFailure::Revoked,
        rustls::CertificateError::UnknownIssuer => CertificateFailure::UnknownIssuer,
        rustls::CertificateError::BadSignature => CertificateFailure::BadSignature,
        rustls::CertificateError::NotValidForName
        | rustls::CertificateError::NotValidForNameContext { .. } => {
            CertificateFailure::NotValidForName
        }
        rustls::CertificateError::InvalidPurpose
        | rustls::CertificateError::InvalidPurposeContext { .. } => {
            CertificateFailure::InvalidPurpose
        }
        other => CertificateFailure::Other(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    #[test]
    fn the_anchors_of_the_operating_system_can_be_used() {
        // The store of the operating system is read by the operating system
        // itself, so this asks the platform for its verifier and not for a
        // certificate.
        let config = TlsAnchors::system()
            .client_config()
            .expect("the verifier of the platform");
        assert!(config.alpn_protocols.is_empty());
    }

    #[test]
    fn a_malformed_anchor_is_refused() {
        // An anchor that is not a certificate cannot be an anchor.
        let anchors = TlsAnchors::only(vec![Vec::new()]);
        assert!(
            anchors.client_config().is_err(),
            "an empty anchor is no anchor"
        );
    }

    #[test]
    fn an_empty_set_of_anchors_is_an_empty_trust_store() {
        // The named anchors are the only anchors, so an empty set of them is
        // a store that holds nothing: it trusts nothing at all. This is the
        // half of the property that no black box test can see - a store of
        // the machine refuses an unknown authority with the same reason an
        // empty store does.
        let roots = root_store(&[]).expect("the store of the anchors");
        assert!(roots.is_empty(), "an empty set of anchors trusts nothing");
    }

    #[test]
    fn the_named_anchors_are_the_anchors_of_the_store() {
        let mut params = rcgen::CertificateParams::new(vec!["127.0.0.1".to_string()])
            .expect("the parameters of the certificate");
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "an anchor");
        let key = rcgen::KeyPair::generate().expect("the key of the certificate");
        let certificate = params.self_signed(&key).expect("the certificate");
        let roots = root_store(&[certificate.der().to_vec()]).expect("the store of the anchors");
        assert_eq!(roots.len(), 1, "the named certificate is an anchor");
    }

    #[test]
    fn a_peer_that_presents_no_certificate_is_a_failure_of_the_handshake() {
        // An empty certificate chain cannot be produced with the server API
        // of `rustls` - a server that has no certificate answers with an
        // alert instead - so the mapping is pinned here rather than on the
        // wire: a chain without a certificate is a failure of the handshake.
        // It is not a refused certificate (there is none to name) and it is
        // certainly not a stream.
        match map_error(rustls::Error::NoCertificatesPresented) {
            TlsError::Handshake(reason) => {
                assert!(
                    reason.contains("no certificates"),
                    "it names the chain: {reason}"
                );
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_certificate_failure_names_itself() {
        assert_eq!(
            CertificateFailure::UnknownIssuer.to_string(),
            "it is not signed by an authority the client trusts"
        );
        assert_eq!(
            CertificateFailure::Expired.to_string(),
            "it is past the end of its validity"
        );
        assert_eq!(
            CertificateFailure::Other("it is odd".into()).to_string(),
            "it is odd"
        );
    }

    /// A socket on this machine, with the timeout of the source.
    fn local_socket(timeout: Duration) -> TcpStream {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
        let port = listener.local_addr().expect("the address").port();
        let socket = TcpStream::connect(("127.0.0.1", port)).expect("a local socket");
        socket
            .set_read_timeout(Some(timeout))
            .expect("the timeout of the source");
        socket
    }

    #[test]
    fn the_socket_keeps_the_timeout_of_the_source_after_the_handshake() {
        let mut socket = DeadlineSocket::new(local_socket(Duration::from_secs(1)));
        socket.start_handshake();
        socket.end_handshake();
        assert_eq!(
            socket.socket.read_timeout().expect("the timeout"),
            Some(Duration::from_secs(1)),
            "the reads that follow the handshake wait as long as the source says"
        );
    }

    #[test]
    fn a_read_of_the_handshake_fails_when_the_deadline_is_over() {
        let mut socket = DeadlineSocket::new(local_socket(Duration::from_secs(1)));
        socket.start_handshake();
        // The deadline is the moment of the handshake itself: no time is left.
        socket.deadline = Some(Instant::now());
        let mut buffer = [0u8; 16];
        let error = socket
            .read(&mut buffer)
            .expect_err("the read waits for a deadline that is over");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }
}
