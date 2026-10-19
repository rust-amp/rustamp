//! A clean-room **input** (`in_`) plugin fixture that routes its audio
//! through a Winamp **output** plugin: the hand-over fixture.
//!
//! # What this is
//!
//! A separate `cdylib` artifact — never a library linked into the test or the
//! host — that implements the input plugin ABI of `specs/S09-plugin-abi.md`
//! §3 and, in particular, the **hand-over** of S09 §3.5 step 2 and §3.7:
//!
//! * **flag bit 0 is set** (S09 §3.3): the fixture routes its audio through a
//!   Winamp output plug-in, so the host must hand it the selected output
//!   module before `play` (S09 §3.1 offset 0x94);
//! * `init` records the members the host filled in — the main window, the
//!   plugin's own module handle and the host callbacks — and notes that
//!   `out_module` is still empty, because the hand-over happens later, before
//!   `play` (S09 §3.5 step 2);
//! * `play` **drives the output plug-in itself**, the way S09 §3.5 steps 3–5
//!   and §3.7 describe: `open`, then `can_write`/`write` for a few silent
//!   frames, then `is_playing`, then the output's clock, then `close`. The
//!   host calls none of those callbacks — the output's clock is the plugin's
//!   clock (S09 §3.7);
//! * `play` **posts** the end-of-track notification `0x402` to the host's main
//!   window with both parameters zero, the way S09 §3.6 demands;
//! * `get_position` takes the position from the output plug-in, not from a
//!   clock of the fixture's own (S09 §3.7);
//! * `stop` and `quit` record the teardown (S09 §3.5 step 6, line 166).
//!
//! The fixture does **not** announce its stream to the host's analyzer,
//! visualization and `set_info` callbacks: that path has its own fixture
//! (`examples/in_rustamp_fixture.rs` with `tests/in_acceptance.rs`), and
//! repeating it here would only couple this test to the *output* fixture's
//! latency constant, which belongs to a different protocol.
//!
//! # What this is not
//!
//! * It is not a mock: the export is resolved by name through the OS loader
//!   and called through the real cdecl ABI, and every callback is a real
//!   function in a real DLL (P3 §7 lists "mocked/statically linked callbacks"
//!   as a false pass).
//! * It is not a third-party plugin and proves nothing about one. It is the
//!   instrumented fixture the acceptance test needs: our own DLL, built only
//!   from S09.
//! * **It opens no window and no audio device.** The only thing it writes to
//!   is the output plug-in the host handed it — in the acceptance test that
//!   is our own `out_` fixture, which opens no device either.
//!
//! The sibling fixture `examples/in_rustamp_fixture.rs` is the *other* half of
//! this story: flag bit 0 clear, so it keeps its own clock and is handed no
//! output at all (S09 §3.7). Between them the two fixtures cover both sides
//! of S09 §3.3.
//!
//! # Evidence
//!
//! Every step the fixture takes is appended to its own trace file in the
//! process temp directory, written **from inside the plugin**. The trace is
//! plugin-side evidence: the host and the test runner cannot forge it.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004), like the rest of this package.

#![cfg(windows)]

#[path = "../tests/common/in_handoff_protocol.rs"]
mod in_handoff_protocol;

use in_handoff_protocol::*;

use std::ffi::{c_char, c_int, c_void};

use rustamp_plugin_abi::common::{
    IN_FLAG_USES_OUTPUT, IN_VERSION, WM_WA_MPEG_EOF, read_c_string_lossy,
};
use rustamp_plugin_abi::input::InModule;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, PostMessageW};

/// The window class the ABI fixes for the host's main window (S09 §1, §8).
const HOST_WINDOW_CLASS: &str = "Winamp v1.x";

/// The extension list and the display name the fixture publishes
/// (S09 §3.1 offset 0x10, §3.4).
///
/// The list is a sequence of string pairs ended by an empty string: the
/// extension list `mp3`, then the display name, then the empty string. The
/// host reads it only to match a file against a module, and it has no
/// module-selection path yet (S09 §3.5 step 1), so nothing checks this list.
const FIXTURE_FILE_EXTENSIONS: &[u8] = b"mp3\0Rustamp in handoff fixture (MP3)\0\0";

/// The track length the fixture reports (S09 §3.1 offset 0x4C).
const FIXTURE_LENGTH_MS: c_int = 30_000;

