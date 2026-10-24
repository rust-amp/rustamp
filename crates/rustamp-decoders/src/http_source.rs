//! The transport of an `http://` or an `https://` stream: the URL, the request
//! of S19 §1 and the body of the response.
//!
//! # What the spec settles
//!
//! S19 §1 carries the request template of the reference player. This module
//! sends exactly that shape:
//!
//! ```text
//! GET http://<host>:<port>/<path> HTTP/1.0
//! Host: <host>:<port>
//! User-Agent: Rustamp/<version>
//! Accept: */*
//! ```
//!
//! * The request line of the `http://` request carries the **absolute** form
//!   - scheme, host, port and path in one URI - and names **HTTP/1.0**.
//! * The head carries a `Host:` line, a `User-Agent:` line and an
//!   `Accept: */*` line, in that order.
//! * The template has one slot for extra header lines between the
//!   `User-Agent` line and the `Accept` line, and what that slot carries is
//!   open (S19 §6, question 9), so this source sends **no** extra header.
//! * There is no `Connection:` line, and HTTP/1.0 is not a keep-alive
//!   protocol: the connection is closed at the end of the body.
//!
//! # What the caller can observe
//!
//! The source writes what it is doing into an [`HttpProgress`]: the phase of
//! the exchange - the phases of S19 §2 - and the counters of the body. The
//! texts of the spec are the application's; this crate shows none of them.
//! See [`crate::http_progress`] for the shape and for what the phases are.
//!
//! # What the spec leaves open, and what this module decides
//!
//! * The **timeout** (S19 §6, questions 1 and 14): [`DEFAULT_TIMEOUT`].
//! * The **status codes** (S19 §6, question 4): only `200` is a body. Every
//!   other status is an error that names the status; the spec carries no
//!   text for it.
//! * The **schemes** (S19 §6, question 8): `http://` as the spec's template
//!   has it, and `https://` as **our own extension** (S19 establishes no
//!   evidence that the reference reads a TLS stream at all; see the section
//!   below). Every other scheme is refused by name.
//! * The **`User-Agent`**. S19 §1 records that the *reference* sends
//!   `Winamp/<version>` and leaves the string open. **We identify as Rustamp**
//!   (owner decision, 2026-10-20): our software is not the reference, and a
//!   server should see whom it is talking to. The line carries this build's own
//!   package version, so it never claims a version we do not have.
//! * The **`Host:` line** (S19 §6, question 10): it carries the authority of
//!   the request line - the host and the port - because that is what the
//!   request line itself carries (RFC 7230 §5.4).
//!
//! # `https://` is an extension of this implementation
//!
//! S19 establishes **no evidence** that the reference player reads a TLS
//! stream: the template of §1 is plain `http://`, and the spec settles no
//! question about a secure stream. Reading an `https://` URL is therefore
//! **our own extension**, and everything about it is our own decision:
//!
//! * The **port** of an `https://` URL that names none is 443.
//! * The **request line** of an `https://` stream carries the **origin form**
//!   (`GET /<path> HTTP/1.0`) and not the absolute form of the template. The
//!   absolute form is the form of a request to a proxy (RFC 7230 §5.3.1);
//!   this source connects straight to the origin, and the origin form is the
//!   request target of such a request.
//! * The **`Host:` line** of an `https://` stream names the port only when it
//!   is not the default port of the scheme (RFC 7230 §5.4). The `http://`
//!   request keeps the line of the template, which names the port always.
//! * The **certificate** of the host is checked against the trust store of
//!   the operating system. A refused certificate is a failure that names
//!   itself, never a plain connection (see [`crate::tls`]).
//! * The **handshake** belongs to the connection: the phase that says that
//!   the host is being reached covers it (see [`crate::http_progress`]).
//!
//! Every other scheme - `ftp://` and the rest - is refused by name, as
//! before.
//!
//! Nothing in this module was taken from a Winamp source, from SDK material
//! or from a disassembly: the request shape comes from S19 §1 and everything
//! else is a decision of this implementation.

use std::fmt;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;

use rustamp_core::error::Error as CoreError;

use crate::http_progress::{HttpPhase, HttpProgress};
use crate::tls::{self, CertificateFailure, TlsAnchors, TlsError};

