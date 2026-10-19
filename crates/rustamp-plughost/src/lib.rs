//! # rustamp-plughost
//!
//! Shared library surface of the 32-bit plugin host. The accompanying binary
//! (`rustamp-plughost32`) is the host process itself; this crate library exists
//! so the table-driven IPC answering surface can be unit tested and so other
//! permissive crates can call it without depending on the binary target.
//!
//! `loader` maps approved 32-bit plugin DLLs and calls their real entry points;
//! `registry` is the control-channel routing around it. `vis` is the
//! visualization (`vis_`) bridge: the module list of a library, the
//! `init`/`render`/`quit` lifecycle of one of its modules and the host
//! callbacks the running visualization answers. Neither the headless surface
//! tests nor `pe_preflight` (which only inspects bytes and executes no image)
//! are DLL acceptance: that is the VM-only Group D path, see
//! `docs/tasks/P3-dll-acceptance.md` and `docs/tasks/W23-p3-acceptance.md`.
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
pub mod registry;
pub mod state_bridge;
pub mod vis;

pub use approval::{ApprovalEntry, ApprovalStore, ApprovalStoreError};
pub use commands::{
    DROPPED, Disposition, PUBLIC_IDS, TABLE as WM_COMMAND_TABLE, command_for_id, disposition,
    drop_reason,
};
pub use host_window::*;
pub use ipc::*;
pub use loader::{
    DspModuleInfo, GeneralModuleInfo, LoadedPlugin, OutModuleInfo, PluginConfigError,
    PluginLoadError, load_plugin, load_plugin_with_store,
};
pub use message_loop::*;
pub use registry::{PluginRegistry, RegistryConfigError, accept_loaded_kind};
pub use rustamp_plugin_abi::common;
pub use vis::{VisError, VisLibraryInfo, VisModuleInfo, VisRunInfo};
