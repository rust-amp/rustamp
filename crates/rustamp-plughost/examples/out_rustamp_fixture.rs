//! A clean-room **output** (`out_`) plugin fixture for the P3 DLL acceptance
//! (`docs/tasks/P3-dll-acceptance.md`).
//!
//! # What this is
//!
//! A separate `cdylib` artifact — never a library linked into the test or the
//! host — that implements the output plugin ABI of
//! `specs/S09-plugin-abi.md` §4 and nothing else:
//!
//! * one undecorated cdecl export, `winampGetOutModule`, that takes no
//!   arguments and returns the plugin's static module (S09 §2, §4.2);
//! * a static module with version `0x10`, a description, a module id >= 65536
//!   and **every** callback slot filled (S09 §4.1);
//! * `init` records the main window and the DLL instance the host filled in
//!   and checks that the window carries the class the ABI fixes (S09 §4.1
//!   lines 233–234, §1);
//! * `quit` records that the host tore the plugin down (S09 §4.2 step 4).
//!
//! The playback callbacks (`open`, `close`, `write`, `can_write`,
//! `is_playing`, `pause`, `flush` and the two time queries) are implemented
//! and record every call, but the **host never calls them**: S09 §4.2 step 3
//! drives the output from the *input* plugin's decode thread. The acceptance
//! test asserts that their trace lines are absent, so a host that drove the
//! output itself would fail the test.
//!
//! `set_volume` and `set_pan` are the two callbacks the host itself may
//! call: S09 §3.5 step 2 has it apply the player's levels to the output
//! before `play` (the hand-over, see `src/handoff.rs`). They are recorded
//! too, so the hand-over's host half is visible in the output plug-in's own
//! trace; the acceptance test asserts that they are absent as well, because
//! no hand-over happens there.
//!
//! # What this is not
//!
//! * It is not a mock: the export is resolved by name through the OS loader
//!   and called through the real cdecl ABI, and every callback is a real
//!   function in a real DLL (P3 §7 lists "mocked/statically linked callbacks"
//!   as a false pass).
//! * It is not a third-party plugin and proves nothing about one. It is the
//!   instrumented fixture P3 §2 asks for: our own DLL, built only from S09.
//! * It opens no window and plays no audio. It is a fixture for the *host's*
//!   side of the output ABI, not a device.
//!
//! # Evidence
//!
//! Every step the fixture takes is appended to its own trace file in the
//! process temp directory, written **from inside the plugin**. The trace is
//! plugin-side evidence: the host and the test runner cannot forge it.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004), like the rest of this package.

#![cfg(windows)]

#[path = "../tests/common/out_protocol.rs"]
mod out_protocol;

use out_protocol::*;

use std::ffi::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use rustamp_plugin_abi::common::OUT_VERSION;
use rustamp_plugin_abi::output::OutModule;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;

/// The window class the ABI fixes for the host's main window (S09 §1, §8).
const HOST_WINDOW_CLASS: &str = "Winamp v1.x";

/// The plugin's static module (S09 §4.1).
///
/// It lives in this DLL's static data, so its address is stable for as long
/// as the DLL is loaded, and the host writes the two host-owned members into
/// it before it calls `init` (S09 §0, §4.1).
static mut MODULE: OutModule = OutModule {
    version: OUT_VERSION,
    description: FIXTURE_DESCRIPTION.as_ptr().cast::<c_char>(),
    id: FIXTURE_MODULE_ID,
    main_window: core::ptr::null_mut(),
    dll_instance: core::ptr::null_mut(),
    config: Some(fixture_config),
    about: Some(fixture_about),
    init: Some(fixture_init),
    quit: Some(fixture_quit),
    open: Some(fixture_open),
    close: Some(fixture_close),
    write: Some(fixture_write),
    can_write: Some(fixture_can_write),
    is_playing: Some(fixture_is_playing),
    pause: Some(fixture_pause),
    set_volume: Some(fixture_set_volume),
    set_pan: Some(fixture_set_pan),
    flush: Some(fixture_flush),
    get_played_ms: Some(fixture_get_played_ms),
    get_written_ms: Some(fixture_get_written_ms),
};