/// The time the source waits for the host.
///
/// S19 §6 (questions 1 and 14) records that the reference's string table
/// carries no timeout text, so neither the existence nor the value of a
/// timeout is established there. This is the choice of this implementation,
/// and it is one value for the whole exchange:
///
/// * it bounds the connect, so a host that does not answer cannot hang the
///   player;
/// * it bounds the read of the response head, so a host that accepts the
///   connection and then says nothing cannot hang the player either;
/// * it bounds the gap between two reads of the body, because the core
///   contract says that a `read` "must not block indefinitely on live streams
///   without returning data or an error"
///   (`rustamp_core::audio::Decoder`).
///
/// Ten seconds is long enough for one round trip to a distant or a loaded
/// server and short enough that a dead host reads as a failure instead of a
/// hang. It is **not** the reference's value: nothing in S19 establishes one.
///
/// The name resolution is **not** covered by it: `std` has no bound for a
/// name lookup, so a host whose resolver hangs hangs the source too.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// The `User-Agent` this source sends.
///
/// S19 §1 records that the **reference** sends `Winamp/<version>`; **we identify
/// as Rustamp** (owner decision, 2026-10-20). Our software is not the reference,
/// and a server should see whom it is talking to. The version is this build's own
/// package version, so the line never claims a version we do not have.
const USER_AGENT: &str = concat!("Rustamp/", env!("CARGO_PKG_VERSION"));

/// The largest response head this source reads.
///
/// A head that grows past it is a server that does not answer with a head:
/// the source refuses it instead of filling its memory with it.
const MAX_HEAD_LEN: usize = 64 * 1024;

/// The failures of the HTTP source.
///
/// The core error type has no variant for "the server answered with an
/// error status", and adding one is a `rustamp-core` contract change (an
/// RFC). The failures of this source are therefore typed here, and they carry
/// the facts a caller needs - the status, the scheme, the phase that timed
/// out - instead of a message alone. [`HttpError`] converts into the core
/// error (see the `From` implementation), so a failure can travel through the
/// engine's error path.
#[derive(Debug)]
pub enum HttpError {
    /// The URL names a scheme this source cannot read.
    UnsupportedScheme {
        /// The scheme of the URL, lower case.
        scheme: String,
    },
    /// The URL is not a usable absolute `http://` or `https://` URL.
    InvalidUrl(String),
    /// The host name could not be resolved.
    Resolve {
        /// The host that could not be resolved.
        host: String,
        /// The failure of the resolver.
        source: io::Error,
    },
    /// The connection could not be established.
    Connect {
        /// The host that could not be reached.
        host: String,
        /// The failure of the connect.
        source: io::Error,
    },
    /// The host did not answer in time.
    Timeout {
        /// What the source was waiting for, in words.
        phase: &'static str,
    },
    /// The server answered with a status other than 200.
    Status {
        /// The status code of the answer.
        code: u16,
        /// The reason phrase of the answer.
        reason: String,
    },
    /// The answer was not a response this source can read.
    Protocol(String),
    /// The TLS handshake refused the certificate of the host.
    ///
    /// A refused certificate is a failure of its own: the source does not
    /// fall back to a plain connection and does not read the stream without a
    /// certificate it trusts.
    Certificate {
        /// Why the certificate was refused.
        reason: CertificateFailure,
    },
    /// The TLS connection failed: the handshake, the anchors or the name of
    /// the host were not usable.
    Tls {
        /// What the TLS layer said.
        reason: String,
    },
    /// The body is not media this crate can decode.
    ///
    /// The failure of the decoder is the core error it raised, unchanged.
    Media(CoreError),
    /// The connection failed while the body was read.
    Io(io::Error),
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedScheme { scheme } => write!(
                f,
                "unsupported URL scheme {scheme:?}: this source reads http:// and https:// URLs only"
            ),
            Self::InvalidUrl(message) => {
                write!(f, "not a usable http:// or https:// URL: {message}")
            }
            Self::Resolve { host, source } => {
                write!(f, "cannot resolve the host {host:?}: {source}")
            }
            Self::Connect { host, source } => {
                write!(f, "cannot connect to the host {host:?}: {source}")
            }
            Self::Timeout { phase } => {
                write!(f, "the host did not answer in time while {phase}")
            }
            Self::Status { code, reason } => {
                write!(f, "the server answered {code} {reason}")
            }
            Self::Protocol(message) => {
                write!(f, "not a usable HTTP response: {message}")
            }
            Self::Certificate { reason } => {
                write!(f, "the certificate of the host was refused: {reason}")
            }
            Self::Tls { reason } => {
                write!(f, "the TLS connection failed: {reason}")
            }
            Self::Media(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for HttpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Resolve { source, .. } | Self::Connect { source, .. } | Self::Io(source) => {
                Some(source)
            }
            Self::Media(e) => Some(e),
            _ => None,
        }
    }
}

