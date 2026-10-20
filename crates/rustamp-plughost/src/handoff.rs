//! The input → output hand-over (S09 §3.5, §3.6, §3.7).
//!
//! A classic input plugin that routes its audio through a Winamp output
//! plugin (flag bit 0, S09 §3.3) does not reach the output on its own: the
//! host hands it the output module first. S09 fixes the whole exchange:
//!
//! * **Before `play`** the host sets the input module's `out_module` member
//!   (S09 §3.1 offset 0x94) to the selected output module and applies the
//!   host's volume and pan to that output (S09 §3.5 step 2).
//! * **During `play`** the *input plugin* drives the output: it opens the
//!   output, writes into it, waits for `is_playing` to go false and closes it
//!   (S09 §3.5 steps 3–5, §3.7, §4.2 step 3). The host calls none of those
//!   callbacks — the output's clock is the plugin's clock (S09 §3.7).
//! * **At the end of the track** the input plugin's decode thread *posts* the
//!   end-of-track notification `0x402` to the main window with both
//!   parameters zero (S09 §3.6). The host records it and reports it to its
//!   caller; the plugin posts it only if it was not asked to stop.
//!
//! * **A seek is one call, and the flush belongs to the plugin.** S09 §3.1
//!   offset 0x54 gives the input module a `seek(int ms)` slot, and S09 §3.6
//!   lists `seek` among the calls the host makes on the UI thread. What
//!   happens next is the plugin's: S09 §3.7 says "On seeking the plugin
//!   calls the output `flush(time_ms)`", and S09 §4.2 step 3 says the
//!   output's `pause`, `set_volume`, `set_pan` and `flush` "come from the UI
//!   or seek path **through the input plugin**". The host therefore calls
//!   the input plugin's `seek` and **no output callback at all** — a
//!   host-side flush would flush the output a second time.
//! * **A pause has the same shape.** The host calls the input module's
//!   `pause` (offset 0x3C) or `unpause` (offset 0x40); the plugin pauses the
//!   output it drives with the output's own `pause(state)` (S09 §3.7,
//!   §4.1 offset 0x38, §4.2 step 3). The host calls no output callback
//!   here either.
//! * **A stop is one call, and the closing belongs to the plugin.** S09
//!   §3.5 step 6: "User stop: the host calls `stop()`; the plugin stops its
//!   decode thread, closes the output and calls `analyzer_deinit()`". The
//!   host's whole part of a stop is that one call into the input module
//!   (S09 §3.1 offset 0x48); the output's `close` is the plugin's call
//!   (S09 §3.6, §3.7, §4.2 step 3), so the host calls no output callback
//!   here either.
//!
//! # What the host does not do here
//!
//! * It does not choose the output by itself. S09 §3.5 step 2 names the
//!   selection by the `config_outname` setting — the output plug-in's file
//!   name — with a fallback to the DirectSound and then the waveOut output.
//!   That setting does not exist yet (S12 owns it) and this host has no stock
//!   outputs, so the *caller* names the output plug-in (see
//!   [`crate::registry::PluginRegistry::select_output`]) and this module only
//!   performs the hand-over.
//! * It does not advance the playlist when the end-of-track notification
//!   arrives. S09 §3.6 says the host advances to the next item or stops; this
//!   host owns no playlist — the player does — so the notification is
//!   reported to the host's caller and nothing is invented beyond that.
//! * It does not flush or pause the output itself. S09 §3.7 and §4.2 step 3
//!   put the output's `flush` and `pause` on the input plugin's side of the
//!   hand-over: the plugin seeks and flushes, the plugin pauses and resumes.
//!   The host's part of a seek is the input module's `seek` call (S09 §3.1
//!   offset 0x54) and nothing else.
//! * It does not forward the player's levels through the input module's own
//!   `set_volume` / `set_pan` slots (S09 §3.1 offsets 0x58 and 0x5C). S09
//!   §3.5 step 2 has the host apply the levels to the **output**, and that
//!   is what this host does; S09 §3.1's "normally forwarded to the output
//!   module" describes what the plugin does when something calls its slot.
//!   Whether the host must *also* call those slots when the user changes the
//!   volume later is not settled by S09 — a gap, not a decision.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};

use rustamp_plugin_abi::PluginKind;
use rustamp_plugin_abi::common::IN_FLAG_USES_OUTPUT;
use rustamp_plugin_abi::input::InModule;
use rustamp_plugin_abi::output::OutModule;

/// The end-of-track notification an input plugin's decode thread posts to the
/// main window (S09 §3.6): `WM_USER + 2`, public name `WM_WA_MPEG_EOF`, both
/// parameters zero.
pub const END_OF_TRACK_MESSAGE: u32 = rustamp_plugin_abi::common::WM_WA_MPEG_EOF;

/// Is this window message the end-of-track notification? (S09 §3.6)
pub fn is_end_of_track_message(msg: u32) -> bool {
    msg == END_OF_TRACK_MESSAGE
}

/// The end-of-track notifications that arrived and were not reported yet.
///
/// The count is process-wide because the notification is posted to the one
/// host window (S09 §3.6) and taken by the thread that pumps its messages.
static PENDING_END_OF_TRACK: AtomicU32 = AtomicU32::new(0);

