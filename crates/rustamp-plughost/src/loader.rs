//! Classic Winamp 2.x plugin DLL loader.
//!
//! Implements the plugin loading path described by `specs/S09-plugin-abi.md` and
//! `docs/tasks/P3-dll-acceptance.md`:
//! 1. **Passive DLL preflight**: checks i386, PE32/DLL structure, bounded sections
//!    and a supported non-forwarded executable export, before any OS load.
//! 2. **Explicit byte approval**: requires full equality with independently
//!    approved bytes supplied by the caller. Never auto-approve a candidate.
//! 3. **Verified-byte snapshot**: revalidates the source, writes a unique private
//!    copy, compares its full bytes and pins its file and canonical ancestors
//!    against ordinary write/delete/rename operations through the OS load.
//!    The source pathname is never passed to `LoadLibraryExW`.
//!    Dependencies use the explicit source directory and System32, not CWD/PATH.
//! 4. **Entry point resolution**: locates the exported entry point (`winampGetInModule2`
//!    for input plugins, `winampDSPGetHeader2` for DSP plugins,
//!    `winampGetGeneralPurposePlugin` for general plugins,
//!    `winampGetOutModule` for output plugins; S09 §2).
//! 5. **Calling convention**: entry points and callbacks are `cdecl` (`extern "C"`,
//!    caller cleans stack; S09 §1 line 38).
//! 6. **Module validation**: verifies that the returned module pointer is non-null,
//!    validates the version field (`0x100` for input plugins per S09 §3.2 line 125,
//!    masking 5.x flags `0x0F00_0000` per S09 §3.2 line 127 and §11 line 437; `0x20`
//!    for DSP plugins per S09 §6.1 line 335, strict; `0x10` for
//!    general plugins per S09 §7.1 line 367, strict; `0x10` for
//!    output plugins per S09 §4.1 line 229, strict), and reads the ANSI
//!    description string (S09 §1 line 40).
//! 7. **Host fields**: fills `main_window`, `dll_instance`, and host callbacks
//!    (`analyzer_*`, `vis_*`, `dsp_*`, `out_module`) before `init()` (S09 §3.1 lines 76–115,
//!    §3.5 lines 153–156).
//! 8. **Plugin initialization**: calls the plugin's `init()` callback on the UI thread
//!    (S09 §3.5 line 156, §7.2 line 377). Sets `set_info` after `init()` returns (S09 §3.5 line 155).
//! 9. **Unload policy**: at shutdown, calls `quit()` on loaded plugins in reverse
//!    order (S09 §3.5 line 166, §7.2 line 380, §4.2 line 255); input, output and
//!    general DLLs are NOT freed at shutdown because plugins may have subclassed
//!    windows (S09 §1 line 46).
//!    Reverse order across several plugins is [`crate::registry::PluginRegistry`]'s
//!    job; a single [`LoadedPlugin`] only quits itself.
//! 10. **Configuration**: [`LoadedPlugin::config`] calls the `config` callback
//!     of a loaded plugin of **any** kind on the UI thread (S09 §1 "Config /
//!     About"). Every kind has such a slot, and the host calls it with the
//!     arguments that kind's slot takes: the input and output kinds take the
//!     parent window handle (S09 §3.1 offset 0x1C, §4.1 offset 0x14), the
//!     general kind takes no argument and reads the parent window from its own
//!     struct (S09 §7.1 offset 0x0C, offset 0x14), and the DSP and
//!     visualization kinds take their module pointer (S09 §6.2 offset 0x0C,
//!     §5.2 offset 0x924). A null slot is refused, never transmuted, and a
//!     plugin that is not live is refused: S09 requires no plugin to provide
//!     a `config` callback, so its absence is a fact about that plugin and
//!     never a verdict on its validity.
//! 11. **Input plugins**: the host fills `main_window`, `dll_instance` and the
//!     host callbacks (the analyzer, visualization and DSP groups and
//!     `set_info`) before `init`, and clears `set_info` first and fills it
//!     again after `init` returned (S09 §3.5 lines 153–156). `out_module`
//!     stays empty: no output plugin is selected, and the selection is the
//!     `config_outname` setting, which is S12's business (S09 §3.1 offset
//!     0x94). The host then drives the plugin's playback — `play`, `pause`,
//!     `unpause`, `stop` — and reads its own state answers (`is_paused`,
//!     `get_length`, `get_position`); see [`crate::input`]. The host has no
//!     audio path into a classic input plugin: the plugin's decode thread,
//!     the output plugin it drives and the analyzer data are not here yet
//!     (S09 §3.6, §3.7).
//! 12. **Output plugins**: the host fills `main_window` and `dll_instance` in
//!     the `Out_Module` and calls `init` (S09 §4.1 lines 233–237, §4.2 line 252).
//!     The host calls **no other** output callback: `open`, `write`,
//!     `can_write`, `is_playing`, `pause`, `set_volume`, `set_pan`, `flush`
//!     and the two time queries are driven by the **input** plugin from its
//!     decode thread (S09 §4.2 step 3, §3.7), and the host has no playback
//!     path of its own. The `id` member is left untouched: the 5.x host
//!     replaces it with the DLL file name (5.x-only), and whether 2.95 does is
//!     open (S09 U-4).
//! 13. **Visualization plugins**: the host resolves `winampVisGetHeader`,
//!     validates the header and enumerates the library's modules (S09 §5.1,
//!     §5.3 step 2). A module is chosen and started later — the user picks
//!     one — so the host fills `parent_window`, `dll_instance`, `sample_rate`
//!     and `channels` into the chosen module and calls its `init` there
//!     (S09 §5.2, §5.3 step 2). The frame loop lives in [`crate::vis`]; the
//!     data that would fill the module's arrays comes from the audio path
//!     and does not exist yet.
//! 14. **DSP plugins**: the host resolves `winampDSPGetHeader2`, validates the
//!     header and enumerates the library's modules (S09 §6.1, §6.3 step 1).
//!     The selected module — the index the stored `config_dspplugin_num`
//!     names, the first module while no selection is persisted — gets the
//!     parent window and the plugin's own module handle and its `init` runs
//!     there (S09 §6.2, §6.3 step 1). The module list, the lifecycle and the
//!     `dsp_is_active`/`dsp_process` host callbacks live in [`crate::dsp`];
//!     the samples that would reach them come from the audio path, which does
//!     not exist yet.
//!
//! Structural preflight is NOT a malware detector or approval gate. A valid
//! trusted DLL and its dependencies can run TLS/DllMain during LoadLibraryExW,
//! before the getter result/version can be checked. See task P5 for limits.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::fmt;
use std::path::{Path, PathBuf};

use crate::fault::{PluginIdentity, plugin_call_scope};
use crate::host_window::HostWindow;
use crate::pe_preflight::PePreflightError;
use rustamp_plugin_abi::PluginKind;
#[cfg(any(windows, test))]
use rustamp_plugin_abi::common::OUT_VERSION;
#[cfg(any(windows, test))]
use rustamp_plugin_abi::common::{GEN_VERSION, read_c_string_lossy};
#[cfg(windows)]
use rustamp_plugin_abi::common::{
    IN_VERSION, IN_VERSION_FLAG_INIT_STATUS, IN_VERSION_FLAG_WIDE_FILENAMES,
};
#[cfg(any(windows, test))]
use rustamp_plugin_abi::dsp::{DspHeader, DspModule};
#[cfg(any(windows, test))]
use rustamp_plugin_abi::general::GeneralPlugin;
#[cfg(windows)]
use rustamp_plugin_abi::input::InModule;
#[cfg(any(windows, test))]
use rustamp_plugin_abi::output::OutModule;
#[cfg(any(windows, test))]
use rustamp_plugin_abi::vis::{VisHeader, VisModule};

#[cfg(any(windows, test))]
use crate::dsp::DspModuleEntry;
use crate::dsp::{DspError, DspLibraryInfo, DspRunInfo};
#[cfg(windows)]
use crate::input::{InModuleInfo, InputError};
use crate::vis::{VisError, VisLibraryInfo, VisModuleEntry, VisRunInfo};

mod image;
use image::PreparedImage;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExW(
        lpLibFileName: *const u16,
        hFile: *mut core::ffi::c_void,
        dwFlags: u32,
    ) -> *mut core::ffi::c_void;
    fn GetProcAddress(
        hModule: *mut core::ffi::c_void,
        lpProcName: *const u8,
    ) -> *mut core::ffi::c_void;
    fn GetLastError() -> u32;
    fn AddDllDirectory(path: *const u16) -> *mut core::ffi::c_void;
    fn RemoveDllDirectory(cookie: *mut core::ffi::c_void) -> i32;
}

// ---------------------------------------------------------------------------
// Host callbacks for InModule (S09 §3.1 lines 102–115, §3.5 lines 153–156)
// ---------------------------------------------------------------------------

#[cfg(windows)]
/// Host callback: `analyzer_init(max_latency_ms, sample_rate)` (S09 §3.1 offset 0x60).
unsafe extern "C" fn host_analyzer_init(
    _max_latency_ms: core::ffi::c_int,
    _sample_rate: core::ffi::c_int,
) {
}

#[cfg(windows)]
/// Host callback: `analyzer_deinit()` (S09 §3.1 offset 0x64).
unsafe extern "C" fn host_analyzer_deinit() {}

#[cfg(windows)]
/// Host callback: `analyzer_feed_pcm(pcm, channels, bits, timestamp_ms)` (S09 §3.1 offset 0x68).
unsafe extern "C" fn host_analyzer_feed_pcm(
    _pcm: *mut core::ffi::c_void,
    _channels: core::ffi::c_int,
    _bits: core::ffi::c_int,
    _timestamp_ms: core::ffi::c_int,
) {
}

#[cfg(windows)]
/// Host callback: `analyzer_get_mode()` -> 4 (none/window-shade; S09 §3.1 offset 0x6C line 105).
unsafe extern "C" fn host_analyzer_get_mode() -> core::ffi::c_int {
    4
}

#[cfg(windows)]
/// Host callback: `analyzer_feed_rows(rows, timestamp_ms, mode)` -> 1 (not collecting; S09 §3.1 offset 0x70 line 106).
unsafe extern "C" fn host_analyzer_feed_rows(
    _rows: *mut core::ffi::c_void,
    _timestamp_ms: core::ffi::c_int,
    _mode: core::ffi::c_int,
) -> core::ffi::c_int {
    1
}

#[cfg(windows)]
/// Host callback: `vis_feed_pcm(pcm, channels, bits, timestamp_ms)` (S09 §3.1 offset 0x74).
unsafe extern "C" fn host_vis_feed_pcm(
    _pcm: *mut core::ffi::c_void,
    _channels: core::ffi::c_int,
    _bits: core::ffi::c_int,
    _timestamp_ms: core::ffi::c_int,
) {
}

#[cfg(windows)]
/// Host callback: `vis_get_mode(spectrum_rows, waveform_rows)` (S09 §3.1 offset 0x78 line 108).
/// Answers for the running visualization: nonzero while one runs, and the row
/// counts it asked for. With none running both counts are zero.
unsafe extern "C" fn host_vis_get_mode(
    spectrum_rows: *mut core::ffi::c_int,
    waveform_rows: *mut core::ffi::c_int,
) -> core::ffi::c_int {
    crate::vis::vis_get_mode(spectrum_rows, waveform_rows)
}

#[cfg(windows)]
/// Host callback: `vis_feed_frame(frame, timestamp_ms)` -> 1 (not accepted; S09 §3.1 offset 0x7C line 109).
unsafe extern "C" fn host_vis_feed_frame(
    _frame: *mut core::ffi::c_void,
    _timestamp_ms: core::ffi::c_int,
) -> core::ffi::c_int {
    1
}

#[cfg(windows)]
/// Host callback: `vis_set_format(sample_rate, channels)` (S09 §3.1 offset 0x80 line 110).
/// Copies the announced stream format into the running visualization module
/// (S09 §3.8).
unsafe extern "C" fn host_vis_set_format(
    sample_rate: core::ffi::c_int,
    channels: core::ffi::c_int,
) {
    crate::vis::vis_set_format(sample_rate, channels);
}

#[cfg(any(windows, test))]
/// Host callback: `dsp_is_active()` (S09 §3.1 offset 0x84 line 111).
/// Returns 1 while a DSP module with a `process` callback is installed
/// (S09 §6.3: one module active at a time), else 0. No EQ stage lives here;
/// the EQ-vs-DSP order is an S09 open item (U-12).
unsafe extern "C" fn host_dsp_is_active() -> core::ffi::c_int {
    if crate::dsp::dsp_is_active() { 1 } else { 0 }
}

#[cfg(any(windows, test))]
/// Host callback: `dsp_process(...)` (S09 §3.1 offset 0x88 line 112).
/// Forwards the in-place buffer to the active DSP module on the input
/// plugin's decode thread (S09 §3.6) and returns the validated frame count.
/// With no active DSP the buffer is untouched and `frames` is returned.
unsafe extern "C" fn host_dsp_process(
    samples: *mut core::ffi::c_void,
    frames: core::ffi::c_int,
    bits: core::ffi::c_int,
    channels: core::ffi::c_int,
    sample_rate: core::ffi::c_int,
) -> core::ffi::c_int {
    // SAFETY: the buffer contract is the input plugin caller's; a missing
    // module or an out-of-range return falls back to `frames` inside.
    unsafe {
        crate::dsp::call_dsp_process(
            crate::dsp::active_dsp_module(),
            samples,
            frames,
            bits,
            channels,
            sample_rate,
        )
    }
}

#[cfg(windows)]
/// Host callback: `set_info(...)` (S09 §3.1 offset 0x90 line 114). Set after init returns (S09 §3.5 line 155).
unsafe extern "C" fn host_set_info(
    _bitrate_kbps: core::ffi::c_int,
    _sample_rate_khz: core::ffi::c_int,
    _channels: core::ffi::c_int,
    _synched: core::ffi::c_int,
) {
}

// ---------------------------------------------------------------------------
// General-purpose plugin initialization (S09 §7)
// ---------------------------------------------------------------------------

/// What actually happened while a general-purpose module was initialized.
///
/// `init_result` is the raw `int` the plugin returned, recorded and **not**
/// interpreted: the role of a non-zero return in 2.95 is S09 U-8 and stays
/// open (P3 §5.4). `init_called` is false when the plugin supplied no `init`
/// slot, so a skipped call is never reported as a successful initialization.
#[cfg(any(windows, test))]
#[derive(Debug)]
struct GeneralInit {
    module: *mut GeneralPlugin,
    init_called: bool,
    init_result: core::ffi::c_int,
}

