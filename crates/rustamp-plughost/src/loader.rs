//! Classic Winamp 2.x plugin DLL loader.
//!
//! Implements the plugin loading path described by `specs/S09-plugin-abi.md` and
//! `docs/tasks/P3-dll-acceptance.md`:
//! 1. **PE machine preflight** ([`crate::pe_preflight::check_pe_i386_machine`]):
//!    validates that the candidate binary is a 32-bit x86 PE image (machine `0x014c`,
//!    S09 §0 lines 28–32) before any OS loader call is made.
//! 2. **OS module mapping**: maps the 32-bit DLL using Win32 `LoadLibraryExW`
//!    with `LOAD_WITH_ALTERED_SEARCH_PATH` so dependent sibling libraries in the
//!    plugin folder (e.g. `openmpt-mpg123.dll` for `in_openmpt.dll`) are resolved
//!    correctly without polluting the host's search path.
//! 3. **Entry point resolution**: locates the exported entry point (`winampGetInModule2`
//!    for input plugins, `winampGetGeneralPurposePlugin` for general plugins; S09 §2).
//! 4. **Calling convention**: entry points and callbacks are `cdecl` (`extern "C"`,
//!    caller cleans stack; S09 §1 line 38).
//! 5. **Module validation**: verifies that the returned module pointer is non-null,
//!    validates the version field (`0x100` for input plugins per S09 §3.2 line 125,
//!    masking 5.x flags `0x0F00_0000` per S09 §3.2 line 127 and §11 line 437; `0x10` for
//!    general plugins per S09 §7.1 line 367), and reads the ANSI description string
//!    (S09 §1 line 40).
//! 6. **Host fields**: fills `main_window`, `dll_instance`, and host callbacks
//!    (`analyzer_*`, `vis_*`, `dsp_*`, `out_module`) before `init()` (S09 §3.1 lines 76–115,
//!    §3.5 lines 153–156).
//! 7. **Plugin initialization**: calls the plugin's `init()` callback on the UI thread
//!    (S09 §3.5 line 156, §7.2 line 377). Sets `set_info` after `init()` returns (S09 §3.5 line 155).
//! 8. **Unload policy**: at shutdown, calls `quit()` on loaded plugins in reverse
//!    order (S09 §3.5 line 166, §7.2 line 380); input and general DLLs are NOT freed
//!    at shutdown because plugins may have subclassed windows (S09 §1 line 46).
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::fmt;
use std::path::{Path, PathBuf};

use crate::host_window::HostWindow;
use crate::pe_preflight::{PePreflightError, check_pe_i386_machine};
use rustamp_plugin_abi::PluginKind;
#[cfg(all(windows, target_arch = "x86"))]
use rustamp_plugin_abi::common::{
    GEN_VERSION, IN_VERSION, IN_VERSION_FLAG_INIT_STATUS, IN_VERSION_FLAG_WIDE_FILENAMES,
    read_c_string_lossy,
};
#[cfg(all(windows, target_arch = "x86"))]
use rustamp_plugin_abi::general::GeneralPlugin;
#[cfg(all(windows, target_arch = "x86"))]
use rustamp_plugin_abi::input::InModule;

#[cfg(all(windows, target_arch = "x86"))]
const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;

#[cfg(all(windows, target_arch = "x86"))]
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
}

// ---------------------------------------------------------------------------
// Host callbacks for InModule (S09 §3.1 lines 102–115, §3.5 lines 153–156)
// ---------------------------------------------------------------------------

#[cfg(all(windows, target_arch = "x86"))]
/// Host callback: `analyzer_init(max_latency_ms, sample_rate)` (S09 §3.1 offset 0x60).
unsafe extern "C" fn host_analyzer_init(
    _max_latency_ms: core::ffi::c_int,
    _sample_rate: core::ffi::c_int,
) {
}

#[cfg(all(windows, target_arch = "x86"))]
/// Host callback: `analyzer_deinit()` (S09 §3.1 offset 0x64).
unsafe extern "C" fn host_analyzer_deinit() {}

#[cfg(all(windows, target_arch = "x86"))]
/// Host callback: `analyzer_feed_pcm(pcm, channels, bits, timestamp_ms)` (S09 §3.1 offset 0x68).
unsafe extern "C" fn host_analyzer_feed_pcm(
    _pcm: *mut core::ffi::c_void,
    _channels: core::ffi::c_int,
    _bits: core::ffi::c_int,
    _timestamp_ms: core::ffi::c_int,
) {
}

#[cfg(all(windows, target_arch = "x86"))]
/// Host callback: `analyzer_get_mode()` -> 4 (none/window-shade; S09 §3.1 offset 0x6C line 105).
unsafe extern "C" fn host_analyzer_get_mode() -> core::ffi::c_int {
    4
}

#[cfg(all(windows, target_arch = "x86"))]
/// Host callback: `analyzer_feed_rows(rows, timestamp_ms, mode)` -> 1 (not collecting; S09 §3.1 offset 0x70 line 106).
unsafe extern "C" fn host_analyzer_feed_rows(
    _rows: *mut core::ffi::c_void,
    _timestamp_ms: core::ffi::c_int,
    _mode: core::ffi::c_int,
) -> core::ffi::c_int {
    1
}

