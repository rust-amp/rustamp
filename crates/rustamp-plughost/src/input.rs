//! The input (`in_`) plugin bridge (S09 section 3).
//!
//! A classic input plugin is a 32-bit DLL that exports `winampGetInModule2`
//! and hands out its static `In_Module` (S09 §2, §3.1). The host fills the
//! members the plugin cannot fill itself — the main window, the plugin's own
//! module handle, the analyzer, visualization and DSP callbacks and
//! `set_info` — calls `init`, and then drives the plugin's playback through
//! `play`, `pause`, `unpause`, `seek` and `stop` (S09 §3.1, §3.5).
//!
//! # What this module does
//!
//! * report the host-visible state of a loaded input module: the slots the
//!   plugin supplied and the members the host filled (`module_info`);
//! * call the plugin's playback and state callbacks through the real cdecl
//!   ABI, refusing a null slot instead of calling through it (S09 §0);
//! * hand the plugin the ANSI path it was given, byte for byte
//!   (S09 §3.1 offset 0x38).
//!
//! # What this module does not do
//!
//! * **It does not decode or play audio.** The host has no audio path into a
//!   classic input plugin: the plugin's own decode thread, the output
//!   plugin it drives and the analyzer and visualization data are not here
//!   (S09 §3.6, §3.7). What the host drives is the plugin's *lifecycle* —
//!   `play`, `pause`, `unpause`, `stop` — and its state answers.
//! * **It does not select an output plugin.** `out_module` (S09 §3.1 offset
//!   0x94) stays empty here: the selection is the `config_outname` setting,
//!   which is S12's business, and a plugin with flag bit 0 clear (S09 §3.3)
//!   drives no output anyway. The hand-over itself — writing the selected
//!   output into that member before `play` — lives in [`crate::handoff`] and
//!   is driven by [`crate::registry::PluginRegistry::play_input`].
//! * **It does not forward volume, pan or the equalizer.** The host has no
//!   path from the player's state to a classic plugin yet, so those slots
//!   stay untouched. The player's levels reach the *output* plugin through
//!   the hand-over (S09 §3.5 step 2), not through these slots.
//!
//! * **It forwards a seek, and nothing else of the seek.** S09 §3.1
//!   offset 0x54 gives the input module a `seek(int ms)` slot, and S09
//!   §3.6 lists `seek` among the calls the host makes on the UI thread. The
//!   host calls that slot and stops there: the plugin signals its own
//!   decode thread to seek and then flushes the output at the new time
//!   itself (S09 §3.1 offset 0x54, §3.7). The host calls no output
//!   callback of its own, or the output would be flushed twice.
//!
//! * **It forwards a stop, and nothing of the closing.** S09 §3.1 offset
//!   0x48 gives the input module a `stop()` slot, and S09 §3.5 step 6 says
//!   what the host does with it: "User stop: the host calls `stop()`". The
//!   rest of the step — the plugin stops its decode thread, closes the
//!   output it drives and calls the host's `analyzer_deinit` — is the
//!   plugin's own work (S09 §3.1 offset 0x48, §3.5 step 6). The host calls
//!   no output callback of its own, or the output would be closed twice.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::ffi::{c_char, c_int};
use std::fmt;
use std::path::Path;

use rustamp_plugin_abi::PluginKind;
use rustamp_plugin_abi::input::InModule;

/// Why a call into an input plugin was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputError {
    /// The loaded plugin is not an input plugin, so the host has no input ABI
    /// to call.
    WrongKind(PluginKind),
    /// The plugin is not live: `quit` already ran, so there is nothing left
    /// to drive.
    NotInitialized,
    /// The plugin's module pointer is null. The loader refuses a null module
    /// at load time (S09 §3.1), so this is a broken plugin, not a fact about
    /// its ABI.
    NullModule,
    /// The plugin supplied no such callback. S09 requires no plugin to
    /// provide one (S09 §0), so its absence is a fact about that plugin and
    /// never a verdict on its validity.
    MissingCallback(&'static str),
    /// The plugin's `play` refused: S09 §3.1 offset 0x38 — 0 is success, -1
    /// is "file not found", any other nonzero is another error that stops
    /// the playlist advance.
    PlayRefused(i32),
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongKind(kind) => {
                write!(
                    f,
                    "the loaded plugin is a {kind:?} plugin, not an input plugin"
                )
            }
            Self::NotInitialized => f.write_str("the plugin is not live; nothing was called"),
            Self::NullModule => {
                f.write_str("the plugin's module pointer is null; nothing was called")
            }
            Self::MissingCallback(name) => {
                write!(
                    f,
                    "the plugin supplied no '{name}' callback; nothing was called"
                )
            }
            Self::PlayRefused(code) => {
                write!(f, "the plugin's play refused with code {code}")
            }
        }
    }
}

impl std::error::Error for InputError {}