/// Validate and initialize a general-purpose plugin module (S09 §7.1, §7.2).
///
/// Mirrors [`init_dsp_module`]: the version is checked, the two host-filled
/// members are written, and `init` is called — in that order, all on the
/// caller's thread, which must be the UI thread owning the host window
/// (S09 §7.2 line 377).
///
/// Callback **slots are reported, never enforced**: S09 §7 does not require a
/// general plugin to create a window, subclass, or provide `config`, so no
/// universal rule is invented here (P3 §3). Requiring the slots is the
/// acceptance runner's job, through [`LoadedPlugin::general_info`], before it
/// calls the matching callback. A null slot is never transmuted or called.
///
/// # Safety
///
/// `module` must be null or point at a live `winampGeneralPurposePlugin` in
/// the mapped plugin's static storage (S09 §0). `host_hwnd` must be the real
/// main-window handle and `hinstance` the plugin's own module handle. The
/// plugin's `init` runs here on the caller's thread.
#[cfg(any(windows, test))]
unsafe fn init_general_module(
    module: *mut GeneralPlugin,
    host_hwnd: usize,
    hinstance: *mut core::ffi::c_void,
) -> Result<GeneralInit, PluginLoadError> {
    if module.is_null() {
        return Err(PluginLoadError::NullModule);
    }
    // SAFETY: caller guarantees a live module for these reads.
    let version = unsafe { (*module).version };
    // S09 §7.1 line 367: 0x10 in 2.95; 0x11 is 5.x-only. Checked before any
    // host field is written and before `init` can run.
    if version != GEN_VERSION {
        return Err(PluginLoadError::UnsupportedVersion {
            kind: PluginKind::General,
            actual: version,
            expected: GEN_VERSION,
        });
    }
    // S09 §7.1 lines 372–373: the host fills these two before init().
    // SAFETY: live module in plugin static data.
    unsafe {
        (*module).parent_window = host_hwnd as *mut core::ffi::c_void;
        (*module).dll_instance = hinstance;
    }
    let mut init_called = false;
    let mut init_result = 0;
    // SAFETY: cdecl, no arguments (S09 §7.1 offset 0x08), non-null slot.
    if let Some(init) = unsafe { (*module).init } {
        init_called = true;
        init_result = unsafe { init() };
    }
    Ok(GeneralInit {
        module,
        init_called,
        init_result,
    })
}

// ---------------------------------------------------------------------------
// Output plugin initialization (S09 §4)
// ---------------------------------------------------------------------------

/// What actually happened while an output module was initialized.
///
/// `init_called` is false when the plugin supplied no `init` slot, so a
/// skipped call is never reported as a successful initialization.
#[cfg(any(windows, test))]
#[derive(Debug)]
struct OutputInit {
    module: *mut OutModule,
    init_called: bool,
}

/// Validate and initialize an output plugin module (S09 §4.1, §4.2).
///
/// Mirrors [`init_general_module`]: the version is checked, the two host-filled
/// members are written, and `init` is called — in that order, all on the
/// caller's thread, which must be the UI thread owning the host window
/// (S09 §4.2 line 252).
///
/// No other output callback is called here. `open`, `write`, `can_write`,
/// `is_playing`, `close`, `pause`, `set_volume`, `set_pan`, `flush` and the
/// two time queries belong to the **input** plugin's decode thread
/// (S09 §4.2 step 3, §3.7); the host only builds the output list and calls
/// `init` and `quit`.
///
/// # Safety
///
/// `module` must be null or point at a live `Out_Module` in the mapped
/// plugin's static storage (S09 §0). `host_hwnd` must be the real main-window
/// handle and `hinstance` the plugin's own module handle. The plugin's `init`
/// runs here on the caller's thread.
#[cfg(any(windows, test))]
unsafe fn init_output_module(
    module: *mut OutModule,
    host_hwnd: usize,
    hinstance: *mut core::ffi::c_void,
) -> Result<OutputInit, PluginLoadError> {
    if module.is_null() {
        return Err(PluginLoadError::NullModule);
    }
    // SAFETY: caller guarantees a live module for these reads.
    let version = unsafe { (*module).version };
    // S09 §4.1 line 229: 0x10 in 2.95. 0x11 is 5.x-only and its description is
    // a wide string, not the ANSI one this host reads, so it is refused
    // (U-7 decision: strict equality, the general and DSP policy).
    if version != OUT_VERSION {
        return Err(PluginLoadError::UnsupportedVersion {
            kind: PluginKind::Output,
            actual: version,
            expected: OUT_VERSION,
        });
    }
    // S09 §4.1 lines 233–234: the host fills the main window and the plugin's
    // own module handle before `init` runs. The `id` member stays as the
    // plugin left it: the 5.x host replaces it with the DLL file name
    // (5.x-only) and whether 2.95 does is open (U-4).
    // SAFETY: live module in plugin static data.
    unsafe {
        (*module).main_window = host_hwnd as *mut core::ffi::c_void;
        (*module).dll_instance = hinstance;
    }
    let mut init_called = false;
    // SAFETY: cdecl, no arguments (S09 §4.1 offset 0x1C), non-null slot.
    if let Some(init) = unsafe { (*module).init } {
        init_called = true;
        unsafe { init() };
    }
    Ok(OutputInit {
        module,
        init_called,
    })
}

// ---------------------------------------------------------------------------
// Error and Result Types
// ---------------------------------------------------------------------------

/// Why loading a classic Winamp plugin DLL failed.
#[derive(Debug)]
pub enum PluginLoadError {
    /// Host process is not running as 32-bit x86 (i686); 32-bit DLLs cannot be mapped into 64-bit processes.
    NotX86Architecture,
    /// Plugin DLL loading requires Windows.
    NotWindowsPlatform,
    /// I/O error reading the plugin DLL image for PE preflight.
    Io(std::io::Error),
    /// Passive DLL preflight refused the file before any code execution.
    Preflight(PePreflightError),
    /// Candidate bytes do not exactly equal the caller's independently approved image.
    NotApproved,
    /// The source or private copy no longer matches the preflighted bytes.
    ImageChanged(&'static str),
    /// A dependency search-directory operation failed before loading.
    SearchDirectoryFailed(u32),
    /// Win32 `LoadLibraryExW` returned NULL.
    OsLoadFailed(u32),
    /// Required entry-point export was not found in the loaded DLL.
    MissingExport(&'static str),
    /// Plugin entry point returned a null module pointer.
    NullModule,
    /// Plugin `init` refused: DSP `init` returned non-zero (S09 §6.2: 0 on
    /// success). Raised before activation; no caller can reach the module.
    InitFailed { kind: PluginKind, code: i32 },
    /// A required plugin callback pointer was null.
    MissingCallback(&'static str),
    /// Plugin returned an unsupported module version.
    UnsupportedVersion {
        kind: PluginKind,
        actual: i32,
        expected: i32,
    },
    /// Host window HWND is zero or invalid.
    MissingHostWindow,
    /// Candidate plugin or approval store operation failed the approval store gate.
    ApprovalStore(crate::approval::ApprovalStoreError),
}

impl fmt::Display for PluginLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotX86Architecture => f.write_str(
                "plugin loading requires a 32-bit x86 (i686) process; cannot map 32-bit DLL into 64-bit process",
            ),
            Self::NotWindowsPlatform => f.write_str("plugin DLL loading is supported on Windows only"),
            Self::Io(e) => write!(f, "plugin image I/O failed: {e}"),
            Self::Preflight(e) => write!(f, "PE DLL preflight refused plugin before loading: {e}"),
            Self::NotApproved => f.write_str("plugin bytes do not match the approved image; refusing before loading"),
            Self::ImageChanged(reason) => write!(f, "plugin bytes changed; refusing before loading: {reason}"),
            Self::SearchDirectoryFailed(code) => write!(f, "plugin dependency directory setup failed before loading: {code}"),
            Self::OsLoadFailed(code) => {
                write!(f, "Win32 LoadLibraryExW failed with error code {code}")
            }
            Self::MissingExport(name) => {
                write!(f, "plugin DLL does not export entry point '{name}'")
            }
            Self::NullModule => f.write_str("plugin entry point returned null module pointer"),
            Self::InitFailed { kind, code } => write!(
                f,
                "{kind:?} plugin init refused with code {code}; refusing before activation"
            ),
            Self::MissingCallback(name) => {
                write!(f, "plugin is missing required callback '{name}'")
            }
            Self::UnsupportedVersion {
                kind,
                actual,
                expected,
            } => write!(
                f,
                "{kind:?} plugin returned unsupported version {actual:#x} (expected {expected:#x})"
            ),
            Self::MissingHostWindow => f.write_str("host window HWND is zero or not created"),
            Self::ApprovalStore(e) => write!(f, "plugin approval store refusal: {e}"),
        }
    }
}

impl std::error::Error for PluginLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Preflight(e) => Some(e),
            Self::ApprovalStore(e) => Some(e),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// LoadedPlugin
// ---------------------------------------------------------------------------

/// Why a plugin's configuration callback was not invoked.
///
/// Opening a configuration dialog is a UI action, so a refusal here is always
/// returned to the caller rather than guessed at: nothing is ever called
/// through a null slot (S09 §0, P3 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginConfigError {
    /// The plugin supplied no `config` callback. S09 requires no plugin to
    /// provide one, so this is a fact about this plugin, not a verdict on its
    /// validity.
    MissingCallback,
    /// The plugin is not live: `quit` already ran, `init` never did, or — for
    /// the DSP and visualization kinds — its module is not the running one any
    /// more.
    NotInitialized,
    /// This loaded plugin has no kind bridge in this host, so the host does
    /// not know which `config` signature to call. Every kind the loader
    /// produces is bridged; the variant stays so that a plugin of a kind this
    /// host cannot drive is a reported error and never a call through a
    /// signature the host would have to guess.
    WrongKind(PluginKind),
}

impl fmt::Display for PluginConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCallback => {
                f.write_str("plugin supplied no config callback; nothing was called")
            }
            Self::NotInitialized => f.write_str("plugin is not initialized; config was not called"),
            Self::WrongKind(kind) => write!(
                f,
                "config is not routed for a {kind:?} plugin; nothing was called"
            ),
        }
    }
}

impl std::error::Error for PluginConfigError {}

#[derive(Debug)]
enum LoadedPluginInner {
    #[cfg(windows)]
    Input {
        module: *mut InModule,
        /// `init` was actually called. False means the slot was null and
        /// skipped (S09 §3.1 offset 0x24).
        init_called: bool,
    },
    /// A DSP library: its header, the modules it hands out and the module
    /// that is active, if any (S09 §6.1, §6.3).
    #[cfg(any(windows, test))]
    Dsp {
        header: *mut DspHeader,
        modules: Vec<DspModuleEntry>,
        /// The module whose `init` ran and that is still the active DSP.
        active: Option<*mut DspModule>,
        init_called: bool,
        init_result: core::ffi::c_int,
    },
    #[cfg(any(windows, test))]
    General {
        module: *mut GeneralPlugin,
        init_called: bool,
        init_result: core::ffi::c_int,
    },
    #[cfg(any(windows, test))]
    Output {
        module: *mut OutModule,
        init_called: bool,
    },
    /// A visualization library: its header, the modules it hands out and the
    /// module that is running, if any (S09 §5.1, §5.3).
    #[cfg(any(windows, test))]
    Vis {
        header: *mut VisHeader,
        modules: Vec<VisModuleEntry>,
        /// The module whose `init` ran and that is still the running
        /// visualization.
        active: Option<*mut VisModule>,
        init_called: bool,
        init_result: core::ffi::c_int,
    },
    #[allow(dead_code)]
    Unsupported,
}

/// A loaded and initialized classic Winamp plugin DLL.
///
/// Holds the module handle (`HINSTANCE`) and the plugin's module structure pointer.
/// Pointers point into the plugin's static data and remain valid as long as the
/// DLL is loaded (S09 §0 line 32).
pub struct LoadedPlugin {
    /// The plugin's identity, boxed so its address does not move: a fault
    /// report reads it from the thread-local record of the running call
    /// (see [`crate::fault`]).
    identity: Box<PluginIdentity>,
    hinstance: *mut core::ffi::c_void,
    /// The host's main window, handed to the plugin (S09 §1 "Parent window").
    /// A visualization module is filled when the user starts it, so the host
    /// keeps the handle (S09 §5.2, §5.3 step 2).
    host_hwnd: usize,
    description: String,
    inner: LoadedPluginInner,
    initialized: bool,
}

impl LoadedPlugin {
    /// Canonical source path; the OS maps a private byte-identical snapshot.
    pub fn path(&self) -> &Path {
        self.identity.path()
    }

    /// Plugin kind.
    pub fn kind(&self) -> PluginKind {
        self.identity.kind()
    }

    /// Description string provided by the plugin (S09 §1 line 40).
    pub fn description(&self) -> &str {
        &self.description
    }

    /// DLL module instance handle (`HINSTANCE`).
    pub fn hinstance(&self) -> *mut core::ffi::c_void {
        self.hinstance
    }

    /// Whether `init()` was successfully called on this plugin.
    ///
    /// For a visualization plugin this means that one of its modules is
    /// running: the module is chosen and initialized when the user starts the
    /// visualization, not when the DLL is loaded (S09 §5.3 step 2). For a DSP
    /// plugin it means that its module still owns the one active slot
    /// (S09 §6.3 step 2).
    pub fn is_initialized(&self) -> bool {
        match self.inner {
            #[cfg(any(windows, test))]
            LoadedPluginInner::Vis { active, .. } => match active {
                Some(module) => crate::vis::active_vis_module() == module,
                None => false,
            },
            #[cfg(any(windows, test))]
            LoadedPluginInner::Dsp { active, .. } => match active {
                Some(module) => crate::dsp::active_dsp_module() == module,
                None => false,
            },
            _ => self.initialized,
        }
    }

