//! A clean-room **input** (`in_`) plugin fixture for the DLL acceptance of
//! the input kind.
//!
//! # What this is
//!
//! A separate `cdylib` artifact — never a library linked into the test or the
//! host — that implements the input plugin ABI of `specs/S09-plugin-abi.md`
//! §3 and nothing else:
//!
//! * one undecorated cdecl export, `winampGetInModule2`, that takes no
//!   arguments and returns the plugin's static module (S09 §2, §3.1);
//! * a static module with version `0x100`, a description, an extension list
//!   and the full callback set (S09 §3.1);
//! * **flag bit 0 clear** (S09 §3.3): the fixture routes no audio through a
//!   Winamp output plugin, so it keeps its own time — the *silent clock* —
//!   and opens no device (S09 §3.7);
//! * `init` records the members the host filled in — the main window, the
//!   plugin's own module handle and the host callbacks — before it returns
//!   (S09 §3.1, §3.5 step 2);
//! * `play` records the path the host handed it, announces the stream through
//!   the host's `analyzer_init`, `vis_set_format` and `set_info` callbacks
//!   and starts the clock (S09 §3.5 step 3);
//! * `pause`, `unpause` and `stop` drive the clock and record themselves
//!   (S09 §3.5 step 6);
//! * `quit` records the teardown (S09 §3.5 line 166).
//!
//! # What this is not
//!
//! * It is not a mock: the export is resolved by name through the OS loader
//!   and called through the real cdecl ABI, and every callback is a real
//!   function in a real DLL (P3 §7 lists "mocked/statically linked
//!   callbacks" as a false pass).
//! * It is not a third-party plugin and proves nothing about one. It is the
//!   instrumented fixture the acceptance test needs: our own DLL, built only
//!   from S09.
//! * **It opens no window and no audio device.** The plugin owns its window
//!   in the real world and this fixture's `config` opens none either, so the
//!   acceptance run stays headless. With flag bit 0 clear the fixture plays
//!   nothing at all: it only keeps time (S09 §3.7).
//!
//! # Evidence
//!
//! Every step the fixture takes is appended to its own trace file in the
//! process temp directory, written **from inside the plugin**. The trace is
//! plugin-side evidence: the host and the test runner cannot forge it.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004), like the rest of this package.

#![cfg(windows)]

#[path = "../tests/common/in_protocol.rs"]
mod in_protocol;

use in_protocol::*;

use std::ffi::{c_char, c_int, c_void};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Instant;

use rustamp_plugin_abi::common::{IN_VERSION, read_c_string_lossy};
use rustamp_plugin_abi::input::InModule;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::GetClassNameW;

/// The window class the ABI fixes for the host's main window (S09 §1, §8).
const HOST_WINDOW_CLASS: &str = "Winamp v1.x";

/// The extension list and the display name the fixture publishes
/// (S09 §3.1 offset 0x10, §3.4).
///
/// The list is a sequence of string pairs ended by an empty string: the
/// extension list `mp3`, then the display name, then the empty string. The
/// host reads it only to match a file against a module, and it has no
/// module-selection path yet (S09 §3.5 step 1), so nothing checks this list.
const FIXTURE_FILE_EXTENSIONS: &[u8] = b"mp3\0Rustamp in fixture (MP3)\0\0";