/// The latency the fixture's `open` reports: 0 ms, like a disk writer
/// (S09 §11 lists `out_disk` as reporting latency 0).
const FIXTURE_LATENCY_MS: c_int = 0;

/// The bytes the fixture accepts in one `write` (S09 §9: at most 8192).
const FIXTURE_BUFFER_BYTES: i32 = 8192;

/// 1 while the fixture is paused.
static PAUSED: AtomicU32 = AtomicU32::new(0);

/// The last volume the host set (0..255).
static VOLUME: AtomicU32 = AtomicU32::new(255);

/// The last pan the host set (-128..128).
static PAN: AtomicI32 = AtomicI32::new(0);

/// The milliseconds the fixture has accepted, as `get_written_ms` reports.
static WRITTEN_MS: AtomicU32 = AtomicU32::new(0);

/// The milliseconds the fixture has played, as `get_played_ms` reports.
static PLAYED_MS: AtomicU32 = AtomicU32::new(0);

/// The output entry point (S09 §2, §4.2).
///
/// cdecl, no arguments, returns the plugin's static module. The name is
/// undecorated, as the ABI requires (S09 §1).
#[unsafe(no_mangle)]
pub extern "C" fn winampGetOutModule() -> *mut OutModule {
    // `MODULE` is this DLL's static data. Its address is stable for the life
    // of the DLL, which is exactly the lifetime S09 §0 gives it. The raw
    // pointer keeps the static itself unborrowed.
    core::ptr::addr_of_mut!(MODULE)
}

/// `init` (S09 §4.1 offset 0x1C): called on the UI thread after the host
/// filled in the main window and the DLL instance.
unsafe extern "C" fn fixture_init() {
    let module = core::ptr::addr_of_mut!(MODULE);
    // SAFETY: the host filled both members in before calling us (S09 §4.1
    // offsets 0x0C and 0x10); reading them through the raw pointer keeps the
    // static itself unborrowed.
    let (parent, instance, id) = unsafe {
        (
            (*module).main_window as usize,
            (*module).dll_instance as usize,
            (*module).id,
        )
    };
    trace(&init_line(parent, instance, id));

    // The window must be the host's real main window: a fixture that cannot
    // see the `Winamp v1.x` class is not testing the real thing (S09 §1).
    let class_ok = window_class_name(parent) == HOST_WINDOW_CLASS;
    trace(&parent_class_line(class_ok));
}

/// `quit` (S09 §4.1 offset 0x20): the host tears the plugin down.
unsafe extern "C" fn fixture_quit() {
    trace(QUIT_LINE);
}

/// `config` (S09 §4.1 offset 0x14): opens the plugin's configuration dialog.
///
/// The host routes it with the parent window when the player asks for the
/// plug-in's configuration; nothing calls it on its own, so the trace line
/// below appears only after such a request.
unsafe extern "C" fn fixture_config(parent: *mut c_void) {
    trace(&config_line(parent as usize));
}

/// `about` (S09 §4.1 offset 0x18): opens the plugin's about box.
///
/// The host does not call it in this milestone.
unsafe extern "C" fn fixture_about(parent: *mut c_void) {
    trace(&format!("{EVENT_ABOUT}parent={:#x}", parent as usize));
}

/// `open` (S09 §4.1 offset 0x24): open the device, report the latency.
///
/// Driven by the input plugin, never by the host (S09 §4.2 step 3).
unsafe extern "C" fn fixture_open(
    sample_rate: c_int,
    channels: c_int,
    bits: c_int,
    buffer_ms: c_int,
    prebuffer_ms: c_int,
) -> c_int {
    trace(&format!(
        "{EVENT_OPEN}rate={sample_rate} channels={channels} bits={bits} \
         buffer={buffer_ms} prebuffer={prebuffer_ms}"
    ));
    WRITTEN_MS.store(0, Ordering::SeqCst);
    PLAYED_MS.store(0, Ordering::SeqCst);
    PAUSED.store(0, Ordering::SeqCst);
    FIXTURE_LATENCY_MS
}

/// `close` (S09 §4.1 offset 0x28): close the device. Driven by the input
/// plugin, never by the host.
unsafe extern "C" fn fixture_close() {
    trace(EVENT_CLOSE);
}