    /// Call `quit()` on the plugin if initialized (S09 §3.5 line 166, §7.2 line 380).
    ///
    /// Per S09 §1 line 46 and §3.5 line 166, the host does not free the DLL at shutdown
    /// because plugins may have subclassed windows or spawned threads.
    pub fn quit(&mut self) {
        if !self.initialized {
            return;
        }
        // `quit` is plugin code like every other call: a fault in it must
        // name this plugin.
        let _scope = plugin_call_scope(&self.identity, "quit");
        match self.inner {
            #[cfg(windows)]
            LoadedPluginInner::Input { module, .. } => {
                if !module.is_null() {
                    // SAFETY: module pointer was verified non-null at load time.
                    if let Some(quit_fn) = unsafe { (*module).quit } {
                        // SAFETY: quit() takes no arguments in InModule (S09 §3.1 offset 0x28).
                        unsafe { quit_fn() };
                    }
                }
            }
            #[cfg(any(windows, test))]
            LoadedPluginInner::Dsp { active, .. } => {
                if let Some(module) = active {
                    // Only the module that still owns the active slot is quit
                    // here. A module another DSP replaced was already quit
                    // when the new one started (S09 §6.3 step 2 and step 4),
                    // and it must not be quit twice.
                    if crate::dsp::active_dsp_module() == module {
                        // SAFETY: module pointer was verified non-null when the
                        // module list was read.
                        if let Some(quit_fn) = unsafe { (*module).quit } {
                            // SAFETY: quit(module) per S09 §6.2 offset 0x18.
                            unsafe { quit_fn(module) };
                        }
                        crate::dsp::clear_active_dsp_module(module);
                    }
                }
            }
            #[cfg(any(windows, test))]
            LoadedPluginInner::General { module, .. } => {
                if !module.is_null() {
                    // SAFETY: module pointer was verified non-null at load time.
                    if let Some(quit_fn) = unsafe { (*module).quit } {
                        // SAFETY: quit() takes no arguments in GeneralPlugin (S09 §7.1 offset 0x10).
                        unsafe { quit_fn() };
                    }
                }
            }
            #[cfg(any(windows, test))]
            LoadedPluginInner::Output { module, .. } => {
                if !module.is_null() {
                    // SAFETY: module pointer was verified non-null at load time.
                    if let Some(quit_fn) = unsafe { (*module).quit } {
                        // SAFETY: quit() takes no arguments in OutModule (S09 §4.1 offset 0x20).
                        unsafe { quit_fn() };
                    }
                }
            }
            #[cfg(any(windows, test))]
            LoadedPluginInner::Vis { active, .. } => {
                if let Some(module) = active {
                    // Only the module that still owns the running slot is quit
                    // here. A module another visualization replaced was
                    // already quit when the new one started (S09 §5.3 step 1
                    // and step 5), and it must not be quit twice.
                    if crate::vis::active_vis_module() == module {
                        // SAFETY: module pointer was verified non-null when the
                        // module list was read.
                        if let Some(quit_fn) = unsafe { (*module).quit } {
                            // SAFETY: quit(module) per S09 §5.2 offset 0x930.
                            unsafe { quit_fn(module) };
                        }
                        crate::vis::clear_active_vis_module(module);
                    }
                }
            }
            LoadedPluginInner::Unsupported => {}
        }
        self.initialized = false;
    }

    /// Host-visible state of a loaded general-purpose module, read back out of
    /// the plugin's own module struct. `None` for any other plugin kind.
    ///
    /// This is what an acceptance runner checks **before** it calls anything
    /// (P3 §4 step 4, §3): the `init`/`config`/`quit` slots as the plugin
    /// supplied them, the two members the host filled before `init`, and
    /// whether `init` actually ran and what it returned. A missing slot is a
    /// fixture/precondition fact reported here, not a refusal invented in the
    /// loader. The struct lives in plugin static data, so this stays readable
    /// after [`quit`](Self::quit) (the mapping is retained, S09 §1 line 46).
    pub fn general_info(&self) -> Option<GeneralModuleInfo> {
        match self.inner {
            #[cfg(any(windows, test))]
            LoadedPluginInner::General {
                module,
                init_called,
                init_result,
            } if !module.is_null() => {
                // SAFETY: the module pointer was verified non-null at load time
                // and points into plugin static data of the retained mapping.
                let m = unsafe { *module };
                Some(GeneralModuleInfo {
                    has_init: m.init.is_some(),
                    has_config: m.config.is_some(),
                    has_quit: m.quit.is_some(),
                    parent_window: m.parent_window as usize,
                    dll_instance: m.dll_instance as usize,
                    init_called,
                    init_result,
                })
            }
            _ => None,
        }
    }

    /// Open a plugin's configuration dialog: call its `config` callback
    /// (S09 §1 "Config / About").
    ///
    /// Every kind has a `config` slot, and the host calls it with the
    /// arguments that kind's slot takes:
    ///
    /// * an **input** or **output** plugin takes the parent window handle
    ///   (S09 §3.1 offset 0x1C, §4.1 offset 0x14) — the host window the
    ///   plugin was handed before `init`;
    /// * a **general** plugin takes no argument and reads the parent window
    ///   from its own struct (S09 §7.1 offset 0x0C and offset 0x14);
    /// * a **DSP** or **visualization** plugin takes its module pointer
    ///   (S09 §6.2 offset 0x0C, §5.2 offset 0x924). Only the module that
    ///   still owns the one active slot is configured: a module another
    ///   plugin replaced is not live, and its parent window is not valid.
    ///
    /// Must be called on the UI thread that owns the host window, which is the
    /// parent the plugin was handed before `init`. The call is synchronous: a
    /// modal dialog returns only when the plugin closes it, so the caller must
    /// expect this to block and must keep pumping afterwards (see
    /// [`crate::message_loop::pump_bounded`]). A null slot, a plugin that is
    /// not live, and a plugin this host has no kind bridge for are all refused
    /// without calling anything; S09 U-15 (the modal/parent context of a
    /// general Config in 2.95) stays open, so no parent or modality is forced
    /// here beyond the handed parent.
    pub fn config(&self) -> Result<(), PluginConfigError> {
        // `config` is plugin code like every other call: a fault in it must
        // name this plugin.
        let _scope = plugin_call_scope(&self.identity, "config");
        match self.inner {
            // S09 §3.1 offset 0x1C: `config(parent window handle)`.
            #[cfg(windows)]
            LoadedPluginInner::Input { module, .. } => {
                if !self.initialized {
                    return Err(PluginConfigError::NotInitialized);
                }
                if module.is_null() {
                    return Err(PluginConfigError::MissingCallback);
                }
                // SAFETY: live module in the retained mapping's static data.
                let Some(config_fn) = (unsafe { (*module).config }) else {
                    return Err(PluginConfigError::MissingCallback);
                };
                // SAFETY: cdecl, one argument (S09 §3.1 offset 0x1C), non-null
                // slot, on the UI thread owning the parent window.
                unsafe { config_fn(self.host_hwnd as *mut core::ffi::c_void) };
                Ok(())
            }
            // S09 §4.1 offset 0x14: `config(parent window handle)`.
            #[cfg(any(windows, test))]
            LoadedPluginInner::Output { module, .. } => {
                if !self.initialized {
                    return Err(PluginConfigError::NotInitialized);
                }
                if module.is_null() {
                    return Err(PluginConfigError::MissingCallback);
                }
                // SAFETY: live module in the retained mapping's static data.
                let Some(config_fn) = (unsafe { (*module).config }) else {
                    return Err(PluginConfigError::MissingCallback);
                };
                // SAFETY: cdecl, one argument (S09 §4.1 offset 0x14), non-null
                // slot, on the UI thread owning the parent window.
                unsafe { config_fn(self.host_hwnd as *mut core::ffi::c_void) };
                Ok(())
            }
            // S09 §7.1 offset 0x0C: `config()`, no arguments. The plugin reads
            // the parent window from its own struct (offset 0x14), which the
            // host filled before `init`.
            #[cfg(any(windows, test))]
            LoadedPluginInner::General { module, .. } => {
                if !self.initialized {
                    return Err(PluginConfigError::NotInitialized);
                }
                if module.is_null() {
                    return Err(PluginConfigError::MissingCallback);
                }
                // SAFETY: live module in the retained mapping's static data.
                let config_slot = unsafe { (*module).config };
                let Some(config_fn) = config_slot else {
                    return Err(PluginConfigError::MissingCallback);
                };
                // SAFETY: cdecl, no arguments (S09 §7.1 offset 0x0C), non-null
                // slot, on the UI thread owning the parent window.
                unsafe { config_fn() };
                Ok(())
            }
            // S09 §6.2 offset 0x0C: `config(module)`.
            #[cfg(any(windows, test))]
            LoadedPluginInner::Dsp { active, .. } => {
                if !self.initialized {
                    return Err(PluginConfigError::NotInitialized);
                }
                let Some(module) = active else {
                    return Err(PluginConfigError::NotInitialized);
                };
                if module.is_null() || crate::dsp::active_dsp_module() != module {
                    // The module is not the active one any more, so it is not
                    // live and its dialog must not be opened (S09 §6.3 step 2).
                    return Err(PluginConfigError::NotInitialized);
                }
                // SAFETY: live module in the retained mapping's static data.
                let config_slot = unsafe { (*module).config };
                let Some(config_fn) = config_slot else {
                    return Err(PluginConfigError::MissingCallback);
                };
                // SAFETY: cdecl, one argument (S09 §6.2 offset 0x0C), non-null
                // slot, on the UI thread owning the parent window.
                unsafe { config_fn(module) };
                Ok(())
            }
            // S09 §5.2 offset 0x924: `config(module)`. The module's parent
            // window is filled when the module starts (S09 §5.3 step 2), so
            // only the running module is configured (S09 §5.3 step 7).
            #[cfg(any(windows, test))]
            LoadedPluginInner::Vis { active, .. } => {
                if !self.initialized {
                    return Err(PluginConfigError::NotInitialized);
                }
                let Some(module) = active else {
                    return Err(PluginConfigError::NotInitialized);
                };
                if module.is_null() || crate::vis::active_vis_module() != module {
                    // Another visualization is running; this one is not, so its
                    // module is not live and its dialog must not be opened
                    // (S09 §5.3 step 1).
                    return Err(PluginConfigError::NotInitialized);
                }
                // SAFETY: live module in the retained mapping's static data.
                let config_slot = unsafe { (*module).config };
                let Some(config_fn) = config_slot else {
                    return Err(PluginConfigError::MissingCallback);
                };
                // SAFETY: cdecl, one argument (S09 §5.2 offset 0x924), non-null
                // slot, on the UI thread owning the parent window.
                unsafe { config_fn(module) };
                Ok(())
            }
            _ => Err(PluginConfigError::WrongKind(self.kind())),
        }
    }

    /// The library-level state of a loaded DSP plugin: the header version, the
    /// library's own name and the list of modules it hands out (S09 §6.1).
    /// `None` for any other plugin kind.
    ///
    /// The module list is what the host shows in the Plug-ins list; the
    /// selection is the index S09 §6.3 step 1 passes to `get_module`, and
    /// [`dsp_start`](Self::dsp_start) starts it.
    pub fn dsp_info(&self) -> Option<DspLibraryInfo> {
        match &self.inner {
            #[cfg(any(windows, test))]
            LoadedPluginInner::Dsp {
                header, modules, ..
            } => {
                // SAFETY: the header lives in the retained mapping's static
                // data, which outlives the plugin (S09 §1 line 46).
                let (version, description) = unsafe {
                    (
                        (**header).version,
                        read_c_string_lossy((**header).description),
                    )
                };
                Some(DspLibraryInfo {
                    version,
                    description,
                    modules: modules.iter().map(|entry| entry.info.clone()).collect(),
                })
            }
            _ => None,
        }
    }

    /// Host-visible state of the active DSP module, read from the plugin's own
    /// module struct. `None` when no module of this plugin is active (nothing
    /// started yet, `quit` already ran, or another DSP took over).
    pub fn dsp_run_info(&self) -> Option<DspRunInfo> {
        match &self.inner {
            #[cfg(any(windows, test))]
            LoadedPluginInner::Dsp {
                modules,
                active,
                init_called,
                init_result,
                ..
            } => {
                let module = (*active)?;
                if crate::dsp::active_dsp_module() != module {
                    return None;
                }
                let index = modules
                    .iter()
                    .find(|entry| entry.module == module)
                    .map(|entry| entry.info.index)
                    .unwrap_or(-1);
                // SAFETY: the module pointer was verified non-null when the
                // module list was read and points into the retained mapping's
                // static data.
                let m = unsafe { *module };
                Some(DspRunInfo {
                    index,
                    parent_window: m.parent_window as usize,
                    dll_instance: m.dll_instance as usize,
                    init_called: *init_called,
                    init_result: *init_result,
                    host_active: crate::dsp::dsp_is_active(),
                })
            }
            _ => None,
        }
    }

    /// Host-visible state of a loaded output module, read from the plugin's own
    /// module struct. `None` for any other plugin kind.
    ///
    /// The struct lives in plugin static data, so this stays readable after
    /// [`quit`](Self::quit) (the mapping is retained, S09 §1 line 46).
    pub fn output_info(&self) -> Option<OutModuleInfo> {
        match self.inner {
            #[cfg(any(windows, test))]
            LoadedPluginInner::Output {
                module,
                init_called,
            } if !module.is_null() => {
                // SAFETY: the module pointer was verified non-null at load time
                // and points into plugin static data of the retained mapping.
                let m = unsafe { *module };
                Some(OutModuleInfo {
                    has_init: m.init.is_some(),
                    has_quit: m.quit.is_some(),
                    main_window: m.main_window as usize,
                    dll_instance: m.dll_instance as usize,
                    id: m.id,
                    init_called,
                })
            }
            _ => None,
        }
    }

    /// Host-visible state of a loaded input module, read out of the plugin's
    /// own module struct. `None` for any other plugin kind.
    ///
    /// The struct lives in plugin static data, so this stays readable after
    /// [`quit`](Self::quit) (the mapping is retained, S09 §1 line 46).
    #[cfg(windows)]
    pub fn input_info(&self) -> Option<InModuleInfo> {
        match self.inner {
            LoadedPluginInner::Input {
                module,
                init_called,
            } if !module.is_null() => Some(crate::input::module_info(module, init_called)),
            _ => None,
        }
    }

    /// The plugin's own module pointer, when this is an input plugin.
    #[cfg(windows)]
    fn input_module(&self) -> Result<*mut InModule, InputError> {
        match self.inner {
            LoadedPluginInner::Input { module, .. } if !module.is_null() => Ok(module),
            LoadedPluginInner::Input { .. } => Err(InputError::NullModule),
            _ => Err(InputError::WrongKind(self.kind())),
        }
    }

