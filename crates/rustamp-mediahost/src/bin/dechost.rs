//! `rustamp-dechost` — out-of-process decoder host (RFC 0006).
//!
//! This binary hosts a single decoder in a separate process. It speaks the
//! control protocol over stdin/stdout (length-prefixed postcard frames) and
//! transfers PCM through a shared-memory ring (RFC 0003).
//!
//! **Stub implementation (M6 card):** only the control protocol skeleton is
//! implemented. The actual decoder opening, PCM ring handling, seek/format
//! change/EOS logic, and process spawning are out of scope and belong to a
//! later card.

use rustamp_ipc::{Message, StreamTransport, Transport};
use rustamp_mediahost::{DecoderHost, MediaHost, MediaHostError, Result};
use std::io::{stdin, stdout};

fn main() -> Result<()> {
    let mut host = DecoderHost::new();
    let mut transport = StreamTransport::new(stdin(), stdout());

    // Send our Hello immediately
    transport
        .send(&Message::hello())
        .map_err(|e| MediaHostError::OperationFailed(format!("failed to send Hello: {e}")))?;

    // Main message loop
    loop {
        let msg = match transport.recv() {
            Ok(msg) => msg,
            Err(rustamp_ipc::IpcError::Closed) => break Ok(()),
            Err(e) => return Err(MediaHostError::OperationFailed(format!("recv error: {e}"))),
        };

        if let Some(response) = host.handle_message(msg)? {
            transport.send(&response).map_err(|e| {
                MediaHostError::OperationFailed(format!("failed to send response: {e}"))
            })?;
        }
    }
}
