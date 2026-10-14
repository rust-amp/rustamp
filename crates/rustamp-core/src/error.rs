//! Error type shared by all crates.

use serde::{Deserialize, Serialize};

use crate::audio::AudioSpec;

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
    /// The stream changed format mid-stream (RFC 0002, C3). The decoder has
    /// already switched: [`Decoder::spec`](crate::audio::Decoder::spec) reports
    /// `to`, and `read` may be called again to continue.
    #[error("stream format changed: {from:?} -> {to:?}")]
    FormatChanged {
        /// Format before the change.
        from: AudioSpec,
        /// Format after the change.
        to: AudioSpec,
    },
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
    /// The stream changed format mid-stream (RFC 0002, C4). Appended at the end
    /// so existing serialized values keep their index.
    FormatChanged,
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
            Error::FormatChanged { .. } => ErrorKind::FormatChanged,
            _ => ErrorKind::Other,
        };
        ErrorInfo {
            kind,
            message: e.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_changed_keeps_its_kind_and_specs() {
        // RFC 0002, C3/C4: the engine can tell a format change from a decode
        // failure and can name both formats.
        let e = Error::FormatChanged {
            from: AudioSpec::new(44100, 2),
            to: AudioSpec::new(48000, 1),
        };
        let info = ErrorInfo::from(&e);
        assert_eq!(info.kind, ErrorKind::FormatChanged);
        assert!(info.message.contains("48000"), "{}", info.message);
        assert!(info.message.contains("44100"), "{}", info.message);
    }
}
