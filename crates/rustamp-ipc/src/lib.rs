//! # rustamp-ipc
//!
//! Transport-agnostic protocol between `rustamp.exe` (x64) and
//! `rustamp-plughost32.exe` (i686).
//!
//! * [`Message`] - the protocol enum (v0), encoded with `postcard`.
//! * [`Transport`] - send/receive framed messages. Implementations: the
//!   in-memory [`MemoryTransport`] (tests) and [`StreamTransport`] over any
//!   `Read`/`Write` pair (named pipes, sockets, stdio) using the
//!   length-prefixed framing in [`framing`].
//!
//! Wire types never contain `usize`/`isize`, so the encoding is identical on
//! both architectures (indices are `u32`, handles `u64`).
//!
//! **Not here yet:** bulk PCM and visualization data will move to a
//! shared-memory ring (a later milestone, M3); `Message::Vis` is the simple
//! fallback path. Semantics of `WinampIpc` messages are defined by spec S10.

pub mod framing;
pub mod message;
pub mod transport;

pub use framing::{MAX_FRAME_LEN, decode, encode, read_frame, write_frame};
pub use message::{Message, PROTOCOL_VERSION, VisFrameWire};
pub use transport::{IpcError, MemoryTransport, StreamTransport, Transport};
