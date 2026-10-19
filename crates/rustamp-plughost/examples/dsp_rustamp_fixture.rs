//! A clean-room **DSP** (`dsp_`) plugin fixture for the DLL acceptance of the
//! DSP kind.
//!
//! # What this is
//!
//! A separate `cdylib` artifact — never a library linked into the test or the
//! host — that implements the DSP plugin ABI of `specs/S09-plugin-abi.md` §6
//! and nothing else:
//!
//! * one undecorated cdecl export, `winampDSPGetHeader2`, that takes no
//!   arguments and returns the plugin's static header (S09 §2, §6.1);
//! * a static header with version `0x20`, a library name and a `get_module`
//!   callback (S09 §6.1);
//! * two static modules: the first publishes a name and a full callback set,
//!   the second's `init` refuses (S09 §6.2);
//! * `init` records the members the host filled in — the parent window and
//!   the DLL instance — before it returns (S09 §6.2, §6.3 step 1);
//! * `process` doubles every 16-bit sample in place and returns the frame
//!   count now in the buffer, so the effect is measurable and exactly
//!   predictable (S09 §6.2);
//! * `quit` records the teardown (S09 §6.3 step 4).
//!
//! # What this is not
//!
//! * It is not a mock: the export is resolved by name through the OS loader
//!   and called through the real cdecl ABI, and every callback is a real
//!   function in a real DLL.
//! * It is not a third-party plugin and proves nothing about one. It is the
//!   instrumented fixture the acceptance test needs: our own DLL, built only
//!   from S09.
//! * **It opens no window at all.** A DSP plugin has no window in the ABI;
//!   even its `config` callback only writes a trace line here, so the
//!   acceptance run stays headless.
//!
//! # Evidence
//!
//! Every step the fixture takes is appended to its own trace file in the
//! process temp directory, written **from inside the plugin**. The trace is
//! plugin-side evidence: the host and the test runner cannot forge it.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004), like the rest of this package.

#![cfg(windows)]

#[path = "../tests/common/dsp_protocol.rs"]
mod dsp_protocol;

use dsp_protocol::*;

use std::ffi::{c_char, c_int, c_void};

use rustamp_plugin_abi::common::DSP_VERSION;
use rustamp_plugin_abi::dsp::{DspHeader, DspModule};

/// The library header (S09 §6.1).
///
/// It lives in this DLL's static data, so its address is stable for as long
/// as the DLL is loaded, and the host reads it through the pointer the entry
/// point returns.
static mut HEADER: DspHeader = DspHeader {
    version: DSP_VERSION,
    description: FIXTURE_LIBRARY_DESCRIPTION.as_ptr().cast::<c_char>(),
    get_module: Some(fixture_get_module),
};

/// The fixture's first module: it starts and processes (S09 §6.2).
static mut MODULE_A: DspModule = DspModule {
    description: FIXTURE_MODULE_A_DESCRIPTION.as_ptr().cast::<c_char>(),
    parent_window: core::ptr::null_mut(),
    dll_instance: core::ptr::null_mut(),
    config: Some(fixture_config),
    init: Some(fixture_init),
    process: Some(fixture_process),
    quit: Some(fixture_quit),
    user_data: core::ptr::null_mut(),
};

/// The fixture's second module: its `init` refuses, so the start is aborted
/// (S09 §6.2, offset 0x10).
static mut MODULE_B: DspModule = DspModule {
    description: FIXTURE_MODULE_B_DESCRIPTION.as_ptr().cast::<c_char>(),
    parent_window: core::ptr::null_mut(),
    dll_instance: core::ptr::null_mut(),
    config: Some(fixture_config),
    init: Some(fixture_init_refusing),
    process: Some(fixture_process),
    quit: Some(fixture_quit),
    user_data: core::ptr::null_mut(),
};

