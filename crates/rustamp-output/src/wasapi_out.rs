//! WASAPI output in **shared** mode.
//!
//! Design (see also the report for task card A1):
//!
//! * Format: 32-bit float at the requested sample rate and channel count
//!   (clamped to 1..=2) with `AUTOCONVERTPCM` enabled. Windows' audio engine
//!   converts to the mix format, so the endpoint always *grants* the requested
//!   format and the engine never has to resample for WASAPI. That is what
//!   shared mode is for; exclusive mode is an M4 item.
//! * Timing: event driven. A dedicated render thread waits on the audio event,
//!   pulls one buffer through the engine's [`RenderFn`], converts `f32` to
//!   little-endian bytes in a preallocated buffer and hands it to
//!   `IAudioRenderClient`. **No allocation, no lock, no I/O, no logging on
//!   that thread.**
//! * Threading: the endpoint interfaces are *not* `Send` in `wasapi` 0.26, so
//!   the render thread creates and owns every COM object. `open` spawns the
//!   thread, which negotiates the format and reports it back; `start` hands the
//!   render callback over a channel. Nothing but plain atomics crosses the
//!   thread boundary, so this file needs no `unsafe`.
//! * `played_frames` counts the frames handed to the device buffer (the
//!   "frames rendered" clock), which is what the engine's position model
//!   expects. It is never reset by `flush`, as RFC 0001 requires.
//!
//! Known limitation (M1): device removal and default-device changes are not
//! handled; the render thread exits and `close` reports the failure.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use rustamp_core::audio::{AudioSpec, OutputDevice, OutputFormat, RenderFn};
use rustamp_core::error::{Error, Result};
use wasapi::{
    AudioClient, DeviceEnumerator, Direction, Handle, SampleType, ShareMode, StreamMode,
    WaveFormat, initialize_mta,
};

/// Channel count requested from WASAPI. More channels are downmixed by the
/// engine; the mix format is stereo on virtually every endpoint.
const MAX_CHANNELS: usize = 2;

/// Requested buffer duration in 100 ns units (200 ms). The audio engine may
/// shorten it; it only bounds the scratch buffer we preallocate.
const BUFFER_DURATION_HNS: i64 = 2_000_000;

/// `wait_for_event` timeout in ms; the thread only re-checks its flags then.
const EVENT_TIMEOUT_MS: u32 = 500;

/// How long the render thread waits for the render callback (or `quit`) after
/// it has opened the endpoint.
const START_TIMEOUT_MS: u64 = 100;

/// Fatal render-thread error codes, reported by `close`.
const ERR_NONE: u8 = 0;
const ERR_COM: u8 = 1;
const ERR_OPEN: u8 = 2;
const ERR_CLIENT: u8 = 3;
const ERR_WRITE: u8 = 4;

fn error_text(code: u8) -> Option<&'static str> {
    match code {
        ERR_COM => Some("COM could not be initialised"),
        ERR_OPEN => Some("the endpoint could not be opened"),
        ERR_CLIENT => Some("IAudioRenderClient is not available"),
        ERR_WRITE => Some("writing to the device failed"),
        _ => None,
    }
}

/// State shared between the control side and the render thread. Atomics only.
struct Shared {
    quit: AtomicBool,
    paused: AtomicBool,
    flush: AtomicBool,
    played: AtomicU64,
    error: AtomicU8,
}

impl Shared {
    fn new() -> Self {
        Shared {
            quit: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            flush: AtomicBool::new(false),
            played: AtomicU64::new(0),
            error: AtomicU8::new(ERR_NONE),
        }
    }
}

/// Messages from the control side to the render thread.
enum Cmd {
    /// Hand over the engine's render callback; playback starts.
    Start(RenderFn),
    /// Stop the thread.
    Quit,
}

/// Result of the negotiation, sent from the render thread to `open`.
struct Opened {
    device_name: String,
    outcome: std::result::Result<OutputFormat, String>,
}

/// WASAPI shared-mode output device (Windows only).
pub struct WasapiOutput {
    spec: Option<AudioSpec>,
    buffer_frames: u32,
    device_name: String,
    shared: Arc<Shared>,
    tx: Option<Sender<Cmd>>,
    handle: Option<JoinHandle<()>>,
}

impl Default for WasapiOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl WasapiOutput {
    /// Not opened yet; call [`OutputDevice::open`].
    pub fn new() -> Self {
        WasapiOutput {
            spec: None,
            buffer_frames: 0,
            device_name: "wasapi".into(),
            shared: Arc::new(Shared::new()),
            tx: None,
            handle: None,
        }
    }

    /// Open the default render endpoint in shared mode.
    pub fn open_default(spec: AudioSpec) -> Result<Self> {
        let mut this = Self::new();
        this.open(spec)?;
        Ok(this)
    }

    /// Friendly name of the endpoint, once opened.
    pub fn device_name(&self) -> &str {
        &self.device_name
    }
}

