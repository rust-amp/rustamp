//! The DSP (`dsp_`) plugin bridge (S09 section 6).
//!
//! A classic DSP plugin is a 32-bit DLL that exports `winampDSPGetHeader2`
//! and hands out one or more `winampDSPModule`s (S09 section 2, section 6.1).
//! The host fills two members of the chosen module — the parent window and
//! the plugin's own module handle — calls `init`, and afterwards runs audio
//! through the module's `process` callback, which works in place and returns
//! the frame count now in the buffer (S09 section 6.2, section 6.3).
//!
//! # What this module does
//!
//! * validate the header (version `0x20`, S09 section 6.1) and enumerate the
//!   library's modules through `get_module`, which answers null past the last
//!   one;
//! * wire the host members into a module and run its `init` (S09 section 6.3
//!   step 1);
//! * process one block of interleaved 16-bit PCM in place and report the
//!   validated frame count (S09 section 6.2);
//! * stop the active module with `quit` (S09 section 6.3 step 4);
//! * answer the input plugin's `dsp_is_active` and `dsp_process` host
//!   callbacks for the active module (S09 section 3.1, offsets 0x84 and 0x88).
//!
//! # What this module does not do
//!
//! * **It does not decide where the samples come from.** The input plugin
//!   owns the decode thread and the buffer (S09 section 3.6); the host only
//!   answers the two callbacks it is given (S09 section 6.3 step 3). The
//!   Rustamp audio path is a separate lane and does not exist yet, so the
//!   `modify_samples` call site is a documented follow-up.
//! * **It does not free the DLL.** S09 section 6.3 step 4 frees the DSP DLL
//!   right after `quit` in the 5.x host; this host keeps the conservative
//!   policy of S09 section 1 line 46 and retains the mapping.
//! * **It does not place the equalizer.** Whether the host EQ runs before or
//!   after the DSP is S09 U-12 and stays open; no EQ stage lives here.
//! * **It does not interpret a nonzero `init` return.** S09 section 6.2 says
//!   "0 on success" and nothing more; the callers decide (see
//!   [`crate::loader`]).
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::ffi::{c_int, c_void};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

use rustamp_plugin_abi::PluginKind;
use rustamp_plugin_abi::common::{
    DSP_VERSION, DSP_VERSION_21, DSP_VERSION_22, read_c_string_lossy,
};
use rustamp_plugin_abi::dsp::{DspHeader, DspModule};

use crate::loader::PluginLoadError;

/// Upper bound on the number of modules the host asks a DSP library for.
///
/// S09 section 6.1 gives `get_module` the same "null past the last one"
/// contract as the visualization header (section 5.1), but does not bound the
/// list. A plugin that never answers null must not hang the host, so the
/// enumeration stops here. Own policy, not S09.
pub const DSP_MAX_MODULES: usize = 64;

/// The module index the host starts when no selection is persisted yet.
///
/// S09 section 6.3 step 1 passes the stored `config_dspplugin_num` to
/// `get_module`. The host stores no selection today (that setting is S12's
/// business), so the first module is the honest default.
pub const DSP_DEFAULT_MODULE_INDEX: i32 = 0;

/// Host-visible state of one module in a DSP library's module list
/// (S09 section 6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DspModuleInfo {
    /// The 0-based index the library hands this module out under.
    pub index: i32,
    /// The module's own name (S09 section 6.2, offset 0x00).
    pub description: String,
    /// The module supplied a `config` callback (S09 section 6.2, offset 0x0C).
    pub has_config: bool,
    /// The module supplied an `init` callback (S09 section 6.2, offset 0x10).
    pub has_init: bool,
    /// The module supplied a `process` callback (S09 section 6.2, offset 0x14).
    pub has_process: bool,
    /// The module supplied a `quit` callback (S09 section 6.2, offset 0x18).
    pub has_quit: bool,
}

/// The library-level state of a loaded DSP plugin (S09 section 6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DspLibraryInfo {
    /// The header version the plugin reported (S09 section 6.1, offset 0x00).
    pub version: i32,
    /// The library's own name (S09 section 6.1, offset 0x04).
    pub description: String,
    /// The modules the library hands out, in index order.
    pub modules: Vec<DspModuleInfo>,
}

