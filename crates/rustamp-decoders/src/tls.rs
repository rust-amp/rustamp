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

use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_platform_verifier::BuilderVerifierExt;

/// The stream of a TLS connection: the TLS layer over a socket.
pub(crate) type TlsStream = StreamOwned<ClientConnection, TcpStream>;

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
                let mut roots = RootCertStore::empty();
                for certificate in certificates {
                    roots
                        .add(CertificateDer::from(certificate.clone()))
                        .map_err(|e| TlsError::Setup(e.to_string()))?;
                }
                Ok(builder.with_root_certificates(roots).with_no_client_auth())
            }
        }
    }
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
    Ok(StreamOwned::new(connection, socket))
}

/// The handshake of a TLS stream.
///
/// The handshake belongs to the connection: it happens while the source says
/// that it is reaching the host, and a failure of it is a failure to connect.
pub(crate) fn handshake(stream: &mut TlsStream) -> Result<(), TlsError> {
    loop {
        let TlsStream { conn, sock } = stream;
        let (read, written) = conn.complete_io(sock).map_err(map_io_error)?;
        if !conn.is_handshaking() {
            return Ok(());
        }
        if read == 0 && written == 0 {
            // The host said nothing and the handshake is not done: the
            // connection ended in the middle of it.
            return Err(TlsError::Handshake(
                "the host closed the connection before the handshake was done".into(),
            ));
        }
    }
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
}
