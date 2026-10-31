//! The input resident: a classic `in_` plug-in as a decoder of the player.
//!
//! A classic input plug-in decodes on a thread of its own and pushes its audio
//! into an output module the host hands it (S09 sections 3.5, 3.7). The media-host
//! framework asks a decoder to be read instead ([`rustamp_mediahost::decoder`]).
//! This module joins the two: the output module the plug-in gets is our own,
//! its `write` fills a queue and the framework's `read` empties it, so the
//! player's engine, its equalizer and its output play what the plug-in decodes.
//! It runs as `rustamp-plughost32 --input-resident <dll> --approvals <store>`,
//! one process per track.
//!
//! `--input-extensions <dll> --approvals <store>` prints what the plug-in plays
//! and exits: the 64-bit player cannot map the DLL to read it itself.
//!
//! As in the DSP resident the main thread is the plug-in's UI thread, where it
//! is loaded and where `play`, `seek` and `stop` run (S09 section 3.6).
//!
//! Own choices: a queue of about a second, and a plug-in's position counted from
//! what it wrote. The title is what the plug-in says of the file.

use std::path::PathBuf;

/// The switch that selects the resident.
pub const MODE_ARG: &str = "--input-resident";
/// The switch that prints the plug-in's extensions.
pub const EXTENSIONS_ARG: &str = "--input-extensions";
/// The switch that names the approval store.
pub const APPROVALS_ARG: &str = "--approvals";

/// The arguments that start the resident for `dll`.
pub fn resident_args(dll: &std::path::Path, approvals: &std::path::Path) -> Vec<String> {
    vec![
        MODE_ARG.to_owned(),
        dll.display().to_string(),
        APPROVALS_ARG.to_owned(),
        approvals.display().to_string(),
    ]
}

/// The arguments that ask for `dll`'s extensions.
pub fn extensions_args(dll: &std::path::Path, approvals: &std::path::Path) -> Vec<String> {
    vec![
        EXTENSIONS_ARG.to_owned(),
        dll.display().to_string(),
        APPROVALS_ARG.to_owned(),
        approvals.display().to_string(),
    ]
}

/// Parses the extensions a plug-in's helper printed: `mp3;mp2` per line.
pub fn parse_extensions(output: &str) -> Vec<String> {
    let mut extensions: Vec<String> = Vec::new();
    for extension in output.lines().flat_map(|line| line.split(';')) {
        let extension = extension.trim().to_ascii_lowercase();
        if !extension.is_empty()
            && extension.chars().all(|c| c.is_ascii_alphanumeric())
            && !extensions.contains(&extension)
        {
            extensions.push(extension);
        }
    }
    extensions
}

/// The value after `mode` and the store on the command line.
pub(crate) fn parse_args(args: &[String], mode: &str) -> Option<(PathBuf, PathBuf)> {
    let dll = args.iter().position(|arg| arg == mode)?;
    let store = args.iter().position(|arg| arg == APPROVALS_ARG)?;
    Some((
        PathBuf::from(args.get(dll + 1)?),
        PathBuf::from(args.get(store + 1)?),
    ))
}

/// One write of a plug-in as samples, by its bit depth.
fn samples_of(bytes: &[u8], bits: i32) -> Vec<f32> {
    match bits {
        8 => bytes
            .iter()
            .map(|byte| (f32::from(*byte) - 128.0) / 128.0)
            .collect(),
        24 => bytes
            .as_chunks::<3>()
            .0
            .iter()
            .map(|[a, b, c]| i32::from_le_bytes([0, *a, *b, *c]) as f32 / 2_147_483_648.0)
            .collect(),
        32 => bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|sample| i32::from_le_bytes(*sample) as f32 / 2_147_483_648.0)
            .collect(),
        _ => bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|sample| f32::from(i16::from_le_bytes(*sample)) / 32768.0)
            .collect(),
    }
}

#[cfg(windows)]
pub use imp::{print_extensions, run};

