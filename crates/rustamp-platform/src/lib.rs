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
//! DPI awareness and window-chrome size math live in [`dpi`]; monitor
//! discovery, scale factors and window-position clamping live in
//! [`displays`].
//! Single instance and command-line forwarding live in [`single_instance`].
//! The tray menu model and the Win32 icon attach live in [`tray`]; global
//! hotkeys live in [`hotkeys`].

pub mod displays;
pub mod dpi;
pub mod hotkeys;
pub mod single_instance;
pub mod tray;

pub use displays::{
    DisplayChanges, DisplayError, DisplaySnapshot, Monitor, ScreenRect, clamp_saved_position,
    display_changes, dpi_at,
};
pub use dpi::{DpiAwareness, DpiError, WindowChrome, init_dpi_awareness, outer_size_for_client};
pub use hotkeys::{
    DEFAULT_HOTKEYS, HotkeyAction, HotkeyBackend, HotkeyCombo, HotkeyError, HotkeyId,
    HotkeyModifiers, HotkeyRegistry, HotkeySession, describe, take_pending_hotkeys,
};
#[cfg(windows)]
pub use hotkeys::{Win32HotkeyBackend, Win32HotkeyError};
pub use single_instance::{
    Claim, ForwardRequest, HandOff, Instance, LockState, StartupAction, claim, forward,
    instance_key, startup_action, take_pending,
};
pub use tray::{TRAY_CALLBACK_MESSAGE, TRAY_MENU, TRAY_TOOLTIP, TrayMenuEntry};
#[cfg(windows)]
pub use tray::{TrayError, TrayIcon};
