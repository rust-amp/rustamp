// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # projectM 4.1.8 C-API FFI declarations
//!
//! Direct C ABI bindings to `libprojectM` 4.1.8 (`projectM.h`, `core.h`, `audio.h`,
//! `render_opengl.h`, `parameters.h`, `memory.h`).
//!
//! These declarations match the upstream C API headers vendored under
//! `third_party/projectm/src/api/include/projectM-4/`.

use core::ffi::{c_char, c_int, c_uint, c_void};

/// Opaque projectM instance handle (`typedef struct projectm* projectm_handle`).
pub type ProjectmHandle = *mut c_void;

/// Channel configuration for audio PCM data.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectmChannels {
    /// Single-channel mono.
    Mono = 1,
    /// Dual-channel stereo.
    Stereo = 2,
}

/// Channel index in PCM buffers.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectmPcmChannel {
    /// Left channel (channel 0).
    ChannelL = 0,
    /// Right channel (channel 1).
    ChannelR = 1,
}

// Function pointer types for dynamic loading / dispatch (LGPL-2.1 §6 relinking)
pub type FnProjectmCreate = unsafe extern "C" fn() -> ProjectmHandle;
pub type FnProjectmDestroy = unsafe extern "C" fn(instance: ProjectmHandle);
pub type FnProjectmLoadPresetFile = unsafe extern "C" fn(
    instance: ProjectmHandle,
    filename: *const c_char,
    smooth_transition: bool,
);
pub type FnProjectmLoadPresetData =
    unsafe extern "C" fn(instance: ProjectmHandle, data: *const c_char, smooth_transition: bool);
pub type FnProjectmResetTextures = unsafe extern "C" fn(instance: ProjectmHandle);
pub type FnProjectmGetVersionComponents =
    unsafe extern "C" fn(major: *mut c_int, minor: *mut c_int, patch: *mut c_int);
pub type FnProjectmGetVersionString = unsafe extern "C" fn() -> *mut c_char;
pub type FnProjectmFreeString = unsafe extern "C" fn(str: *const c_char);
pub type FnProjectmPcmGetMaxSamples = unsafe extern "C" fn() -> c_uint;
pub type FnProjectmPcmAddFloat = unsafe extern "C" fn(
    instance: ProjectmHandle,
    samples: *const f32,
    count: c_uint,
    channels: ProjectmChannels,
);
pub type FnProjectmPcmAddInt16 = unsafe extern "C" fn(
    instance: ProjectmHandle,
    samples: *const i16,
    count: c_uint,
    channels: ProjectmChannels,
);
pub type FnProjectmPcmAddUint8 = unsafe extern "C" fn(
    instance: ProjectmHandle,
    samples: *const u8,
    count: c_uint,
    channels: ProjectmChannels,
);
pub type FnProjectmOpenglRenderFrame = unsafe extern "C" fn(instance: ProjectmHandle);
pub type FnProjectmSetWindowSize =
    unsafe extern "C" fn(instance: ProjectmHandle, width: usize, height: usize);
pub type FnProjectmGetWindowSize =
    unsafe extern "C" fn(instance: ProjectmHandle, width: *mut usize, height: *mut usize);
pub type FnProjectmSetFps = unsafe extern "C" fn(instance: ProjectmHandle, fps: i32);
pub type FnProjectmGetFps = unsafe extern "C" fn(instance: ProjectmHandle) -> i32;
pub type FnProjectmSetBeatSensitivity =
    unsafe extern "C" fn(instance: ProjectmHandle, sensitivity: f32);
pub type FnProjectmGetBeatSensitivity = unsafe extern "C" fn(instance: ProjectmHandle) -> f32;
pub type FnProjectmSetHardCutDuration =
    unsafe extern "C" fn(instance: ProjectmHandle, seconds: f64);
pub type FnProjectmGetHardCutDuration = unsafe extern "C" fn(instance: ProjectmHandle) -> f64;
pub type FnProjectmSetHardCutEnabled =
    unsafe extern "C" fn(instance: ProjectmHandle, enabled: bool);
pub type FnProjectmGetHardCutEnabled = unsafe extern "C" fn(instance: ProjectmHandle) -> bool;
pub type FnProjectmSetSoftCutDuration =
    unsafe extern "C" fn(instance: ProjectmHandle, seconds: f64);
