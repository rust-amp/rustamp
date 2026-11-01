//! The DSP resident: a classic `dsp_` plug-in in the player's audio path.
//!
//! The engine processes its blocks on its decode thread, and a classic DSP
//! plug-in is a 32-bit DLL, so a block has to cross into a 32-bit process and
//! back. The media-host framework already does that for a resident
//! ([`rustamp_mediahost::dsp`]): a configure handshake, two shared-memory rings
//! and block pumping. This module is the resident behind it, started as
//! `rustamp-plughost32 --dsp-resident`, one process per playing chain.
//!
//! The main thread is the plug-in's UI thread (S09 section 7.2): it owns the
//! host window, loads the plug-in and keeps pumping messages. The framework's
//! serve loop runs on a second thread, which calls `process` like the decode
//! thread of an input plug-in does (S09 section 3.6).
//!
//! A plug-in may answer with fewer or more frames than it got (S09 section 6.2)
//! and the engine's blocks keep their length, so the output passes through a
//! queue. Own choices: blocks of 576 frames, the classic visualization block; a
//! short answer plays silence rather than stopping; and a plug-in that keeps
//! producing more than it gets loses its oldest output past ten seconds.
//!
//! Between the blocks the player's notes arrive ([`ResidentChain::note`]): the
//! track, a pause and resume for an output plug-in, and the request for a DSP
//! plug-in's dialog. The dialog runs here, in the instance that processes the
//! audio, because a plug-in's dialog changes what that instance does; the audio
//! keeps flowing meanwhile, as the dialog holds the main thread and not the
//! serve thread.
//!
//! An output plug-in sees a track as the classic players show it: `open` and
//! `close` around each file. So a new file lets the plug-in play out what it
//! holds, closes it and opens it again: the disk writer names one file per track
//! from the title it reads when it is opened.
//!
//! Two short-lived modes answer for a player that plays nothing:
//! `--dsp-modules <dll> --approvals <store>` prints the names of a library's
//! modules, and `--dsp-configure <dll> --approvals <store> --module <n>` shows
//! the dialog of a module and quits it, which is when plug-ins save.

use std::path::PathBuf;

use rustamp_proto::host::DspStageDesc;

/// The command-line switch that selects this mode of the host binary.
pub const MODE_ARG: &str = "--dsp-resident";
/// The switch that prints the names of a library's modules.
pub const MODULES_ARG: &str = "--dsp-modules";
/// The switch that shows a module's dialog.
pub const CONFIGURE_ARG: &str = "--dsp-configure";
/// The switch that names the module for [`CONFIGURE_ARG`].
const MODULE_ARG: &str = "--module";

/// The identifier of the one stage this resident serves.
pub const STAGE_ID: &str = "classic-dsp";

/// The identifier of the stage that hands the blocks to an output plug-in.
pub const OUTPUT_STAGE_ID: &str = "classic-output";

/// The stage that plays through the output plug-in `dll`.
pub fn output_stage(dll: &std::path::Path, approvals: &std::path::Path) -> DspStageDesc {
    DspStageDesc {
        id: OUTPUT_STAGE_ID.to_owned(),
        ..stage(dll, approvals, 0)
    }
}

/// The stage that runs module `module` of `dll`, approved by the store at
/// `approvals`.
pub fn stage(dll: &std::path::Path, approvals: &std::path::Path, module: u32) -> DspStageDesc {
    DspStageDesc {
        id: STAGE_ID.to_owned(),
        params: format!("{}\n{}\n{module}", dll.display(), approvals.display()).into_bytes(),
    }
}

/// The arguments that ask for the names of `dll`'s modules.
pub fn modules_args(dll: &std::path::Path, approvals: &std::path::Path) -> Vec<String> {
    vec![
        MODULES_ARG.to_owned(),
        dll.display().to_string(),
        crate::input_resident::APPROVALS_ARG.to_owned(),
        approvals.display().to_string(),
    ]
}