    /// Start playback of `path` through the input plugin (S09 §3.5 step 3).
    ///
    /// The host hands the plugin the path as an ANSI string and reports the
    /// plugin's own answer: 0 is success, -1 is "file not found" and any
    /// other nonzero is another error that stops the playlist advance
    /// (S09 §3.1 offset 0x38). A plugin that supplied no `play` slot is
    /// refused, never called through a null slot (S09 §0).
    ///
    /// The host has no audio path into a classic input plugin yet: the
    /// plugin's own decode thread, the output plugin it drives and the
    /// analyzer data are not here (S09 §3.6, §3.7). What this call does is
    /// start the plugin's playback and give the acceptance path a lifecycle
    /// to pin.
    #[cfg(windows)]
    pub fn input_play(&self, path: &Path) -> Result<(), InputError> {
        if !self.initialized {
            return Err(InputError::NotInitialized);
        }
        let module = self.input_module()?;
        // `play` is plugin code like every other call: a fault in it must
        // name this plugin.
        let _scope = plugin_call_scope(&self.identity, "play");
        crate::input::call_play(module, path)
    }

    /// Pause the input plugin's playback (S09 §3.1 offset 0x3C).
    #[cfg(windows)]
    pub fn input_pause(&self) -> Result<(), InputError> {
        if !self.initialized {
            return Err(InputError::NotInitialized);
        }
        let module = self.input_module()?;
        // `pause` is plugin code like every other call: a fault in it must
        // name this plugin.
        let _scope = plugin_call_scope(&self.identity, "pause");
        crate::input::call_pause(module)
    }

    /// Resume the input plugin's playback (S09 §3.1 offset 0x40).
    #[cfg(windows)]
    pub fn input_unpause(&self) -> Result<(), InputError> {
        if !self.initialized {
            return Err(InputError::NotInitialized);
        }
        let module = self.input_module()?;
        // `unpause` is plugin code like every other call: a fault in it must
        // name this plugin.
        let _scope = plugin_call_scope(&self.identity, "unpause");
        crate::input::call_unpause(module)
    }

    /// Stop the input plugin's playback and let it unload the stream
    /// (S09 §3.5 step 6, S09 §3.1 offset 0x48).
    #[cfg(windows)]
    pub fn input_stop(&self) -> Result<(), InputError> {
        if !self.initialized {
            return Err(InputError::NotInitialized);
        }
        let module = self.input_module()?;
        // `stop` is plugin code like every other call: a fault in it must
        // name this plugin.
        let _scope = plugin_call_scope(&self.identity, "stop");
        crate::input::call_stop(module)
    }

    /// Ask the input plugin whether it is paused (S09 §3.1 offset 0x44).
    ///
    /// The answer is the plugin's own, never one the host invented.
    #[cfg(windows)]
    pub fn input_is_paused(&self) -> Result<bool, InputError> {
        if !self.initialized {
            return Err(InputError::NotInitialized);
        }
        let module = self.input_module()?;
        // `is_paused` is plugin code like every other call: a fault in it
        // must name this plugin.
        let _scope = plugin_call_scope(&self.identity, "is_paused");
        crate::input::call_is_paused(module)
    }

    /// Ask the input plugin for the track length in milliseconds
    /// (S09 §3.1 offset 0x4C).
    ///
    /// The answer is the plugin's own, never one the host invented.
    #[cfg(windows)]
    pub fn input_length_ms(&self) -> Result<i32, InputError> {
        if !self.initialized {
            return Err(InputError::NotInitialized);
        }
        let module = self.input_module()?;
        // `get_length` is plugin code like every other call: a fault in it
        // must name this plugin.
        let _scope = plugin_call_scope(&self.identity, "get_length");
        crate::input::call_get_length(module)
    }

    /// Ask the input plugin for the playback position in milliseconds
    /// (S09 §3.1 offset 0x50).
    ///
    /// The answer is the plugin's own, never one the host invented. S09
    /// §3.6 lets the analyzer and visualization threads ask for it, so the
    /// call has to be safe from any thread the host owns.
    #[cfg(windows)]
    pub fn input_position_ms(&self) -> Result<i32, InputError> {
        if !self.initialized {
            return Err(InputError::NotInitialized);
        }
        let module = self.input_module()?;
        // `get_position` is plugin code like every other call: a fault in it
        // must name this plugin.
        let _scope = plugin_call_scope(&self.identity, "get_position");
        crate::input::call_get_position(module)
    }

    /// The library-level state of a loaded visualization plugin: the header
    /// version, the library's own name and the list of modules it hands out
    /// (S09 §5.1). `None` for any other plugin kind.
    ///
    /// The module list is what the host shows in the visualization list; the
    /// user picks one of its indices, and
    /// [`vis_start`](Self::vis_start) starts it.
    pub fn vis_info(&self) -> Option<VisLibraryInfo> {
        match &self.inner {
            #[cfg(any(windows, test))]
            LoadedPluginInner::Vis {
                header, modules, ..
            } => {
                // SAFETY: the header lives in the retained mapping's static
                // data, which outlives the plugin (S09 §1 line 46).
                let (version, description) = unsafe {
                    (
                        (**header).version,
                        read_c_string_lossy((**header).description),
                    )
                };
                Some(VisLibraryInfo {
                    version,
                    description,
                    modules: modules.iter().map(|entry| entry.info.clone()).collect(),
                })
            }
            _ => None,
        }
    }

    /// Host-visible state of the running visualization module, read from the
    /// plugin's own module struct. `None` when no module of this plugin is
    /// running (nothing started yet, `quit` already ran, or another
    /// visualization took over).
    pub fn vis_run_info(&self) -> Option<VisRunInfo> {
        match &self.inner {
            #[cfg(any(windows, test))]
            LoadedPluginInner::Vis {
                modules,
                active,
                init_called,
                init_result,
                ..
            } => {
                let module = (*active)?;
                if crate::vis::active_vis_module() != module {
                    return None;
                }
                let index = modules
                    .iter()
                    .find(|entry| entry.module == module)
                    .map(|entry| entry.info.index)
                    .unwrap_or(-1);
                // SAFETY: the module pointer was verified non-null when the
                // module list was read and points into the retained mapping's
                // static data.
                let m = unsafe { *module };
                Some(VisRunInfo {
                    index,
                    parent_window: m.parent_window as usize,
                    dll_instance: m.dll_instance as usize,
                    sample_rate: m.sample_rate,
                    channels: m.channels,
                    latency_ms: m.latency_ms,
                    delay_ms: m.delay_ms,
                    init_called: *init_called,
                    init_result: *init_result,
                })
            }
            _ => None,
        }
    }

    /// Start one of the library's modules (S09 §5.3 step 2).
    ///
    /// The host fills the parent window, the plugin's own module handle, the
    /// sample rate and the channel count into the chosen module and then calls
    /// its `init`. A nonzero return aborts the start (S09 §5.2), and the
    /// module is then not running. A visualization that is already running is
    /// stopped first, because only one visualization is active at a time
    /// (S09 §5.3 step 1).
    pub fn vis_start(&mut self, index: i32) -> Result<(), VisError> {
        let (module, host_hwnd, hinstance) = match &self.inner {
            #[cfg(any(windows, test))]
            LoadedPluginInner::Vis { modules, .. } => {
                let Some(entry) = modules.iter().find(|entry| entry.info.index == index) else {
                    return Err(VisError::UnknownModule(index));
                };
                (entry.module, self.host_hwnd, self.hinstance)
            }
            _ => return Err(VisError::WrongKind(self.kind())),
        };
        // S09 §5.3 step 1: a running visualization is stopped before another
        // one starts. The module that is stopped belongs to the plugin that
        // was running, not to this one, so no fault scope names this plugin
        // here: a fault in that other plugin's `quit` is reported without a
        // name rather than with the wrong one.
        crate::vis::stop_active_vis_module();
        // SAFETY: the module lives in the mapped plugin's static data, which
        // the retained mapping keeps alive; `init` runs on this thread, which
        // owns the host window.
        let init = {
            let _scope = plugin_call_scope(&self.identity, "init");
            unsafe { crate::vis::init_vis_module(module, host_hwnd, hinstance) }?
        };
        if let LoadedPluginInner::Vis {
            active,
            init_called,
            init_result,
            ..
        } = &mut self.inner
        {
            *active = Some(init.module);
            *init_called = init.init_called;
            *init_result = init.init_result;
        }
        self.initialized = true;
        crate::vis::set_active_vis_module(init.module);
        Ok(())
    }

    /// Draw one frame of the running visualization (S09 §5.3 step 4b).
    ///
    /// The plugin's own return value is reported: 0 continues the
    /// visualization, a nonzero value ends it and the host then calls `quit`
    /// (S09 §5.2, offset 0x92C). With no module running — nothing started
    /// yet, or another visualization took over — nothing is called.
    pub fn vis_render(&self) -> Result<i32, VisError> {
        let module = match self.inner {
            #[cfg(any(windows, test))]
            LoadedPluginInner::Vis { active, .. } => match active {
                Some(module) => module,
                None => return Err(VisError::NotRunning),
            },
            _ => return Err(VisError::WrongKind(self.kind())),
        };
        if crate::vis::active_vis_module() != module {
            // Another visualization is running; this one is not.
            return Err(VisError::NotRunning);
        }
        // `render` is plugin code like every other call: a fault in it must
        // name this plugin.
        let _scope = plugin_call_scope(&self.identity, "render");
        // SAFETY: the module lives in the mapped plugin's static data, which
        // the retained mapping keeps alive.
        unsafe { crate::vis::render_vis_frame(module) }
    }

    /// Start one of the library's modules (S09 §6.3 step 1).
    ///
    /// The host fills the parent window and the plugin's own module handle
    /// into the chosen module and then calls its `init`. Only one DSP module
    /// is active at a time (S09 §6.3 step 2), so the module that was active
    /// before is quit first (S09 §6.3 step 4).
    ///
    /// A nonzero `init` return aborts the start. S09 §6.2 says "0 on success"
    /// and nothing more about a nonzero value, so refusing it is an own
    /// choice (P6 §2), not a spec fact; the module is then not active.
    pub fn dsp_start(&mut self, index: i32) -> Result<(), DspError> {
        let (module, host_hwnd, hinstance) = match &self.inner {
            #[cfg(any(windows, test))]
            LoadedPluginInner::Dsp { modules, .. } => {
                let Some(entry) = modules.iter().find(|entry| entry.info.index == index) else {
                    return Err(DspError::UnknownModule(index));
                };
                (entry.module, self.host_hwnd, self.hinstance)
            }
            _ => return Err(DspError::WrongKind(self.kind())),
        };
        // S09 §6.3 step 2: only one DSP module is active at a time, so the
        // module that was active before is quit when the selection changes.
        // That module belongs to the plugin that was running, not to this one,
        // so no fault scope names this plugin here: a fault in that other
        // plugin's `quit` is reported without a name rather than with the
        // wrong one.
        crate::dsp::stop_active_dsp_module();
        // SAFETY: the module lives in the mapped plugin's static data, which
        // the retained mapping keeps alive; `init` runs on this thread, which
        // owns the host window.
        let Some(init) = ({
            let _scope = plugin_call_scope(&self.identity, "init");
            unsafe { crate::dsp::init_dsp_module(module, host_hwnd, hinstance) }
        }) else {
            return Err(DspError::NotRunning);
        };
        if init.init_result != 0 {
            return Err(DspError::InitFailed(init.init_result));
        }
        if let LoadedPluginInner::Dsp {
            active,
            init_called,
            init_result,
            ..
        } = &mut self.inner
        {
            *active = Some(init.module);
            *init_called = init.init_called;
            *init_result = init.init_result;
        }
        self.initialized = true;
        crate::dsp::set_active_dsp_module(init.module);
        Ok(())
    }

    /// Run interleaved 16-bit PCM through the host's `dsp_process` callback, the
    /// path a real input plugin's decode loop takes (S09 §3.1 offset 0x88).
    ///
    /// `pcm` holds `frames * channels` samples; it is grown to twice that for the
    /// plugin (S09 §3.5 line 207), processed in place and then truncated to the
    /// validated output frame count, which is returned. `None` when this is not
    /// an initialized DSP plugin, `channels` is not positive, `pcm` is not a
    /// whole number of frames, or another DSP module owns the one active slot
    /// (S09 §6.3 step 2). A crashing or wedged plugin takes this thread
    /// down with it (no isolation, P6 §4).
    pub fn dsp_process_i16(
        &self,
        pcm: &mut Vec<i16>,
        channels: usize,
        sample_rate: u32,
    ) -> Option<usize> {
        if !self.initialized || channels == 0 || !pcm.len().is_multiple_of(channels) {
            return None;
        }
        match self.inner {
            #[cfg(any(windows, test))]
            LoadedPluginInner::Dsp { active, .. } => {
                let module = active?;
                if crate::dsp::active_dsp_module() != module {
                    // Another DSP owns the one active slot; this one must not
                    // be driven (S09 §6.3 step 2).
                    return None;
                }
                let frames = pcm.len() / channels;
                pcm.resize(pcm.len() * 2, 0);
                // `process` is plugin code like every other call: a fault in it
                // must name this plugin.
                let _scope = plugin_call_scope(&self.identity, "process");
                // SAFETY: the buffer holds twice `frames * channels` samples and the
                // callback runs the active module synchronously on this thread.
                let out = unsafe {
                    host_dsp_process(
                        pcm.as_mut_ptr() as *mut core::ffi::c_void,
                        frames as core::ffi::c_int,
                        16,
                        channels as core::ffi::c_int,
                        sample_rate as core::ffi::c_int,
                    )
                };
                let out = usize::try_from(out).unwrap_or(frames);
                pcm.truncate(out * channels);
                Some(out)
            }
            _ => None,
        }
    }

    /// Wrap an already-initialized synthetic general module as a loaded plugin,
    /// for host-side tests of the general lifecycle (`config`, `quit` order).
    ///
    /// No DLL is mapped and no export is resolved, so a test built on this
    /// proves host logic only — never DLL acceptance (P3 §7 false passes).
    #[cfg(test)]
    pub(crate) fn general_for_test(
        name: &str,
        module: *mut GeneralPlugin,
        init_called: bool,
        init_result: core::ffi::c_int,
    ) -> Self {
        Self {
            identity: PluginIdentity::new(Path::new(name), PluginKind::General),
            hinstance: core::ptr::null_mut(),
            host_hwnd: 0,
            description: name.to_string(),
            inner: LoadedPluginInner::General {
                module,
                init_called,
                init_result,
            },
            initialized: true,
        }
    }

