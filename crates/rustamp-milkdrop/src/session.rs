// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! A running visualization: a worker thread owns the GPU and the engine. The
//! window asks for presets and sizes, hands over audio and picks up the newest
//! frame; it never waits for the GPU.
//!
//! Whatever goes wrong in there, from a machine without a graphics adapter to a
//! panic in a driver callback, ends as a [`Status`] the window can show. The
//! player keeps playing.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TrySendError};
use lactea::{FrameClock, PresetControl, Transition};
use rustamp_core::vis::VisFrame;

use crate::audio::PcmFeed;
use crate::gpu::Gpu;
use crate::presets::PresetSet;
use crate::render::Renderer;
use crate::selection::{Outcome, Selector, Step};
use crate::textures::FolderTextures;

/// The frame rate on a real GPU (presets are written for about this).
const FPS: u32 = 60;
/// The frame rate on the software adapter.
const SOFTWARE_FPS: u32 = 30;
/// The longest side of a rendered frame on a real GPU; a larger window scales it.
const MAX_SIDE: u32 = 2560;
/// The longest side on the software adapter.
const SOFTWARE_MAX_SIDE: u32 = 640;
/// With no audio for this long the player is stopped or paused: silence goes in,
/// so that the picture calms down rather than freezing on the last beat.
const SILENCE_AFTER: Duration = Duration::from_millis(250);
/// How many audio frames wait for the worker before new ones are dropped.
const AUDIO_QUEUE: usize = 16;
/// One silent audio frame's samples per channel, per tick.
const SILENCE: [f32; 2 * 256] = [0.0; 2 * 256];
/// How many notes wait before the oldest is dropped: nobody may be reading them.
const MAX_NOTES: usize = 64;

/// No graphics API at all, as on a machine without a graphics adapter.
pub const NO_BACKENDS: wgpu::Backends = wgpu::Backends::empty();

/// What a session is started with.
#[derive(Clone, Debug)]
pub struct Settings {
    /// The preset folder; `None` uses the presets the player brings along.
    pub preset_folder: Option<PathBuf>,
    /// Seconds between timed preset changes; `0` turns the timed change off.
    pub preset_seconds: f64,
    /// Whether a timed change picks at random rather than the next in the list.
    pub random_order: bool,
    /// The size of the first frames.
    pub size: (u32, u32),
    /// The graphics APIs that may be used.
    pub backends: wgpu::Backends,
    /// Render on the software adapter even when a real GPU is present.
    pub software_only: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            preset_folder: None,
            preset_seconds: 30.0,
            random_order: true,
            size: (640, 480),
            backends: wgpu::Backends::all(),
            software_only: false,
        }
    }
}

/// A request from the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// The next preset.
    Next,
    /// The preset shown before this one.
    Previous,
    /// Any preset.
    Random,
}

/// How the session stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// The worker is opening the device and loading the first preset.
    Starting,
    /// Frames are being rendered.
    Running {
        /// The preset on the screen.
        preset: String,
    },
    /// Nothing can be rendered: no graphics adapter. The text says so.
    NoGpu(String),
    /// Nothing can be rendered: no usable preset. The text says so.
    NoPresets(String),
    /// Rendering stopped on an error. The text says which.
    Failed(String),
}

/// A finished frame.
#[derive(Debug)]
pub struct RenderedFrame {
    /// Counts up with every frame of the session.
    pub generation: u64,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes of RGBA, top row first.
    pub rgba: Vec<u8>,
}

#[derive(Debug)]
enum Message {
    Audio(Box<VisFrame>, u32),
    Request(Request),
    Resize(u32, u32),
    Quit,
}

struct Shared {
    status: Status,
    frame: Option<Arc<RenderedFrame>>,
    notes: Vec<String>,
}

impl Shared {
    fn note(&mut self, note: String) {
        if self.notes.len() >= MAX_NOTES {
            self.notes.remove(0);
        }
        self.notes.push(note);
    }
}

type SharedState = Arc<Mutex<Shared>>;

/// The lock is only held to copy a value, and a panic in the worker must not
/// take the window down with it.
fn lock(shared: &SharedState) -> MutexGuard<'_, Shared> {
    shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A running visualization. Dropping it stops the worker.