/// The trace line the fixture writes when the host stops the stream
/// (S09 §3.5 step 6).
///
/// The host has no path that stops a plugin on its own, so this line appears
/// only after a stop request the host's caller has to make.
const EVENT_STOP: &str = "playback stopped";

/// The trace line the fixture writes when it finds no output plug-in.
///
/// S09 §3.5 step 2 says the host shows an error and aborts when no output
/// plug-in exists, so `play` is never called in that case. The line records
/// that the host broke that rule.
const EVENT_OUT_MISSING: &str = "out missing";

/// The fixture's static module (S09 §3.1).
///
/// It lives in this DLL's static data, so its address is stable for as long
/// as the DLL is loaded, and the host writes the members it owns into it
/// before it calls `init` (S09 §0, §3.5 step 2).
///
/// `flags` carries **flag bit 0** (S09 §3.3): this plugin routes its audio
/// through a Winamp output plug-in, so the host hands it the selected output
/// module before `play` (S09 §3.5 step 2, §3.7). `out_module` starts empty
/// for the same reason the sibling fixture's does: the hand-over has not
/// happened yet.
static mut MODULE: InModule = InModule {
    version: IN_VERSION,
    description: FIXTURE_DESCRIPTION.as_ptr().cast::<c_char>(),
    main_window: core::ptr::null_mut(),
    dll_instance: core::ptr::null_mut(),
    file_extensions: FIXTURE_FILE_EXTENSIONS.as_ptr().cast_mut().cast::<c_char>(),
    seekable: 0,
    flags: IN_FLAG_USES_OUTPUT,
    config: Some(fixture_config),
    about: None,
    init: Some(fixture_init),
    quit: Some(fixture_quit),
    get_file_info: None,
    info_box: None,
    is_our_file: None,
    play: Some(fixture_play),
    pause: None,
    unpause: None,
    is_paused: None,
    stop: Some(fixture_stop),
    get_length: Some(fixture_get_length),
    get_position: Some(fixture_get_position),
    seek: None,
    set_volume: None,
    set_pan: None,
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
    eq_set: None,
    set_info: None,
    out_module: core::ptr::null_mut(),
};

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
    // `init` and sets only after it returned (S09 §3.5 lines 154–155), and
    // `out_module`, which the hand-over fills only before `play`
    // (S09 §3.5 step 2).
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

/// `play` (S09 §3.1 offset 0x38): start playback.
///
/// The fixture opens no file and no device: it sends its audio to the output
/// plug-in the host handed it, and that plug-in is the only thing it writes
/// to (S09 §3.3, §3.7). It drives the output the way S09 §3.5 steps 3–5
/// describe and posts the end-of-track notification S09 §3.6 asks for.
unsafe extern "C" fn fixture_play(file: *const c_char) -> c_int {
    // SAFETY: the host hands over a NUL-terminated ANSI path (S09 §3.1
    // offset 0x38).
    let path = unsafe { read_c_string_lossy(file) };
    trace(&play_line(&path));

    let module = core::ptr::addr_of_mut!(MODULE);
    // SAFETY: the host filled the members in before it called `init`
    // (S09 §3.5 step 2) and wrote the selected output into `out_module`
    // before it called `play` (S09 §3.5 step 2).
    let (out, set_info, analyzer, vis, dsp) = unsafe {
        (
            (*module).out_module,
            (*module).set_info.is_some(),
            (*module).analyzer_init.is_some(),
            (*module).vis_feed_pcm.is_some(),
            (*module).dsp_is_active.is_some(),
        )
    };
    trace(&host_slots_line(
        analyzer,
        vis,
        dsp,
        set_info,
        !out.is_null(),
    ));

    if out.is_null() {
        // S09 §3.5 step 2: with no output plug-in the host shows an error and
        // aborts, so `play` is never reached. A plugin that is called anyway
        // has nowhere to send its audio (S09 §3.7).
        trace(EVENT_OUT_MISSING);
        return 1;
    }

    // S09 §3.7: the plugin calls the output's `open` in `play` with the
    // stream's rate, channel count, bit depth and buffer/prebuffer hints, and
    // takes the maximum latency from the return value.
    // SAFETY: the host wrote a live output module into `out_module` before
    // it called `play` (S09 §3.5 step 2, §3.7).
    unsafe { open_output(out) };

    // S09 §3.5 step 4: the decode loop asks the output how much room it has,
    // writes the decoded frames into it, and asks whether it still plays
    // (S09 §3.5 step 5). The frames are silent: the fixture decodes no
    // file, it only proves that the audio path runs through the output.
    for _ in 0..FIXTURE_FRAMES {
        // SAFETY: the host wrote a live output module into `out_module`
        // before it called `play` (S09 §3.5 step 2, §3.7).
        unsafe { write_frame(out) };
    }

    // S09 §3.5 step 5: the plugin waits for the output's `is_playing` to go
    // false before it posts the end-of-track notification. The fixture has
    // written its whole — silent — track by now, so it asks once: the output
    // accounts its buffer in whole milliseconds and the fixture's silence is
    // shorter than one of them, so the answer is that nothing is left to
    // play and the track is over.
    // SAFETY: the host wrote a live output module into `out_module` before
    // it called `play` (S09 §3.5 step 2, §3.7).
    unsafe { ask_is_playing(out) };

    // S09 §3.7: the timing comes from the output — played-time for what is
    // being heard, written-time for the timestamps.
    // SAFETY: the host wrote a live output module into `out_module` before
    // it called `play` (S09 §3.5 step 2, §3.7).
    unsafe { read_output_time(out) };

    // The output is closed again. S09 §3.5 step 6 has the plugin close the
    // output when the host stops the stream; this fixture's track ended on
    // its own, so it releases the output right away.
    // SAFETY: the host wrote a live output module into `out_module` before
    // it called `play` (S09 §3.5 step 2, §3.7).
    unsafe { close_output(out) };

    // S09 §3.6: the plugin posts the end-of-track notification `0x402` to the
    // main window with both parameters zero. The plugin posts it only if it
    // was not asked to stop — this fixture reached the end of its track on
    // its own.
    let posted = post_end_of_track();
    trace(&end_of_track_line(posted));
    0
}

