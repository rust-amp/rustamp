//! Engine-side decoder-resident adapter (RFC 0006 §5).
//!
//! This module presents a media-host [`DecoderSession`] as the core
//! [`Decoder`] trait the engine already drives. The decode thread
//! (`crate::decode`) keeps opening decoders through its
//! [`DecoderRegistry`] and keeps running the same gapless loop, seek drain,
//! format-change flush and audio-clock bookkeeping: a resident track is just
//! another decoder behind the same trait object. There is deliberately **no**
//! second decode path in the engine.
//!
//! ```text
//! in-process: registry -> symphonia Decoder -> DSP -> converter -> ring
//! resident:   registry -> ResidentFactory -> helper process -> PCM ring
//!             -> HostDecoder/ProcessDecoder -> DSP -> converter -> ring
//! ```
//!
//! # Opt-in, never a silent default
//!
//! The default registry (`rustamp_decoders::default_registry`) contains no
//! resident factory, so a default engine decodes exactly as before. To play
//! through the resident, the caller registers a [`ResidentFactory`] holding an
//! explicit [`ResidentConfig`] (which names the helper binary). Registration
//! order decides ties (RFC 0013): register the resident factory first to
//! prefer it, last to prefer in-process, or alone to force it. A missing
//! helper binary degrades to "format unsupported"; a refusal or a death is a
//! loud decode error, never silence and never end-of-stream.
//!
//! # Threading
//!
//! [`HostDecoder`] and [`ProcessDecoder`] block with bounded timeouts and take
//! ordinary locks inside the control peer. They belong on the decode worker
//! only — never on the render callback (no blocking, no locks there).
//!
//! # Timeouts (own choices)
//!
//! The wire speaks milliseconds, so seeks quantize to whole milliseconds and
//! durations round to them; sub-millisecond precision of an in-process seek is
//! not preserved. [`DEFAULT_READ_TIMEOUT`] and [`DEFAULT_SEEK_TIMEOUT`] bound
//! every blocking wait so a wedged resident fails the track instead of hanging
//! the decode thread. Prefetch defaults to [`DEFAULT_PREFETCH_FRAMES`] frames
//! of ring capacity hint.
//!
//! # The DSP resident (RFC 0006 §6)
//!
//! The same seam one level down: [`HostDspChain`] presents a media-host
//! [`DspSession`] as the core [`DspStage`] the engine already drives, so the
//! decode thread's gapless loop, seek drain, format-change flush and latency
//! trim/flush are unchanged — a resident chain is just another stage behind
//! the same trait object, and there is deliberately no second DSP path in the
//! engine. [`DspResidentConfig::open`] spawns the helper and returns the stage
//! to install with [`Engine::set_dsp_chain`](crate::engine::Engine::set_dsp_chain);
//! the default engine chain is empty, so the resident is opt-in only, never a
//! silent default.
//!
//! # Latency compensation
//!
//! The stage reports the resident's `DspReady` latency through
//! [`DspStage::latency_frames`], and the decode thread compensates it exactly
//! like an in-process stage's (RFC 0006 §6.2): after every `prepare`/`reset`
//! the first `latency` frames the chain emits are dropped, and at the end of
//! every track `latency` frames of silence are pushed through so the audio
//! the chain still holds comes out. Because the adapter's `process` is one
//! synchronous block round-trip (write the block, read the same block back,
//! delayed by the chain's latency), that machinery is correct across a seek
//! (`reset` + trim) and across a gapless seam (flush at the track end + trim
//! at the next track's start) with no extra wiring: no frame is lost, none is
//! duplicated and no silence enters either seam. If the resident's report
//! disagrees with reality (a lying or buggy stage), the compensation trims or
//! flushes the wrong number of frames — the error is bounded by the reported
//! latency and shows up as a small shift or a short tail, never as a hang or
//! a crash; the report is trusted because RFC 0006 §6.2 makes it the
//! contract, exactly as for an in-process stage's `latency_frames`.
//!
//! # Mid-run failures
//!
//! The core `DspStage::process` contract is infallible (M14 finding F1; RFC
//! 0014 is still a draft), so a resident that dies or wedges mid-stream cannot
//! return an error per block. The adapter latches the first failure into a
//! [`DspFailureWatch`] — pass the engine's
//! [`Engine::resident_dsp_watch`](crate::engine::Engine::resident_dsp_watch)
//! when building the chain — and passes audio through bit-exactly from then
//! on; [`Engine::poll`](crate::engine::Engine::poll) takes the latch and
//! fails the track: the resident's death fails the track, never the player
//! (RFC 0006 §8). A refusal at configure is a loud error at
//! [`DspResidentConfig::open`] (or at `prepare` for a reconfigure), before or
//! instead of playback; a missing helper binary degrades to "stage
//! unavailable", never to a silent in-process fallback.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rustamp_core::audio::{
    AudioSpec, Confidence, Decoder, DecoderFactory, DspStage, MediaSource, ProbeHint,
};
use rustamp_core::error::{Error as CoreError, ErrorInfo, Result as CoreResult};
use rustamp_core::state::StreamInfo;
use rustamp_mediahost::decoder::{DecoderSession, file_location};
use rustamp_mediahost::dsp::DspSession;
use rustamp_mediahost::pcm::PcmRings;
use rustamp_mediahost::{MediaHostError, Result as HostResult};
use rustamp_proto::host::{DspStageDesc, MediaLocationWire, PcmSpecWire};

/// Ring-capacity hint sent with `DecoderOpen` when the caller did not pick one.
///
/// Own choice: about 185 ms of stereo at 44.1 kHz, large enough that the host
/// pump rarely blocks behind a full ring, small enough to keep seeks snappy.
pub const DEFAULT_PREFETCH_FRAMES: u32 = 8192;

/// Bounded wait for one blocking PCM read from the resident.
///
/// Own choice: long enough for a loaded helper to answer, short enough that a
/// wedged resident fails the track instead of hanging the decode thread. This
/// is a per-`read` bound; a healthy stream never reaches it because data, EOS
/// or a terminal failure wakes the wait first.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Bounded wait for the seek handshake (`DecoderSeek` … `DecoderSeeked`).
///
/// Own choice, same rationale as [`DEFAULT_READ_TIMEOUT`].
pub const DEFAULT_SEEK_TIMEOUT: Duration = Duration::from_secs(5);

/// File extensions the resident factory claims, mirroring what the symphonia
/// resident behind the helper serves today (MP3/FLAC/Vorbis/WAV/AIFF/CAF/
/// AAC/ALAC/MKA).
///
/// Own choice: the list exists so file dialogs and callers can see what the
/// resident is for; probing itself only checks the extension hint (see
/// [`ResidentFactory::probe`]).
pub const RESIDENT_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "wav", "aiff", "aif", "caf", "m4a", "aac", "alac", "mka",
];

/// How to reach and bound one decoder-resident helper process.
///
/// The helper binary is named explicitly: there is no `PATH` lookup and no
/// default, so a missing binary is a loud, diagnosable failure rather than an
/// accidental silent fallback.
#[derive(Debug, Clone)]
pub struct ResidentConfig {
    /// Absolute path of the resident helper binary (for example the symphonia
    /// resident next to `rustamp.exe`).
    pub helper: PathBuf,
    /// Ring-capacity hint sent with `DecoderOpen`.
    pub prefetch_frames: u32,
    /// Per-`read` bound; a wedged resident fails the track after this.
    pub read_timeout: Duration,
    /// Bound for one seek handshake.
    pub seek_timeout: Duration,
}

impl ResidentConfig {
    /// Opt in to resident decoding through the helper at `helper`.
    pub fn new(helper: PathBuf) -> Self {
        ResidentConfig {
            helper,
            prefetch_frames: DEFAULT_PREFETCH_FRAMES,
            read_timeout: DEFAULT_READ_TIMEOUT,
            seek_timeout: DEFAULT_SEEK_TIMEOUT,
        }
    }

    /// Override the ring-capacity hint (must stay above zero; zero is
    /// rejected loudly when the decoder is opened).
    pub fn with_prefetch_frames(mut self, prefetch_frames: u32) -> Self {
        self.prefetch_frames = prefetch_frames;
        self
    }

    /// Override both blocking bounds.
    pub fn with_timeouts(mut self, read_timeout: Duration, seek_timeout: Duration) -> Self {
        self.read_timeout = read_timeout;
        self.seek_timeout = seek_timeout;
        self
    }
}

/// Core [`AudioSpec`] for a wire [`PcmSpecWire`].
///
/// Infallible: the wire holds `u8` channels and a `u32` rate, both of which
/// fit the core spec. A zero rate or zero channels never reaches here — the
/// session validates the handshake before the adapter is built.
fn audio_spec_from_wire(spec: PcmSpecWire) -> AudioSpec {
    AudioSpec::new(spec.sample_rate_hz, u16::from(spec.channels))
}

