//! WASAPI output, in **shared** or **exclusive** mode.
//!
//! Design (see also the report for task card A1):
//!
//! The mode is chosen by the caller on the device itself
//! ([`WasapiOutput::with_mode`]); there is one device type and one
//! [`OutputDevice`] implementation for both modes.
//!
//! * Format, shared mode: 32-bit float at the requested sample rate and
//!   channel count (clamped to 1..=2) with `AUTOCONVERTPCM` enabled. Windows'
//!   audio engine converts to the mix format, so the endpoint always
//!   *grants* the requested format and the engine never has to resample for
//!   WASAPI.
//! * Format, exclusive mode: the stream is opened on the device itself, so
//!   the Windows mixer is not involved and the samples reach the hardware
//!   unconverted. The device's own format is used when it accepts it, and
//!   the engine converts the source to the device's rate; the samples are
//!   then written in the device's own representation (16/24/32-bit integer
//!   or 32-bit float). The period is the device's own, never below the
//!   device's minimum. The rules and the reasoning are in
//!   [`crate::wasapi_exclusive`].
//! * Timing: event driven. A dedicated render thread waits on the audio event,
//!   pulls one buffer through the engine's [`RenderFn`], converts `f32` to
//!   the device's own bytes in a preallocated buffer and hands it to
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
//! Failure handling: a failed `open` names the endpoint that was tried and
//! lists the render endpoints that exist — it never falls back to another
//! device. An endpoint that disappears or is invalidated **mid-run** is latched
//! as a fatal render-thread error ([`OutputDevice::stream_error`], reported by
//! `close`) instead of being spun on silently. A format change is the same
//! event: `AUTOCONVERTPCM` fixes the client's format for its lifetime, so when
//! the endpoint renegotiates its mix format (a device change), Windows
//! invalidates the client and its calls start failing — that surfaces as the
//! same latched loss, and a recovery needs a new `open` (RFC 0012 exposes this
//! to the engine via [`OutputDevice::stream_error`]).
//!
//! An invalidated client is told apart from the other failures: Windows reports
//! it as `AUDCLNT_E_DEVICE_INVALIDATED`, and it is terminal — the client needs
//! a new `open` and never recovers on its own. The *device* behind it may be
//! alive (the system default endpoint changed) or gone (it was unplugged), and
//! that difference decides what the recovery is: a stream that follows the
//! system default continues on the device that is the default now, while a
//! stream bound to a named device can only fail until that device is back. The
//! decision is [`crate::recovery::decide`], and the latched error says which
//! one it is.
//!
//! [`WasapiOutput::with_device_name`] selects one render endpoint by name, like
//! [`crate::CpalOutput::with_device_name`]; an absent or ambiguous name fails
//! `open`.
//!
//! # Exclusive mode and the downgrade
//!
//! A device that refuses the exclusive stream — because another application
//! holds it, or because it accepts no format the player can produce — is
//! played in **shared** mode on the *same* endpoint instead. The downgrade
//! is never silent:
//!
//! * [`WasapiOutput::mode_in_use`] reports the mode the stream really runs
//!   in, and [`OutputDevice::name`] carries it (`"Speakers [wasapi shared]"`),
//!   so a caller that only holds a `Box<dyn OutputDevice>` sees it too;
//! * [`WasapiOutput::downgrade_reason`] says why the exclusive stream was
//!   refused.
//!
//! The downgrade is a *mode* change, not a device change: the endpoint is
//! the one the caller asked for, and the same rule as in shared mode holds —
//! no backend substitutes a different device behind the caller's back.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use rustamp_core::audio::{AudioSpec, OutputDevice, OutputFormat, RenderFn};
use rustamp_core::error::{Error, Result};
use wasapi::{
    AudioClient, CapabilityProbe, Device, DeviceEnumerator, Direction, Handle, SampleType,
    ShareMode, StreamMode, WasapiError, WaveFormat, initialize_mta,
};

use crate::recovery::{self, DeviceFault, OutputSelection};
use crate::wasapi_exclusive::{DeviceFormat, MAX_CHANNELS, SampleLayout};

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
/// The endpoint disappeared or stopped accepting work while the stream ran.
const ERR_GONE: u8 = 5;
/// The audio client was invalidated while the stream ran: the endpoint was
/// removed, the system default endpoint changed, or the endpoint renegotiated
/// its mix format.
const ERR_INVALIDATED: u8 = 6;

/// `AUDCLNT_E_DEVICE_INVALIDATED` (0x88890004).
///
/// Windows invalidates the audio client when the endpoint is removed, when the
/// system default endpoint changes, and when the endpoint renegotiates its mix
/// format. The client never recovers on its own: it needs a new `open`.
const AUDCLNT_E_DEVICE_INVALIDATED: i32 = 0x8889_0004u32 as i32;

/// Consecutive failures of a device call on the render thread that are
/// tolerated before the stream is declared dead. A transient hiccup must not
/// kill playback, but a device that is gone must not be polled forever without
/// anyone learning of it.
const MAX_CONSECUTIVE_FAILURES: u32 = 64;

/// The fault a Windows error code describes, if it describes one.
///
/// Only the invalidation matters for the recovery: it says that the *client*
/// is invalid, not that the device is broken. The device may well be alive
/// (only the default endpoint changed) or gone (it was unplugged) — the
/// recovery decision ([`crate::recovery::decide`]) tells the two apart by the
/// selection.
fn fault_of_hresult(code: i32) -> Option<u8> {
    if code == AUDCLNT_E_DEVICE_INVALIDATED {
        Some(ERR_INVALIDATED)
    } else {
        None
    }
}