/// Host-visible state of a loaded input module (S09 §3.1).
///
/// This is what an acceptance runner checks **before** it calls anything: the
/// slots the plugin supplied, the members the host filled before `init` and
/// whether `init` actually ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InModuleInfo {
    /// The plugin supplied a `config` callback (S09 §3.1 offset 0x1C).
    pub has_config: bool,
    /// The plugin supplied an `init` callback (S09 §3.1 offset 0x24).
    pub has_init: bool,
    /// The plugin supplied a `quit` callback (S09 §3.1 offset 0x28).
    pub has_quit: bool,
    /// The plugin supplied a `play` callback (S09 §3.1 offset 0x38).
    pub has_play: bool,
    /// The plugin supplied a `stop` callback (S09 §3.1 offset 0x48).
    pub has_stop: bool,
    /// The host filled the analyzer callbacks (S09 §3.1 offsets 0x60–0x70).
    pub has_analyzer: bool,
    /// The host filled the visualization callbacks (S09 §3.1 offsets 0x74–0x80).
    pub has_vis: bool,
    /// The host filled the DSP callbacks (S09 §3.1 offsets 0x84, 0x88).
    pub has_dsp: bool,
    /// The host filled `set_info` (S09 §3.1 offset 0x90). It is set **after**
    /// `init` returned (S09 §3.5 line 155), so it is filled here.
    pub has_set_info: bool,
    /// The host filled `out_module` (S09 §3.1 offset 0x94). It stays empty
    /// until the hand-over before `play` writes the selected output plugin
    /// into it (S09 §3.5 step 2, §3.7); see [`crate::handoff`].
    pub has_out_module: bool,
    /// `main_window` as filled by the host before `init` (S09 §3.1 offset
    /// 0x08).
    pub main_window: usize,
    /// `dll_instance` as filled by the host before `init` (S09 §3.1 offset
    /// 0x0C).
    pub dll_instance: usize,
    /// `init` was actually called. False means the slot was null and skipped.
    pub init_called: bool,
}

/// The host-visible state of a loaded input module.
///
/// # Safety
///
/// `module` must point at a live `In_Module` in the mapped plugin's static
/// storage (S09 §0).
pub(crate) fn module_info(module: *mut InModule, init_called: bool) -> InModuleInfo {
    // SAFETY: the module pointer was verified non-null when the plugin was
    // loaded and points into the retained mapping's static data.
    let m = unsafe { *module };
    InModuleInfo {
        has_config: m.config.is_some(),
        has_init: m.init.is_some(),
        has_quit: m.quit.is_some(),
        has_play: m.play.is_some(),
        has_stop: m.stop.is_some(),
        has_analyzer: m.analyzer_init.is_some(),
        has_vis: m.vis_feed_pcm.is_some(),
        has_dsp: m.dsp_is_active.is_some(),
        has_set_info: m.set_info.is_some(),
        has_out_module: !m.out_module.is_null(),
        main_window: m.main_window as usize,
        dll_instance: m.dll_instance as usize,
        init_called,
    }
}

/// Call the plugin's `play` (S09 §3.1 offset 0x38).
///
/// The plugin's own answer is the host's answer: 0 is success, -1 is "file
/// not found" and any other nonzero is another error that stops the playlist
/// advance (S09 §3.1 offset 0x38).
///
/// # Safety
///
/// `module` must point at a live `In_Module` in the mapped plugin's static
/// storage (S09 §0).
pub(crate) fn call_play(module: *mut InModule, path: &Path) -> Result<(), InputError> {
    // SAFETY: the module pointer was verified non-null when the plugin was
    // loaded.
    let Some(play) = (unsafe { (*module).play }) else {
        return Err(InputError::MissingCallback("play"));
    };
    let bytes = ansi_path(path);
    // SAFETY: cdecl, one ANSI path argument (S09 §3.1 offset 0x38), a
    // non-null slot, and a NUL-terminated buffer that is alive for the call.
    let code = unsafe { play(bytes.as_ptr().cast::<c_char>()) };
    if code == 0 {
        Ok(())
    } else {
        Err(InputError::PlayRefused(code))
    }
}

/// Call the plugin's `stop` (S09 §3.1 offset 0x48).
///
/// The plugin's duty is the whole of the step: it stops its decode thread,
/// closes the output it drives and calls the host's `analyzer_deinit` before
/// it returns (S09 §3.1 offset 0x48, §3.5 step 6). The host calls no output
/// callback of its own here — the output's `close` is the plugin's call
/// (S09 §3.6, §3.7, §4.2 step 3).
///
/// # Safety
///
/// `module` must point at a live `In_Module` in the mapped plugin's static
/// storage (S09 §0).
pub(crate) fn call_stop(module: *mut InModule) -> Result<(), InputError> {
    // SAFETY: the module pointer was verified non-null when the plugin was
    // loaded.
    let Some(stop) = (unsafe { (*module).stop }) else {
        return Err(InputError::MissingCallback("stop"));
    };
    // SAFETY: cdecl, no arguments (S09 §3.1 offset 0x48), non-null slot.
    unsafe { stop() };
    Ok(())
}