#[cfg(all(windows, target_arch = "x86"))]
/// Host callback: `vis_feed_pcm(pcm, channels, bits, timestamp_ms)` (S09 §3.1 offset 0x74).
unsafe extern "C" fn host_vis_feed_pcm(
    _pcm: *mut core::ffi::c_void,
    _channels: core::ffi::c_int,
    _bits: core::ffi::c_int,
    _timestamp_ms: core::ffi::c_int,
) {
}

#[cfg(all(windows, target_arch = "x86"))]
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

#[cfg(all(windows, target_arch = "x86"))]
/// Host callback: `vis_feed_frame(frame, timestamp_ms)` -> 1 (not accepted; S09 §3.1 offset 0x7C line 109).
unsafe extern "C" fn host_vis_feed_frame(
    _frame: *mut core::ffi::c_void,
    _timestamp_ms: core::ffi::c_int,
) -> core::ffi::c_int {
    1
}

#[cfg(all(windows, target_arch = "x86"))]
/// Host callback: `vis_set_format(sample_rate, channels)` (S09 §3.1 offset 0x80 line 110).
unsafe extern "C" fn host_vis_set_format(
    _sample_rate: core::ffi::c_int,
    _channels: core::ffi::c_int,
) {
}

#[cfg(all(windows, target_arch = "x86"))]
/// Host callback: `dsp_is_active()` -> 0 (no DSP active; S09 §3.1 offset 0x84 line 111).
unsafe extern "C" fn host_dsp_is_active() -> core::ffi::c_int {
    0
}

#[cfg(all(windows, target_arch = "x86"))]
/// Host callback: `dsp_process(...)` -> frames (unmodified; S09 §3.1 offset 0x88 line 112).
unsafe extern "C" fn host_dsp_process(
    _samples: *mut core::ffi::c_void,
    frames: core::ffi::c_int,
    _bits: core::ffi::c_int,
    _channels: core::ffi::c_int,
    _sample_rate: core::ffi::c_int,
) -> core::ffi::c_int {
    frames
}

#[cfg(all(windows, target_arch = "x86"))]
/// Host callback: `set_info(...)` (S09 §3.1 offset 0x90 line 114). Set after init returns (S09 §3.5 line 155).
unsafe extern "C" fn host_set_info(
    _bitrate_kbps: core::ffi::c_int,
    _sample_rate_khz: core::ffi::c_int,
    _channels: core::ffi::c_int,
    _synched: core::ffi::c_int,
) {
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
    /// Passive PE machine header preflight refused the file.
    Preflight(PePreflightError),
    /// Win32 `LoadLibraryExW` returned NULL.
    OsLoadFailed(u32),
    /// Required entry-point export was not found in the loaded DLL.
    MissingExport(&'static str),
    /// Plugin entry point returned a null module pointer.
    NullModule,
    /// Plugin returned an unsupported module version.
    UnsupportedVersion {
        kind: PluginKind,
        actual: i32,
        expected: i32,
    },
    /// Host window HWND is zero or invalid.
    MissingHostWindow,
}

impl fmt::Display for PluginLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotX86Architecture => f.write_str(
                "plugin loading requires a 32-bit x86 (i686) process; cannot map 32-bit DLL into 64-bit process",
            ),
            Self::NotWindowsPlatform => f.write_str("plugin DLL loading is supported on Windows only"),
            Self::Io(e) => write!(f, "I/O error reading plugin image: {e}"),
            Self::Preflight(e) => write!(f, "PE machine preflight refused plugin: {e}"),
            Self::OsLoadFailed(code) => {
                write!(f, "Win32 LoadLibraryExW failed with error code {code}")
            }
            Self::MissingExport(name) => {
                write!(f, "plugin DLL does not export entry point '{name}'")
            }
            Self::NullModule => f.write_str("plugin entry point returned null module pointer"),
            Self::UnsupportedVersion {
                kind,
                actual,
                expected,
            } => write!(
                f,
                "{kind:?} plugin returned unsupported version {actual:#x} (expected {expected:#x})"
            ),
            Self::MissingHostWindow => f.write_str("host window HWND is zero or not created"),
        }
    }
}

impl std::error::Error for PluginLoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            Self::Preflight(e) => Some(e),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// LoadedPlugin
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum LoadedPluginInner {
    #[cfg(all(windows, target_arch = "x86"))]
    Input { module: *mut InModule },
    #[cfg(all(windows, target_arch = "x86"))]
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
    /// Filesystem path to the loaded DLL.
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
            #[cfg(all(windows, target_arch = "x86"))]
            LoadedPluginInner::Input { module } => {
                if !module.is_null() {
                    // SAFETY: module pointer was verified non-null at load time.
                    if let Some(quit_fn) = unsafe { (*module).quit } {
                        // SAFETY: quit() takes no arguments in InModule (S09 §3.1 offset 0x28).
                        unsafe { quit_fn() };
                    }
                }
            }
            #[cfg(all(windows, target_arch = "x86"))]
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
}

