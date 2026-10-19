//! A clean-room plugin fixture that **faults on purpose**, for the crash
//! isolation acceptance (PLAN M3: "Crash isolation: restart the host on
//! plugin crash").
//!
//! # What this is
//!
//! A separate `cdylib` artifact — never a library linked into the test or the
//! host — that implements the general-purpose plugin ABI of
//! `specs/S09-plugin-abi.md` §7 and one thing more:
//!
//! * one undecorated cdecl export, `winampGetGeneralPurposePlugin`, that
//!   returns the plugin's static module (S09 §2, §7.1);
//! * a static module with version `0x10` and the `init`/`quit` slots
//!   (S09 §7.1);
//! * `init` writes through a null pointer, which raises a Windows access
//!   violation **inside the host process**.
//!
//! # What it is for
//!
//! The host cannot survive a faulting plugin — continuing past an access
//! violation is undefined behaviour — so the host **reports** the fault and
//! ends with its own exit code, and the supervisor starts a fresh host
//! (PLAN M3). This fixture is the only way to prove that: it is the one
//! plugin that really faults, in the real host process, through the real OS
//! loader.
//!
//! It is **VM-only**: a test that loads it is `#[ignore]`d and refuses to run
//! outside the disposable test VM (see `tests/fault_acceptance.rs`).
//!
//! # Evidence
//!
//! The fixture appends one line to its own trace file **before** it faults,
//! written from inside the plugin and named after the host's process id —
//! so the acceptance test can prove that the fault happened in the plugin's
//! `init` and not somewhere else.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004), like the rest of this package.

#![cfg(windows)]

#[path = "../tests/common/crash_protocol.rs"]
mod crash_protocol;

use crash_protocol::*;

use std::ffi::c_int;

use rustamp_plugin_abi::common::GEN_VERSION;
use rustamp_plugin_abi::general::GeneralPlugin;

/// The plugin's static module (S09 §7.1).
///
/// It lives in this DLL's static data, so its address is stable for as long
/// as the DLL is loaded, and the host writes the two host-owned members into
/// it before it calls `init` (S09 §0, §7.1).
static mut MODULE: GeneralPlugin = GeneralPlugin {
    version: GEN_VERSION,
    description: c"Rustamp crashing gen fixture".as_ptr(),
    init: Some(crash_init),
    config: None,
    quit: Some(crash_quit),
    parent_window: core::ptr::null_mut(),
    dll_instance: core::ptr::null_mut(),
};

/// The general-purpose entry point (S09 §2).
#[unsafe(no_mangle)]
pub extern "C" fn winampGetGeneralPurposePlugin() -> *mut GeneralPlugin {
    // `MODULE` is this DLL's static data. Its address is stable for the life
    // of the DLL, which is exactly the lifetime S09 §0 gives it. The raw
    // pointer keeps the static itself unborrowed.
    core::ptr::addr_of_mut!(MODULE)
}

/// `init` (S09 §7.1 offset 0x08): the fault.
///
/// The trace line comes first, so the acceptance test can tell "the plugin's
/// init ran and then faulted" from "something else faulted". The write is
/// volatile, so the compiler cannot remove it, and it is what raises the
/// access violation.
unsafe extern "C" fn crash_init() -> c_int {
    trace(&format!(
        "init entered in {FIXTURE_FILE_NAME}, about to fault"
    ));
    // SAFETY: deliberately undefined behaviour — writing through a null
    // pointer is the access violation this fixture exists to raise.
    unsafe {
        std::ptr::write_volatile(std::ptr::null_mut::<u8>(), 1);
    }
    // Unreachable in practice: the store above faults.
    0
}

/// `quit` (S09 §7.1 offset 0x10): never reached — the host is already gone.
unsafe extern "C" fn crash_quit() {
    trace("quit");
}

/// The fixture's own trace file.
///
/// Appended from inside the plugin, so it is plugin-side evidence that the
/// host or the test runner cannot forge.
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

/// The trace file of this process.
fn trace_path() -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "{TRACE_FILE_PREFIX}{}.{TRACE_FILE_EXTENSION}",
        std::process::id()
    ))
}
