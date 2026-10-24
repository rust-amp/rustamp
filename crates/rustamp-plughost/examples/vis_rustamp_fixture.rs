//! A clean-room **visualization** (`vis_`) plugin fixture for the DLL
//! acceptance of the visualization kind.
//!
//! # What this is
//!
//! A separate `cdylib` artifact — never a library linked into the test or the
//! host — that implements the visualization plugin ABI of
//! `specs/S09-plugin-abi.md` §5 and nothing else:
//!
//! * one undecorated cdecl export, `winampVisGetHeader`, that takes no
//!   arguments and returns the plugin's static header (S09 §2, §5.1);
//! * a static header with version `0x101`, a library name and a
//!   `get_module` callback (S09 §5.1);
//! * two static modules: the first publishes a name, a latency, a delay and
//!   two spectrum and two waveform rows, the second's `init` refuses
//!   (S09 §5.2);
//! * `init` records the members the host filled in — the parent window, the
//!   DLL instance, the sample rate and the channel count — before it returns
//!   (S09 §5.2, §5.3 step 2);
//! * `render` records the first bytes of the spectrum and the waveform row
//!   the host filled in (S09 §5.2, §5.3 step 4b);
//! * `quit` records the teardown (S09 §5.3 step 5).
//!
//! # What this is not
//!
//! * It is not a mock: the export is resolved by name through the OS loader
//!   and called through the real cdecl ABI, and every callback is a real
//!   function in a real DLL.
//! * It is not a third-party plugin and proves nothing about one. It is the
//!   instrumented fixture the acceptance test needs: our own DLL, built only
//!   from S09.
//! * **It opens no window at all.** The plugin owns its window in the real
//!   world (S09 §5.3), and the host never creates one for it; this fixture
//!   therefore creates none either, so the acceptance run stays headless.
//!
//! # Evidence
//!
//! Every step the fixture takes is appended to its own trace file in the
//! process temp directory, written **from inside the plugin**. The trace is
//! plugin-side evidence: the host and the test runner cannot forge it.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004), like the rest of this package.

#![cfg(windows)]

#[path = "../tests/common/vis_protocol.rs"]
mod vis_protocol;

use vis_protocol::*;

use std::ffi::{c_char, c_int};

use rustamp_plugin_abi::common::VIS_VERSION;
use rustamp_plugin_abi::vis::{VisHeader, VisModule};

/// The library header (S09 §5.1).
///
/// It lives in this DLL's static data, so its address is stable for as long
/// as the DLL is loaded, and the host reads it through the pointer the entry
/// point returns.
static mut HEADER: VisHeader = VisHeader {
    version: VIS_VERSION,
    description: FIXTURE_LIBRARY_DESCRIPTION.as_ptr().cast::<c_char>(),
    get_module: Some(fixture_get_module),
};

/// The fixture's first module: it starts (S09 §5.2).
static mut MODULE_A: VisModule = VisModule {
    description: FIXTURE_MODULE_A_DESCRIPTION.as_ptr().cast::<c_char>(),
    parent_window: core::ptr::null_mut(),
    dll_instance: core::ptr::null_mut(),
    sample_rate: 0,
    channels: 0,
    latency_ms: FIXTURE_LATENCY_MS,
    delay_ms: FIXTURE_DELAY_MS,
    spectrum_rows: FIXTURE_SPECTRUM_ROWS,
    waveform_rows: FIXTURE_WAVEFORM_ROWS,
    spectrum_data: [[0; 576]; 2],
    waveform_data: [[0; 576]; 2],
    config: Some(fixture_config),
    init: Some(fixture_init),
    render: Some(fixture_render),
    quit: Some(fixture_quit),
    user_data: core::ptr::null_mut(),
};

/// The fixture's second module: its `init` refuses, so the start is aborted
/// (S09 §5.2, offset 0x928).
static mut MODULE_B: VisModule = VisModule {
    description: FIXTURE_MODULE_B_DESCRIPTION.as_ptr().cast::<c_char>(),
    parent_window: core::ptr::null_mut(),
    dll_instance: core::ptr::null_mut(),
    sample_rate: 0,
    channels: 0,
    latency_ms: 0,
    delay_ms: 0,
    spectrum_rows: 0,
    waveform_rows: 0,
    spectrum_data: [[0; 576]; 2],
    waveform_data: [[0; 576]; 2],
    config: Some(fixture_config),
    init: Some(fixture_init_refusing),
    render: Some(fixture_render),
    quit: Some(fixture_quit),
    user_data: core::ptr::null_mut(),
};