impl Drop for LoadedPlugin {
    fn drop(&mut self) {
        self.quit();
        // S09 §1 line 46: Input and general DLLs are NOT freed at shutdown.
    }
}

// ---------------------------------------------------------------------------
// Loading Entry Points
// ---------------------------------------------------------------------------

/// Load and initialize a 32-bit Winamp plugin DLL into the host.
///
/// Preflights the file with [`check_pe_i386_machine`] to verify 32-bit x86 architecture,
/// maps the module with `LoadLibraryExW` (`LOAD_WITH_ALTERED_SEARCH_PATH`), resolves the
/// entry point, verifies the module version, fills the host-provided fields, and invokes
/// the plugin's `init()` callback on the UI thread.
///
/// # Safety
///
/// Must be called on the UI thread owning `window`. The target plugin code must be trusted
/// reference code (clean-room inspected, e.g. freely licensed `in_openmpt.dll`).
pub unsafe fn load_plugin(
    path: &Path,
    window: &HostWindow,
) -> Result<LoadedPlugin, PluginLoadError> {
    // 1. Passive PE machine preflight on image bytes (S09 §0, M3hC finding 11).
    let image_bytes = std::fs::read(path).map_err(PluginLoadError::Io)?;
    check_pe_i386_machine(&image_bytes).map_err(PluginLoadError::Preflight)?;

    // 2. Target architecture check: 32-bit x86 only (S09 §0 lines 28–32).
    #[cfg(not(all(windows, target_arch = "x86")))]
    {
        let _ = window;
        Err(PluginLoadError::NotX86Architecture)
    }

    #[cfg(all(windows, target_arch = "x86"))]
    {
        // SAFETY: caller verified safety invariants for load_plugin.
        unsafe { load_plugin_x86(path, window) }
    }
}

#[cfg(all(windows, target_arch = "x86"))]
unsafe fn load_plugin_x86(
    path: &Path,
    window: &HostWindow,
) -> Result<LoadedPlugin, PluginLoadError> {
    let host_hwnd = window.hwnd_raw();
    if host_hwnd == 0 {
        return Err(PluginLoadError::MissingHostWindow);
    }

    use std::os::windows::ffi::OsStrExt;
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let path_str = canonical.to_string_lossy();
    let clean_path = if let Some(stripped) = path_str.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        canonical
    };

    let mut wide_path: Vec<u16> = clean_path.as_os_str().encode_wide().collect();
    wide_path.push(0);

    // SAFETY: Path is NUL-terminated wide string; LOAD_WITH_ALTERED_SEARCH_PATH resolves sibling DLLs.
    let hinstance = unsafe {
        LoadLibraryExW(
            wide_path.as_ptr(),
            core::ptr::null_mut(),
            LOAD_WITH_ALTERED_SEARCH_PATH,
        )
    };
    if hinstance.is_null() {
        let err = unsafe { GetLastError() };
        return Err(PluginLoadError::OsLoadFailed(err));
    }

    // 3. Resolve entry points: check Input plugin first, then General plugin (S09 §2).
    let in_export = unsafe {
        GetProcAddress(
            hinstance,
            rustamp_plugin_abi::common::EXPORT_IN_GET_MODULE.as_ptr(),
        )
    };
    if !in_export.is_null() {
        // SAFETY: in_export was resolved from the mapped module.
        return unsafe { load_input_module(clean_path, hinstance, in_export, host_hwnd) };
    }

    let gen_export = unsafe {
        GetProcAddress(
            hinstance,
            rustamp_plugin_abi::common::EXPORT_GEN_GET_PLUGIN.as_ptr(),
        )
    };
    if !gen_export.is_null() {
        // SAFETY: gen_export was resolved from the mapped module.
        return unsafe { load_general_module(clean_path, hinstance, gen_export, host_hwnd) };
    }

    Err(PluginLoadError::MissingExport(
        "winampGetInModule2 or winampGetGeneralPurposePlugin",
    ))
}

#[cfg(all(windows, target_arch = "x86"))]
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

#[cfg(all(windows, target_arch = "x86"))]
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

    #[test]
    fn preflight_rejection_prevents_os_loading() {
        let bad_bytes = b"not a pe file at all";
        let temp_path = std::env::temp_dir().join("test_bad_preflight_pe.dll");
        std::fs::write(&temp_path, bad_bytes).unwrap();

        let window = HostWindow::dummy();

        // SAFETY: temp file is plain test data.
        let result = unsafe { load_plugin(&temp_path, &window) };
        let _ = std::fs::remove_file(&temp_path);
        assert!(matches!(result, Err(PluginLoadError::Preflight(_))));
    }

    #[test]
    fn missing_file_returns_io_error() {
        let non_existent = Path::new("does_not_exist_in_rustamp_plugin_loader.dll");
        let window = HostWindow::dummy();

        // SAFETY: non-existent path.
        let result = unsafe { load_plugin(non_existent, &window) };
        assert!(matches!(result, Err(PluginLoadError::Io(_))));
    }
}
