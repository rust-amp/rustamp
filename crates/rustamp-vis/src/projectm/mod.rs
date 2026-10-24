// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # projectM 4.1.8 classic visualization integration
//!
//! Provides clean-room C-API FFI declarations, audio adaptation, and the classic
//! Winamp 2.95 visualization plugin wrapper (`vis_projectm`) implementing S09 §5.

pub mod audio;
pub mod ffi;
pub mod wrapper;

pub use audio::PcmAudioAdapter;
pub use ffi::{ProjectMApi, ProjectmChannels, ProjectmHandle};
pub use wrapper::{
    projectm_vis_config, projectm_vis_get_module, projectm_vis_init, projectm_vis_quit,
    projectm_vis_render, winampVisGetHeader,
};