impl From<HttpError> for CoreError {
    /// The core error a failure of this source becomes.
    ///
    /// The core type has no variant for an HTTP status, so the failures that
    /// are not I/O failures keep their own text in [`CoreError::Other`]: the
    /// message names the status, the scheme or the phase, so the user sees
    /// what happened. The failure of the decoder keeps its own kind, because
    /// it is a decode failure and not a transport failure.
    fn from(e: HttpError) -> Self {
        match e {
            HttpError::Media(e) => e,
            HttpError::Io(e) => CoreError::Io(e),
            other => CoreError::Other(other.to_string()),
        }
    }
}

impl From<TlsError> for HttpError {
    /// The failure of the TLS layer as a failure of this source.
    ///
    /// The certificate keeps its own kind: a refused certificate is a
    /// failure a caller can tell apart from a handshake that broke for
    /// another reason.
    fn from(e: TlsError) -> Self {
        match e {
            TlsError::Certificate(reason) => HttpError::Certificate { reason },
            TlsError::Timeout => HttpError::Timeout {
                phase: "the TLS handshake",
            },
            TlsError::Handshake(reason) => HttpError::Tls { reason },
            TlsError::Setup(reason) => HttpError::Tls { reason },
        }
    }
}

// ---------------------------------------------------------------------------
// URL
// ---------------------------------------------------------------------------

/// The scheme of a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scheme {
    /// `http://`: a plain socket.
    Http,
    /// `https://`: a TLS stream.
    Https,
}

impl Scheme {
    /// The port of the scheme, for a URL that names none.
    const fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }
}

/// The parts of an `http://` or `https://` URL that the request needs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Url {
    /// The scheme of the URL.
    scheme: Scheme,
    /// The host, without the brackets of an IPv6 literal.
    host: String,
    /// Whether the host is an IPv6 literal. It keeps its brackets in the
    /// request line and in the `Host:` line, where an address literal needs
    /// them.
    ipv6: bool,
    /// The port; the default port of the scheme when the URL names none.
    port: u16,
    /// The path and the query: the request target behind the authority.
    target: String,
}