/// Handle one window message that is not a classic IPC query (S09 §3.6).
///
/// The host window procedure calls this for every message that is not
/// `WM_WA_IPC`. The end-of-track notification an input plugin posts is
/// recorded here, so the host's caller can be told about it.
pub(crate) fn on_window_message(msg: u32) {
    if is_end_of_track_message(msg) {
        PENDING_END_OF_TRACK.fetch_add(1, Ordering::SeqCst);
    }
}

/// Take the end-of-track notifications that arrived and were not reported yet.
///
/// The host reports every notification exactly once: the count is drained
/// here, so a second call reports nothing until another notification
/// arrives.
pub fn take_end_of_track() -> u32 {
    PENDING_END_OF_TRACK.swap(0, Ordering::SeqCst)
}

/// Why the host refused to hand an output to an input plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffError {
    /// No plugin carries this id.
    UnknownPlugin(u32),
    /// The plugin is not of the kind this call needs.
    WrongKind {
        /// The kind the call needs.
        expected: PluginKind,
        /// The kind the loaded plugin has.
        actual: PluginKind,
    },
    /// The plugin is not live: `quit` already ran, so nothing was called.
    NotInitialized,
    /// The input plugin routes its audio through a Winamp output plugin
    /// (flag bit 0, S09 §3.3) and the host has no output to give it
    /// (S09 §3.5 step 2: "if none exists, show an error and abort").
    NoOutputSelected,
    /// The plugin's module pointer is null (S09 §3.1, §4.1).
    NullModule,
    /// The plugin supplied no such callback. S09 requires no plugin to
    /// provide one (S09 §0), so its absence is a fact about that plugin.
    MissingCallback(&'static str),
    /// The plugin's own `play` refused (S09 §3.1 offset 0x38): 0 is success,
    /// -1 is "file not found", any other nonzero is another error that stops
    /// the playlist advance.
    PlayRefused(i32),
}

impl fmt::Display for HandoffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPlugin(id) => write!(f, "no plugin with id {id} is loaded"),
            Self::WrongKind { expected, actual } => {
                write!(
                    f,
                    "expected a {expected:?} plugin, but the loaded one is a {actual:?} plugin"
                )
            }
            Self::NotInitialized => f.write_str("the plugin is not live; nothing was called"),
            Self::NoOutputSelected => {
                f.write_str("the input plugin needs an output plugin and none is selected")
            }
            Self::NullModule => {
                f.write_str("the plugin's module pointer is null; nothing was called")
            }
            Self::MissingCallback(name) => {
                write!(
                    f,
                    "the plugin supplied no '{name}' callback; nothing was called"
                )
            }
            Self::PlayRefused(code) => write!(f, "the plugin's play refused with code {code}"),
        }
    }
}

impl std::error::Error for HandoffError {}

/// A refused input-plugin call is a refused hand-over call.
///
/// The input bridge ([`crate::input`]) names the kind it wanted; every route
/// that reaches an input plugin wants an input plugin, so the expected kind
/// is fixed here.
#[cfg(any(windows, test))]
impl From<crate::input::InputError> for HandoffError {
    fn from(error: crate::input::InputError) -> Self {
        match error {
            crate::input::InputError::PlayRefused(code) => Self::PlayRefused(code),
            crate::input::InputError::NotInitialized => Self::NotInitialized,
            crate::input::InputError::NullModule => Self::NullModule,
            crate::input::InputError::MissingCallback(name) => Self::MissingCallback(name),
            crate::input::InputError::WrongKind(actual) => Self::WrongKind {
                expected: PluginKind::Input,
                actual,
            },
        }
    }
}

/// Does this input plugin route its audio through a Winamp output plugin?
///
/// Flag bit 0 of the `flags` member (S09 §3.1 offset 0x18, §3.3). Only such a
/// plugin is handed an output module (S09 §3.5 step 2, §3.7).
pub(crate) fn uses_output_plugin(input: &InModule) -> bool {
    input.flags & IN_FLAG_USES_OUTPUT != 0
}