impl OutputDevice for WasapiOutput {
    fn name(&self) -> &str {
        &self.device_name
    }

    fn open(&mut self, spec: AudioSpec) -> Result<OutputFormat> {
        if spec.sample_rate == 0 {
            return Err(Error::InvalidArgument("sample rate 0".into()));
        }
        if self.handle.is_some() {
            return Err(Error::Output("wasapi: already open".into()));
        }
        self.shared.quit.store(false, Ordering::Release);
        self.shared.paused.store(false, Ordering::Release);
        self.shared.flush.store(false, Ordering::Release);
        self.shared.played.store(0, Ordering::Release);
        self.shared.error.store(ERR_NONE, Ordering::Release);

        let (tx, rx) = std::sync::mpsc::channel::<Cmd>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Opened>();
        let shared = Arc::clone(&self.shared);
        let want = AudioSpec::new(
            spec.sample_rate,
            spec.channels.clamp(1, MAX_CHANNELS as u16),
        );
        let handle = std::thread::Builder::new()
            .name("rustamp-wasapi".into())
            .spawn(move || render_thread(shared, want, rx, ready_tx))
            .map_err(|e| Error::Output(format!("wasapi: render thread: {e}")))?;
        let opened = ready_rx
            .recv()
            .map_err(|_| Error::Output("wasapi: render thread died while opening".into()))?;
        self.device_name = opened.device_name;
        let format = match opened.outcome {
            Ok(f) => f,
            Err(e) => {
                self.shared.quit.store(true, Ordering::Release);
                let _ = handle.join();
                return Err(Error::Output(format!("wasapi: {e}")));
            }
        };
        self.spec = Some(format.spec);
        self.buffer_frames = format.buffer_frames;
        self.tx = Some(tx);
        self.handle = Some(handle);
        Ok(format)
    }

    fn start(&mut self, render: RenderFn) -> Result<()> {
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| Error::Output("wasapi: not open".into()))?;
        tx.send(Cmd::Start(render))
            .map_err(|_| Error::Output("wasapi: render thread is gone".into()))
    }

    fn pause(&mut self) -> Result<()> {
        self.shared.paused.store(true, Ordering::Release);
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        self.shared.paused.store(false, Ordering::Release);
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        // The render thread performs the actual `IAudioClient::Reset`, so it
        // cannot race with a buffer that is being filled.
        self.shared.flush.store(true, Ordering::Release);
        Ok(())
    }

    fn latency(&self) -> Duration {
        match self.spec {
            Some(spec) if spec.sample_rate > 0 => {
                spec.duration_of(u64::from(self.buffer_frames.max(1)))
            }
            _ => Duration::ZERO,
        }
    }

    fn played_frames(&self) -> u64 {
        self.shared.played.load(Ordering::Acquire)
    }

    fn close(&mut self) -> Result<()> {
        self.shared.quit.store(true, Ordering::Release);
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Cmd::Quit);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.spec = None;
        match error_text(self.shared.error.load(Ordering::Acquire)) {
            Some(e) => Err(Error::Output(format!("wasapi: render thread: {e}"))),
            None => Ok(()),
        }
    }
}

impl Drop for WasapiOutput {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// Everything the endpoint needs, created on the render thread.
struct Endpoint {
    client: AudioClient,
    render_client: wasapi::AudioRenderClient,
    event: Handle,
    spec: AudioSpec,
    buffer_frames: u32,
}

fn open_endpoint(want: AudioSpec) -> std::result::Result<(Endpoint, String), (u8, String)> {
    initialize_mta()
        .ok()
        .map_err(|e| (ERR_COM, format!("COM init failed: {e}")))?;
    let enumerator = DeviceEnumerator::new().map_err(|e| (ERR_OPEN, e.to_string()))?;
    let device = enumerator
        .get_default_device(&Direction::Render)
        .map_err(|e| (ERR_OPEN, e.to_string()))?;
    let name = device
        .get_friendlyname()
        .unwrap_or_else(|_| "default endpoint".into());
    let mut client = device
        .get_iaudioclient()
        .map_err(|e| (ERR_OPEN, e.to_string()))?;
    let format = WaveFormat::new(
        32,
        32,
        &SampleType::Float,
        want.sample_rate as usize,
        usize::from(want.channels),
        None,
    );
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: BUFFER_DURATION_HNS,
    };
    client
        .initialize_client(&format, &Direction::Render, &mode)
        .map_err(|e| (ERR_OPEN, e.to_string()))?;
    let event = client
        .set_get_eventhandle()
        .map_err(|e| (ERR_OPEN, e.to_string()))?;
    let render_client = client
        .get_audiorenderclient()
        .map_err(|e| (ERR_CLIENT, e.to_string()))?;
    let buffer_frames = client
        .get_buffer_size()
        .map_err(|e| (ERR_OPEN, e.to_string()))?;
    Ok((
        Endpoint {
            client,
            render_client,
            event,
            spec: want,
            buffer_frames,
        },
        name,
    ))
}