/// The code a failed device call latches.
///
/// An invalidated client is terminal — Windows invalidated it for good — so
/// it is latched at once and the render thread stops. Every other failure
/// keeps the caller's own code and the bounded retry streak.
fn fault_code(err: &WasapiError, fallback: u8) -> u8 {
    match err {
        WasapiError::Windows(e) => fault_of_hresult(e.code().0).unwrap_or(fallback),
        _ => fallback,
    }
}

/// The fault a latched error code describes, if it describes a mid-run fault.
///
/// The open-phase codes (`ERR_COM`, `ERR_OPEN`, `ERR_CLIENT`) describe a
/// failed *open*, not a device that died during a run.
fn fault_of_code(code: u8) -> Option<DeviceFault> {
    match code {
        ERR_INVALIDATED => Some(DeviceFault::Invalidated),
        ERR_GONE => Some(DeviceFault::Lost),
        ERR_WRITE => Some(DeviceFault::WriteFailed),
        _ => None,
    }
}

fn error_text(code: u8) -> Option<&'static str> {
    match code {
        ERR_COM => Some("COM could not be initialised"),
        ERR_OPEN => Some("the endpoint could not be opened"),
        ERR_CLIENT => Some("IAudioRenderClient is not available"),
        // The mid-run faults share their wording with the recovery report, so
        // the two can never drift apart.
        _ => fault_of_code(code).map(recovery::fault_text),
    }
}

/// Counts consecutive failures of one device call. The first `tick` at or past
/// `limit` reports the streak as latched; the render thread quits then, so the
/// latched case is seen exactly once.
struct FailureStreak {
    limit: u32,
    count: u32,
}

impl FailureStreak {
    const fn new(limit: u32) -> Self {
        FailureStreak { limit, count: 0 }
    }

    /// Record a failure; `true` once the streak has reached its limit.
    fn tick(&mut self) -> bool {
        self.count += 1;
        self.count >= self.limit
    }

    /// Record a success.
    fn reset(&mut self) {
        self.count = 0;
    }
}

/// Latch a fatal error and ask the render loop to stop.
fn fail(shared: &Shared, code: u8) {
    shared.error.store(code, Ordering::Release);
    shared.quit.store(true, Ordering::Release);
}

/// A failed endpoint open: the error code for `close`, the endpoint name (best
/// effort) and the message for the caller.
struct OpenError {
    code: u8,
    device_name: String,
    message: String,
}

/// Message for a failed endpoint open: it names what was tried and lists the
/// render endpoints that do exist, so the caller can pick another one.
fn open_failure_message(target: &str, cause: &str, alternatives: &[String]) -> String {
    if alternatives.is_empty() {
        format!("opening {target} failed: {cause}; no render endpoint is available")
    } else {
        format!(
            "opening {target} failed: {cause}; available render endpoints: {}",
            alternatives.join(" | ")
        )
    }
}

/// Friendly names of the render endpoints currently present (best effort; a
/// failing enumeration yields an empty list, never an error).
pub(crate) fn endpoint_names(enumerator: &DeviceEnumerator) -> Vec<String> {
    let Ok(collection) = enumerator.get_device_collection(&Direction::Render) else {
        return Vec::new();
    };
    let Ok(count) = collection.get_nbr_devices() else {
        return Vec::new();
    };
    (0..count)
        .filter_map(|index| collection.get_device_at_index(index).ok())
        .filter_map(|device| device.get_friendlyname().ok())
        .collect()
}

/// An [`OpenError`] for a failure on an endpoint that is already known by name.
fn named_open_error(enumerator: &DeviceEnumerator, code: u8, name: &str, cause: &str) -> OpenError {
    OpenError {
        code,
        device_name: name.to_owned(),
        message: open_failure_message(&format!("\"{name}\""), cause, &endpoint_names(enumerator)),
    }
}

/// The one render endpoint whose friendly name matches `wanted` (the rule of
/// [`crate::selection::matches`]: trimmed, case-insensitive, contained in the
/// friendly name), mirroring [`crate::CpalOutput::with_device_name`]. An
/// absent or ambiguous name fails and lists what exists.
fn find_render_endpoint(
    enumerator: &DeviceEnumerator,
    wanted: &str,
) -> std::result::Result<(Device, String), OpenError> {
    let target = format!("the render endpoint named like \"{wanted}\"");
    let collection = enumerator
        .get_device_collection(&Direction::Render)
        .map_err(|e| OpenError {
            code: ERR_OPEN,
            device_name: wanted.to_owned(),
            message: open_failure_message(&target, &e.to_string(), &[]),
        })?;
    let count = collection.get_nbr_devices().unwrap_or(0);
    let mut matches = Vec::new();
    let mut names = Vec::new();
    for index in 0..count {
        let Ok(device) = collection.get_device_at_index(index) else {
            continue;
        };
        let name = device
            .get_friendlyname()
            .unwrap_or_else(|_| format!("endpoint {index}"));
        if crate::selection::matches(wanted, &name) {
            matches.push((device, name.clone()));
        }
        names.push(name);
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        n => {
            let cause = if n == 0 {
                "no render endpoint matches".to_owned()
            } else {
                format!("{n} render endpoints match")
            };
            Err(OpenError {
                code: ERR_OPEN,
                device_name: wanted.to_owned(),
                message: open_failure_message(&target, &cause, &names),
            })
        }
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
    /// The mode the stream was really opened with.
    mode: WasapiMode,
    /// Why the exclusive mode was refused, when it was.
    downgrade: Option<String>,
    outcome: std::result::Result<OutputFormat, String>,
}

/// How a WASAPI stream is opened on the endpoint.
///
/// The mode is requested by the caller on the device
/// ([`WasapiOutput::with_mode`]); there is no second device type for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WasapiMode {
    /// Shared mode: the Windows audio engine owns the endpoint, mixes the
    /// streams of every application and converts their formats. The default.
    #[default]
    Shared,
    /// Exclusive mode: the stream is opened on the device itself, in the
    /// device's own format, and the Windows mixer is not involved.
    ///
    /// A device that refuses the exclusive stream is played in shared mode
    /// instead, and the downgrade is reported — see
    /// [`WasapiOutput::downgrade_reason`].
    Exclusive,
}