pub struct Session {
    sender: Sender<Message>,
    shared: SharedState,
    worker: Option<JoinHandle<()>>,
}

impl Session {
    /// Starts the worker. This returns at once: see [`Session::status`].
    pub fn start(settings: Settings) -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(AUDIO_QUEUE);
        let shared = Arc::new(Mutex::new(Shared {
            status: Status::Starting,
            frame: None,
            notes: Vec::new(),
        }));
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::Builder::new()
            .name("milkdrop".to_owned())
            .spawn(move || run_guarded(settings, &receiver, &worker_shared));
        match worker {
            Ok(worker) => Self {
                sender,
                shared,
                worker: Some(worker),
            },
            Err(error) => {
                lock(&shared).status =
                    Status::Failed(format!("the render thread could not start: {error}"));
                Self {
                    sender,
                    shared,
                    worker: None,
                }
            }
        }
    }

    /// Hands over one snapshot of the audio; `source_rate_hz` is the decoded
    /// stream's rate. When the worker is behind the snapshot is dropped.
    pub fn feed(&self, frame: &VisFrame, source_rate_hz: u32) {
        match self
            .sender
            .try_send(Message::Audio(Box::new(frame.clone()), source_rate_hz))
        {
            Ok(()) | Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {}
        }
    }

    /// Asks for another preset.
    pub fn request(&self, request: Request) {
        // A worker that has stopped has said why in its status.
        let _ = self.sender.send(Message::Request(request));
    }

    /// Tells the worker the size to render at. A session on the software adapter
    /// renders at a smaller size, and the window scales the picture.
    pub fn resize(&self, width: u32, height: u32) {
        let _ = self.sender.send(Message::Resize(width, height));
    }

    /// How the session stands.
    pub fn status(&self) -> Status {
        lock(&self.shared).status.clone()
    }

    /// The newest frame.
    pub fn frame(&self) -> Option<Arc<RenderedFrame>> {
        lock(&self.shared).frame.clone()
    }

    /// Takes the notes raised since the last call: which adapter, which preset,
    /// what a preset could not have.
    pub fn take_notes(&self) -> Vec<String> {
        std::mem::take(&mut lock(&self.shared).notes)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.sender.send(Message::Quit);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Fits `size` into the longest side the adapter is given, keeping the aspect.
fn render_size(size: (u32, u32), software: bool) -> (u32, u32) {
    let max_side = if software {
        SOFTWARE_MAX_SIDE
    } else {
        MAX_SIDE
    };
    let (width, height) = (size.0.max(1), size.1.max(1));
    let longest = width.max(height);
    if longest <= max_side {
        return (width, height);
    }
    let scale = |side: u32| (u64::from(side) * u64::from(max_side) / u64::from(longest)) as u32;
    (scale(width).max(1), scale(height).max(1))
}

fn run_guarded(settings: Settings, receiver: &Receiver<Message>, shared: &SharedState) {
    let run = std::panic::AssertUnwindSafe(|| run(settings, receiver, shared));
    let stopped = match std::panic::catch_unwind(run) {
        Ok(Ok(())) => return,
        Ok(Err(status)) => status,
        Err(panic) => {
            let what = panic
                .downcast_ref::<&str>()
                .map(|text| (*text).to_owned())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown".to_owned());
            Status::Failed(format!("the renderer stopped unexpectedly: {what}"))
        }
    };
    let mut guard = lock(shared);
    // The window shows the status; a note makes the reason visible in the
    // player's diagnostics as well.
    if let Status::NoGpu(message) | Status::NoPresets(message) | Status::Failed(message) = &stopped
    {
        guard.note(message.clone());
    }
    guard.status = stopped;
    drop(guard);
    // The session stays: the window shows the status until it is closed.
    while !matches!(receiver.recv(), Ok(Message::Quit) | Err(_)) {}
}

fn run(
    settings: Settings,
    receiver: &Receiver<Message>,
    shared: &SharedState,
) -> Result<(), Status> {
    let presets = PresetSet::discover(settings.preset_folder.as_deref())
        .map_err(|error| Status::NoPresets(error.to_string()))?;
    let gpu = Gpu::open(settings.backends, settings.software_only).map_err(|error| {
        Status::NoGpu(format!(
            "MilkDrop needs a graphics adapter (Direct3D 12 or Vulkan), and {error}."
        ))
    })?;
    let software = gpu.software;
    let fps = if software { SOFTWARE_FPS } else { FPS };
    let textures = settings
        .preset_folder
        .as_deref()
        .map(|folder| Box::new(FolderTextures::scan(folder)) as Box<dyn lactea::TextureProvider>);
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |elapsed| elapsed.as_nanos() as u64);
    let mut size = render_size(settings.size, software);
    let mut renderer = Renderer::new(gpu, size, fps, seed, textures)
        .map_err(|error| Status::Failed(format!("the renderer could not start: {error}")))?;
    let device_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let handler_error = Arc::clone(&device_error);
    renderer
        .device()
        .on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
            if let Ok(mut slot) = handler_error.lock() {
                slot.get_or_insert_with(|| error.to_string());
            }
        }));
    let lost_error = Arc::clone(&device_error);
    renderer
        .device()
        .set_device_lost_callback(move |_reason, message| {
            if let Ok(mut slot) = lost_error.lock() {
                slot.get_or_insert(message);
            }
        });
    lock(shared).note(format!(
        "rendering on {} ({})",
        renderer.adapter(),
        if renderer.is_software() {
            "software"
        } else {
            "GPU"
        }
    ));

    let mut control = PresetControl::new(seed);
    control.set_preset_duration(settings.preset_seconds);
    control.set_preset_locked(settings.preset_seconds <= 0.0);
    let mut selector = Selector::new(presets.items(), seed);
    let start = Instant::now();
    let mut time = 0.0;
    switch(
        &mut selector,
        Step::First,
        true,
        &mut renderer,
        &mut control,
        time,
        shared,
    )?;

    let mut feed = PcmFeed::default();
    let mut last_audio = Instant::now();
    let mut frame_number = 0u32;
    let mut generation = 0u64;
    let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
    let mut next_frame = Instant::now();
    loop {
        loop {
            let message = match receiver.recv_deadline(next_frame) {
                Ok(message) => message,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            };
            match message {
                Message::Quit => return Ok(()),
                Message::Audio(frame, rate) => {
                    let pcm = feed.new_samples(&frame, rate);
                    if !pcm.is_empty() {
                        renderer.engine_mut().push_pcm_f32(pcm, 2);
                        last_audio = Instant::now();
                    }
                }
                Message::Request(request) => {
                    let step = match request {
                        Request::Next => Step::Next,
                        Request::Previous => Step::Previous,
                        Request::Random => Step::Random,
                    };
                    switch(
                        &mut selector,
                        step,
                        true,
                        &mut renderer,
                        &mut control,
                        time,
                        shared,
                    )?;
                }
                Message::Resize(width, height) => {
                    size = render_size((width, height), software);
                    renderer.resize(size).map_err(|error| {
                        Status::Failed(format!("the renderer could not resize: {error}"))
                    })?;
                }
            }
        }
        if last_audio.elapsed() >= SILENCE_AFTER {
            feed.reset();
            renderer.engine_mut().push_pcm_f32(&SILENCE, 2);
        }
        time = start.elapsed().as_secs_f64();
        let (width, height, rgba) = renderer
            .render(FrameClock {
                time,
                frame: frame_number,
            })
            .map_err(|error| Status::Failed(format!("a frame could not be rendered: {error}")))?;
        frame_number = frame_number.wrapping_add(1);
        generation += 1;
        if let Some(error) = device_error.lock().ok().and_then(|mut slot| slot.take()) {
            return Err(Status::Failed(format!(
                "the graphics device reported an error: {error}"
            )));
        }
        lock(shared).frame = Some(Arc::new(RenderedFrame {
            generation,
            width,
            height,
            rgba,
        }));
        let vol = renderer.engine_mut().audio_frame().vol;
        if let Some(request) = control.update(time, vol) {
            let step = if settings.random_order {
                Step::Random
            } else {
                Step::Next
            };
            switch(
                &mut selector,
                step,
                request.hard_cut,
                &mut renderer,
                &mut control,
                time,
                shared,
            )?;
        }
        next_frame += interval;
        // A machine that cannot keep the rate does not build up a debt.
        let now = Instant::now();
        if next_frame < now {
            next_frame = now;
        }
    }
}