/// The audio thread: negotiate the endpoint, then render until `quit`.
fn render_thread(shared: Arc<Shared>, want: AudioSpec, rx: Receiver<Cmd>, ready: Sender<Opened>) {
    match open_endpoint(want) {
        Ok((endpoint, name)) => {
            let format = OutputFormat {
                spec: endpoint.spec,
                buffer_frames: endpoint.buffer_frames,
            };
            if ready
                .send(Opened {
                    device_name: name,
                    outcome: Ok(format),
                })
                .is_err()
            {
                return;
            }
            render_loop(shared, endpoint, rx);
        }
        Err((code, message)) => {
            shared.error.store(code, Ordering::Release);
            let _ = ready.send(Opened {
                device_name: "wasapi".into(),
                outcome: Err(message),
            });
        }
    }
}

/// Wait for the render callback, then stream. Must not allocate, lock or log.
fn render_loop(shared: Arc<Shared>, endpoint: Endpoint, rx: Receiver<Cmd>) {
    let mut render = loop {
        if shared.quit.load(Ordering::Acquire) {
            return;
        }
        match rx.recv_timeout(Duration::from_millis(START_TIMEOUT_MS)) {
            Ok(Cmd::Start(render)) => break render,
            Ok(Cmd::Quit) => return,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    };

    let Endpoint {
        client,
        render_client,
        event,
        spec,
        buffer_frames,
    } = endpoint;
    let channels = usize::from(spec.channels);
    let mut samples: Vec<f32> = vec![0.0; buffer_frames as usize * channels];
    let mut bytes: Vec<u8> = vec![0u8; samples.len() * 4];

    // The event only fires once the stream runs, so start it before waiting and
    // fill the first buffer, as the Core Audio documentation asks.
    let mut streaming = client.start_stream().is_ok();
    if streaming {
        write_available(
            &client,
            &render_client,
            &mut render,
            &mut samples,
            &mut bytes,
            &shared,
            buffer_frames,
            channels,
        );
    }
    while !shared.quit.load(Ordering::Acquire) {
        if shared.paused.load(Ordering::Acquire) {
            if streaming {
                let _ = client.stop_stream();
                streaming = false;
            }
            let _ = event.wait_for_event(EVENT_TIMEOUT_MS);
            continue;
        }
        if !streaming {
            if client.start_stream().is_err() {
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            streaming = true;
        }
        match event.wait_for_event(EVENT_TIMEOUT_MS) {
            Ok(()) => {}
            Err(_) => continue,
        }
        if shared.quit.load(Ordering::Acquire) {
            break;
        }
        if shared.flush.swap(false, Ordering::AcqRel) {
            // Drop what the device still holds (seek / stop).
            let _ = client.reset_stream();
        }
        write_available(
            &client,
            &render_client,
            &mut render,
            &mut samples,
            &mut bytes,
            &shared,
            buffer_frames,
            channels,
        );
    }
    if streaming {
        let _ = client.stop_stream();
    }
}

/// Pull the free part of the device buffer through `render` and hand it over.
#[allow(clippy::too_many_arguments)]
fn write_available(
    client: &AudioClient,
    render_client: &wasapi::AudioRenderClient,
    render: &mut RenderFn,
    samples: &mut [f32],
    bytes: &mut [u8],
    shared: &Shared,
    buffer_frames: u32,
    channels: usize,
) {
    let available = match client.get_available_space_in_frames() {
        Ok(n) => n.min(buffer_frames) as usize,
        Err(_) => return,
    };
    if available == 0 {
        return;
    }
    let n = available * channels;
    render(&mut samples[..n]);
    for (dst, src) in bytes[..n * 4]
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(samples[..n].iter())
    {
        dst.copy_from_slice(&src.to_le_bytes());
    }
    if render_client
        .write_to_device(available, &bytes[..n * 4], None)
        .is_err()
    {
        shared.error.store(ERR_WRITE, Ordering::Release);
        shared.quit.store(true, Ordering::Release);
        return;
    }
    shared.played.fetch_add(available as u64, Ordering::AcqRel);
}

/// Whether a shared-mode render client can be initialised in this process.
/// Used by the CLI example to fall back to the null output on machines without
/// an audio endpoint (for example CI).
pub fn is_shared_mode_available() -> bool {
    if initialize_mta().ok().is_err() {
        return false;
    }
    let Ok(enumerator) = DeviceEnumerator::new() else {
        return false;
    };
    let Ok(device) = enumerator.get_default_device(&Direction::Render) else {
        return false;
    };
    let Ok(mut client) = device.get_iaudioclient() else {
        return false;
    };
    let format = WaveFormat::new(32, 32, &SampleType::Float, 44100, 2, None);
    let mode = StreamMode::EventsShared {
        autoconvert: true,
        buffer_duration_hns: BUFFER_DURATION_HNS,
    };
    client
        .initialize_client(&format, &Direction::Render, &mode)
        .is_ok()
        && client.get_sharemode() == Some(ShareMode::Shared)
}
