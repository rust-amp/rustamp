// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # Classic Winamp 2.95 visualization plugin wrapper for projectM 4.1.8
//!
//! Implements the S09 §5 classic plugin ABI:
//! - Entry point: `winampVisGetHeader` exporting [`VisHeader`].
//! - Version: `0x101` ([`VIS_VERSION`]).
//! - Module: [`VisModule`] with exact 2360-byte layout (S09 §5.2).
//! - Waveform audio feed: 2 rows of 576 bytes adapted to projectM stereo PCM.
//! - Callbacks: `config`, `init`, `render`, `quit`.

use core::ffi::{c_char, c_int, c_void};
use std::sync::Mutex;

use rustamp_plugin_abi::common::{Hinstance, Hwnd, VIS_MAX_ROWS, VIS_ROW_LEN, VIS_VERSION};
use rustamp_plugin_abi::vis::{VisHeader, VisModule};

use super::audio::PcmAudioAdapter;
use super::ffi::{ProjectMApi, ProjectmHandle};

/// Static plugin and module descriptions (ANSI null-terminated).
static PLUGIN_DESCRIPTION: &[u8] = b"projectM Visualizer 4.1.8\0";
static MODULE_DESCRIPTION: &[u8] = b"projectM 4.1.8 MilkDrop visualizer\0";

/// Internal runtime state for an active visualization session.
pub struct ProjectMState {
    pub parent_window: Hwnd,
    pub dll_instance: Hinstance,
    pub sample_rate: i32,
    pub channels: i32,
    pub api: Option<ProjectMApi>,
    pub handle: Option<ProjectmHandle>,
    pub audio_adapter: PcmAudioAdapter,
    pub width: usize,
    pub height: usize,
    pub frames_rendered: u64,
}

impl ProjectMState {
    pub fn new(
        parent_window: Hwnd,
        dll_instance: Hinstance,
        sample_rate: i32,
        channels: i32,
    ) -> Self {
        Self {
            parent_window,
            dll_instance,
            sample_rate,
            channels,
            api: None,
            handle: None,
            audio_adapter: PcmAudioAdapter::new(),
            width: 640,
            height: 480,
            frames_rendered: 0,
        }
    }
}

/// Global module representation with S09 §5.2 2360-byte layout.
static mut PROJECTM_MODULE: VisModule = VisModule {
    description: MODULE_DESCRIPTION.as_ptr() as *const c_char,
    parent_window: core::ptr::null_mut(),
    dll_instance: core::ptr::null_mut(),
    sample_rate: 44100,
    channels: 2,
    latency_ms: 0,
    delay_ms: 10,
    spectrum_rows: 0, // projectM computes FFT from waveform; 0 spectrum rows requested
    waveform_rows: 2, // stereo waveform rows wanted
    spectrum_data: [[0u8; VIS_ROW_LEN]; VIS_MAX_ROWS],
    waveform_data: [[0u8; VIS_ROW_LEN]; VIS_MAX_ROWS],
    config: Some(projectm_vis_config),
    init: Some(projectm_vis_init),
    render: Some(projectm_vis_render),
    quit: Some(projectm_vis_quit),
    user_data: core::ptr::null_mut(),
};

/// Global plugin header with S09 §5.1 layout.
static mut PROJECTM_HEADER: VisHeader = VisHeader {
    version: VIS_VERSION,
    description: PLUGIN_DESCRIPTION.as_ptr() as *const c_char,
    get_module: Some(projectm_vis_get_module),
};

static MODULE_LOCK: Mutex<()> = Mutex::new(());

/// Returns the visualization module at `index` (0-based).
///
/// Only index 0 exists for this plugin. Returns null for index >= 1.
///
/// # Safety
///
/// The caller must be the host visualization manager adhering to the S09 §5
/// plugin lifecycle.
pub unsafe extern "C" fn projectm_vis_get_module(index: c_int) -> *mut VisModule {
    if index == 0 {
        &raw mut PROJECTM_MODULE
    } else {
        core::ptr::null_mut()
    }
}

/// Winamp classic visualization entry point.
///
/// Resolves and returns the plugin's [`VisHeader`].
///
/// # Safety
///
/// The caller must treat the returned pointer as static and not free it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn winampVisGetHeader() -> *mut VisHeader {
    &raw mut PROJECTM_HEADER
}

