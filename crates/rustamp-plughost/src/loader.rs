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
//!    for input plugins, `winampGetGeneralPurposePlugin` for general plugins; S09 §2).
//! 5. **Calling convention**: entry points and callbacks are `cdecl` (`extern "C"`,
//!    caller cleans stack; S09 §1 line 38).
//! 6. **Module validation**: verifies that the returned module pointer is non-null,
//!    validates the version field (`0x100` for input plugins per S09 §3.2 line 125,
//!    masking 5.x flags `0x0F00_0000` per S09 §3.2 line 127 and §11 line 437; `0x10` for
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
#[cfg(windows)]
use rustamp_plugin_abi::common::{
    GEN_VERSION, IN_VERSION, IN_VERSION_FLAG_INIT_STATUS, IN_VERSION_FLAG_WIDE_FILENAMES,
    read_c_string_lossy,
};
#[cfg(windows)]
use rustamp_plugin_abi::general::GeneralPlugin;
#[cfg(windows)]
use rustamp_plugin_abi::input::InModule;

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
/// Host callback: `dsp_is_active()` -> 0 (no DSP active; S09 §3.1 offset 0x84 line 111).
unsafe extern "C" fn host_dsp_is_active() -> core::ffi::c_int {
    0
}

#[cfg(windows)]
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
    #[cfg(windows)]
    Input { module: *mut InModule },
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
        PluginKind::General => (
            rustamp_plugin_abi::common::EXPORT_GEN_GET_PLUGIN,
            "winampGetGeneralPurposePlugin",
        ),
        _ => unreachable!("preflight selects input/general only"),
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
        PluginKind::General => unsafe {
            load_general_module(image.path, hinstance, export, host_hwnd)
        },
        _ => unreachable!("preflight selects input/general only"),
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
}
