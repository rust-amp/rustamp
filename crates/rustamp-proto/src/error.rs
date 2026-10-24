//! Serializable error description carried on the wire.
//!
//! The rich `Error` type (with `#[from] std::io::Error` and the `thiserror`
//! derive) stays in `rustamp-core`; only the flat [`ErrorInfo`] a peer can
//! decode travels in `Event::Error`.

use serde::{Deserialize, Serialize};

/// Coarse classification of an error, carried in events.
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
    /// A DSP stage failed mid-run or refused configuration (RFC 0014).
    /// Appended at the end so existing serialized values keep their index.
    Dsp,
}

/// Serializable description of an error, used by `Event::Error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    /// Classification.
    pub kind: ErrorKind,
    /// Human-readable message.
    pub message: String,
}