#[cfg(windows)]
mod imp {
    use std::collections::VecDeque;
    use std::ffi::{CStr, c_char, c_int};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::{Duration, Instant};

    use rustamp_mediahost::control::{ControlPeer, Liveness};
    use rustamp_mediahost::decoder::{
        DecoderInfo, DecoderResident, ResidentDecoder, serve_decoder,
    };
    use rustamp_mediahost::pcm::NamedPcmRings;
    use rustamp_mediahost::{MediaHostError, Result};
    use rustamp_plugin_abi::common::Hwnd;
    use rustamp_plugin_abi::output::OutModule;
    use rustamp_proto::host::{MediaLocationWire, PcmSpecWire};
    use rustamp_proto::meta::TrackMetadata;
    use rustamp_proto::state::{PlayerState, TransportState};

    use super::{EXTENSIONS_ARG, MODE_ARG, parse_args, samples_of};
    use crate::approval::ApprovalStore;
    use crate::host_window::HostWindow;
    use crate::ipc::IpcSurface;
    use crate::loader::{LoadedPlugin, load_plugin_with_store};

    /// Bounds the wait for a plug-in to open its output.
    const OPEN_WAIT: Duration = Duration::from_secs(15);
    /// Bounds the wait for a plug-in's next write.
    const READ_WAIT: Duration = Duration::from_secs(20);
    /// The seconds of audio the queue holds before `can_write` says no.
    const QUEUE_SECONDS: usize = 1;

    /// What the plug-in's decode thread pushed.
    #[derive(Default)]
    struct Capture {
        spec: Option<(i32, i32, i32)>,
        queue: VecDeque<f32>,
        written_samples: u64,
        base_ms: i64,
        closed: bool,
    }

    /// The output callbacks have no context argument, and the process serves
    /// one track.
    static CAPTURE: Mutex<Option<Capture>> = Mutex::new(None);
    static CAPTURE_CHANGED: Condvar = Condvar::new();

    fn with_capture<T>(action: impl FnOnce(&mut Capture) -> T) -> T {
        let mut guard = CAPTURE.lock().unwrap_or_else(|poison| poison.into_inner());
        let result = action(guard.get_or_insert_with(Capture::default));
        CAPTURE_CHANGED.notify_all();
        result
    }

    fn queue_limit(capture: &Capture) -> usize {
        capture.spec.map_or(0, |(rate, channels, _)| {
            rate.max(1) as usize * channels.max(1) as usize * QUEUE_SECONDS
        })
    }

    fn written_ms(capture: &Capture) -> i64 {
        let Some((rate, channels, _)) = capture.spec else {
            return capture.base_ms;
        };
        let per_second = i64::from(rate.max(1)) * i64::from(channels.max(1));
        capture.base_ms + (capture.written_samples as i64 * 1000) / per_second
    }

