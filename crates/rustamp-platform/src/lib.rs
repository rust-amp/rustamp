// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-platform
//!
//! Platform layer: single instance and command-line forwarding, media keys,
//! taskbar integration, file associations and the external-control window.
//! Win32 first; macOS/Linux stubs later.
//!
//! Skeleton (milestone M0). Codes against the frozen contracts in
//! [`rustamp_core`]; see `docs/rfc/0001-core-contracts.md`.
//!
//! DPI awareness and window-chrome size math live in [`dpi`].
//! Single instance and command-line forwarding live in [`single_instance`].

pub mod dpi;
pub mod single_instance;

pub use dpi::{DpiAwareness, DpiError, WindowChrome, init_dpi_awareness, outer_size_for_client};
pub use single_instance::{
    Claim, ForwardRequest, HandOff, Instance, LockState, StartupAction, claim, forward,
    instance_key, startup_action, take_pending,
};