/// Host-visible state of the active DSP module (S09 section 6.2).
///
/// This is what an acceptance runner checks **before** it calls anything: the
/// members the host filled in and what the plugin's `init` returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DspRunInfo {
    /// The index of the active module in the library's module list.
    pub index: i32,
    /// `parent_window` as filled by the host before `init` (S09 section 6.2,
    /// offset 0x04).
    pub parent_window: usize,
    /// `dll_instance` as filled by the host before `init` (S09 section 6.2,
    /// offset 0x08).
    pub dll_instance: usize,
    /// `init` was actually called. False means the slot was null and skipped.
    pub init_called: bool,
    /// The raw value `init` returned.
    pub init_result: c_int,
    /// The host's `dsp_is_active` answers nonzero for exactly this module
    /// (S09 section 3.1, offset 0x84).
    pub host_active: bool,
}

/// One module of a DSP library: the plugin's own struct and the state the
/// host read out of it.
#[derive(Debug)]
pub(crate) struct DspModuleEntry {
    /// The plugin's static module struct. Its address is stable while the DLL
    /// is mapped (S09 section 0).
    pub(crate) module: *mut DspModule,
    /// What the host read out of that struct.
    pub(crate) info: DspModuleInfo,
}

/// Why a DSP operation was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DspError {
    /// The plugin is not a DSP plugin, so the operation is not routed for it.
    WrongKind(PluginKind),
    /// The library has no module under this index.
    UnknownModule(i32),
    /// The module's `init` refused: a nonzero return aborts the start.
    InitFailed(i32),
    /// No DSP module is active, so there is nothing to drive.
    NotRunning,
}

impl fmt::Display for DspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongKind(kind) => write!(
                f,
                "the DSP bridge is not routed for a {kind:?} plugin; nothing was called"
            ),
            Self::UnknownModule(index) => {
                write!(f, "the DSP library has no module with index {index}")
            }
            Self::InitFailed(code) => write!(
                f,
                "the DSP module refused to start with code {code}; nothing was activated"
            ),
            Self::NotRunning => f.write_str("no DSP module is active"),
        }
    }
}

impl std::error::Error for DspError {}

/// What happened while a DSP module was initialized.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DspInit {
    /// The module the library handed out.
    pub(crate) module: *mut DspModule,
    /// Whether an `init` slot was present and called.
    pub(crate) init_called: bool,
    /// The raw value `init` returned. S09 section 6.2 says "0 on success" and
    /// nothing more about a nonzero value, so the caller decides what it
    /// means.
    pub(crate) init_result: c_int,
}

