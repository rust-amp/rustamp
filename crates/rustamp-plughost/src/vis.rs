//! The visualization (`vis_`) plugin bridge (S09 section 5).
//!
//! A classic visualization plugin is a 32-bit DLL that exports
//! `winampVisGetHeader` and hands out one or more `winampVisModule`s
//! (S09 section 2, section 5.1). The host fills four members of the chosen
//! module — the parent window, the plugin's own module handle, the stream
//! sample rate and the channel count — calls `init`, and then draws frames by
//! calling `render` (S09 section 5.2, section 5.3).
//!
//! # What this module does
//!
//! * validate the header (version `0x101`, S09 section 5.1) and enumerate the
//!   library's modules through `get_module`, which answers null past the last
//!   one (S09 section 5.1);
//! * wire the host members into a module and run its `init` (S09 section 5.3
//!   step 2);
//! * draw one frame: refresh the data arrays, call `render`, report the
//!   plugin's own return value (S09 section 5.2, section 5.3 step 4b);
//! * stop the running visualization with `quit` (S09 section 5.3 step 5);
//! * answer the input plugin's `vis_get_mode` and `vis_set_format` host
//!   callbacks for the running visualization (S09 section 3.1 offset 0x78,
//!   section 3.8).
//!
//! # What this module does not do
//!
//! * **It does not feed the data arrays.** The host copies the spectrum and
//!   waveform rows of the frame nearest to (playback position + `latency_ms`)
//!   into the module before every `render` (S09 section 5.3 step 4b). That
//!   frame comes from the input plugin's decode thread and from the analyzer,
//!   which is the audio path. Until that path exists, every frame is the
//!   zeroed one S09 section 5.3 prescribes for a visualization that has no
//!   frame.
//! * **It does not run the vis thread.** S09 section 5.3 runs the loop on a
//!   dedicated thread that also pumps the plugin's window messages. The host
//!   process owns that thread; this module owns the per-frame step.
//! * **It does not free the DLL.** S09 section 5.3 step 5 frees the
//!   visualization DLL after `quit` in the 5.x host; this host keeps the
//!   conservative policy of S09 section 1 line 46 and retains the mapping.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::ffi::{c_int, c_void};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

use rustamp_plugin_abi::PluginKind;
use rustamp_plugin_abi::common::{
    VIS_DEFAULT_CHANNELS, VIS_DEFAULT_SAMPLE_RATE, VIS_MAX_ROWS, VIS_ROW_LEN, VIS_VERSION,
    read_c_string_lossy,
};
use rustamp_plugin_abi::vis::{VisHeader, VisModule};

use crate::loader::PluginLoadError;

/// Upper bound on the number of modules the host asks a library for.
///
/// S09 section 5.1 says the host calls `get_module(0)`, `get_module(1)`, ...
/// until the answer is null; it does not bound the list. A plugin that never
/// answers null must not hang the host, so the enumeration stops here. Own
/// policy, not S09.
pub const VIS_MAX_MODULES: usize = 64;

/// Host-visible state of one module in a visualization library's module list
/// (S09 section 5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisModuleInfo {
    /// The 0-based index the library hands this module out under.
    pub index: i32,
    /// The module's own name (S09 section 5.2, offset 0x000).
    pub description: String,
    /// How far ahead of "now" the module wants its data (S09 section 5.2,
    /// offset 0x014).
    pub latency_ms: i32,
    /// The pause the host inserts after each `render` (S09 section 5.2,
    /// offset 0x018).
    pub delay_ms: i32,
    /// Spectrum rows the module asks for, 0..=2 (S09 section 5.2, offset
    /// 0x01C).
    pub spectrum_rows: i32,
    /// Waveform rows the module asks for, 0..=2 (S09 section 5.2, offset
    /// 0x020).
    pub waveform_rows: i32,
    /// The module supplied a `config` callback (S09 section 5.2, offset 0x924).
    pub has_config: bool,
    /// The module supplied an `init` callback (S09 section 5.2, offset 0x928).
    pub has_init: bool,
    /// The module supplied a `render` callback (S09 section 5.2, offset 0x92C).
    pub has_render: bool,
    /// The module supplied a `quit` callback (S09 section 5.2, offset 0x930).
    pub has_quit: bool,
}

/// The library-level state of a loaded visualization plugin (S09 section 5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisLibraryInfo {
    /// The header version the plugin reported (S09 section 5.1, offset 0x00).
    pub version: i32,
    /// The library's own name (S09 section 5.1, offset 0x04).
    pub description: String,
    /// The modules the library hands out, in index order.
    pub modules: Vec<VisModuleInfo>,
}