impl Url {
    /// The authority of the URL: the host and the port.
    ///
    /// It is what the request line and the `Host:` line carry (RFC 7230
    /// §5.4: the `Host:` field carries the authority of the request target).
    fn authority(&self) -> String {
        if self.ipv6 {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// The host of the URL, with the brackets of an IPv6 literal.
    fn host_only(&self) -> String {
        if self.ipv6 {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        }
    }

    /// The authority of the `Host:` line.
    ///
    /// The `http://` request carries the line of the template of S19 §1, and
    /// that line names the port always. The `https://` request is our own:
    /// it names the port only when it is not the default port of the scheme
    /// (RFC 7230 §5.4).
    fn host_authority(&self) -> String {
        if self.scheme == Scheme::Https && self.port == Scheme::Https.default_port() {
            self.host_only()
        } else {
            self.authority()
        }
    }

    /// The request target of the request line.
    ///
    /// The `http://` request carries the absolute form of the template of
    /// S19 §1. The `https://` request is our own and connects straight to the
    /// origin: the origin form is the request target of such a request, and
    /// the absolute form is the form of a request to a proxy (RFC 7230
    /// §5.3.1).
    fn request_target(&self) -> String {
        match self.scheme {
            Scheme::Http => format!("http://{}{}", self.authority(), self.target),
            Scheme::Https => self.target.clone(),
        }
    }
}

/// Reads an `http://` or an `https://` URL.
///
/// The scheme decides first: only `http` and `https` are read, and every
/// other scheme is refused by name (S19 §6, question 8).
fn parse_url(url: &str) -> Result<Url, HttpError> {
    let Some((scheme, rest)) = url.split_once("://") else {
        return Err(HttpError::InvalidUrl("the URL names no scheme".into()));
    };
    let scheme = if scheme.eq_ignore_ascii_case("http") {
        Scheme::Http
    } else if scheme.eq_ignore_ascii_case("https") {
        Scheme::Https
    } else {
        return Err(HttpError::UnsupportedScheme {
            scheme: scheme.to_ascii_lowercase(),
        });
    };
    let (authority, path) = match rest.find(['/', '?', '#']) {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    };
    if authority.contains('@') {
        return Err(HttpError::InvalidUrl(
            "the URL carries credentials, which this source does not send".into(),
        ));
    }
    let (host, ipv6, port) = split_authority(authority, scheme)?;
    // The fragment belongs to the client, not to the request.
    let path = path.split('#').next().unwrap_or("");
    let target = if path.is_empty() {
        "/".to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    Ok(Url {
        scheme,
        host,
        ipv6,
        port,
        target,
    })
}

/// The host and the port of an authority.
fn split_authority(authority: &str, scheme: Scheme) -> Result<(String, bool, u16), HttpError> {
    let (host, ipv6, port) = if let Some(rest) = authority.strip_prefix('[') {
        // An IPv6 literal: "[::1]" or "[::1]:8080".
        let Some(end) = rest.find(']') else {
            return Err(HttpError::InvalidUrl(
                "the IPv6 literal of the URL is not closed".into(),
            ));
        };
        let after = &rest[end + 1..];
        let port = match after.strip_prefix(':') {
            Some(port) => parse_port(port)?,
            None if after.is_empty() => scheme.default_port(),
            None => {
                return Err(HttpError::InvalidUrl(
                    "the URL carries something behind its IPv6 literal".into(),
                ));
            }
        };
        (rest[..end].to_string(), true, port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_string(), false, parse_port(port)?),
            None => (authority.to_string(), false, scheme.default_port()),
        }
    };
    if host.is_empty() {
        return Err(HttpError::InvalidUrl("the URL names no host".into()));
    }
    Ok((host, ipv6, port))
}

/// The port of an authority.
fn parse_port(port: &str) -> Result<u16, HttpError> {
    port.parse::<u16>()
        .map_err(|_| HttpError::InvalidUrl(format!("the port {port:?} is not a port number")))
}

// ---------------------------------------------------------------------------
// Request
// ---------------------------------------------------------------------------

/// The request bytes of S19 §1.
///
/// The lines are separated by CRLF, as HTTP requires; the template of the
/// spec shows the lines, not their line ending. The `https://` request is our
/// own and carries the origin form; see the module docs.
fn request_bytes(url: &Url) -> Vec<u8> {
    let mut request = Vec::with_capacity(160);
    request.extend_from_slice(b"GET ");
    request.extend_from_slice(url.request_target().as_bytes());
    request.extend_from_slice(b" HTTP/1.0\r\n");
    request.extend_from_slice(b"Host: ");
    request.extend_from_slice(url.host_authority().as_bytes());
    request.extend_from_slice(b"\r\n");
    request.extend_from_slice(b"User-Agent: ");
    request.extend_from_slice(USER_AGENT.as_bytes());
    request.extend_from_slice(b"\r\n");
    request.extend_from_slice(b"Accept: */*\r\n");
    request.extend_from_slice(b"\r\n");
    request
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

/// The transport under an HTTP exchange: a plain socket or a TLS stream.
enum Transport {
    /// A plain socket: the transport of an `http://` stream.
    Plain(TcpStream),
    /// A TLS stream: the transport of an `https://` stream.
    Tls(Box<tls::TlsStream>),
}

impl Read for Transport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(socket) => socket.read(buf),
            Self::Tls(stream) => stream.read(buf),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(socket) => socket.write(buf),
            Self::Tls(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(socket) => socket.flush(),
            Self::Tls(stream) => stream.flush(),
        }
    }
}

// ---------------------------------------------------------------------------
// Response
// ---------------------------------------------------------------------------

/// The head of the response: the status line and the header lines.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Head {
    /// The status code of the answer.
    code: u16,
    /// The reason phrase of the answer.
    reason: String,
    /// The length of the body, when the server declared one.
    length: Option<u64>,
}

/// Reads the head of the response.
///
/// The bytes that came with the head are returned with it: a server that
/// sends the head and the first bytes of the body in one segment must not
/// lose them.
fn read_head(socket: &mut Transport) -> Result<(Head, Vec<u8>), HttpError> {
    let mut buffer: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = read_socket(socket, &mut chunk)
            .map_err(|e| map_read_error(e, "reading the response"))?;
        if n == 0 {
            return Err(HttpError::Protocol(
                "the server closed the connection before it answered".into(),
            ));
        }
        buffer.extend_from_slice(&chunk[..n]);
        if let Some(end) = head_end(&buffer) {
            let text = std::str::from_utf8(&buffer[..end])
                .map_err(|_| HttpError::Protocol("the head of the response is not text".into()))?;
            let head = parse_head(text)?;
            let rest = buffer[end + 4..].to_vec();
            return Ok((head, rest));
        }
        if buffer.len() > MAX_HEAD_LEN {
            return Err(HttpError::Protocol(format!(
                "the head of the response is longer than {MAX_HEAD_LEN} bytes"
            )));
        }
    }
}

