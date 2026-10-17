//! # rustamp-plughost
//!
//! Shared library surface of the 32-bit plugin host. The accompanying binary
//! (`rustamp-plughost32`) is the host process itself; this crate library exists
//! so the table-driven IPC answering surface can be unit tested and so other
//! permissive crates can call it without depending on the binary target.
//! No DLL-loading path exists yet. Headless surface tests are not DLL
//! acceptance; `pe_preflight` only inspects bytes and executes no image.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

pub mod approval;
mod commands;
pub mod control;
pub mod host_window;
pub mod ipc;
pub mod loader;
pub mod message_loop;
pub mod pe_preflight;
pub mod state_bridge;

pub use approval::{ApprovalEntry, ApprovalStore, ApprovalStoreError};
pub use commands::{
    DROPPED, Disposition, PUBLIC_IDS, TABLE as WM_COMMAND_TABLE, command_for_id, disposition,
    drop_reason,
};
pub use host_window::*;
pub use ipc::*;
pub use loader::{LoadedPlugin, PluginLoadError, load_plugin, load_plugin_with_store};
pub use message_loop::*;
pub use rustamp_plugin_abi::common;