/// Host-visible state of the running visualization module (S09 section 5.2).
///
/// This is what an acceptance runner checks **before** it calls anything: the
/// members the host filled in and what the plugin's `init` returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisRunInfo {
    /// The index of the running module in the library's module list.
    pub index: i32,
    /// `parent_window` as filled by the host before `init` (S09 section 5.2,
    /// offset 0x004).
    pub parent_window: usize,
    /// `dll_instance` as filled by the host before `init` (S09 section 5.2,
    /// offset 0x008).
    pub dll_instance: usize,
    /// `sample_rate` as filled by the host before `init` (S09 section 5.2,
    /// offset 0x00C).
    pub sample_rate: i32,
    /// `channels` as filled by the host before `init` (S09 section 5.2,
    /// offset 0x010).
    pub channels: i32,
    /// The module's own latency (S09 section 5.2, offset 0x014).
    pub latency_ms: i32,
    /// The module's own delay (S09 section 5.2, offset 0x018).
    pub delay_ms: i32,
    /// `init` was actually called. False means the slot was null and skipped.
    pub init_called: bool,
    /// The raw value `init` returned.
    pub init_result: c_int,
}

/// One module of a visualization library: the plugin's own struct and the
/// state the host read out of it.
#[derive(Debug)]
pub(crate) struct VisModuleEntry {
    /// The plugin's static module struct. Its address is stable while the DLL
    /// is mapped (S09 section 0).
    pub(crate) module: *mut VisModule,
    /// What the host read out of that struct.
    pub(crate) info: VisModuleInfo,
}

/// Why a visualization could not be started or driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisError {
    /// The plugin is not a visualization plugin, so the operation is not
    /// routed for it.
    WrongKind(PluginKind),
    /// The library has no module under this index.
    UnknownModule(i32),
    /// The module's `init` refused: a nonzero return aborts the start
    /// (S09 section 5.2, offset 0x928).
    InitFailed(i32),
    /// No visualization module is running, so there is nothing to draw.
    NotRunning,
    /// The running module supplied no `render` callback (S09 section 5.2,
    /// offset 0x92C).
    MissingRender,
}

impl fmt::Display for VisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongKind(kind) => write!(
                f,
                "visualization is not routed for a {kind:?} plugin; nothing was called"
            ),
            Self::UnknownModule(index) => {
                write!(
                    f,
                    "the visualization library has no module with index {index}"
                )
            }
            Self::InitFailed(code) => write!(
                f,
                "the visualization module refused to start with code {code}; nothing was activated"
            ),
            Self::NotRunning => f.write_str("no visualization module is running"),
            Self::MissingRender => f.write_str(
                "the visualization module supplied no render callback; nothing was called",
            ),
        }
    }
}

impl std::error::Error for VisError {}

/// What happened while a visualization module was initialized.
#[derive(Debug)]
pub(crate) struct VisInit {
    /// The module the library handed out.
    pub(crate) module: *mut VisModule,
    /// Whether an `init` slot was present and called.
    pub(crate) init_called: bool,
    /// The raw value `init` returned. A nonzero value is an error
    /// ([`VisError::InitFailed`]), so a successful start always reports 0.
    pub(crate) init_result: c_int,
}

/// Validate a visualization header and enumerate the library's modules
/// (S09 section 5.1, section 5.3 step 2).
///
/// The header version must be `0x101` (S09 section 5.1): `0x102` belongs to
/// the 5.x-only main-window-taking entry point and is refused, like every
/// other 5.x-only version in this host. The modules are read by asking
/// `get_module(0)`, `get_module(1)`, ... until the answer is null.
///
/// # Safety
///
/// `header` must be null or point at a live `winampVisHeader` in the mapped
/// plugin's static data (S09 section 0). The plugin's `get_module` runs here
/// on the caller's thread.
pub(crate) unsafe fn list_vis_modules(
    header: *mut VisHeader,
) -> Result<Vec<VisModuleEntry>, PluginLoadError> {
    if header.is_null() {
        return Err(PluginLoadError::NullModule);
    }
    // SAFETY: caller guarantees a live header for this read.
    let version = unsafe { (*header).version };
    // S09 section 5.1: 0x101 in 2.x. 0x102 is 5.x-only and refused (U-7
    // decision: strict equality, the general, output and DSP policy).
    if version != VIS_VERSION {
        return Err(PluginLoadError::UnsupportedVersion {
            kind: PluginKind::Visualization,
            actual: version,
            expected: VIS_VERSION,
        });
    }
    // SAFETY: same live header.
    let get_module = unsafe { (*header).get_module };
    let Some(get_module) = get_module else {
        return Err(PluginLoadError::MissingCallback(
            "winampVisHeader.get_module",
        ));
    };
    let mut modules = Vec::new();
    for index in 0..VIS_MAX_MODULES as i32 {
        // SAFETY: cdecl getter from the mapped image (or the test double).
        let module = unsafe { get_module(index) };
        if module.is_null() {
            break;
        }
        modules.push(VisModuleEntry {
            module,
            // SAFETY: the module came from the plugin's own getter and is
            // live in the mapped image's static data.
            info: unsafe { read_vis_module_info(module, index) },
        });
    }
    Ok(modules)
}