/// The end of the head: the CRLFCRLF that closes it.
fn head_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

/// Reads the status line and the header lines of a response.
fn parse_head(text: &str) -> Result<Head, HttpError> {
    let mut lines = text.split("\r\n");
    let status = lines.next().unwrap_or_default();
    let mut parts = status.splitn(3, ' ');
    let version = parts.next().unwrap_or_default();
    if !version.starts_with("HTTP/") {
        return Err(HttpError::Protocol(format!(
            "the status line {status:?} is not one"
        )));
    }
    let code = parts
        .next()
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| {
            HttpError::Protocol(format!("the status line {status:?} carries no status code"))
        })?;
    let reason = parts.next().unwrap_or_default().trim().to_string();

    let mut length = None;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(HttpError::Protocol(format!(
                "the header line {line:?} carries no name"
            )));
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                length = Some(value.parse::<u64>().map_err(|_| {
                    HttpError::Protocol(format!("the Content-Length {value:?} is not a number"))
                })?);
            }
            "transfer-encoding" if !value.eq_ignore_ascii_case("identity") => {
                // The body of this source is the body of the response, not a
                // transfer-coded one: a chunked or a compressed body is
                // refused by name instead of being read as if it were plain.
                // A response to an HTTP/1.0 request is not chunked (S19 §1).
                return Err(HttpError::Protocol(format!(
                    "the server answered with the transfer encoding {value:?}, which this source does not read"
                )));
            }
            _ => {}
        }
    }
    Ok(Head {
        code,
        reason,
        length,
    })
}

// ---------------------------------------------------------------------------
// Body
// ---------------------------------------------------------------------------

/// The body of the response: the bytes the server sends after the head.
pub(crate) struct Body {
    /// The transport the body comes from.
    transport: Transport,
    /// The bytes that came with the head: the server may send them and the
    /// first bytes of the body in one segment.
    leftover: Vec<u8>,
    /// What the body, the decoder and the application share: the phase of
    /// the transfer and the counters of the body.
    progress: HttpProgress,
    /// The body is over.
    done: bool,
}

impl Body {
    /// The body of the answer, counted by `progress`.
    fn new(transport: Transport, progress: HttpProgress, leftover: Vec<u8>) -> Self {
        Body {
            transport,
            leftover,
            progress,
            done: false,
        }
    }

    /// The bytes the server declared.
    fn declared(&self) -> Option<u64> {
        self.progress.total()
    }

    /// Notes that the connection died before the declared length was there.
    ///
    /// Without a declared length there is no oracle: a body that ends
    /// cannot be told from a body that was cut.
    fn note_cut(&self) {
        let Some(declared) = self.progress.total() else {
            return;
        };
        if self.progress.bytes_read() < declared {
            self.progress.mark_cut();
        }
    }
}

impl Read for Body {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        // Never read past the length the server declared: the bytes behind it
        // are not part of this body.
        let limit = match self.progress.total() {
            Some(declared) => {
                let received = self.progress.bytes_read();
                if received >= declared {
                    // The body is where the server said it ends: the server
                    // closed the connection at the end of it.
                    self.done = true;
                    self.progress.set_phase(HttpPhase::Closed);
                    return Ok(0);
                }
                let left = declared - received;
                buf.len().min(usize::try_from(left).unwrap_or(usize::MAX))
            }
            None => buf.len(),
        };
        if !self.leftover.is_empty() {
            // The bytes that came with the head are the beginning of the
            // body: they count towards the declared length like any other.
            let n = self.leftover.len().min(limit);
            buf[..n].copy_from_slice(&self.leftover[..n]);
            self.leftover.drain(..n);
            self.progress.add_bytes(n as u64);
            return Ok(n);
        }
        match self.transport.read(&mut buf[..limit]) {
            Ok(0) => {
                self.done = true;
                // The connection ended. With a declared length that is a cut:
                // the server promised bytes that did not arrive. Without one
                // it is the end of the body, which is how a response without
                // a `Content-Length` ends.
                self.note_cut();
                if self.progress.total().is_some() {
                    // The host closed the connection before the body it
                    // promised was there: the connection was lost.
                    self.progress.set_phase(HttpPhase::ConnectionLost);
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        "the connection closed before the body was complete",
                    ));
                }
                self.progress.set_phase(HttpPhase::Closed);
                Ok(0)
            }
            Ok(n) => {
                self.progress.add_bytes(n as u64);
                Ok(n)
            }
            Err(e) => {
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) {
                    // A timeout is not a lost connection: the host is slow,
                    // not gone, and S19 carries no text for it (§6,
                    // questions 1 and 14). The phase stays where it is.
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the stream delivered nothing for the whole timeout",
                    ));
                }
                self.note_cut();
                self.progress.set_phase(HttpPhase::ConnectionLost);
                Err(e)
            }
        }
    }
}