/// The fixture's static module (S09 §3.1).
///
/// It lives in this DLL's static data, so its address is stable for as long
/// as the DLL is loaded, and the host writes the members it owns into it
/// before it calls `init` (S09 §0, §3.5 step 2).
///
/// The host members the plugin cannot fill itself — the analyzer,
/// visualization and DSP callbacks and `set_info` — start empty: the host
/// writes them (S09 §3.5 step 2). `out_module` stays empty as well, because
/// no output plugin is selected (S09 §3.1 offset 0x94).
static mut MODULE: InModule = InModule {
    version: IN_VERSION,
    description: FIXTURE_DESCRIPTION.as_ptr().cast::<c_char>(),
    main_window: core::ptr::null_mut(),
    dll_instance: core::ptr::null_mut(),
    file_extensions: FIXTURE_FILE_EXTENSIONS.as_ptr().cast_mut().cast::<c_char>(),
    seekable: 1,
    flags: 0,
    config: Some(fixture_config),
    about: Some(fixture_about),
    init: Some(fixture_init),
    quit: Some(fixture_quit),
    get_file_info: Some(fixture_get_file_info),
    info_box: Some(fixture_info_box),
    is_our_file: Some(fixture_is_our_file),
    play: Some(fixture_play),
    pause: Some(fixture_pause),
    unpause: Some(fixture_unpause),
    is_paused: Some(fixture_is_paused),
    stop: Some(fixture_stop),
    get_length: Some(fixture_get_length),
    get_position: Some(fixture_get_position),
    seek: Some(fixture_seek),
    set_volume: Some(fixture_set_volume),
    set_pan: Some(fixture_set_pan),
    analyzer_init: None,
    analyzer_deinit: None,
    analyzer_feed_pcm: None,
    analyzer_get_mode: None,
    analyzer_feed_rows: None,
    vis_feed_pcm: None,
    vis_get_mode: None,
    vis_feed_frame: None,
    vis_set_format: None,
    dsp_is_active: None,
    dsp_process: None,
    eq_set: Some(fixture_eq_set),
    set_info: None,
    out_module: core::ptr::null_mut(),
};

// ---------------------------------------------------------------------------
// The silent clock (S09 §3.7)
// ---------------------------------------------------------------------------

/// 1 while the fixture is paused.
static CLOCK_PAUSED: AtomicU32 = AtomicU32::new(0);

/// The millisecond the clock was started at.
static CLOCK_START_MS: AtomicU64 = AtomicU64::new(0);

/// The millisecond the current pause began at.
static CLOCK_PAUSE_MS: AtomicU64 = AtomicU64::new(0);

/// The milliseconds spent paused so far.
static CLOCK_PAUSED_TOTAL_MS: AtomicU64 = AtomicU64::new(0);