impl WasapiMode {
    /// The name of the mode, as it appears in the device name.
    fn label(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Exclusive => "exclusive",
        }
    }
}

/// The name the device reports: the endpoint's name and the mode the stream
/// runs in.
///
/// The mode is part of the name, so a downgrade from exclusive to shared is
/// visible to a caller that only holds a `Box<dyn OutputDevice>`.
fn display_name(endpoint: &str, mode: WasapiMode) -> String {
    format!("{endpoint} [wasapi {}]", mode.label())
}

/// WASAPI output device, in shared or exclusive mode (Windows only).
pub struct WasapiOutput {
    spec: Option<AudioSpec>,
    buffer_frames: u32,
    /// The friendly name of the endpoint, without the mode.
    device_name: String,
    /// The name reported through [`OutputDevice::name`]: the endpoint's name
    /// and the mode the stream runs in.
    display_name: String,
    /// The mode the caller asked for.
    mode: WasapiMode,
    /// The mode the stream really runs in (a refusal downgrades it to
    /// shared).
    mode_in_use: WasapiMode,
    /// Why the exclusive mode was refused, when it was.
    downgrade: Option<String>,
    /// Endpoint to open instead of the default one (`with_device_name`).
    wanted: Option<String>,
    /// Whether `start` has handed a render callback to the render thread.
    started: bool,
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
    ///
    /// The stream is opened in shared mode; use [`WasapiOutput::with_mode`]
    /// to ask for exclusive mode instead.
    pub fn new() -> Self {
        WasapiOutput {
            spec: None,
            buffer_frames: 0,
            device_name: "wasapi".into(),
            display_name: "wasapi".into(),
            mode: WasapiMode::Shared,
            mode_in_use: WasapiMode::Shared,
            downgrade: None,
            wanted: None,
            started: false,
            shared: Arc::new(Shared::new()),
            tx: None,
            handle: None,
        }
    }

    /// Ask for the given mode. Chains after any constructor:
    /// `WasapiOutput::with_device_name("Speakers").with_mode(WasapiMode::Exclusive)`.
    ///
    /// Exclusive mode is a *request*, not a guarantee: a device that refuses
    /// it is played in shared mode, and the downgrade is reported through
    /// [`WasapiOutput::mode_in_use`] and
    /// [`WasapiOutput::downgrade_reason`].
    pub fn with_mode(mut self, mode: WasapiMode) -> Self {
        self.mode = mode;
        self
    }

    /// The mode the caller asked for.
    pub fn requested_mode(&self) -> WasapiMode {
        self.mode
    }

    /// The mode the stream really runs in.
    ///
    /// It is the requested mode, except when the device refused the
    /// exclusive stream — then it is [`WasapiMode::Shared`], and
    /// [`WasapiOutput::downgrade_reason`] says why.
    pub fn mode_in_use(&self) -> WasapiMode {
        self.mode_in_use
    }

    /// Why the exclusive mode was refused, when it was.
    ///
    /// `None` in shared mode, and `None` in an exclusive stream that was
    /// opened as requested.
    pub fn downgrade_reason(&self) -> Option<&str> {
        self.downgrade.as_deref()
    }

    /// Not opened yet; [`OutputDevice::open`] opens the one render endpoint
    /// whose friendly name matches `name` instead of the system default,
    /// like [`crate::CpalOutput::with_device_name`].
    ///
    /// The match is the rule of [`crate::selection::matches`]: the name is
    /// trimmed and compared case-insensitively, and the endpoint's friendly
    /// name must *contain* it ("Speakers" matches "Speakers (Realtek)").
    ///
    /// `open` fails, naming the endpoint and listing the render endpoints that
    /// exist, when none or more than one matches. It never falls back to the
    /// default endpoint.
    pub fn with_device_name(name: impl Into<String>) -> Self {
        let mut this = Self::new();
        this.wanted = Some(name.into());
        this
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

    /// The latched fatal render-thread error, if the stream died (the endpoint
    /// was removed or invalidated, a write failed, ...).
    ///
    /// Delegates to the [`OutputDevice::stream_error`] trait method.
    pub fn stream_error(&self) -> Option<Error> {
        <Self as OutputDevice>::stream_error(self)
    }

    /// How the stream was selected: the system default endpoint, or the device
    /// the caller named.
    ///
    /// The selection decides where a recovery can land, so it is part of the
    /// decision the latched fault is reported with
    /// ([`crate::recovery::decide`]).
    fn selection(&self) -> OutputSelection {
        match self.wanted {
            Some(_) => OutputSelection::Pinned,
            None => OutputSelection::Default,
        }
    }
}

impl OutputDevice for WasapiOutput {
    fn name(&self) -> &str {
        &self.display_name
    }