    /// Wrap an already-initialized synthetic output module as a loaded plugin,
    /// for host-side tests of the output lifecycle (`init`, `config`, `quit`
    /// order).
    ///
    /// `host_hwnd` is the parent window the host handed the plugin; the
    /// output's `config` receives it as its argument (S09 §4.1 offset 0x14).
    ///
    /// No DLL is mapped and no export is resolved, so a test built on this
    /// proves host logic only — never DLL acceptance (P3 §7 false passes).
    #[cfg(test)]
    pub(crate) fn output_for_test(
        name: &str,
        module: *mut OutModule,
        init_called: bool,
        host_hwnd: usize,
    ) -> Self {
        Self {
            identity: PluginIdentity::new(Path::new(name), PluginKind::Output),
            hinstance: core::ptr::null_mut(),
            host_hwnd,
            description: name.to_string(),
            inner: LoadedPluginInner::Output {
                module,
                init_called,
            },
            initialized: true,
        }
    }

    /// Wrap an already-initialized synthetic input module as a loaded plugin,
    /// for host-side tests of the input lifecycle (`config`).
    ///
    /// `host_hwnd` is the parent window the host handed the plugin; the
    /// input's `config` receives it as its argument (S09 §3.1 offset 0x1C).
    ///
    /// No DLL is mapped and no export is resolved, so a test built on this
    /// proves host logic only — never DLL acceptance (P3 §7 false passes).
    #[cfg(test)]
    #[cfg(windows)]
    pub(crate) fn input_for_test(name: &str, module: *mut InModule, host_hwnd: usize) -> Self {
        Self {
            identity: PluginIdentity::new(Path::new(name), PluginKind::Input),
            hinstance: core::ptr::null_mut(),
            host_hwnd,
            description: name.to_string(),
            inner: LoadedPluginInner::Input {
                module,
                init_called: true,
            },
            initialized: true,
        }
    }

    /// Wrap a synthetic visualization library as a loaded plugin, for
    /// host-side tests of the visualization lifecycle (module list, `init`,
    /// `render`, `quit`).
    ///
    /// No DLL is mapped and no export is resolved, so a test built on this
    /// proves host logic only — never DLL acceptance (P3 §7 false passes).
    #[cfg(test)]
    pub(crate) fn vis_for_test(
        name: &str,
        header: *mut VisHeader,
        modules: Vec<VisModuleEntry>,
        host_hwnd: usize,
    ) -> Self {
        Self {
            identity: PluginIdentity::new(Path::new(name), PluginKind::Visualization),
            hinstance: core::ptr::null_mut(),
            host_hwnd,
            description: name.to_string(),
            inner: LoadedPluginInner::Vis {
                header,
                modules,
                active: None,
                init_called: false,
                init_result: 0,
            },
            initialized: false,
        }
    }

    /// Wrap a synthetic DSP library as a loaded plugin, for host-side tests of
    /// the DSP lifecycle (module list, `init`, `process`, `quit`, `config`).
    ///
    /// No DLL is mapped and no export is resolved, so a test built on this
    /// proves host logic only — never DLL acceptance (P3 §7 false passes).
    #[cfg(test)]
    pub(crate) fn dsp_for_test(
        name: &str,
        header: *mut DspHeader,
        modules: Vec<DspModuleEntry>,
        host_hwnd: usize,
    ) -> Self {
        Self {
            identity: PluginIdentity::new(Path::new(name), PluginKind::Dsp),
            hinstance: core::ptr::null_mut(),
            host_hwnd,
            description: name.to_string(),
            inner: LoadedPluginInner::Dsp {
                header,
                modules,
                active: None,
                init_called: false,
                init_result: 0,
            },
            initialized: false,
        }
    }

    /// A loaded plugin the host has no kind bridge for, for tests that check
    /// what the kind-specific entry points refuse. The loader never produces
    /// one: every kind it maps is bridged.
    #[cfg(test)]
    pub(crate) fn unsupported_for_test(kind: PluginKind) -> Self {
        Self {
            identity: PluginIdentity::new(Path::new("unsupported"), kind),
            hinstance: core::ptr::null_mut(),
            host_hwnd: 0,
            description: String::new(),
            inner: LoadedPluginInner::Unsupported,
            initialized: true,
        }
    }
}

/// Host-visible state of a loaded general-purpose module
/// (see [`LoadedPlugin::general_info`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneralModuleInfo {
    /// The plugin supplied an `init` callback (S09 §7.1 offset 0x08).
    pub has_init: bool,
    /// The plugin supplied a `config` callback (S09 §7.1 offset 0x0C). S09 §7
    /// does not require one; our acceptance fixture is required to have one.
    pub has_config: bool,
    /// The plugin supplied a `quit` callback (S09 §7.1 offset 0x10).
    pub has_quit: bool,
    /// `parent_window` as filled by the host before `init`.
    pub parent_window: usize,
    /// `dll_instance` as filled by the host before `init`.
    pub dll_instance: usize,
    /// `init` was actually called. False means the slot was null and skipped.
    pub init_called: bool,
    /// The raw value `init` returned, recorded and not interpreted (S09 U-8).
    pub init_result: core::ffi::c_int,
}

/// Host-visible state of a loaded output module
/// (see [`LoadedPlugin::output_info`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutModuleInfo {
    /// The plugin supplied an `init` callback (S09 §4.1 offset 0x1C).
    pub has_init: bool,
    /// The plugin supplied a `quit` callback (S09 §4.1 offset 0x20).
    pub has_quit: bool,
    /// `main_window` as filled by the host before `init` (S09 §4.1 offset 0x0C).
    pub main_window: usize,
    /// `dll_instance` as filled by the host before `init` (S09 §4.1 offset 0x10).
    pub dll_instance: usize,
    /// The module id as the plugin left it. The host does not overwrite it: the
    /// 5.x host replaces it with the DLL file name, and whether 2.95 does is
    /// open (S09 §4.1 line 231, S09 U-4).
    pub id: i32,
    /// `init` was actually called. False means the slot was null and skipped.
    pub init_called: bool,
}

impl Drop for LoadedPlugin {
    fn drop(&mut self) {
        self.quit();
        // S09 §1 line 46: Input, output and general DLLs are NOT freed at
        // shutdown. DSP §6.3 frees the DLL after quit; this host retains it
        // instead (same conservative policy) until the VM proves free-after-quit
        // safe.
    }
}

// ---------------------------------------------------------------------------
// Loading Entry Points
// ---------------------------------------------------------------------------

/// Load and initialize a 32-bit Winamp plugin DLL into the host.
///
/// Requires canonicalization and passive DLL preflight, refuses source changes,
/// and loads only a sealed, byte-identical private snapshot. Preflight/identity
/// refusals precede TLS/DllMain. ABI results (null module/version) necessarily
/// follow DllMain and the getter; they are not pre-execution safety checks.
/// `approved_image` is a full-byte allowlist entry, not a hash or a file path.
/// There is deliberately no fallback that approves bytes read from `path`.
///
/// # Safety
///
/// Must be called on the UI thread owning `window`. The target plugin code must be trusted
/// code explicitly approved for execution (e.g. freely licensed `in_openmpt.dll`),
/// including its transitive dependencies. Its pointers/callbacks must obey S09.
/// Preflight cannot establish these invariants or reject arbitrary valid malware.
/// `approved_image` must come from an independently trusted approval/staging
/// decision, not be copied automatically from the untrusted candidate at load
/// time. Approval of a filename, machine header or export name is insufficient.
pub unsafe fn load_plugin(
    path: &Path,
    approved_image: &[u8],
    window: &HostWindow,
) -> Result<LoadedPlugin, PluginLoadError> {
    let image = PreparedImage::read(path)?;
    image.require_approval(approved_image)?;

    // 2. Target architecture check: 32-bit x86 only (S09 §0 lines 28–32).
    #[cfg(not(windows))]
    {
        let _ = (window, image);
        Err(PluginLoadError::NotWindowsPlatform)
    }

    #[cfg(windows)]
    {
        if !cfg!(target_arch = "x86") {
            return Err(PluginLoadError::NotX86Architecture);
        }
        // SAFETY: caller verified safety invariants for load_plugin.
        unsafe { load_plugin_x86(image, window) }
    }
}

/// Load and initialize a 32-bit Winamp plugin DLL using an explicit approval store.
///
/// 1. Reads the candidate plugin DLL via [`PreparedImage::read`].
///    This canonicalizes the path, verifies PE structure, section bounds, and undecorated export getters.
/// 2. Verifies the candidate's exact bytes against the [`crate::approval::ApprovalStore`].
///    Refuses if unapproved, wrong machine, wrong size, or hash mismatch.
/// 3. Executes the verified-byte snapshot pipeline:
///    Re-opens the source under `FILE_SHARE_READ`, locks ancestors, copies to a private snapshot,
///    verifies snapshot bytes, and passes only the private snapshot to `LoadLibraryExW`.
///
/// Keeps all existing preflight, snapshot, and anti-TOCTOU guarantees intact.
///
/// # Safety
///
/// Must be called on the UI thread owning `window`. The caller relies on the approval store's
/// prior authorization decision for execution of the plugin (including its dependencies).
pub unsafe fn load_plugin_with_store(
    path: &Path,
    store: &crate::approval::ApprovalStore,
    window: &HostWindow,
) -> Result<LoadedPlugin, PluginLoadError> {
    let image = PreparedImage::read(path)?;
    image.require_store_approval(store)?;

    #[cfg(not(windows))]
    {
        let _ = (window, image);
        Err(PluginLoadError::NotWindowsPlatform)
    }

    #[cfg(windows)]
    {
        if !cfg!(target_arch = "x86") {
            return Err(PluginLoadError::NotX86Architecture);
        }
        // SAFETY: caller verified safety invariants and store approval.
        unsafe { load_plugin_x86(image, window) }
    }
}

#[cfg(windows)]
// SAFETY: caller must have checked independent byte approval and must trust
// the DLL/dependencies for execution, including TLS and DllMain.
unsafe fn map_plugin_image(
    image: &PreparedImage,
) -> Result<*mut core::ffi::c_void, PluginLoadError> {
    if !cfg!(target_arch = "x86") {
        return Err(PluginLoadError::NotX86Architecture);
    }
    let sealed = image.seal()?;
    let directory = wide_path(image.path.parent().expect("canonical file has parent"))?;
    // AddDllDirectory changes only the explicitly selected USER_DIRS set. Never
    // enable legacy CWD/PATH search. The caller trusts dependencies separately.
    // SAFETY: directory is an absolute, NUL-terminated Windows path.
    let cookie = unsafe { AddDllDirectory(directory.as_ptr()) };
    if cookie.is_null() {
        // SAFETY: GetLastError has no pointer arguments; capture immediately.
        return Err(PluginLoadError::SearchDirectoryFailed(unsafe {
            GetLastError()
        }));
    }
    let search_directory = SearchDirectory(cookie);
    let wide_path = wide_path(&sealed.path())?;
    // SAFETY: the snapshot and its namespace stay pinned through this call.
    // Caller approves execution; structural checks do not establish trust.
    let hinstance = unsafe {
        LoadLibraryExW(
            wide_path.as_ptr(),
            core::ptr::null_mut(),
            0x0000_0100 | 0x0000_0400 | 0x0000_0800, // DLL_LOAD_DIR | USER_DIRS | SYSTEM32
        )
    };
    if hinstance.is_null() {
        // SAFETY: capture this thread's loader failure immediately.
        let err = unsafe { GetLastError() };
        return Err(PluginLoadError::OsLoadFailed(err));
    }
    sealed.retain_for_process();
    drop(search_directory);
    Ok(hinstance)
}

#[cfg(windows)]
// SAFETY: same UI-thread and approved-code invariants as load_plugin.
unsafe fn load_plugin_x86(
    image: PreparedImage,
    window: &HostWindow,
) -> Result<LoadedPlugin, PluginLoadError> {
    let host_hwnd = window.hwnd_raw();
    if host_hwnd == 0 {
        return Err(PluginLoadError::MissingHostWindow);
    }
    // SAFETY: load_plugin checked approval; caller trusts code/dependencies.
    let hinstance = unsafe { map_plugin_image(&image)? };

    // Preflight chose the export without executing DllMain or a getter. Actual
    // address resolution is still checked; a trusted DllMain can alter memory.
    let (name, diagnostic) = match image.kind {
        PluginKind::Input => (
            rustamp_plugin_abi::common::EXPORT_IN_GET_MODULE,
            "winampGetInModule2",
        ),
        PluginKind::Dsp => (
            rustamp_plugin_abi::common::EXPORT_DSP_GET_HEADER,
            "winampDSPGetHeader2",
        ),
        PluginKind::General => (
            rustamp_plugin_abi::common::EXPORT_GEN_GET_PLUGIN,
            "winampGetGeneralPurposePlugin",
        ),
        PluginKind::Output => (
            rustamp_plugin_abi::common::EXPORT_OUT_GET_MODULE,
            "winampGetOutModule",
        ),
        PluginKind::Visualization => (
            rustamp_plugin_abi::common::EXPORT_VIS_GET_HEADER,
            "winampVisGetHeader",
        ),
    };
    // SAFETY: mapped module handle, static NUL-terminated export name.
    let export = unsafe { GetProcAddress(hinstance, name.as_ptr()) };
    if export.is_null() {
        return Err(PluginLoadError::MissingExport(diagnostic));
    }
    match image.kind {
        // SAFETY: approved mapped getter and UI-thread HWND, per caller contract.
        PluginKind::Input => unsafe { load_input_module(image.path, hinstance, export, host_hwnd) },
        // SAFETY: approved mapped getter and UI-thread HWND, per caller contract.
        PluginKind::Dsp => unsafe { load_dsp_module(image.path, hinstance, export, host_hwnd) },
        // SAFETY: approved mapped getter and UI-thread HWND, per caller contract.
        PluginKind::General => unsafe {
            load_general_module(image.path, hinstance, export, host_hwnd)
        },
        // SAFETY: approved mapped getter and UI-thread HWND, per caller contract.
        PluginKind::Output => unsafe {
            load_output_module(image.path, hinstance, export, host_hwnd)
        },
        // SAFETY: approved mapped getter and UI-thread HWND, per caller contract.
        PluginKind::Visualization => unsafe {
            load_vis_module(image.path, hinstance, export, host_hwnd)
        },
    }
}