/// Takes one step through the presets. A list that has run empty ends the
/// session with that message; a step that only failed keeps what is on the screen.
fn switch(
    selector: &mut Selector,
    step: Step,
    hard_cut: bool,
    renderer: &mut Renderer,
    control: &mut PresetControl,
    time: f64,
    shared: &SharedState,
) -> Result<(), Status> {
    let total = selector.len();
    let outcome = selector.step(step, hard_cut, |item, hard| {
        let bytes = PresetSet::read(item).map_err(|error| error.to_string())?;
        let transition = if hard {
            Transition::Cut
        } else {
            Transition::Soft {
                seconds: control.soft_cut_duration(),
            }
        };
        let mut guard = lock(shared);
        renderer.load(
            &bytes,
            PresetSet::display_name(item),
            transition,
            &mut |note| {
                guard.note(note);
            },
        )
    });
    match outcome {
        Outcome::Loaded(item) => {
            if hard_cut {
                control.start_preset(time);
            } else {
                control.start_smoothing(time);
            }
            renderer
                .engine_mut()
                .set_loaded_preset_duration(control.preset_duration_sampled());
            let preset = PresetSet::display_name(&item).to_owned();
            let mut guard = lock(shared);
            guard.note(format!("preset: {preset}"));
            guard.status = Status::Running { preset };
            Ok(())
        }
        Outcome::Failed(reason) if !matches!(lock(shared).status, Status::Starting) => {
            lock(shared).note(format!("no other preset could be loaded: {reason}"));
            Ok(())
        }
        Outcome::Failed(reason) => Err(Status::NoPresets(format!(
            "None of the {total} presets could be loaded. The last reason: {reason}"
        ))),
        Outcome::Empty => Err(Status::NoPresets(format!(
            "None of the {total} presets could be loaded."
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait_for(session: &Session, what: impl Fn(&Status) -> bool) -> Status {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let status = session.status();
            if what(&status) {
                return status;
            }
            assert!(Instant::now() < deadline, "still {status:?}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_size_is_capped_by_the_adapter_and_keeps_its_aspect() {
        assert_eq!(render_size((640, 480), false), (640, 480));
        assert_eq!(render_size((640, 480), true), (640, 480));
        assert_eq!(render_size((1920, 1080), true), (640, 360));
        assert_eq!(render_size((3840, 2160), false), (2560, 1440));
        assert_eq!(render_size((0, 0), false), (1, 1), "a minimized window");
        assert_eq!(render_size((10_000, 1), true), (640, 1), "never zero");
    }

    #[test]
    fn a_machine_without_a_graphics_adapter_gets_a_message_and_no_crash() {
        let session = Session::start(Settings {
            backends: NO_BACKENDS,
            ..Settings::default()
        });
        let status = wait_for(&session, |status| *status != Status::Starting);
        let Status::NoGpu(message) = status else {
            panic!("expected the no-GPU message, got {status:?}");
        };
        assert!(message.contains("graphics adapter"), "{message}");
        // The session still answers, and it closes.
        session.request(Request::Next);
        session.resize(320, 200);
        session.feed(&VisFrame::default(), 44_100);
        assert!(session.frame().is_none());
    }

    #[test]
    fn an_empty_preset_folder_gets_a_message_before_any_gpu_is_touched() {
        let dir = tempfile::tempdir().unwrap();
        let session = Session::start(Settings {
            preset_folder: Some(dir.path().to_path_buf()),
            ..Settings::default()
        });
        let status = wait_for(&session, |status| *status != Status::Starting);
        let Status::NoPresets(message) = status else {
            panic!("expected the no-presets message, got {status:?}");
        };
        assert!(message.contains("holds no MilkDrop presets"), "{message}");
    }

    #[test]
    fn a_full_audio_queue_never_blocks_the_caller() {
        let session = Session::start(Settings {
            backends: NO_BACKENDS,
            ..Settings::default()
        });
        let frame = VisFrame::default();
        let start = Instant::now();
        for _ in 0..AUDIO_QUEUE * 8 {
            session.feed(&frame, 44_100);
        }
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
