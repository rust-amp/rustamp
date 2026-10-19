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
    //! output exactly as S09 §3.5 steps 3–5 describe, and the scripted output
    //! plugin records every call in order, naming the caller: the host or the
    //! input plugin's `play`.

    use std::cell::{Cell, RefCell};
    use std::ffi::{c_char, c_int};

    use rustamp_plugin_abi::common::{IN_FLAG_USES_OUTPUT, IN_VERSION, OUT_VERSION};
    use rustamp_plugin_abi::input::InModule;
    use rustamp_plugin_abi::output::OutModule;

    thread_local! {
        /// The calls the doubles recorded, in order.
        static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    thread_local! {
        /// 1 while the scripted input plugin is inside its `play`.
        static IN_PLAY: Cell<bool> = const { Cell::new(false) };
    }

    thread_local! {
        /// The scripted input module of this thread.
        static INPUT_MODULE: Cell<*mut InModule> = const { Cell::new(core::ptr::null_mut()) };
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

    /// The caller of an output callback: the input plugin's `play` or the host.
    fn caller() -> &'static str {
        if IN_PLAY.with(|flag| flag.get()) {
            "input"
        } else {
            "host"
        }
    }

    /// A scripted input module (S09 §3.1).
    ///
    /// `uses_output` sets flag bit 0 (S09 §3.3): such a plugin expects the
    /// host to hand it an output module.
    pub(crate) fn input_module(uses_output: bool) -> *mut InModule {
        // Leaked on purpose: the module must outlive the registry that holds a
        // raw pointer to it, exactly as a mapped DLL's static data does.
        let module: &'static mut InModule = Box::leak(Box::new(InModule {
            version: IN_VERSION,
            description: c"scripted in".as_ptr(),
            flags: if uses_output { IN_FLAG_USES_OUTPUT } else { 0 },
            init: Some(in_init),
            quit: Some(in_quit),
            config: Some(in_config),
            play: Some(in_play),
            stop: Some(in_stop),
            pause: Some(in_pause),
            unpause: Some(in_unpause),
            is_paused: Some(in_is_paused),
            get_length: Some(in_get_length),
            get_position: Some(in_get_position),
            ..InModule::default()
        }));
        let module = module as *mut InModule;
        INPUT_MODULE.with(|slot| slot.set(module));
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

    unsafe extern "C" fn in_stop() {
        record("stop".to_string());
    }

    unsafe extern "C" fn in_pause() {
        record("pause".to_string());
    }

    unsafe extern "C" fn in_unpause() {
        record("unpause".to_string());
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

    /// The scripted input plugin's `play` (S09 §3.1 offset 0x38).
    ///
    /// It drives the output the way S09 §3.5 steps 3–5 describe: `open`, then
    /// `can_write`/`write`, then `is_playing`, then `close`. The output it
    /// drives is the one the host wrote into the `out_module` member
    /// (S09 §3.7) — the plugin never learns about it any other way.
    unsafe extern "C" fn in_play(_file: *const c_char) -> c_int {
        let module = INPUT_MODULE.with(|slot| slot.get());
        // SAFETY: the test installed the module before the host called `play`.
        let out = unsafe { (*module).out_module };
        record(format!(
            "play out={}",
            if out.is_null() { "none" } else { "set" }
        ));
        IN_PLAY.with(|flag| flag.set(true));
        if !out.is_null() {
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
                if let Some(close) = (*out).close {
                    close();
                }
            }
        }
        IN_PLAY.with(|flag| flag.set(false));
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
        0
    }

    unsafe extern "C" fn out_close() {
        record(format!("close {}", caller()));
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
        0
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
        let input =
            LoadedPlugin::input_for_test("scripted_in", doubles::input_module(uses_output), 0x1234);
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
    fn the_output_pointer_is_written_into_the_input_module() {
        let input = doubles::input_module(true);
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