/// The milliseconds since the clock's baseline was taken, on a monotonic
/// clock.
///
/// The fixture has flag bit 0 clear, so it keeps its own time instead of
/// taking it from an output plugin (S09 §3.7). The baseline is taken once,
/// at the first call, so every later reading is relative to it.
fn now_ms() -> u64 {
    static BASE: OnceLock<Instant> = OnceLock::new();
    BASE.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Start the clock at `play`.
fn start_clock() {
    CLOCK_START_MS.store(now_ms(), Ordering::SeqCst);
    CLOCK_PAUSED_TOTAL_MS.store(0, Ordering::SeqCst);
    CLOCK_PAUSE_MS.store(0, Ordering::SeqCst);
    CLOCK_PAUSED.store(0, Ordering::SeqCst);
}

/// Freeze the clock.
///
/// The pause instant is stored before the flag is raised, so a position read
/// never sees a pause without the instant it began at.
fn pause_clock() {
    if CLOCK_PAUSED.load(Ordering::SeqCst) == 0 {
        CLOCK_PAUSE_MS.store(now_ms(), Ordering::SeqCst);
        CLOCK_PAUSED.store(1, Ordering::SeqCst);
    }
}

/// Run the clock again, remembering the time spent paused.
fn resume_clock() {
    if CLOCK_PAUSED.load(Ordering::SeqCst) == 1 {
        let paused_for = now_ms().saturating_sub(CLOCK_PAUSE_MS.load(Ordering::SeqCst));
        CLOCK_PAUSED_TOTAL_MS.fetch_add(paused_for, Ordering::SeqCst);
        CLOCK_PAUSED.store(0, Ordering::SeqCst);
    }
}

/// The clock's position in milliseconds.
///
/// The position is the time since `play`, minus the time spent paused. While
/// the clock is paused it stands still: the paused stretch is subtracted from
/// the elapsed time, so both halves grow by the same amount. The clock is
/// read **once** here, so the two halves always come from the same instant
/// and the position is exactly the same on every call while paused.
fn position_ms() -> c_int {
    let now = now_ms();
    let elapsed = now.saturating_sub(CLOCK_START_MS.load(Ordering::SeqCst));
    let elapsed = elapsed.saturating_sub(CLOCK_PAUSED_TOTAL_MS.load(Ordering::SeqCst));
    let elapsed = if CLOCK_PAUSED.load(Ordering::SeqCst) != 0 {
        elapsed.saturating_sub(now.saturating_sub(CLOCK_PAUSE_MS.load(Ordering::SeqCst)))
    } else {
        elapsed
    };
    c_int::try_from(elapsed).unwrap_or(c_int::MAX)
}

// ---------------------------------------------------------------------------
// The ABI (S09 §3.1)
// ---------------------------------------------------------------------------

/// The input entry point (S09 §2, §3.1).
///
/// cdecl, no arguments, returns the plugin's static module. The name is
/// undecorated, as the ABI requires (S09 §1).
#[unsafe(no_mangle)]
pub extern "C" fn winampGetInModule2() -> *mut InModule {
    // `MODULE` is this DLL's static data. Its address is stable for the life
    // of the DLL, which is exactly the lifetime S09 §0 gives it. The raw
    // pointer keeps the static itself unborrowed.
    core::ptr::addr_of_mut!(MODULE)
}

/// `init` (S09 §3.1 offset 0x24): the host filled the members and called us.
unsafe extern "C" fn fixture_init() {
    let module = core::ptr::addr_of_mut!(MODULE);
    // SAFETY: the host filled the members in before calling us (S09 §3.5
    // step 2); reading them through the raw pointer keeps the static itself
    // unborrowed.
    let (parent, instance, analyzer, vis, dsp, set_info, out_module) = unsafe {
        (
            (*module).main_window as usize,
            (*module).dll_instance as usize,
            (*module).analyzer_init.is_some(),
            (*module).vis_feed_pcm.is_some(),
            (*module).dsp_is_active.is_some(),
            (*module).set_info.is_some(),
            !(*module).out_module.is_null(),
        )
    };
    trace(&init_line(parent, instance));

    // The window must be the host's real main window: a fixture that cannot
    // see the `Winamp v1.x` class is not testing the real thing (S09 §1).
    trace(&parent_class_line(
        window_class_name(parent) == HOST_WINDOW_CLASS,
    ));

    // The host members the plugin cannot fill itself must already be there
    // (S09 §3.5 step 2) — except `set_info`, which the host clears before
    // `init` and sets only after it returned (S09 §3.5 lines 154–155).
    trace(&host_slots_line(analyzer, vis, dsp, set_info, out_module));
}

/// `quit` (S09 §3.1 offset 0x28): the host tears the plugin down.
unsafe extern "C" fn fixture_quit() {
    trace(EVENT_QUIT);
}

/// `config` (S09 §3.1 offset 0x1C): open the configuration dialog.
///
/// The host routes it with the parent window when the player asks for the
/// plug-in's configuration. This fixture opens **no window**: it only writes
/// this trace line, so the route is proven headlessly.
unsafe extern "C" fn fixture_config(parent: *mut c_void) {
    trace(&config_line(parent as usize));
}

/// `about` (S09 §3.1 offset 0x20): open the about box.
///
/// The host has no route to it, so the trace line below appears only after a
/// call the host never makes.
unsafe extern "C" fn fixture_about(parent: *mut c_void) {
    trace(&format!("{EVENT_ABOUT}parent={:#x}", parent as usize));
}

/// `play` (S09 §3.1 offset 0x38): start playback.
///
/// The fixture opens no file and no device: flag bit 0 is clear, so it does
/// its own timekeeping (S09 §3.3, §3.7). It announces the stream through
/// the host callbacks the way S09 §3.5 step 3 describes.
unsafe extern "C" fn fixture_play(file: *const c_char) -> c_int {
    // SAFETY: the host hands over a NUL-terminated ANSI path (S09 §3.1
    // offset 0x38).
    let path = unsafe { read_c_string_lossy(file) };
    trace(&play_line(&path));

    let module = core::ptr::addr_of_mut!(MODULE);
    // SAFETY: the host filled the members in before it called `init`
    // (S09 §3.5 step 2).
    let (analyzer_init, vis_set_format, set_info, analyzer, vis, dsp, out_module) = unsafe {
        (
            (*module).analyzer_init,
            (*module).vis_set_format,
            (*module).set_info,
            (*module).analyzer_init.is_some(),
            (*module).vis_feed_pcm.is_some(),
            (*module).dsp_is_active.is_some(),
            !(*module).out_module.is_null(),
        )
    };

    // The host sets `set_info` only after `init` returned (S09 §3.5 line
    // 155), so by now it must be there.
    trace(&host_slots_line(
        analyzer,
        vis,
        dsp,
        set_info.is_some(),
        out_module,
    ));

    // S09 §3.5 step 3: the output is opened first — there is none here — and
    // then `analyzer_init`, `vis_set_format` and `set_info` follow.
    if let Some(init) = analyzer_init {
        trace(&analyzer_init_line(FIXTURE_LATENCY_MS, FIXTURE_SAMPLE_RATE));
        // SAFETY: the host's own callback, cdecl, two integers (S09 §3.1
        // offset 0x60).
        unsafe { init(FIXTURE_LATENCY_MS, FIXTURE_SAMPLE_RATE) };
    }
    if let Some(set_format) = vis_set_format {
        trace(&vis_set_format_line(FIXTURE_SAMPLE_RATE, FIXTURE_CHANNELS));
        // SAFETY: the host's own callback, cdecl, two integers (S09 §3.1
        // offset 0x80).
        unsafe { set_format(FIXTURE_SAMPLE_RATE, FIXTURE_CHANNELS) };
    }
    if let Some(info) = set_info {
        trace(&set_info_line(
            FIXTURE_BITRATE_KBPS,
            FIXTURE_SAMPLE_RATE_KHZ,
            FIXTURE_CHANNELS,
            0,
        ));
        // SAFETY: the host's own callback, cdecl, four integers (S09 §3.1
        // offset 0x90).
        unsafe {
            info(
                FIXTURE_BITRATE_KBPS,
                FIXTURE_SAMPLE_RATE_KHZ,
                FIXTURE_CHANNELS,
                0,
            )
        };
    }

    start_clock();
    0
}

/// `stop` (S09 §3.1 offset 0x48): stop and unload the stream.
unsafe extern "C" fn fixture_stop() {
    trace(EVENT_STOP);
    let module = core::ptr::addr_of_mut!(MODULE);
    // SAFETY: the host filled the member in before it called `init`
    // (S09 §3.5 step 2).
    let deinit = unsafe { (*module).analyzer_deinit };
    if let Some(deinit) = deinit {
        trace(EVENT_ANALYZER_DEINIT);
        // SAFETY: the host's own callback, cdecl, no arguments (S09 §3.1
        // offset 0x64).
        unsafe { deinit() };
    }
}

/// `pause` (S09 §3.1 offset 0x3C): pause the clock.
unsafe extern "C" fn fixture_pause() {
    pause_clock();
    trace(&paused_line(true));
}

/// `unpause` (S09 §3.1 offset 0x40): run the clock again.
unsafe extern "C" fn fixture_unpause() {
    resume_clock();
    trace(&paused_line(false));
}

/// `is_paused` (S09 §3.1 offset 0x44): 1 while the clock is paused.
unsafe extern "C" fn fixture_is_paused() -> c_int {
    c_int::from(CLOCK_PAUSED.load(Ordering::SeqCst) != 0)
}

/// `get_length` (S09 §3.1 offset 0x4C): the track length in milliseconds.
unsafe extern "C" fn fixture_get_length() -> c_int {
    FIXTURE_LENGTH_MS
}

/// `get_position` (S09 §3.1 offset 0x50): the position in milliseconds.
unsafe extern "C" fn fixture_get_position() -> c_int {
    position_ms()
}

/// `seek` (S09 §3.1 offset 0x54): the host has no seek path into a classic
/// plugin yet, so this only records that it was called.
unsafe extern "C" fn fixture_seek(ms: c_int) {
    trace(&format!("{EVENT_SEEK}ms={ms}"));
}

/// `set_volume` (S09 §3.1 offset 0x58): the host has no path from the
/// player's volume to a classic plugin yet, so this only records the call.
unsafe extern "C" fn fixture_set_volume(volume: c_int) {
    trace(&format!("{EVENT_SET_VOLUME}value={volume}"));
}

/// `set_pan` (S09 §3.1 offset 0x5C): the host has no path from the player's
/// balance to a classic plugin yet, so this only records the call.
unsafe extern "C" fn fixture_set_pan(pan: c_int) {
    trace(&format!("{EVENT_SET_PAN}value={pan}"));
}

/// `eq_set` (S09 §3.1 offset 0x8C): the host has no path from the player's
/// equalizer to a classic plugin yet, so this only records the call.
unsafe extern "C" fn fixture_eq_set(on: c_int, _bands: *mut c_char, preamp: c_int) {
    trace(&format!("{EVENT_EQ_SET}on={on} preamp={preamp}"));
}

/// `get_file_info` (S09 §3.1 offset 0x2C): fill the title and the length.
///
/// The host has no module-selection path yet, so the trace line below appears
/// only after a call the host never makes.
unsafe extern "C" fn fixture_get_file_info(
    file: *const c_char,
    title: *mut c_char,
    length_ms: *mut c_int,
) {
    // SAFETY: the host hands over a NUL-terminated ANSI path.
    let path = unsafe { read_c_string_lossy(file) };
    trace(&format!("{EVENT_FILE_INFO}file={path}"));
    if !title.is_null() {
        // The title buffer holds 2048 bytes (S09 §3.1 offset 0x2C).
        let text = b"Rustamp in fixture\0";
        // SAFETY: the caller guarantees a buffer of at least 2048 bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(text.as_ptr().cast::<c_char>(), title, text.len())
        };
    }
    if !length_ms.is_null() {
        // SAFETY: the caller guarantees a writable int pointer.
        unsafe { *length_ms = FIXTURE_LENGTH_MS };
    }
}

/// `info_box` (S09 §3.1 offset 0x30): show the file-info dialog.
///
/// The host has no file-info path yet, so the trace line below appears only
/// after a call the host never makes. The answer is 1: "unchanged"
/// (S09 §3.1 offset 0x30).
unsafe extern "C" fn fixture_info_box(file: *const c_char, _parent: *mut c_void) -> c_int {
    // SAFETY: the host hands over a NUL-terminated ANSI path.
    let path = unsafe { read_c_string_lossy(file) };
    trace(&format!("{EVENT_INFO_BOX}file={path}"));
    1
}

/// `is_our_file` (S09 §3.1 offset 0x34): the host asks every module before
/// it matches the extension lists (S09 §3.5 step 1).
///
/// The host has no module-selection path yet, so the trace line below appears
/// only after a call the host never makes. The fixture claims nothing.
unsafe extern "C" fn fixture_is_our_file(name: *const c_char) -> c_int {
    // SAFETY: the host hands over a NUL-terminated ANSI name.
    let name = unsafe { read_c_string_lossy(name) };
    trace(&format!("{EVENT_IS_OUR_FILE}name={name}"));
    0
}

// ---------------------------------------------------------------------------
// Evidence
// ---------------------------------------------------------------------------

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
