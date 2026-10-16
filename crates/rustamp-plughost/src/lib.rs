//! # rustamp-plughost
//!
//! Shared library surface of the 32-bit plugin host. The accompanying binary
//! (`rustamp-plughost32`) is the host process itself; this crate library exists
//! so the table-driven IPC answering surface can be unit tested and so other
//! permissive crates can call it without depending on the binary target.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

mod commands;
pub mod ipc;

pub use commands::{
    DROPPED, Disposition, PUBLIC_IDS, TABLE as WM_COMMAND_TABLE, command_for_id, disposition,
    drop_reason,
};
pub use ipc::*;
