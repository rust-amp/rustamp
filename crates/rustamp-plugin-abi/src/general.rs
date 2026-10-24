//! General purpose plugin `winampGeneralPurposePlugin` (S09 section 7).

use crate::common::{Hinstance, Hwnd, layout_asserts};
use core::ffi::{c_char, c_int};

/// General purpose plugin, 28 bytes (S09 section 7.1). Callbacks are cdecl.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct GeneralPlugin {
    /// 0x00, plugin: 0x10 in 2.95 (0x11 is 5.x-only).
    pub version: c_int,
    /// 0x04, plugin: description.
    pub description: *const c_char,
    /// 0x08, plugin: 0 = success (role of the return in 2.95 unverified, U-8).
    pub init: Option<unsafe extern "C" fn() -> c_int>,
    /// 0x0C, plugin: open configuration (no arguments).
    pub config: Option<unsafe extern "C" fn()>,
    /// 0x10, plugin: called at exit.
    pub quit: Option<unsafe extern "C" fn()>,
    /// 0x14, host: Winamp main window.
    pub parent_window: Hwnd,
    /// 0x18, host: plugin's module handle.
    pub dll_instance: Hinstance,
}

impl Default for GeneralPlugin {
    fn default() -> Self {
        // SAFETY: integer, raw pointer and Option<fn> fields only; all-zero is valid.
        unsafe { core::mem::zeroed() }
    }
}

layout_asserts!(GeneralPlugin, size = 28, {
    version = 0x00, description = 0x04, init = 0x08, config = 0x0C, quit = 0x10,
    parent_window = 0x14, dll_instance = 0x18,
});