/// Saturating [`Duration`] to whole wire milliseconds.
fn ms_from_duration(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// A media-host failure as a loud core error.
///
/// Never silence and never end-of-stream: every variant becomes either a
/// decode failure, an unsupported-format/misuse error or a format change the
/// caller already translated. Timeouts and disconnects (a wedged or dead
/// resident) are decode errors so the decode thread reports them on the bus
/// and fails the track, never the player.
fn host_error_to_core(error: MediaHostError, context: &str) -> CoreError {
    match error {
        MediaHostError::TimedOut => CoreError::Decode(format!("resident {context} timed out")),
        MediaHostError::Disconnected(message) => {
            CoreError::Decode(format!("resident {context} disconnected: {message}"))
        }
        MediaHostError::OperationFailed(message) => {
            CoreError::Decode(format!("resident {context} failed: {message}"))
        }
        MediaHostError::InvalidMessage(message) => {
            CoreError::Decode(format!("resident {context} protocol violation: {message}"))
        }
        MediaHostError::NotImplemented(message) => {
            CoreError::Decode(format!("resident {context} not implemented: {message}"))
        }
        MediaHostError::Ring(error) => {
            CoreError::Decode(format!("resident {context} ring failure: {error}"))
        }
        MediaHostError::FormatChanged => {
            CoreError::Decode(format!("resident {context} unexpected format signal"))
        }
    }
}

/// Translate one blocking session read into core [`Decoder::read`] semantics.
///
/// `spec` is the adapter's cached format: on [`MediaHostError::FormatChanged`]
/// it is refreshed from the session (which already switched) and the core
/// `FormatChanged { from, to }` carries both sides, exactly like an
/// in-process decoder. `Ok(0)` is the session's sticky drained-EOS and is
/// forwarded unchanged.
fn convert_read<F: PcmRings>(
    session: &mut DecoderSession<F>,
    out: &mut [f32],
    read_timeout: Duration,
    spec: &mut AudioSpec,
) -> CoreResult<usize> {
    let channels = usize::from(spec.channels.max(1));
    if out.is_empty() || !out.len().is_multiple_of(channels) {
        return Err(CoreError::InvalidArgument(
            "resident read buffer must hold whole frames".into(),
        ));
    }
    match session.read(out, Some(read_timeout)) {
        Ok(frames) => Ok(frames),
        Err(MediaHostError::FormatChanged) => {
            let to = audio_spec_from_wire(session.info().spec);
            let from = *spec;
            *spec = to;
            Err(CoreError::FormatChanged { from, to })
        }
        Err(error) => Err(host_error_to_core(error, "read")),
    }
}

/// Translate one session seek into core [`Decoder::seek`] semantics.
///
/// The wire quantizes to milliseconds; the returned position is what the
/// resident reports as reached. The cached spec is refreshed because a seek
/// may land on a new format.
fn convert_seek<F: PcmRings>(
    session: &mut DecoderSession<F>,
    to: Duration,
    seek_timeout: Duration,
    spec: &mut AudioSpec,
) -> CoreResult<Duration> {
    match session.seek(ms_from_duration(to), seek_timeout) {
        Ok(reached_ms) => {
            *spec = audio_spec_from_wire(session.info().spec);
            Ok(Duration::from_millis(reached_ms))
        }
        Err(error) => Err(host_error_to_core(error, "seek")),
    }
}

/// Technical info for the main window from the cached spec and the last
/// telemetry the session reported.
fn stream_info_for(spec: AudioSpec, bitrate_kbps: Option<u32>) -> StreamInfo {
    StreamInfo {
        bitrate_kbps,
        sample_rate_hz: spec.sample_rate,
        channels: spec.channels,
    }
}

/// A media-host session as the core [`Decoder`] the engine drives.
///
/// The generic ring mapping keeps this testable without spawning a process:
/// production uses [`NamedPcmRings`][rustamp_mediahost::pcm::NamedPcmRings] on
/// Windows (see [`ProcessDecoder`]), while tests drive the same adapter over
/// in-memory rings. Format handshake values are replayed from the session's
/// `DecoderOpened` data; damage/bitrate telemetry from its last
/// `DecoderStats`.
pub struct HostDecoder<F: PcmRings> {
    session: DecoderSession<F>,
    read_timeout: Duration,
    seek_timeout: Duration,
    spec: AudioSpec,
}

impl<F: PcmRings> HostDecoder<F> {
    /// Wrap an open session. The session already completed the format
    /// handshake, so the spec is taken from it.
    pub fn new(session: DecoderSession<F>, read_timeout: Duration, seek_timeout: Duration) -> Self {
        let spec = audio_spec_from_wire(session.info().spec);
        HostDecoder {
            session,
            read_timeout,
            seek_timeout,
            spec,
        }
    }

    /// The wrapped session (for diagnostics and tests).
    pub fn session(&self) -> &DecoderSession<F> {
        &self.session
    }

    /// The wrapped session, mutably.
    pub fn session_mut(&mut self) -> &mut DecoderSession<F> {
        &mut self.session
    }
}

impl<F: PcmRings> Decoder for HostDecoder<F> {
    fn spec(&self) -> AudioSpec {
        self.spec
    }

    fn read(&mut self, out: &mut [f32]) -> CoreResult<usize> {
        convert_read(&mut self.session, out, self.read_timeout, &mut self.spec)
    }

    fn seek(&mut self, to: Duration) -> CoreResult<Duration> {
        convert_seek(&mut self.session, to, self.seek_timeout, &mut self.spec)
    }

    fn duration(&self) -> Option<Duration> {
        self.session.info().duration_ms.map(Duration::from_millis)
    }

    fn metadata(&self) -> rustamp_core::meta::TrackMetadata {
        self.session.info().metadata.clone()
    }

    fn stream_info(&self) -> StreamInfo {
        stream_info_for(self.spec, self.session.stats().1)
    }

    fn is_seekable(&self) -> bool {
        self.session.info().seekable
    }

    fn skipped_packets(&self) -> u64 {
        self.session.stats().0
    }
}

/// A spawned helper process as the core [`Decoder`] the engine drives.
///
/// This is the production counterpart of [`HostDecoder`]: it owns the child
/// (killed and reaped on drop by the framework) and delegates every call to
/// its session with the same translations. Only available on Windows, where
/// the named PCM mappings exist.
///
/// The session is behind a mutex because the core [`Decoder`] trait splits
/// `&self` (spec, duration, metadata, telemetry) and `&mut self` (read, seek)
/// while every session accessor needs exclusivity. The decode worker is the
/// only thread that ever touches this decoder, so the mutex is never
/// contended; it is never used on the audio thread.
#[cfg(windows)]
pub struct ProcessDecoder {
    process: std::sync::Mutex<rustamp_mediahost::decoder::DecoderProcess>,
    read_timeout: Duration,
    seek_timeout: Duration,
    spec: AudioSpec,
    pid: u32,
}

#[cfg(windows)]
impl ProcessDecoder {
    /// Wrap a spawned helper. Reads the handshake spec from its session.
    pub fn new(
        mut process: rustamp_mediahost::decoder::DecoderProcess,
        read_timeout: Duration,
        seek_timeout: Duration,
    ) -> Self {
        let spec = audio_spec_from_wire(process.session().info().spec);
        let pid = process.pid();
        ProcessDecoder {
            process: std::sync::Mutex::new(process),
            read_timeout,
            seek_timeout,
            spec,
            pid,
        }
    }

    /// Lock the session. The decode worker is the only accessor, so this never
    /// blocks in practice; a poisoned mutex means the worker already panicked.
    fn lock(
        &self,
    ) -> CoreResult<std::sync::MutexGuard<'_, rustamp_mediahost::decoder::DecoderProcess>> {
        self.process
            .lock()
            .map_err(|_| CoreError::Decode("resident session lock poisoned".into()))
    }

    /// OS child id of the helper (for diagnostics and VM crash-isolation tests).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Terminate and reap a stalled helper; the next read fails loudly.
    pub fn terminate(&self) -> HostResult<()> {
        self.process
            .lock()
            .map_err(|_| MediaHostError::Disconnected("resident session lock poisoned".into()))?
            .terminate()
    }
}

#[cfg(windows)]
impl Decoder for ProcessDecoder {
    fn spec(&self) -> AudioSpec {
        self.spec
    }

    fn read(&mut self, out: &mut [f32]) -> CoreResult<usize> {
        let mut process = self
            .process
            .lock()
            .map_err(|_| CoreError::Decode("resident session lock poisoned".into()))?;
        convert_read(process.session(), out, self.read_timeout, &mut self.spec)
    }

    fn seek(&mut self, to: Duration) -> CoreResult<Duration> {
        let mut process = self
            .process
            .lock()
            .map_err(|_| CoreError::Decode("resident session lock poisoned".into()))?;
        convert_seek(process.session(), to, self.seek_timeout, &mut self.spec)
    }

    fn duration(&self) -> Option<Duration> {
        self.lock()
            .ok()?
            .session()
            .info()
            .duration_ms
            .map(Duration::from_millis)
    }

    fn metadata(&self) -> rustamp_core::meta::TrackMetadata {
        self.lock()
            .map(|mut process| process.session().info().metadata.clone())
            .unwrap_or_default()
    }

    fn stream_info(&self) -> StreamInfo {
        let spec = self.spec;
        self.lock()
            .map(|mut process| {
                let session = process.session();
                stream_info_for(spec, session.stats().1)
            })
            .unwrap_or_else(|_| stream_info_for(spec, None))
    }

    fn is_seekable(&self) -> bool {
        self.lock()
            .map(|mut process| process.session().info().seekable)
            .unwrap_or(false)
    }

    fn skipped_packets(&self) -> u64 {
        self.lock()
            .map(|mut process| process.session().stats().0)
            .unwrap_or(0)
    }
}

/// Opt-in [`DecoderFactory`] that opens every track through the helper.
///
/// Registering this factory is what selects the resident: a registry without
/// it decodes exactly as before. `probe` reports `EXTENSION` on the formats
/// the resident serves and `NONE` otherwise, so ties with the in-process
/// factories break by registration order (RFC 0013) — first registration
/// wins. `Reader` sources are refused without spawning (they cannot cross the
/// boundary, RFC 0006 §5.1); the engine queue only holds paths and URLs, so
/// this never triggers there. Non-UTF-8 local paths fail loudly instead of
/// being mangled, and a missing helper binary degrades to "format
/// unsupported" while a refusal or death is a loud decode error.
#[derive(Debug, Clone)]
pub struct ResidentFactory {
    config: ResidentConfig,
}

impl ResidentFactory {
    /// Serve the resident behind `config` for every claimed track.
    pub fn new(config: ResidentConfig) -> Self {
        ResidentFactory { config }
    }

    /// The helper and bounds this factory opens with.
    pub fn config(&self) -> &ResidentConfig {
        &self.config
    }
}

impl DecoderFactory for ResidentFactory {
    fn name(&self) -> &str {
        "resident"
    }

    fn extensions(&self) -> &[&str] {
        RESIDENT_EXTENSIONS
    }

    fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
        match hint.extension {
            Some(ext) if RESIDENT_EXTENSIONS.contains(&ext) => Confidence::EXTENSION,
            _ => Confidence::NONE,
        }
    }

    fn open(&self, source: MediaSource) -> CoreResult<Box<dyn Decoder>> {
        #[cfg(not(windows))]
        {
            let _ = source;
            return Err(CoreError::Unsupported(
                "resident decoding requires Windows named PCM mappings",
            ));
        }
        #[cfg(windows)]
        {
            use rustamp_mediahost::control::Liveness;
            use rustamp_mediahost::decoder::DecoderProcess;

            if self.config.prefetch_frames == 0 {
                return Err(CoreError::InvalidArgument(
                    "resident prefetch_frames must be above zero".into(),
                ));
            }
            let location = match source {
                MediaSource::Path(path) => {
                    file_location(&path).map_err(|error| CoreError::Decode(error.to_string()))?
                }
                MediaSource::Url(url) => MediaLocationWire::Url(url),
                MediaSource::Reader { .. } => {
                    return Err(CoreError::Unsupported(
                        "reader sources stay in-process; the resident only opens paths and URLs",
                    ));
                }
            };
            let process = DecoderProcess::spawn(
                &self.config.helper,
                location,
                self.config.prefetch_frames,
                Liveness::default(),
            )
            .map_err(|error| match &error {
                MediaHostError::OperationFailed(message)
                    if message.starts_with("decoder helper unavailable") =>
                {
                    CoreError::UnsupportedFormat(format!(
                        "resident helper {}: {message}",
                        self.config.helper.display()
                    ))
                }
                other => CoreError::Decode(other.to_string()),
            })?;
            Ok(Box::new(ProcessDecoder::new(
                process,
                self.config.read_timeout,
                self.config.seek_timeout,
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// DSP resident adapter (RFC 0006 §6)
// ---------------------------------------------------------------------------

/// In-ring capacity in frames for a resident DSP chain when the caller does
/// not pick one.
///
/// Own choice: the block round-trip is synchronous (one block out, the same
/// block back), so the capacity is a liveness/coupling choice, not a latency
/// one — room for a few blocks while the resident pumps, far under the
/// mapping limit.
pub const DEFAULT_DSP_RING_FRAMES: u32 = 4096;

/// Bounded wait for one whole block round-trip (write + read-back) through
/// the resident.
///
/// Own choice: long enough for a loaded helper to answer, short enough that
/// a wedged resident fails the track instead of hanging the decode thread.
/// The engine's join of the decode thread is bounded by this too.
pub const DEFAULT_DSP_BLOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// A latched resident-DSP failure, polled by the engine.
///
/// The core [`DspStage::process`] contract is infallible (M14 finding F1;
/// RFC 0014 is still a draft), so a resident chain that dies or wedges
/// mid-stream cannot return an error from `process`. The adapter latches the
/// first failure here and passes audio through bit-exactly from then on;
/// [`Engine::poll`](crate::engine::Engine::poll) takes it and fails the track
/// (RFC 0006 §8: the resident's death fails the track, never the player).
/// The lock is taken on the decode worker and the engine's caller thread
/// only — never on the render callback.
#[derive(Debug, Default)]
pub struct DspFailureWatch {
    failure: std::sync::Mutex<Option<ErrorInfo>>,
}

impl DspFailureWatch {
    /// A fresh, empty watch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Latch a failure; the first one sticks, later ones are dropped.
    pub fn latch(&self, info: ErrorInfo) {
        let mut failure = self
            .failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failure.is_none() {
            *failure = Some(info);
        }
    }

    /// Take the latched failure, if any. The engine handles it exactly once.
    pub fn take(&self) -> Option<ErrorInfo> {
        self.failure
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

/// Wire spec for a core spec. The wire carries `u8` channels, so a wider spec
/// is refused loudly instead of truncated.
fn wire_spec(spec: AudioSpec) -> CoreResult<PcmSpecWire> {
    let channels = u8::try_from(spec.channels).map_err(|_| {
        CoreError::InvalidArgument(format!(
            "resident DSP cannot carry {} channels",
            spec.channels
        ))
    })?;
    Ok(PcmSpecWire {
        sample_rate_hz: spec.sample_rate,
        channels,
    })
}

/// A media-host DSP failure as a loud core error.
///
/// Never silence and never end-of-stream: every variant becomes a `Dsp`
/// error, so the engine reports it and fails the track, never the player
/// (RFC 0006 §8). Timeouts and disconnects (a wedged or dead resident) are
/// `Dsp` errors for the same reason.
fn dsp_error_to_core(error: MediaHostError, context: &str) -> CoreError {
    match error {
        MediaHostError::TimedOut => CoreError::Dsp(format!("resident DSP {context} timed out")),
        MediaHostError::Disconnected(message) => {
            CoreError::Dsp(format!("resident DSP {context} disconnected: {message}"))
        }
        MediaHostError::OperationFailed(message) => {
            CoreError::Dsp(format!("resident DSP {context} failed: {message}"))
        }
        MediaHostError::InvalidMessage(message) => CoreError::Dsp(format!(
            "resident DSP {context} protocol violation: {message}"
        )),
        MediaHostError::NotImplemented(message) => {
            CoreError::Dsp(format!("resident DSP {context} not implemented: {message}"))
        }
        MediaHostError::Ring(error) => {
            CoreError::Dsp(format!("resident DSP {context} ring failure: {error}"))
        }
        MediaHostError::FormatChanged => {
            CoreError::Dsp(format!("resident DSP {context} unexpected format signal"))
        }
    }
}

/// A helper-spawn failure as a core error: a missing binary degrades to
/// "stage unavailable" (RFC 0006 §8), anything else is a loud `Dsp` error.
///
/// Own choice: the core `Unsupported` variant carries a `&'static str`, so
/// the message cannot name the configured helper path; the caller constructed
/// the config and knows it. The degradation phrase leads the message either
/// way, exactly like the decoder adapter's `UnsupportedFormat` mapping.
fn spawn_error_to_core(error: MediaHostError) -> CoreError {
    match error {
        MediaHostError::OperationFailed(message)
            if message.starts_with("DSP helper unavailable") =>
        {
            CoreError::Unsupported("stage unavailable: resident DSP helper missing")
        }
        other => dsp_error_to_core(other, "configure"),
    }
}

/// One bounded block round-trip through the resident: write the whole block,
/// read the same number of frames back. The resident processes strictly in
/// order, so the block comes back delayed by the chain's latency — exactly
/// the `DspStage` contract the decode thread compensates (`dsp_trim` after
/// every prepare/reset, `flush_dsp` at the end of every track). `Ok(0)` from
/// a read is the session's sticky EOS, which the adapter never asks for
/// mid-stream: it is a loud failure, never silence.
fn process_block<F: PcmRings>(
    session: &mut DspSession<F>,
    buf: &mut [f32],
    block_timeout: Duration,
) -> CoreResult<()> {
    session
        .write_timeout(buf, Some(block_timeout))
        .map_err(|error| dsp_error_to_core(error, "write"))?;
    let channels = usize::from(session.spec().channels.max(1));
    let mut filled = 0;
    while filled < buf.len() {
        let frames = session
            .read(&mut buf[filled..], Some(block_timeout))
            .map_err(|error| dsp_error_to_core(error, "read"))?;
        if frames == 0 {
            return Err(CoreError::Dsp(
                "resident DSP closed its output mid-stream".into(),
            ));
        }
        filled += frames * channels;
    }
    Ok(())
}

/// Shared control state of a resident DSP chain: the stage list a reconfigure
/// sends, the ring capacity, the bounded round-trip, the engine-polled
/// failure watch and the sticky poisoned state.
struct ChainControl {
    stages: Vec<DspStageDesc>,
    ring_frames: u32,
    block_timeout: Duration,
    watch: Arc<DspFailureWatch>,
    /// The first failure sticks; `Some` means the chain is poisoned and
    /// bypasses bit-exactly (RFC 0014's terminal rule, held adapter-side
    /// while the core contract is infallible).
    poisoned: Option<CoreError>,
}

impl ChainControl {
    fn new(
        stages: Vec<DspStageDesc>,
        ring_frames: u32,
        block_timeout: Duration,
        watch: Arc<DspFailureWatch>,
    ) -> Self {
        ChainControl {
            stages,
            ring_frames,
            block_timeout,
            watch,
            poisoned: None,
        }
    }

    /// Latch a failure: the first one sticks and is published to the watch.
    fn latch(&mut self, error: CoreError) {
        if self.poisoned.is_none() {
            self.watch.latch(ErrorInfo::from(&error));
            self.poisoned = Some(error);
        }
    }

    /// `DspStage::prepare`: reconfigure the resident on a format change,
    /// clear its stage state otherwise. The decode thread calls `prepare` at
    /// every track start, and the `DspStage` contract clears stage state at a
    /// track change (the engine's own stages leave `prepare` with fresh
    /// state), so a same-format prepare resets the resident's chain too.
    fn prepare<F: PcmRings>(
        &mut self,
        session: &mut DspSession<F>,
        spec: AudioSpec,
    ) -> CoreResult<()> {
        if self.poisoned.is_some() {
            return Err(CoreError::Dsp(
                "resident DSP chain already failed; replace the stage".into(),
            ));
        }
        if spec.sample_rate == 0 || spec.channels == 0 {
            return Err(CoreError::Unsupported("resident DSP needs a non-zero spec"));
        }
        let wire = wire_spec(spec)?;
        if wire == session.spec() {
            self.reset(session);
            if self.poisoned.is_some() {
                return Err(CoreError::Dsp(
                    "resident DSP chain already failed; replace the stage".into(),
                ));
            }
        } else if let Err(error) = session
            .reconfigure(wire, &self.stages, self.ring_frames)
            .map_err(|error| dsp_error_to_core(error, "configure"))
        {
            let err_msg = error.to_string();
            self.latch(error);
            return Err(CoreError::Dsp(err_msg));
        }
        Ok(())
    }

    /// `DspStage::process`: one bounded block round-trip. A failure latches
    /// (the buffer is left untouched from then on: bit-exact bypass) — the
    /// engine's next poll fails the track.
    fn process<F: PcmRings>(&mut self, session: &mut DspSession<F>, buf: &mut [f32]) {
        if self.poisoned.is_some() {
            return;
        }
        if let Err(error) = process_block(session, buf, self.block_timeout) {
            self.latch(error);
        }
    }

    /// `DspStage::reset` (seek): clear the resident's stage state; the rings
    /// are untouched, and the decode thread drops the chain's start-up frames
    /// (`dsp_trim`) right after, so the seek stays exact.
    ///
    /// No wall-clock wait here by design: the resident applies `DspReset`
    /// asynchronously — the session documents it as neither an acknowledgement
    /// nor an ordering fence for PCM already queued — so no finite sleep is
    /// derived from the contract and any fixed duration only narrows the race
    /// instead of closing it. The decode protocol is robust to the race by
    /// construction (a stale prefix lands inside the trimmed latency window);
    /// a true delivery fence needs a session-level acknowledgement owned by
    /// `rustamp-mediahost`, named as follow-up in `docs/tasks/A22-audio-rest.md`.
    fn reset<F: PcmRings>(&mut self, session: &mut DspSession<F>) {
        if self.poisoned.is_some() {
            return;
        }
        if let Err(error) = session
            .reset()
            .map_err(|error| dsp_error_to_core(error, "reset"))
        {
            self.latch(error);
        }
    }

    /// `DspStage::latency_frames`: the resident's `DspReady` report, which the
    /// decode thread sums into its trim/flush compensation exactly like an
    /// in-process stage's (RFC 0006 §6.2). Zero once poisoned: the bypass is
    /// undelayed, so trimming would drop real audio.
    fn latency<F: PcmRings>(&self, session: &DspSession<F>) -> u32 {
        if self.poisoned.is_some() {
            0
        } else {
            session.latency_frames()
        }
    }
}

/// A media-host DSP session as the core [`DspStage`] the engine drives.
///
/// The DSP counterpart of [`HostDecoder`]: the engine keeps driving the same
/// `Vec<Box<dyn DspStage>>` chain — the gapless loop, the seek drain, the
/// format-change flush and the latency trim/flush in [`crate::decode`] are
/// unchanged, because a resident chain is just another stage behind the same
/// trait object.
///
/// ```text
/// in-process: decoder -> EQ -> [DspStage]* -> Limiter -> converter -> ring
/// resident:   decoder -> EQ -> HostDspChain -> helper process (two rings)
///             -> Limiter -> converter -> ring
/// ```
///
/// The generic ring mapping keeps this testable without spawning a process:
/// production uses [`NamedPcmRings`][rustamp_mediahost::pcm::NamedPcmRings] on
/// Windows (see [`ProcessDspChain`]), while tests drive the same adapter over
/// the real rings in one process.
///
/// The stage runs on the decode worker only — never on the render callback
/// (no blocking, no locks there).
pub struct HostDspChain<F: PcmRings> {
    session: DspSession<F>,
    control: ChainControl,
}

impl<F: PcmRings> HostDspChain<F> {
    /// Wrap an open session. The session already completed the configure
    /// handshake; `stages`/`ring_frames` are what a later reconfigure sends,
    /// `block_timeout` bounds every block round-trip, and `watch` is where a
    /// mid-run failure latches (the engine's
    /// [`Engine::resident_dsp_watch`](crate::engine::Engine::resident_dsp_watch)).
    pub fn new(
        session: DspSession<F>,
        stages: Vec<DspStageDesc>,
        ring_frames: u32,
        block_timeout: Duration,
        watch: Arc<DspFailureWatch>,
    ) -> Self {
        HostDspChain {
            session,
            control: ChainControl::new(stages, ring_frames, block_timeout, watch),
        }
    }

    /// The wrapped session (for diagnostics and tests).
    pub fn session(&self) -> &DspSession<F> {
        &self.session
    }

    /// The wrapped session, mutably.
    pub fn session_mut(&mut self) -> &mut DspSession<F> {
        &mut self.session
    }
}

impl<F: PcmRings> DspStage for HostDspChain<F> {
    fn name(&self) -> &str {
        "resident-dsp"
    }

    fn prepare(&mut self, spec: AudioSpec) -> CoreResult<()> {
        self.control.prepare(&mut self.session, spec)
    }

    fn process(&mut self, buf: &mut [f32]) {
        self.control.process(&mut self.session, buf)
    }

    fn reset(&mut self) {
        self.control.reset(&mut self.session)
    }

    fn latency_frames(&self) -> u32 {
        self.control.latency(&self.session)
    }
}

/// A spawned helper process as the core [`DspStage`] the engine drives.
///
/// The production counterpart of [`HostDspChain`]: it owns the child (killed
/// and reaped on drop by the framework) and delegates every call to its
/// session with the same translations. Only available on Windows, where the
/// named PCM mappings exist.
///
/// The child is behind a mutex because the core [`DspStage`] trait splits
/// `&self`/`&mut self` while the session needs exclusivity. The decode worker
/// is the only thread that ever touches this stage, so the mutex is never
/// contended; it is never used on the audio thread.
#[cfg(windows)]
pub struct ProcessDspChain {
    process: std::sync::Mutex<rustamp_mediahost::dsp::DspProcess>,
    control: ChainControl,
}

#[cfg(windows)]
impl ProcessDspChain {
    /// Wrap a spawned helper with the control state a [`HostDspChain`] would
    /// have built.
    pub fn new(
        process: rustamp_mediahost::dsp::DspProcess,
        stages: Vec<DspStageDesc>,
        ring_frames: u32,
        block_timeout: Duration,
        watch: Arc<DspFailureWatch>,
    ) -> Self {
        ProcessDspChain {
            process: std::sync::Mutex::new(process),
            control: ChainControl::new(stages, ring_frames, block_timeout, watch),
        }
    }

    /// Lock the child. The decode worker is the only accessor, so this never
    /// blocks in practice; a poisoned mutex means the worker already panicked.
    fn lock(&self) -> CoreResult<std::sync::MutexGuard<'_, rustamp_mediahost::dsp::DspProcess>> {
        self.process
            .lock()
            .map_err(|_| CoreError::Dsp("resident DSP session lock poisoned".into()))
    }

    /// OS child id of the helper (for diagnostics and VM crash-isolation
    /// tests).
    pub fn pid(&self) -> u32 {
        self.process
            .lock()
            .map(|process| process.pid())
            .unwrap_or(0)
    }

    /// Terminate and reap a stalled helper; the next `process` fails loudly.
    pub fn terminate(&self) -> HostResult<()> {
        self.lock()
            .map_err(|_| MediaHostError::Disconnected("resident DSP session lock poisoned".into()))?
            .terminate()
    }
}

#[cfg(windows)]
impl DspStage for ProcessDspChain {
    fn name(&self) -> &str {
        "resident-dsp"
    }

    fn prepare(&mut self, spec: AudioSpec) -> CoreResult<()> {
        let Self { process, control } = self;
        let Ok(process) = process.get_mut() else {
            return Err(CoreError::Dsp("resident DSP session lock poisoned".into()));
        };
        control.prepare(process.session(), spec)
    }

    fn process(&mut self, buf: &mut [f32]) {
        let Self { process, control } = self;
        let Ok(process) = process.get_mut() else {
            control.latch(CoreError::Dsp("resident DSP session lock poisoned".into()));
            return;
        };
        control.process(process.session(), buf);
    }

    fn reset(&mut self) {
        let Self { process, control } = self;
        let Ok(process) = process.get_mut() else {
            control.latch(CoreError::Dsp("resident DSP session lock poisoned".into()));
            return;
        };
        control.reset(process.session());
    }

    fn latency_frames(&self) -> u32 {
        let Self { process, control } = self;
        match process.lock() {
            Ok(mut guard) => control.latency(guard.session()),
            Err(_) => 0,
        }
    }
}

/// How to reach and bound one DSP-resident helper process.
///
/// The helper binary is named explicitly: there is no `PATH` lookup and no
/// default, so a missing binary is a loud, diagnosable failure rather than
/// an accidental silent fallback. Constructing and opening this config is the
/// opt-in: the default engine chain is empty, and nothing here runs unless
/// the caller opens a chain and installs it with
/// [`Engine::set_dsp_chain`](crate::engine::Engine::set_dsp_chain).
#[derive(Debug, Clone)]
pub struct DspResidentConfig {
    /// Absolute path of the resident helper binary (for example
    /// `rustamp-dsphost-stages` next to `rustamp.exe`).
    pub helper: PathBuf,
    /// The ordered stage list every configure sends (`DspConfigure`).
    pub stages: Vec<DspStageDesc>,
    /// Capacity of the player-created in-ring, in frames.
    pub ring_frames: u32,
    /// Bound for one whole block round-trip; a wedged resident fails the
    /// track after this instead of hanging the decode thread.
    pub block_timeout: Duration,
}

impl DspResidentConfig {
    /// Opt in to resident DSP processing through the helper at `helper`,
    /// running the ordered `stages`.
    pub fn new(helper: PathBuf, stages: Vec<DspStageDesc>) -> Self {
        DspResidentConfig {
            helper,
            stages,
            ring_frames: DEFAULT_DSP_RING_FRAMES,
            block_timeout: DEFAULT_DSP_BLOCK_TIMEOUT,
        }
    }

    /// Override the in-ring capacity (zero is rejected loudly at open).
    pub fn with_ring_frames(mut self, ring_frames: u32) -> Self {
        self.ring_frames = ring_frames;
        self
    }

    /// Override the block round-trip bound.
    pub fn with_block_timeout(mut self, block_timeout: Duration) -> Self {
        self.block_timeout = block_timeout;
        self
    }

    /// Open the chain in a helper process, configured for `spec`, and return
    /// it as the [`DspStage`] to install.
    ///
    /// `spec` is the format the chain opens at — the device spec is the
    /// natural choice; the first `prepare` with a different format
    /// reconfigures the resident. `watch` is the engine's
    /// [`Engine::resident_dsp_watch`](crate::engine::Engine::resident_dsp_watch):
    /// a mid-run resident failure latches there and the engine fails the
    /// track.
    ///
    /// A refusal at configure (unknown stage id, undecodable params) is a
    /// loud `Dsp` error here, before anything is installed; a missing helper
    /// binary degrades to "stage unavailable" (`Unsupported`), per RFC 0006
    /// §8 — never a silent in-process fallback.
    pub fn open(
        &self,
        spec: AudioSpec,
        watch: Arc<DspFailureWatch>,
    ) -> CoreResult<Box<dyn DspStage>> {
        #[cfg(not(windows))]
        {
            let _ = (spec, watch);
            return Err(CoreError::Unsupported(
                "resident DSP requires Windows named PCM mappings",
            ));
        }
        #[cfg(windows)]
        {
            use rustamp_mediahost::control::Liveness;
            use rustamp_mediahost::dsp::DspProcess;

            if self.ring_frames == 0 {
                return Err(CoreError::InvalidArgument(
                    "resident DSP ring_frames must be above zero".into(),
                ));
            }
            let wire = wire_spec(spec)?;
            let process = DspProcess::spawn(
                &self.helper,
                wire,
                self.stages.clone(),
                self.ring_frames,
                Liveness::default(),
            )
            .map_err(spawn_error_to_core)?;
            Ok(Box::new(ProcessDspChain::new(
                process,
                self.stages.clone(),
                self.ring_frames,
                self.block_timeout,
                watch,
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_spec_converts_to_core_spec() {
        let wire = PcmSpecWire {
            sample_rate_hz: 44_100,
            channels: 2,
        };
        assert_eq!(audio_spec_from_wire(wire), AudioSpec::new(44_100, 2));
    }

    #[test]
    fn durations_quantize_to_whole_milliseconds() {
        assert_eq!(ms_from_duration(Duration::from_millis(1000)), 1000);
        assert_eq!(
            ms_from_duration(Duration::from_micros(1_999)),
            1,
            "sub-millisecond precision is lost on the wire"
        );
    }

    #[test]
    fn timeouts_and_disconnects_are_loud_decode_errors() {
        let error = host_error_to_core(MediaHostError::TimedOut, "read");
        assert!(
            matches!(&error, CoreError::Decode(message) if message.contains("timed out")),
            "{error:?}"
        );
        let error = host_error_to_core(
            MediaHostError::Disconnected("control stream ended".into()),
            "read",
        );
        assert!(
            matches!(&error, CoreError::Decode(message) if message.contains("disconnected")),
            "{error:?}"
        );
    }

    #[test]
    fn refusals_are_loud_and_never_silent() {
        let error = host_error_to_core(
            MediaHostError::OperationFailed("garbage is not audio".into()),
            "open",
        );
        assert!(
            matches!(&error, CoreError::Decode(message) if message.contains("garbage")),
            "{error:?}"
        );
    }

    #[test]
    fn factory_claims_only_resident_extensions() {
        let factory = ResidentFactory::new(ResidentConfig::new(PathBuf::from("helper.exe")));
        assert_eq!(factory.name(), "resident");
        assert!(factory.extensions().contains(&"mp3"));
        let hint = ProbeHint {
            extension: Some("mp3"),
            header: &[],
        };
        assert_eq!(factory.probe(&hint), Confidence::EXTENSION);
        let hint = ProbeHint {
            extension: Some("sid"),
            header: &[],
        };
        assert_eq!(factory.probe(&hint), Confidence::NONE);
    }

    #[test]
    fn reader_sources_are_refused_without_spawning() {
        use std::io::Cursor;
        let factory = ResidentFactory::new(ResidentConfig::new(PathBuf::from("helper.exe")));
        let reader = Box::new(Cursor::new(vec![0u8; 8]));
        let outcome = factory.open(MediaSource::Reader {
            reader,
            extension: Some("mp3".into()),
        });
        assert!(
            matches!(outcome, Err(CoreError::Unsupported(_))),
            "a reader must never spawn a helper"
        );
    }

    #[test]
    fn zero_prefetch_is_rejected_loudly() {
        let factory = ResidentFactory::new(
            ResidentConfig::new(PathBuf::from("helper.exe")).with_prefetch_frames(0),
        );
        // On non-Windows this is Unsupported (no spawn platform); on Windows it
        // is InvalidArgument before any process exists. Either way no helper
        // is spawned for a zero capacity.
        match factory.open(MediaSource::Path(PathBuf::from("track.mp3"))) {
            Err(CoreError::InvalidArgument(_)) => {}
            #[cfg(not(windows))]
            Err(CoreError::Unsupported(_)) => {}
            Err(error) => panic!("zero prefetch must not open, got {error}"),
            Ok(_) => panic!("zero prefetch must not open"),
        }
    }
}

#[cfg(all(test, windows))]
mod hosted_tests {
    //! Adapter behaviour over a real in-process session with a fake resident.
    //! Headless, one Windows process, no helper binary spawned: byte-stream
    //! control pairs exercise the production framing and the real ring/wake
    //! API, exactly like the framework's own suite.

    use std::io::{Cursor, Read, Write};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use rustamp_mediahost::control::{ControlPeer, Liveness};
    use rustamp_mediahost::decoder::{
        DecoderInfo, DecoderResident, DecoderSession, ResidentDecoder, serve_decoder,
    };
    use rustamp_mediahost::pcm::NamedPcmRings;
    use rustamp_mediahost::{MediaHostError, Result as HostResult};
    use rustamp_proto::host::MediaLocationWire;

    use super::*;

    const TOTAL: usize = 2048;
    const CHANGE_AT: usize = 512;
    const WAIT: Duration = Duration::from_secs(5);

    struct FakeResident;

    struct FakeDecoder {
        mode: String,
        frame: usize,
        changed: bool,
    }

    impl DecoderResident for FakeResident {
        fn open(&self, location: MediaLocationWire) -> HostResult<Box<dyn ResidentDecoder>> {
            let MediaLocationWire::Url(mode) = location else {
                return Err(MediaHostError::OperationFailed(
                    "fake resident takes modes as URLs".into(),
                ));
            };
            if mode == "fake:refuse" {
                return Err(MediaHostError::OperationFailed(
                    "fake resident refuses this file".into(),
                ));
            }
            Ok(Box::new(FakeDecoder {
                mode,
                frame: 0,
                changed: false,
            }))
        }
    }

    impl ResidentDecoder for FakeDecoder {
        fn info(&self) -> DecoderInfo {
            DecoderInfo {
                spec: PcmSpecWire {
                    sample_rate_hz: if self.changed { 2000 } else { 1000 },
                    channels: if self.changed { 1 } else { 2 },
                },
                duration_ms: Some(TOTAL as u64),
                seekable: true,
                metadata: Default::default(),
            }
        }

        fn read(&mut self, out: &mut [f32]) -> HostResult<usize> {
            if self.mode == "fake:slow" && self.frame == 0 {
                thread::sleep(Duration::from_millis(150));
            }
            if self.mode == "fake:fail" && self.frame > 0 {
                return Err(MediaHostError::OperationFailed(
                    "fake decode failure".into(),
                ));
            }
            if self.mode == "fake:format" && !self.changed && self.frame == CHANGE_AT {
                self.changed = true;
                return Err(MediaHostError::FormatChanged);
            }
            let channels = usize::from(self.info().spec.channels);
            let end = if self.mode == "fake:format" && !self.changed {
                CHANGE_AT
            } else {
                TOTAL
            };
            let frames = (end - self.frame).min(out.len() / channels).min(257);
            for (index, frame) in out[..frames * channels]
                .chunks_exact_mut(channels)
                .enumerate()
            {
                frame[0] = (self.frame + index) as f32 / 8.0;
                if channels == 2 {
                    frame[1] = -frame[0];
                }
            }
            self.frame += frames;
            Ok(frames)
        }

        fn seek(&mut self, to_ms: u64) -> HostResult<u64> {
            self.frame = (to_ms as usize).min(TOTAL);
            self.changed = self.mode == "fake:format" && self.frame >= CHANGE_AT;
            Ok(self.frame as u64)
        }

        fn stats(&self) -> (u64, Option<u32>) {
            (7, Some(128))
        }
    }

    struct ByteReader {
        rx: mpsc::Receiver<Vec<u8>>,
        bytes: Cursor<Vec<u8>>,
    }

    struct ByteWriter(mpsc::Sender<Vec<u8>>);

    impl Read for ByteReader {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if out.is_empty() {
                return Ok(0);
            }
            loop {
                let count = self.bytes.read(out)?;
                if count > 0 {
                    return Ok(count);
                }
                match self.rx.recv() {
                    Ok(bytes) => self.bytes = Cursor::new(bytes),
                    Err(_) => return Ok(0),
                }
            }
        }
    }

    impl Write for ByteWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .send(bytes.to_vec())
                .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn streams() -> ((ByteReader, ByteWriter), (ByteReader, ByteWriter)) {
        let (a_tx, b_rx) = mpsc::channel();
        let (b_tx, a_rx) = mpsc::channel();
        (
            (
                ByteReader {
                    rx: a_rx,
                    bytes: Cursor::new(Vec::new()),
                },
                ByteWriter(a_tx),
            ),
            (
                ByteReader {
                    rx: b_rx,
                    bytes: Cursor::new(Vec::new()),
                },
                ByteWriter(b_tx),
            ),
        )
    }

    fn start(
        mode: &str,
        capacity: u32,
        read_timeout: Duration,
        seek_timeout: Duration,
    ) -> (HostDecoder<NamedPcmRings>, mpsc::Receiver<HostResult<()>>) {
        let ((ar, aw), (br, bw)) = streams();
        let client = ControlPeer::start(ar, aw, Liveness::default(), false).unwrap();
        let host = ControlPeer::start(br, bw, Liveness::default(), true).unwrap();
        let (done, result) = mpsc::channel();
        thread::spawn(move || {
            let outcome = serve_decoder(&host, &NamedPcmRings, &FakeResident);
            let _ = done.send(outcome);
        });
        let session = DecoderSession::open(
            client,
            NamedPcmRings,
            MediaLocationWire::Url(mode.into()),
            capacity,
        )
        .unwrap();
        (
            HostDecoder::new(session, read_timeout, seek_timeout),
            result,
        )
    }

    fn finish(decoder: HostDecoder<NamedPcmRings>, done: mpsc::Receiver<HostResult<()>>) {
        drop(decoder);
        done.recv_timeout(WAIT)
            .expect("host worker must finish within watchdog")
            .unwrap();
    }

    #[test]
    fn handshake_replays_and_transfer_is_bit_exact_with_sticky_eos() {
        let (mut decoder, done) = start("fake:plain", 3, WAIT, WAIT);
        assert_eq!(decoder.spec(), AudioSpec::new(1000, 2));
        assert_eq!(
            decoder.duration(),
            Some(Duration::from_millis(TOTAL as u64))
        );
        assert!(decoder.is_seekable());

        let mut received = Vec::new();
        let mut out = [0.0; 22];
        loop {
            match decoder.read(&mut out).unwrap() {
                0 => break,
                frames => received.extend_from_slice(&out[..frames * 2]),
            }
        }
        let expected: Vec<f32> = (0..TOTAL)
            .flat_map(|index| [index as f32 / 8.0, -(index as f32 / 8.0)])
            .collect();
        assert_eq!(
            received.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
            expected.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
        );
        assert_eq!(decoder.skipped_packets(), 7);
        // EOS is sticky: it stays zero after the ring is drained.
        assert_eq!(decoder.read(&mut out).unwrap(), 0);
        assert_eq!(decoder.read(&mut out).unwrap(), 0);
        finish(decoder, done);
    }

    #[test]
    fn seek_lands_and_reads_the_target() {
        let (mut decoder, done) = start("fake:plain", 3, WAIT, WAIT);
        let reached = decoder.seek(Duration::from_millis(29)).unwrap();
        assert_eq!(reached, Duration::from_millis(29));
        let mut out = [0.0; 2];
        assert_eq!(decoder.read(&mut out).unwrap(), 1);
        assert_eq!(out, [29.0 / 8.0, -29.0 / 8.0]);
        finish(decoder, done);
    }

    #[test]
    fn format_change_maps_to_core_format_changed_after_draining_old_pcm() {
        let (mut decoder, done) = start("fake:format", 3, WAIT, WAIT);
        let mut frame = 0usize;
        let mut changes = 0;
        let mut out = [0.0; 14];
        loop {
            let channels = usize::from(decoder.spec().channels);
            match decoder.read(&mut out) {
                Ok(0) => break,
                Ok(frames) => {
                    for samples in out[..frames * channels].chunks_exact(channels) {
                        assert_eq!(samples[0], frame as f32 / 8.0);
                        frame += 1;
                    }
                }
                Err(CoreError::FormatChanged { from, to }) => {
                    assert_eq!(from, AudioSpec::new(1000, 2));
                    assert_eq!(to, AudioSpec::new(2000, 1));
                    assert_eq!(frame, CHANGE_AT);
                    assert_eq!(decoder.spec(), to);
                    changes += 1;
                }
                Err(error) => panic!("unexpected error: {error}"),
            }
        }
        assert_eq!(frame, TOTAL);
        assert_eq!(changes, 1);
        finish(decoder, done);
    }

    #[test]
    fn refusal_is_loud_and_never_eos() {
        let ((ar, aw), (br, bw)) = streams();
        let client = ControlPeer::start(ar, aw, Liveness::default(), false).unwrap();
        let host = ControlPeer::start(br, bw, Liveness::default(), true).unwrap();
        let (done, result) = mpsc::channel();
        thread::spawn(move || {
            let outcome = serve_decoder(&host, &NamedPcmRings, &FakeResident);
            let _ = done.send(outcome);
        });
        let outcome = DecoderSession::open(
            client,
            NamedPcmRings,
            MediaLocationWire::Url("fake:refuse".into()),
            3,
        );
        // The honest contract is a loud refusal that carries the resident's
        // reason - never an accepted session, never EOS. The error variant is
        // deliberately not pinned: the framework's control monitor consumes
        // the resident's terminal DecoderError and fails the peer sticky
        // (rustamp-mediahost/src/control.rs), so the player sees
        // Disconnected("resident decoder failed: operation failed: ..."),
        // and the OperationFailed arm in DecoderSession::open is unreachable
        // (the monitor never forwards DecoderError to the session). The
        // variant is the framework's internal mapping, not this adapter's;
        // the reason pin below is the stronger claim - a bare transport
        // failure (TimedOut, a reason-less Disconnected) can never pass it.
        let described = match &outcome {
            Ok(_) => "Ok(session): the refused open was accepted".to_string(),
            Err(error) => format!("Err({error:?})"),
        };
        let Err(error) = outcome else {
            panic!("refusal must be loud, never a session (got {described})");
        };
        assert!(
            error.to_string().contains("refuses"),
            "the loud refusal must carry the resident's refusal reason (got {described})"
        );
        // The host's terminal sequence is one send plus the framework's
        // bounded flush (5 s CONTROL_WAIT) and return; the test's bound must
        // cover a full flush plus loaded-VM scheduling headroom. The claim
        // is unchanged: the host must end with an error instead of stalling.
        result
            .recv_timeout(WAIT * 3)
            .expect("host must end after a refused open instead of stalling")
            .expect_err("a refused open must end the host with an error");
    }

    #[test]
    fn death_is_loud_and_never_silent_eos() {
        let (mut decoder, done) = start("fake:fail", 3, WAIT, WAIT);
        let mut out = [0.0; 14];
        loop {
            match decoder.read(&mut out) {
                Ok(_) => {}
                Err(CoreError::Decode(message)) => {
                    assert!(message.contains("fake decode failure"), "{message}");
                    break;
                }
                Err(error) => panic!("a dead resident must be Decode, got {error}"),
            }
        }
        drop(decoder);
        assert!(done.recv_timeout(WAIT).unwrap().is_err());
    }

    #[test]
    fn bounded_wait_never_hangs_and_retry_delivers_pcm() {
        let (mut decoder, done) = start(
            "fake:slow",
            1,
            Duration::from_millis(20),
            Duration::from_secs(5),
        );
        let mut out = [0.0; 2];
        let start = Instant::now();
        assert!(
            matches!(decoder.read(&mut out), Err(CoreError::Decode(_))),
            "a wedged resident must fail loudly after the bound"
        );
        assert!(start.elapsed() < WAIT);
        // fake:slow sleeps for 150 ms; allow it to finish and deliver before retry.
        thread::sleep(Duration::from_millis(150));
        assert_eq!(
            decoder.read(&mut out).unwrap(),
            1,
            "retry after the bound must deliver PCM"
        );
        drop(decoder);
        let _ = done.recv_timeout(WAIT);
    }

    #[test]
    fn control_loss_maps_to_loud_decode_error() {
        // The session turns a dead host into Disconnected (pinned by the
        // framework suite); the adapter must surface it as a loud Decode,
        // never as silence or EOS. Exercise the mapping directly so this
        // unit test needs no packet framing.
        let error = host_error_to_core(
            MediaHostError::Disconnected("control stream ended".into()),
            "read",
        );
        assert!(
            matches!(&error, CoreError::Decode(message) if message.contains("disconnected")),
            "{error:?}"
        );
    }
}

#[cfg(test)]
mod dsp_tests {
    //! Pure mapping/config tests for the DSP resident adapter: spec
    //! conversion, loud error mapping, the failure watch and the opt-in
    //! config. No rings, no session, no process.

    use rustamp_core::error::ErrorKind;

    use super::*;

    #[test]
    fn dsp_wire_spec_converts_and_rejects_wide_channels() {
        let wire = wire_spec(AudioSpec::new(44_100, 2)).unwrap();
        assert_eq!(
            wire,
            PcmSpecWire {
                sample_rate_hz: 44_100,
                channels: 2
            }
        );
        let error = wire_spec(AudioSpec::new(44_100, 300)).unwrap_err();
        assert!(matches!(error, CoreError::InvalidArgument(_)), "{error}");
        assert!(error.to_string().contains("300"), "{error}");
    }

    #[test]
    fn dsp_failures_are_loud_dsp_errors() {
        for error in [
            MediaHostError::TimedOut,
            MediaHostError::Disconnected("control stream ended".into()),
            MediaHostError::OperationFailed("unknown DSP stage: fuzz".into()),
            MediaHostError::InvalidMessage("expected DspReady".into()),
            MediaHostError::NotImplemented("no resident linked".into()),
        ] {
            let core = dsp_error_to_core(error, "write");
            assert!(matches!(core, CoreError::Dsp(_)), "{core:?}");
            assert!(core.to_string().contains("resident DSP"), "{core}");
        }
    }

    #[test]
    fn dsp_failure_watch_latches_the_first_failure_once() {
        let watch = DspFailureWatch::new();
        assert!(watch.take().is_none());
        watch.latch(ErrorInfo {
            kind: ErrorKind::Other,
            message: "first".into(),
        });
        watch.latch(ErrorInfo {
            kind: ErrorKind::Other,
            message: "second".into(),
        });
        let info = watch.take().expect("the first failure must be latched");
        assert_eq!(info.message, "first");
        assert!(watch.take().is_none(), "the watch is handled exactly once");
    }

    #[test]
    fn dsp_config_defaults_and_overrides() {
        let config = DspResidentConfig::new(
            PathBuf::from("helper.exe"),
            vec![DspStageDesc {
                id: "gain".into(),
                params: 2.0f32.to_le_bytes().to_vec(),
            }],
        );
        assert_eq!(config.ring_frames, DEFAULT_DSP_RING_FRAMES);
        assert_eq!(config.block_timeout, DEFAULT_DSP_BLOCK_TIMEOUT);
        assert_eq!(config.stages.len(), 1);
        let config = config
            .with_ring_frames(64)
            .with_block_timeout(Duration::from_millis(20));
        assert_eq!(config.ring_frames, 64);
        assert_eq!(config.block_timeout, Duration::from_millis(20));
    }

    #[test]
    fn dsp_zero_ring_frames_is_rejected_loudly() {
        let config =
            DspResidentConfig::new(PathBuf::from("helper.exe"), Vec::new()).with_ring_frames(0);
        let watch = Arc::new(DspFailureWatch::new());
        // On non-Windows this is Unsupported (no spawn platform); on Windows
        // it is InvalidArgument before any process exists. Either way no
        // helper is spawned for a zero capacity.
        match config.open(AudioSpec::new(44_100, 2), watch) {
            Err(CoreError::InvalidArgument(_)) => {}
            #[cfg(not(windows))]
            Err(CoreError::Unsupported(_)) => {}
            Err(error) => panic!("zero ring frames must not open, got {error}"),
            Ok(_) => panic!("zero ring frames must not open"),
        }
    }

    #[test]
    fn dsp_missing_helper_is_stage_unavailable() {
        let error = spawn_error_to_core(MediaHostError::OperationFailed(
            "DSP helper unavailable: program not found".into(),
        ));
        assert!(matches!(error, CoreError::Unsupported(_)), "{error}");
        assert!(error.to_string().contains("stage unavailable"), "{error}");
        assert!(error.to_string().contains("resident DSP helper"), "{error}");
        // Any other spawn failure is a loud Dsp error, never silence.
        let error = spawn_error_to_core(MediaHostError::OperationFailed(
            "unknown DSP stage: fuzz".into(),
        ));
        assert!(matches!(error, CoreError::Dsp(_)), "{error}");
    }
}

#[cfg(all(test, windows))]
mod dsp_hosted_tests {
    //! Adapter and engine behaviour over a real in-process DSP session with a
    //! fake resident. Headless, one Windows process, no helper binary
    //! spawned: byte-stream control pairs exercise the production framing and
    //! the real named rings, exactly like the framework's own suite.

    use std::collections::VecDeque;
    use std::io::{Cursor, Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Instant;

    use rustamp_core::audio::{
        AudioSpec, Confidence, Decoder, DecoderFactory, DecoderRegistry, MediaSource, ProbeHint,
    };
    use rustamp_core::bus::{EventBus, EventReceiver};
    use rustamp_core::error::{Error as CoreError, Result as CoreResult};
    use rustamp_core::event::Event;
    use rustamp_core::meta::TrackMetadata;
    use rustamp_core::state::{StreamInfo, TransportState};
    use rustamp_mediahost::control::{ControlPeer, Liveness};
    use rustamp_mediahost::dsp::{DspResident, DspSession, ResidentChain, serve_dsp};
    use rustamp_mediahost::pcm::NamedPcmRings;
    use rustamp_mediahost::{MediaHostError, Result as HostResult};
    use rustamp_output::{NullClock, NullOutput};
    use rustamp_proto::host::{DspStageDesc, PcmSpecWire};

    use crate::decode::TrackSource;
    use crate::engine::{Engine, EngineConfig};

    use super::*;

    const SPEC: AudioSpec = AudioSpec::new(44_100, 2);
    const SPEC_WIRE: PcmSpecWire = PcmSpecWire {
        sample_rate_hz: 44_100,
        channels: 2,
    };
    const WAIT: Duration = Duration::from_secs(5);

    /// Probe bound for the asynchronous `DspReset`: how many host-serviced
    /// blocks a cleared-state probe may take to observe the reset. A progress
    /// bound, not a time bound — iterations advance only on completed block
    /// round-trips (each bounded by the block timeout and loud on expiry), so
    /// load cannot expire it early the way a wall-clock sleep expires. Attempt
    /// one is the common case; the rest covers consecutive control-plane
    /// scheduling blackouts. Only the VM can prove the bound always suffices.
    const RESET_SETTLE_BLOCKS: usize = 16;

    fn gain_desc(factor: f32) -> DspStageDesc {
        DspStageDesc {
            id: "gain".into(),
            params: factor.to_le_bytes().to_vec(),
        }
    }

    fn stage(id: &str) -> DspStageDesc {
        DspStageDesc {
            id: id.into(),
            params: Vec::new(),
        }
    }

    fn delay_desc(latency: u32) -> DspStageDesc {
        DspStageDesc {
            id: "fake:delay".into(),
            params: latency.to_le_bytes().to_vec(),
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Gain,
        Delay,
        Slow,
        Die,
    }

    /// One fake resident stage: a gain, or a delay line that holds back
    /// `latency` frames and reports them (the look-ahead shape), or a
    /// wedge/crash surrogate.
    struct FakeChain {
        kind: Kind,
        factor: f32,
        latency: usize,
        channels: usize,
        pending: VecDeque<f32>,
        calls: usize,
    }

    impl FakeChain {
        fn new(kind: Kind, factor: f32, latency: usize, channels: usize) -> Self {
            let mut pending = VecDeque::new();
            pending.resize(latency * channels, 0.0);
            FakeChain {
                kind,
                factor,
                latency,
                channels,
                pending,
                calls: 0,
            }
        }
    }

    impl ResidentChain for FakeChain {
        fn latency_frames(&self) -> u32 {
            self.latency as u32
        }

        fn process(&mut self, buf: &mut [f32]) {
            self.calls += 1;
            match self.kind {
                Kind::Gain => {
                    for sample in buf.iter_mut() {
                        *sample *= self.factor;
                    }
                }
                Kind::Delay => {
                    for frame in buf.chunks_exact_mut(self.channels) {
                        for sample in frame.iter_mut() {
                            let delayed = self.pending.pop_front().unwrap_or(0.0);
                            self.pending.push_back(*sample);
                            *sample = delayed;
                        }
                    }
                }
                Kind::Slow => {
                    if self.calls == 1 {
                        thread::sleep(Duration::from_millis(150));
                    }
                    for sample in buf.iter_mut() {
                        *sample *= self.factor;
                    }
                }
                Kind::Die => {
                    if self.calls == 3 {
                        panic!("fake resident crash");
                    }
                    for sample in buf.iter_mut() {
                        *sample *= self.factor;
                    }
                }
            }
        }

        fn reset(&mut self) {
            self.pending.clear();
            self.pending.resize(self.latency * self.channels, 0.0);
        }
    }

    /// An ordered list of fake stages as one chain.
    struct MultiChain {
        stages: Vec<FakeChain>,
        latency: u32,
    }

    impl ResidentChain for MultiChain {
        fn latency_frames(&self) -> u32 {
            self.latency
        }

        fn process(&mut self, buf: &mut [f32]) {
            for stage in &mut self.stages {
                stage.process(buf);
            }
        }

        fn reset(&mut self) {
            for stage in &mut self.stages {
                stage.reset();
            }
        }
    }

    /// Fake DSP resident: `gain`/`passthrough` behave like the production
    /// stages; the `fake:*` ids are test-only modes (a delay line, a wedge, a
    /// crash, a configure refusal, a reconfigure refusal).
    struct FakeDspResident {
        opens: AtomicUsize,
    }

    impl FakeDspResident {
        fn new() -> Self {
            FakeDspResident {
                opens: AtomicUsize::new(0),
            }
        }
    }

    impl DspResident for FakeDspResident {
        fn open(
            &self,
            spec: PcmSpecWire,
            stages: &[DspStageDesc],
        ) -> HostResult<Box<dyn ResidentChain>> {
            if spec.sample_rate_hz == 0 || spec.channels == 0 {
                return Err(MediaHostError::InvalidMessage("zero PCM spec".into()));
            }
            let channels = usize::from(spec.channels);
            let mut built = Vec::new();
            let mut latency = 0u32;
            for desc in stages {
                let chain = match desc.id.as_str() {
                    "gain" => {
                        if desc.params.len() != 4 {
                            return Err(MediaHostError::OperationFailed(
                                "gain params must be 4 LE bytes".into(),
                            ));
                        }
                        let factor = f32::from_le_bytes([
                            desc.params[0],
                            desc.params[1],
                            desc.params[2],
                            desc.params[3],
                        ]);
                        if !factor.is_finite() {
                            return Err(MediaHostError::OperationFailed(
                                "gain factor must be finite".into(),
                            ));
                        }
                        FakeChain::new(Kind::Gain, factor, 0, channels)
                    }
                    "passthrough" => FakeChain::new(Kind::Gain, 1.0, 0, channels),
                    "fake:delay" => {
                        if desc.params.len() != 4 {
                            return Err(MediaHostError::OperationFailed(
                                "delay params must be 4 LE bytes".into(),
                            ));
                        }
                        let frames = u32::from_le_bytes([
                            desc.params[0],
                            desc.params[1],
                            desc.params[2],
                            desc.params[3],
                        ]);
                        if frames == 0 || frames > 4096 {
                            return Err(MediaHostError::OperationFailed(
                                "delay latency out of range".into(),
                            ));
                        }
                        FakeChain::new(Kind::Delay, 1.0, frames as usize, channels)
                    }
                    "fake:slow" => FakeChain::new(Kind::Slow, 2.0, 0, channels),
                    "fake:die" => FakeChain::new(Kind::Die, 2.0, 0, channels),
                    "fake:refuse" => {
                        return Err(MediaHostError::OperationFailed(
                            "fake resident refuses this chain".into(),
                        ));
                    }
                    "fake:reconfig-refuse" => {
                        if self.opens.fetch_add(1, Ordering::Relaxed) > 0 {
                            return Err(MediaHostError::OperationFailed(
                                "fake resident refuses a reconfigure".into(),
                            ));
                        }
                        FakeChain::new(Kind::Gain, 1.0, 0, channels)
                    }
                    unknown => {
                        return Err(MediaHostError::OperationFailed(format!(
                            "unknown DSP stage: {unknown}"
                        )));
                    }
                };
                latency += chain.latency as u32;
                built.push(chain);
            }
            Ok(Box::new(MultiChain {
                stages: built,
                latency,
            }))
        }
    }

    struct ByteReader {
        rx: mpsc::Receiver<Vec<u8>>,
        bytes: Cursor<Vec<u8>>,
    }

    struct ByteWriter(mpsc::Sender<Vec<u8>>);

    impl Read for ByteReader {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if out.is_empty() {
                return Ok(0);
            }
            loop {
                let count = self.bytes.read(out)?;
                if count > 0 {
                    return Ok(count);
                }
                match self.rx.recv() {
                    Ok(bytes) => self.bytes = Cursor::new(bytes),
                    Err(_) => return Ok(0),
                }
            }
        }
    }

    impl Write for ByteWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .send(bytes.to_vec())
                .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn streams() -> ((ByteReader, ByteWriter), (ByteReader, ByteWriter)) {
        let (a_tx, b_rx) = mpsc::channel();
        let (b_tx, a_rx) = mpsc::channel();
        (
            (
                ByteReader {
                    rx: a_rx,
                    bytes: Cursor::new(Vec::new()),
                },
                ByteWriter(a_tx),
            ),
            (
                ByteReader {
                    rx: b_rx,
                    bytes: Cursor::new(Vec::new()),
                },
                ByteWriter(b_tx),
            ),
        )
    }

    type HostOutcome = mpsc::Receiver<std::thread::Result<HostResult<()>>>;

    /// Open a resident DSP session in-process: a byte-stream control pair and
    /// the real named rings, with `serve_dsp` on a host thread. No helper
    /// binary is spawned. The host thread is wrapped in `catch_unwind` so a
    /// crashing fake resident is reaped with a bound instead of poisoning the
    /// test process.
    fn open_session(
        stages: Vec<DspStageDesc>,
        spec: PcmSpecWire,
        ring_frames: u32,
    ) -> (DspSession<NamedPcmRings>, HostOutcome) {
        let ((ar, aw), (br, bw)) = streams();
        let client = ControlPeer::start(ar, aw, Liveness::default(), false).unwrap();
        let host = ControlPeer::start(br, bw, Liveness::default(), true).unwrap();
        let (done, result) = mpsc::channel();
        thread::spawn(move || {
            let resident = FakeDspResident::new();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                serve_dsp(&host, &NamedPcmRings, &resident)
            }));
            let _ = done.send(outcome);
        });
        let session = DspSession::open(client, NamedPcmRings, spec, &stages, ring_frames).unwrap();
        (session, result)
    }

    fn open_chain(
        stages: Vec<DspStageDesc>,
        spec: PcmSpecWire,
        ring_frames: u32,
        block_timeout: Duration,
    ) -> (
        HostDspChain<NamedPcmRings>,
        Arc<DspFailureWatch>,
        HostOutcome,
    ) {
        let watch = Arc::new(DspFailureWatch::new());
        let (session, host) = open_session(stages.clone(), spec, ring_frames);
        let chain = HostDspChain::new(
            session,
            stages,
            ring_frames,
            block_timeout,
            Arc::clone(&watch),
        );
        (chain, watch, host)
    }

    fn reap(host: HostOutcome) {
        let _ = host.recv_timeout(WAIT);
    }

    #[test]
    fn dsp_gain_round_trip_is_bit_exact_with_zero_latency() {
        let (mut chain, _watch, host) = open_chain(vec![gain_desc(2.0)], SPEC_WIRE, 8, WAIT);
        assert_eq!(chain.name(), "resident-dsp");
        assert_eq!(chain.latency_frames(), 0);
        let input: Vec<f32> = (0..7)
            .flat_map(|i| [i as f32 / 4.0, -(i as f32 / 4.0)])
            .collect();
        let mut buf = input.clone();
        chain.process(&mut buf);
        for (got, want) in buf.iter().zip(input.iter()) {
            assert_eq!(*got, want * 2.0);
        }
        assert_eq!(chain.session().frames_dropped(), 0);
        drop(chain);
        let outcome = host.recv_timeout(WAIT).expect("host must finish");
        assert!(
            matches!(outcome, Ok(Ok(()))),
            "a graceful drop ends the host with Ok, got {outcome:?}"
        );
    }

    #[test]
    fn dsp_delay_chain_round_trip_is_the_delayed_stream_and_reset_clears_it() {
        const LATENCY: u32 = 3;
        let (mut chain, _watch, host) = open_chain(vec![delay_desc(LATENCY)], SPEC_WIRE, 8, WAIT);
        assert_eq!(chain.latency_frames(), LATENCY);

        // The block comes back delayed by the chain's latency: the first
        // LATENCY frames are the chain's initial state (zeros).
        let input: Vec<f32> = (0..10).flat_map(|i| [i as f32, -(i as f32)]).collect();
        let mut expected = vec![0.0; LATENCY as usize * 2];
        expected.extend_from_slice(&input[..(10 - LATENCY as usize) * 2]);
        let mut buf = input.clone();
        chain.process(&mut buf);
        assert_eq!(buf, expected);

        // A reset (seek) clears the chain state — the next cleared block
        // starts from the initial state again — and never touches the rings.
        let generations = chain.session().generations();
        chain.reset();
        assert_eq!(chain.session().generations(), generations);
        // The resident applies `DspReset` asynchronously (no ordering fence),
        // so the cleared state is eventual: re-send the reset and re-probe
        // until the cleared output appears, bounded in host-serviced blocks
        // (progress, never wall-clock). Stale output carries the previous
        // block's nonzero tail, so this cannot false-pass; a reset that never
        // lands fails the final assertion.
        let mut buf = input.clone();
        let mut settled = false;
        for _ in 0..RESET_SETTLE_BLOCKS {
            chain.reset();
            buf.copy_from_slice(&input);
            chain.process(&mut buf);
            if buf == expected {
                settled = true;
                break;
            }
        }
        assert!(settled, "reset must clear the resident's state");
        assert_eq!(buf, expected);

        drop(chain);
        let outcome = host.recv_timeout(WAIT).expect("host must finish");
        assert!(matches!(outcome, Ok(Ok(()))), "{outcome:?}");
    }

    #[test]
    fn dsp_prepare_reconfigures_on_a_format_change() {
        let (mut chain, _watch, host) = open_chain(vec![gain_desc(2.0)], SPEC_WIRE, 8, WAIT);
        let generations = chain.session().generations();
        chain.prepare(AudioSpec::new(48_000, 1)).unwrap();
        assert_eq!(
            chain.session().spec(),
            PcmSpecWire {
                sample_rate_hz: 48_000,
                channels: 1
            }
        );
        assert_ne!(
            chain.session().generations(),
            generations,
            "a reconfigure bumps both generations"
        );
        let mut buf = [0.25f32, -0.5, 0.125];
        chain.process(&mut buf);
        assert_eq!(buf, [0.5, -1.0, 0.25]);
        drop(chain);
        reap(host);
    }

    #[test]
    fn dsp_prepare_with_the_same_format_clears_state_without_reconfigure() {
        const LATENCY: u32 = 2;
        let (mut chain, _watch, host) = open_chain(vec![delay_desc(LATENCY)], SPEC_WIRE, 8, WAIT);
        let input: Vec<f32> = (0..6).flat_map(|i| [i as f32, -(i as f32)]).collect();
        let mut buf = input.clone();
        chain.process(&mut buf);
        // The delay line now holds the block's tail: without a reset the next
        // block would start with that tail instead of the initial state.
        let generations = chain.session().generations();
        chain.prepare(SPEC).unwrap();
        assert_eq!(
            chain.session().generations(),
            generations,
            "a same-format prepare never touches the rings"
        );
        let mut expected = vec![0.0; LATENCY as usize * 2];
        expected.extend_from_slice(&input[..(6 - LATENCY as usize) * 2]);
        // Same eventual-consistency note as the delay round-trip above: the
        // same-format prepare clears through the asynchronous reset, so
        // re-prepare and re-probe until the cleared output appears, bounded
        // in host-serviced blocks. The tail is nonzero, so no false pass.
        let mut buf = input.clone();
        let mut settled = false;
        for _ in 0..RESET_SETTLE_BLOCKS {
            chain.prepare(SPEC).unwrap();
            buf.copy_from_slice(&input);
            chain.process(&mut buf);
            if buf == expected {
                settled = true;
                break;
            }
        }
        assert!(settled, "prepare must clear the resident's state");
        assert_eq!(buf, expected, "prepare must clear the resident's state");
        drop(chain);
        reap(host);
    }

    #[test]
    fn dsp_refusal_at_prepare_is_loud_and_poisons_the_chain() {
        let (mut chain, watch, host) =
            open_chain(vec![stage("fake:reconfig-refuse")], SPEC_WIRE, 8, WAIT);
        let error = chain.prepare(AudioSpec::new(48_000, 1)).unwrap_err();
        assert!(matches!(error, CoreError::Dsp(_)), "{error}");
        assert!(
            error.to_string().contains("refuses a reconfigure"),
            "{error}"
        );
        // The refusal is latched for the engine and the chain bypasses.
        let info = watch.take().expect("the refusal must latch");
        assert!(info.message.contains("resident DSP"), "{info:?}");
        assert_eq!(
            chain.latency_frames(),
            0,
            "a poisoned chain is an undelayed bypass"
        );
        let input = [0.5f32, -0.25];
        let mut buf = input;
        chain.process(&mut buf);
        assert_eq!(buf, input, "a poisoned chain passes audio through");
        // The sticky error is returned again, without touching the session.
        assert!(matches!(
            chain.prepare(SPEC).unwrap_err(),
            CoreError::Dsp(_)
        ));
        drop(chain);
        let outcome = host.recv_timeout(WAIT).expect("host must finish");
        assert!(
            matches!(outcome, Ok(Err(_))),
            "a refused reconfigure ends the host with an error, {outcome:?}"
        );
    }

    #[test]
    fn dsp_death_mid_stream_is_loud_and_poisons_the_chain() {
        let (mut chain, watch, host) = open_chain(vec![stage("fake:die")], SPEC_WIRE, 8, WAIT);
        // Two good blocks (the fake crashes on its third process call).
        let mut buf = [0.5f32, -0.25, 0.125, 0.0625];
        chain.process(&mut buf);
        assert_eq!(buf, [1.0, -0.5, 0.25, 0.125]);
        let mut buf = [0.5f32, -0.25, 0.125, 0.0625];
        chain.process(&mut buf);
        assert_eq!(buf, [1.0, -0.5, 0.25, 0.125]);
        // The third block round-trip dies with the host: the adapter latches
        // a loud failure and leaves the buffer untouched.
        let input = [0.5f32, -0.25, 0.125, 0.0625];
        let mut buf = input;
        chain.process(&mut buf);
        assert_eq!(buf, input, "a dead resident must not corrupt the block");
        let info = watch.take().expect("the death must latch");
        assert!(info.message.contains("resident DSP"), "{info:?}");
        assert_eq!(chain.latency_frames(), 0);
        // The host thread died with the crash (bounded reap via the channel).
        let outcome = host.recv_timeout(WAIT).expect("host must finish");
        assert!(outcome.is_err(), "the crash panic, got {outcome:?}");
        drop(chain);
    }

    #[test]
    fn dsp_wedge_is_a_bounded_wait_and_poisons_the_chain() {
        let (mut chain, watch, host) = open_chain(
            vec![stage("fake:slow")],
            SPEC_WIRE,
            8,
            Duration::from_millis(20),
        );
        let input = [0.5f32, -0.25, 0.125, 0.0625];
        let mut buf = input;
        let start = Instant::now();
        chain.process(&mut buf);
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(1),
            "the block round-trip must be bounded, took {elapsed:?}"
        );
        assert_eq!(buf, input, "a wedged resident must not corrupt the block");
        let info = watch.take().expect("the wedge must latch");
        assert!(info.message.contains("timed out"), "{info:?}");
        assert_eq!(chain.latency_frames(), 0);
        drop(chain);
        let outcome = host.recv_timeout(WAIT).expect("host must finish");
        assert!(outcome.is_ok(), "{outcome:?}");
    }

    // -----------------------------------------------------------------------
    // Engine level: the adapter installed as the engine's DSP chain
    // -----------------------------------------------------------------------

    /// The sample value at `frame` for track `seed`: never zero, so silence
    /// in the output is always detectable.
    fn pattern(seed: u64, frame: u64) -> f32 {
        let x = frame
            .wrapping_mul(2_654_435_761)
            .wrapping_add(seed.wrapping_mul(7919))
            % 2001;
        0.1 + x as f32 / 2000.0
    }

    fn expected(seed: u64, from: u64, n: u64) -> Vec<f32> {
        (0..n).map(|i| pattern(seed, from + i)).collect()
    }

    /// In-process decoder factory for `track<seed>_<frames>.tst` placeholders.
    struct PatternFactory;

    impl DecoderFactory for PatternFactory {
        fn name(&self) -> &str {
            "pattern"
        }

        fn extensions(&self) -> &[&str] {
            &["tst"]
        }

        fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
            if hint.extension == Some("tst") {
                Confidence::CONTENT
            } else {
                Confidence::NONE
            }
        }

        fn open(&self, source: MediaSource) -> CoreResult<Box<dyn Decoder>> {
            let name = match &source {
                MediaSource::Path(p) => p.display().to_string(),
                MediaSource::Url(u) => u.clone(),
                MediaSource::Reader { .. } => {
                    return Err(CoreError::Unsupported("fake decoder takes paths only"));
                }
            };
            let stem = name.rsplit(['/', '\\']).next().unwrap_or(name.as_str());
            let stem = stem.strip_suffix(".tst").unwrap_or(stem);
            let (seed, frames) = stem
                .strip_prefix("track")
                .and_then(|rest| rest.split_once('_'))
                .map(|(s, f)| (s.parse().unwrap_or(1), f.parse().unwrap_or(3000)))
                .unwrap_or((1, 3000));
            Ok(Box::new(PatternDecoder {
                seed,
                total: frames,
                pos: 0,
            }))
        }
    }

    struct PatternDecoder {
        seed: u64,
        total: u64,
        pos: u64,
    }

    impl Decoder for PatternDecoder {
        fn spec(&self) -> AudioSpec {
            SPEC
        }

        fn read(&mut self, out: &mut [f32]) -> CoreResult<usize> {
            let n = (out.len() / 2).min((self.total - self.pos) as usize);
            for i in 0..n {
                let v = pattern(self.seed, self.pos + i as u64);
                out[i * 2] = v;
                out[i * 2 + 1] = v;
            }
            self.pos += n as u64;
            Ok(n)
        }

        fn seek(&mut self, to: Duration) -> CoreResult<Duration> {
            let frames = (to.as_secs_f64() * f64::from(SPEC.sample_rate)).round() as u64;
            self.pos = frames.min(self.total);
            Ok(SPEC.duration_of(self.pos))
        }

        fn duration(&self) -> Option<Duration> {
            Some(SPEC.duration_of(self.total))
        }

        fn metadata(&self) -> TrackMetadata {
            TrackMetadata::default()
        }

        fn stream_info(&self) -> StreamInfo {
            StreamInfo {
                bitrate_kbps: None,
                sample_rate_hz: SPEC.sample_rate,
                channels: SPEC.channels,
            }
        }

        fn skipped_packets(&self) -> u64 {
            0
        }
    }

    /// An engine whose installed DSP chain runs in the fake resident.
    struct DspHarness {
        engine: Engine,
        clock: NullClock,
        events: EventReceiver,
        host: HostOutcome,
        _dir: tempfile::TempDir,
    }

    impl DspHarness {
        fn new(names: &[&str], stages: Vec<DspStageDesc>, block_timeout: Duration) -> DspHarness {
            let dir = tempfile::tempdir().expect("temp dir");
            let (output, clock) = NullOutput::pair();
            let bus = EventBus::new();
            let events = bus.subscribe();
            let mut registry = DecoderRegistry::new();
            registry.register(Arc::new(PatternFactory));
            let mut engine = Engine::new(registry, Box::new(output), bus, EngineConfig::default())
                .expect("engine construction");
            // Bit-exact comparisons: the default-on limiter would clamp the
            // synthetic pattern's overs.
            engine.set_limiter_enabled(false);
            let (session, host) = open_session(stages.clone(), SPEC_WIRE, DEFAULT_DSP_RING_FRAMES);
            let chain = HostDspChain::new(
                session,
                stages,
                DEFAULT_DSP_RING_FRAMES,
                block_timeout,
                engine.resident_dsp_watch(),
            );
            engine.set_dsp_chain(vec![Box::new(chain)]);
            let queue = names
                .iter()
                .map(|name| {
                    let path = dir.path().join(name);
                    std::fs::write(&path, b"").expect("placeholder track");
                    TrackSource::path(path)
                })
                .collect();
            engine.set_queue(queue);
            DspHarness {
                engine,
                clock,
                events,
                host,
                _dir: dir,
            }
        }

        fn drain_events(&mut self) -> Vec<Event> {
            self.events.try_iter().collect()
        }

        fn wait_until(&mut self, mut cond: impl FnMut(&Engine) -> bool, max_frames: u64) -> bool {
            if cond(&self.engine) {
                return true;
            }
            let mut pumped = 0;
            while pumped < max_frames {
                self.engine.poll().expect("poll");
                if cond(&self.engine) {
                    return true;
                }
                self.clock.pump(256);
                pumped += 256;
                thread::sleep(Duration::from_millis(1));
            }
            cond(&self.engine)
        }

        /// Poll and pump until `cond` holds or `timeout` elapses. Unlike
        /// `wait_until`, which bounds a wait by pumped frames, this waits on
        /// the run's own condition with a wall-clock stall escape: a pump
        /// only advances the run while the ring holds audio, so a pump count
        /// can expire on underruns (the start drain, the StartDrained
        /// handshake, a lagging decode thread) without the run having made
        /// progress. The escape is generous - only a wedged run reaches it.
        fn wait_pumping(
            &mut self,
            mut cond: impl FnMut(&Engine) -> bool,
            timeout: Duration,
        ) -> bool {
            let deadline = Instant::now() + timeout;
            loop {
                self.engine.poll().expect("poll");
                if cond(&self.engine) {
                    return true;
                }
                if Instant::now() >= deadline {
                    return false;
                }
                self.clock.pump(256);
                thread::sleep(Duration::from_millis(1));
            }
        }

        fn wait_idle(&mut self, mut cond: impl FnMut(&Engine) -> bool, timeout: Duration) -> bool {
            let deadline = Instant::now() + timeout;
            loop {
                self.engine.poll().expect("poll");
                if cond(&self.engine) {
                    return true;
                }
                if Instant::now() >= deadline {
                    return false;
                }
                thread::sleep(Duration::from_millis(1));
            }
        }

        fn wait_track(&mut self, index: u32) -> bool {
            self.wait_idle(
                |e| e.current_index() == Some(index) && e.duration_frames().is_some(),
                WAIT,
            )
        }

        fn wait_buffered(&mut self) -> bool {
            self.wait_idle(|e| e.end_of_stream().is_some(), WAIT)
        }

        fn settle_seek(&mut self, target: u64) -> bool {
            let deadline = Instant::now() + WAIT;
            loop {
                self.engine.poll().expect("poll");
                if self.engine.position_frames() == target {
                    return true;
                }
                if Instant::now() >= deadline {
                    return false;
                }
                self.clock.pump(64);
                thread::sleep(Duration::from_millis(1));
            }
        }

        /// Drop the engine (which drops the chain and shuts the resident
        /// down) and reap the host thread.
        fn finish(self) {
            drop(self.engine);
            let _ = self.host.recv_timeout(WAIT);
        }
    }

    #[test]
    fn engine_compensates_the_resident_latency_exactly() {
        // A resident chain with a real look-ahead: 100 frames of latency.
        // The decode thread must drop the chain's 100 start-up frames and
        // push 100 frames of silence at the end, so the ring receives the
        // track's 3000 frames exactly — no frame lost, none duplicated, no
        // silence at the seams.
        let mut h = DspHarness::new(&["track7_3000.tst"], vec![delay_desc(100)], WAIT);
        h.engine.play_index(0).unwrap();
        assert!(h.wait_track(0));
        assert!(
            h.wait_buffered(),
            "the track must buffer through the resident"
        );
        assert_eq!(h.engine.duration_frames(), Some(3000));

        let mut rec = Vec::new();
        let mut pumped = 0u64;
        while pumped < 3000 {
            let step = 500.min(3000 - pumped);
            h.clock.pump_record(step, &mut rec);
            pumped += step;
            h.engine.poll().unwrap();
        }
        let left: Vec<f32> = rec.iter().step_by(2).copied().collect();
        assert_eq!(left.len(), 3000);
        assert_eq!(&left[..], &expected(7, 0, 3000)[..]);

        assert!(
            h.wait_until(|e| e.transport() == TransportState::Stopped, 3000 + 8192),
            "the run must finish"
        );
        assert_eq!(h.engine.position_frames(), 3000);
        assert_eq!(h.engine.buffered_frames(), 0);
        assert!(h.drain_events().contains(&Event::PlaylistEnded));
        h.finish();
    }

    #[test]
    fn engine_gapless_seam_through_the_resident_chain_is_exact() {
        let mut h = DspHarness::new(
            &["track7_3000.tst", "track2_3000.tst"],
            vec![delay_desc(100)],
            WAIT,
        );
        h.engine.play_index(0).unwrap();
        assert!(h.wait_track(0));
        assert!(
            h.wait_buffered(),
            "both tracks must buffer through the resident"
        );

        let mut rec = Vec::new();
        let mut pumped = 0u64;
        while pumped < 6000 {
            let step = 500.min(6000 - pumped);
            h.clock.pump_record(step, &mut rec);
            pumped += step;
            h.engine.poll().unwrap();
        }
        let left: Vec<f32> = rec.iter().step_by(2).copied().collect();
        assert_eq!(left.len(), 6000);
        assert_eq!(&left[..3000], &expected(7, 0, 3000)[..]);
        assert_eq!(&left[3000..], &expected(2, 0, 3000)[..]);
        assert_eq!(left[2999], pattern(7, 2999));
        assert_eq!(left[3000], pattern(2, 0));
        assert!(!left.contains(&0.0), "the seam must not contain silence");

        assert!(
            h.wait_until(
                |e| e.transport() == TransportState::Stopped,
                // Consumption bound in 256-frame pump steps: the joint 6000
                // frames plus one full engine ring of overlap slack for the
                // drain/ack handshake and loaded-VM pump skew. The exact
                // assertions below do the real work; this only bounds
                // pathology.
                6000 + 16_384
            ),
            "the run must finish"
        );
        assert_eq!(h.engine.position_frames(), 3000);
        assert_eq!(h.engine.current_index(), Some(1));
        assert!(h.drain_events().contains(&Event::PlaylistEnded));
        h.finish();
    }

    #[test]
    fn engine_seek_through_the_resident_chain_lands_exactly() {
        let mut h = DspHarness::new(&["track7_44100.tst"], vec![delay_desc(100)], WAIT);
        h.engine.play_index(0).unwrap();
        assert!(h.wait_track(0));
        h.engine.seek_frames(1000).unwrap();
        assert!(h.settle_seek(1000), "the seek must land on 1000");

        // Wait for fresh post-seek audio without pumping silence in between,
        // then compare against the pattern at the settled position.
        let deadline = Instant::now() + WAIT;
        let p0 = loop {
            h.engine.poll().unwrap();
            if h.engine.buffered_frames() >= 4096 {
                break h.engine.position_frames();
            }
            if Instant::now() >= deadline {
                panic!("the ring must refill after the seek");
            }
            thread::sleep(Duration::from_millis(1));
        };
        let mut rec = Vec::new();
        h.clock.pump_record(1024, &mut rec);
        h.engine.poll().unwrap();
        assert_eq!(h.engine.position_frames(), p0 + 1024);
        let left: Vec<f32> = rec.iter().step_by(2).copied().collect();
        assert_eq!(&left[..], &expected(7, p0, 1024)[..]);
        h.finish();
    }

    #[test]
    fn engine_fails_the_track_when_the_resident_dies() {
        let mut h = DspHarness::new(&["track7_44100.tst"], vec![stage("fake:die")], WAIT);
        h.engine.play_index(0).unwrap();
        assert!(h.wait_track(0));
        // The fake crashes on its third block: the adapter latches, the
        // engine's poll fails the track (RFC 0006 §8) — never the player.
        assert!(
            h.wait_idle(|e| e.transport() == TransportState::Stopped, WAIT),
            "the run must stop when the resident dies"
        );
        let position = h.engine.position_frames();
        assert!(
            position < 44_100,
            "the track must fail before its end (at {position})"
        );
        let events = h.drain_events();
        let error = events
            .iter()
            .find_map(|event| match event {
                Event::Error(info) => Some(info),
                _ => None,
            })
            .expect("a resident death must publish an error event");
        assert!(error.message.contains("resident DSP"), "{error:?}");
        assert!(h.engine.last_error().is_some());

        // The player survives: a new run plays (the poisoned chain bypasses
        // bit-exactly) and finishes naturally.
        h.engine.play_index(0).unwrap();
        assert_eq!(h.engine.transport(), TransportState::Playing);
        // The wait measures what the run actually needs - the transport
        // reaching Stopped - with a generous stall escape, not a pump-count
        // proxy: a pump only advances the run while the ring holds audio, so
        // counting pumped frames bounds pathology, never progress, and any
        // underrun (the start drain, a lagging decode thread, the
        // StartDrained handshake) burns budget without advancing the run. A
        // healthy run finishes in well under a second of pumping; only a
        // wedged run reaches the escape, and it fails loudly with the step
        // named. The claim is unchanged: the run must finish, and the
        // position must land exactly at 44_100.
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
            "the second run must finish"
        );
        assert_eq!(h.engine.position_frames(), 44_100);
        h.finish();
    }

    #[test]
    fn engine_fails_the_track_when_the_resident_wedges() {
        let mut h = DspHarness::new(
            &["track7_44100.tst"],
            vec![stage("fake:slow")],
            Duration::from_millis(20),
        );
        h.engine.play_index(0).unwrap();
        assert!(h.wait_track(0));
        // The resident sleeps on its first block; the adapter's bounded wait
        // (20 ms) latches a timeout and the engine fails the track — the
        // engine can never hang on a wedged resident.
        assert!(
            h.wait_idle(|e| e.transport() == TransportState::Stopped, WAIT),
            "the run must stop when the resident wedges"
        );
        let events = h.drain_events();
        let error = events
            .iter()
            .find_map(|event| match event {
                Event::Error(info) => Some(info),
                _ => None,
            })
            .expect("a wedged resident must publish an error event");
        assert!(error.message.contains("timed out"), "{error:?}");
        h.finish();
    }
}