/// `write` (S09 §4.1 offset 0x2C): take at most 8192 bytes, never block.
///
/// Returns 0 when the data was taken, nonzero when it could not be taken yet.
unsafe extern "C" fn fixture_write(_data: *mut c_char, len: c_int) -> c_int {
    trace(&format!("{EVENT_WRITE}len={len}"));
    if !(0..=FIXTURE_BUFFER_BYTES).contains(&len) {
        // The ABI caps a write at 8192 bytes (S09 §9); a longer request is
        // refused instead of being taken.
        return 1;
    }
    // 16-bit stereo at 44.1 kHz is the format the fixture assumes; the
    // counter is the fixture's own bookkeeping.
    let bytes_per_ms = 44100 * 2 * 2 / 1000;
    WRITTEN_MS.fetch_add((len as u32) / bytes_per_ms, Ordering::SeqCst);
    0
}

/// `can_write` (S09 §4.1 offset 0x30): how many bytes fit right now.
///
/// Driven by the input plugin, never by the host (S09 §4.2 step 3).
unsafe extern "C" fn fixture_can_write() -> c_int {
    trace(&format!("{EVENT_CAN_WRITE}{FIXTURE_BUFFER_BYTES}"));
    FIXTURE_BUFFER_BYTES
}

/// `is_playing` (S09 §4.1 offset 0x34): nonzero while audio is still audible.
///
/// Driven by the input plugin, never by the host (S09 §4.2 step 3).
unsafe extern "C" fn fixture_is_playing() -> c_int {
    let playing = i32::from(WRITTEN_MS.load(Ordering::SeqCst) > PLAYED_MS.load(Ordering::SeqCst));
    trace(&format!("{EVENT_IS_PLAYING}{playing}"));
    playing
}

/// `pause` (S09 §4.1 offset 0x38): 1 pauses, 0 resumes; returns the previous
/// state.
unsafe extern "C" fn fixture_pause(state: c_int) -> c_int {
    let previous = PAUSED.swap(u32::from(state != 0), Ordering::SeqCst);
    previous as c_int
}

/// `set_volume` (S09 §4.1 offset 0x3C): 0..255.
///
/// The host calls this one itself, as part of the hand-over before `play`
/// (S09 §3.5 step 2).
unsafe extern "C" fn fixture_set_volume(volume: c_int) {
    trace(&format!("{EVENT_SET_VOLUME}{volume}"));
    VOLUME.store(volume.clamp(0, 255) as u32, Ordering::SeqCst);
}

/// `set_pan` (S09 §4.1 offset 0x40): -128..128.
///
/// The host calls this one itself, as part of the hand-over before `play`
/// (S09 §3.5 step 2).
unsafe extern "C" fn fixture_set_pan(pan: c_int) {
    trace(&format!("{EVENT_SET_PAN}{pan}"));
    PAN.store(pan.clamp(-128, 128), Ordering::SeqCst);
}

/// `flush` (S09 §4.1 offset 0x44): drop the buffered audio and restart the
/// clock at `time_ms`.
unsafe extern "C" fn fixture_flush(time_ms: c_int) {
    PLAYED_MS.store(time_ms.max(0) as u32, Ordering::SeqCst);
    WRITTEN_MS.store(time_ms.max(0) as u32, Ordering::SeqCst);
}

/// `get_played_ms` (S09 §4.1 offset 0x48): milliseconds actually played.
unsafe extern "C" fn fixture_get_played_ms() -> c_int {
    PLAYED_MS.load(Ordering::SeqCst) as c_int
}

/// `get_written_ms` (S09 §4.1 offset 0x4C): milliseconds written so far.
unsafe extern "C" fn fixture_get_written_ms() -> c_int {
    WRITTEN_MS.load(Ordering::SeqCst) as c_int
}

/// The class name of a window, or an empty string.
fn window_class_name(hwnd: usize) -> String {
    if hwnd == 0 {
        return String::new();
    }
    let mut buffer = [0_u16; 64];
    // SAFETY: the buffer is writable for its length and the handle is live.
    let length = unsafe { GetClassNameW(HWND(hwnd as *mut c_void), &mut buffer) };
    if length <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..length as usize])
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
/// that neither the host nor the test runner can forge (P3 §5.5, §7).
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