/// The arguments that show the dialog of module `module` of `dll`.
pub fn configure_args(
    dll: &std::path::Path,
    approvals: &std::path::Path,
    module: u32,
) -> Vec<String> {
    vec![
        CONFIGURE_ARG.to_owned(),
        dll.display().to_string(),
        crate::input_resident::APPROVALS_ARG.to_owned(),
        approvals.display().to_string(),
        MODULE_ARG.to_owned(),
        module.to_string(),
    ]
}

fn parse_stage(params: &[u8]) -> Option<(PathBuf, PathBuf, i32)> {
    let text = std::str::from_utf8(params).ok()?;
    let mut lines = text.split('\n');
    let dll = lines.next()?;
    let approvals = lines.next()?;
    let module = lines.next()?.parse().ok()?;
    Some((PathBuf::from(dll), PathBuf::from(approvals), module))
}

/// The value after [`MODULE_ARG`]; the first module without one.
fn parse_module(args: &[String]) -> i32 {
    args.iter()
        .position(|arg| arg == MODULE_ARG)
        .and_then(|at| args.get(at + 1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(crate::dsp::DSP_DEFAULT_MODULE_INDEX)
}

#[cfg(windows)]
pub use imp::{configure, print_modules, run};

#[cfg(windows)]
mod imp {
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{Receiver, Sender, channel};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use rustamp_ipc::Message;
    use rustamp_mediahost::control::{ControlPeer, Liveness};
    use rustamp_mediahost::dsp::{DspResident, ResidentChain, serve_dsp};
    use rustamp_mediahost::pcm::NamedPcmRings;
    use rustamp_mediahost::{MediaHostError, Result};
    use rustamp_proto::command::Command;
    use rustamp_proto::host::{DspStageDesc, PcmSpecWire};
    use rustamp_proto::state::{PlayerState, TransportState};

    use super::{CONFIGURE_ARG, MODULES_ARG, OUTPUT_STAGE_ID, STAGE_ID, parse_stage};
    use crate::approval::ApprovalStore;
    use crate::host_window::HostWindow;
    use crate::input_resident::{APPROVALS_ARG, parse_args};
    use crate::ipc::IpcSurface;
    use crate::loader::{LoadedPlugin, load_plugin_with_store};

    /// Frames handed to the plug-in at once.
    const BLOCK_FRAMES: usize = 576;
    /// Seconds of surplus output the queue keeps.
    const MAX_QUEUE_SECONDS: usize = 10;
    /// Bounds the wait for an output plug-in to play out before a new file; the
    /// rest is cut off. Own choice, well below the player's bound for a block.
    const DRAIN_WAIT: Duration = Duration::from_secs(5);
    /// Bounds the wait for the main thread to take a new track.
    const TRACK_WAIT: Duration = Duration::from_secs(1);

    /// The loaded plug-in, shared with the serve thread.
    ///
    /// The main thread owns it for the whole process and drops it only after
    /// the serve thread has ended. `process` running on another thread than
    /// `init` is the classic contract (S09 section 3.6).
    #[derive(Clone, Copy)]
    struct SharedPlugin(*const LoadedPlugin);
    // SAFETY: see the type's documentation.
    unsafe impl Send for SharedPlugin {}

    /// A load the serve thread asks the main thread for.
    struct LoadRequest {
        dll: PathBuf,
        approvals: PathBuf,
        module: i32,
        reply: Sender<std::result::Result<SharedPlugin, String>>,
    }

    /// What the serve thread asks of the main thread, which owns the window and
    /// the plug-in's dialogs.
    enum ToMain {
        Load(LoadRequest),
        Track {
            position: u32,
            title: String,
            file: String,
            done: Option<Sender<()>>,
        },
        Transport(TransportState),
        Configure,
    }

    struct ClassicResident {
        to_main: Sender<ToMain>,
        /// A reconfigure finds the plug-in here: the main thread may be inside
        /// the plug-in's dialog, and the audio must not wait for it.
        loaded: Mutex<Option<SharedPlugin>>,
    }

    impl ClassicResident {
        fn plugin(&self, stage: &DspStageDesc) -> Result<SharedPlugin> {
            let mut loaded = self
                .loaded
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if let Some(plugin) = *loaded {
                return Ok(plugin);
            }
            let (dll, approvals, module) = parse_stage(&stage.params).ok_or_else(|| {
                MediaHostError::OperationFailed("undecodable classic DSP stage".to_owned())
            })?;
            let (reply, answer) = channel();
            self.to_main
                .send(ToMain::Load(LoadRequest {
                    dll,
                    approvals,
                    module,
                    reply,
                }))
                .map_err(|_| {
                    MediaHostError::OperationFailed("the host window is gone".to_owned())
                })?;
            let plugin = answer
                .recv()
                .map_err(|_| MediaHostError::OperationFailed("the host window is gone".to_owned()))?
                .map_err(MediaHostError::OperationFailed)?;
            *loaded = Some(plugin);
            Ok(plugin)
        }
    }

    impl DspResident for ClassicResident {
        fn open(
            &self,
            spec: PcmSpecWire,
            stages: &[DspStageDesc],
        ) -> Result<Box<dyn ResidentChain>> {
            let [stage] = stages else {
                return Err(MediaHostError::OperationFailed(format!(
                    "the classic DSP resident serves one stage, not {}",
                    stages.len()
                )));
            };
            if stage.id != STAGE_ID && stage.id != OUTPUT_STAGE_ID {
                return Err(MediaHostError::OperationFailed(format!(
                    "unknown stage {:?}",
                    stage.id
                )));
            }
            let plugin = self.plugin(stage)?;
            if stage.id == OUTPUT_STAGE_ID {
                // SAFETY: the main thread keeps the plug-in alive (`SharedPlugin`).
                let module = unsafe { &*plugin.0 }.output_module_ptr().ok_or_else(|| {
                    MediaHostError::OperationFailed("this is not an output plug-in".to_owned())
                })?;
                return Ok(Box::new(OutputChain {
                    module,
                    spec,
                    opened: false,
                    bytes: Vec::new(),
                    to_main: self.to_main.clone(),
                    next_track: None,
                    file: None,
                    paused: false,
                    clock: PlayedClock::default(),
                }));
            }
            Ok(Box::new(ClassicChain {
                plugin,
                channels: usize::from(spec.channels),
                sample_rate: spec.sample_rate_hz,
                block: Vec::with_capacity(BLOCK_FRAMES * 2 * usize::from(spec.channels)),
                queue: VecDeque::new(),
                to_main: self.to_main.clone(),
            }))
        }
    }

    struct ClassicChain {
        plugin: SharedPlugin,
        channels: usize,
        sample_rate: u32,
        block: Vec<i16>,
        queue: VecDeque<f32>,
        to_main: Sender<ToMain>,
    }

    impl ResidentChain for ClassicChain {
        fn latency_frames(&self) -> u32 {
            0
        }

        fn process(&mut self, buf: &mut [f32]) -> Result<()> {
            // SAFETY: the main thread keeps the plug-in alive until this thread
            // has ended (see `SharedPlugin`).
            let plugin = unsafe { &*self.plugin.0 };
            for input in buf.chunks(BLOCK_FRAMES * self.channels) {
                self.block.clear();
                self.block.extend(
                    input
                        .iter()
                        .map(|sample| (sample.clamp(-1.0, 1.0) * 32767.0) as i16),
                );
                plugin
                    .dsp_process_i16(&mut self.block, self.channels, self.sample_rate)
                    .ok_or_else(|| {
                        MediaHostError::OperationFailed("the DSP plug-in is not running".to_owned())
                    })?;
                self.queue
                    .extend(self.block.iter().map(|sample| f32::from(*sample) / 32767.0));
            }
            let limit = MAX_QUEUE_SECONDS * self.sample_rate as usize * self.channels;
            if self.queue.len() > limit + buf.len() {
                let surplus = self.queue.len() - limit - buf.len();
                self.queue.drain(..surplus);
            }
            let wanted = buf.len().min(self.queue.len());
            let silence = buf.len() - wanted;
            buf[..silence].fill(0.0);
            for (out, sample) in buf[silence..].iter_mut().zip(self.queue.drain(..wanted)) {
                *out = sample;
            }
            Ok(())
        }

        fn reset(&mut self) {
            self.queue.clear();
        }

        fn note(&mut self, message: &Message) {
            let request = match message {
                Message::NowPlaying {
                    position,
                    title,
                    file,
                } => ToMain::Track {
                    position: *position,
                    title: title.clone(),
                    file: file.clone(),
                    done: None,
                },
                Message::CommandToHost(Command::ConfigurePlugin { .. }) => ToMain::Configure,
                _ => return,
            };
            // Only a main thread that is gone refuses, and then so does the rest.
            let _ = self.to_main.send(request);
        }
    }

    /// Hands every block to an output plug-in and passes it on unchanged: the
    /// player waits for the block's return, which is the plug-in's `can_write`
    /// reaching back (see `rustamp_app::plugin_output`).
    struct OutputChain {
        module: *mut rustamp_plugin_abi::output::OutModule,
        spec: PcmSpecWire,
        opened: bool,
        bytes: Vec<u8>,
        to_main: Sender<ToMain>,
        /// The track the player named since the last block.
        next_track: Option<(u32, String, String)>,
        /// The file the plug-in is open for.
        file: Option<String>,
        paused: bool,
        clock: PlayedClock,
    }
    // SAFETY: the module lives in the plug-in's static data, and only this
    // thread drives its audio calls, as the decode thread of an input plug-in
    // does (S09 section 3.7).
    unsafe impl Send for OutputChain {}

    /// A plug-in's clock as one that never falls: the plug-in starts its own over
    /// when it is opened and flushed.
    #[derive(Default)]
    struct PlayedClock {
        total_ms: u64,
        last_ms: i32,
    }

    impl PlayedClock {
        fn read(&mut self, ms: i32) {
            self.total_ms += u64::try_from(ms.saturating_sub(self.last_ms)).unwrap_or(0);
            self.last_ms = self.last_ms.max(ms);
        }

        fn start_over(&mut self) {
            self.last_ms = 0;
        }
    }

    /// The most the plug-in takes at once (S09 section 4.1).
    const OUTPUT_WRITE_BYTES: usize = 8192;

    /// How long one block may wait for the plug-in to take any of it before the
    /// write is given up as failed. Longer than any buffer the host asks for.
    const WRITE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

    impl OutputChain {
        fn open(&mut self) -> Result<()> {
            // SAFETY: a live module; the callbacks run on this thread.
            let module = unsafe { &*self.module };
            let open = module.open.ok_or_else(|| {
                MediaHostError::OperationFailed("the output plug-in cannot open".to_owned())
            })?;
            let rate = self.spec.sample_rate_hz as i32;
            let channels = i32::from(self.spec.channels);
            // SAFETY: the plug-in's own callback; -1 asks for its default
            // buffer and prebuffer (S09 section 4.2).
            let latency = unsafe { open(rate, channels, 16, -1, -1) };
            crate::fault::trace_event(&format!(
                "output open {rate} Hz {channels} ch: latency {latency}"
            ));
            if latency < 0 {
                return Err(MediaHostError::OperationFailed(format!(
                    "the output plug-in refused {rate} Hz, {channels} channels"
                )));
            }
            if let Some(set_volume) = module.set_volume {
                // SAFETY: as above. The engine applies the player's volume.
                unsafe { set_volume(255) };
            }
            self.opened = true;
            if self.paused {
                self.set_paused(true);
            }
            Ok(())
        }

        fn read_clock(&mut self) {
            // SAFETY: a live module; the callback runs on the thread that writes.
            if self.opened
                && let Some(played) = unsafe { (*self.module).get_played_ms }
            {
                // SAFETY: the plug-in's own callback.
                self.clock.read(unsafe { played() });
            }
        }

        fn close(&mut self) {
            if !self.opened {
                return;
            }
            self.read_clock();
            self.clock.start_over();
            self.opened = false;
            // SAFETY: a live module: the main thread drops the plug-in only
            // after the serve thread has ended.
            if let Some(close) = unsafe { (*self.module).close } {
                // SAFETY: the plug-in's own callback.
                unsafe { close() };
            }
        }

        /// Lets the plug-in play out what it holds, as far as the bound allows.
        fn play_out(&self) {
            // SAFETY: a live module; the callbacks run on this thread.
            let Some(is_playing) = (unsafe { (*self.module).is_playing }) else {
                return;
            };
            let deadline = Instant::now() + DRAIN_WAIT;
            // SAFETY: the plug-in's own callback.
            while unsafe { is_playing() } != 0 && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
        }

        fn set_paused(&mut self, paused: bool) {
            self.paused = paused;
            if !self.opened {
                return;
            }
            // SAFETY: a live module; the callback runs on the thread that writes.
            if let Some(pause) = unsafe { (*self.module).pause } {
                // SAFETY: the plug-in's own callback; the previous state it
                // answers with is ours already.
                unsafe { pause(i32::from(paused)) };
            }
        }

        /// Drops what the plug-in still holds, so a pause is heard at once.
        /// The plug-in's own `pause` does not stop a buffer it already plays
        /// (`out_wave` opens with two seconds), and a player that keeps sounding
        /// after the pause is what a listener calls broken.
        fn drop_buffered(&mut self) {
            self.pending.clear();
            if !self.opened {
                return;
            }
            // SAFETY: a live module; the callback runs on the thread that writes.
            if let Some(flush) = unsafe { (*self.module).flush } {
                // SAFETY: the plug-in's own callback; 0 restarts its clock at
                // the current position, which the host does not read for an
                // output.
                unsafe { flush(0) };
            }
        }

        /// A new file closes the plug-in for the old one. The main thread takes
        /// the track before the plug-in is opened and reads it.
        fn take_next_track(&mut self) {
            let Some((position, title, file)) = self.next_track.take() else {
                return;
            };
            if self.opened && self.file.as_deref() != Some(file.as_str()) {
                self.play_out();
                self.close();
            }
            self.file = Some(file.clone());
            let (done, taken) = channel();
            let request = ToMain::Track {
                position,
                title,
                file,
                done: Some(done),
            };
            if self.to_main.send(request).is_ok() && taken.recv_timeout(TRACK_WAIT).is_err() {
                crate::fault::trace_event("the main thread did not take the track in time");
            }
        }
    }

    impl ResidentChain for OutputChain {
        fn latency_frames(&self) -> u32 {
            0
        }

        fn process(&mut self, buf: &mut [f32]) -> Result<()> {
            self.take_next_track();
            if !self.opened {
                self.open()?;
            }
            // SAFETY: a live module; the callbacks run on this thread.
            let module = unsafe { &*self.module };
            let (Some(can_write), Some(write)) = (module.can_write, module.write) else {
                return Err(MediaHostError::OperationFailed(
                    "the output plug-in cannot write".to_owned(),
                ));
            };
            self.bytes.clear();
            for sample in buf.iter() {
                let value = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
                self.bytes.extend_from_slice(&value.to_le_bytes());
            }
            let frame = usize::from(self.spec.channels) * 2;
            // The plug-in says how many bytes it takes *right now*; that is what
            // it gets, in whole frames. Asking for a fixed 8192 first and waiting
            // until they fit is how a plug-in with a smaller buffer or a coarser
            // free-space answer stalls the whole audio path: `out_wave` stopped
            // after about a second and `out_disk` never took a block at all
            // ("resident DSP read timed out").
            let started = std::time::Instant::now();
            let mut written = 0usize;
            let mut smallest_free = usize::MAX;
            while written < self.bytes.len() {
                // SAFETY: the plug-in's own callback.
                let free = unsafe { can_write() } as usize;
                smallest_free = smallest_free.min(free);
                if free == 0 {
                    if started.elapsed() >= WRITE_WAIT {
                        return Err(MediaHostError::OperationFailed(format!(
                            "the output plug-in takes no audio for {} ms (can_write was {smallest_free} at its smallest)",
                            WRITE_WAIT.as_millis()
                        )));
                    }
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                }
                let take =
                    free.min(self.bytes.len() - written).min(OUTPUT_WRITE_BYTES) / frame * frame;
                if take == 0 {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                }
                // SAFETY: `take` bytes from `written` on are readable for this
                // call, and the ABI allows the plug-in to touch them.
                unsafe { write(self.bytes.as_mut_ptr().add(written).cast(), take as i32) };
                written += take;
            }
            Ok(())
        }

        fn reset(&mut self) {
            self.read_clock();
            self.clock.start_over();
            // SAFETY: a live module; the callbacks run on this thread.
            let module = unsafe { &*self.module };
            if self.opened
                && let Some(flush) = module.flush
            {
                // SAFETY: the plug-in's own callback; the clock starts over.
                unsafe { flush(0) };
            }
        }

        fn played_ms(&mut self) -> Option<u64> {
            // SAFETY: a live module.
            unsafe { (*self.module).get_played_ms }?;
            self.read_clock();
            self.opened.then_some(self.clock.total_ms)
        }

        fn note(&mut self, message: &Message) {
            let transport = match message {
                Message::NowPlaying {
                    position,
                    title,
                    file,
                } => {
                    crate::fault::trace_event(&format!(
                        "output chain: the track is known before open: {position} {title}"
                    ));
                    self.next_track = Some((*position, title.clone(), file.clone()));
                    return;
                }
                Message::CommandToHost(Command::PausePlugin { .. }) => {
                    crate::fault::trace_event(&format!(
                        "output chain: pause at played {:?}",
                        self.played_ms()
                    ));
                    self.set_paused(true);
                    // A paused plug-in that keeps playing its buffer is heard as
                    // a player that ignores the pause: `out_wave` opens with two
                    // seconds and does exactly that. The pause therefore drops
                    // what it holds, so the cable is silent from the command on.
                    self.drop_buffered();
                    TransportState::Paused
                }
                Message::CommandToHost(Command::UnpausePlugin { .. }) => {
                    crate::fault::trace_event(&format!(
                        "output chain: unpause at played {:?}",
                        self.played_ms()
                    ));
                    self.set_paused(false);
                    TransportState::Playing
                }
                _ => return,
            };
            let _ = self.to_main.send(ToMain::Transport(transport));
        }
    }

    impl Drop for OutputChain {
        fn drop(&mut self) {
            self.close();
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

    /// Starts `module` when it is not the one the load started. A library that
    /// no longer has it keeps its first: the selection outlives the file.
    fn start_module(plugin: &mut LoadedPlugin, module: i32) -> std::result::Result<(), String> {
        if module == crate::dsp::DSP_DEFAULT_MODULE_INDEX {
            return Ok(());
        }
        match plugin.dsp_start(module) {
            Err(crate::dsp::DspError::UnknownModule(_)) => {
                crate::fault::trace_event(&format!("no DSP module {module}, keeping the first"));
                Ok(())
            }
            other => other.map_err(|error| format!("DSP module {module}: {error:?}")),
        }
    }

    /// Loads the plug-in once: a reconfigure of the serve thread does not ask.
    fn answer_load(
        request: LoadRequest,
        window: &HostWindow,
        loaded: &mut Option<Box<LoadedPlugin>>,
    ) {
        let result = load(&request.dll, &request.approvals, window).and_then(|mut plugin| {
            start_module(&mut plugin, request.module)?;
            Ok(plugin)
        });
        match result {
            Ok(plugin) => {
                let plugin = loaded.insert(Box::new(plugin));
                let _ = request
                    .reply
                    .send(Ok(SharedPlugin(std::ptr::from_ref(&**plugin))));
            }
            Err(problem) => {
                let _ = request.reply.send(Err(problem));
            }
        }
    }

    fn answer(
        request: ToMain,
        window: &HostWindow,
        loaded: &mut Option<Box<LoadedPlugin>>,
        state: &mut PlayerState,
    ) {
        match request {
            ToMain::Load(request) => answer_load(request, window, loaded),
            ToMain::Track {
                position,
                title,
                file,
                done,
            } => {
                state.transport = TransportState::Playing;
                state.current_index = Some(position);
                state.playlist_len = position + 1;
                apply_state(window, state);
                crate::plugin_commands::announce_track(window, position, &title, &file);
                if let Some(done) = done {
                    let _ = done.send(());
                }
            }
            ToMain::Transport(transport) => {
                state.transport = transport;
                apply_state(window, state);
            }
            ToMain::Configure => {
                let result = match loaded {
                    Some(plugin) => plugin.config().map_err(|error| format!("{error:?}")),
                    None => Err("no plug-in is loaded".to_owned()),
                };
                if let Err(problem) = result {
                    crate::fault::trace_event(&format!("no dialog: {problem}"));
                }
            }
        }
    }

    /// What a plug-in of this process reads of the player.
    fn apply_state(window: &HostWindow, state: &PlayerState) {
        if window.apply_player_state(state).is_err() {
            crate::fault::trace_event("the player's state could not be set");
        }
    }

    fn pump_until_done(window: &HostWindow, requests: &Receiver<ToMain>, done: &AtomicBool) {
        let mut loaded = None;
        let mut state = PlayerState::default();
        while !done.load(Ordering::Acquire) {
            while let Ok(request) = requests.try_recv() {
                answer(request, window, &mut loaded, &mut state);
            }
            if !crate::message_loop::pump_bounded(window, 64) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // The plug-in's `quit` runs on its UI thread, after the last block.
        drop(loaded);
    }

    /// Runs the resident until the player closes the control channel.
    pub fn run() -> i32 {
        let _fault_guard = crate::fault::install_fault_guard();
        let window = match HostWindow::create(IpcSurface::new()) {
            Ok(window) => window,
            Err(error) => {
                eprintln!("rustamp-plughost32: cannot create host window: {error}");
                return 1;
            }
        };
        let (to_main, requests) = channel();
        let done = Arc::new(AtomicBool::new(false));
        let serve = {
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                let result = ControlPeer::start(
                    std::io::stdin(),
                    std::io::stdout(),
                    Liveness::default(),
                    true,
                )
                .and_then(|peer| {
                    let resident = ClassicResident {
                        to_main,
                        loaded: Mutex::new(None),
                    };
                    serve_dsp(&peer, &NamedPcmRings, &resident)
                });
                done.store(true, Ordering::Release);
                result
            })
        };
        pump_until_done(&window, &requests, &done);
        match serve.join() {
            Ok(Ok(())) => 0,
            Ok(Err(error)) => {
                eprintln!("rustamp-plughost32: the DSP resident ended: {error}");
                1
            }
            Err(_) => 1,
        }
    }

    /// The window and the plug-in of a short-lived mode.
    fn open_for(mode: &str) -> std::result::Result<(HostWindow, LoadedPlugin, i32), i32> {
        let args: Vec<String> = std::env::args().collect();
        let Some((dll, approvals)) = parse_args(&args, mode) else {
            eprintln!("rustamp-plughost32: {mode} <dll> {APPROVALS_ARG} <store>");
            return Err(2);
        };
        let window = HostWindow::create(IpcSurface::new()).map_err(|error| {
            eprintln!("rustamp-plughost32: cannot create host window: {error}");
            1
        })?;
        let plugin = load(&dll, &approvals, &window).map_err(|problem| {
            eprintln!("rustamp-plughost32: {problem}");
            1
        })?;
        Ok((window, plugin, super::parse_module(&args)))
    }

    /// `--dsp-modules`: prints the names of the library's modules, one per line.
    pub fn print_modules() -> i32 {
        let _fault_guard = crate::fault::install_fault_guard();
        let (_window, plugin, _) = match open_for(MODULES_ARG) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let Some(library) = plugin.dsp_info() else {
            eprintln!("rustamp-plughost32: this is not a DSP plug-in");
            return 1;
        };
        for module in library.modules {
            println!("{}", module.description);
        }
        0
    }

    /// Whether this thread shows a window: a dialog may outlive the `config`
    /// call that opened it.
    fn shows_a_window() -> bool {
        use windows::Win32::Foundation::{HWND, LPARAM};
        use windows::Win32::System::Threading::GetCurrentThreadId;
        use windows::Win32::UI::WindowsAndMessaging::{EnumThreadWindows, IsWindowVisible};
        use windows::core::BOOL;
        unsafe extern "system" fn look(window: HWND, found: LPARAM) -> BOOL {
            // SAFETY: Windows names a live window; `found` is the flag below,
            // which outlives the enumeration.
            unsafe {
                if IsWindowVisible(window).as_bool() {
                    *(found.0 as *mut bool) = true;
                    return BOOL(0);
                }
            }
            BOOL(1)
        }
        let mut found = false;
        // SAFETY: the callback only writes the flag; the enumeration ends
        // before the flag does. It answers false when it was stopped early.
        let _ = unsafe {
            EnumThreadWindows(
                GetCurrentThreadId(),
                Some(look),
                LPARAM(std::ptr::from_mut(&mut found) as isize),
            )
        };
        found
    }

    /// `--dsp-configure`: shows the module's dialog. Dropping the plug-in quits
    /// the module, which is when many plug-ins write what the dialog changed.
    pub fn configure() -> i32 {
        let _fault_guard = crate::fault::install_fault_guard();
        let (window, mut plugin, module) = match open_for(CONFIGURE_ARG) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        if let Err(problem) = start_module(&mut plugin, module) {
            eprintln!("rustamp-plughost32: {problem}");
            return 1;
        }
        if let Err(error) = plugin.config() {
            eprintln!("rustamp-plughost32: no dialog: {error:?}");
            return 1;
        }
        while shows_a_window() && crate::message_loop::pump_bounded(&window, 64) {
            std::thread::sleep(Duration::from_millis(5));
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stage_carries_both_paths_and_the_module() {
        let desc = stage(
            std::path::Path::new(r"C:\Plugins\dsp ä.dll"),
            std::path::Path::new(r"C:\cfg\approved.toml"),
            2,
        );
        assert_eq!(desc.id, STAGE_ID);
        assert_eq!(
            parse_stage(&desc.params),
            Some((
                PathBuf::from(r"C:\Plugins\dsp ä.dll"),
                PathBuf::from(r"C:\cfg\approved.toml"),
                2
            ))
        );
        assert_eq!(parse_stage(b"no separator"), None);
        assert_eq!(parse_stage(b"a\nb"), None);
        assert_eq!(parse_stage(&[0xFF, b'\n']), None);
    }

    #[test]
    fn the_short_modes_name_the_module_and_default_to_the_first() {
        let args = configure_args(
            std::path::Path::new(r"C:\dsp.dll"),
            std::path::Path::new(r"C:\a.toml"),
            3,
        );
        assert_eq!(parse_module(&args), 3);
        let args = modules_args(
            std::path::Path::new(r"C:\dsp.dll"),
            std::path::Path::new(r"C:\a.toml"),
        );
        assert_eq!(parse_module(&args), crate::dsp::DSP_DEFAULT_MODULE_INDEX);
        assert_eq!(args[0], MODULES_ARG);
    }
}