#[cfg(windows)]
fn wide_path(path: &Path) -> Result<Vec<u16>, PluginLoadError> {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<_> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(PluginLoadError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "NUL in plugin path",
        )));
    }
    wide.push(0);
    Ok(wide)
}

#[cfg(windows)]
struct SearchDirectory(*mut core::ffi::c_void);

#[cfg(windows)]
impl Drop for SearchDirectory {
    fn drop(&mut self) {
        // SAFETY: this guard owns a successful AddDllDirectory cookie.
        unsafe { RemoveDllDirectory(self.0) };
    }
}

#[cfg(windows)]
unsafe fn load_input_module(
    path: PathBuf,
    hinstance: *mut core::ffi::c_void,
    proc_ptr: *mut core::ffi::c_void,
    host_hwnd: usize,
) -> Result<LoadedPlugin, PluginLoadError> {
    let identity = PluginIdentity::new(&path, PluginKind::Input);
    // S09 §1 line 38, §2 line 56: cdecl, no arguments, returns *mut InModule.
    type InGetModuleFn = unsafe extern "C" fn() -> *mut InModule;
    let get_module: InGetModuleFn = unsafe { core::mem::transmute(proc_ptr) };
    // The getter is plugin code: a fault in it must name this plugin.
    let module_ptr = {
        let _scope = plugin_call_scope(&identity, "get module");
        unsafe { get_module() }
    };
    if module_ptr.is_null() {
        return Err(PluginLoadError::NullModule);
    }

    // S09 §3.2 lines 121–131: 2.95 expects 0x100.
    // Mask 5.x wide-filename (0x0F00_0000) and init-status (0xF000_0000) flags.
    let version = unsafe { (*module_ptr).version };
    let base_version =
        version & !(IN_VERSION_FLAG_WIDE_FILENAMES as i32 | IN_VERSION_FLAG_INIT_STATUS as i32);
    if base_version != IN_VERSION {
        return Err(PluginLoadError::UnsupportedVersion {
            kind: PluginKind::Input,
            actual: version,
            expected: IN_VERSION,
        });
    }

    let description = unsafe { read_c_string_lossy((*module_ptr).description) };

    // S09 §3.1 lines 76–115, §3.5 lines 153–156: write host-filled members before init().
    unsafe {
        (*module_ptr).main_window = host_hwnd as *mut core::ffi::c_void;
        (*module_ptr).dll_instance = hinstance;
        (*module_ptr).analyzer_init = Some(host_analyzer_init);
        (*module_ptr).analyzer_deinit = Some(host_analyzer_deinit);
        (*module_ptr).analyzer_feed_pcm = Some(host_analyzer_feed_pcm);
        (*module_ptr).analyzer_get_mode = Some(host_analyzer_get_mode);
        (*module_ptr).analyzer_feed_rows = Some(host_analyzer_feed_rows);
        (*module_ptr).vis_feed_pcm = Some(host_vis_feed_pcm);
        (*module_ptr).vis_get_mode = Some(host_vis_get_mode);
        (*module_ptr).vis_feed_frame = Some(host_vis_feed_frame);
        (*module_ptr).vis_set_format = Some(host_vis_set_format);
        (*module_ptr).dsp_is_active = Some(host_dsp_is_active);
        (*module_ptr).dsp_process = Some(host_dsp_process);
        (*module_ptr).set_info = None; // Cleared before init
        // S09 §3.1 offset 0x94, §3.5 step 2: the host sets `out_module` to the
        // selected output before `play`. No output is selected yet — the host
        // has no playback path, and the selection itself (`config_outname`) is
        // S12's business — so the slot stays empty instead of naming an
        // output the input plugin would then have to use.
        (*module_ptr).out_module = core::ptr::null_mut();
    }

    // Call init() (S09 §3.1 offset 0x24, §3.5 line 156).
    let mut init_called = false;
    if let Some(init_fn) = unsafe { (*module_ptr).init } {
        init_called = true;
        let _scope = plugin_call_scope(&identity, "init");
        unsafe { init_fn() };
    }

    // S09 §3.5 line 155: set_info is set to host's own function after init returns.
    unsafe {
        (*module_ptr).set_info = Some(host_set_info);
    }

    Ok(LoadedPlugin {
        identity,
        hinstance,
        host_hwnd,
        description,
        inner: LoadedPluginInner::Input {
            module: module_ptr,
            init_called,
        },
        initialized: true,
    })
}

#[cfg(windows)]
unsafe fn load_dsp_module(
    path: PathBuf,
    hinstance: *mut core::ffi::c_void,
    proc_ptr: *mut core::ffi::c_void,
    host_hwnd: usize,
) -> Result<LoadedPlugin, PluginLoadError> {
    let identity = PluginIdentity::new(&path, PluginKind::Dsp);
    // S09 §1 line 38, §2 line 59: cdecl getter returning the static DSP header.
    // In 2.x it takes no arguments; a 5.x main-window argument would be a
    // harmless extra under cdecl, so calling with none is correct either way (U-9).
    type DspGetHeaderFn = unsafe extern "C" fn() -> *mut DspHeader;
    let get_header: DspGetHeaderFn = unsafe { core::mem::transmute(proc_ptr) };
    // The getter and the module list are plugin code: a fault in them must
    // name this plugin.
    let header_ptr = {
        let _scope = plugin_call_scope(&identity, "get header");
        unsafe { get_header() }
    };
    // SAFETY: the header lives in the mapped plugin's static data; the
    // plugin's `get_module` runs here on the UI thread (S09 §6.3 step 1).
    let modules = {
        let _scope = plugin_call_scope(&identity, "list modules");
        unsafe { crate::dsp::list_dsp_modules(header_ptr)? }
    };
    // The description is read before any module runs, so a plugin that
    // rewrites its own pointer later cannot change what the host reported.
    // SAFETY: `list_dsp_modules` refused a null header, so it is live here.
    let description = unsafe { read_c_string_lossy((*header_ptr).description) };

    // S09 §6.3 step 1: the host calls `get_module(config_dspplugin_num)` and
    // then `init`. No selection is persisted yet (`config_dspplugin_num` is
    // S12's business), so the first module is the honest default.
    let Some(entry) = modules
        .iter()
        .find(|entry| entry.info.index == crate::dsp::DSP_DEFAULT_MODULE_INDEX)
    else {
        // S09 §6.3 step 1: a null header or module unloads the DLL.
        return Err(PluginLoadError::NullModule);
    };
    let module = entry.module;
    // SAFETY: live module in the mapped plugin's static data; `init` runs on
    // the UI thread that owns the host window.
    let Some(init) = ({
        let _scope = plugin_call_scope(&identity, "init");
        unsafe { crate::dsp::init_dsp_module(module, host_hwnd, hinstance) }
    }) else {
        return Err(PluginLoadError::NullModule);
    };
    // Own choice, not S09: §6.2 says "0 on success" and nothing more about a
    // nonzero return. The host refuses before activation, so no caller can
    // reach a module the plugin did not accept (P6 §2).
    if init.init_result != 0 {
        return Err(PluginLoadError::InitFailed {
            kind: PluginKind::Dsp,
            code: init.init_result,
        });
    }
    crate::dsp::set_active_dsp_module(module);

    Ok(LoadedPlugin {
        identity,
        hinstance,
        host_hwnd,
        description,
        inner: LoadedPluginInner::Dsp {
            header: header_ptr,
            modules,
            active: Some(module),
            init_called: init.init_called,
            init_result: init.init_result,
        },
        initialized: true,
    })
}

#[cfg(windows)]
unsafe fn load_general_module(
    path: PathBuf,
    hinstance: *mut core::ffi::c_void,
    proc_ptr: *mut core::ffi::c_void,
    host_hwnd: usize,
) -> Result<LoadedPlugin, PluginLoadError> {
    let identity = PluginIdentity::new(&path, PluginKind::General);
    // S09 §1 line 38, §2 line 60: cdecl, no arguments, returns *mut GeneralPlugin.
    type GenGetModuleFn = unsafe extern "C" fn() -> *mut GeneralPlugin;
    let get_module: GenGetModuleFn = unsafe { core::mem::transmute(proc_ptr) };
    // The getter is plugin code: a fault in it must name this plugin.
    let module_ptr = {
        let _scope = plugin_call_scope(&identity, "get module");
        unsafe { get_module() }
    };

    // The description is read before init() so a plugin that rewrites its own
    // pointer during init cannot change what the host reported it as.
    let description = if module_ptr.is_null() {
        String::new()
    } else {
        // SAFETY: non-null module in the mapped image's static data; a null
        // description pointer reads as empty and is not dereferenced (S09 §3).
        unsafe { read_c_string_lossy((*module_ptr).description) }
    };

    // SAFETY: getter result from the approved mapped image; UI thread.
    let init = {
        let _scope = plugin_call_scope(&identity, "init");
        unsafe { init_general_module(module_ptr, host_hwnd, hinstance) }?
    };

    Ok(LoadedPlugin {
        identity,
        hinstance,
        host_hwnd,
        description,
        inner: LoadedPluginInner::General {
            module: init.module,
            init_called: init.init_called,
            init_result: init.init_result,
        },
        initialized: true,
    })
}

#[cfg(windows)]
unsafe fn load_output_module(
    path: PathBuf,
    hinstance: *mut core::ffi::c_void,
    proc_ptr: *mut core::ffi::c_void,
    host_hwnd: usize,
) -> Result<LoadedPlugin, PluginLoadError> {
    let identity = PluginIdentity::new(&path, PluginKind::Output);
    // S09 §1 line 38, §2 line 57: cdecl, no arguments, returns *mut Out_Module.
    type OutGetModuleFn = unsafe extern "C" fn() -> *mut OutModule;
    let get_module: OutGetModuleFn = unsafe { core::mem::transmute(proc_ptr) };
    // The getter is plugin code: a fault in it must name this plugin.
    let module_ptr = {
        let _scope = plugin_call_scope(&identity, "get module");
        unsafe { get_module() }
    };
    if module_ptr.is_null() {
        return Err(PluginLoadError::NullModule);
    }

    // The description is read before init() so a plugin that rewrites its own
    // pointer during init cannot change what the host reported it as.
    // SAFETY: non-null module in the mapped image's static data; a null
    // description pointer reads as empty and is not dereferenced (S09 §4.1).
    let description = unsafe { read_c_string_lossy((*module_ptr).description) };

    // SAFETY: getter result from the approved mapped image; UI thread.
    let init = {
        let _scope = plugin_call_scope(&identity, "init");
        unsafe { init_output_module(module_ptr, host_hwnd, hinstance) }?
    };

    Ok(LoadedPlugin {
        identity,
        hinstance,
        host_hwnd,
        description,
        inner: LoadedPluginInner::Output {
            module: init.module,
            init_called: init.init_called,
        },
        initialized: true,
    })
}

