//! Visualization plugin header and module (S09 section 5).

use crate::common::{Hinstance, Hwnd, VIS_MAX_ROWS, VIS_ROW_LEN, layout_asserts};
use core::ffi::{c_char, c_int, c_void};

/// Visualization header, 12 bytes (S09 section 5.1).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VisHeader {
    /// 0x00, plugin: 0x101 in 2.x (0x102 is 5.x-only).
    pub version: c_int,
    /// 0x04, plugin: library name.
    pub description: *const c_char,
    /// 0x08, plugin: module `index` (0-based) or null when out of range.
    pub get_module: Option<unsafe extern "C" fn(index: c_int) -> *mut VisModule>,
}

/// Visualization module, 2360 (0x938) bytes (S09 section 5.2). Callbacks are cdecl
/// and receive the module pointer.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct VisModule {
    /// 0x000, plugin: module name.
    pub description: *const c_char,
    /// 0x004, host: Winamp main window, valid before `init` and `config`.
    pub parent_window: Hwnd,
    /// 0x008, host: plugin's module handle.
    pub dll_instance: Hinstance,
    /// 0x00C, host: sample rate in Hz (44100 default).
    pub sample_rate: c_int,
    /// 0x010, host: channel count (default 2).
    pub channels: c_int,
    /// 0x014, plugin: ms ahead of "now" the data should be taken from.
    pub latency_ms: c_int,
    /// 0x018, plugin: ms the host sleeps after each `render`.
    pub delay_ms: c_int,
    /// 0x01C, plugin: spectrum rows wanted (0..=2).
    pub spectrum_rows: c_int,
    /// 0x020, plugin: waveform rows wanted (0..=2).
    pub waveform_rows: c_int,
    /// 0x024, host: 2 x 576 unsigned bytes.
    pub spectrum_data: [[u8; VIS_ROW_LEN]; VIS_MAX_ROWS],
    /// 0x4A4, host: 2 x 576 bytes (signed 8-bit amplitudes in 5.x; 2.95 unverified, U-3).
    pub waveform_data: [[u8; VIS_ROW_LEN]; VIS_MAX_ROWS],
    /// 0x924, plugin: configuration dialog.
    pub config: Option<unsafe extern "C" fn(module: *mut VisModule)>,
    /// 0x928, plugin: 0 = success, nonzero aborts the start.
    pub init: Option<unsafe extern "C" fn(module: *mut VisModule) -> c_int>,
    /// 0x92C, plugin: draw a frame; 0 = continue, nonzero ends the visualization.
    pub render: Option<unsafe extern "C" fn(module: *mut VisModule) -> c_int>,
    /// 0x930, plugin: destroy windows and resources.
    pub quit: Option<unsafe extern "C" fn(module: *mut VisModule)>,
    /// 0x934, plugin: free for plugin use.
    pub user_data: *mut c_void,
}

impl Default for VisHeader {
    fn default() -> Self {
        // SAFETY: integer, raw pointer and Option<fn> fields only; all-zero is valid.
        unsafe { core::mem::zeroed() }
    }
}

impl Default for VisModule {
    fn default() -> Self {
        // SAFETY: integer, raw pointer, byte-array and Option<fn> fields only; all-zero is valid.
        unsafe { core::mem::zeroed() }
    }
}

layout_asserts!(VisHeader, size = 12, { version = 0x00, description = 0x04, get_module = 0x08 });
layout_asserts!(VisModule, size = 0x938, {
    description = 0x000, parent_window = 0x004, dll_instance = 0x008, sample_rate = 0x00C,
    channels = 0x010, latency_ms = 0x014, delay_ms = 0x018, spectrum_rows = 0x01C,
    waveform_rows = 0x020, spectrum_data = 0x024, waveform_data = 0x4A4, config = 0x924,
    init = 0x928, render = 0x92C, quit = 0x930, user_data = 0x934,
});
