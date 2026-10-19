//! # rustamp-mediahost
//!
//! Out-of-process decoder and DSP host binaries (RFC 0006).
//!
//! This crate provides two intentionally unavailable framework binaries:
//! - `rustamp-dechost`: refuses opens because no decoder resident is linked
//! - `rustamp-dsphost`: refuses configuration because no DSP resident is linked
//!
//! The real residents ship separately as `rustamp-dechost-symphonia` and
//! `rustamp-dsphost-stages`. Keeping them separate is the RFC 0006 §8 license
//! boundary, not an unfinished PCM transfer.
//!
//! Both are 64-bit only (their residents are native 64-bit libraries).
//!
//! **License:** MIT OR Apache-2.0 (RFC 0006 §8 erratum). Codec libraries belong
//! in separately licensed resident crates, never in this framework.
//!
//! [`decoder`] implements the lossless decoder conversation and resident boundary;
//! [`dsp`] implements the lossless two-ring DSP conversation the same way;
//! [`control`] supervises the control stream independently of blocking PCM I/O.
//! Neither API belongs on an audio render callback.

pub mod control;
pub mod decoder;
pub mod dsp;
pub mod pcm;

use thiserror::Error;

/// Errors that can occur in the media host.
#[derive(Debug, Error)]
pub enum MediaHostError {
    /// An IPC, liveness, timeout or ring failure; never interpreted as EOS.
    #[error("media host disconnected or failed: {0}")]
    Disconnected(String),

    /// A resident changed the stream specification; retry with the new spec.
    #[error("decoder format changed")]
    FormatChanged,

    /// A caller's explicit deadline elapsed without PCM or a control response.
    #[error("media host wait timed out")]
    TimedOut,

    /// Ring validation or transfer failed.
    #[error(transparent)]
    Ring(#[from] rustamp_ipc::shm::RingError),
    /// The protocol message was not recognized or is invalid for this host.
    #[error("invalid protocol message: {0}")]
    InvalidMessage(String),

    /// The requested operation is not available in this build: no resident is
    /// linked, or the platform has no cross-process PCM mapping.
    #[error("not implemented: {0}")]
    NotImplemented(String),

    /// A decoder or DSP operation failed.
    #[error("operation failed: {0}")]
    OperationFailed(String),
}

/// Result type for media host operations.
pub type Result<T> = std::result::Result<T, MediaHostError>;
