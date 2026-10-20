//! Encoding and length-prefixed framing.
//!
//! Frame layout: `u32` little-endian payload length, then the `postcard`
//! payload. Usable over any byte stream (named pipe, socket, stdio).
//!
//! # The contract
//!
//! The framing stands between two processes, so it is written for a byte
//! stream that hands out its bytes in arbitrary chunks: a read may return
//! fewer bytes than were asked for, and a write may accept fewer bytes than
//! were offered. Neither is an error - both are normal on a pipe - and
//! neither may change what the peer sees. The invariants:
//!
//! * **A frame is all or nothing.** [`read_frame`] returns a message only
//!   when the whole frame arrived. A stream that ends inside a frame is
//!   [`IpcError::Truncated`], never a shorter message.
//! * **A clean end is not a broken frame.** A stream that ends exactly at a
//!   frame boundary is [`IpcError::Closed`]; one that ends inside a frame is
//!   [`IpcError::Truncated`]. The two are different failures and stay
//!   different: the first is an orderly end, the second a broken peer.
//! * **A frame is bounded before it is allocated.** A length prefix above
//!   [`MAX_FRAME_LEN`] is refused ([`IpcError::FrameTooLarge`]) *before* a
//!   buffer of that size exists, so a corrupt or hostile prefix cannot make
//!   the reader allocate (the DoS class the MP4 probe showed in wave247).
//! * **A frame is written whole.** [`write_frame`] writes the length prefix
//!   and the payload as one piece and reports a failure instead of leaving a
//!   half frame behind. A writer that accepts fewer bytes than offered is
//!   written to until the frame is complete; a writer that fails leaves the
//!   stream broken, and [`StreamTransport`](crate::StreamTransport) then
//!   refuses to carry another frame on it.
//!
//! The framing never reads past the end of a frame: it asks for exactly the
//! bytes it needs, so the frame that follows starts where this one ended.

use std::io::{Read, Write};

use crate::message::Message;
use crate::transport::IpcError;

/// Largest accepted payload (16 MiB); protects against corrupt length prefixes.
///
/// The rule, in both directions:
///
/// * a frame whose declared length is **at most** this value is carried;
/// * a frame whose declared length is **above** it is refused with
///   [`IpcError::FrameTooLarge`] **before** a buffer of that size exists.
///
/// The writer applies the same limit, so a frame the reader would refuse is
/// never written.
pub const MAX_FRAME_LEN: u32 = 16 * 1024 * 1024;

/// Encode a message to bytes (no length prefix).
pub fn encode(msg: &Message) -> Result<Vec<u8>, IpcError> {
    Ok(postcard::to_allocvec(msg)?)
}

/// Decode a message from bytes (no length prefix).
///
/// The bytes must hold one message. A payload that is longer than the
/// message decodes to the message and ignores the rest: a frame bounds its
/// payload with the length prefix, so the extra bytes stay inside the frame
/// and the stream stays framed.
pub fn decode(bytes: &[u8]) -> Result<Message, IpcError> {
    Ok(postcard::from_bytes(bytes)?)
}

/// Write one framed message and flush.
///
/// The length prefix and the payload go out as one piece, so a stream that
/// takes the frame whole never sees a prefix without its payload. A writer
/// that accepts fewer bytes than offered is written to until the frame is
/// complete; a writer that fails reports the failure, and the bytes it
/// accepted so far are a broken frame - which
/// [`StreamTransport`](crate::StreamTransport) refuses to follow with
/// another one.
pub fn write_frame<W: Write>(w: &mut W, msg: &Message) -> Result<(), IpcError> {
    let payload = encode(msg)?;
    let len = u32::try_from(payload.len())
        .ok()
        .filter(|l| *l <= MAX_FRAME_LEN)
        .ok_or(IpcError::FrameTooLarge(payload.len() as u64))?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_le_bytes());
    frame.extend_from_slice(&payload);
    w.write_all(&frame)?;
    w.flush()?;
    Ok(())
}

/// Read one framed message.
///
/// A clean EOF before the first byte of a frame yields [`IpcError::Closed`];
/// a stream that ends inside a frame yields [`IpcError::Truncated`] - never
/// a shorter message.
pub fn read_frame<R: Read>(r: &mut R) -> Result<Message, IpcError> {
    let mut len_buf = [0u8; 4];
    let prefix = read_full(r, &mut len_buf)?;
    if prefix == 0 {
        // The stream ended exactly at a frame boundary.
        return Err(IpcError::Closed);
    }
    if prefix < len_buf.len() {
        // The stream ended inside the length prefix: a broken frame, not a
        // clean end.
        return Err(IpcError::Truncated);
    }
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_LEN {
        // Refused before the buffer of that size exists.
        return Err(IpcError::FrameTooLarge(u64::from(len)));
    }
    let mut payload = vec![0u8; len as usize];
    if read_full(r, &mut payload)? < len as usize {
        // The stream ended inside the payload.
        return Err(IpcError::Truncated);
    }
    decode(&payload)
}

/// Read into `buf` until it is full and report how many bytes arrived.
///
/// A short read is normal on a byte stream, so the loop continues until the
/// buffer is full. The count is the number of bytes that arrived: fewer
/// than `buf.len()` means the stream ended in the middle, and the caller
/// turns that into [`IpcError::Truncated`].
fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<usize, IpcError> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(filled)
}
