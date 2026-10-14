//! Input plugin module `In_Module` (S09 section 3).

use crate::common::{Hinstance, Hwnd, layout_asserts};
use crate::output::OutModule;
use core::ffi::{c_char, c_int, c_void};

/// Input module, 38 members, 0x98 bytes (S09 section 3.1). All callbacks are cdecl.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InModule {
    /// 0x00, plugin: 0x100 in 2.95 (S09 3.2).
    pub version: c_int,
    /// 0x04, plugin: ANSI description string.
    pub description: *const c_char,
    /// 0x08, host: Winamp main window, set before `init`.
    pub main_window: Hwnd,
    /// 0x0C, host: plugin's module handle.
    pub dll_instance: Hinstance,
    /// 0x10, plugin: double-NUL-terminated list of (extension list, display name) pairs (S09 3.4).
    pub file_extensions: *mut c_char,
    /// 0x14, plugin: nonzero if the current stream is seekable.
    pub seekable: c_int,
    /// 0x18, plugin: flag bits, see `IN_FLAG_*` (S09 3.3).
    pub flags: c_int,
    /// 0x1C, plugin: open configuration dialog.
    pub config: Option<unsafe extern "C" fn(parent: Hwnd)>,
    /// 0x20, plugin: open about box.
    pub about: Option<unsafe extern "C" fn(parent: Hwnd)>,
    /// 0x24, plugin: called once at startup (return value ignored in 2.x).
    pub init: Option<unsafe extern "C" fn()>,
    /// 0x28, plugin: called once at exit.
    pub quit: Option<unsafe extern "C" fn()>,
    /// 0x2C, plugin: (path or null, title buffer of 2048 bytes, length-in-ms out; -1 unknown).
    pub get_file_info: Option<
        unsafe extern "C" fn(file: *const c_char, title: *mut c_char, length_ms: *mut c_int),
    >,
    /// 0x30, plugin: show file-info dialog; 0 if edited, 1 if unchanged.
    pub info_box: Option<unsafe extern "C" fn(file: *const c_char, parent: Hwnd) -> c_int>,
    /// 0x34, plugin: nonzero if this plugin claims the name.
    pub is_our_file: Option<unsafe extern "C" fn(name: *const c_char) -> c_int>,
    /// 0x38, plugin: start playback; 0 success, -1 not found, other nonzero error.
    pub play: Option<unsafe extern "C" fn(file: *const c_char) -> c_int>,
    /// 0x3C, plugin: pause.
    pub pause: Option<unsafe extern "C" fn()>,
    /// 0x40, plugin: resume.
    pub unpause: Option<unsafe extern "C" fn()>,
    /// 0x44, plugin: 1 if paused.
    pub is_paused: Option<unsafe extern "C" fn() -> c_int>,
    /// 0x48, plugin: stop and unload the stream.
    pub stop: Option<unsafe extern "C" fn()>,
    /// 0x4C, plugin: track length in ms.
    pub get_length: Option<unsafe extern "C" fn() -> c_int>,
    /// 0x50, plugin: position in ms; may be called from other threads.
    pub get_position: Option<unsafe extern "C" fn() -> c_int>,
    /// 0x54, plugin: seek to ms.
    pub seek: Option<unsafe extern "C" fn(ms: c_int)>,
    /// 0x58, plugin: volume 0..255.
    pub set_volume: Option<unsafe extern "C" fn(volume: c_int)>,
    /// 0x5C, plugin: pan -127..127.
    pub set_pan: Option<unsafe extern "C" fn(pan: c_int)>,
    /// 0x60, host: (max latency ms, sample rate), called from `play`.
    pub analyzer_init: Option<unsafe extern "C" fn(max_latency_ms: c_int, sample_rate: c_int)>,
    /// 0x64, host: called from `stop`.
    pub analyzer_deinit: Option<unsafe extern "C" fn()>,
    /// 0x68, host: (interleaved PCM, channels, bits per sample, timestamp ms); >= 576 frames per call.
    pub analyzer_feed_pcm: Option<
        unsafe extern "C" fn(pcm: *mut c_void, channels: c_int, bits: c_int, timestamp_ms: c_int),
    >,
    /// 0x6C, host: wanted analyzer data mode (1 spectrum, 2 scope, 3 both, 4 none).
    pub analyzer_get_mode: Option<unsafe extern "C" fn() -> c_int>,
    /// 0x70, host: (rows, timestamp ms, mode) -> nonzero if host is not collecting.
    pub analyzer_feed_rows:
        Option<unsafe extern "C" fn(rows: *mut c_void, timestamp_ms: c_int, mode: c_int) -> c_int>,
    /// 0x74, host: same arguments as `analyzer_feed_pcm`, for visualization.
    pub vis_feed_pcm: Option<
        unsafe extern "C" fn(pcm: *mut c_void, channels: c_int, bits: c_int, timestamp_ms: c_int),
    >,
    /// 0x78, host: (spectrum rows out, waveform rows out) -> nonzero if a vis is running.
    pub vis_get_mode:
        Option<unsafe extern "C" fn(spectrum_rows: *mut c_int, waveform_rows: *mut c_int) -> c_int>,
    /// 0x7C, host: (frame data, timestamp ms) -> nonzero if not accepted.
    pub vis_feed_frame:
        Option<unsafe extern "C" fn(frame: *mut c_void, timestamp_ms: c_int) -> c_int>,
    /// 0x80, host: (sample rate, channels), in that order.
    pub vis_set_format: Option<unsafe extern "C" fn(sample_rate: c_int, channels: c_int)>,
    /// 0x84, host: nonzero if the host will modify samples.
    pub dsp_is_active: Option<unsafe extern "C" fn() -> c_int>,
    /// 0x88, host: (16-bit samples, frames, bits, channels, rate) -> frames now in the buffer.
    pub dsp_process: Option<
        unsafe extern "C" fn(
            samples: *mut c_void,
            frames: c_int,
            bits: c_int,
            channels: c_int,
            sample_rate: c_int,
        ) -> c_int,
    >,
    /// 0x8C, plugin (optional): (on, 10 band bytes, preamp); values 0..63, 31 = 0 dB.
    pub eq_set: Option<unsafe extern "C" fn(on: c_int, bands: *mut c_char, preamp: c_int)>,
    /// 0x90, host: (bitrate kbps, sample rate kHz, channels, synched); -1 = no change.
    pub set_info: Option<
        unsafe extern "C" fn(
            bitrate_kbps: c_int,
            sample_rate_khz: c_int,
            channels: c_int,
            synched: c_int,
        ),
    >,
    /// 0x94, host: selected output module, set before `play` when flag bit 0 is set.
    pub out_module: *mut OutModule,
}