#[cfg(windows)]
unsafe fn load_vis_module(
    path: PathBuf,
    hinstance: *mut core::ffi::c_void,
    proc_ptr: *mut core::ffi::c_void,
    host_hwnd: usize,
) -> Result<LoadedPlugin, PluginLoadError> {
    let identity = PluginIdentity::new(&path, PluginKind::Visualization);
    // S09 §1 line 38, §2 line 58: cdecl getter returning the static
    // visualization header. In 2.x it takes no arguments; a 5.x main-window
    // argument would be a harmless extra under cdecl, so calling with none is
    // correct either way (U-9).
    type VisGetHeaderFn = unsafe extern "C" fn() -> *mut VisHeader;
    let get_header: VisGetHeaderFn = unsafe { core::mem::transmute(proc_ptr) };
    // The getter and the module list are plugin code: a fault in them must
    // name this plugin.
    let header_ptr = {
        let _scope = plugin_call_scope(&identity, "get header");
        unsafe { get_header() }
    };
    // SAFETY: the header lives in the mapped plugin's static data; the
    // plugin's `get_module` runs here on the UI thread (S09 §5.3 step 2).
    let modules = {
        let _scope = plugin_call_scope(&identity, "list modules");
        unsafe { crate::vis::list_vis_modules(header_ptr)? }
    };
    // The description is read before any module runs, so a plugin that
    // rewrites its own pointer later cannot change what the host reported.
    // SAFETY: `list_vis_modules` refused a null header, so it is live here.
    let description = unsafe { read_c_string_lossy((*header_ptr).description) };

    // No module is initialized yet: the user picks one, and the host starts
    // it then (S09 §5.3 step 1 and step 2).
    Ok(LoadedPlugin {
        identity,
        hinstance,
        host_hwnd,
        description,
        inner: LoadedPluginInner::Vis {
            header: header_ptr,
            modules,
            active: None,
            init_called: false,
            init_result: 0,
        },
        initialized: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pe_preflight::test_dll;
    use rustamp_plugin_abi::common::GEN_VERSION_WIDE_DESC;
    use std::cell::Cell;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(bytes: &[u8]) -> Self {
            use std::io::Write;
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "rustamp-preflight-test-{}-{}.dll",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            file.write_all(bytes).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_file(&self.0).unwrap();
        }
    }

    #[test]
    fn preflight_rejection_prevents_os_loading() {
        let fixture = Fixture::new(b"not a pe file at all");
        let window = HostWindow::dummy();
        // SAFETY: temp file is plain test data.
        let result = unsafe { load_plugin(&fixture.0, &[], &window) };
        assert!(matches!(result, Err(PluginLoadError::Preflight(_))));
    }

    #[test]
    fn wrong_architecture_and_hostile_i386_missing_export_are_refused_before_loading() {
        let mut missing_export = test_dll(0x014c);
        missing_export[0x260] = b'_';
        for (bytes, reason) in [
            (test_dll(0x8664), "0x8664"),
            (missing_export, "exports neither"),
        ] {
            let fixture = Fixture::new(&bytes);
            // SAFETY: both candidates must fail passive checks, before OS calls.
            let result = unsafe { load_plugin(&fixture.0, &[], &HostWindow::dummy()) };
            let error = result.err().expect("unsafe candidate was not refused");
            assert!(matches!(error, PluginLoadError::Preflight(_)));
            assert!(error.to_string().contains(reason), "{error}");
            assert!(error.to_string().contains("before loading"));
        }
    }

    #[test]
    fn structurally_valid_but_unapproved_i386_code_is_refused_before_loading() {
        let approved = test_dll(0x014c);
        let mut hostile = approved.clone();
        hostile[0x300] = 0xcc; // Opaque code byte; layout/export checks still match.
        assert_eq!(
            crate::pe_preflight::check_pe_i386_dll(&hostile),
            Ok(PluginKind::Input)
        );
        let fixture = Fixture::new(&hostile);
        // SAFETY: unapproved bytes must be refused before any executable load.
        let result = unsafe { load_plugin(&fixture.0, &approved, &HostWindow::dummy()) };
        let error = result.err().expect("unapproved image was not refused");
        assert!(matches!(error, PluginLoadError::NotApproved));
        assert!(error.to_string().contains("approved image"));
        assert!(error.to_string().contains("before loading"));
    }

    #[cfg(windows)]
    #[test]
    fn changed_bytes_between_preflight_and_sealing_are_refused_even_with_same_headers_and_size() {
        let mut bytes = test_dll(0x014c);
        let fixture = Fixture::new(&bytes);
        let prepared = PreparedImage::read(&fixture.0).unwrap();
        prepared.require_approval(&bytes).unwrap();
        bytes[0x500] ^= 1; // Same architecture, length, export and timestamps are irrelevant.
        std::fs::write(&fixture.0, &bytes).unwrap();
        let error = prepared
            .seal()
            .err()
            .expect("changed source was not refused");
        assert!(matches!(error, PluginLoadError::ImageChanged(_)));
        assert!(error.to_string().contains("source changed after preflight"));
        // No OS-load helper is called anywhere in this test.
    }

    #[cfg(windows)]
    #[test]
    fn unchanged_bytes_are_copied_exactly_and_locked_not_loaded() {
        let bytes = test_dll(0x014c);
        let fixture = Fixture::new(&bytes);
        let prepared = PreparedImage::read(&fixture.0).unwrap();
        prepared.require_approval(&bytes).unwrap();
        assert!(prepared.path.is_absolute());
        let sealed = prepared.seal().unwrap();
        let snapshot = sealed.path();
        assert_ne!(snapshot, prepared.path);
        assert_eq!(std::fs::read(&snapshot).unwrap(), bytes);
        assert!(
            std::fs::OpenOptions::new()
                .write(true)
                .open(&snapshot)
                .is_err()
        );
        assert!(std::fs::remove_file(&snapshot).is_err());
        assert!(
            std::fs::rename(
                snapshot.parent().unwrap(),
                snapshot.parent().unwrap().with_extension("moved")
            )
            .is_err()
        );
        drop(sealed);
        assert!(!snapshot.exists(), "unmapped snapshot must be cleaned up");
    }

    #[cfg(windows)]
    #[test]
    fn disappearance_after_preflight_is_an_io_refusal_not_a_relative_path_fallback() {
        let fixture = Fixture::new(&test_dll(0x014c));
        let prepared = PreparedImage::read(&fixture.0).unwrap();
        drop(fixture);
        assert!(matches!(prepared.seal(), Err(PluginLoadError::Io(_))));
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "VM-only i686: maps approved in_openmpt and executes DLL initialization; no host window/getter/init"]
    fn approved_real_dll_maps_headlessly_from_verified_snapshot() {
        crate::host_window::require_test_vm();
        if !cfg!(target_arch = "x86") {
            panic!("real DLL mapping test requires i686");
        }
        let base = PathBuf::from(
            std::env::var_os("RUSTAMP_TESTPLUGINS").expect("missing staged approved DLL path"),
        );
        let path = if base.is_file() {
            base
        } else {
            let nested = base.join("libopenmpt/Winamp/in_openmpt.dll");
            if nested.is_file() {
                nested
            } else {
                base.join("in_openmpt.dll")
            }
        };
        assert_eq!(
            path.file_name().unwrap(),
            "in_openmpt.dll",
            "this test approves only the staged fixture"
        );
        // Like the integration runner, this test trusts the owner-approved staged
        // fixture, not an arbitrary candidate path as a production trust policy.
        let approved = std::fs::read(&path).expect("approved in_openmpt fixture missing");
        let image = PreparedImage::read(&path).unwrap();
        image.require_approval(&approved).unwrap();
        // SAFETY: VM proved before file access; i686; approved staged fixture and
        // dependencies. This invokes the production map primitive, not a mock.
        let module = unsafe { map_plugin_image(&image) }.expect("approved DLL must still map");
        assert!(!module.is_null());
        // Intentionally no HostWindow, getter, plugin init/quit, audio or unload.
        // The OS necessarily executes TLS/DllMain during this mapping test.
    }

    #[test]
    fn missing_file_returns_io_error() {
        let non_existent = Path::new("does_not_exist_in_rustamp_plugin_loader.dll");
        let window = HostWindow::dummy();

        // SAFETY: non-existent path.
        let result = unsafe { load_plugin(non_existent, &[], &window) };
        assert!(matches!(result, Err(PluginLoadError::Io(_))));
    }

    #[test]
    fn loader_with_store_approved_image_loads_to_arch_boundary() {
        use crate::approval::ApprovalStore;
        let mut store = ApprovalStore::new();
        let bytes = test_dll(0x014c);
        let fixture = Fixture::new(&bytes);
        store.approve_bytes("test_plugin.dll", &bytes).unwrap();

        // SAFETY: fixture is test data, window is dummy.
        let result = unsafe { load_plugin_with_store(&fixture.0, &store, &HostWindow::dummy()) };

        #[cfg(not(windows))]
        assert!(matches!(result, Err(PluginLoadError::NotWindowsPlatform)));

        #[cfg(windows)]
        {
            if !cfg!(target_arch = "x86") {
                assert!(matches!(result, Err(PluginLoadError::NotX86Architecture)));
            }
        }
    }

    #[test]
    fn loader_with_store_changed_image_is_refused_before_loading() {
        use crate::approval::ApprovalStore;
        let mut store = ApprovalStore::new();
        let bytes = test_dll(0x014c);
        store.approve_bytes("test_plugin.dll", &bytes).unwrap();

        let mut hostile = bytes.clone();
        hostile[0x300] ^= 0x99;
        let fixture = Fixture::new(&hostile);

        // SAFETY: candidate must fail before any execution.
        let result = unsafe { load_plugin_with_store(&fixture.0, &store, &HostWindow::dummy()) };
        assert!(matches!(result, Err(PluginLoadError::NotApproved)));
    }

    #[test]
    fn loader_with_store_malformed_store_refuses_loudly() {
        use crate::approval::ApprovalStore;
        let malformed = "version = 999\n";
        let err = ApprovalStore::parse(malformed).expect_err("unsupported version must fail");
        assert!(matches!(
            err,
            crate::approval::ApprovalStoreError::UnsupportedVersion(999)
        ));

        let temp_corrupt =
            std::env::temp_dir().join(format!("rustamp-corrupt-{}.toml", std::process::id()));
        std::fs::write(&temp_corrupt, b"invalid toml garbage without version").unwrap();
        let open_err =
            ApprovalStore::open_or_empty(&temp_corrupt).expect_err("corrupt file must not load");
        assert!(matches!(
            open_err,
            crate::approval::ApprovalStoreError::Malformed(_)
        ));
        let _ = std::fs::remove_file(&temp_corrupt);
    }

    #[test]
    fn loader_with_empty_store_approves_nothing() {
        use crate::approval::ApprovalStore;
        let empty_store = ApprovalStore::new();
        let bytes = test_dll(0x014c);
        let fixture = Fixture::new(&bytes);

        // SAFETY: candidate must fail approval before any execution.
        let result =
            unsafe { load_plugin_with_store(&fixture.0, &empty_store, &HostWindow::dummy()) };
        assert!(matches!(result, Err(PluginLoadError::NotApproved)));
    }

    // -----------------------------------------------------------------------
    // General-purpose plugin lifecycle (S09 §7, P3 §4 steps 4, 5, 9, 10)
    //
    // Synthetic in-process modules, as the DSP tests above use. These prove
    // the host's own order of operations and refusals; they map no DLL and
    // resolve no export, so they are NOT DLL acceptance (P3 §7).
    // -----------------------------------------------------------------------

    thread_local! {
        /// What the synthetic general callbacks recorded, in call order.
        static GEN_LOG: std::cell::RefCell<Vec<String>> = const {
            std::cell::RefCell::new(Vec::new())
        };
        /// The module `gen_init_reading_fields` reads its host fields from.
        static GEN_MODULE: Cell<*mut GeneralPlugin> = const { Cell::new(core::ptr::null_mut()) };
    }

    fn gen_note(entry: impl Into<String>) {
        GEN_LOG.with(|log| log.borrow_mut().push(entry.into()));
    }

    fn gen_log() -> Vec<String> {
        GEN_LOG.with(|log| log.borrow().clone())
    }

    fn gen_log_clear() {
        GEN_LOG.with(|log| log.borrow_mut().clear());
        GEN_MODULE.with(|slot| slot.set(core::ptr::null_mut()));
    }

    /// Reads the two members the host must have filled *before* `init` runs,
    /// so a pass pins the ordering and not just the final field values.
    unsafe extern "C" fn gen_init_reading_fields() -> core::ffi::c_int {
        let module = GEN_MODULE.with(|slot| slot.get());
        if module.is_null() {
            gen_note("init with no module");
            return -1;
        }
        // SAFETY: the test owns the module and keeps it alive across the call.
        let (parent, instance) = unsafe { ((*module).parent_window, (*module).dll_instance) };
        gen_note(format!(
            "init parent={:#x} instance={:#x}",
            parent as usize, instance as usize
        ));
        0
    }

    unsafe extern "C" fn gen_init_refusing() -> core::ffi::c_int {
        gen_note("init");
        7
    }

    unsafe extern "C" fn gen_config() {
        gen_note("config");
    }

    unsafe extern "C" fn gen_quit() {
        gen_note("quit");
    }

    fn gen_module(
        init: Option<unsafe extern "C" fn() -> core::ffi::c_int>,
        config: Option<unsafe extern "C" fn()>,
        quit: Option<unsafe extern "C" fn()>,
    ) -> GeneralPlugin {
        GeneralPlugin {
            version: GEN_VERSION,
            description: c"synthetic general".as_ptr(),
            init,
            config,
            quit,
            ..GeneralPlugin::default()
        }
    }

    #[test]
    fn general_version_refusal_precedes_any_callback() {
        gen_log_clear();
        let mut module = gen_module(
            Some(gen_init_reading_fields),
            Some(gen_config),
            Some(gen_quit),
        );
        // 0x11 is 5.x-only (S09 §7.1); 2.95 expects 0x10 exactly.
        module.version = GEN_VERSION_WIDE_DESC;
        GEN_MODULE.with(|slot| slot.set(&mut module));
        // SAFETY: live stack module; no DLL is involved.
        let err = unsafe {
            init_general_module(&mut module, 0x1234, 0x5678_usize as *mut core::ffi::c_void)
        }
        .expect_err("version 0x11 must be refused");
        assert!(
            matches!(
                err,
                PluginLoadError::UnsupportedVersion {
                    kind: PluginKind::General,
                    actual: 0x11,
                    expected: 0x10,
                }
            ),
            "unexpected refusal: {err}"
        );
        assert!(
            gen_log().is_empty(),
            "no plugin callback may run before the version check, but ran {:?}",
            gen_log()
        );
        // Nothing of the host was published into a module it refused.
        assert!(module.parent_window.is_null());
        assert!(module.dll_instance.is_null());
    }

    #[test]
    fn general_host_fields_are_visible_to_init_and_recorded() {
        gen_log_clear();
        let mut module = gen_module(
            Some(gen_init_reading_fields),
            Some(gen_config),
            Some(gen_quit),
        );
        GEN_MODULE.with(|slot| slot.set(&mut module));
        // SAFETY: live stack module; no DLL is involved.
        let init = unsafe {
            init_general_module(&mut module, 0xBEEF, 0xCAFE_usize as *mut core::ffi::c_void)
        }
        .expect("a 0x10 module with every slot must initialize");
        assert!(init.init_called);
        assert_eq!(init.init_result, 0);
        // The ordering: init saw both host members already filled.
        assert_eq!(gen_log(), vec!["init parent=0xbeef instance=0xcafe"]);

        let plugin = LoadedPlugin::general_for_test(
            "gen_synthetic",
            &mut module,
            init.init_called,
            init.init_result,
        );
        assert_eq!(
            plugin.general_info().expect("general plugins report info"),
            GeneralModuleInfo {
                has_init: true,
                has_config: true,
                has_quit: true,
                parent_window: 0xBEEF,
                dll_instance: 0xCAFE,
                init_called: true,
                init_result: 0,
            }
        );
        // A general plugin is not a DSP one and says so.
        assert_eq!(plugin.dsp_info(), None);
    }

    #[test]
    fn general_null_module_and_null_init_are_reported_not_called() {
        gen_log_clear();
        // SAFETY: a null module is exactly the input under test.
        let err = unsafe {
            init_general_module(
                core::ptr::null_mut(),
                0x1,
                0x2_usize as *mut core::ffi::c_void,
            )
        }
        .expect_err("a null module must be refused");
        assert!(matches!(err, PluginLoadError::NullModule), "{err}");

        let mut bare = gen_module(None, None, None);
        // SAFETY: live stack module with every plugin slot null.
        let init =
            unsafe { init_general_module(&mut bare, 0x11, 0x22_usize as *mut core::ffi::c_void) }
                .expect("S09 §7 does not require any slot, so this still loads");
        assert!(!init.init_called, "a null init must not be reported as run");
        assert_eq!(init.init_result, 0);
        assert!(gen_log().is_empty());

        let plugin = LoadedPlugin::general_for_test(
            "gen_bare",
            &mut bare,
            init.init_called,
            init.init_result,
        );
        assert_eq!(
            plugin.general_info().unwrap(),
            GeneralModuleInfo {
                has_init: false,
                has_config: false,
                has_quit: false,
                parent_window: 0x11,
                dll_instance: 0x22,
                init_called: false,
                init_result: 0,
            },
            "the absent slots are what an acceptance runner refuses on"
        );
    }

    #[test]
    fn general_init_refusal_is_recorded_and_not_turned_into_a_verdict() {
        gen_log_clear();
        let mut module = gen_module(Some(gen_init_refusing), Some(gen_config), Some(gen_quit));
        // SAFETY: live stack module; no DLL is involved.
        let init =
            unsafe { init_general_module(&mut module, 0x30, 0x40_usize as *mut core::ffi::c_void) }
                .expect("S09 U-8 is open: a non-zero general init is recorded, not a refusal");
        assert!(init.init_called);
        assert_eq!(init.init_result, 7, "the plugin's own value must survive");
        assert_eq!(gen_log(), vec!["init"]);
    }

    #[test]
    fn general_config_runs_once_and_a_null_slot_is_refused() {
        gen_log_clear();
        let mut module = gen_module(None, Some(gen_config), Some(gen_quit));
        let plugin = LoadedPlugin::general_for_test("gen_cfg", &mut module, false, 0);
        assert_eq!(plugin.config(), Ok(()));
        assert_eq!(gen_log(), vec!["config"], "config must run exactly once");

        gen_log_clear();
        let mut without = gen_module(None, None, Some(gen_quit));
        let silent = LoadedPlugin::general_for_test("gen_nocfg", &mut without, false, 0);
        assert_eq!(silent.config(), Err(PluginConfigError::MissingCallback));
        assert!(
            gen_log().is_empty(),
            "a null config slot must never be called"
        );
    }

    #[test]
    fn general_config_after_quit_and_on_other_kinds_is_refused() {
        gen_log_clear();
        let mut module = gen_module(None, Some(gen_config), Some(gen_quit));
        let mut plugin = LoadedPlugin::general_for_test("gen_cfg", &mut module, false, 0);
        plugin.quit();
        assert_eq!(plugin.config(), Err(PluginConfigError::NotInitialized));
        assert_eq!(gen_log(), vec!["quit"], "only quit may have run");

        // A plugin the host has no kind bridge for is refused without a call:
        // the host does not know which `config` signature to use, and it never
        // guesses one. The loader maps no such plugin; the variant exists so
        // that such a request is a reported error.
        let other = LoadedPlugin::unsupported_for_test(PluginKind::General);
        assert_eq!(
            other.config(),
            Err(PluginConfigError::WrongKind(PluginKind::General))
        );
        assert_eq!(other.general_info(), None);
    }

    #[test]
    fn general_quit_runs_once_and_is_idempotent() {
        gen_log_clear();
        let mut module = gen_module(
            Some(gen_init_reading_fields),
            Some(gen_config),
            Some(gen_quit),
        );
        let mut plugin = LoadedPlugin::general_for_test("gen_quit", &mut module, true, 0);
        assert!(plugin.is_initialized());
        plugin.quit();
        assert!(!plugin.is_initialized());
        plugin.quit();
        drop(plugin); // Drop quits too; it must not call the plugin again.
        assert_eq!(
            gen_log(),
            vec!["quit"],
            "quit must reach the plugin exactly once"
        );
        // The module struct stays readable after quit (the mapping is retained).
        assert_eq!(module.version, GEN_VERSION);
    }

    // -----------------------------------------------------------------------
    // Output plugin lifecycle (S09 §4)
    //
    // Synthetic in-process modules, as the general tests above use. These
    // prove the host's own order of operations and refusals; they map no DLL
    // and resolve no export, so they are NOT DLL acceptance (P3 §7).
    // -----------------------------------------------------------------------

    thread_local! {
        /// What the synthetic output callbacks recorded, in call order.
        static OUT_LOG: std::cell::RefCell<Vec<String>> = const {
            std::cell::RefCell::new(Vec::new())
        };
        /// The module `out_init_reading_fields` reads its host fields from.
        static OUT_MODULE: Cell<*mut OutModule> = const { Cell::new(core::ptr::null_mut()) };
    }

    fn out_note(entry: impl Into<String>) {
        OUT_LOG.with(|log| log.borrow_mut().push(entry.into()));
    }

    fn out_log() -> Vec<String> {
        OUT_LOG.with(|log| log.borrow().clone())
    }

    fn out_log_clear() {
        OUT_LOG.with(|log| log.borrow_mut().clear());
        OUT_MODULE.with(|slot| slot.set(core::ptr::null_mut()));
    }

    /// Reads the two members the host must have filled *before* `init` runs,
    /// so a pass pins the ordering and not just the final field values.
    unsafe extern "C" fn out_init_reading_fields() {
        let module = OUT_MODULE.with(|slot| slot.get());
        if module.is_null() {
            out_note("init with no module");
            return;
        }
        // SAFETY: the test owns the module and keeps it alive across the call.
        let (parent, instance, id) =
            unsafe { ((*module).main_window, (*module).dll_instance, (*module).id) };
        out_note(format!(
            "init parent={:#x} instance={:#x} id={id}",
            parent as usize, instance as usize
        ));
    }

    unsafe extern "C" fn out_quit() {
        out_note("quit");
    }

    /// The output's `config` receives the parent window the host handed the
    /// plugin (S09 §4.1 offset 0x14).
    unsafe extern "C" fn out_config(parent: *mut core::ffi::c_void) {
        out_note(format!("config parent={:#x}", parent as usize));
    }

    /// The plugin's own `open`: the host must never call it (S09 §4.2 step 3
    /// drives the output from the input plugin's decode thread).
    unsafe extern "C" fn out_open(
        _sample_rate: core::ffi::c_int,
        _channels: core::ffi::c_int,
        _bits: core::ffi::c_int,
        _buffer_ms: core::ffi::c_int,
        _prebuffer_ms: core::ffi::c_int,
    ) -> core::ffi::c_int {
        out_note("open");
        0
    }

    fn out_module(
        init: Option<unsafe extern "C" fn()>,
        quit: Option<unsafe extern "C" fn()>,
    ) -> OutModule {
        OutModule {
            version: OUT_VERSION,
            description: c"synthetic output".as_ptr(),
            // A third-party module should use a value >= 65536 (S09 §4.1).
            id: 0x0001_0000,
            init,
            quit,
            open: Some(out_open),
            ..OutModule::default()
        }
    }

    #[test]
    fn output_version_refusal_precedes_any_callback() {
        out_log_clear();
        let mut module = out_module(Some(out_init_reading_fields), Some(out_quit));
        // 0x11 is 5.x-only (S09 §4.1); 2.95 expects 0x10 exactly.
        module.version = rustamp_plugin_abi::common::OUT_VERSION_WIDE_DESC;
        OUT_MODULE.with(|slot| slot.set(&mut module));
        // SAFETY: live stack module; no DLL is involved.
        let err = unsafe {
            init_output_module(&mut module, 0x1234, 0x5678_usize as *mut core::ffi::c_void)
        }
        .expect_err("version 0x11 must be refused");
        assert!(
            matches!(
                err,
                PluginLoadError::UnsupportedVersion {
                    kind: PluginKind::Output,
                    actual: 0x11,
                    expected: 0x10,
                }
            ),
            "unexpected refusal: {err}"
        );
        assert!(
            out_log().is_empty(),
            "no plugin callback may run before the version check, but ran {:?}",
            out_log()
        );
        // Nothing of the host was published into a module it refused.
        assert!(module.main_window.is_null());
        assert!(module.dll_instance.is_null());
    }

    #[test]
    fn output_host_fields_are_visible_to_init_and_the_id_is_untouched() {
        out_log_clear();
        let mut module = out_module(Some(out_init_reading_fields), Some(out_quit));
        OUT_MODULE.with(|slot| slot.set(&mut module));
        // SAFETY: live stack module; no DLL is involved.
        let init = unsafe {
            init_output_module(&mut module, 0xBEEF, 0xCAFE_usize as *mut core::ffi::c_void)
        }
        .expect("a 0x10 module must initialize");
        assert!(init.init_called);
        // The ordering: init saw both host members already filled, and the
        // host did not touch the id (S09 §4.1 line 231, U-4).
        assert_eq!(
            out_log(),
            vec!["init parent=0xbeef instance=0xcafe id=65536"]
        );

        let plugin =
            LoadedPlugin::output_for_test("out_synthetic", &mut module, init.init_called, 0);
        assert_eq!(
            plugin.output_info().expect("output plugins report info"),
            OutModuleInfo {
                has_init: true,
                has_quit: true,
                main_window: 0xBEEF,
                dll_instance: 0xCAFE,
                id: 0x0001_0000,
                init_called: true,
            }
        );
        // An output plugin is neither a general nor a DSP one and says so.
        assert_eq!(plugin.general_info(), None);
        assert_eq!(plugin.dsp_info(), None);
    }

    #[test]
    fn output_host_calls_no_playback_callback() {
        out_log_clear();
        let mut module = out_module(Some(out_init_reading_fields), Some(out_quit));
        OUT_MODULE.with(|slot| slot.set(&mut module));
        // SAFETY: live stack module; no DLL is involved.
        unsafe { init_output_module(&mut module, 0x11, 0x22_usize as *mut core::ffi::c_void) }
            .expect("a 0x10 module must initialize");
        assert!(
            !out_log().iter().any(|entry| entry == "open"),
            "the host must not open the output: the input plugin drives it (S09 §4.2)"
        );
    }

    #[test]
    fn output_null_module_and_null_init_are_reported_not_called() {
        out_log_clear();
        // SAFETY: a null module is exactly the input under test.
        let err = unsafe {
            init_output_module(
                core::ptr::null_mut(),
                0x1,
                0x2_usize as *mut core::ffi::c_void,
            )
        }
        .expect_err("a null module must be refused");
        assert!(matches!(err, PluginLoadError::NullModule), "{err}");

        let mut bare = out_module(None, None);
        // SAFETY: live stack module with every plugin slot null.
        let init =
            unsafe { init_output_module(&mut bare, 0x11, 0x22_usize as *mut core::ffi::c_void) }
                .expect("S09 §4 requires no slot, so this still loads");
        assert!(!init.init_called, "a null init must not be reported as run");
        assert!(out_log().is_empty());

        let plugin = LoadedPlugin::output_for_test("out_bare", &mut bare, init.init_called, 0);
        assert_eq!(
            plugin.output_info().unwrap(),
            OutModuleInfo {
                has_init: false,
                has_quit: false,
                main_window: 0x11,
                dll_instance: 0x22,
                id: 0x0001_0000,
                init_called: false,
            },
            "the absent slots are what an acceptance runner refuses on"
        );
    }

    #[test]
    fn output_quit_runs_once_and_is_idempotent() {
        out_log_clear();
        let mut module = out_module(Some(out_init_reading_fields), Some(out_quit));
        let mut plugin = LoadedPlugin::output_for_test("out_quit", &mut module, true, 0);
        assert!(plugin.is_initialized());
        plugin.quit();
        assert!(!plugin.is_initialized());
        plugin.quit();
        drop(plugin); // Drop quits too; it must not call the plugin again.
        assert_eq!(
            out_log(),
            vec!["quit"],
            "quit must reach the plugin exactly once"
        );
        // The module struct stays readable after quit (the mapping is retained).
        assert_eq!(module.version, OUT_VERSION);
    }

    #[test]
    fn output_config_runs_once_with_the_parent_window() {
        out_log_clear();
        let mut module = out_module(Some(out_init_reading_fields), Some(out_quit));
        module.config = Some(out_config);
        // The host window the plugin was handed is what its `config` gets
        // (S09 §4.1 offset 0x14).
        let plugin = LoadedPlugin::output_for_test("out_config", &mut module, true, 0xBEEF);
        assert_eq!(plugin.config(), Ok(()));
        assert_eq!(
            out_log(),
            vec!["config parent=0xbeef"],
            "the config callback must run exactly once, with the parent window"
        );

        // A module without a `config` slot is refused, never called.
        out_log_clear();
        let mut bare = out_module(Some(out_init_reading_fields), Some(out_quit));
        let plugin = LoadedPlugin::output_for_test("out_bare", &mut bare, true, 0xBEEF);
        assert_eq!(plugin.config(), Err(PluginConfigError::MissingCallback));
        assert!(
            out_log().is_empty(),
            "a null config slot must never be called"
        );

        // A plugin that was quit is not live and must not be configured.
        out_log_clear();
        let mut module = out_module(Some(out_init_reading_fields), Some(out_quit));
        module.config = Some(out_config);
        let mut plugin = LoadedPlugin::output_for_test("out_quit", &mut module, true, 0xBEEF);
        plugin.quit();
        assert_eq!(plugin.config(), Err(PluginConfigError::NotInitialized));
        assert_eq!(out_log(), vec!["quit"], "only quit may have run");
    }

    // -----------------------------------------------------------------------
    // Input plugin configuration (S09 §3.1 offset 0x1C)
    //
    // The input kind exists only on Windows (the loader maps 32-bit DLLs
    // there), so its route is exercised on Windows only. The module is a
    // synthetic in-process struct: no DLL is mapped and no export is
    // resolved, so this proves host logic only (P3 §7).
    // -----------------------------------------------------------------------

    #[cfg(windows)]
    thread_local! {
        /// What the synthetic input callbacks recorded, in call order.
        static IN_LOG: std::cell::RefCell<Vec<String>> = const {
            std::cell::RefCell::new(Vec::new())
        };
    }

    #[cfg(windows)]
    fn in_log() -> Vec<String> {
        IN_LOG.with(|log| log.borrow().clone())
    }

    #[cfg(windows)]
    fn in_log_clear() {
        IN_LOG.with(|log| log.borrow_mut().clear());
    }

    /// The input's `config` receives the parent window the host handed the
    /// plugin (S09 §3.1 offset 0x1C).
    #[cfg(windows)]
    unsafe extern "C" fn in_config(parent: *mut core::ffi::c_void) {
        IN_LOG.with(|log| {
            log.borrow_mut()
                .push(format!("config parent={:#x}", parent as usize))
        });
    }

    #[cfg(windows)]
    fn in_module(config: Option<unsafe extern "C" fn(*mut core::ffi::c_void)>) -> InModule {
        InModule {
            version: IN_VERSION,
            description: c"synthetic input".as_ptr(),
            config,
            ..InModule::default()
        }
    }

    #[cfg(windows)]
    #[test]
    fn input_config_runs_once_with_the_parent_window() {
        in_log_clear();
        let mut module = in_module(Some(in_config));
        let plugin = LoadedPlugin::input_for_test("in_config", &mut module, 0xBEEF);
        assert_eq!(plugin.config(), Ok(()));
        assert_eq!(
            in_log(),
            vec!["config parent=0xbeef"],
            "the config callback must run exactly once, with the parent window"
        );

        // A module without a `config` slot is refused, never called.
        in_log_clear();
        let mut bare = in_module(None);
        let plugin = LoadedPlugin::input_for_test("in_bare", &mut bare, 0xBEEF);
        assert_eq!(plugin.config(), Err(PluginConfigError::MissingCallback));
        assert!(
            in_log().is_empty(),
            "a null config slot must never be called"
        );
    }
}