pub type FnProjectmGetSoftCutDuration = unsafe extern "C" fn(instance: ProjectmHandle) -> f64;
pub type FnProjectmSetPresetDuration = unsafe extern "C" fn(instance: ProjectmHandle, seconds: f64);
pub type FnProjectmGetPresetDuration = unsafe extern "C" fn(instance: ProjectmHandle) -> f64;
pub type FnProjectmSetPresetLocked = unsafe extern "C" fn(instance: ProjectmHandle, lock: bool);
pub type FnProjectmGetPresetLocked = unsafe extern "C" fn(instance: ProjectmHandle) -> bool;
pub type FnProjectmSetEasterEgg = unsafe extern "C" fn(instance: ProjectmHandle, value: f32);
pub type FnProjectmGetEasterEgg = unsafe extern "C" fn(instance: ProjectmHandle) -> f32;

/// Dynamically resolved or statically linked projectM 4.1.8 C API table.
///
/// Supports LGPL-2.1 §6 relinking by resolving functions dynamically from
/// `projectM-4.dll` at runtime when the DLL is deployed beside `vis_projectm.dll`.
#[derive(Debug, Clone)]
pub struct ProjectMApi {
    pub create: FnProjectmCreate,
    pub destroy: FnProjectmDestroy,
    pub load_preset_file: FnProjectmLoadPresetFile,
    pub load_preset_data: FnProjectmLoadPresetData,
    pub reset_textures: FnProjectmResetTextures,
    pub get_version_components: FnProjectmGetVersionComponents,
    pub get_version_string: FnProjectmGetVersionString,
    pub free_string: FnProjectmFreeString,
    pub pcm_get_max_samples: FnProjectmPcmGetMaxSamples,
    pub pcm_add_float: FnProjectmPcmAddFloat,
    pub pcm_add_int16: FnProjectmPcmAddInt16,
    pub pcm_add_uint8: FnProjectmPcmAddUint8,
    pub opengl_render_frame: FnProjectmOpenglRenderFrame,
    pub set_window_size: FnProjectmSetWindowSize,
    pub get_window_size: FnProjectmGetWindowSize,
    pub set_fps: FnProjectmSetFps,
    pub get_fps: FnProjectmGetFps,
    pub set_beat_sensitivity: FnProjectmSetBeatSensitivity,
    pub get_beat_sensitivity: FnProjectmGetBeatSensitivity,
    pub set_hard_cut_duration: FnProjectmSetHardCutDuration,
    pub get_hard_cut_duration: FnProjectmGetHardCutDuration,
    pub set_hard_cut_enabled: FnProjectmSetHardCutEnabled,
    pub get_hard_cut_enabled: FnProjectmGetHardCutEnabled,
    pub set_soft_cut_duration: FnProjectmSetSoftCutDuration,
    pub get_soft_cut_duration: FnProjectmGetSoftCutDuration,
    pub set_preset_duration: FnProjectmSetPresetDuration,
    pub get_preset_duration: FnProjectmGetPresetDuration,
    pub set_preset_locked: FnProjectmSetPresetLocked,
    pub get_preset_locked: FnProjectmGetPresetLocked,
    pub set_easter_egg: FnProjectmSetEasterEgg,
    pub get_easter_egg: FnProjectmGetEasterEgg,
}