/// The symphonia view of the body: a stream that cannot be seeked.
pub(crate) struct BodySource {
    /// The body behind it.
    body: Body,
}

impl Read for BodySource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.body.read(buf)
    }
}

impl Seek for BodySource {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "an HTTP stream cannot be seeked",
        ))
    }
}

impl symphonia::core::io::MediaSource for BodySource {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        self.body.declared()
    }
}

// ---------------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------------

/// The HTTP source: a connection, a request and the body of the response.
pub(crate) struct HttpSource {
    /// The body of the response.
    body: Body,
}

impl HttpSource {
    /// Connects to `url`, sends the request of S19 §1 and reads the head of
    /// the answer.
    ///
    /// A status other than 200 is a failure that names the status: the spec
    /// carries no text for it (S19 §6, question 4), so the source says what
    /// the server answered and stops there - the body of an error answer is
    /// not media.
    ///
    /// The certificate of an `https://` host is checked against `anchors`:
    /// the trust store of the operating system, or the anchors the caller
    /// names.
    ///
    /// The transfer writes what it is doing into `progress`: the phase of
    /// the exchange and the counters of the body.
    pub(crate) fn open(
        url: &str,
        timeout: Duration,
        progress: &HttpProgress,
        anchors: &TlsAnchors,
    ) -> Result<Self, HttpError> {
        progress.set_phase(HttpPhase::Connecting);
        match Self::exchange(url, timeout, progress, anchors) {
            Ok(source) => Ok(source),
            Err(e) => {
                // A failure before the connection is up is a failure to
                // connect: the host could not be reached.
                if progress.phase() == HttpPhase::Connecting {
                    progress.set_phase(HttpPhase::ConnectFailed);
                }
                Err(e)
            }
        }
    }

    /// The exchange itself: the connect, the request and the head.
    fn exchange(
        url: &str,
        timeout: Duration,
        progress: &HttpProgress,
        anchors: &TlsAnchors,
    ) -> Result<Self, HttpError> {
        let url = parse_url(url)?;
        let transport = Self::send_request(&url, timeout, progress, anchors)?;
        Self::read_response(transport, progress)
    }

    /// Connects to the host and sends the request of S19 §1.
    ///
    /// The phase is [`HttpPhase::SendingRequest`] from the moment the
    /// connection is up until the answer is read. The handshake of an
    /// `https://` stream belongs to the connection and happens before that:
    /// the phase says that the host is being reached while it runs.
    fn send_request(
        url: &Url,
        timeout: Duration,
        progress: &HttpProgress,
        anchors: &TlsAnchors,
    ) -> Result<Transport, HttpError> {
        let socket = connect(url, timeout)?;
        socket
            .set_read_timeout(Some(timeout))
            .map_err(HttpError::Io)?;
        socket
            .set_write_timeout(Some(timeout))
            .map_err(HttpError::Io)?;
        let mut transport = match url.scheme {
            Scheme::Http => Transport::Plain(socket),
            Scheme::Https => {
                let mut stream = tls::client_stream(&url.host, anchors, socket)?;
                tls::handshake(&mut stream)?;
                Transport::Tls(Box::new(stream))
            }
        };
        progress.set_phase(HttpPhase::SendingRequest);
        transport
            .write_all(&request_bytes(url))
            .map_err(|e| map_write_error(e, "sending the request"))?;
        // The request is on the wire when the flush is done: a TLS stream
        // keeps the bytes in its own buffer until then.
        transport
            .flush()
            .map_err(|e| map_write_error(e, "sending the request"))?;
        Ok(transport)
    }

    /// Reads the head of the answer and prepares its body.
    ///
    /// The phase is [`HttpPhase::ReadingResponse`] while the head is read
    /// and [`HttpPhase::Retrieving`] from the moment the head is there.
    fn read_response(mut transport: Transport, progress: &HttpProgress) -> Result<Self, HttpError> {
        progress.set_phase(HttpPhase::ReadingResponse);
        let (head, leftover) = read_head(&mut transport)?;
        if head.code != 200 {
            return Err(HttpError::Status {
                code: head.code,
                reason: head.reason,
            });
        }
        // The length the server declared is the total of the transfer: it is
        // there from now on, and there is none when the server declared no
        // length.
        progress.set_total(head.length);
        progress.set_phase(HttpPhase::Retrieving);
        Ok(HttpSource {
            body: Body::new(transport, progress.clone(), leftover),
        })
    }

