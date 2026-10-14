//! Encoding and length-prefixed framing.
//!
//! Frame layout: `u32` little-endian payload length, then the `postcard`
//! payload. Usable over any byte stream (named pipe, socket, stdio).

use std::io::{Read, Write};

use crate::message::Message;
use crate::transport::IpcError;

/// Largest accepted payload (16 MiB); protects against corrupt length prefixes.
pub const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;

/// Encode a message to bytes (no length prefix).
pub fn encode(msg: &Message) -> Result<Vec<u8>, IpcError> {
    Ok(postcard::to_allocvec(msg)?)
}

/// Decode a message from bytes (no length prefix).
pub fn decode(bytes: &[u8]) -> Result<Message, IpcError> {
    Ok(postcard::from_bytes(bytes)?)
}

/// Write one framed message and flush.
pub fn write_frame<W: Write>(w: &mut W, msg: &Message) -> Result<(), IpcError> {
    let payload = encode(msg)?;
    let len = u32::try_from(payload.len())
        .ok()
        .filter(|l| *l <= MAX_FRAME_LEN)
        .ok_or(IpcError::FrameTooLarge(payload.len() as u64))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(&payload)?;
    w.flush()?;
    Ok(())
}

/// Read one framed message. A clean EOF before the first byte of a frame
/// yields [`IpcError::Closed`].
pub fn read_frame<R: Read>(r: &mut R) -> Result<Message, IpcError> {
    let mut len_buf = [0u8; 4];
    match r.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Err(IpcError::Closed),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_LEN {
        return Err(IpcError::FrameTooLarge(u64::from(len)));
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload)?;
    decode(&payload)
}