    fn open(&mut self, spec: AudioSpec) -> Result<OutputFormat> {
        if spec.sample_rate == 0 {
            return Err(Error::InvalidArgument("wasapi: sample rate 0".into()));
        }
        if spec.channels == 0 {
            return Err(Error::InvalidArgument("wasapi: channel count 0".into()));
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
        let want = AudioSpec::new(spec.sample_rate, spec.channels.clamp(1, MAX_CHANNELS));
        let wanted = self.wanted.clone();
        let mode = self.mode;
        let handle = std::thread::Builder::new()
            .name("rustamp-wasapi".into())
            .spawn(move || render_thread(shared, want, wanted, mode, rx, ready_tx))
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
        self.mode_in_use = opened.mode;
        self.downgrade = opened.downgrade;
        self.display_name = display_name(&self.device_name, self.mode_in_use);
        self.spec = Some(format.spec);
        self.buffer_frames = format.buffer_frames;
        self.tx = Some(tx);
        self.handle = Some(handle);
        Ok(format)
    }

    fn start(&mut self, render: RenderFn) -> Result<()> {
        if self.started {
            // The render thread has already left its command loop; a second
            // `Start` would be queued and silently ignored, so refuse it.
            return Err(Error::Output("wasapi: already started".into()));
        }
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| Error::Output("wasapi: not open".into()))?;
        tx.send(Cmd::Start(render))
            .map_err(|_| Error::Output("wasapi: render thread is gone".into()))?;
        self.started = true;
        Ok(())
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

    fn stream_error(&self) -> Option<Error> {
        let code = self.shared.error.load(Ordering::Acquire);
        let text = error_text(code)?;
        let message = match fault_of_code(code) {
            // A device that died during the run: the message says what the
            // recovery is, so the user knows what the next Play will do.
            Some(fault) => recovery::message(fault, self.selection(), &self.device_name),
            // A failed open: the plain reason, named after the device.
            None => format!("{}: {text}", self.device_name),
        };
        Some(Error::Output(message))
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
        self.started = false;
        match error_text(self.shared.error.load(Ordering::Acquire)) {
            Some(e) => Err(Error::Output(format!(
                "wasapi: \"{}\": render thread: {e}",
                self.device_name
            ))),
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
    /// The sample layout the device expects. Shared mode always gets 32-bit
    /// float (Windows converts it), exclusive mode the device's own layout.
    layout: SampleLayout,
    /// Whether the stream is exclusive. An exclusive stream is written one
    /// whole buffer per event, and its first buffer is filled before the
    /// stream starts.
    exclusive: bool,
}

/// What an endpoint open produced.
struct OpenedEndpoint {
    /// The stream that was opened.
    endpoint: Endpoint,
    /// The friendly name of the endpoint.
    name: String,
    /// The mode the stream really runs in.
    mode: WasapiMode,
    /// Why the exclusive mode was refused, when it was.
    downgrade: Option<String>,
}

fn open_endpoint(
    want: AudioSpec,
    wanted: Option<&str>,
    mode: WasapiMode,
) -> std::result::Result<OpenedEndpoint, OpenError> {
    initialize_mta().ok().map_err(|e| OpenError {
        code: ERR_COM,
        device_name: "the default render endpoint".into(),
        message: format!("COM could not be initialised: {e}"),
    })?;
    let enumerator = DeviceEnumerator::new().map_err(|e| OpenError {
        code: ERR_OPEN,
        device_name: "the default render endpoint".into(),
        message: format!("the device enumerator could not be created: {e}"),
    })?;
    let (device, name) = match wanted {
        Some(wanted) => find_render_endpoint(&enumerator, wanted)?,
        None => {
            let device = enumerator
                .get_default_device(&Direction::Render)
                .map_err(|e| OpenError {
                    code: ERR_OPEN,
                    device_name: "the default render endpoint".into(),
                    message: open_failure_message(
                        "the default render endpoint",
                        &e.to_string(),
                        &endpoint_names(&enumerator),
                    ),
                })?;
            let name = device
                .get_friendlyname()
                .unwrap_or_else(|_| "default render endpoint".into());
            (device, name)
        }
    };
    match mode {
        WasapiMode::Shared => {
            let endpoint = open_shared(&device, &enumerator, &name, want)?;
            Ok(OpenedEndpoint {
                endpoint,
                name,
                mode: WasapiMode::Shared,
                downgrade: None,
            })
        }
        WasapiMode::Exclusive => match open_exclusive(&device, want) {
            Ok(endpoint) => Ok(OpenedEndpoint {
                endpoint,
                name,
                mode: WasapiMode::Exclusive,
                downgrade: None,
            }),
            Err(reason) => {
                // The device refuses the exclusive stream: play it in shared
                // mode on the same endpoint and report the downgrade. The
                // endpoint is not changed — only the mode is.
                let endpoint =
                    open_shared(&device, &enumerator, &name, want).map_err(|e| OpenError {
                        code: e.code,
                        device_name: e.device_name,
                        message: format!(
                            "{}; the exclusive stream was refused as well: {reason}",
                            e.message
                        ),
                    })?;
                Ok(OpenedEndpoint {
                    endpoint,
                    name: name.clone(),
                    mode: WasapiMode::Shared,
                    downgrade: Some(format!("{name}: {reason}")),
                })
            }
        },
    }
}

/// Open the endpoint in shared mode: 32-bit float with the automatic format
/// conversion of the Windows audio engine.
///
/// The engine grants the requested format whatever the device's mix format
/// is, so the engine never has to resample for WASAPI.
fn open_shared(
    device: &Device,
    enumerator: &DeviceEnumerator,
    name: &str,
    want: AudioSpec,
) -> std::result::Result<Endpoint, OpenError> {
    let mut client = device
        .get_iaudioclient()
        .map_err(|e| named_open_error(enumerator, ERR_OPEN, name, &e.to_string()))?;
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
        .map_err(|e| named_open_error(enumerator, ERR_OPEN, name, &e.to_string()))?;
    let event = client
        .set_get_eventhandle()
        .map_err(|e| named_open_error(enumerator, ERR_OPEN, name, &e.to_string()))?;
    let render_client = client
        .get_audiorenderclient()
        .map_err(|e| named_open_error(enumerator, ERR_CLIENT, name, &e.to_string()))?;
    let buffer_frames = client
        .get_buffer_size()
        .map_err(|e| named_open_error(enumerator, ERR_OPEN, name, &e.to_string()))?;
    Ok(Endpoint {
        client,
        render_client,
        event,
        spec: want,
        buffer_frames,
        layout: SampleLayout::Float32,
        exclusive: false,
    })
}

/// Open the endpoint in exclusive mode.
///
/// The stream is opened on the device itself, in the device's own format
/// when the device accepts it. The error is the reason why the device
/// refused the exclusive stream; the caller falls back to shared mode and
/// reports it.
fn open_exclusive(device: &Device, want: AudioSpec) -> std::result::Result<Endpoint, String> {
    let mut client = device
        .get_iaudioclient()
        .map_err(|e| format!("the audio client could not be created: {e}"))?;
    // The device's own format: the mix format of the endpoint.
    let mix = client
        .get_mixformat()
        .map_err(|e| format!("the mix format of the device could not be read: {e}"))?;
    let native = DeviceFormat::of_wave(&mix).ok_or_else(|| {
        format!(
            "the device's own format ({} Hz, {} channels, {}-bit) is one the player cannot produce",
            mix.get_samplespersec(),
            mix.get_nchannels(),
            mix.get_bitspersample()
        )
    })?;
    // The formats the device accepts in exclusive mode.
    let accepted = accepted_formats(device, &client, &mix, want)?;
    // The device's periods: the default one, and the smallest it accepts.
    let (default_period, min_period) = client
        .get_device_period()
        .map_err(|e| format!("the device period could not be read: {e}"))?;
    let formats: Vec<DeviceFormat> = accepted.iter().map(|a| a.format).collect();
    let plan = crate::wasapi_exclusive::plan(want, native, &formats, default_period, min_period)
        .ok_or_else(|| "the device accepts no format the player can open exclusively".to_owned())?;
    // The format the device accepted, with the channel mask it accepted.
    let wave = accepted
        .iter()
        .find(|a| a.format == plan.format)
        .map(|a| a.wave.clone())
        .unwrap_or_else(|| wave_format_of(plan.format));
    client
        .initialize_client(
            &wave,
            &Direction::Render,
            &StreamMode::EventsExclusive {
                period_hns: plan.period_hns,
            },
        )
        .map_err(|e| format!("the exclusive stream was refused: {e}"))?;
    let event = client
        .set_get_eventhandle()
        .map_err(|e| format!("the event handle could not be created: {e}"))?;
    let render_client = client
        .get_audiorenderclient()
        .map_err(|e| format!("the render client is not available: {e}"))?;
    let buffer_frames = client
        .get_buffer_size()
        .map_err(|e| format!("the buffer size could not be read: {e}"))?;
    Ok(Endpoint {
        client,
        render_client,
        event,
        spec: plan.format.spec(),
        buffer_frames,
        layout: plan.format.layout,
        exclusive: true,
    })
}

/// A format the device accepts in exclusive mode.
struct AcceptedFormat {
    /// The format to open the stream with, with the channel mask the device
    /// accepted.
    wave: WaveFormat,
    /// The same format as plain data, for the selection.
    format: DeviceFormat,
}

/// The formats the device accepts in exclusive mode.
///
/// The device's own format is asked about first, and the formats it accepts
/// at its own rate — with the channel counts the player drives — and at the
/// source's rate are probed with the capability probe of the `wasapi` crate.
/// The probe only queries the device, it never initialises a client, so it
/// does not disturb anything that is playing.
fn accepted_formats(
    device: &Device,
    client: &AudioClient,
    mix: &WaveFormat,
    want: AudioSpec,
) -> std::result::Result<Vec<AcceptedFormat>, String> {
    let mut accepted: Vec<AcceptedFormat> = Vec::new();
    if let Ok(wave) = client.is_supported_exclusive_with_quirks(mix) {
        push_accepted(&mut accepted, wave);
    }
    let mut probe = CapabilityProbe::new(device)
        .map_err(|e| format!("the exclusive capability probe could not be created: {e}"))?;
    let native_rate = mix.get_samplespersec();
    // The rate and channel count pairs the device is asked about: its own,
    // its own rate with the channel counts the player drives, and the
    // source's own pair.
    let mut pairs: Vec<(u32, u16)> = vec![(native_rate, mix.get_nchannels())];
    for channels in 1..=MAX_CHANNELS {
        pairs.push((native_rate, channels));
    }
    pairs.push((want.sample_rate, want.channels));
    let mut asked: Vec<(u32, u16)> = Vec::new();
    for (rate, channels) in pairs {
        if channels == 0 || asked.contains(&(rate, channels)) {
            continue;
        }
        asked.push((rate, channels));
        for wave in probe.supported_formats(rate as usize, channels as usize) {
            push_accepted(&mut accepted, wave);
        }
    }
    Ok(accepted)
}

/// Add a format the device accepted, unless it is already known.
fn push_accepted(accepted: &mut Vec<AcceptedFormat>, wave: WaveFormat) {
    let Some(format) = DeviceFormat::of_wave(&wave) else {
        return;
    };
    if accepted.iter().any(|a| a.format == format) {
        return;
    }
    accepted.push(AcceptedFormat { wave, format });
}

/// The WASAPI format of a device format.
fn wave_format_of(format: DeviceFormat) -> WaveFormat {
    let (store_bits, valid_bits, sample_type) = match format.layout {
        SampleLayout::Float32 => (32, 32, SampleType::Float),
        SampleLayout::Int16 => (16, 16, SampleType::Int),
        SampleLayout::Int24Packed => (24, 24, SampleType::Int),
        SampleLayout::Int24Padded => (32, 24, SampleType::Int),
        SampleLayout::Int32 => (32, 32, SampleType::Int),
    };
    WaveFormat::new(
        store_bits,
        valid_bits,
        &sample_type,
        format.sample_rate as usize,
        format.channels as usize,
        None,
    )
}

/// The audio thread: negotiate the endpoint, then render until `quit`.
fn render_thread(
    shared: Arc<Shared>,
    want: AudioSpec,
    wanted: Option<String>,
    mode: WasapiMode,
    rx: Receiver<Cmd>,
    ready: Sender<Opened>,
) {
    match open_endpoint(want, wanted.as_deref(), mode) {
        Ok(opened) => {
            let format = OutputFormat {
                spec: opened.endpoint.spec,
                buffer_frames: opened.endpoint.buffer_frames,
            };
            if ready
                .send(Opened {
                    device_name: opened.name,
                    mode: opened.mode,
                    downgrade: opened.downgrade,
                    outcome: Ok(format),
                })
                .is_err()
            {
                return;
            }
            render_loop(shared, opened.endpoint, rx);
        }
        Err(e) => {
            shared.error.store(e.code, Ordering::Release);
            let _ = ready.send(Opened {
                device_name: e.device_name,
                mode,
                downgrade: None,
                outcome: Err(e.message),
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
        layout,
        exclusive,
    } = endpoint;
    let channels = usize::from(spec.channels);
    let mut samples: Vec<f32> = vec![0.0; buffer_frames as usize * channels];
    let mut bytes: Vec<u8> = vec![0u8; samples.len() * layout.bytes_per_sample()];
    let mut failures = FailureStreak::new(MAX_CONSECUTIVE_FAILURES);

    // An exclusive stream is written one whole buffer per event, and its
    // first buffer is filled *before* the stream starts, so the device never
    // plays an empty one. A shared stream is started first and filled with
    // whatever the engine has room for.
    if exclusive {
        write_available(
            &client,
            &render_client,
            &mut render,
            &mut samples,
            &mut bytes,
            &shared,
            &mut failures,
            buffer_frames,
            channels,
            layout,
        );
    }
    // The event only fires once the stream runs, so start it before waiting and
    // fill the first buffer, as the Core Audio documentation asks.
    let mut streaming = client.start_stream().is_ok();
    if streaming && !exclusive {
        write_available(
            &client,
            &render_client,
            &mut render,
            &mut samples,
            &mut bytes,
            &shared,
            &mut failures,
            buffer_frames,
            channels,
            layout,
        );
    }
    // The device's buffer was emptied by a reset and holds no audio yet.
    // The next buffer is then written right away: waiting for the event
    // would wait for the device to ask for data, and a device with an empty
    // buffer does not necessarily ask before it is given something.
    let mut buffer_emptied = false;
    while !shared.quit.load(Ordering::Acquire) {
        if shared.paused.load(Ordering::Acquire) {
            if streaming {
                let _ = client.stop_stream();
                streaming = false;
            }
            if shared.flush.swap(false, Ordering::AcqRel) {
                // Drop what the device still holds (seek / stop).
                // Reset requires the stream to be stopped.
                let _ = client.reset_stream();
                buffer_emptied = true;
            }
            let _ = event.wait_for_event(EVENT_TIMEOUT_MS);
            continue;
        }
        if !streaming {
            if shared.flush.swap(false, Ordering::AcqRel) {
                // Reset requires the stream to be stopped.
                let _ = client.reset_stream();
                buffer_emptied = true;
            }
            match client.start_stream() {
                Ok(()) => {
                    failures.reset();
                    streaming = true;
                }
                Err(err) => {
                    // An invalidated client is terminal: Windows invalidated it
                    // for good, so the stream needs a new `open` and there is
                    // nothing to retry.
                    if fault_code(&err, ERR_GONE) == ERR_INVALIDATED {
                        fail(&shared, ERR_INVALIDATED);
                        return;
                    }
                    // A device that is gone fails here forever; after a bounded
                    // number of tries the stream is declared dead instead of
                    // spinning in silence.
                    if failures.tick() {
                        fail(&shared, ERR_GONE);
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
            }
        }
        if buffer_emptied {
            buffer_emptied = false;
            write_available(
                &client,
                &render_client,
                &mut render,
                &mut samples,
                &mut bytes,
                &shared,
                &mut failures,
                buffer_frames,
                channels,
                layout,
            );
            continue;
        }
        match event.wait_for_event(EVENT_TIMEOUT_MS) {
            Ok(()) => {}
            Err(_) => {
                // No buffer arrived within the timeout: the stream is either
                // quiet or the endpoint is gone. Probing the client tells the
                // two apart; a failing probe is the loss signal.
                match client.get_available_space_in_frames() {
                    Ok(_) => failures.reset(),
                    Err(err) => {
                        // An invalidated client is terminal (see above).
                        if fault_code(&err, ERR_GONE) == ERR_INVALIDATED {
                            fail(&shared, ERR_INVALIDATED);
                            return;
                        }
                        if failures.tick() {
                            fail(&shared, ERR_GONE);
                            return;
                        }
                    }
                }
                continue;
            }
        }
        if shared.quit.load(Ordering::Acquire) {
            break;
        }
        if shared.flush.swap(false, Ordering::AcqRel) {
            // Drop what the device still holds (seek / stop).
            // Reset requires the stream to be stopped.
            if streaming {
                let _ = client.stop_stream();
                let _ = client.reset_stream();
                streaming = client.start_stream().is_ok();
            } else {
                let _ = client.reset_stream();
            }
            // The buffer is empty now, and the write below fills it right
            // away — no event is needed for that.
        }
        write_available(
            &client,
            &render_client,
            &mut render,
            &mut samples,
            &mut bytes,
            &shared,
            &mut failures,
            buffer_frames,
            channels,
            layout,
        );
    }
    if streaming {
        let _ = client.stop_stream();
    }
}

/// Pull the free part of the device buffer through `render` and hand it over.
///
/// In shared mode the free part is what the engine has room for. In
/// exclusive mode the device takes one whole buffer per event, and the
/// buffer is written in the device's own sample layout.
#[allow(clippy::too_many_arguments)]
fn write_available(
    client: &AudioClient,
    render_client: &wasapi::AudioRenderClient,
    render: &mut RenderFn,
    samples: &mut [f32],
    bytes: &mut [u8],
    shared: &Shared,
    failures: &mut FailureStreak,
    buffer_frames: u32,
    channels: usize,
    layout: SampleLayout,
) {
    let available = match client.get_available_space_in_frames() {
        Ok(n) => {
            failures.reset();
            n.min(buffer_frames) as usize
        }
        Err(err) => {
            // An invalidated client is terminal: Windows invalidated it for
            // good, so the stream needs a new `open` and there is nothing to
            // retry.
            if fault_code(&err, ERR_GONE) == ERR_INVALIDATED {
                fail(shared, ERR_INVALIDATED);
                return;
            }
            // A client whose buffer space cannot be read is invalidated; a
            // single failure can be transient, a persistent one is device loss.
            if failures.tick() {
                fail(shared, ERR_GONE);
            }
            return;
        }
    };
    if available == 0 {
        return;
    }
    let n = available * channels;
    render(&mut samples[..n]);
    let width = layout.bytes_per_sample();
    crate::wasapi_exclusive::write_samples(&samples[..n], layout, &mut bytes[..n * width]);
    if let Err(err) = render_client.write_to_device(available, &bytes[..n * width], None) {
        // A write that fails because the client was invalidated is the
        // invalidation, not a write failure: the device may be alive and only
        // the default endpoint changed.
        fail(shared, fault_code(&err, ERR_WRITE));
        return;
    }
    shared.played.fetch_add(available as u64, Ordering::AcqRel);
}

/// Whether a shared-mode render client can be initialised in this process.
/// Used by [`crate::default_output`] to choose the backend, and by the CLI
/// example to fall back to the null output on machines without an audio
/// endpoint (for example CI).
///
/// # Never call this on a thread that will own a window
///
/// This probe initialises COM **on the calling thread, in the multi-threaded
/// apartment**, and it does so *before* it looks for an endpoint — so it
/// consumes the thread's apartment even when it answers `false`. A thread in
/// the MTA can never enter a single-threaded apartment afterwards:
/// `OleInitialize` returns `RPC_E_CHANGED_MODE`, and `winit` panics on that
/// when it creates a window with drag-and-drop support. Callers that may be on
/// a UI thread go through [`crate::default_output`], which runs this probe on a
/// thread of its own; the real endpoint is opened by [`render_thread`], which
/// likewise initialises COM only for itself.
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

#[cfg(test)]
mod tests {
    use super::*;

    // These tests exercise only pure helpers: they open no device and start no
    // audio, so they are safe on any machine.

    #[test]
    fn an_open_failure_names_the_endpoint_and_lists_the_alternatives() {
        let alternatives = vec!["Speakers (Realtek)".to_owned(), "Headphones".to_owned()];
        let message =
            open_failure_message("\"CABLE Input\"", "the endpoint is in use", &alternatives);
        assert!(message.contains("CABLE Input"), "{message}");
        assert!(message.contains("in use"), "{message}");
        assert!(message.contains("Realtek"), "{message}");
        assert!(message.contains("Headphones"), "{message}");
    }

    #[test]
    fn an_open_failure_without_alternatives_says_so() {
        let message = open_failure_message("the default render endpoint", "denied", &[]);
        assert!(message.contains("default render endpoint"), "{message}");
        assert!(message.contains("no render endpoint"), "{message}");
    }

    #[test]
    fn device_loss_has_its_own_message() {
        let gone = error_text(ERR_GONE).expect("ERR_GONE has text");
        assert!(gone.contains("device"), "{gone}");
        assert_ne!(error_text(ERR_GONE), error_text(ERR_WRITE));
        // Every code `close` can report has a message.
        for code in [
            ERR_NONE,
            ERR_COM,
            ERR_OPEN,
            ERR_CLIENT,
            ERR_WRITE,
            ERR_GONE,
            ERR_INVALIDATED,
        ] {
            assert_eq!(error_text(code).is_some(), code != ERR_NONE);
        }
    }

    #[test]
    fn an_invalidated_client_is_its_own_fault() {
        // Windows invalidates the client when the endpoint is removed, when the
        // system default endpoint changes, and when the endpoint renegotiates
        // its mix format. The client never recovers on its own: it needs a new
        // `open`, so the failure is latched at once instead of being retried.
        assert_eq!(
            fault_of_hresult(AUDCLNT_E_DEVICE_INVALIDATED),
            Some(ERR_INVALIDATED)
        );
        // Any other Windows error keeps the caller's own code.
        assert_eq!(fault_of_hresult(0x8000_0001u32.cast_signed()), None);
        assert_eq!(
            fault_of_code(ERR_INVALIDATED),
            Some(DeviceFault::Invalidated),
            "the invalidation is the fault the recovery decides on"
        );
    }

    #[test]
    fn the_latched_error_names_the_recovery() {
        // A stream that follows the system default continues on the device
        // that is the default now, so the message says so.
        let out = WasapiOutput::new();
        fail(&out.shared, ERR_INVALIDATED);
        let err = OutputDevice::stream_error(&out).expect("latched error");
        let msg = err.to_string();
        assert!(msg.contains("invalidated"), "{msg}");
        assert!(
            msg.contains("press Play to continue on the current default device"),
            "{msg}"
        );
    }

    #[test]
    fn a_pinned_device_cannot_continue_on_the_default() {
        // The user named the device, and the backend never substitutes another
        // one behind their back: the recovery is the device coming back, or
        // the user choosing another one.
        let out = WasapiOutput::with_device_name("CABLE Input");
        fail(&out.shared, ERR_GONE);
        let err = OutputDevice::stream_error(&out).expect("latched error");
        let msg = err.to_string();
        assert!(msg.contains("disappeared"), "{msg}");
        assert!(msg.contains("choose another device"), "{msg}");
        assert!(
            !msg.contains("current default device"),
            "a pinned device never continues on the default: {msg}"
        );
    }

    #[test]
    fn a_failure_streak_latches_only_at_the_limit() {
        let mut streak = FailureStreak::new(3);
        assert!(!streak.tick());
        assert!(!streak.tick());
        assert!(streak.tick(), "the limit is reached");
        streak.reset();
        assert!(!streak.tick(), "a success clears the streak");
    }

    #[test]
    fn open_rejects_an_impossible_format_before_touching_a_device() {
        let mut out = WasapiOutput::new();
        assert!(matches!(
            out.open(AudioSpec::new(0, 2)),
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            out.open(AudioSpec::new(44100, 0)),
            Err(Error::InvalidArgument(_))
        ));
        assert_eq!(out.played_frames(), 0);
        assert!(out.stream_error().is_none());
    }

    #[test]
    fn wasapi_stream_error_reflects_failure_streak() {
        let out = WasapiOutput::new();
        assert!(OutputDevice::stream_error(&out).is_none());
        let mut streak = FailureStreak::new(MAX_CONSECUTIVE_FAILURES);
        for _ in 0..MAX_CONSECUTIVE_FAILURES - 1 {
            assert!(!streak.tick());
        }
        assert!(streak.tick());
        fail(&out.shared, ERR_GONE);
        let err = OutputDevice::stream_error(&out).expect("latched error");
        let msg = err.to_string();
        assert!(msg.contains("wasapi"), "{msg}");
        assert!(msg.contains("device"), "{msg}");
    }

    #[test]
    fn the_default_mode_is_shared() {
        let out = WasapiOutput::new();
        assert_eq!(out.requested_mode(), WasapiMode::Shared);
        assert_eq!(out.mode_in_use(), WasapiMode::Shared);
        assert_eq!(out.downgrade_reason(), None);
    }

    #[test]
    fn the_exclusive_mode_is_a_request_on_the_device() {
        let out = WasapiOutput::new().with_mode(WasapiMode::Exclusive);
        assert_eq!(out.requested_mode(), WasapiMode::Exclusive);
    }

    #[test]
    fn the_mode_survives_the_device_name_selection() {
        let out = WasapiOutput::with_device_name("Speakers").with_mode(WasapiMode::Exclusive);
        assert_eq!(out.requested_mode(), WasapiMode::Exclusive);
    }

    #[test]
    fn the_device_name_carries_the_mode_in_use() {
        // The mode is part of the name, so a downgrade from exclusive to
        // shared is visible to a caller that only holds the trait object.
        assert_eq!(
            display_name("Speakers (Realtek)", WasapiMode::Shared),
            "Speakers (Realtek) [wasapi shared]"
        );
        assert_eq!(
            display_name("Speakers (Realtek)", WasapiMode::Exclusive),
            "Speakers (Realtek) [wasapi exclusive]"
        );
    }
}