/// Read the host-visible state out of a visualization module (S09 section 5.2).
///
/// # Safety
///
/// `module` must point at a live `winampVisModule` in plugin static data.
unsafe fn read_vis_module_info(module: *mut VisModule, index: i32) -> VisModuleInfo {
    // SAFETY: the caller guarantees a live module for these reads.
    let m = unsafe { *module };
    VisModuleInfo {
        index,
        // SAFETY: a null description pointer reads as empty and is not
        // dereferenced (S09 section 1).
        description: unsafe { read_c_string_lossy(m.description) },
        latency_ms: m.latency_ms,
        delay_ms: m.delay_ms,
        spectrum_rows: m.spectrum_rows,
        waveform_rows: m.waveform_rows,
        has_config: m.config.is_some(),
        has_init: m.init.is_some(),
        has_render: m.render.is_some(),
        has_quit: m.quit.is_some(),
    }
}

/// Fill the host members of a visualization module and run its `init`
/// (S09 section 5.2, section 5.3 step 2).
///
/// The host writes the parent window, the plugin's own module handle, the
/// stream sample rate and the channel count before `init` runs (S09 section
/// 5.3 step 2). The rate and the channel count are the defaults S09 section
/// 5.2 gives (44100 Hz and 2 channels); the input plugin replaces them with
/// the values it announces through `vis_set_format` (S09 section 3.8).
///
/// # Safety
///
/// `module` must point at a live `winampVisModule` in the mapped plugin's
/// static data; the plugin's `init` runs here on the caller's thread, which
/// must be the thread that owns the host window.
pub(crate) unsafe fn init_vis_module(
    module: *mut VisModule,
    host_hwnd: usize,
    hinstance: *mut c_void,
) -> Result<VisInit, VisError> {
    if module.is_null() {
        return Err(VisError::NotRunning);
    }
    // SAFETY: live module in plugin static data.
    unsafe {
        (*module).parent_window = host_hwnd as *mut c_void;
        (*module).dll_instance = hinstance;
        (*module).sample_rate = VIS_DEFAULT_SAMPLE_RATE;
        (*module).channels = VIS_DEFAULT_CHANNELS;
    }
    let mut init_called = false;
    let mut init_result = 0;
    // SAFETY: cdecl, one argument (S09 section 5.2, offset 0x928).
    if let Some(init) = unsafe { (*module).init } {
        init_called = true;
        init_result = unsafe { init(module) };
        if init_result != 0 {
            // S09 section 5.2: a nonzero return aborts the start. The caller
            // must not activate the module.
            return Err(VisError::InitFailed(init_result));
        }
    }
    Ok(VisInit {
        module,
        init_called,
        init_result,
    })
}

/// The visualization module that is running, if any.
///
/// S09 section 5.3: one visualization is active at a time, and the host's
/// `vis_get_mode` and `vis_set_format` callbacks answer for the running one
/// (S09 section 3.1 offset 0x78, section 3.8). The pointer lives in an
/// atomic because the input plugin's decode thread reads it (S09 section 3.6)
/// while the host's own thread starts and stops the visualization.
static ACTIVE_VIS_MODULE: AtomicUsize = AtomicUsize::new(0);

/// The visualization module that is running, or null.
pub(crate) fn active_vis_module() -> *mut VisModule {
    ACTIVE_VIS_MODULE.load(Ordering::SeqCst) as *mut VisModule
}

/// Make `module` the running visualization. A null pointer clears the slot.
pub(crate) fn set_active_vis_module(module: *mut VisModule) {
    ACTIVE_VIS_MODULE.store(module as usize, Ordering::SeqCst);
}

/// Clear the running visualization, but only if the slot still holds
/// `module`: a replaced or quitting plugin must not deactivate its successor.
pub(crate) fn clear_active_vis_module(module: *mut VisModule) {
    let _ =
        ACTIVE_VIS_MODULE.compare_exchange(module as usize, 0, Ordering::SeqCst, Ordering::SeqCst);
}