    unsafe extern "C" fn out_nothing_parent(_parent: Hwnd) {}
    unsafe extern "C" fn out_nothing() {}
    unsafe extern "C" fn out_open(
        rate: c_int,
        channels: c_int,
        bits: c_int,
        _buffer_ms: c_int,
        _prebuffer_ms: c_int,
    ) -> c_int {
        crate::fault::trace_event(&format!("output open {rate} Hz {channels} ch {bits} bits"));
        with_capture(|capture| {
            capture.spec = Some((rate, channels, bits));
            capture.closed = false;
        });
        0
    }
    unsafe extern "C" fn out_close() {
        crate::fault::trace_event("output close");
        with_capture(|capture| capture.closed = true);
    }
    unsafe extern "C" fn out_write(data: *mut c_char, len: c_int) -> c_int {
        if data.is_null() || len <= 0 {
            return 0;
        }
        // SAFETY: the plug-in hands `len` readable bytes for this call (S09 §4).
        let bytes = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len as usize) };
        with_capture(|capture| {
            let bits = capture.spec.map_or(16, |spec| spec.2);
            let samples = samples_of(bytes, bits);
            capture.written_samples += samples.len() as u64;
            capture.queue.extend(samples);
        });
        0
    }
    /// The bytes the queue still has room for. A plug-in compares the answer
    /// with the block it wants to write, and a fixed answer below that block
    /// would keep it waiting for ever.
    unsafe extern "C" fn out_can_write() -> c_int {
        with_capture(|capture| {
            let free = queue_limit(capture).saturating_sub(capture.queue.len());
            let bytes = capture.spec.map_or(2, |spec| (spec.2.max(8) / 8) as usize);
            (free * bytes).min(c_int::MAX as usize) as c_int
        })
    }
    unsafe extern "C" fn out_is_playing() -> c_int {
        with_capture(|capture| c_int::from(!capture.queue.is_empty()))
    }
    unsafe extern "C" fn out_pause(_state: c_int) -> c_int {
        0
    }
    unsafe extern "C" fn out_set_level(_level: c_int) {}
    unsafe extern "C" fn out_flush(time_ms: c_int) {
        with_capture(|capture| {
            capture.queue.clear();
            capture.written_samples = 0;
            capture.base_ms = i64::from(time_ms);
        });
    }
    unsafe extern "C" fn out_played_ms() -> c_int {
        with_capture(|capture| {
            let Some((rate, channels, _)) = capture.spec else {
                return capture.base_ms as c_int;
            };
            let queued = (capture.queue.len() as i64 * 1000)
                / (i64::from(rate.max(1)) * i64::from(channels.max(1)));
            (written_ms(capture) - queued) as c_int
        })
    }
    unsafe extern "C" fn out_written_ms() -> c_int {
        with_capture(|capture| written_ms(capture) as c_int)
    }

    /// The module the plug-in gets. It stays at one address for the process.
    struct CaptureModule(std::cell::UnsafeCell<OutModule>);
    // SAFETY: only the plug-in's callbacks read it, and nothing writes it after
    // the module is built; the callbacks synchronize on `CAPTURE`.
    unsafe impl Sync for CaptureModule {}

    static CAPTURE_MODULE: CaptureModule = CaptureModule(std::cell::UnsafeCell::new(OutModule {
        version: 0x10,
        description: c"Rustamp".as_ptr().cast_mut(),
        id: 65_536,
        main_window: std::ptr::null_mut(),
        dll_instance: std::ptr::null_mut(),
        config: Some(out_nothing_parent),
        about: Some(out_nothing_parent),
        init: Some(out_nothing),
        quit: Some(out_nothing),
        open: Some(out_open),
        close: Some(out_close),
        write: Some(out_write),
        can_write: Some(out_can_write),
        is_playing: Some(out_is_playing),
        pause: Some(out_pause),
        set_volume: Some(out_set_level),
        set_pan: Some(out_set_level),
        flush: Some(out_flush),
        get_played_ms: Some(out_played_ms),
        get_written_ms: Some(out_written_ms),
    }));

    /// What a plug-in says of the file it plays: its title, its length and
    /// whether it can seek in it.
    type FileInfo = (Option<String>, Option<u64>, bool);

    /// What the serve thread asks of the plug-in's UI thread.
    enum Request {
        Play {
            path: PathBuf,
            reply: Sender<std::result::Result<FileInfo, String>>,
        },
        Seek {
            to_ms: u64,
            reply: Sender<std::result::Result<(), String>>,
        },
    }

    struct PluginResident {
        requests: Sender<Request>,
    }

    fn gone() -> MediaHostError {
        MediaHostError::OperationFailed("the plug-in's window is gone".to_owned())
    }

    impl DecoderResident for PluginResident {
        fn open(&self, location: MediaLocationWire) -> Result<Box<dyn ResidentDecoder>> {
            let path = match location {
                MediaLocationWire::File(path) => PathBuf::from(path),
                MediaLocationWire::Url(url) => PathBuf::from(url),
            };
            let (reply, answer) = channel();
            self.requests
                .send(Request::Play { path, reply })
                .map_err(|_| gone())?;
            let (title, duration_ms, seekable) = answer
                .recv()
                .map_err(|_| gone())?
                .map_err(MediaHostError::OperationFailed)?;
            // The plug-in names the format when it opens the output, from its
            // own thread.
            let deadline = Instant::now() + OPEN_WAIT;
            let mut guard = CAPTURE.lock().unwrap_or_else(|poison| poison.into_inner());
            let spec = loop {
                if let Some(spec) = guard.get_or_insert_with(Capture::default).spec {
                    break spec;
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(MediaHostError::OperationFailed(
                        "the input plug-in never opened its output".to_owned(),
                    ));
                }
                guard = CAPTURE_CHANGED
                    .wait_timeout(guard, left)
                    .unwrap_or_else(|poison| poison.into_inner())
                    .0;
            };
            drop(guard);
            Ok(Box::new(PluginDecoder {
                info: DecoderInfo {
                    spec: PcmSpecWire {
                        sample_rate_hz: spec.0.max(1) as u32,
                        channels: spec.1.clamp(1, 255) as u16 as _,
                    },
                    duration_ms,
                    seekable,
                    metadata: TrackMetadata {
                        title,
                        ..TrackMetadata::default()
                    },
                },
                requests: self.requests.clone(),
            }))
        }
    }

    struct PluginDecoder {
        info: DecoderInfo,
        requests: Sender<Request>,
    }

    impl ResidentDecoder for PluginDecoder {
        fn info(&self) -> DecoderInfo {
            self.info.clone()
        }

        fn stats(&self) -> (u64, Option<u32>) {
            (0, crate::input::reported_bitrate())
        }

        fn read(&mut self, out: &mut [f32]) -> Result<usize> {
            let channels = usize::from(self.info.spec.channels).max(1);
            let deadline = Instant::now() + READ_WAIT;
            let mut guard = CAPTURE.lock().unwrap_or_else(|poison| poison.into_inner());
            loop {
                let capture = guard.get_or_insert_with(Capture::default);
                let frames = (capture.queue.len() / channels).min(out.len() / channels);
                if frames > 0 {
                    for (slot, sample) in
                        out.iter_mut().zip(capture.queue.drain(..frames * channels))
                    {
                        *slot = sample;
                    }
                    CAPTURE_CHANGED.notify_all();
                    return Ok(frames);
                }
                // The end: the plug-in posted its end of track, or closed.
                if capture.closed || crate::handoff::take_end_of_track() > 0 {
                    return Ok(0);
                }
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    return Err(MediaHostError::OperationFailed(
                        "the input plug-in stopped writing".to_owned(),
                    ));
                }
                guard = CAPTURE_CHANGED
                    .wait_timeout(guard, left.min(Duration::from_millis(50)))
                    .unwrap_or_else(|poison| poison.into_inner())
                    .0;
            }
        }

        fn seek(&mut self, to_ms: u64) -> Result<u64> {
            let (reply, answer) = channel();
            self.requests
                .send(Request::Seek { to_ms, reply })
                .map_err(|_| gone())?;
            answer
                .recv()
                .map_err(|_| gone())?
                .map_err(MediaHostError::OperationFailed)?;
            // What was queued belongs before the seek. The plug-in flushes its
            // output as well, when its thread gets to it.
            with_capture(|capture| capture.queue.clear());
            Ok(to_ms)
        }
    }

    /// Loads the approved plug-in; a refusal names the reason.
    fn load(
        dll: &Path,
        approvals: &Path,
        window: &HostWindow,
    ) -> std::result::Result<LoadedPlugin, String> {
        let store = ApprovalStore::open_or_empty(approvals)
            .map_err(|error| format!("{} could not be read: {error}", approvals.display()))?;
        // SAFETY: the store approved this exact image; a fault in it ends this
        // process, not the player, which is what the process is for.
        unsafe { load_plugin_with_store(dll, &store, window) }
            .map_err(|error| format!("{} could not be loaded: {error:?}", dll.display()))
    }

    /// The plug-in's own file as the current track: `in_msx` reads it back
    /// through the playlist IPCs. The process serves one track, so it is the
    /// first and only entry of the playlist the plug-in can see.
    fn announce_own_file(window: &HostWindow, plugin: &LoadedPlugin, path: &Path) {
        let file = path.display().to_string();
        let title = plugin
            .input_file_info(path)
            .ok()
            .and_then(|(title, _)| title)
            .or_else(|| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| file.clone());
        let state = PlayerState {
            transport: TransportState::Playing,
            current_index: Some(0),
            playlist_len: 1,
            ..PlayerState::default()
        };
        if window.apply_player_state(&state).is_err() {
            crate::fault::trace_event("the playlist position could not be set");
        }
        crate::plugin_commands::announce_track(window, 0, &title, &file);
    }

    fn answer(request: Request, plugin: &LoadedPlugin, window: &HostWindow) {
        match request {
            Request::Play { path, reply } => {
                let result = (|| {
                    let input = plugin
                        .input_module_ptr()
                        .ok_or_else(|| "this is not an input plug-in".to_owned())?;
                    // SAFETY: the module lives in the mapped plug-in's static
                    // data, the output in this process's.
                    unsafe {
                        if !crate::handoff::uses_output_plugin(&*input) {
                            return Err("the plug-in does not use an output module".to_owned());
                        }
                        crate::handoff::hand_off_output(input, CAPTURE_MODULE.0.get(), 255, 0)
                            .map_err(|error| format!("{error:?}"))?;
                    }
                    announce_own_file(window, plugin, &path);
                    plugin
                        .input_play(&path)
                        .map_err(|error| format!("{error:?}"))?;
                    crate::fault::trace_event(&format!("play {} returned", path.display()));
                    let (title, known) = plugin.input_file_info(&path).unwrap_or((None, None));
                    let length = plugin.input_length_ms().unwrap_or(-1);
                    let length = u64::try_from(length).ok().filter(|ms| *ms > 0);
                    // SAFETY: the module lives in the mapped plug-in's static
                    // data; the plug-in sets the member when it opens a file.
                    let seekable = unsafe { (*input).seekable } != 0;
                    Ok((title, length.or(known), seekable))
                })();
                let _ = reply.send(result);
            }
            Request::Seek { to_ms, reply } => {
                let time = i32::try_from(to_ms).unwrap_or(i32::MAX);
                let _ = reply.send(
                    plugin
                        .input_seek(time)
                        .map_err(|error| format!("{error:?}")),
                );
            }
        }
    }

    fn pump(
        window: &HostWindow,
        plugin: &LoadedPlugin,
        requests: &Receiver<Request>,
        done: &AtomicBool,
    ) {
        while !done.load(Ordering::Acquire) {
            while let Ok(request) = requests.try_recv() {
                answer(request, plugin, window);
            }
            if !crate::message_loop::pump_bounded(window, 64) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// `--input-extensions`: prints the plug-in's extensions, one list per line.
    pub fn print_extensions() -> i32 {
        let args: Vec<String> = std::env::args().collect();
        let Some((dll, approvals)) = parse_args(&args, EXTENSIONS_ARG) else {
            eprintln!("rustamp-plughost32: {EXTENSIONS_ARG} <dll> --approvals <store>");
            return 2;
        };
        let _fault_guard = crate::fault::install_fault_guard();
        let window = match HostWindow::create(IpcSurface::new()) {
            Ok(window) => window,
            Err(error) => {
                eprintln!("rustamp-plughost32: cannot create host window: {error}");
                return 1;
            }
        };
        let plugin = match load(&dll, &approvals, &window) {
            Ok(plugin) => plugin,
            Err(error) => {
                eprintln!("rustamp-plughost32: {error}");
                return 1;
            }
        };
        let Some(input) = plugin.input_module_ptr() else {
            eprintln!("rustamp-plughost32: this is not an input plug-in");
            return 1;
        };
        // SAFETY: a double-NUL-terminated list of pairs in the plug-in's static
        // data (S09 section 3.4): extensions, then a name.
        unsafe {
            let mut cursor = (*input).file_extensions.cast_const();
            while !cursor.is_null() && *cursor != 0 {
                let extensions = CStr::from_ptr(cursor);
                println!("{}", extensions.to_string_lossy());
                cursor = cursor.add(extensions.to_bytes().len() + 1);
                if *cursor == 0 {
                    break;
                }
                let name = CStr::from_ptr(cursor);
                cursor = cursor.add(name.to_bytes().len() + 1);
            }
        }
        0
    }

    /// Runs the resident until the player closes the control channel.
    pub fn run() -> i32 {
        let args: Vec<String> = std::env::args().collect();
        let Some((dll, approvals)) = parse_args(&args, MODE_ARG) else {
            eprintln!("rustamp-plughost32: {MODE_ARG} <dll> --approvals <store>");
            return 2;
        };
        let _fault_guard = crate::fault::install_fault_guard();
        let window = match HostWindow::create(IpcSurface::new()) {
            Ok(window) => window,
            Err(error) => {
                eprintln!("rustamp-plughost32: cannot create host window: {error}");
                return 1;
            }
        };
        let plugin = load(&dll, &approvals, &window);
        let (requests, incoming) = channel();
        let done = Arc::new(AtomicBool::new(false));
        let load_problem = plugin.as_ref().err().cloned();
        let serve = {
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                let result = ControlPeer::start(
                    std::io::stdin(),
                    std::io::stdout(),
                    Liveness::default(),
                    true,
                )
                .and_then(|peer| match load_problem {
                    // The refusal reaches the player as the track's error.
                    Some(problem) => {
                        let refused = RefusedResident(problem);
                        serve_decoder(&peer, &NamedPcmRings, &refused)
                    }
                    None => serve_decoder(&peer, &NamedPcmRings, &PluginResident { requests }),
                });
                done.store(true, Ordering::Release);
                result
            })
        };
        if let Ok(plugin) = &plugin {
            pump(&window, plugin, &incoming, &done);
            let _ = plugin.input_stop();
        }
        match serve.join() {
            Ok(Ok(())) => 0,
            Ok(Err(error)) => {
                eprintln!("rustamp-plughost32: the input resident ended: {error}");
                1
            }
            Err(_) => 1,
        }
    }

    struct RefusedResident(String);

    impl DecoderResident for RefusedResident {
        fn open(&self, _location: MediaLocationWire) -> Result<Box<dyn ResidentDecoder>> {
            Err(MediaHostError::OperationFailed(self.0.clone()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_read_lower_case_once_and_clean() {
        assert_eq!(
            parse_extensions("MOD;s3m;it\nmod;XM\n\nbad ext;.x\n"),
            ["mod", "s3m", "it", "xm"]
        );
    }

    #[test]
    fn the_arguments_round_trip() {
        let args = resident_args(
            std::path::Path::new(r"C:\Plugins\in ä.dll"),
            std::path::Path::new(r"C:\cfg\a.toml"),
        );
        assert_eq!(
            parse_args(&args, MODE_ARG),
            Some((
                PathBuf::from(r"C:\Plugins\in ä.dll"),
                PathBuf::from(r"C:\cfg\a.toml")
            ))
        );
        assert_eq!(parse_args(&args[1..], MODE_ARG), None);
    }

    #[test]
    fn every_bit_depth_comes_out_in_range() {
        assert_eq!(samples_of(&[0, 128, 255], 8)[1], 0.0);
        assert_eq!(samples_of(&[0x00, 0x40], 16), [0.5]);
        assert_eq!(samples_of(&[0x00, 0x00, 0x40], 24), [0.5]);
        assert_eq!(samples_of(&[0x00, 0x00, 0x00, 0x40], 32), [0.5]);
    }
}