/// Call the plugin's `pause` (S09 §3.1 offset 0x3C).
///
/// # Safety
///
/// `module` must point at a live `In_Module` in the mapped plugin's static
/// storage (S09 §0).
pub(crate) fn call_pause(module: *mut InModule) -> Result<(), InputError> {
    // SAFETY: the module pointer was verified non-null when the plugin was
    // loaded.
    let Some(pause) = (unsafe { (*module).pause }) else {
        return Err(InputError::MissingCallback("pause"));
    };
    // SAFETY: cdecl, no arguments (S09 §3.1 offset 0x3C), non-null slot.
    unsafe { pause() };
    Ok(())
}

/// Call the plugin's `unpause` (S09 §3.1 offset 0x40).
///
/// # Safety
///
/// `module` must point at a live `In_Module` in the mapped plugin's static
/// storage (S09 §0).
pub(crate) fn call_unpause(module: *mut InModule) -> Result<(), InputError> {
    // SAFETY: the module pointer was verified non-null when the plugin was
    // loaded.
    let Some(unpause) = (unsafe { (*module).unpause }) else {
        return Err(InputError::MissingCallback("unpause"));
    };
    // SAFETY: cdecl, no arguments (S09 §3.1 offset 0x40), non-null slot.
    unsafe { unpause() };
    Ok(())
}

/// Ask the plugin whether it is paused (S09 §3.1 offset 0x44).
///
/// # Safety
///
/// `module` must point at a live `In_Module` in the mapped plugin's static
/// storage (S09 §0).
pub(crate) fn call_is_paused(module: *mut InModule) -> Result<bool, InputError> {
    // SAFETY: the module pointer was verified non-null when the plugin was
    // loaded.
    let Some(is_paused) = (unsafe { (*module).is_paused }) else {
        return Err(InputError::MissingCallback("is_paused"));
    };
    // SAFETY: cdecl, no arguments, returns an int (S09 §3.1 offset 0x44),
    // non-null slot.
    Ok(unsafe { is_paused() } != 0)
}

/// Ask the plugin for the track length in milliseconds (S09 §3.1 offset 0x4C).
///
/// # Safety
///
/// `module` must point at a live `In_Module` in the mapped plugin's static
/// storage (S09 §0).
pub(crate) fn call_get_length(module: *mut InModule) -> Result<c_int, InputError> {
    // SAFETY: the module pointer was verified non-null when the plugin was
    // loaded.
    let Some(get_length) = (unsafe { (*module).get_length }) else {
        return Err(InputError::MissingCallback("get_length"));
    };
    // SAFETY: cdecl, no arguments, returns an int (S09 §3.1 offset 0x4C),
    // non-null slot.
    Ok(unsafe { get_length() })
}

/// Ask the plugin for the position in milliseconds (S09 §3.1 offset 0x50).
///
/// # Safety
///
/// `module` must point at a live `In_Module` in the mapped plugin's static
/// storage (S09 §0).
pub(crate) fn call_get_position(module: *mut InModule) -> Result<c_int, InputError> {
    // SAFETY: the module pointer was verified non-null when the plugin was
    // loaded.
    let Some(get_position) = (unsafe { (*module).get_position }) else {
        return Err(InputError::MissingCallback("get_position"));
    };
    // SAFETY: cdecl, no arguments, returns an int (S09 §3.1 offset 0x50),
    // non-null slot.
    Ok(unsafe { get_position() })
}

/// Call the plugin's `seek` (S09 §3.1 offset 0x54).
///
/// The target is the plugin's own millisecond value. S09 names no range for
/// it, so the host forwards the number unchanged and lets the plugin decide
/// what it means (S09 §3.1 offset 0x54: "Seek. Normally signals the decode
/// thread to seek and then flushes the output at the new time").
///
/// This is the **whole** of the host's part of a seek. The output's
/// `flush(time_ms)` is the plugin's call, not the host's (S09 §3.7: "On
/// seeking the plugin calls the output `flush(time_ms)`"; S09 §4.2 step 3:
/// the output's `flush` comes "from the UI or seek path through the input
/// plugin"). A host-side flush would flush the output a second time.
///
/// # Safety
///
/// `module` must point at a live `In_Module` in the mapped plugin's static
/// storage (S09 §0).
pub(crate) fn call_seek(module: *mut InModule, time_ms: c_int) -> Result<(), InputError> {
    // SAFETY: the module pointer was verified non-null when the plugin was
    // loaded.
    let Some(seek) = (unsafe { (*module).seek }) else {
        return Err(InputError::MissingCallback("seek"));
    };
    // SAFETY: cdecl, one int argument in milliseconds (S09 §3.1 offset
    // 0x54), non-null slot.
    unsafe { seek(time_ms) };
    Ok(())
}

/// The ANSI form of a path, NUL-terminated (S09 §1, §3.1 offset 0x38).
///
/// The plugin ABI takes ANSI strings. On Windows the byte form of a path is
/// what the `…A` calls take, so the path's own bytes are handed over
/// unchanged and NUL-terminated. A path that is not representable in the
/// system ANSI code page reaches the plugin as its UTF-8 bytes: S09 does not
/// say what a 2.95 host does with such a path, and inventing a code-page
/// conversion here would be a guess.
fn ansi_path(path: &Path) -> Vec<u8> {
    let mut bytes = path.as_os_str().as_encoded_bytes().to_vec();
    bytes.push(0);
    bytes
}
