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
//!    `winampGetGeneralPurposePlugin` for general plugins; S09 §2).
//! 5. **Calling convention**: entry points and callbacks are `cdecl` (`extern "C"`,
//!    caller cleans stack; S09 §1 line 38).
//! 6. **Module validation**: verifies that the returned module pointer is non-null,
//!    validates the version field (`0x100` for input plugins per S09 §3.2 line 125,
//!    masking 5.x flags `0x0F00_0000` per S09 §3.2 line 127 and §11 line 437; `0x20`
//!    for DSP plugins per S09 §6.1 line 335, strict; `0x10` for
//!    general plugins per S09 §7.1 line 367), and reads the ANSI description string
//!    (S09 §1 line 40).
//! 7. **Host fields**: fills `main_window`, `dll_instance`, and host callbacks
//!    (`analyzer_*`, `vis_*`, `dsp_*`, `out_module`) before `init()` (S09 §3.1 lines 76–115,
//!    §3.5 lines 153–156).
//! 8. **Plugin initialization**: calls the plugin's `init()` callback on the UI thread
//!    (S09 §3.5 line 156, §7.2 line 377). Sets `set_info` after `init()` returns (S09 §3.5 line 155).
//! 9. **Unload policy**: at shutdown, calls `quit()` on loaded plugins in reverse
//!    order (S09 §3.5 line 166, §7.2 line 380); input and general DLLs are NOT freed
//!    at shutdown because plugins may have subclassed windows (S09 §1 line 46).
//!
//! Structural preflight is NOT a malware detector or approval gate. A valid
//! trusted DLL and its dependencies can run TLS/DllMain during LoadLibraryExW,
//! before the getter result/version can be checked. See task P5 for limits.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::fmt;
use std::path::{Path, PathBuf};

