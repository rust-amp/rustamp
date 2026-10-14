//! Error type shared by all crates.

use serde::{Deserialize, Serialize};

/// Result alias using [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors raised across the Rustamp contract boundaries.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Underlying I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// No decoder recognized the media.
    #[error("unsupported format: {0}")]
    UnsupportedFormat(String),
    /// A decoder failed while reading or seeking.
    #[error("decode error: {0}")]
    Decode(String),
    /// An output device failed.
    #[error("output error: {0}")]
    Output(String),
    /// A DSP stage rejected its configuration.
    #[error("dsp error: {0}")]
    Dsp(String),
    /// The operation is not supported (e.g. seeking in a live stream).
    #[error("unsupported operation: {0}")]
    Unsupported(&'static str),
    /// An argument was out of range or otherwise invalid.
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    /// A referenced playlist entry or other object does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// Configuration could not be read or is invalid.
    #[error("config error: {0}")]
    Config(String),
    /// A channel peer has gone away.
    #[error("channel disconnected")]
    Disconnected,
    /// Anything else.
    #[error("{0}")]
    Other(String),
}

/// Coarse classification of an [`Error`], carried in events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorKind {
    /// I/O failure.
    Io,
    /// Unsupported format.
    UnsupportedFormat,
    /// Decode failure.
    Decode,
    /// Output device failure.
    Output,
    /// Anything else.
    Other,
}

/// Serializable description of an error, used by `Event::Error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    /// Classification.
    pub kind: ErrorKind,
    /// Human-readable message.
    pub message: String,
}

impl From<&Error> for ErrorInfo {
    fn from(e: &Error) -> Self {
        let kind = match e {
            Error::Io(_) => ErrorKind::Io,
            Error::UnsupportedFormat(_) => ErrorKind::UnsupportedFormat,
            Error::Decode(_) => ErrorKind::Decode,
            Error::Output(_) => ErrorKind::Output,
            _ => ErrorKind::Other,
        };
        ErrorInfo {
            kind,
            message: e.to_string(),
        }
    }
}