/// `stop` (S09 §3.1 offset 0x48): stop and unload the stream (S09 §3.5
/// step 6).
unsafe extern "C" fn fixture_stop() {
    trace(EVENT_STOP);
}

/// `get_length` (S09 §3.1 offset 0x4C): the track length in milliseconds.
unsafe extern "C" fn fixture_get_length() -> c_int {
    FIXTURE_LENGTH_MS
}

/// `get_position` (S09 §3.1 offset 0x50): the position in milliseconds.
///
/// The position comes from the output plug-in, not from a clock of the
/// fixture's own: with flag bit 0 set the output is the clock (S09 §3.7).
unsafe extern "C" fn fixture_get_position() -> c_int {
    let module = core::ptr::addr_of_mut!(MODULE);
    // SAFETY: the host wrote the selected output into `out_module` before it
    // called `play` (S09 §3.5 step 2).
    let out = unsafe { (*module).out_module };
    if out.is_null() {
        return 0;
    }
    // SAFETY: live output module in the mapped plugin's static data.
    unsafe {
        match (*out).get_played_ms {
            Some(get_played_ms) => get_played_ms(),
            None => 0,
        }
    }
}

/// Open the output plug-in (S09 §3.7).
///
/// The output's `open` reports the maximum latency the plugin has to hand
/// to the host's `analyzer_init` (S09 §3.7). This fixture does not announce
/// its stream to the analyzer — that path has its own fixture — so the
/// latency is not needed here and only the call itself is recorded.
///
/// # Safety
///
/// `out` must point at a live `Out_Module` in a mapped output plug-in's
/// static data (S09 §0, §3.7).
unsafe fn open_output(out: *mut rustamp_plugin_abi::output::OutModule) {
    // SAFETY: the caller guarantees a live output module.
    let open = unsafe { (*out).open };
    let Some(open) = open else {
        return;
    };
    trace(&out_open_line(
        FIXTURE_SAMPLE_RATE,
        FIXTURE_CHANNELS,
        FIXTURE_BITS,
        FIXTURE_BUFFER_MS,
        FIXTURE_PREBUFFER_MS,
    ));
    // SAFETY: cdecl, five integers (S09 §4.1 offset 0x24), a live module.
    unsafe {
        open(
            FIXTURE_SAMPLE_RATE,
            FIXTURE_CHANNELS,
            FIXTURE_BITS,
            FIXTURE_BUFFER_MS,
            FIXTURE_PREBUFFER_MS,
        )
    };
}

