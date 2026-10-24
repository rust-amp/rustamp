//! DSP plugin header and module (S09 section 6). Layouts come from the SDK/5.x
//! per S09 and are not checked against a 2.x DSP DLL.

use crate::common::{Hinstance, Hwnd, layout_asserts};
use core::ffi::{c_char, c_int, c_void};

/// DSP header as in 2.x, 12 bytes (S09 section 6.1).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DspHeader {
    /// 0x00, plugin: 0x20 for 2.x.
    pub version: c_int,
    /// 0x04, plugin: description.
    pub description: *const c_char,
    /// 0x08, plugin: module `index` or null.
    pub get_module: Option<unsafe extern "C" fn(index: c_int) -> *mut DspModule>,
}

/// DSP header as in 5.x, 16 bytes (S09 section 6.1). 5.x-only.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DspHeader5x {
    /// 0x00, plugin: 0x21/0x22.
    pub version: c_int,
    /// 0x04, plugin: description.
    pub description: *const c_char,
    /// 0x08, plugin: module `index` or null.
    pub get_module: Option<unsafe extern "C" fn(index: c_int) -> *mut DspModule>,
    /// 0x0C, plugin: key check. 5.x-only, absent from 2.x headers.
    pub key_check: Option<unsafe extern "C" fn(key: c_int) -> c_int>,
}

/// DSP module, 32 bytes (S09 section 6.2). Callbacks are cdecl and take the module pointer.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct DspModule {
    /// 0x00, plugin: description.
    pub description: *const c_char,
    /// 0x04, host: Winamp main window, set before `init`.
    pub parent_window: Hwnd,
    /// 0x08, host: plugin's module handle.
    pub dll_instance: Hinstance,
    /// 0x0C, plugin: configuration.
    pub config: Option<unsafe extern "C" fn(module: *mut DspModule)>,
    /// 0x10, plugin: 0 on success.
    pub init: Option<unsafe extern "C" fn(module: *mut DspModule) -> c_int>,
    /// 0x14, plugin: process in place, returns frames to output (half..double of input).
    pub process: Option<
        unsafe extern "C" fn(
            module: *mut DspModule,
            samples: *mut c_void,
            frames: c_int,
            bits: c_int,
            channels: c_int,
            sample_rate: c_int,
        ) -> c_int,
    >,
    /// 0x18, plugin: shutdown.
    pub quit: Option<unsafe extern "C" fn(module: *mut DspModule)>,
    /// 0x1C, plugin: free for plugin use.
    pub user_data: *mut c_void,
}

impl Default for DspHeader {
    fn default() -> Self {
        // SAFETY: integer, raw pointer and Option<fn> fields only; all-zero is valid.
        unsafe { core::mem::zeroed() }
    }
}
impl Default for DspHeader5x {
    fn default() -> Self {
        // SAFETY: integer, raw pointer and Option<fn> fields only; all-zero is valid.
        unsafe { core::mem::zeroed() }
    }
}
impl Default for DspModule {
    fn default() -> Self {
        // SAFETY: integer, raw pointer and Option<fn> fields only; all-zero is valid.
        unsafe { core::mem::zeroed() }
    }
}

layout_asserts!(DspHeader, size = 12, { version = 0x00, description = 0x04, get_module = 0x08 });
layout_asserts!(DspHeader5x, size = 16, {
    version = 0x00, description = 0x04, get_module = 0x08, key_check = 0x0C,
});
layout_asserts!(DspModule, size = 32, {
    description = 0x00, parent_window = 0x04, dll_instance = 0x08, config = 0x0C,
    init = 0x10, process = 0x14, quit = 0x18, user_data = 0x1C,
});
