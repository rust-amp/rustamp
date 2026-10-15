//! # rustamp-mediahost
//!
//! Out-of-process decoder and DSP host binaries (RFC 0006).
//!
//! This crate provides two binaries:
//! - `rustamp-dechost`: hosts a single decoder process
//! - `rustamp-dsphost`: hosts a DSP chain process
//!
//! Both are 64-bit only (their residents are native 64-bit libraries).
//!
//! **License:** GPL-3.0-or-later **without** the §7 plugin exception (RFC 0006 §8).
//! This crate exists to link copyleft decoder/DSP code that the exception cannot cover.
//! Nothing permissive may depend on this crate.
//!
//! **Scope (M6 card):** stubs plus protocol only. The actual PCM transfer, ring wake
//! implementation, opening a real decoder, and process spawning are out of scope
//! and belong to a later card that requires the lossless-delivery work (RFC 0007).

use rustamp_ipc::Message;
use thiserror::Error;

/// Errors that can occur in the media host.
#[derive(Debug, Error)]
pub enum MediaHostError {
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
            Message::Hello { .. } => {
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
            Message::Shutdown => {
                // Clean shutdown
                std::process::exit(0);
            }
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
            Message::Hello { .. } => Ok(Some(Message::hello())),
            Message::Ping(token) => Ok(Some(Message::Pong(token))),
            Message::DspConfigure { .. } => Err(MediaHostError::NotImplemented(
                "DspConfigure not implemented in stub".into(),
            )),
            Message::DspReset => Err(MediaHostError::NotImplemented(
                "DspReset not implemented in stub".into(),
            )),
            Message::Shutdown => {
                std::process::exit(0);
            }
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
            protocol_version: 2,
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
            protocol_version: 2,
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