#[cfg(windows)]
impl ProjectMApi {
    /// Attempts to dynamically load `projectM-4.dll` using Windows `LoadLibraryW`.
    ///
    /// # Safety
    ///
    /// Calls Windows API `LoadLibraryW` and `GetProcAddress`.
    pub unsafe fn load_dynamic(dll_path: &std::path::Path) -> Result<Self, String> {
        use std::os::windows::ffi::OsStrExt;

        let wide: Vec<u16> = dll_path.as_os_str().encode_wide().chain(Some(0)).collect();

        unsafe extern "system" {
            fn LoadLibraryW(lpLibFileName: *const u16) -> *mut c_void;
            fn GetProcAddress(hModule: *mut c_void, lpProcName: *const c_char) -> *mut c_void;
        }

        let module = unsafe { LoadLibraryW(wide.as_ptr()) };
        if module.is_null() {
            return Err(format!("Failed to load library: {}", dll_path.display()));
        }

        macro_rules! resolve {
            ($name:ident, $sym:expr, $fn_type:ty) => {{
                let sym_cstr = concat!($sym, "\0");
                let ptr = unsafe { GetProcAddress(module, sym_cstr.as_ptr() as *const c_char) };
                if ptr.is_null() {
                    return Err(format!("Missing symbol '{}' in projectM DLL", $sym));
                }
                unsafe { core::mem::transmute::<*mut c_void, $fn_type>(ptr) }
            }};
        }

        Ok(Self {
            create: resolve!(create, "projectm_create", FnProjectmCreate),
            destroy: resolve!(destroy, "projectm_destroy", FnProjectmDestroy),
            load_preset_file: resolve!(
                load_preset_file,
                "projectm_load_preset_file",
                FnProjectmLoadPresetFile
            ),
            load_preset_data: resolve!(
                load_preset_data,
                "projectm_load_preset_data",
                FnProjectmLoadPresetData
            ),
            reset_textures: resolve!(
                reset_textures,
                "projectm_reset_textures",
                FnProjectmResetTextures
            ),
            get_version_components: resolve!(
                get_version_components,
                "projectm_get_version_components",
                FnProjectmGetVersionComponents
            ),
            get_version_string: resolve!(
                get_version_string,
                "projectm_get_version_string",
                FnProjectmGetVersionString
            ),
            free_string: resolve!(free_string, "projectm_free_string", FnProjectmFreeString),
            pcm_get_max_samples: resolve!(
                pcm_get_max_samples,
                "projectm_pcm_get_max_samples",
                FnProjectmPcmGetMaxSamples
            ),
            pcm_add_float: resolve!(
                pcm_add_float,
                "projectm_pcm_add_float",
                FnProjectmPcmAddFloat
            ),
            pcm_add_int16: resolve!(
                pcm_add_int16,
                "projectm_pcm_add_int16",
                FnProjectmPcmAddInt16
            ),
            pcm_add_uint8: resolve!(
                pcm_add_uint8,
                "projectm_pcm_add_uint8",
                FnProjectmPcmAddUint8
            ),
            opengl_render_frame: resolve!(
                opengl_render_frame,
                "projectm_opengl_render_frame",
                FnProjectmOpenglRenderFrame
            ),
            set_window_size: resolve!(
                set_window_size,
                "projectm_set_window_size",
                FnProjectmSetWindowSize
            ),
            get_window_size: resolve!(
                get_window_size,
                "projectm_get_window_size",
                FnProjectmGetWindowSize
            ),
            set_fps: resolve!(set_fps, "projectm_set_fps", FnProjectmSetFps),
            get_fps: resolve!(get_fps, "projectm_get_fps", FnProjectmGetFps),
            set_beat_sensitivity: resolve!(
                set_beat_sensitivity,
                "projectm_set_beat_sensitivity",
                FnProjectmSetBeatSensitivity
            ),
            get_beat_sensitivity: resolve!(
                get_beat_sensitivity,
                "projectm_get_beat_sensitivity",
                FnProjectmGetBeatSensitivity
            ),
            set_hard_cut_duration: resolve!(
                set_hard_cut_duration,
                "projectm_set_hard_cut_duration",
                FnProjectmSetHardCutDuration
            ),
            get_hard_cut_duration: resolve!(
                get_hard_cut_duration,
                "projectm_get_hard_cut_duration",
                FnProjectmGetHardCutDuration
            ),
            set_hard_cut_enabled: resolve!(
                set_hard_cut_enabled,
                "projectm_set_hard_cut_enabled",
                FnProjectmSetHardCutEnabled
            ),
            get_hard_cut_enabled: resolve!(
                get_hard_cut_enabled,
                "projectm_get_hard_cut_enabled",
                FnProjectmGetHardCutEnabled
            ),
            set_soft_cut_duration: resolve!(
                set_soft_cut_duration,
                "projectm_set_soft_cut_duration",
                FnProjectmSetSoftCutDuration
            ),
            get_soft_cut_duration: resolve!(
                get_soft_cut_duration,
                "projectm_get_soft_cut_duration",
                FnProjectmGetSoftCutDuration
            ),
            set_preset_duration: resolve!(
                set_preset_duration,
                "projectm_set_preset_duration",
                FnProjectmSetPresetDuration
            ),
            get_preset_duration: resolve!(
                get_preset_duration,
                "projectm_get_preset_duration",
                FnProjectmGetPresetDuration
            ),
            set_preset_locked: resolve!(
                set_preset_locked,
                "projectm_set_preset_locked",
                FnProjectmSetPresetLocked
            ),
            get_preset_locked: resolve!(
                get_preset_locked,
                "projectm_get_preset_locked",
                FnProjectmGetPresetLocked
            ),
            set_easter_egg: resolve!(
                set_easter_egg,
                "projectm_set_easter_egg",
                FnProjectmSetEasterEgg
            ),
            get_easter_egg: resolve!(
                get_easter_egg,
                "projectm_get_easter_egg",
                FnProjectmGetEasterEgg
            ),
        })
    }
}