/// Ask the output plug-in for room and write one silent frame into it
/// (S09 §3.5 step 4).
///
/// # Safety
///
/// `out` must point at a live `Out_Module` in a mapped output plug-in's
/// static data (S09 §0, §3.7).
unsafe fn write_frame(out: *mut rustamp_plugin_abi::output::OutModule) {
    // SAFETY: the caller guarantees a live output module.
    let (can_write, write) = unsafe { ((*out).can_write, (*out).write) };
    if let Some(can_write) = can_write {
        // SAFETY: cdecl, no arguments (S09 §4.1 offset 0x30), live module.
        let free = unsafe { can_write() };
        trace(&format!("{EVENT_OUT_CAN_WRITE}{free}"));
    }
    let Some(write) = write else {
        return;
    };
    // A silent 16-bit stereo frame: the fixture decodes no file, it only
    // proves that the audio path runs through the output (S09 §3.7).
    let mut frame = [0_u8; FIXTURE_FRAME_BYTES];
    // SAFETY: cdecl, a writable buffer and its length (S09 §4.1 offset
    // 0x2C), live module; the buffer outlives the call.
    unsafe { write(frame.as_mut_ptr().cast::<c_char>(), frame.len() as c_int) };
    trace(&out_write_line(frame.len() as c_int));
}

/// Ask the output plug-in whether it still plays (S09 §3.5 step 5).
///
/// # Safety
///
/// `out` must point at a live `Out_Module` in a mapped output plug-in's
/// static data (S09 §0, §3.7).
unsafe fn ask_is_playing(out: *mut rustamp_plugin_abi::output::OutModule) {
    // SAFETY: the caller guarantees a live output module.
    let is_playing = unsafe { (*out).is_playing };
    let Some(is_playing) = is_playing else {
        return;
    };
    // SAFETY: cdecl, no arguments (S09 §4.1 offset 0x34), live module.
    let playing = unsafe { is_playing() };
    trace(&format!("{EVENT_OUT_IS_PLAYING}{playing}"));
}

/// Read the output plug-in's clock (S09 §3.7).
///
/// # Safety
///
/// `out` must point at a live `Out_Module` in a mapped output plug-in's
/// static data (S09 §0, §3.7).
unsafe fn read_output_time(out: *mut rustamp_plugin_abi::output::OutModule) {
    // SAFETY: the caller guarantees a live output module.
    let (get_played_ms, get_written_ms) = unsafe { ((*out).get_played_ms, (*out).get_written_ms) };
    let Some(get_played_ms) = get_played_ms else {
        return;
    };
    // SAFETY: cdecl, no arguments (S09 §4.1 offset 0x48), live module.
    let played = unsafe { get_played_ms() };
    let written = match get_written_ms {
        Some(get_written_ms) => {
            // SAFETY: cdecl, no arguments (S09 §4.1 offset 0x4C), live module.
            unsafe { get_written_ms() }
        }
        None => 0,
    };
    trace(&format!(
        "{EVENT_OUT_TIME}played={played} written={written}"
    ));
}

/// Close the output plug-in again (S09 §3.5 step 6).
///
/// # Safety
///
/// `out` must point at a live `Out_Module` in a mapped output plug-in's
/// static data (S09 §0, §3.7).
unsafe fn close_output(out: *mut rustamp_plugin_abi::output::OutModule) {
    // SAFETY: the caller guarantees a live output module.
    let close = unsafe { (*out).close };
    let Some(close) = close else {
        return;
    };
    // SAFETY: cdecl, no arguments (S09 §4.1 offset 0x28), live module.
    unsafe { close() };
    trace(EVENT_OUT_CLOSE);
}

/// Post the end-of-track notification to the host's main window (S09 §3.6).
///
/// The notification is **posted**, never sent, and both parameters are zero.
/// It says whether the message really reached the host's message queue.
fn post_end_of_track() -> bool {
    let module = core::ptr::addr_of_mut!(MODULE);
    // SAFETY: the host filled the main window before it called `init`
    // (S09 §3.5 step 2).
    let hwnd = unsafe { (*module).main_window };
    if hwnd.is_null() {
        return false;
    }
    // SAFETY: the handle is the host's main window, the message number is
    // the end-of-track notification (S09 §3.6) and both parameters are zero.
    unsafe { PostMessageW(Some(HWND(hwnd)), WM_WA_MPEG_EOF, WPARAM(0), LPARAM(0)).is_ok() }
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
