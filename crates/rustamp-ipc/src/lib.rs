//! # rustamp-ipc
//!
//! Transport-agnostic protocol between `rustamp.exe` (x64) and
//! `rustamp-plughost32.exe` (i686).
//!
//! * [`Message`] - the protocol enum (v1), encoded with `postcard`.
//! * [`Transport`] - send/receive framed messages. Implementations: the
//!   in-memory [`MemoryTransport`] (tests) and [`StreamTransport`] over any
//!   `Read`/`Write` pair (named pipes, sockets, stdio) using the
//!   length-prefixed framing in [`framing`].
//! * [`shm`] - the shared-memory rings that carry bulk PCM and visualization
//!   data between the two processes (RFC 0003): a lock-free, allocation-free
//!   SPSC ring with the same byte layout on x64 and i686.
//!
//! Wire types never contain `usize`/`isize`, so the encoding is identical on
//! both architectures (indices are `u32`, handles `u64`).
//!
//! **Not here yet:** the semantics of `WinampIpc` messages are defined by spec
//! S10; `Message::Vis` remains the small-frame fallback path next to the vis
//! ring.

pub mod framing;
pub mod message;
pub mod shm;
pub mod transport;

pub use framing::{MAX_FRAME_LEN, decode, encode, read_frame, write_frame};
pub use message::{Message, PROTOCOL_VERSION, VisFrameWire};
pub use transport::{IpcError, MemoryTransport, StreamTransport, Transport};