use crate::host_window::HostWindow;
use crate::pe_preflight::PePreflightError;
use rustamp_plugin_abi::PluginKind;
#[cfg(any(windows, test))]
use rustamp_plugin_abi::common::DSP_VERSION;
#[cfg(windows)]
use rustamp_plugin_abi::common::{
    GEN_VERSION, IN_VERSION, IN_VERSION_FLAG_INIT_STATUS, IN_VERSION_FLAG_WIDE_FILENAMES,
    read_c_string_lossy,
};
#[cfg(any(windows, test))]
use rustamp_plugin_abi::dsp::{DspHeader, DspModule};
#[cfg(windows)]
use rustamp_plugin_abi::general::GeneralPlugin;
#[cfg(windows)]
use rustamp_plugin_abi::input::InModule;
#[cfg(any(windows, test))]
use std::sync::atomic::{AtomicUsize, Ordering};

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
/// Returns 0 when no visualization is running; zeroes row counts.
unsafe extern "C" fn host_vis_get_mode(
    spectrum_rows: *mut core::ffi::c_int,
    waveform_rows: *mut core::ffi::c_int,
) -> core::ffi::c_int {
    if !spectrum_rows.is_null() {
        unsafe { *spectrum_rows = 0 };
    }
    if !waveform_rows.is_null() {
        unsafe { *waveform_rows = 0 };
    }
    0
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
unsafe extern "C" fn host_vis_set_format(
    _sample_rate: core::ffi::c_int,
    _channels: core::ffi::c_int,
) {
}

#[cfg(windows)]
/// Host callback: `dsp_is_active()` (S09 §3.1 offset 0x84 line 111).
/// Returns 1 while a DSP module with a `process` callback is installed
/// (S09 §6.3: one module active at a time), else 0. No EQ stage lives here;
/// the EQ-vs-DSP order is an S09 open item (U-12).
unsafe extern "C" fn host_dsp_is_active() -> core::ffi::c_int {
    let module = active_dsp_module();
    if module.is_null() {
        return 0;
    }
    // SAFETY: the slot only ever holds a module installed by a successful DSP
    // load, whose static storage outlives the retained DLL mapping (S09 §0).
    match unsafe { (*module).process } {
        Some(_) => 1,
        None => 0,
    }
}

#[cfg(windows)]
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
        call_dsp_process(
            active_dsp_module(),
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
// DSP bridge (S09 §6)
//
// Audio path: the input plugin owns its decode thread (S09 §3.6) and its
// buffer. When our `host_dsp_is_active` answers nonzero it must have sized
// that buffer for up to twice the input frames (S09 §3.5 line 207); it then
// calls our `host_dsp_process`, which runs the active DSP module's `process`
// synchronously in place on that same thread and returns the frame count now
// in the buffer. Downstream the input plugin feeds the (possibly resized)
// buffer to the vis/analyzer feed calls and the output `write` loop.
// Position in the chain: DSP runs inside the input plugin's decode loop,
// before the vis/analyzer feeds; where the host EQ applies relative to it is
// an S09 open item (U-12) this bridge does not decide.
// ---------------------------------------------------------------------------

/// Module index used when the host has no stored `config_dspplugin_num`
/// (S09 §6.3 passes the stored number; with no selection persisted yet the
/// first module is the honest default).
#[cfg(any(windows, test))]
const DSP_DEFAULT_MODULE_INDEX: core::ffi::c_int = 0;

/// Process-wide active DSP module backing the `dsp_is_active` / `dsp_process`
/// host callbacks (S09 §3.1 offsets 0x84/0x88). Only one DSP module is active
/// at a time (S09 §6.3); zero means none. An atomic suffices: the UI thread
/// installs/clears it at load/quit, the decode thread reads it, and the
/// pointed-to module lives in plugin static data while its DLL stays mapped.
#[cfg(any(windows, test))]
static ACTIVE_DSP_MODULE: AtomicUsize = AtomicUsize::new(0);

/// Read the active DSP module for the decode-thread host callbacks.
#[cfg(any(windows, test))]
fn active_dsp_module() -> *mut DspModule {
    ACTIVE_DSP_MODULE.load(Ordering::SeqCst) as *mut DspModule
}

/// Install `module` as the active DSP module. Called after a successful DSP
/// `init`; a null clears the slot.
#[cfg(any(windows, test))]
fn set_active_dsp_module(module: *mut DspModule) {
    ACTIVE_DSP_MODULE.store(module as usize, Ordering::SeqCst);
}

/// Clear the active slot, but only if it still holds `module`: a replaced or
/// quitting plugin must not deactivate its successor.
#[cfg(any(windows, test))]
fn clear_active_dsp_module(module: *mut DspModule) {
    let _ =
        ACTIVE_DSP_MODULE.compare_exchange(module as usize, 0, Ordering::SeqCst, Ordering::SeqCst);
}

/// Validate a DSP `process` return value (S09 §6.2: output frames between
/// half and double the input).
///
/// Own choice, not in S09: a return outside that range cannot be trusted with
/// the shared in-place buffer, so the host keeps the input frame count and
/// leaves the tail untouched instead of propagating garbage downstream. The
/// cdecl contract offers the decode thread no error channel, so this fallback
/// is silent at the audio path by necessity; misbehavior is observable only
/// as "length unchanged". Non-positive input is passed through untouched.
#[cfg(any(windows, test))]
fn dsp_output_frames(
    input_frames: core::ffi::c_int,
    reported_frames: core::ffi::c_int,
) -> core::ffi::c_int {
    if input_frames <= 0 {
        return input_frames;
    }
    let half = input_frames / 2 + input_frames % 2;
    let double = input_frames.checked_mul(2).unwrap_or(core::ffi::c_int::MAX);
    if reported_frames < half || reported_frames > double {
        input_frames
    } else {
        reported_frames
    }
}

/// Drive one DSP `process` call with validation (S09 §6.2).
///
/// Returns the frame count the downstream consumer must use: the plugin's
/// validated return, or `frames` unchanged when there is no module, no
/// `process` callback, a null sample pointer, or an out-of-range return.
/// A crashing or wedged plugin still takes the calling thread down with it;
/// isolation is a future milestone, documented in the task card.
///
/// # Safety
///
/// `module` must be null or a valid `*mut DspModule` whose static storage is
/// alive. When non-null with a `process` callback and a non-null `samples`,
/// `samples` must point to a readable/writable buffer of at least
/// `frames * channels` samples sized for up to twice `frames` frames, and the
/// plugin code runs synchronously on the caller's thread.
#[cfg(any(windows, test))]
unsafe fn call_dsp_process(
    module: *mut DspModule,
    samples: *mut core::ffi::c_void,
    frames: core::ffi::c_int,
    bits: core::ffi::c_int,
    channels: core::ffi::c_int,
    sample_rate: core::ffi::c_int,
) -> core::ffi::c_int {
    if module.is_null() || samples.is_null() || frames <= 0 {
        return frames;
    }
    // SAFETY: caller guarantees a live module for this read.
    let process = unsafe { (*module).process };
    let Some(process) = process else {
        return frames;
    };
    // SAFETY: module and buffer per the caller's contract; the trusted plugin
    // executes here on the caller's thread.
    let reported = unsafe { process(module, samples, frames, bits, channels, sample_rate) };
    dsp_output_frames(frames, reported)
}

/// Validate a DSP header, select module 0, fill host fields and run `init`
/// (S09 §6.1, §6.3). Returns the activated module. Separated from the OS
/// mapping step so headless tests can drive it with a synthetic header.
///
/// # Safety
///
/// `header` must be null or point at a live `winampDSPHeader`. `host_hwnd`
/// must be the real main-window handle on the UI thread; `hinstance` the
/// mapped plugin's own module handle. The plugin's `get_module`/`init` run
/// here on the caller's thread.
#[cfg(any(windows, test))]
unsafe fn init_dsp_module(
    header: *mut DspHeader,
    host_hwnd: usize,
    hinstance: *mut core::ffi::c_void,
) -> Result<*mut DspModule, PluginLoadError> {
    if header.is_null() {
        return Err(PluginLoadError::NullModule);
    }
    // SAFETY: caller guarantees a live header for these reads.
    let version = unsafe { (*header).version };
    // S09 §6.1 line 335: 0x20 for 2.x. The 5.x variants 0x21/0x22 are refused
    // (U-7 decision: strict equality, mirroring the general-plugin policy).
    if version != DSP_VERSION {
        return Err(PluginLoadError::UnsupportedVersion {
            kind: PluginKind::Dsp,
            actual: version,
            expected: DSP_VERSION,
        });
    }
    // SAFETY: same live header.
    let get_module = unsafe { (*header).get_module };
    let Some(get_module) = get_module else {
        return Err(PluginLoadError::MissingCallback(
            "winampDSPHeader.get_module",
        ));
    };
    // S09 §6.3: the host passes the stored module number; none is persisted
    // yet, so module 0 is the honest default.
    // SAFETY: cdecl getter from the approved mapped image (or the test double).
    let module = unsafe { get_module(DSP_DEFAULT_MODULE_INDEX) };
    if module.is_null() {
        return Err(PluginLoadError::NullModule);
    }
    // S09 §6.2: host fills parent window and instance before `init`.
    // SAFETY: live module in plugin static data.
    unsafe {
        (*module).parent_window = host_hwnd as *mut core::ffi::c_void;
        (*module).dll_instance = hinstance;
    }
    // S09 §6.2: `init` returns 0 on success. A refusal fails loudly here,
    // before activation: the retained mapping stays (input/general policy,
    // S09 §1 line 46) but no DSP becomes active and no caller reaches it.
    // SAFETY: init(module) per S09 §6.2 offset 0x10, on the UI thread.
    if let Some(init) = unsafe { (*module).init } {
        let code = unsafe { init(module) };
        if code != 0 {
            return Err(PluginLoadError::InitFailed {
                kind: PluginKind::Dsp,
                code,
            });
        }
    }
    set_active_dsp_module(module);
    Ok(module)
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

#[derive(Debug)]
enum LoadedPluginInner {
    #[cfg(windows)]
    Input { module: *mut InModule },
    #[cfg(windows)]
    Dsp { module: *mut DspModule },
    #[cfg(windows)]
    General { module: *mut GeneralPlugin },
    #[allow(dead_code)]
    Unsupported,
}

/// A loaded and initialized classic Winamp plugin DLL.
///
/// Holds the module handle (`HINSTANCE`) and the plugin's module structure pointer.
/// Pointers point into the plugin's static data and remain valid as long as the
/// DLL is loaded (S09 §0 line 32).
pub struct LoadedPlugin {
    path: PathBuf,
    kind: PluginKind,
    hinstance: *mut core::ffi::c_void,
    description: String,
    inner: LoadedPluginInner,
    initialized: bool,
}

impl LoadedPlugin {
    /// Canonical source path; the OS maps a private byte-identical snapshot.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Plugin kind.
    pub fn kind(&self) -> PluginKind {
        self.kind
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
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Call `quit()` on the plugin if initialized (S09 §3.5 line 166, §7.2 line 380).
    ///
    /// Per S09 §1 line 46 and §3.5 line 166, the host does not free the DLL at shutdown
    /// because plugins may have subclassed windows or spawned threads.
    pub fn quit(&mut self) {
        if !self.initialized {
            return;
        }
        match self.inner {
            #[cfg(windows)]
            LoadedPluginInner::Input { module } => {
                if !module.is_null() {
                    // SAFETY: module pointer was verified non-null at load time.
                    if let Some(quit_fn) = unsafe { (*module).quit } {
                        // SAFETY: quit() takes no arguments in InModule (S09 §3.1 offset 0x28).
                        unsafe { quit_fn() };
                    }
                }
            }
            #[cfg(windows)]
            LoadedPluginInner::Dsp { module } => {
                if !module.is_null() {
                    // SAFETY: module pointer was verified non-null at load time.
                    if let Some(quit_fn) = unsafe { (*module).quit } {
                        // SAFETY: quit(module) per S09 §6.2 offset 0x18.
                        unsafe { quit_fn(module) };
                    }
                    clear_active_dsp_module(module);
                }
            }
            #[cfg(windows)]
            LoadedPluginInner::General { module } => {
                if !module.is_null() {
                    // SAFETY: module pointer was verified non-null at load time.
                    if let Some(quit_fn) = unsafe { (*module).quit } {
                        // SAFETY: quit() takes no arguments in GeneralPlugin (S09 §7.1 offset 0x10).
                        unsafe { quit_fn() };
                    }
                }
            }
            LoadedPluginInner::Unsupported => {}
        }
        self.initialized = false;
    }

    /// Host-visible state of a loaded DSP module, read from the plugin's own
    /// module struct. `None` for any other plugin kind (and off Windows).
    ///
    /// The struct lives in plugin static data, so this stays readable after
    /// [`quit`](Self::quit) (the mapping is retained); `host_active` then reads
    /// false because `quit` clears the active slot.
    pub fn dsp_info(&self) -> Option<DspModuleInfo> {
        match self.inner {
            #[cfg(windows)]
            LoadedPluginInner::Dsp { module } if !module.is_null() => {
                // SAFETY: the module pointer was verified non-null at load time and
                // points into plugin static data of the retained mapping.
                let m = unsafe { *module };
                Some(DspModuleInfo {
                    has_config: m.config.is_some(),
                    has_init: m.init.is_some(),
                    has_process: m.process.is_some(),
                    has_quit: m.quit.is_some(),
                    parent_window: m.parent_window as usize,
                    dll_instance: m.dll_instance as usize,
                    // SAFETY: argument-free host callback reading the atomic slot.
                    host_active: unsafe { host_dsp_is_active() } != 0
                        && active_dsp_module() == module,
                })
            }
            _ => None,
        }
    }

    /// Run interleaved 16-bit PCM through the host's `dsp_process` callback, the
    /// path a real input plugin's decode loop takes (S09 §3.1 offset 0x88).
    ///
    /// `pcm` holds `frames * channels` samples; it is grown to twice that for the
    /// plugin (S09 §3.5 line 207), processed in place and then truncated to the
    /// validated output frame count, which is returned. `None` when this is not
    /// an initialized DSP plugin, `channels` is not positive or `pcm` is not a
    /// whole number of frames. A crashing or wedged plugin takes this thread
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
            #[cfg(windows)]
            LoadedPluginInner::Dsp { .. } => {
                let frames = pcm.len() / channels;
                pcm.resize(pcm.len() * 2, 0);
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
}

/// Host-visible state of a loaded DSP module (see [`LoadedPlugin::dsp_info`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DspModuleInfo {
    /// The plugin supplied a `config` callback (never invoked by the host here).
    pub has_config: bool,
    /// The plugin supplied an `init` callback.
    pub has_init: bool,
    /// The plugin supplied a `process` callback.
    pub has_process: bool,
    /// The plugin supplied a `quit` callback.
    pub has_quit: bool,
    /// `parent_window` as filled by the host before `init`.
    pub parent_window: usize,
    /// `dll_instance` as filled by the host before `init`.
    pub dll_instance: usize,
    /// The host's `dsp_is_active` answers nonzero for exactly this module.
    pub host_active: bool,
}

impl Drop for LoadedPlugin {
    fn drop(&mut self) {
        self.quit();
        // S09 §1 line 46: Input and general DLLs are NOT freed at shutdown.
        // DSP §6.3 frees the DLL after quit; this host retains it instead
        // (same conservative policy) until the VM proves free-after-quit safe.
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
        _ => unreachable!("preflight selects input/dsp/general only"),
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
        _ => unreachable!("preflight selects input/dsp/general only"),
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
    // S09 §1 line 38, §2 line 56: cdecl, no arguments, returns *mut InModule.
    type InGetModuleFn = unsafe extern "C" fn() -> *mut InModule;
    let get_module: InGetModuleFn = unsafe { core::mem::transmute(proc_ptr) };
    let module_ptr = unsafe { get_module() };
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
        (*module_ptr).out_module = core::ptr::null_mut();
    }

    // Call init() (S09 §3.1 offset 0x24, §3.5 line 156).
    if let Some(init_fn) = unsafe { (*module_ptr).init } {
        unsafe { init_fn() };
    }

    // S09 §3.5 line 155: set_info is set to host's own function after init returns.
    unsafe {
        (*module_ptr).set_info = Some(host_set_info);
    }

    Ok(LoadedPlugin {
        path,
        kind: PluginKind::Input,
        hinstance,
        description,
        inner: LoadedPluginInner::Input { module: module_ptr },
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
    // S09 §1 line 38, §2 line 59: cdecl getter returning the static DSP header.
    // In 2.x it takes no arguments; a 5.x main-window argument would be a
    // harmless extra under cdecl, so calling with none is correct either way (U-9).
    type DspGetHeaderFn = unsafe extern "C" fn() -> *mut DspHeader;
    let get_header: DspGetHeaderFn = unsafe { core::mem::transmute(proc_ptr) };
    let header_ptr = unsafe { get_header() };
    // SAFETY: header lives in plugin static data; UI thread; approved image.
    let module_ptr = unsafe { init_dsp_module(header_ptr, host_hwnd, hinstance) }?;
    let description = unsafe { read_c_string_lossy((*module_ptr).description) };

    Ok(LoadedPlugin {
        path,
        kind: PluginKind::Dsp,
        hinstance,
        description,
        inner: LoadedPluginInner::Dsp { module: module_ptr },
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
    // S09 §1 line 38, §2 line 60: cdecl, no arguments, returns *mut GeneralPlugin.
    type GenGetModuleFn = unsafe extern "C" fn() -> *mut GeneralPlugin;
    let get_module: GenGetModuleFn = unsafe { core::mem::transmute(proc_ptr) };
    let module_ptr = unsafe { get_module() };
    if module_ptr.is_null() {
        return Err(PluginLoadError::NullModule);
    }

    // S09 §7.1 line 367: 0x10 in 2.95.
    let version = unsafe { (*module_ptr).version };
    if version != GEN_VERSION {
        return Err(PluginLoadError::UnsupportedVersion {
            kind: PluginKind::General,
            actual: version,
            expected: GEN_VERSION,
        });
    }

    let description = unsafe { read_c_string_lossy((*module_ptr).description) };

    // S09 §7.1 lines 372–373: host fills parent_window and dll_instance before init().
    unsafe {
        (*module_ptr).parent_window = host_hwnd as *mut core::ffi::c_void;
        (*module_ptr).dll_instance = hinstance;
    }

    // Call init() (S09 §7.1 offset 0x08, §7.2 line 377).
    if let Some(init_fn) = unsafe { (*module_ptr).init } {
        let _ = unsafe { init_fn() };
        // S09 U-8: role of non-zero return value in 2.95 is unverified.
    }

    Ok(LoadedPlugin {
        path,
        kind: PluginKind::General,
        hinstance,
        description,
        inner: LoadedPluginInner::General { module: module_ptr },
        initialized: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pe_preflight::test_dll;

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

    // -- DSP bridge tests: synthetic in-memory modules, no DLL is loaded. --

    /// Serializes the tests that touch the process-wide active-DSP slot.
    static DSP_SLOT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Doubles every sample of a stereo i16 buffer in place (measurable).
    unsafe extern "C" fn dsp_test_gain(
        _module: *mut DspModule,
        samples: *mut core::ffi::c_void,
        frames: core::ffi::c_int,
        _bits: core::ffi::c_int,
        _channels: core::ffi::c_int,
        _sample_rate: core::ffi::c_int,
    ) -> core::ffi::c_int {
        // SAFETY: tests pass a valid i16 buffer of `frames` stereo frames.
        let out = samples as *mut i16;
        let mut i = 0_isize;
        while i < frames as isize * 2 {
            // SAFETY: index bounded by the test buffer.
            unsafe {
                let v = *out.offset(i);
                *out.offset(i) = v.saturating_mul(2);
            }
            i += 1;
        }
        frames
    }

    /// Lies about the output length (10x); the bridge must not propagate it.
    unsafe extern "C" fn dsp_test_liar(
        _module: *mut DspModule,
        _samples: *mut core::ffi::c_void,
        frames: core::ffi::c_int,
        _bits: core::ffi::c_int,
        _channels: core::ffi::c_int,
        _sample_rate: core::ffi::c_int,
    ) -> core::ffi::c_int {
        frames.saturating_mul(10)
    }

    unsafe extern "C" fn dsp_test_refusing_init(_module: *mut DspModule) -> core::ffi::c_int {
        1
    }

    fn dsp_gain_module() -> DspModule {
        DspModule {
            description: c"test gain".as_ptr(),
            process: Some(dsp_test_gain),
            ..DspModule::default()
        }
    }

    fn dsp_liar_module() -> DspModule {
        DspModule {
            description: c"test liar".as_ptr(),
            process: Some(dsp_test_liar),
            ..DspModule::default()
        }
    }

    fn dsp_no_process_module() -> DspModule {
        DspModule {
            description: c"test silent".as_ptr(),
            process: None,
            ..DspModule::default()
        }
    }

    fn dsp_refusing_module() -> DspModule {
        DspModule {
            description: c"test refusing".as_ptr(),
            init: Some(dsp_test_refusing_init),
            process: Some(dsp_test_gain),
            ..DspModule::default()
        }
    }

    use std::cell::Cell;
    thread_local! {
        /// Module the shared test getter returns; each test sets its own.
        static DSP_TEST_MODULE: Cell<*mut DspModule> = const { Cell::new(core::ptr::null_mut()) };
    }

    unsafe extern "C" fn dsp_get_test_module(_index: core::ffi::c_int) -> *mut DspModule {
        DSP_TEST_MODULE.with(|slot| slot.get())
    }

    unsafe extern "C" fn dsp_get_null_module(_index: core::ffi::c_int) -> *mut DspModule {
        core::ptr::null_mut()
    }

    #[test]
    fn dsp_gain_process_is_measurable_over_the_bridge() {
        let mut module = dsp_gain_module();
        let mut pcm = vec![1000_i16, -1000, 2000, -2000];
        // SAFETY: stack module and a buffer of 2 stereo frames.
        let out = unsafe {
            call_dsp_process(
                &mut module,
                pcm.as_mut_ptr() as *mut core::ffi::c_void,
                2,
                16,
                2,
                44_100,
            )
        };
        assert_eq!(out, 2);
        assert_eq!(pcm, vec![2000, -2000, 4000, -4000]);
    }

    #[test]
    fn dsp_without_process_or_addresses_is_a_passthrough() {
        let mut module = dsp_no_process_module();
        let mut pcm = vec![1_i16, 2, 3, 4];
        // SAFETY: stack module and buffer; then null addresses.
        let out = unsafe {
            call_dsp_process(
                &mut module,
                pcm.as_mut_ptr() as *mut core::ffi::c_void,
                2,
                16,
                2,
                44_100,
            )
        };
        assert_eq!(out, 2);
        assert_eq!(pcm, vec![1, 2, 3, 4]);
        assert_eq!(
            unsafe {
                call_dsp_process(
                    core::ptr::null_mut(),
                    pcm.as_mut_ptr() as *mut core::ffi::c_void,
                    2,
                    16,
                    2,
                    44_100,
                )
            },
            2
        );
        assert_eq!(
            unsafe { call_dsp_process(&mut module, core::ptr::null_mut(), 2, 16, 2, 44_100) },
            2
        );
    }

    #[test]
    fn dsp_output_frame_count_edges_are_honored_and_garbage_is_ignored() {
        assert_eq!(dsp_output_frames(100, 50), 50);
        assert_eq!(dsp_output_frames(100, 200), 200);
        assert_eq!(dsp_output_frames(101, 51), 51);
        for bad in [49, 201, 0, -3, i32::MAX] {
            assert_eq!(dsp_output_frames(100, bad), 100, "bad={bad}");
        }
        assert_eq!(dsp_output_frames(0, 0), 0);
        assert_eq!(dsp_output_frames(-4, 100), -4);
    }

    #[test]
    fn dsp_lying_process_return_does_not_propagate() {
        let mut module = dsp_liar_module();
        let mut pcm = vec![7_i16, 8, 9, 10];
        // SAFETY: stack module and a buffer of 2 stereo frames.
        let out = unsafe {
            call_dsp_process(
                &mut module,
                pcm.as_mut_ptr() as *mut core::ffi::c_void,
                2,
                16,
                2,
                44_100,
            )
        };
        assert_eq!(out, 2, "10x return must fall back to the input count");
    }

    #[test]
    fn dsp_header_init_refusal_version_and_null_paths() {
        let _guard = DSP_SLOT_LOCK.lock().unwrap();
        assert!(active_dsp_module().is_null());
        let mut refusing = dsp_refusing_module();
        let mut gain = dsp_gain_module();
        // SAFETY: stack headers and modules outliving each call; no library.
        unsafe {
            DSP_TEST_MODULE.with(|slot| slot.set(&mut refusing));
            let mut header = DspHeader {
                version: DSP_VERSION,
                description: c"refusing".as_ptr(),
                get_module: Some(dsp_get_test_module),
            };
            let err = init_dsp_module(&mut header, 0x1234, core::ptr::null_mut())
                .expect_err("refusing init was not loud");
            assert!(
                matches!(
                    err,
                    PluginLoadError::InitFailed {
                        kind: PluginKind::Dsp,
                        code: 1
                    }
                ),
                "{err:?}"
            );
            assert!(
                active_dsp_module().is_null(),
                "refused DSP must not activate"
            );

            DSP_TEST_MODULE.with(|slot| slot.set(&mut gain));
            let mut wrong_version = DspHeader {
                version: 0x21,
                description: c"5x".as_ptr(),
                get_module: Some(dsp_get_test_module),
            };
            let err = init_dsp_module(&mut wrong_version, 0x1234, core::ptr::null_mut())
                .expect_err("5.x version was not refused");
            assert!(
                matches!(
                    err,
                    PluginLoadError::UnsupportedVersion {
                        kind: PluginKind::Dsp,
                        actual: 0x21,
                        expected: DSP_VERSION
                    }
                ),
                "{err:?}"
            );

            let err = init_dsp_module(core::ptr::null_mut(), 0x1234, core::ptr::null_mut())
                .expect_err("null header was not refused");
            assert!(matches!(err, PluginLoadError::NullModule), "{err:?}");

            let mut no_getter = DspHeader {
                version: DSP_VERSION,
                description: c"no getter".as_ptr(),
                get_module: None,
            };
            let err = init_dsp_module(&mut no_getter, 0x1234, core::ptr::null_mut())
                .expect_err("null get_module was not refused");
            assert!(
                matches!(err, PluginLoadError::MissingCallback(_)),
                "{err:?}"
            );

            let mut null_module = DspHeader {
                version: DSP_VERSION,
                description: c"null module".as_ptr(),
                get_module: Some(dsp_get_null_module),
            };
            let err = init_dsp_module(&mut null_module, 0x1234, core::ptr::null_mut())
                .expect_err("null module was not refused");
            assert!(matches!(err, PluginLoadError::NullModule), "{err:?}");

            DSP_TEST_MODULE.with(|slot| slot.set(&mut gain));
            let mut ok = DspHeader {
                version: DSP_VERSION,
                description: c"gain".as_ptr(),
                get_module: Some(dsp_get_test_module),
            };
            let module = init_dsp_module(&mut ok, 0x1234, 0x5678 as *mut core::ffi::c_void)
                .expect("valid DSP header was refused");
            assert!(!module.is_null());
            assert_eq!(active_dsp_module(), module);
            assert_eq!((*module).parent_window, 0x1234 as *mut core::ffi::c_void);
            assert_eq!((*module).dll_instance, 0x5678 as *mut core::ffi::c_void);
            clear_active_dsp_module(module);
            assert!(active_dsp_module().is_null());
        }
    }

    #[cfg(windows)]
    #[test]
    fn dsp_host_callbacks_follow_the_active_module() {
        let _guard = DSP_SLOT_LOCK.lock().unwrap();
        assert!(active_dsp_module().is_null());
        // SAFETY: callbacks take no addresses; slot starts empty.
        assert_eq!(unsafe { host_dsp_is_active() }, 0);
        let mut module = dsp_gain_module();
        let mut silent = dsp_no_process_module();
        let mut pcm = vec![3_i16, -3, 5, -5];
        set_active_dsp_module(&mut silent);
        // SAFETY: slot holds a live stack module for the assertions below.
        assert_eq!(
            unsafe { host_dsp_is_active() },
            0,
            "module without process must read inactive"
        );
        let out = unsafe {
            host_dsp_process(pcm.as_mut_ptr() as *mut core::ffi::c_void, 2, 16, 2, 44_100)
        };
        assert_eq!(out, 2);
        assert_eq!(pcm, vec![3, -3, 5, -5]);
        set_active_dsp_module(&mut module);
        assert_eq!(unsafe { host_dsp_is_active() }, 1);
        let out = unsafe {
            host_dsp_process(pcm.as_mut_ptr() as *mut core::ffi::c_void, 2, 16, 2, 44_100)
        };
        assert_eq!(out, 2);
        assert_eq!(pcm, vec![6, -6, 10, -10]);
        // A stale clear for a module that no longer owns the slot is a no-op.
        clear_active_dsp_module(&mut silent);
        assert_eq!(active_dsp_module(), &mut module as *mut DspModule);
        clear_active_dsp_module(&mut module);
        assert!(active_dsp_module().is_null());
        assert_eq!(unsafe { host_dsp_is_active() }, 0);
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
}