/// Validate a DSP header and enumerate the library's modules
/// (S09 section 6.1, section 6.3 step 1).
///
/// The header version must be `0x20` (S09 section 6.1): `0x21` and `0x22`
/// belong to the 5.x-only main-window-taking entry point and are refused,
/// like every other 5.x-only version in this host (S09 U-7: strict equality).
/// The modules are read by asking `get_module(0)`, `get_module(1)`, ... until
/// the answer is null.
///
/// # Safety
///
/// `header` must be null or point at a live `winampDSPHeader` in the mapped
/// plugin's static data (S09 section 0). The plugin's `get_module` runs here
/// on the caller's thread.
pub(crate) unsafe fn list_dsp_modules(
    header: *mut DspHeader,
) -> Result<Vec<DspModuleEntry>, PluginLoadError> {
    if header.is_null() {
        return Err(PluginLoadError::NullModule);
    }
    // SAFETY: caller guarantees a live header for this read.
    let version = unsafe { (*header).version };
    // S09 section 6.1: 0x20 for 2.x. The 5.x variants 0x21/0x22 are accepted
    // (owner decision 2026-10-25: 5.x plug-ins are welcome; U-7 closes as a
    // range). Their header is 16 bytes (a trailing `key_check` the host never
    // calls); the module layout is the same.
    if version != DSP_VERSION && version != DSP_VERSION_21 && version != DSP_VERSION_22 {
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
    let mut modules = Vec::new();
    for index in 0..DSP_MAX_MODULES as i32 {
        // SAFETY: cdecl getter from the mapped image (or the test double).
        let module = unsafe { get_module(index) };
        if module.is_null() {
            break;
        }
        modules.push(DspModuleEntry {
            module,
            // SAFETY: the module came from the plugin's own getter and is
            // live in the mapped image's static data.
            info: unsafe { read_dsp_module_info(module, index) },
        });
    }
    Ok(modules)
}

/// Read the host-visible state out of a DSP module (S09 section 6.2).
///
/// # Safety
///
/// `module` must point at a live `winampDSPModule` in plugin static data.
unsafe fn read_dsp_module_info(module: *mut DspModule, index: i32) -> DspModuleInfo {
    // SAFETY: the caller guarantees a live module for these reads.
    let m = unsafe { *module };
    DspModuleInfo {
        index,
        // SAFETY: a null description pointer reads as empty and is not
        // dereferenced (S09 section 1).
        description: unsafe { read_c_string_lossy(m.description) },
        has_config: m.config.is_some(),
        has_init: m.init.is_some(),
        has_process: m.process.is_some(),
        has_quit: m.quit.is_some(),
    }
}

/// Fill the host members of a DSP module and run its `init`
/// (S09 section 6.2, section 6.3 step 1).
///
/// The host writes the parent window and the plugin's own module handle
/// before `init` runs (S09 section 6.2, offsets 0x04 and 0x08). The plugin's
/// own return value is **reported, not interpreted**: S09 section 6.2 says "0
/// on success" and nothing more, so the caller decides what a nonzero value
/// means.
///
/// `None` means the module pointer was null, so nothing was called.
///
/// # Safety
///
/// `module` must be null or point at a live `winampDSPModule` in plugin
/// static data; the plugin's `init` runs here on the caller's thread, which
/// must be the thread that owns the host window.
pub(crate) unsafe fn init_dsp_module(
    module: *mut DspModule,
    host_hwnd: usize,
    hinstance: *mut c_void,
) -> Option<DspInit> {
    if module.is_null() {
        return None;
    }
    // SAFETY: live module in plugin static data.
    unsafe {
        (*module).parent_window = host_hwnd as *mut c_void;
        (*module).dll_instance = hinstance;
    }
    let mut init_called = false;
    let mut init_result = 0;
    // SAFETY: cdecl, one argument (S09 section 6.2, offset 0x10).
    if let Some(init) = unsafe { (*module).init } {
        init_called = true;
        init_result = unsafe { init(module) };
    }
    Some(DspInit {
        module,
        init_called,
        init_result,
    })
}

/// The DSP module that is active, if any.
///
/// S09 section 6.3 step 2: only one DSP module is active at a time, and the
/// host's `dsp_is_active` and `dsp_process` callbacks answer for that one
/// (S09 section 3.1, offsets 0x84 and 0x88). The pointer lives in an atomic
/// because the input plugin's decode thread reads it (S09 section 3.6) while
/// the host's own thread starts and stops the module.
static ACTIVE_DSP_MODULE: AtomicUsize = AtomicUsize::new(0);

/// The DSP module that is active, or null.
pub(crate) fn active_dsp_module() -> *mut DspModule {
    ACTIVE_DSP_MODULE.load(Ordering::SeqCst) as *mut DspModule
}

/// Make `module` the active DSP module. A null pointer clears the slot.
pub(crate) fn set_active_dsp_module(module: *mut DspModule) {
    ACTIVE_DSP_MODULE.store(module as usize, Ordering::SeqCst);
}

/// Clear the active slot, but only if it still holds `module`: a replaced or
/// quitting plugin must not deactivate its successor.
pub(crate) fn clear_active_dsp_module(module: *mut DspModule) {
    let _ =
        ACTIVE_DSP_MODULE.compare_exchange(module as usize, 0, Ordering::SeqCst, Ordering::SeqCst);
}

/// Would the host modify the samples? (S09 section 3.1, offset 0x84).
///
/// The answer is nonzero while a DSP module with a `process` callback is
/// active. No equalizer stage lives here: the order of the host EQ and the
/// DSP is S09 U-12 and stays open.
pub(crate) fn dsp_is_active() -> bool {
    let module = active_dsp_module();
    if module.is_null() {
        return false;
    }
    // SAFETY: the slot only ever holds a module installed by a successful DSP
    // start, whose static storage outlives the retained DLL mapping (S09 §0).
    unsafe { (*module).process }.is_some()
}

/// Stop the active DSP module, if any (S09 section 6.3 step 4).
///
/// The active module's `quit` is called and the slot is cleared. The DLL
/// stays mapped: the 5.x host frees the DSP DLL after `quit` (S09 section 6.3
/// step 4), this host keeps the conservative policy of S09 section 1 line 46.
pub(crate) fn stop_active_dsp_module() {
    let module = active_dsp_module();
    if module.is_null() {
        return;
    }
    // SAFETY: the slot holds a live module in plugin static data.
    if let Some(quit) = unsafe { (*module).quit } {
        // SAFETY: cdecl, one argument (S09 section 6.2, offset 0x18).
        unsafe { quit(module) };
    }
    clear_active_dsp_module(module);
}

/// Validate a DSP `process` return value (S09 section 6.2: output frames
/// between half and double the input).
///
/// Own choice, not in S09: a return outside that range cannot be trusted with
/// the shared in-place buffer, so the host keeps the input frame count and
/// leaves the tail untouched instead of propagating garbage downstream. The
/// cdecl contract offers the decode thread no error channel, so this fallback
/// is silent at the audio path by necessity; misbehavior is observable only
/// as "length unchanged". Non-positive input is passed through untouched.
fn dsp_output_frames(input_frames: c_int, reported_frames: c_int) -> c_int {
    if input_frames <= 0 {
        return input_frames;
    }
    let half = input_frames / 2 + input_frames % 2;
    let double = input_frames.checked_mul(2).unwrap_or(c_int::MAX);
    if reported_frames < half || reported_frames > double {
        input_frames
    } else {
        reported_frames
    }
}

/// Drive one DSP `process` call with validation (S09 section 6.2).
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
pub(crate) unsafe fn call_dsp_process(
    module: *mut DspModule,
    samples: *mut c_void,
    frames: c_int,
    bits: c_int,
    channels: c_int,
    sample_rate: c_int,
) -> c_int {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::LoadedPlugin;
    use std::cell::{Cell, RefCell};
    use std::sync::Mutex;

    /// Serializes the tests that touch the process-wide active-DSP slot.
    static DSP_SLOT_LOCK: Mutex<()> = Mutex::new(());

    thread_local! {
        /// The modules the synthetic getter hands out, in index order.
        static DSP_MODULES: RefCell<Vec<*mut DspModule>> = const { RefCell::new(Vec::new()) };
        /// How often the synthetic getter was called.
        static DSP_GETTER_CALLS: Cell<i32> = const { Cell::new(0) };
        /// What the synthetic callbacks recorded, in call order.
        static DSP_LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn dsp_note(entry: impl Into<String>) {
        DSP_LOG.with(|log| log.borrow_mut().push(entry.into()));
    }

    fn dsp_log() -> Vec<String> {
        DSP_LOG.with(|log| log.borrow().clone())
    }

    fn dsp_log_clear() {
        DSP_LOG.with(|log| log.borrow_mut().clear());
        DSP_MODULES.with(|modules| modules.borrow_mut().clear());
        DSP_GETTER_CALLS.with(|calls| calls.set(0));
    }

    /// A getter that answers the modules of [`DSP_MODULES`] and null past the
    /// last one (S09 section 6.1).
    unsafe extern "C" fn test_get_module(index: c_int) -> *mut DspModule {
        DSP_GETTER_CALLS.with(|calls| calls.set(calls.get() + 1));
        DSP_MODULES.with(|modules| {
            let modules = modules.borrow();
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            modules.get(index).copied().unwrap_or(core::ptr::null_mut())
        })
    }

    /// A getter that never answers null: the enumeration must still stop.
    unsafe extern "C" fn test_get_module_forever(_index: c_int) -> *mut DspModule {
        DSP_GETTER_CALLS.with(|calls| calls.set(calls.get() + 1));
        DSP_MODULES.with(|modules| {
            let mut modules = modules.borrow_mut();
            if modules.is_empty() {
                // A stable, non-null module the getter always answers with.
                let module: &'static mut DspModule = Box::leak(Box::new(test_module(c"endless")));
                modules.push(module as *mut DspModule);
            }
            modules[0]
        })
    }

    /// `init` reads the members the host must have filled before it runs.
    unsafe extern "C" fn test_init(module: *mut DspModule) -> c_int {
        if module.is_null() {
            dsp_note("init with no module");
            return -1;
        }
        // SAFETY: the test owns the module and keeps it alive across the call.
        let (parent, instance) = unsafe {
            (
                (*module).parent_window as usize,
                (*module).dll_instance as usize,
            )
        };
        dsp_note(format!("init parent={parent:#x} instance={instance:#x}"));
        0
    }

    /// `init` of the module that refuses to start.
    unsafe extern "C" fn test_init_refusing(_module: *mut DspModule) -> c_int {
        dsp_note("init refused");
        1
    }

    /// `process` doubles every sample, so the effect is measurable.
    unsafe extern "C" fn test_process(
        _module: *mut DspModule,
        samples: *mut c_void,
        frames: c_int,
        _bits: c_int,
        _channels: c_int,
        _sample_rate: c_int,
    ) -> c_int {
        let out = samples as *mut i16;
        let mut i = 0_isize;
        while i < frames as isize * 2 {
            // SAFETY: index bounded by the caller's buffer.
            unsafe {
                let value = *out.offset(i);
                *out.offset(i) = value.saturating_mul(2);
            }
            i += 1;
        }
        dsp_note(format!("process frames={frames}"));
        frames
    }

    /// `process` of the module without a callback: nothing may be called.
    unsafe extern "C" fn test_process_liar(
        _module: *mut DspModule,
        _samples: *mut c_void,
        frames: c_int,
        _bits: c_int,
        _channels: c_int,
        _sample_rate: c_int,
    ) -> c_int {
        dsp_note("process lied");
        frames.saturating_mul(10)
    }

    /// `quit` records the teardown.
    unsafe extern "C" fn test_quit(_module: *mut DspModule) {
        dsp_note("quit");
    }

    /// `config` records the call. A DSP config opens a dialog, so the host
    /// only calls it when a caller asks for it explicitly.
    unsafe extern "C" fn test_config(_module: *mut DspModule) {
        dsp_note("config");
    }

    /// A synthetic module with the fields a test needs.
    fn test_module(description: &'static core::ffi::CStr) -> DspModule {
        DspModule {
            description: description.as_ptr(),
            config: Some(test_config),
            init: Some(test_init),
            process: Some(test_process),
            quit: Some(test_quit),
            ..DspModule::default()
        }
    }

    /// A header whose getter hands out `modules`.
    fn test_header(modules: Vec<*mut DspModule>) -> DspHeader {
        DSP_MODULES.with(|slot| *slot.borrow_mut() = modules);
        DspHeader {
            version: DSP_VERSION,
            description: c"synthetic dsp".as_ptr(),
            get_module: Some(test_get_module),
        }
    }

    #[test]
    fn the_module_list_is_read_until_the_getter_answers_null() {
        dsp_log_clear();
        let mut first = test_module(c"first");
        let mut second = test_module(c"second");
        let mut header = test_header(vec![&mut first, &mut second]);
        // SAFETY: live stack header and modules; no DLL is involved.
        let modules = unsafe { list_dsp_modules(&mut header) }.expect("a valid header must list");
        assert_eq!(modules.len(), 2, "the list stops at the first null answer");
        assert_eq!(modules[0].info.index, 0);
        assert_eq!(modules[0].info.description, "first");
        assert!(modules[0].info.has_config);
        assert!(modules[0].info.has_init);
        assert!(modules[0].info.has_process);
        assert!(modules[0].info.has_quit);
        assert_eq!(modules[1].info.index, 1);
        assert_eq!(modules[1].info.description, "second");
        assert_eq!(
            DSP_GETTER_CALLS.with(|calls| calls.get()),
            3,
            "the getter is asked for the module after the last one, to see the null"
        );
    }

    #[test]
    fn a_wrong_header_version_is_refused_before_any_module_is_read() {
        dsp_log_clear();
        let mut module = test_module(c"first");
        let mut header = test_header(vec![&mut module]);
        // Anything outside 0x20..=0x22 is not a DSP header this host knows.
        for version in [0x1F, 0x23, 0x100, -1] {
            header.version = version;
            // SAFETY: live stack header; no DLL is involved.
            let error = unsafe { list_dsp_modules(&mut header) }
                .expect_err("an unknown header version must be refused");
            assert!(
                matches!(
                    error,
                    PluginLoadError::UnsupportedVersion {
                        kind: PluginKind::Dsp,
                        actual: _,
                        expected: DSP_VERSION,
                    }
                ),
                "{error:?}"
            );
        }
        assert_eq!(
            DSP_GETTER_CALLS.with(|calls| calls.get()),
            0,
            "no module may be read before the version check"
        );
    }

    #[test]
    fn a_null_header_and_a_null_getter_are_refused() {
        dsp_log_clear();
        // SAFETY: a null header is exactly the input under test.
        let error = unsafe { list_dsp_modules(core::ptr::null_mut()) }
            .expect_err("a null header must be refused");
        assert!(matches!(error, PluginLoadError::NullModule), "{error:?}");

        let mut header = DspHeader {
            version: DSP_VERSION,
            description: c"no getter".as_ptr(),
            get_module: None,
        };
        // SAFETY: live stack header; no DLL is involved.
        let error =
            unsafe { list_dsp_modules(&mut header) }.expect_err("a null getter must be refused");
        assert!(
            matches!(error, PluginLoadError::MissingCallback(_)),
            "{error:?}"
        );
    }

    #[test]
    fn the_enumeration_stops_at_the_own_bound() {
        dsp_log_clear();
        let mut header = DspHeader {
            version: DSP_VERSION,
            description: c"endless".as_ptr(),
            get_module: Some(test_get_module_forever),
        };
        // The synthetic getter answers with a module for every index, so the
        // list never ends by itself. The bound is what keeps such a plugin
        // from hanging the host (own policy, documented in DSP_MAX_MODULES).
        // SAFETY: live stack header; no DLL is involved.
        let modules = unsafe { list_dsp_modules(&mut header) }.expect("a valid header must list");
        assert_eq!(modules.len(), DSP_MAX_MODULES);
        assert_eq!(
            DSP_GETTER_CALLS.with(|calls| calls.get()),
            DSP_MAX_MODULES as i32,
            "the getter must not be asked past the bound"
        );
        assert_eq!(DSP_MAX_MODULES, 64);
    }

    #[test]
    fn init_fills_the_host_members_before_it_runs() {
        let _guard = DSP_SLOT_LOCK.lock().unwrap();
        dsp_log_clear();
        let mut module = test_module(c"first");
        // SAFETY: live stack module; no DLL is involved.
        let init = unsafe { init_dsp_module(&mut module, 0xBEEF, 0xCAFE as *mut c_void) }
            .expect("a module with an init slot must start");
        assert!(init.init_called);
        assert_eq!(init.init_result, 0);
        // S09 §6.3 step 1: the host members are filled before `init` runs.
        assert_eq!(dsp_log(), vec!["init parent=0xbeef instance=0xcafe"]);
    }

    #[test]
    fn a_null_module_is_refused_and_nothing_is_called() {
        dsp_log_clear();
        // SAFETY: a null module is exactly the input under test.
        let init = unsafe { init_dsp_module(core::ptr::null_mut(), 0x1, 0x2 as *mut c_void) };
        assert!(init.is_none(), "a null module must not be initialized");
        assert!(dsp_log().is_empty());
    }

    #[test]
    fn a_refusing_init_is_reported_and_not_activated() {
        let _guard = DSP_SLOT_LOCK.lock().unwrap();
        dsp_log_clear();
        let mut module = test_module(c"refusing");
        module.init = Some(test_init_refusing);
        // SAFETY: live stack module; no DLL is involved.
        let init = unsafe { init_dsp_module(&mut module, 0x1, 0x2 as *mut c_void) }
            .expect("the module itself is not null");
        assert!(init.init_called);
        assert_eq!(init.init_result, 1, "the plugin's own value is reported");
        assert_eq!(dsp_log(), vec!["init refused"]);
        assert!(
            active_dsp_module().is_null(),
            "a refusing init must not activate anything"
        );
    }

    #[test]
    fn the_process_callback_runs_in_place_and_reports_its_own_count() {
        dsp_log_clear();
        let mut module = test_module(c"first");
        let mut pcm = vec![1000_i16, -1000, 2000, -2000];
        // SAFETY: stack module and a buffer of 2 stereo frames.
        let out = unsafe {
            call_dsp_process(
                &mut module,
                pcm.as_mut_ptr() as *mut c_void,
                2,
                16,
                2,
                44_100,
            )
        };
        assert_eq!(out, 2);
        assert_eq!(pcm, vec![2000, -2000, 4000, -4000]);
        assert_eq!(dsp_log(), vec!["process frames=2"]);
    }

    #[test]
    fn a_missing_process_or_a_null_address_is_a_passthrough() {
        dsp_log_clear();
        let mut module = test_module(c"first");
        module.process = None;
        let mut pcm = vec![1_i16, 2, 3, 4];
        // SAFETY: stack module and buffer; then null addresses.
        let out = unsafe {
            call_dsp_process(
                &mut module,
                pcm.as_mut_ptr() as *mut c_void,
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
                    pcm.as_mut_ptr() as *mut c_void,
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
        assert!(dsp_log().is_empty(), "no plugin callback may have run");
    }

    #[test]
    fn the_output_frame_count_edges_are_honored_and_garbage_is_ignored() {
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
    fn a_lying_process_return_does_not_propagate() {
        dsp_log_clear();
        let mut module = test_module(c"liar");
        module.process = Some(test_process_liar);
        let mut pcm = vec![7_i16, 8, 9, 10];
        // SAFETY: stack module and a buffer of 2 stereo frames.
        let out = unsafe {
            call_dsp_process(
                &mut module,
                pcm.as_mut_ptr() as *mut c_void,
                2,
                16,
                2,
                44_100,
            )
        };
        assert_eq!(out, 2, "10x return must fall back to the input count");
        assert_eq!(dsp_log(), vec!["process lied"]);
    }

    #[test]
    fn the_host_callbacks_follow_the_active_module() {
        let _guard = DSP_SLOT_LOCK.lock().unwrap();
        dsp_log_clear();
        assert!(active_dsp_module().is_null());
        assert!(!dsp_is_active(), "nothing is active yet");

        let mut module = test_module(c"first");
        set_active_dsp_module(&mut module);
        assert!(
            dsp_is_active(),
            "a module with a process callback is active"
        );
        clear_active_dsp_module(&mut module);
        assert!(!dsp_is_active());
        assert!(active_dsp_module().is_null());

        // A module without a `process` callback reads inactive: the host would
        // not touch the samples (S09 §3.1, offset 0x84).
        let mut silent = test_module(c"silent");
        silent.process = None;
        set_active_dsp_module(&mut silent);
        assert!(!dsp_is_active());
        // A stale clear for a module that no longer owns the slot is a no-op.
        clear_active_dsp_module(&mut module);
        assert_eq!(active_dsp_module(), &mut silent as *mut DspModule);
        clear_active_dsp_module(&mut silent);
        assert!(active_dsp_module().is_null());
    }

    #[test]
    fn only_one_dsp_module_is_active_at_a_time() {
        let _guard = DSP_SLOT_LOCK.lock().unwrap();
        dsp_log_clear();
        let mut first = test_module(c"first");
        let mut second = test_module(c"second");
        let mut header = test_header(vec![&mut first, &mut second]);
        // SAFETY: live stack header and modules; no DLL is involved.
        let modules = unsafe { list_dsp_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::dsp_for_test("dsp_synthetic", &mut header, modules, 0x99);

        // S09 §6.3 step 1: the host starts the selected module at load time.
        plugin.dsp_start(0).expect("the first module must start");
        assert!(plugin.is_initialized());
        assert_eq!(dsp_log(), vec!["init parent=0x99 instance=0x0"]);

        // S09 §6.3 step 2: only one module is active at a time, so the first
        // one is quit when the selection changes.
        plugin.dsp_start(1).expect("the second module must start");
        assert_eq!(
            dsp_log(),
            vec![
                "init parent=0x99 instance=0x0",
                "quit",
                "init parent=0x99 instance=0x0"
            ]
        );
        assert!(plugin.is_initialized());
    }

    #[test]
    fn the_plugin_lifecycle_runs_init_process_and_quit_once() {
        let _guard = DSP_SLOT_LOCK.lock().unwrap();
        dsp_log_clear();
        let mut module = test_module(c"first");
        let mut header = test_header(vec![&mut module]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_dsp_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::dsp_for_test("dsp_synthetic", &mut header, modules, 0x99);

        plugin.dsp_start(0).expect("the module must start");
        assert!(plugin.is_initialized());
        let library = plugin.dsp_info().expect("a DSP plugin reports its library");
        assert_eq!(library.version, DSP_VERSION);
        assert_eq!(library.modules.len(), 1);
        assert_eq!(library.modules[0].description, "first");
        let run = plugin
            .dsp_run_info()
            .expect("an active module reports its state");
        assert_eq!(run.index, 0);
        assert_eq!(run.parent_window, 0x99, "the host filled the parent window");
        assert_eq!(run.dll_instance, 0x0);
        assert!(run.init_called);
        assert_eq!(run.init_result, 0);
        assert!(run.host_active, "the host would modify the samples");

        // The audio path: one block through the module's `process`.
        let mut pcm = vec![100_i16, -100, 200, -200];
        let out = plugin
            .dsp_process_i16(&mut pcm, 2, 44_100)
            .expect("an active module processes the block");
        assert_eq!(out, 2);
        assert_eq!(pcm, vec![200, -200, 400, -400]);

        plugin.quit();
        assert!(!plugin.is_initialized());
        assert!(
            plugin.dsp_run_info().is_none(),
            "no module runs after the quit"
        );
        assert_eq!(
            dsp_log().last(),
            Some(&"quit".to_string()),
            "quit must reach the plugin"
        );
        // A second quit must not call the plugin again.
        let before = dsp_log().len();
        plugin.quit();
        assert_eq!(dsp_log().len(), before);
    }

    #[test]
    fn an_unknown_module_index_and_a_wrong_kind_are_refused() {
        let _guard = DSP_SLOT_LOCK.lock().unwrap();
        dsp_log_clear();
        let mut module = test_module(c"first");
        let mut header = test_header(vec![&mut module]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_dsp_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::dsp_for_test("dsp_synthetic", &mut header, modules, 0x99);
        assert_eq!(plugin.dsp_start(7), Err(DspError::UnknownModule(7)));
        assert!(dsp_log().is_empty(), "no module may be started");

        // A plugin of another kind has no DSP bridge.
        let mut other = LoadedPlugin::unsupported_for_test(PluginKind::Output);
        assert_eq!(
            other.dsp_start(0),
            Err(DspError::WrongKind(PluginKind::Output))
        );
        assert!(other.dsp_info().is_none());
        assert!(other.dsp_run_info().is_none());
    }

    #[test]
    fn the_config_callback_is_routed_to_the_active_module() {
        let _guard = DSP_SLOT_LOCK.lock().unwrap();
        dsp_log_clear();
        let mut module = test_module(c"first");
        let mut header = test_header(vec![&mut module]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_dsp_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::dsp_for_test("dsp_synthetic", &mut header, modules, 0x99);
        plugin.dsp_start(0).expect("the module must start");
        assert_eq!(plugin.config(), Ok(()));
        assert_eq!(dsp_log(), vec!["init parent=0x99 instance=0x0", "config"]);

        // A module without a `config` slot is refused, never called.
        dsp_log_clear();
        let mut bare = test_module(c"bare");
        bare.config = None;
        let mut header = test_header(vec![&mut bare]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_dsp_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::dsp_for_test("dsp_bare", &mut header, modules, 0x99);
        plugin.dsp_start(0).expect("the module must start");
        assert_eq!(
            plugin.config(),
            Err(crate::loader::PluginConfigError::MissingCallback)
        );
        assert!(
            !dsp_log().iter().any(|entry| entry == "config"),
            "a null config slot must never be called"
        );

        // A module that is not the active one any more is not live: its
        // dialog must not be opened (S09 §6.3 step 2).
        dsp_log_clear();
        let mut live = test_module(c"live");
        let mut header = test_header(vec![&mut live]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_dsp_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::dsp_for_test("dsp_live", &mut header, modules, 0x99);
        plugin.dsp_start(0).expect("the module must start");
        plugin.quit();
        assert_eq!(
            plugin.config(),
            Err(crate::loader::PluginConfigError::NotInitialized)
        );
        assert!(
            !dsp_log().iter().any(|entry| entry == "config"),
            "a quit module must not be configured"
        );
    }
}