/// Stop the visualization that is running, if any (S09 section 5.3 step 1 and
/// step 5).
///
/// The running module's `quit` is called and the slot is cleared. The DLL
/// stays mapped: the 5.x host frees the visualization DLL after `quit`
/// (S09 section 5.3 step 5), this host keeps the conservative policy of
/// S09 section 1 line 46.
pub(crate) fn stop_active_vis_module() {
    let module = active_vis_module();
    if module.is_null() {
        return;
    }
    // SAFETY: the slot holds a live module in plugin static data.
    if let Some(quit) = unsafe { (*module).quit } {
        // SAFETY: cdecl, one argument (S09 section 5.2, offset 0x930).
        unsafe { quit(module) };
    }
    clear_active_vis_module(module);
}

/// Zero the module's data arrays (S09 section 5.3 step 4b).
///
/// The host copies the frame's spectrum and waveform rows into these arrays
/// before every `render`. With no frame available — the case until the audio
/// path feeds the host — both arrays are zeroed completely, which is what
/// S09 section 5.3 prescribes for a visualization that has no frame.
///
/// # Safety
///
/// `module` must point at a live `winampVisModule` in plugin static data.
pub(crate) unsafe fn zero_vis_data(module: *mut VisModule) {
    // SAFETY: the caller guarantees a live module; both arrays are inline.
    unsafe {
        (*module).spectrum_data = [[0; VIS_ROW_LEN]; VIS_MAX_ROWS];
        (*module).waveform_data = [[0; VIS_ROW_LEN]; VIS_MAX_ROWS];
    }
}

/// Draw one frame (S09 section 5.3 step 4b).
///
/// The data arrays are refreshed first — here: zeroed, because no frame is
/// available — and then the plugin's `render` is called. Its return value is
/// the plugin's own: 0 continues the visualization, a nonzero value ends it
/// and the host then calls `quit` (S09 section 5.2, offset 0x92C).
///
/// # Safety
///
/// `module` must point at a live `winampVisModule` in plugin static data; the
/// plugin's `render` runs here on the caller's thread.
pub(crate) unsafe fn render_vis_frame(module: *mut VisModule) -> Result<c_int, VisError> {
    if module.is_null() {
        return Err(VisError::NotRunning);
    }
    // SAFETY: live module in plugin static data.
    let Some(render) = (unsafe { (*module).render }) else {
        return Err(VisError::MissingRender);
    };
    // SAFETY: same live module.
    unsafe { zero_vis_data(module) };
    // SAFETY: cdecl, one argument (S09 section 5.2, offset 0x92C).
    Ok(unsafe { render(module) })
}

/// Answer the input plugin's `vis_get_mode` for the running visualization
/// (S09 section 3.1, offset 0x78).
///
/// The answer is nonzero while a visualization is running, and the row counts
/// are the ones the running module asked for. With no visualization running
/// both counts are zero and the answer is 0.
pub(crate) fn vis_get_mode(spectrum_rows: *mut c_int, waveform_rows: *mut c_int) -> c_int {
    let module = active_vis_module();
    if module.is_null() {
        write_row_counts(spectrum_rows, waveform_rows, 0, 0);
        return 0;
    }
    // SAFETY: the slot holds a live module in plugin static data.
    let (spectrum, waveform) = unsafe { ((*module).spectrum_rows, (*module).waveform_rows) };
    write_row_counts(spectrum_rows, waveform_rows, spectrum, waveform);
    1
}

/// Write the row counts into the caller's out-parameters, if they are there.
fn write_row_counts(
    spectrum_rows: *mut c_int,
    waveform_rows: *mut c_int,
    spectrum: c_int,
    waveform: c_int,
) {
    if !spectrum_rows.is_null() {
        // SAFETY: the caller guarantees a writable int pointer.
        unsafe { *spectrum_rows = spectrum };
    }
    if !waveform_rows.is_null() {
        // SAFETY: the caller guarantees a writable int pointer.
        unsafe { *waveform_rows = waveform };
    }
}

/// Copy the announced stream format into the running visualization module
/// (S09 section 3.8).
///
/// `vis_set_format` is called by the input plugin when a stream starts
/// (S09 section 3.5 step 3); the host copies the sample rate and the channel
/// count into the running module's own members. With no visualization running
/// there is no module to copy them into, and nothing happens.
pub(crate) fn vis_set_format(sample_rate: c_int, channels: c_int) {
    let module = active_vis_module();
    if module.is_null() {
        return;
    }
    // SAFETY: the slot holds a live module in plugin static data.
    unsafe {
        (*module).sample_rate = sample_rate;
        (*module).channels = channels;
    }
}

