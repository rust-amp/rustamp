//! Error type shared by all crates.
//!
//! [`Error`] and [`Result`] stay here; the serializable wire description
//! ([`ErrorInfo`]/[`ErrorKind`]) lives in `rustamp-proto` (RFC 0004) and is
//! re-exported at its original paths. The conversion from [`Error`] to
//! [`ErrorInfo`] stays with [`Error`], which owns the classification.

use crate::audio::AudioSpec;

pub use rustamp_proto::error::{ErrorInfo, ErrorKind};

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
    /// A DSP stage failed: refused configuration or a mid-run fault (RFC 0014).
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

impl From<&Error> for ErrorInfo {
    fn from(e: &Error) -> Self {
        let kind = match e {
            Error::Io(_) => ErrorKind::Io,
            Error::UnsupportedFormat(_) => ErrorKind::UnsupportedFormat,
            Error::Decode(_) => ErrorKind::Decode,
            Error::Output(_) => ErrorKind::Output,
            Error::Dsp(_) => ErrorKind::Dsp,
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
    fn dsp_faults_keep_their_kind_and_message() {
        // RFC 0014 §3.2: a mid-run DSP fault classifies as `Dsp` (not the
        // `Other` fallthrough) and its diagnostic text survives the
        // conversion into the wire description.
        let e = Error::Dsp("clipping detected".into());
        let info = ErrorInfo::from(&e);
        assert_eq!(info.kind, ErrorKind::Dsp);
        assert_eq!(info.message, "dsp error: clipping detected");
    }

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