/// The visualization entry point (S09 §2, §5.1).
///
/// cdecl, no arguments, returns the plugin's static header. The name is
/// undecorated, as the ABI requires (S09 §1).
#[unsafe(no_mangle)]
pub extern "C" fn winampVisGetHeader() -> *mut VisHeader {
    // `HEADER` is this DLL's static data. Its address is stable for the life
    // of the DLL, which is exactly the lifetime S09 §0 gives it. The raw
    // pointer keeps the static itself unborrowed.
    core::ptr::addr_of_mut!(HEADER)
}

/// `get_module(index)` (S09 §5.1): the module under `index`, or null when the
/// index is past the last one. The host calls 0, 1, 2... until the answer is
/// null (S09 §5.1).
unsafe extern "C" fn fixture_get_module(index: c_int) -> *mut VisModule {
    match index {
        0 => core::ptr::addr_of_mut!(MODULE_A),
        1 => core::ptr::addr_of_mut!(MODULE_B),
        _ => core::ptr::null_mut(),
    }
}

/// Which of the two modules a callback was called on.
fn module_index(module: *mut VisModule) -> usize {
    if module == core::ptr::addr_of_mut!(MODULE_A) {
        0
    } else {
        1
    }
}

/// `init` (S09 §5.2, offset 0x928): called after the host filled the members.
///
/// Records what the plugin saw, so the acceptance test can prove the host
/// filled them **before** the call.
unsafe extern "C" fn fixture_init(module: *mut VisModule) -> c_int {
    // SAFETY: the host filled the members in before calling us (S09 §5.2).
    let (parent, instance, rate, channels) = unsafe {
        (
            (*module).parent_window as usize,
            (*module).dll_instance as usize,
            (*module).sample_rate,
            (*module).channels,
        )
    };
    trace(&init_line(
        module_index(module),
        parent,
        instance,
        rate,
        channels,
    ));
    0
}

/// `init` of the second module: it refuses, which aborts the start
/// (S09 §5.2, offset 0x928).
unsafe extern "C" fn fixture_init_refusing(module: *mut VisModule) -> c_int {
    // SAFETY: the host filled the members in before calling us (S09 §5.2).
    let (parent, instance, rate, channels) = unsafe {
        (
            (*module).parent_window as usize,
            (*module).dll_instance as usize,
            (*module).sample_rate,
            (*module).channels,
        )
    };
    trace(&init_line(
        module_index(module),
        parent,
        instance,
        rate,
        channels,
    ));
    FIXTURE_INIT_REFUSAL_CODE
}

/// `render` (S09 §5.2, offset 0x92C): draw one frame from the arrays the
/// host filled in.
unsafe extern "C" fn fixture_render(module: *mut VisModule) -> c_int {
    // SAFETY: the host filled the arrays in before calling us (S09 §5.2).
    let (spectrum, waveform) =
        unsafe { ((*module).spectrum_data[0][0], (*module).waveform_data[0][0]) };
    trace(&render_line(module_index(module), spectrum, waveform));
    0
}

/// `quit` (S09 §5.2, offset 0x930): the host tears the module down.
unsafe extern "C" fn fixture_quit(module: *mut VisModule) {
    trace(&quit_line(module_index(module)));
}

/// `config` (S09 §5.2, offset 0x924): opens the plugin's configuration
/// dialog.
///
/// The host routes it to the running module when the player asks for the
/// visualization plug-in's configuration; nothing calls it on its own, so the
/// trace line below appears only after such a request.
unsafe extern "C" fn fixture_config(module: *mut VisModule) {
    trace(&config_line(module_index(module)));
}

/// The fixture's own trace file.
fn trace_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "{TRACE_FILE_PREFIX}{}.{TRACE_FILE_EXTENSION}",
        std::process::id()
    ))
}

/// Append one line to the fixture's own trace.
///
/// The trace is written from inside the plugin, so it is plugin-side evidence
/// that neither the host nor the test runner can forge.
fn trace(event: &str) {
    use std::io::Write;
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(trace_path())
    else {
        return;
    };
    let _ = writeln!(file, "{event}");
    let _ = file.flush();
}