    /// The body of the response, as symphonia sees it.
    pub(crate) fn into_source(self) -> BodySource {
        BodySource { body: self.body }
    }
}

/// Connects to the host of the URL.
///
/// Every address the host resolves to is tried in turn, with the timeout of
/// the source: a host that has several addresses and one of them is dead is
/// still reachable.
fn connect(url: &Url, timeout: Duration) -> Result<TcpStream, HttpError> {
    let host = url.host.clone();
    let addresses: Vec<SocketAddr> = (host.as_str(), url.port)
        .to_socket_addrs()
        .map_err(|e| HttpError::Resolve {
            host: host.clone(),
            source: e,
        })?
        .collect();
    if addresses.is_empty() {
        return Err(HttpError::Resolve {
            host,
            source: io::Error::new(io::ErrorKind::NotFound, "the host has no address"),
        });
    }
    let mut last = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(socket) => return Ok(socket),
            Err(e) => last = Some(e),
        }
    }
    Err(HttpError::Connect {
        host,
        source: last.unwrap_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "the host has no address to connect to",
            )
        }),
    })
}

/// Reads from the transport, turning a timeout into the timeout of the source.
fn read_socket(socket: &mut Transport, buf: &mut [u8]) -> io::Result<usize> {
    match socket.read(buf) {
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
            ) =>
        {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the read timed out",
            ))
        }
        other => other,
    }
}

/// The failure of a read that timed out.
fn map_read_error(e: io::Error, phase: &'static str) -> HttpError {
    if matches!(
        e.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        HttpError::Timeout { phase }
    } else {
        HttpError::Io(e)
    }
}

