//! # rustamp-plughost
//!
//! Shared library surface of the 32-bit plugin host. The accompanying binary
//! (`rustamp-plughost32`) is the host process itself; this crate library exists
//! so the table-driven IPC answering surface can be unit tested and so other
//! permissive crates can call it without depending on the binary target.
//!
//! `loader` maps approved 32-bit plugin DLLs and calls their real entry points;
//! `registry` is the control-channel routing around it and the start/stop path
//! of the running visualization. `input` is the input (`in_`) bridge: the host
//! members of a classic input module, the
//! `init`/`play`/`pause`/`stop`/`quit` lifecycle and the state answers the
//! plugin itself gives. `handoff` is the input → output hand-over: the host
//! writes the selected output plugin into the input module before `play`, and
//! it records the end-of-track notification an input plugin posts. `vis` is the
//! visualization (`vis_`) bridge: the module list of a library, the
//! `init`/`render`/`quit` lifecycle of one of its modules, the state of the
//! render loop the host drives ([`vis::VisLoop`]) and the host callbacks the
//! running visualization answers. `dsp` is the DSP (`dsp_`) bridge: the
//! module list of a library, the `init`/`process`/`quit`/`config` lifecycle of
//! one of its modules and the `dsp_is_active`/`dsp_process` host callbacks the
//! input plugin's decode loop uses. Neither the headless surface
//! tests nor `pe_preflight` (which only inspects bytes and executes no image)
//! are DLL acceptance: that is the VM-only Group D path, see
//! `docs/tasks/P3-dll-acceptance.md` and `docs/tasks/W23-p3-acceptance.md`.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

pub mod approval;
mod commands;
pub mod control;
pub mod dialog_watchdog;
pub mod dsp;
pub mod fault;
pub mod handoff;
pub mod host_services;
pub mod host_window;
#[cfg(any(windows, test))]
pub mod input;
pub mod ipc;
pub mod loader;
pub mod message_loop;
pub mod pe_preflight;
pub mod registry;
pub mod restart;
pub mod side_by_side;
pub mod state_bridge;
pub mod vis;

pub use approval::{ApprovalEntry, ApprovalStore, ApprovalStoreError};
pub use commands::{
    DROPPED, Disposition, PUBLIC_IDS, TABLE as WM_COMMAND_TABLE, command_for_id, disposition,
    drop_reason,
};
pub use dsp::{DspError, DspLibraryInfo, DspModuleInfo, DspRunInfo};
#[cfg(windows)]
pub use fault::{FaultGuard, install_fault_guard};
pub use fault::{PLUGIN_FAULT_EXIT_CODE, PluginIdentity, plugin_call_scope};
pub use handoff::{END_OF_TRACK_MESSAGE, HandoffError, is_end_of_track_message, take_end_of_track};
pub use host_window::*;
#[cfg(any(windows, test))]
pub use input::{InModuleInfo, InputError};
pub use ipc::*;
pub use loader::{
    GeneralModuleInfo, LoadedPlugin, OutModuleInfo, PluginConfigError, PluginLoadError,
    load_plugin, load_plugin_with_store,
};
pub use message_loop::*;
pub use registry::{
    PluginRegistry, RegistryConfigError, RegistryVisError, RunningVis, accept_loaded_kind,
};
pub use restart::{CrashLedger, HostExit, RestartDecision, RestartPolicy, classify_exit};
pub use rustamp_plugin_abi::common;
pub use vis::{
    VisError, VisLibraryInfo, VisLoop, VisModuleInfo, VisPlayback, VisRows, VisRunInfo, VisStep,
};
