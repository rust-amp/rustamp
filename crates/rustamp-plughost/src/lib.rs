//! # rustamp-plughost
//!
//! Shared library surface of the 32-bit plugin host. The accompanying binary
//! (`rustamp-plughost32`) is the host process itself; this crate library exists
//! so the table-driven IPC answering surface can be unit tested and so other
//! permissive crates can call it without depending on the binary target.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

mod commands;
pub mod host_window;
pub mod ipc;
pub mod message_loop;
pub mod state_bridge;

pub use commands::{
    DROPPED, Disposition, PUBLIC_IDS, TABLE as WM_COMMAND_TABLE, command_for_id, disposition,
    drop_reason,
};
pub use host_window::*;
pub use ipc::*;
pub use message_loop::*;
pub use rustamp_plugin_abi::common;