/// Serializes the tests that touch the process-wide running-visualization
/// slot, exactly as the DSP bridge's slot lock does.
///
/// The slot is process-global, so every test that starts, stops or inspects
/// the running visualization takes this lock — including the ones in other
/// modules that drive a visualization through [`crate::loader::LoadedPlugin`].
#[cfg(test)]
pub(crate) static VIS_SLOT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::{LoadedPlugin, PluginLoadError};
    use std::cell::{Cell, RefCell};

    thread_local! {
        /// The modules the synthetic getter hands out, in index order.
        static VIS_MODULES: RefCell<Vec<*mut VisModule>> = const { RefCell::new(Vec::new()) };
        /// How often the synthetic getter was called.
        static VIS_GETTER_CALLS: Cell<i32> = const { Cell::new(0) };
    }

    /// A getter that answers the modules of [`VIS_MODULES`] and null past the
    /// last one (S09 §5.1).
    unsafe extern "C" fn test_get_module(index: c_int) -> *mut VisModule {
        VIS_GETTER_CALLS.with(|calls| calls.set(calls.get() + 1));
        VIS_MODULES.with(|modules| {
            let modules = modules.borrow();
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            modules.get(index).copied().unwrap_or(core::ptr::null_mut())
        })
    }

    /// A getter that never answers null: the enumeration must still stop.
    unsafe extern "C" fn test_get_module_forever(_index: c_int) -> *mut VisModule {
        VIS_GETTER_CALLS.with(|calls| calls.set(calls.get() + 1));
        VIS_MODULES.with(|modules| {
            let mut modules = modules.borrow_mut();
            if modules.is_empty() {
                // A stable, non-null module the getter always answers with.
                let module: &'static mut VisModule = Box::leak(Box::new(test_module(c"endless")));
                modules.push(module as *mut VisModule);
            }
            modules[0]
        })
    }

    /// A synthetic module with the fields a test needs.
    fn test_module(description: &'static core::ffi::CStr) -> VisModule {
        VisModule {
            description: description.as_ptr(),
            latency_ms: 14,
            delay_ms: 14,
            spectrum_rows: 2,
            waveform_rows: 2,
            init: Some(test_init),
            render: Some(test_render),
            quit: Some(test_quit),
            ..VisModule::default()
        }
    }

    thread_local! {
        /// What the synthetic callbacks recorded, in call order.
        static VIS_LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn vis_note(entry: impl Into<String>) {
        VIS_LOG.with(|log| log.borrow_mut().push(entry.into()));
    }

    fn vis_log() -> Vec<String> {
        VIS_LOG.with(|log| log.borrow().clone())
    }

    fn vis_log_clear() {
        VIS_LOG.with(|log| log.borrow_mut().clear());
        VIS_MODULES.with(|modules| modules.borrow_mut().clear());
        VIS_GETTER_CALLS.with(|calls| calls.set(0));
    }

    /// `init` reads the members the host must have filled before it runs.
    unsafe extern "C" fn test_init(module: *mut VisModule) -> c_int {
        if module.is_null() {
            vis_note("init with no module");
            return -1;
        }
        // SAFETY: the test owns the module and keeps it alive across the call.
        let (parent, instance, rate, channels) = unsafe {
            (
                (*module).parent_window as usize,
                (*module).dll_instance as usize,
                (*module).sample_rate,
                (*module).channels,
            )
        };
        vis_note(format!(
            "init parent={parent:#x} instance={instance:#x} rate={rate} channels={channels}"
        ));
        0
    }

    /// `init` of the module that refuses to start.
    unsafe extern "C" fn test_init_refusing(_module: *mut VisModule) -> c_int {
        vis_note("init refused");
        1
    }

    /// `render` reads the first bytes of both data arrays.
    unsafe extern "C" fn test_render(module: *mut VisModule) -> c_int {
        if module.is_null() {
            return -1;
        }
        // SAFETY: the test owns the module and keeps it alive across the call.
        let (spectrum, waveform) =
            unsafe { ((*module).spectrum_data[0][0], (*module).waveform_data[0][0]) };
        vis_note(format!("render spectrum={spectrum} waveform={waveform}"));
        0
    }

    /// `render` of the module that ends the visualization.
    unsafe extern "C" fn test_render_ending(_module: *mut VisModule) -> c_int {
        vis_note("render ending");
        5
    }

    /// `quit` records the teardown.
    unsafe extern "C" fn test_quit(_module: *mut VisModule) {
        vis_note("quit");
    }

    /// `config` records the call. A visualization config opens a dialog, so
    /// the host calls it on the UI thread (S09 §5.3 step 7).
    unsafe extern "C" fn test_config(_module: *mut VisModule) {
        vis_note("config");
    }

    /// A header whose getter hands out `modules`.
    fn test_header(modules: Vec<*mut VisModule>) -> VisHeader {
        VIS_MODULES.with(|slot| *slot.borrow_mut() = modules);
        VisHeader {
            version: VIS_VERSION,
            description: c"synthetic vis".as_ptr(),
            get_module: Some(test_get_module),
        }
    }

    #[test]
    fn the_module_list_is_read_until_the_getter_answers_null() {
        vis_log_clear();
        let mut first = test_module(c"first");
        let mut second = test_module(c"second");
        let mut header = test_header(vec![&mut first, &mut second]);
        // SAFETY: live stack header and modules; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        assert_eq!(modules.len(), 2, "the list stops at the first null answer");
        assert_eq!(modules[0].info.index, 0);
        assert_eq!(modules[0].info.description, "first");
        assert_eq!(modules[1].info.index, 1);
        assert_eq!(modules[1].info.description, "second");
        // S09 §5.2: the host reads the plugin's own row counts and cadence.
        assert_eq!(modules[0].info.latency_ms, 14);
        assert_eq!(modules[0].info.delay_ms, 14);
        assert_eq!(modules[0].info.spectrum_rows, 2);
        assert_eq!(modules[0].info.waveform_rows, 2);
        assert!(modules[0].info.has_init);
        assert!(modules[0].info.has_render);
        assert!(modules[0].info.has_quit);
        assert!(!modules[0].info.has_config);
        assert_eq!(
            VIS_GETTER_CALLS.with(|calls| calls.get()),
            3,
            "the getter is asked for the module after the last one, to see the null"
        );
    }

    #[test]
    fn a_wrong_header_version_is_refused_before_any_module_is_read() {
        vis_log_clear();
        let mut module = test_module(c"first");
        let mut header = test_header(vec![&mut module]);
        // 0x102 is the 5.x-only main-window-taking entry point (S09 §5.1).
        header.version = rustamp_plugin_abi::common::VIS_VERSION_5X;
        // SAFETY: live stack header; no DLL is involved.
        let error = unsafe { list_vis_modules(&mut header) }.expect_err("0x102 must be refused");
        assert!(
            matches!(
                error,
                PluginLoadError::UnsupportedVersion {
                    kind: PluginKind::Visualization,
                    actual: 0x102,
                    expected: VIS_VERSION,
                }
            ),
            "{error:?}"
        );
        assert_eq!(
            VIS_GETTER_CALLS.with(|calls| calls.get()),
            0,
            "no module may be read before the version check"
        );
    }

    #[test]
    fn a_null_header_and_a_null_getter_are_refused() {
        vis_log_clear();
        // SAFETY: a null header is exactly the input under test.
        let error = unsafe { list_vis_modules(core::ptr::null_mut()) }
            .expect_err("a null header must be refused");
        assert!(matches!(error, PluginLoadError::NullModule), "{error:?}");

        let mut header = VisHeader {
            version: VIS_VERSION,
            description: c"no getter".as_ptr(),
            get_module: None,
        };
        // SAFETY: live stack header; no DLL is involved.
        let error =
            unsafe { list_vis_modules(&mut header) }.expect_err("a null getter must be refused");
        assert!(
            matches!(error, PluginLoadError::MissingCallback(_)),
            "{error:?}"
        );
    }

    #[test]
    fn the_enumeration_stops_at_the_own_bound() {
        vis_log_clear();
        let mut header = VisHeader {
            version: VIS_VERSION,
            description: c"endless".as_ptr(),
            get_module: Some(test_get_module_forever),
        };
        // The synthetic getter answers with a module for every index, so the
        // list never ends by itself. The bound is what keeps such a plugin
        // from hanging the host (own policy, documented in VIS_MAX_MODULES).
        // SAFETY: live stack header; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        assert_eq!(modules.len(), VIS_MAX_MODULES);
        assert_eq!(
            VIS_GETTER_CALLS.with(|calls| calls.get()),
            VIS_MAX_MODULES as i32,
            "the getter must not be asked past the bound"
        );
        assert_eq!(VIS_MAX_MODULES, 64);
    }

    #[test]
    fn init_fills_the_host_members_before_it_runs() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        // SAFETY: live stack module; no DLL is involved.
        let init = unsafe { init_vis_module(&mut module, 0xBEEF, 0xCAFE as *mut c_void) }
            .expect("a module with an init slot must start");
        assert!(init.init_called);
        assert_eq!(init.init_result, 0);
        // S09 §5.3 step 2: the host members are filled before `init` runs.
        assert_eq!(
            vis_log(),
            vec!["init parent=0xbeef instance=0xcafe rate=44100 channels=2"]
        );
    }

    #[test]
    fn a_refusing_init_is_not_activated() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"refusing");
        module.init = Some(test_init_refusing);
        // SAFETY: live stack module; no DLL is involved.
        let error = unsafe { init_vis_module(&mut module, 0x1, 0x2 as *mut c_void) }
            .expect_err("a refusing init must be loud");
        assert_eq!(error, VisError::InitFailed(1));
        assert_eq!(vis_log(), vec!["init refused"]);
    }

    #[test]
    fn a_render_frame_zeroes_the_arrays_and_reports_the_plugins_own_code() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        // SAFETY: the test owns the module and keeps it alive across the call.
        unsafe { zero_vis_data(&mut module) };
        module.spectrum_data[0][0] = 200;
        module.waveform_data[1][575] = 200;
        // SAFETY: live stack module; no DLL is involved.
        let code =
            unsafe { render_vis_frame(&mut module) }.expect("a module with render must draw");
        assert_eq!(code, 0, "0 means the visualization continues (S09 §5.2)");
        assert_eq!(
            vis_log(),
            vec!["render spectrum=0 waveform=0"],
            "the arrays are zeroed completely before the frame is drawn"
        );

        let mut ending = test_module(c"ending");
        ending.render = Some(test_render_ending);
        // SAFETY: live stack module; no DLL is involved.
        let code =
            unsafe { render_vis_frame(&mut ending) }.expect("a module with render must draw");
        assert_eq!(
            code, 5,
            "the plugin's own return value is reported unchanged"
        );
    }

    #[test]
    fn a_module_without_render_is_refused_not_called() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"silent");
        module.render = None;
        // SAFETY: live stack module; no DLL is involved.
        let error = unsafe { render_vis_frame(&mut module) }.expect_err("a null render must fail");
        assert_eq!(error, VisError::MissingRender);
        assert!(vis_log().is_empty());
    }

    #[test]
    fn the_running_module_answers_the_host_callbacks() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        // Nothing is running yet.
        let mut spectrum = 7;
        let mut waveform = 7;
        assert_eq!(vis_get_mode(&mut spectrum, &mut waveform), 0);
        assert_eq!(
            (spectrum, waveform),
            (0, 0),
            "no rows without a running module"
        );

        // S09 §3.8: the announced format is copied into the running module.
        set_active_vis_module(&mut module);
        vis_set_format(48_000, 1);
        assert_eq!(module.sample_rate, 48_000);
        assert_eq!(module.channels, 1);

        // S09 §3.1 offset 0x78: the running module's row counts are reported.
        assert_eq!(vis_get_mode(&mut spectrum, &mut waveform), 1);
        assert_eq!((spectrum, waveform), (2, 2));
        // Null out-parameters are allowed.
        assert_eq!(
            vis_get_mode(core::ptr::null_mut(), core::ptr::null_mut()),
            1
        );

        stop_active_vis_module();
        assert_eq!(vis_log(), vec!["quit"], "stopping runs the module's quit");
        assert!(active_vis_module().is_null());
        assert_eq!(vis_get_mode(&mut spectrum, &mut waveform), 0);
    }

    #[test]
    fn only_one_visualization_runs_at_a_time() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut first = test_module(c"first");
        let mut second = test_module(c"second");
        let mut header = test_header(vec![&mut first, &mut second]);
        // SAFETY: live stack header and modules; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::vis_for_test("vis_synthetic", &mut header, modules, 0x99);

        plugin.vis_start(0).expect("the first module must start");
        assert!(plugin.is_initialized());
        assert_eq!(vis_log().len(), 1, "only the first init ran so far");

        // S09 §5.3 step 1: the running visualization is stopped first.
        plugin.vis_start(1).expect("the second module must start");
        assert_eq!(
            vis_log(),
            vec![
                "init parent=0x99 instance=0x0 rate=44100 channels=2",
                "quit",
                "init parent=0x99 instance=0x0 rate=44100 channels=2",
            ]
        );
        assert!(plugin.is_initialized());
    }

    #[test]
    fn the_plugin_lifecycle_runs_init_render_and_quit_once() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        let mut header = test_header(vec![&mut module]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::vis_for_test("vis_synthetic", &mut header, modules, 0x99);

        // Nothing runs before the user picks a module.
        assert!(!plugin.is_initialized());
        assert_eq!(plugin.vis_render(), Err(VisError::NotRunning));
        assert!(vis_log().is_empty());

        plugin.vis_start(0).expect("the module must start");
        assert!(plugin.is_initialized());
        let run = plugin
            .vis_run_info()
            .expect("a running visualization reports its state");
        assert_eq!(run.index, 0);
        assert_eq!(run.parent_window, 0x99, "the host filled the parent window");
        assert_eq!(run.sample_rate, 44_100);
        assert_eq!(run.channels, 2);
        assert_eq!(run.latency_ms, 14);
        assert_eq!(run.delay_ms, 14);
        assert!(run.init_called);
        assert_eq!(run.init_result, 0);
        assert_eq!(plugin.vis_render().expect("a frame must be drawn"), 0);
        assert_eq!(plugin.vis_render().expect("a frame must be drawn"), 0);
        assert_eq!(
            vis_log(),
            vec![
                "init parent=0x99 instance=0x0 rate=44100 channels=2",
                "render spectrum=0 waveform=0",
                "render spectrum=0 waveform=0",
            ]
        );

        plugin.quit();
        assert!(!plugin.is_initialized());
        assert!(
            plugin.vis_run_info().is_none(),
            "no module runs after the quit"
        );
        assert_eq!(plugin.vis_render(), Err(VisError::NotRunning));
        assert_eq!(
            vis_log().last(),
            Some(&"quit".to_string()),
            "quit must reach the plugin"
        );
        // A second quit must not call the plugin again.
        let before = vis_log().len();
        plugin.quit();
        assert_eq!(vis_log().len(), before);
    }

    #[test]
    fn the_config_callback_is_routed_to_the_running_module() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        module.config = Some(test_config);
        let mut header = test_header(vec![&mut module]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::vis_for_test("vis_synthetic", &mut header, modules, 0x99);

        // Nothing runs before the user picks a module, so there is no live
        // module to configure (S09 §5.3 step 2).
        assert_eq!(
            plugin.config(),
            Err(crate::loader::PluginConfigError::NotInitialized)
        );
        assert!(
            vis_log().is_empty(),
            "nothing may be called before the start"
        );

        plugin.vis_start(0).expect("the module must start");
        assert_eq!(plugin.config(), Ok(()));
        assert_eq!(
            vis_log(),
            vec![
                "init parent=0x99 instance=0x0 rate=44100 channels=2",
                "config"
            ],
            "the config callback must run exactly once, on the running module"
        );

        // A module without a `config` slot is refused, never called.
        vis_log_clear();
        let mut bare = test_module(c"bare");
        let mut header = test_header(vec![&mut bare]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::vis_for_test("vis_bare", &mut header, modules, 0x99);
        plugin.vis_start(0).expect("the module must start");
        assert_eq!(
            plugin.config(),
            Err(crate::loader::PluginConfigError::MissingCallback)
        );
        assert!(
            !vis_log().iter().any(|entry| entry == "config"),
            "a null config slot must never be called"
        );

        // A module that is not running any more is not live: its dialog must
        // not be opened (S09 §5.3 step 1).
        vis_log_clear();
        let mut ended = test_module(c"ended");
        ended.config = Some(test_config);
        let mut header = test_header(vec![&mut ended]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::vis_for_test("vis_ended", &mut header, modules, 0x99);
        plugin.vis_start(0).expect("the module must start");
        plugin.quit();
        assert_eq!(
            plugin.config(),
            Err(crate::loader::PluginConfigError::NotInitialized)
        );
        assert!(
            !vis_log().iter().any(|entry| entry == "config"),
            "a quit module must not be configured"
        );
    }

    #[test]
    fn an_unknown_module_index_and_a_wrong_kind_are_refused() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        let mut header = test_header(vec![&mut module]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::vis_for_test("vis_synthetic", &mut header, modules, 0x99);
        assert_eq!(plugin.vis_start(7), Err(VisError::UnknownModule(7)));
        assert!(vis_log().is_empty(), "no module may be started");

        // A plugin of another kind has no visualization bridge.
        let mut other = LoadedPlugin::unsupported_for_test(PluginKind::Output);
        assert_eq!(
            other.vis_start(0),
            Err(VisError::WrongKind(PluginKind::Output))
        );
        assert_eq!(
            other.vis_render(),
            Err(VisError::WrongKind(PluginKind::Output))
        );
        assert!(other.vis_info().is_none());
    }
}
