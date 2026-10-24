//! Output plugin module `Out_Module` (S09 section 4).

use crate::common::{Hinstance, Hwnd, layout_asserts};
use core::ffi::{c_char, c_int};

/// Output module, 20 members, 0x50 bytes (S09 section 4.1). All callbacks are cdecl.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct OutModule {
    /// 0x00, plugin: 0x10 in 2.95.
    pub version: c_int,
    /// 0x04, plugin: ANSI description.
    pub description: *const c_char,
    /// 0x08, plugin: module id (>= 65536 for third parties). The 5.x host overwrites it
    /// with a pointer to a copy of the DLL file name (5.x-only), hence kept as raw 32 bits.
    pub id: c_int,
    /// 0x0C, host: Winamp main window.
    pub main_window: Hwnd,
    /// 0x10, host: plugin's module handle.
    pub dll_instance: Hinstance,
    /// 0x14, plugin: configuration dialog.
    pub config: Option<unsafe extern "C" fn(parent: Hwnd)>,
    /// 0x18, plugin: about box.
    pub about: Option<unsafe extern "C" fn(parent: Hwnd)>,
    /// 0x1C, plugin: called when the output list is built.
    pub init: Option<unsafe extern "C" fn()>,
    /// 0x20, plugin: called at exit.
    pub quit: Option<unsafe extern "C" fn()>,
    /// 0x24, plugin: (rate, channels, bits, buffer ms, prebuffer ms) -> max latency ms or negative.
    /// A buffer_ms of -666 means the output clock syncs video.
    pub open: Option<
        unsafe extern "C" fn(
            sample_rate: c_int,
            channels: c_int,
            bits: c_int,
            buffer_ms: c_int,
            prebuffer_ms: c_int,
        ) -> c_int,
    >,
    /// 0x28, plugin: close the device.
    pub close: Option<unsafe extern "C" fn()>,
    /// 0x2C, plugin: non-blocking write of at most 8192 bytes; 0 = taken.
    pub write: Option<unsafe extern "C" fn(data: *mut c_char, len: c_int) -> c_int>,
    /// 0x30, plugin: bytes writable right now.
    pub can_write: Option<unsafe extern "C" fn() -> c_int>,
    /// 0x34, plugin: nonzero while buffered audio is still audible.
    pub is_playing: Option<unsafe extern "C" fn() -> c_int>,
    /// 0x38, plugin: 1 pauses, 0 resumes; returns previous state.
    pub pause: Option<unsafe extern "C" fn(state: c_int) -> c_int>,
    /// 0x3C, plugin: volume 0..255.
    pub set_volume: Option<unsafe extern "C" fn(volume: c_int)>,
    /// 0x40, plugin: pan -128..128.
    pub set_pan: Option<unsafe extern "C" fn(pan: c_int)>,
    /// 0x44, plugin: drop buffered audio and restart the clock at `time_ms`.
    pub flush: Option<unsafe extern "C" fn(time_ms: c_int)>,
    /// 0x48, plugin: ms actually played since the last flush.
    pub get_played_ms: Option<unsafe extern "C" fn() -> c_int>,
    /// 0x4C, plugin: ms written so far.
    pub get_written_ms: Option<unsafe extern "C" fn() -> c_int>,
}

impl Default for OutModule {
    fn default() -> Self {
        // SAFETY: every field is an integer, raw pointer or Option<fn>; all-zero is valid.
        unsafe { core::mem::zeroed() }
    }
}

layout_asserts!(OutModule, size = 0x50, {
    version = 0x00, description = 0x04, id = 0x08, main_window = 0x0C, dll_instance = 0x10,
    config = 0x14, about = 0x18, init = 0x1C, quit = 0x20, open = 0x24, close = 0x28,
    write = 0x2C, can_write = 0x30, is_playing = 0x34, pause = 0x38, set_volume = 0x3C,
    set_pan = 0x40, flush = 0x44, get_played_ms = 0x48, get_written_ms = 0x4C,
});