/// Hand the selected output to the input plugin (S09 §3.5 step 2, §3.7).
///
/// The output module pointer is written into the input module's `out_module`
/// member (S09 §3.1 offset 0x94) and the host's volume and pan are applied to
/// the output — both **before** `play` runs, so the plugin finds the output it
/// must use and the output already carries the player's levels.
///
/// The host drives **no** output callback: `open`, `can_write`, `write`,
/// `is_playing` and `close` belong to the input plugin's decode thread
/// (S09 §3.5 steps 3–5, §3.7, §4.2 step 3).
///
/// The volume is `0..=255` and the pan the player's balance, which the host
/// passes on unchanged — the same way the host's IPC surface maps the
/// balance onto the pan value (see [`crate::state_bridge`]). Both are inside
/// the ranges the output expects (S09 §4.1 offsets 0x3C and 0x40).
///
/// # Safety
///
/// `input` must point at a live `In_Module` and `output` at a live
/// `Out_Module`, both in the mapped plugins' static data (S09 §0).
pub(crate) unsafe fn hand_off_output(
    input: *mut InModule,
    output: *mut OutModule,
    volume: u8,
    pan: i8,
) -> Result<(), HandoffError> {
    if input.is_null() || output.is_null() {
        return Err(HandoffError::NullModule);
    }
    // SAFETY: the caller guarantees a live input module.
    unsafe {
        (*input).out_module = output;
    }
    // S09 §3.5 step 2: "then applies volume and pan".
    // SAFETY: the caller guarantees a live output module; a missing slot is
    // simply not called (S09 §0).
    unsafe {
        if let Some(set_volume) = (*output).set_volume {
            set_volume(i32::from(volume));
        }
        if let Some(set_pan) = (*output).set_pan {
            set_pan(i32::from(pan));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod doubles {
    //! Scripted input and output plugins for the hand-over tests.
    //!
    //! They are not mocks of the host: they are real `In_Module` and
    //! `Out_Module` structures with real cdecl callbacks — the shape a mapped
    //! DLL's static data has (S09 §0). The scripted input plugin drives the
    //! output exactly as S09 §3.5 steps 3–5 describe, flushes it on a seek
    //! (S09 §3.7) and pauses it (S09 §3.7, §4.1 offset 0x38); the scripted
    //! output plugin records every call in order, naming the caller: the
    //! host or the input plugin's own code.

    use std::cell::{Cell, RefCell};
    use std::ffi::{c_char, c_int};

    use rustamp_plugin_abi::common::{IN_FLAG_USES_OUTPUT, IN_VERSION, OUT_VERSION};
    use rustamp_plugin_abi::input::InModule;
    use rustamp_plugin_abi::output::OutModule;

    thread_local! {
        /// The calls the doubles recorded, in order.
        static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    /// Who calls the output plugin: the host itself, or the input plugin
    /// that drives the output (S09 §3.5 steps 3–5, §3.7, §4.2 step 3).
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Caller {
        /// The host's own code — today only the hand-over's levels.
        Host,
        /// The input plugin's own code.
        InputPlugin,
    }

    thread_local! {
        /// The caller of the output callbacks.
        static CALLER: Cell<Caller> = const { Cell::new(Caller::Host) };
    }

    thread_local! {
        /// The scripted input module of this thread.
        static INPUT_MODULE: Cell<*mut InModule> = const { Cell::new(core::ptr::null_mut()) };
    }

    thread_local! {
        /// Whether the scripted track ends on its own (S09 §3.5 step 5).
        static TRACK_ENDS: Cell<bool> = const { Cell::new(true) };
    }

    thread_local! {
        /// Whether the scripted output is open and still playing.
        ///
        /// The output's `is_playing` answers whether it is still producing
        /// (S09 §4.1 offset 0x38), and the plugin closes the output it
        /// drives when the track ends or when the user stops (S09 §3.1 offset
        /// 0x48, §3.5 steps 5–6).
        static OUTPUT_OPEN: Cell<bool> = const { Cell::new(false) };
    }

    /// The calls the doubles recorded, in order.
    pub(crate) fn calls() -> Vec<String> {
        CALLS.with(|calls| calls.borrow().clone())
    }

    /// Forget every recorded call.
    pub(crate) fn clear() {
        CALLS.with(|calls| calls.borrow_mut().clear());
    }

    /// Record one call.
    fn record(entry: String) {
        CALLS.with(|calls| calls.borrow_mut().push(entry));
    }

    /// The caller of an output callback: the input plugin's own code or the
    /// host.
    fn caller() -> &'static str {
        match CALLER.with(|slot| slot.get()) {
            Caller::Host => "host",
            Caller::InputPlugin => "input",
        }
    }

    /// Run `body` as the input plugin's own code, so the output sees the
    /// plugin as its caller (S09 §3.7).
    fn as_input_plugin<R>(body: impl FnOnce() -> R) -> R {
        let previous = CALLER.with(|slot| slot.replace(Caller::InputPlugin));
        let result = body();
        CALLER.with(|slot| slot.set(previous));
        result
    }

    /// The output the scripted input plugin drives, if the host handed it one
    /// (S09 §3.1 offset 0x94).
    fn output_of_input() -> *mut OutModule {
        let module = INPUT_MODULE.with(|slot| slot.get());
        if module.is_null() {
            return core::ptr::null_mut();
        }
        // SAFETY: the scripted input module lives in the test's static data.
        unsafe { (*module).out_module }
    }

    /// What the scripted input plugin offers (S09 §3.1).
    #[derive(Clone, Copy)]
    pub(crate) struct InputScript {
        /// Flag bit 0 (S09 §3.3): the plugin routes its audio through a Winamp
        /// output plugin, so the host hands it one.
        pub uses_output: bool,
        /// The plugin supplies a `seek` slot (S09 §3.1 offset 0x54). S09
        /// requires no plugin to provide one (S09 §0), so its absence is a
        /// fact about that plugin.
        pub has_seek: bool,
        /// The plugin supplies a `stop` slot (S09 §3.1 offset 0x48). S09
        /// requires no plugin to provide one (S09 §0), so its absence is a
        /// fact about that plugin.
        pub has_stop: bool,
        /// The scripted track ends on its own right away: the plugin closes
        /// the output once the output stops playing (S09 §3.5 steps 4–5).
        /// With the flag clear the track keeps playing, so the output stays
        /// open until the plugin's `stop` closes it (S09 §3.1 offset 0x48,
        /// §3.5 step 6).
        pub track_ends: bool,
    }

    impl Default for InputScript {
        fn default() -> Self {
            Self {
                uses_output: true,
                has_seek: true,
                has_stop: true,
                track_ends: true,
            }
        }
    }

    /// A scripted input module (S09 §3.1).
    ///
    /// `script` says what the plugin offers: whether it routes its audio
    /// through an output plugin (flag bit 0, S09 §3.3), whether it supplies a
    /// `seek` slot (S09 §3.1 offset 0x54) and a `stop` slot (S09 §3.1 offset
    /// 0x48), and whether its track ends on its own (S09 §3.5 step 5).
    pub(crate) fn input_module(script: InputScript) -> *mut InModule {
        // Leaked on purpose: the module must outlive the registry that holds a
        // raw pointer to it, exactly as a mapped DLL's static data does.
        let module: &'static mut InModule = Box::leak(Box::new(InModule {
            version: IN_VERSION,
            description: c"scripted in".as_ptr(),
            flags: if script.uses_output {
                IN_FLAG_USES_OUTPUT
            } else {
                0
            },
            init: Some(in_init),
            quit: Some(in_quit),
            config: Some(in_config),
            play: Some(in_play),
            stop: script.has_stop.then_some(in_stop),
            pause: Some(in_pause),
            unpause: Some(in_unpause),
            is_paused: Some(in_is_paused),
            get_length: Some(in_get_length),
            get_position: Some(in_get_position),
            seek: script.has_seek.then_some(in_seek),
            ..InModule::default()
        }));
        let module = module as *mut InModule;
        INPUT_MODULE.with(|slot| slot.set(module));
        TRACK_ENDS.with(|ends| ends.set(script.track_ends));
        // A new scripted plugin starts with a closed output.
        OUTPUT_OPEN.with(|open| open.set(false));
        module
    }

    /// A scripted output module (S09 §4.1).
    pub(crate) fn output_module() -> *mut OutModule {
        // Leaked on purpose: see `input_module`.
        let module: &'static mut OutModule = Box::leak(Box::new(OutModule {
            version: OUT_VERSION,
            description: c"scripted out".as_ptr(),
            id: 0x0001_0000,
            init: Some(out_init),
            quit: Some(out_quit),
            config: Some(out_config),
            open: Some(out_open),
            close: Some(out_close),
            write: Some(out_write),
            can_write: Some(out_can_write),
            is_playing: Some(out_is_playing),
            pause: Some(out_pause),
            set_volume: Some(out_set_volume),
            set_pan: Some(out_set_pan),
            flush: Some(out_flush),
            get_played_ms: Some(out_get_played_ms),
            get_written_ms: Some(out_get_written_ms),
            ..OutModule::default()
        }));
        module as *mut OutModule
    }

    unsafe extern "C" fn in_init() {
        record("init".to_string());
    }

    unsafe extern "C" fn in_quit() {
        record("quit".to_string());
    }

    unsafe extern "C" fn in_config(_parent: *mut core::ffi::c_void) {
        record("config".to_string());
    }

    /// The scripted input plugin's `stop` (S09 §3.1 offset 0x48).
    ///
    /// S09 §3.5 step 6: "User stop: the host calls `stop()`; the plugin stops
    /// its decode thread, closes the output and calls `analyzer_deinit()`".
    /// The closing is the plugin's own work — the host calls no output
    /// callback of its own.
    unsafe extern "C" fn in_stop() {
        record("stop".to_string());
        let out = output_of_input();
        if out.is_null() || !OUTPUT_OPEN.with(|open| open.get()) {
            return;
        }
        as_input_plugin(|| {
            // SAFETY: live output module in the test's static data.
            unsafe {
                if let Some(close) = (*out).close {
                    close();
                }
            }
        });
    }

    /// The scripted input plugin's `pause` (S09 §3.1 offset 0x3C).
    ///
    /// S09 §3.7 and §4.1 offset 0x38: the output's `pause` takes the state
    /// and returns the previous one, and the plugin — not the host — drives
    /// it (S09 §4.2 step 3).
    unsafe extern "C" fn in_pause() {
        record("pause".to_string());
        forward_pause(1);
    }

    /// The scripted input plugin's `unpause` (S09 §3.1 offset 0x40).
    unsafe extern "C" fn in_unpause() {
        record("unpause".to_string());
        forward_pause(0);
    }

    /// The scripted plugin pauses or resumes the output it drives.
    fn forward_pause(state: c_int) {
        let out = output_of_input();
        if out.is_null() {
            return;
        }
        as_input_plugin(|| {
            // SAFETY: live output module in the test's static data.
            unsafe {
                if let Some(pause) = (*out).pause {
                    pause(state);
                }
            }
        });
    }

    unsafe extern "C" fn in_is_paused() -> c_int {
        0
    }

    unsafe extern "C" fn in_get_length() -> c_int {
        30_000
    }

    unsafe extern "C" fn in_get_position() -> c_int {
        0
    }

    /// The scripted input plugin's `seek` (S09 §3.1 offset 0x54).
    ///
    /// S09 §3.7: "On seeking the plugin calls the output `flush(time_ms)`."
    /// The host's own part of the seek is the call into this function.
    unsafe extern "C" fn in_seek(time_ms: c_int) {
        record(format!("seek {time_ms}"));
        let out = output_of_input();
        if out.is_null() {
            return;
        }
        as_input_plugin(|| {
            // SAFETY: live output module in the test's static data.
            unsafe {
                if let Some(flush) = (*out).flush {
                    flush(time_ms);
                }
            }
        });
    }

    /// The scripted input plugin's `play` (S09 §3.1 offset 0x38).
    ///
    /// It drives the output the way S09 §3.5 steps 3–5 describe: `open`, then
    /// `can_write`/`write`, then `is_playing`, then `close`. The output it
    /// drives is the one the host wrote into the `out_module` member
    /// (S09 §3.7) — the plugin never learns about it any other way.
    ///
    /// With a track that does not end on its own the output stays open: the
    /// plugin holds it until the user stops (S09 §3.1 offset 0x48, §3.5
    /// step 6).
    unsafe extern "C" fn in_play(_file: *const c_char) -> c_int {
        let module = INPUT_MODULE.with(|slot| slot.get());
        // SAFETY: the test installed the module before the host called `play`.
        let out = unsafe { (*module).out_module };
        record(format!(
            "play out={}",
            if out.is_null() { "none" } else { "set" }
        ));
        if !out.is_null() {
            as_input_plugin(|| {
                // SAFETY: live output module in the test's static data.
                unsafe {
                    if let Some(open) = (*out).open {
                        open(44_100, 2, 16, 0, 0);
                    }
                    if let Some(can_write) = (*out).can_write {
                        can_write();
                    }
                    if let Some(write) = (*out).write {
                        let mut buffer = [0_u8; 16];
                        write(buffer.as_mut_ptr().cast::<c_char>(), buffer.len() as c_int);
                    }
                    if let Some(is_playing) = (*out).is_playing {
                        is_playing();
                    }
                    // S09 §3.5 step 5: the track ends when the output stops
                    // playing, and the plugin then closes the output.
                    if TRACK_ENDS.with(|ends| ends.get())
                        && let Some(close) = (*out).close
                    {
                        close();
                    }
                }
            });
        }
        0
    }

    unsafe extern "C" fn out_init() {
        record("out init".to_string());
    }

    unsafe extern "C" fn out_quit() {
        record("out quit".to_string());
    }

    unsafe extern "C" fn out_config(_parent: *mut core::ffi::c_void) {
        record("out config".to_string());
    }

    unsafe extern "C" fn out_open(
        _sample_rate: c_int,
        _channels: c_int,
        _bits: c_int,
        _buffer_ms: c_int,
        _prebuffer_ms: c_int,
    ) -> c_int {
        record(format!("open {}", caller()));
        OUTPUT_OPEN.with(|open| open.set(true));
        0
    }

    unsafe extern "C" fn out_close() {
        record(format!("close {}", caller()));
        OUTPUT_OPEN.with(|open| open.set(false));
    }

    unsafe extern "C" fn out_write(_data: *mut c_char, len: c_int) -> c_int {
        record(format!("write {} len={len}", caller()));
        0
    }

    unsafe extern "C" fn out_can_write() -> c_int {
        record(format!("can_write {}", caller()));
        8192
    }

    unsafe extern "C" fn out_is_playing() -> c_int {
        record(format!("is_playing {}", caller()));
        // S09 §4.1 offset 0x38: the output is still playing while it is
        // open.
        OUTPUT_OPEN.with(|open| c_int::from(open.get()))
    }

    unsafe extern "C" fn out_pause(state: c_int) -> c_int {
        record(format!("pause {state} {}", caller()));
        0
    }

    unsafe extern "C" fn out_set_volume(volume: c_int) {
        record(format!("set_volume {} {volume}", caller()));
    }

    unsafe extern "C" fn out_set_pan(pan: c_int) {
        record(format!("set_pan {} {pan}", caller()));
    }

    unsafe extern "C" fn out_flush(time_ms: c_int) {
        record(format!("flush {time_ms} {}", caller()));
    }

    unsafe extern "C" fn out_get_played_ms() -> c_int {
        0
    }

    unsafe extern "C" fn out_get_written_ms() -> c_int {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::doubles;
    use super::*;
    use crate::approval::ApprovalStore;
    use crate::loader::LoadedPlugin;
    use crate::registry::PluginRegistry;
    use std::path::Path;

    /// The path the host hands to the plugin (S09 §3.1 offset 0x38).
    const TRACK: &str = r"C:\music\track.mp3";

    /// A registry with a scripted input and a scripted output plugin.
    fn registry_with(uses_output: bool) -> (PluginRegistry, u32, u32) {
        registry_with_script(doubles::InputScript {
            uses_output,
            ..doubles::InputScript::default()
        })
    }

    /// A registry with a scripted input plugin of the given shape and a
    /// scripted output plugin.
    fn registry_with_script(script: doubles::InputScript) -> (PluginRegistry, u32, u32) {
        let input =
            LoadedPlugin::input_for_test("scripted_in", doubles::input_module(script), 0x1234);
        let output =
            LoadedPlugin::output_for_test("scripted_out", doubles::output_module(), true, 0x1234);
        let mut registry = PluginRegistry::new(ApprovalStore::new());
        let output_id = registry.insert_for_test(output);
        let input_id = registry.insert_for_test(input);
        (registry, input_id, output_id)
    }

    #[test]
    fn the_output_is_handed_over_before_play_and_the_plugin_drives_it() {
        doubles::clear();
        let (mut registry, input_id, output_id) = registry_with(true);

        // S09 §3.5 step 2: a plugin that routes its audio through an output
        // plugin needs one. Without a selection the host refuses and calls
        // nothing.
        assert_eq!(
            registry.play_input(input_id, Path::new(TRACK), 200, -10),
            Err(HandoffError::NoOutputSelected)
        );
        assert!(
            doubles::calls().is_empty(),
            "a refused hand-over must call nothing: {:?}",
            doubles::calls()
        );

        registry
            .select_output(output_id)
            .expect("the output plug-in must be selectable");

        registry
            .play_input(input_id, Path::new(TRACK), 200, -10)
            .expect("the hand-over and the play must succeed");

        assert_eq!(
            doubles::calls(),
            vec![
                // The host applies the levels to the output before `play`
                // (S09 §3.5 step 2).
                "set_volume host 200",
                "set_pan host -10",
                // The plugin finds the output in `out_module` and drives it
                // (S09 §3.5 steps 3–5, §3.7). Every playback call comes from
                // the plugin, never from the host.
                "play out=set",
                "open input",
                "can_write input",
                "write input len=16",
                "is_playing input",
                "close input",
            ]
        );
    }

    #[test]
    fn a_plugin_without_the_output_flag_gets_no_output() {
        doubles::clear();
        let (mut registry, input_id, output_id) = registry_with(false);
        registry
            .select_output(output_id)
            .expect("the output plug-in must be selectable");

        registry
            .play_input(input_id, Path::new(TRACK), 200, -10)
            .expect("the play must succeed");

        // S09 §3.7: without flag bit 0 the plugin supplies its own volume,
        // pan and timekeeping, so the host hands it nothing.
        assert_eq!(doubles::calls(), vec!["play out=none"]);
    }

    #[test]
    fn a_seek_reaches_the_input_plugin_and_the_plugin_flushes_the_output() {
        doubles::clear();
        let (mut registry, input_id, output_id) = registry_with(true);
        registry
            .select_output(output_id)
            .expect("the output plug-in must be selectable");
        registry
            .play_input(input_id, Path::new(TRACK), 200, -10)
            .expect("the hand-over and the play must succeed");
        doubles::clear();

        registry
            .seek_input(input_id, 12_345)
            .expect("the seek must reach the plugin");

        assert_eq!(
            doubles::calls(),
            vec![
                // The host's whole part of a seek is the input plugin's
                // `seek` (S09 §3.1 offset 0x54, §3.6).
                "seek 12345",
                // The plugin flushes the output at the new time itself
                // (S09 §3.7). The host must not flush it as well.
                "flush 12345 input",
            ],
            "the output must be flushed once, and by the plugin"
        );
    }

    #[test]
    fn the_host_calls_no_output_callback_on_the_seek_and_pause_path() {
        // S09 §3.7: "On seeking the plugin calls the output `flush(time_ms)`."
        // S09 §4.2 step 3: the output's `pause`, `set_volume`, `set_pan` and
        // `flush` come "from the UI or seek path through the input plugin".
        // A host-side flush or pause would drive the output a second time.
        doubles::clear();
        let (mut registry, input_id, output_id) = registry_with(true);
        registry
            .select_output(output_id)
            .expect("the output plug-in must be selectable");
        registry
            .play_input(input_id, Path::new(TRACK), 200, -10)
            .expect("the hand-over and the play must succeed");
        doubles::clear();

        registry
            .seek_input(input_id, 1_000)
            .expect("the seek must reach the plugin");
        registry
            .pause_input(input_id)
            .expect("the pause must reach the plugin");
        registry
            .unpause_input(input_id)
            .expect("the resume must reach the plugin");

        let calls = doubles::calls();
        assert!(
            !calls.iter().any(|call| call.ends_with(" host")),
            "no output callback may come from the host here: {calls:?}"
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.starts_with("flush"))
                .count(),
            1,
            "the output must be flushed exactly once: {calls:?}"
        );
    }

    #[test]
    fn a_seek_on_a_plugin_without_an_output_flushes_nothing() {
        // S09 §3.7: without flag bit 0 the plugin supplies its own volume,
        // pan and timekeeping and drives no output — so there is no output
        // to flush.
        doubles::clear();
        let (mut registry, input_id, output_id) = registry_with(false);
        registry
            .select_output(output_id)
            .expect("the output plug-in must be selectable");
        registry
            .play_input(input_id, Path::new(TRACK), 200, -10)
            .expect("the play must succeed");
        doubles::clear();

        registry
            .seek_input(input_id, 1_000)
            .expect("the seek must reach the plugin");

        assert_eq!(doubles::calls(), vec!["seek 1000"]);
    }

    #[test]
    fn a_seek_is_refused_when_there_is_no_input_plugin_to_seek() {
        doubles::clear();
        let (registry, input_id, output_id) = registry_with(true);

        // An id no plugin carries.
        assert_eq!(
            registry.seek_input(9_999, 0),
            Err(HandoffError::UnknownPlugin(9_999))
        );
        // A plugin of another kind: the host must not reach into its slots.
        assert_eq!(
            registry.seek_input(output_id, 0),
            Err(HandoffError::WrongKind {
                expected: PluginKind::Input,
                actual: PluginKind::Output,
            })
        );
        // The input plugin is reachable, so this call must reach it.
        assert_eq!(registry.seek_input(input_id, 0), Ok(()));
        assert_eq!(
            doubles::calls(),
            vec!["seek 0"],
            "only the input plugin's own seek may have been called"
        );
    }

    #[test]
    fn a_seek_is_refused_when_the_plugin_has_no_seek_slot() {
        // S09 §0: a callback slot may be null and a null slot is never called
        // through. The absence of a `seek` slot is a fact about that plugin
        // (S09 §3.1 offset 0x54), never a verdict on its validity.
        doubles::clear();
        let (registry, input_id, _output_id) = registry_with_script(doubles::InputScript {
            has_seek: false,
            ..doubles::InputScript::default()
        });

        assert_eq!(
            registry.seek_input(input_id, 1_000),
            Err(HandoffError::MissingCallback("seek"))
        );
        assert!(
            doubles::calls().is_empty(),
            "a refused seek must call nothing: {:?}",
            doubles::calls()
        );
    }

    #[test]
    fn a_pause_reaches_the_output_through_the_input_plugin() {
        // S09 §3.1 offsets 0x3C and 0x40: the host pauses and resumes the
        // input plugin. S09 §3.7 and §4.1 offset 0x38: the plugin pauses the
        // output it drives with the output's own `pause(state)`, which takes
        // the state and returns the previous one.
        doubles::clear();
        let (mut registry, input_id, output_id) = registry_with(true);
        registry
            .select_output(output_id)
            .expect("the output plug-in must be selectable");
        registry
            .play_input(input_id, Path::new(TRACK), 200, -10)
            .expect("the hand-over and the play must succeed");
        doubles::clear();

        registry
            .pause_input(input_id)
            .expect("the pause must reach the plugin");
        registry
            .unpause_input(input_id)
            .expect("the resume must reach the plugin");

        assert_eq!(
            doubles::calls(),
            vec![
                // The host pauses the input plugin (S09 §3.1 offset 0x3C).
                "pause",
                // The plugin pauses the output it drives (S09 §3.7,
                // §4.1 offset 0x38): 1 pauses, 0 resumes.
                "pause 1 input",
                "unpause",
                "pause 0 input",
            ]
        );
    }

    #[test]
    fn the_stop_reaches_the_input_plugin_and_the_plugin_closes_the_output() {
        // S09 §3.5 step 6: "User stop: the host calls `stop()`; the plugin
        // stops its decode thread, closes the output and calls
        // `analyzer_deinit()`". The host's whole part of a stop is that one
        // call into the input plugin (S09 §3.1 offset 0x48).
        doubles::clear();
        let (mut registry, input_id, output_id) = registry_with_script(doubles::InputScript {
            // The track keeps playing, so the output stays open until the
            // plugin's stop closes it.
            track_ends: false,
            ..doubles::InputScript::default()
        });
        registry
            .select_output(output_id)
            .expect("the output plug-in must be selectable");
        registry
            .play_input(input_id, Path::new(TRACK), 200, -10)
            .expect("the hand-over and the play must succeed");
        doubles::clear();

        registry
            .stop_input(input_id)
            .expect("the stop must reach the plugin");

        assert_eq!(
            doubles::calls(),
            vec![
                // The host calls the input plugin's `stop` (S09 §3.1 offset
                // 0x48).
                "stop",
                // The plugin closes the output it drives (S09 §3.1 offset
                // 0x48, §3.5 step 6).
                "close input",
            ]
        );
        assert!(
            !doubles::calls().iter().any(|call| call.ends_with(" host")),
            "the host must call no output callback on the stop path: {:?}",
            doubles::calls()
        );
    }

    #[test]
    fn a_stop_on_a_plugin_without_an_output_closes_nothing() {
        // S09 §3.7: without flag bit 0 the plugin drives no output, so there
        // is no output for the stop to close.
        doubles::clear();
        let (mut registry, input_id, output_id) = registry_with_script(doubles::InputScript {
            uses_output: false,
            track_ends: false,
            ..doubles::InputScript::default()
        });
        registry
            .select_output(output_id)
            .expect("the output plug-in must be selectable");
        registry
            .play_input(input_id, Path::new(TRACK), 200, -10)
            .expect("the play must succeed");
        doubles::clear();

        registry
            .stop_input(input_id)
            .expect("the stop must reach the plugin");

        assert_eq!(doubles::calls(), vec!["stop"]);
    }

    #[test]
    fn a_stop_is_refused_when_there_is_no_input_plugin_to_stop() {
        doubles::clear();
        let (registry, input_id, output_id) = registry_with(true);

        // An id no plugin carries.
        assert_eq!(
            registry.stop_input(9_999),
            Err(HandoffError::UnknownPlugin(9_999))
        );
        // A plugin of another kind: the host must not reach into its slots.
        assert_eq!(
            registry.stop_input(output_id),
            Err(HandoffError::WrongKind {
                expected: PluginKind::Input,
                actual: PluginKind::Output,
            })
        );
        // The input plugin is reachable, so this call must reach it.
        assert_eq!(registry.stop_input(input_id), Ok(()));
        assert_eq!(
            doubles::calls(),
            vec!["stop"],
            "only the input plugin's own stop may have been called"
        );
    }

    #[test]
    fn a_stop_is_refused_when_the_plugin_has_no_stop_slot() {
        // S09 §0: a callback slot may be null and a null slot is never called
        // through. The absence of a `stop` slot is a fact about that plugin
        // (S09 §3.1 offset 0x48), never a verdict on its validity.
        doubles::clear();
        let (registry, input_id, _output_id) = registry_with_script(doubles::InputScript {
            has_stop: false,
            ..doubles::InputScript::default()
        });

        assert_eq!(
            registry.stop_input(input_id),
            Err(HandoffError::MissingCallback("stop"))
        );
        assert!(
            doubles::calls().is_empty(),
            "a refused stop must call nothing: {:?}",
            doubles::calls()
        );
    }

    #[test]
    fn a_stop_is_refused_when_the_plugin_is_not_live() {
        // `quit` already ran, so there is nothing left to drive (S09 §3.5
        // line 166).
        doubles::clear();
        let (mut registry, input_id, _output_id) = registry_with(true);
        registry.quit_all();
        doubles::clear();

        assert_eq!(
            registry.stop_input(input_id),
            Err(HandoffError::NotInitialized)
        );
        assert!(
            doubles::calls().is_empty(),
            "a refused stop must call nothing: {:?}",
            doubles::calls()
        );
    }

    #[test]
    fn every_stop_reaches_the_plugin_and_the_plugin_owns_what_a_second_one_means() {
        // The host keeps no playback state: `play_input` hands the output
        // over and calls `play`, `stop_input` calls `stop` — the same shape
        // as the seek and the pause routes. S09 §3.5 step 6 says what the
        // host does on a user stop, and S09 §3.1 offset 0x48 says what the
        // plugin does with it; neither says that a stop without a play is a
        // host-side error. The host forwards it and the plugin decides.
        doubles::clear();
        let (mut registry, input_id, output_id) = registry_with_script(doubles::InputScript {
            track_ends: false,
            ..doubles::InputScript::default()
        });
        registry
            .select_output(output_id)
            .expect("the output plug-in must be selectable");
        registry
            .play_input(input_id, Path::new(TRACK), 200, -10)
            .expect("the hand-over and the play must succeed");
        doubles::clear();

        registry
            .stop_input(input_id)
            .expect("the first stop must reach the plugin");
        registry
            .stop_input(input_id)
            .expect("the second stop must reach the plugin as well");

        assert_eq!(
            doubles::calls(),
            vec![
                "stop",
                // The plugin closes the output it drives (S09 §3.5 step 6).
                "close input",
                // The second stop reaches the plugin again. The host does not
                // swallow it; the output is already closed, so nothing is
                // closed twice.
                "stop",
            ]
        );
    }

    #[test]
    fn the_output_pointer_is_written_into_the_input_module() {
        let input = doubles::input_module(doubles::InputScript::default());
        let output = doubles::output_module();
        // SAFETY: live modules in the test's static data.
        assert!(
            unsafe { (*input).out_module }.is_null(),
            "the module starts without an output (S09 §3.1 offset 0x94)"
        );
        // SAFETY: live modules in the test's static data.
        unsafe { hand_off_output(input, output, 255, 0) }.expect("the hand-over must succeed");
        // SAFETY: live modules in the test's static data.
        assert_eq!(
            unsafe { (*input).out_module },
            output,
            "the host must write the output into the input module (S09 §3.1 offset 0x94)"
        );
    }

    #[test]
    fn only_an_output_plugin_can_be_selected() {
        let (mut registry, input_id, _output_id) = registry_with(true);
        assert_eq!(
            registry.select_output(input_id),
            Err(HandoffError::WrongKind {
                expected: PluginKind::Output,
                actual: PluginKind::Input,
            })
        );
        assert_eq!(
            registry.select_output(9_999),
            Err(HandoffError::UnknownPlugin(9_999))
        );
        assert_eq!(registry.selected_output(), None);
    }

    #[test]
    fn an_end_of_track_notification_is_reported_once() {
        // S09 §3.6: the input plugin's decode thread posts 0x402 to the main
        // window with both parameters zero.
        assert_eq!(END_OF_TRACK_MESSAGE, 0x402);
        assert!(is_end_of_track_message(END_OF_TRACK_MESSAGE));
        // Drain whatever an earlier run left behind.
        let _ = take_end_of_track();

        on_window_message(END_OF_TRACK_MESSAGE);
        assert_eq!(
            take_end_of_track(),
            1,
            "the end-of-track notification must reach the host's caller"
        );
        assert_eq!(
            take_end_of_track(),
            0,
            "the notification must be reported exactly once"
        );

        // Another window message is not an end-of-track notification.
        on_window_message(rustamp_plugin_abi::common::WM_USER);
        assert_eq!(take_end_of_track(), 0);
    }
}