/// The DSP entry point (S09 §2, §6.1).
///
/// cdecl, no arguments, returns the plugin's static header. The name is
/// undecorated, as the ABI requires (S09 §1).
#[unsafe(no_mangle)]
pub extern "C" fn winampDSPGetHeader2() -> *mut DspHeader {
    // `HEADER` is this DLL's static data. Its address is stable for the life
    // of the DLL, which is exactly the lifetime S09 §0 gives it. The raw
    // pointer keeps the static itself unborrowed.
    core::ptr::addr_of_mut!(HEADER)
}

/// `get_module(index)` (S09 §6.1): the module under `index`, or null when the
/// index is past the last one. The host calls 0, 1, 2... until the answer is
/// null (S09 §6.1).
unsafe extern "C" fn fixture_get_module(index: c_int) -> *mut DspModule {
    match index {
        0 => core::ptr::addr_of_mut!(MODULE_A),
        1 => core::ptr::addr_of_mut!(MODULE_B),
        _ => core::ptr::null_mut(),
    }
}

/// Which of the two modules a callback was called on.
fn module_index(module: *mut DspModule) -> usize {
    if module == core::ptr::addr_of_mut!(MODULE_A) {
        0
    } else {
        1
    }
}

/// `init` (S09 §6.2, offset 0x10): called after the host filled the members.
///
/// Records what the plugin saw, so the acceptance test can prove the host
/// filled them **before** the call.
unsafe extern "C" fn fixture_init(module: *mut DspModule) -> c_int {
    // SAFETY: the host filled the members in before calling us (S09 §6.2).
    let (parent, instance) = unsafe {
        (
            (*module).parent_window as usize,
            (*module).dll_instance as usize,
        )
    };
    trace(&init_line(module_index(module), parent, instance));
    0
}

/// `init` of the second module: it refuses, which aborts the start
/// (S09 §6.2, offset 0x10).
unsafe extern "C" fn fixture_init_refusing(module: *mut DspModule) -> c_int {
    // SAFETY: the host filled the members in before calling us (S09 §6.2).
    let (parent, instance) = unsafe {
        (
            (*module).parent_window as usize,
            (*module).dll_instance as usize,
        )
    };
    trace(&init_line(module_index(module), parent, instance));
    FIXTURE_INIT_REFUSAL_CODE
}

/// `process` (S09 §6.2, offset 0x14): work in place and report the frame
/// count now in the buffer.
///
/// The fixture doubles every 16-bit sample, so the effect is measurable and
/// exactly predictable. It returns the frame count it was given, which is
/// inside the half..double range S09 §6.2 allows.
unsafe extern "C" fn fixture_process(
    module: *mut DspModule,
    samples: *mut c_void,
    frames: c_int,
    _bits: c_int,
    channels: c_int,
    _sample_rate: c_int,
) -> c_int {
    if samples.is_null() || frames <= 0 || channels <= 0 {
        return frames;
    }
    let out = samples as *mut i16;
    let count = frames as isize * channels as isize;
    let mut i = 0_isize;
    while i < count {
        // SAFETY: index bounded by the caller's buffer.
        unsafe {
            let value = *out.offset(i);
            *out.offset(i) = value.saturating_mul(FIXTURE_GAIN);
        }
        i += 1;
    }
    // SAFETY: the host filled the buffer in before calling us (S09 §6.2).
    let first = unsafe { *out };
    trace(&process_line(module_index(module), frames, frames, first));
    frames
}

/// `quit` (S09 §6.2, offset 0x18): the host tears the module down.
unsafe extern "C" fn fixture_quit(module: *mut DspModule) {
    trace(&quit_line(module_index(module)));
}

/// `config` (S09 §6.2, offset 0x0C): the plugin's configuration dialog.
///
/// The real thing opens a window. This fixture writes a trace line and
/// nothing else, so the acceptance test can prove the route without a dialog.
unsafe extern "C" fn fixture_config(module: *mut DspModule) {
    trace(&format!("{EVENT_CONFIG}module={}", module_index(module)));
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
