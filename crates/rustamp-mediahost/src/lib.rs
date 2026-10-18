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
//! boundary, not an unfinished PCM transfer. The legacy [`DecoderHost`] and
//! [`DspHost`] single-message stubs below are not used by either serving loop.
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

use rustamp_ipc::Message;
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

    /// The requested operation is not implemented (stub).
    #[error("not implemented: {0}")]
    NotImplemented(String),

    /// A decoder or DSP operation failed.
    #[error("operation failed: {0}")]
    OperationFailed(String),
}

/// Result type for media host operations.
pub type Result<T> = std::result::Result<T, MediaHostError>;

/// Trait for a host that can handle control messages.
pub trait MediaHost {
    /// Handle an incoming control message from the main process.
    fn handle_message(&mut self, msg: Message) -> Result<Option<Message>>;

    /// The host's process ID (for Hello messages).
    fn pid(&self) -> u32;
}

/// Stub decoder host implementation.
pub struct DecoderHost {
    pid: u32,
}

impl DecoderHost {
    /// Create a new decoder host stub.
    pub fn new() -> Self {
        Self {
            pid: std::process::id(),
        }
    }
}

impl Default for DecoderHost {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaHost for DecoderHost {
    fn handle_message(&mut self, msg: Message) -> Result<Option<Message>> {
        match msg {
            Message::Hello {
                protocol_version, ..
            } if protocol_version == rustamp_ipc::PROTOCOL_VERSION => {
                // Respond with our Hello
                Ok(Some(Message::hello()))
            }
            Message::Ping(token) => {
                // Respond with Pong
                Ok(Some(Message::Pong(token)))
            }
            Message::DecoderOpen { .. } => {
                // Stub: not implemented yet (later card)
                Err(MediaHostError::NotImplemented(
                    "DecoderOpen not implemented in stub".into(),
                ))
            }
            Message::DecoderSeek { .. } => Err(MediaHostError::NotImplemented(
                "DecoderSeek not implemented in stub".into(),
            )),
            Message::Shutdown => Ok(None),
            _ => Err(MediaHostError::InvalidMessage(format!(
                "DecoderHost does not handle {:?}",
                msg
            ))),
        }
    }

    fn pid(&self) -> u32 {
        self.pid
    }
}

/// Stub DSP host implementation.
pub struct DspHost {
    pid: u32,
}

impl DspHost {
    /// Create a new DSP host stub.
    pub fn new() -> Self {
        Self {
            pid: std::process::id(),
        }
    }
}

impl Default for DspHost {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaHost for DspHost {
    fn handle_message(&mut self, msg: Message) -> Result<Option<Message>> {
        match msg {
            Message::Hello {
                protocol_version, ..
            } if protocol_version == rustamp_ipc::PROTOCOL_VERSION => Ok(Some(Message::hello())),
            Message::Ping(token) => Ok(Some(Message::Pong(token))),
            Message::DspConfigure { .. } => Err(MediaHostError::NotImplemented(
                "DspConfigure not implemented in stub".into(),
            )),
            Message::DspReset => Err(MediaHostError::NotImplemented(
                "DspReset not implemented in stub".into(),
            )),
            // A reply, not a command: the stub never sends DspResetAck, so
            // receiving one is a protocol violation (RFC 0015).
            Message::DspResetAck => Err(MediaHostError::InvalidMessage(
                "DspResetAck is a host-to-player reply; the DSP host does not handle it".into(),
            )),
            Message::Shutdown => Ok(None),
            _ => Err(MediaHostError::InvalidMessage(format!(
                "DspHost does not handle {:?}",
                msg
            ))),
        }
    }

    fn pid(&self) -> u32 {
        self.pid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_ipc::Message;

    #[test]
    fn decoder_host_hello_roundtrip() {
        let mut host = DecoderHost::new();
        let hello = Message::Hello {
            protocol_version: rustamp_ipc::PROTOCOL_VERSION,
            pid: 1234,
        };
        let resp = host.handle_message(hello).unwrap().unwrap();
        assert!(matches!(resp, Message::Hello { .. }));
    }

    #[test]
    fn decoder_host_ping_pong() {
        let mut host = DecoderHost::new();
        let resp = host.handle_message(Message::Ping(42)).unwrap().unwrap();
        assert_eq!(resp, Message::Pong(42));
    }

    #[test]
    fn dsp_host_hello_roundtrip() {
        let mut host = DspHost::new();
        let hello = Message::Hello {
            protocol_version: rustamp_ipc::PROTOCOL_VERSION,
            pid: 5678,
        };
        let resp = host.handle_message(hello).unwrap().unwrap();
        assert!(matches!(resp, Message::Hello { .. }));
    }

    #[test]
    fn dsp_host_ping_pong() {
        let mut host = DspHost::new();
        let resp = host.handle_message(Message::Ping(99)).unwrap().unwrap();
        assert_eq!(resp, Message::Pong(99));
    }
}