/// Module configuration dialog callback.
///
/// # Safety
///
/// If `module` is non-null, it must point to a valid [`VisModule`].
pub unsafe extern "C" fn projectm_vis_config(module: *mut VisModule) {
    if !module.is_null() {
        // Configuration stub: in full host mode, presents preset/display dialog
    }
}

/// Module initialization callback.
///
/// Returns 0 on success; non-zero aborts startup.
///
/// # Safety
///
/// `module` must point to an initialized [`VisModule`] conforming to S09 §5.2.
pub unsafe extern "C" fn projectm_vis_init(module: *mut VisModule) -> c_int {
    let _guard = match MODULE_LOCK.lock() {
        Ok(g) => g,
        Err(_) => return 1,
    };

    if module.is_null() {
        return 1;
    }

    let m = unsafe { &mut *module };
    let mut state = Box::new(ProjectMState::new(
        m.parent_window,
        m.dll_instance,
        if m.sample_rate > 0 {
            m.sample_rate
        } else {
            44100
        },
        if m.channels > 0 { m.channels } else { 2 },
    ));

    // Attempt to locate and load projectM-4.dll dynamically from Plugins/projectM/
    #[cfg(windows)]
    {
        let candidate_paths = [
            std::path::PathBuf::from("Plugins/projectM/projectM-4.dll"),
            std::path::PathBuf::from("projectM-4.dll"),
        ];

        for path in &candidate_paths {
            if !path.exists() {
                continue;
            }
            if let Ok(api) = unsafe { ProjectMApi::load_dynamic(path) } {
                let handle = unsafe { (api.create)() };
                if !handle.is_null() {
                    unsafe {
                        (api.set_window_size)(handle, state.width, state.height);
                        (api.set_fps)(handle, 60);
                        (api.set_preset_duration)(handle, 15.0);
                    }
                    state.handle = Some(handle);
                }
                state.api = Some(api);
                break;
            }
        }
    }

    m.user_data = Box::into_raw(state) as *mut c_void;
    0 // 0 = success
}

/// Module frame render callback.
///
/// Feeds audio waveform data into projectM and renders one frame.
/// Returns 0 to continue visualization, non-zero to end.
///
/// # Safety
///
/// `module` must point to a valid, initialized [`VisModule`] whose `waveform_data`
/// contains the current audio frame.
pub unsafe extern "C" fn projectm_vis_render(module: *mut VisModule) -> c_int {
    if module.is_null() {
        return 1;
    }

    let m = unsafe { &mut *module };
    if m.user_data.is_null() {
        return 0;
    }

    let state = unsafe { &mut *(m.user_data as *mut ProjectMState) };
    state.frames_rendered = state.frames_rendered.wrapping_add(1);

    // Audio adaptation: convert waveform data to float PCM and pass to projectM
    let (pcm_samples, channels) = state
        .audio_adapter
        .convert_waveform(&m.waveform_data, state.channels);

    if let (Some(api), Some(handle)) = (&state.api, state.handle) {
        unsafe {
            // Feed PCM audio: 576 samples per channel
            (api.pcm_add_float)(handle, pcm_samples.as_ptr(), VIS_ROW_LEN as u32, channels);

            // Render visualization frame
            (api.opengl_render_frame)(handle);
        }
    }

    0 // 0 = continue
}

/// Module termination callback.
///
/// Releases projectM resources and frees internal state.
///
/// # Safety
///
/// `module` must point to a valid [`VisModule`] previously initialized by [`projectm_vis_init`].
pub unsafe extern "C" fn projectm_vis_quit(module: *mut VisModule) {
    let _guard = match MODULE_LOCK.lock() {
        Ok(g) => g,
        Err(_) => return,
    };

    if module.is_null() {
        return;
    }

    let m = unsafe { &mut *module };
    if !m.user_data.is_null() {
        let mut state = unsafe { Box::from_raw(m.user_data as *mut ProjectMState) };
        if let (Some(api), Some(handle)) = (&state.api, state.handle.take()) {
            unsafe {
                (api.destroy)(handle);
            }
        }
        m.user_data = core::ptr::null_mut();
    }
}