/// Input module with the 39th member (S09 section 3.1 "5.x-only tail"). 5.x-only:
/// present only if the version carries [`crate::common::IN_VERSION_FLAG_INIT_STATUS`].
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct InModuleExt {
    /// The 38 common members.
    pub base: InModule,
    /// 0x98, host: pointer to the host service table. 5.x-only.
    pub host_services: *mut c_void,
}

impl Default for InModule {
    fn default() -> Self {
        // SAFETY: every field is an integer, raw pointer or Option<fn>; all-zero is valid.
        unsafe { core::mem::zeroed() }
    }
}

layout_asserts!(InModule, size = 0x98, {
    version = 0x00, description = 0x04, main_window = 0x08, dll_instance = 0x0C,
    file_extensions = 0x10, seekable = 0x14, flags = 0x18, config = 0x1C, about = 0x20,
    init = 0x24, quit = 0x28, get_file_info = 0x2C, info_box = 0x30, is_our_file = 0x34,
    play = 0x38, pause = 0x3C, unpause = 0x40, is_paused = 0x44, stop = 0x48,
    get_length = 0x4C, get_position = 0x50, seek = 0x54, set_volume = 0x58, set_pan = 0x5C,
    analyzer_init = 0x60, analyzer_deinit = 0x64, analyzer_feed_pcm = 0x68,
    analyzer_get_mode = 0x6C, analyzer_feed_rows = 0x70, vis_feed_pcm = 0x74,
    vis_get_mode = 0x78, vis_feed_frame = 0x7C, vis_set_format = 0x80, dsp_is_active = 0x84,
    dsp_process = 0x88, eq_set = 0x8C, set_info = 0x90, out_module = 0x94,
});
layout_asserts!(InModuleExt, size = 0x9C, { base = 0x00, host_services = 0x98 });