/// The failure of a write that timed out.
fn map_write_error(e: io::Error, phase: &'static str) -> HttpError {
    if matches!(
        e.kind(),
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
    ) {
        HttpError::Timeout { phase }
    } else {
        HttpError::Io(e)
    }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    #[test]
    fn an_http_url_is_split_into_its_parts() {
        let url = parse_url("http://example.com/music/song.mp3").expect("a URL");
        assert_eq!(url.host, "example.com");
        assert!(!url.ipv6);
        assert_eq!(url.port, 80);
        assert_eq!(url.target, "/music/song.mp3");
        assert_eq!(url.authority(), "example.com:80");
        assert_eq!(url.request_target(), "http://example.com:80/music/song.mp3");
    }

    #[test]
    fn a_url_without_a_path_names_the_root() {
        let url = parse_url("http://example.com").expect("a URL");
        assert_eq!(url.target, "/");
        let url = parse_url("http://example.com?x=1").expect("a URL");
        assert_eq!(url.target, "/?x=1");
    }

    #[test]
    fn the_query_goes_with_the_path_and_the_fragment_does_not() {
        let url = parse_url("http://example.com/a.mp3?x=1&y=2#anchor").expect("a URL");
        assert_eq!(url.target, "/a.mp3?x=1&y=2");
    }

    #[test]
    fn a_port_is_read_when_the_url_names_one() {
        let url = parse_url("http://127.0.0.1:8080/stream").expect("a URL");
        assert_eq!(url.host, "127.0.0.1");
        assert_eq!(url.port, 8080);
        assert_eq!(url.authority(), "127.0.0.1:8080");
    }

    #[test]
    fn an_ipv6_literal_keeps_its_brackets_in_the_authority() {
        let url = parse_url("http://[::1]:8080/stream").expect("a URL");
        assert_eq!(url.host, "::1");
        assert!(url.ipv6);
        assert_eq!(url.port, 8080);
        assert_eq!(url.authority(), "[::1]:8080");
    }

    #[test]
    fn another_scheme_is_refused_by_name() {
        let err = parse_url("ftp://example.com/a.mp3").expect_err("ftp is not read");
        match err {
            HttpError::UnsupportedScheme { scheme } => assert_eq!(scheme, "ftp"),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn an_https_url_names_the_default_port_of_its_scheme() {
        let url = parse_url("https://example.com/music/song.mp3").expect("a URL");
        assert_eq!(url.scheme, Scheme::Https);
        assert_eq!(url.host, "example.com");
        assert_eq!(url.port, 443);
        assert_eq!(url.target, "/music/song.mp3");
    }

    #[test]
    fn an_https_request_carries_the_origin_form() {
        let url = parse_url("https://example.com/music/song.mp3?x=1").expect("a URL");
        let request = String::from_utf8(request_bytes(&url)).expect("text");
        assert_eq!(
            request,
            format!(
                "GET /music/song.mp3?x=1 HTTP/1.0\r\n\
                 Host: example.com\r\n\
                 User-Agent: {USER_AGENT}\r\n\
                 Accept: */*\r\n\
                 \r\n"
            )
        );
    }

    #[test]
    fn the_host_line_of_an_https_request_names_a_port_that_is_not_the_default() {
        let url = parse_url("https://example.com:8443/music/song.mp3").expect("a URL");
        assert_eq!(url.host_authority(), "example.com:8443");
        let url = parse_url("https://example.com/music/song.mp3").expect("a URL");
        assert_eq!(url.host_authority(), "example.com");
    }

    #[test]
    fn the_host_line_of_an_http_request_names_the_port_always() {
        // The request of S19 §1 names the port in the `Host:` line, and the
        // `http://` request keeps that line.
        let url = parse_url("http://example.com/music/song.mp3").expect("a URL");
        assert_eq!(url.host_authority(), "example.com:80");
    }

    #[test]
    fn a_url_without_a_scheme_is_not_a_url() {
        assert!(matches!(
            parse_url("example.com/a.mp3"),
            Err(HttpError::InvalidUrl(_))
        ));
    }

    #[test]
    fn credentials_are_refused() {
        assert!(matches!(
            parse_url("http://user:pass@example.com/a.mp3"),
            Err(HttpError::InvalidUrl(_))
        ));
    }

    #[test]
    fn the_request_is_the_one_the_template_describes() {
        let url = parse_url("http://example.com:8080/music/song.mp3").expect("a URL");
        let request = String::from_utf8(request_bytes(&url)).expect("text");
        assert_eq!(
            request,
            format!(
                "GET http://example.com:8080/music/song.mp3 HTTP/1.0\r\n\
                 Host: example.com:8080\r\n\
                 User-Agent: {USER_AGENT}\r\n\
                 Accept: */*\r\n\
                 \r\n"
            )
        );
    }

    /// The request is sent before the answer is read: the phase says that
    /// the request is on its way while it is.
    ///
    /// The window is too short to be caught from the outside - a request of
    /// a hundred bytes is written in a moment - so the step that sends it
    /// is driven here.
    #[test]
    fn the_request_is_sent_before_the_answer_is_read() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
        let port = listener.local_addr().expect("the address").port();
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("a connection");
            let mut sink = [0u8; 1024];
            let _ = socket.read(&mut sink);
        });
        let url = parse_url(&format!("http://127.0.0.1:{port}/stream.mp3")).expect("a URL");
        let progress = HttpProgress::new();
        let transport = HttpSource::send_request(
            &url,
            Duration::from_secs(5),
            &progress,
            &TlsAnchors::system(),
        )
        .expect("the request");
        assert_eq!(
            progress.snapshot().phase,
            HttpPhase::SendingRequest,
            "the request is on its way"
        );
        drop(transport);
        server.join().expect("the server");
    }

    #[test]
    fn the_head_of_a_response_is_read() {
        let head = parse_head("HTTP/1.0 200 OK\r\nContent-Length: 42\r\n\r\n").expect("a head");
        assert_eq!(head.code, 200);
        assert_eq!(head.reason, "OK");
        assert_eq!(head.length, Some(42));
    }

    #[test]
    fn a_response_without_a_length_states_none() {
        let head =
            parse_head("HTTP/1.0 200 OK\r\nContent-Type: audio/mpeg\r\n\r\n").expect("a head");
        assert_eq!(head.length, None);
    }

    #[test]
    fn a_status_line_without_a_code_is_refused() {
        assert!(matches!(
            parse_head("HTTP/1.0\r\n\r\n"),
            Err(HttpError::Protocol(_))
        ));
        assert!(matches!(
            parse_head("not a status line\r\n\r\n"),
            Err(HttpError::Protocol(_))
        ));
    }

    #[test]
    fn a_chunked_body_is_refused() {
        assert!(matches!(
            parse_head("HTTP/1.0 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"),
            Err(HttpError::Protocol(_))
        ));
        assert!(matches!(
            parse_head("HTTP/1.0 200 OK\r\nTransfer-Encoding: gzip\r\n\r\n"),
            Err(HttpError::Protocol(_))
        ));
    }

    #[test]
    fn a_header_line_without_a_name_is_refused() {
        assert!(matches!(
            parse_head("HTTP/1.0 200 OK\r\nnonsense\r\n\r\n"),
            Err(HttpError::Protocol(_))
        ));
    }
}
