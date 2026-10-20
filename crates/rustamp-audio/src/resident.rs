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
//! # Supervision (RFC 0006 §8)
//!
//! The supervision policy belongs to the engine, not to the contract: the
//! engine spawns the helper, runs the `Hello`/`Ping` liveness on the control
//! channel and kills and reaps it on close. [`ResidentConfig::liveness`] is
//! that policy, and [`DEFAULT_RESIDENT_LIVENESS`] is its default — a stall
//! escape rather than the framework's 5 s wall-clock trip wire, because a
//! *dead* helper is caught by the control EOF at once and only a *wedged* one
//! waits for the `Pong` deadline, which the per-`read` bound already covers
//! much earlier.
//!
//! # Restart policy (RFC 0006 §8)
//!
//! RFC 0006 §8 puts the restart policy on the engine side: "Restart policy
//! (retry once, then fail the track) is engine policy, not contract." The
//! engine's policy is [`RestartingDecoder`]: a resident that fails mid-track
//! is **restarted once** — a fresh helper is spawned, the same source is
//! reopened and the track continues at the last position the engine had taken
//! from it. A second failure ends the track loudly with the existing
//! `Decode` error; the player is never the thing that fails.
//!
//! Three points are the engine's own, and the RFC does not decide them:
//!
//! * **Where the track resumes.** The RFC says "retry once" but not where.
//!   The narrow reading is a retry of the *helper*, not of the track: the
//!   track continues at the last known position, so no audio is replayed and
//!   none is skipped. That position is the number of frames the engine took
//!   from the resident, and the wire quantizes it to whole milliseconds (the
//!   same tolerance as any seek).
//! * **What counts as a failure worth a retry.** The RFC names the death
//!   signals ("a missed `Pong` or an unexpected close"), but the framework
//!   reports a dead helper, a wedged helper and a helper that reported a
//!   decode failure through one and the same channel, so the engine cannot
//!   tell them apart without a change to the media-host contract. The policy
//!   therefore retries *any* mid-track failure of the resident once — which
//!   is exactly "try once more, then drop the track".
//! * **What is lost.** Nothing that was audible: the frames the dead helper
//!   had produced but not yet delivered are re-produced by the restarted
//!   helper, because the resume position is the last position the *engine*
//!   had taken. The only imprecision is the millisecond the wire quantizes
//!   the resume to (the same tolerance as any seek). A resident that cannot
//!   seek is therefore never restarted after it delivered audio — a restart
//!   would replay the track from its start.
//!
//! The policy covers a failure *while the track plays*. A failure during a
//! seek is the engine's seek path (it reports the failed seek and keeps
//! playing); the next read then triggers the restart, which resumes at the
//! position before the seek.
//!
//! # Where the two-process path is proven
//!
//! The tests in this module drive the adapter over an in-process control pair.
//! The real thing — a helper in its own process, the real named ring, the
//! real control channel — is proven by `tests/resident_process.rs`, which is
//! `#[ignore]`d because it spawns a process and therefore runs only in the
//! Hyper-V test VM.
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
//! synchronous block round-trip (write the block in lockstep chunks, read
//! every chunk back, delayed by the chain's latency), that machinery is
//! correct across a seek (`reset` + trim) and across a gapless seam (flush
//! at the track end + trim at the next track's start) with no extra wiring:
//! no frame is lost, none is duplicated and no silence enters either seam.
//! If the resident's report disagrees with reality (a lying or buggy stage),
//! the compensation trims or flushes the wrong number of frames — the error
//! is bounded by the reported latency and shows up as a small shift or a
//! short tail, never as a hang or a crash; the report is trusted because
//! RFC 0006 §6.2 makes it the contract, exactly as for an in-process
//! stage's `latency_frames`.
//!
//! # The lockstep round-trip (no deadlock by construction)
//!
//! The adapter's block round-trip is a **lockstep**: the block is split into
//! chunks of at most [`DSP_CHUNK_FRAMES`] frames, and the adapter writes one
//! chunk, reads that chunk back, and only then writes the next one. The
//! resident worker moves frames strictly in order from the in-ring to its
//! back-ring, so the frames a chunk's read-back waits for are exactly that
//! chunk, and the frames in flight (written, not yet read back) never exceed
//! one chunk.
//!
//! That is what makes the round trip deadlock-free. A single write of the
//! whole block — the pattern the W14 review found in this adapter (finding
//! F1, claim C6-2) — fills the resident's back-ring before a single frame is
//! read back; the resident then blocks on its back-ring, stops taking from
//! the in-ring, and the player's write waits for a resident that waits for
//! the player's read: the bounded-buffer deadlock, bounded only by the block
//! timeout. With the lockstep the resident's back-ring holds at most one
//! chunk plus the block it keeps in hand, so it never blocks there, never
//! stops draining the in-ring, and the player's write always finds room.
//!
//! The chunk size is the resident worker's own block size, so the frames the
//! chain processes, their order and the values it returns do not change; the
//! chunk is additionally clamped to the in-ring capacity the caller chose, so
//! one chunk is always one ring write.
//!
//! # Mid-run failures
//!
//! The core `DspStage::process` contract is fallible (RFC 0014, closing M14
//! finding F1): the adapter returns a resident that dies, wedges or is
//! refused mid-stream as `Err(Error::Dsp(..))` from `process`, carrying the
//! failure text. The engine treats it exactly like a decoder error — report
//! it, flush the chain, end the track loudly (RFC 0006 §8: the resident's
//! death fails the track, never the player) — so no engine-polled latch is
//! needed anymore. The first failure sticks on the stage: a failed chain is
//! poisoned and resuming on it is an error (RFC 0014 §4.3); recovery is an
//! explicit replacement with a fresh chain. A refusal at configure is a loud
//! error at [`DspResidentConfig::open`] (or at `prepare` for a reconfigure,
//! which the decode thread discards — the poisoned stage returns the latched
//! refusal from its next `process`), before or instead of playback; a missing
//! helper binary degrades to "stage unavailable", never to a silent
//! in-process fallback.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rustamp_core::audio::{
    AudioSpec, Confidence, Decoder, DecoderFactory, DspStage, MediaSource, ProbeHint,
};
use rustamp_core::error::{Error as CoreError, Result as CoreResult};
use rustamp_core::state::StreamInfo;
use rustamp_mediahost::control::Liveness;
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

/// Liveness policy the engine uses for a resident helper.
///
/// Own choice, and RFC 0006 §8 puts the supervision policy on the engine
/// side: "the engine spawns the helper, runs `Hello`/`Ping` liveness on the
/// control channel, and kills and reaps it on close".
///
/// The framework's `Liveness::default()` treats a missed `Pong` as death
/// after 5 s. That is a wall-clock trip wire, and this repository's own test
/// guest proved that class: a healthy-but-slow schedule crosses it and the
/// track dies for a reason that never happened (docs/tasks/M24 §1.3). The two
/// failure modes are not equally urgent:
///
/// * A **dead** helper is caught at once by the control EOF — the peer sees
///   the closed stream, not a missing `Pong`.
/// * A **wedged** helper (alive, but not answering) is the only case this
///   deadline covers, and the engine's per-`read` bound
///   ([`DEFAULT_READ_TIMEOUT`]) already fails that track much earlier.
///
/// So the default here is a stall escape rather than a schedule bound, and
/// [`ResidentConfig::with_liveness`] lets a caller tighten it.
pub const DEFAULT_RESIDENT_LIVENESS: Liveness = Liveness {
    interval: Duration::from_millis(100),
    timeout: Duration::from_secs(30),
};

/// How many times a failed resident may be restarted within one track.
///
/// RFC 0006 §8: "Restart policy (retry once, then fail the track) is engine
/// policy, not contract." The RFC's own value is one; the engine keeps it as
/// a policy in [`ResidentConfig`] so a caller can tighten it (zero switches
/// the restart off) without touching the contract.
pub const DEFAULT_RESIDENT_RESTARTS: u32 = 1;

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
    /// `Hello`/`Ping` liveness the engine supervises the helper with
    /// (RFC 0006 §8). See [`DEFAULT_RESIDENT_LIVENESS`].
    pub liveness: Liveness,
    /// Restarts a failed resident may spend in one track (RFC 0006 §8). See
    /// [`DEFAULT_RESIDENT_RESTARTS`].
    pub restarts: u32,
}

impl ResidentConfig {
    /// Opt in to resident decoding through the helper at `helper`.
    pub fn new(helper: PathBuf) -> Self {
        ResidentConfig {
            helper,
            prefetch_frames: DEFAULT_PREFETCH_FRAMES,
            read_timeout: DEFAULT_READ_TIMEOUT,
            seek_timeout: DEFAULT_SEEK_TIMEOUT,
            liveness: DEFAULT_RESIDENT_LIVENESS,
            restarts: DEFAULT_RESIDENT_RESTARTS,
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

    /// Override the `Hello`/`Ping` liveness the engine supervises the helper
    /// with. The default is [`DEFAULT_RESIDENT_LIVENESS`].
    pub fn with_liveness(mut self, liveness: Liveness) -> Self {
        self.liveness = liveness;
        self
    }

    /// Override the restart budget of one track (RFC 0006 §8). The default
    /// is [`DEFAULT_RESIDENT_RESTARTS`]; zero switches the restart off, so a
    /// failed resident ends the track at once.
    pub fn with_restarts(mut self, restarts: u32) -> Self {
        self.restarts = restarts;
        self
    }
}

/// The wire location for one engine source, or a loud refusal.
///
/// RFC 0006 §5.1: the host opens the paths and URLs it can reach itself.
/// A `MediaSource::Reader` cannot cross the process boundary, and a
/// non-UTF-8 local path cannot be encoded on the wire yet (HANDOFF-3), so
/// both are refused loudly instead of being mangled or silently decoded
/// in-process.
fn host_location(source: &MediaSource) -> CoreResult<MediaLocationWire> {
    match source {
        MediaSource::Path(path) => {
            file_location(path).map_err(|error| CoreError::Decode(error.to_string()))
        }
        MediaSource::Url(url) => Ok(MediaLocationWire::Url(url.clone())),
        MediaSource::Reader { .. } => Err(CoreError::Unsupported(
            "reader sources stay in-process; the resident only opens paths and URLs",
        )),
    }
}

/// The engine format for one wire format, or a loud refusal.
///
/// The negotiation is one-sided (RFC 0006 §5.2): the host owns the format —
/// it answers `DecoderOpened` with a wire spec — and the engine adopts it.
/// A format the wire cannot carry (no channels, no sample rate) is refused
/// here instead of becoming an `AudioSpec` with a zero rate that would
/// divide by zero in the clock.
fn negotiated_spec(spec: PcmSpecWire) -> CoreResult<AudioSpec> {
    rustamp_mediahost::pcm::validate_spec(spec)
        .map_err(|error| CoreError::Decode(format!("resident handshake: {error}")))?;
    Ok(AudioSpec::new(
        spec.sample_rate_hz,
        u16::from(spec.channels),
    ))
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
            let to = negotiated_spec(session.info().spec)?;
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
            *spec = negotiated_spec(session.info().spec)?;
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
    /// handshake, so the spec is taken from it. A handshake the engine cannot
    /// adopt (no channels, no sample rate) is refused loudly instead of
    /// becoming a decoder with a broken clock.
    pub fn new(
        session: DecoderSession<F>,
        read_timeout: Duration,
        seek_timeout: Duration,
    ) -> CoreResult<Self> {
        let spec = negotiated_spec(session.info().spec)?;
        Ok(HostDecoder {
            session,
            read_timeout,
            seek_timeout,
            spec,
        })
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
    /// Wrap a spawned helper. Reads the handshake spec from its session; a
    /// format the engine cannot adopt is refused loudly.
    pub fn new(
        mut process: rustamp_mediahost::decoder::DecoderProcess,
        read_timeout: Duration,
        seek_timeout: Duration,
    ) -> CoreResult<Self> {
        let spec = negotiated_spec(process.session().info().spec)?;
        let pid = process.pid();
        Ok(ProcessDecoder {
            process: std::sync::Mutex::new(process),
            read_timeout,
            seek_timeout,
            spec,
            pid,
        })
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

/// Reopens a resident for the same source after a failure.
///
/// RFC 0006 §8 puts the restart policy on the engine side, and the engine
/// spawns the helper: the adapter knows *that* a helper has to come back,
/// but not *how* one is spawned. This is that seam. The production
/// implementation spawns a fresh helper process; the tests open a fresh
/// in-process session.
pub trait ResidentReopen: Send {
    /// Open a fresh decoder for the same source, at its start.
    ///
    /// The caller seeks it to the position the track had reached; a failure
    /// here ends the track.
    fn reopen(&mut self) -> CoreResult<Box<dyn Decoder>>;
}

/// A resident decoder that restarts its helper once per track
/// (RFC 0006 §8).
///
/// The engine's restart policy: "retry once, then fail the track". A
/// resident that fails mid-track is replaced by a fresh one — the helper is
/// spawned again, the same source is reopened and the track continues at the
/// last position the engine had taken from it. A second failure ends the
/// track loudly; the player never fails with it.
///
/// The wrapper is deliberately thin: every other call goes to the decoder in
/// use, so the engine keeps driving a single [`Decoder`] and there is no
/// second decode path (RFC 0006 §10.4).
pub struct RestartingDecoder {
    decoder: Box<dyn Decoder>,
    reopen: Box<dyn ResidentReopen>,
    /// Frames the engine took from the resident since the last seek: the
    /// position a restarted helper has to resume at.
    position_frames: u64,
    /// Damage the previous helpers reported. The counter is cumulative over
    /// the track (RFC 0002 C1), so a restart adds to it instead of resetting
    /// it.
    skipped_base: u64,
    /// Restarts this track has spent.
    restarts: u32,
    /// Restarts this track may spend.
    budget: u32,
}

impl RestartingDecoder {
    /// Wrap `decoder`; `reopen` brings the helper back and `restarts` is the
    /// number of restarts one track may spend.
    pub fn new(decoder: Box<dyn Decoder>, reopen: Box<dyn ResidentReopen>, restarts: u32) -> Self {
        RestartingDecoder {
            decoder,
            reopen,
            position_frames: 0,
            skipped_base: 0,
            restarts: 0,
            budget: restarts,
        }
    }

    /// The frames a duration names in a spec.
    fn frames_at(spec: AudioSpec, at: Duration) -> u64 {
        (at.as_secs_f64() * f64::from(spec.sample_rate.max(1))).round() as u64
    }

    /// Whether a failure may still be retried.
    ///
    /// The budget is per track, so the retry never loops. A resident that
    /// cannot seek is not restarted after it delivered audio: the fresh
    /// helper would replay the track from its start.
    fn can_restart(&self) -> bool {
        self.restarts < self.budget && (self.position_frames == 0 || self.decoder.is_seekable())
    }

    /// Restart the helper and reopen the same source at the last known
    /// position.
    ///
    /// The failure that triggered the restart is part of the message, so the
    /// report names both the cause and the restart.
    fn restart(&mut self, cause: &CoreError) -> CoreResult<()> {
        self.restarts += 1;
        let spec = self.decoder.spec();
        let mut fresh = self
            .reopen
            .reopen()
            .map_err(|error| restart_failure(cause, &error))?;
        if fresh.spec() != spec {
            // The engine adopted the old format when it opened the track; a
            // reopened helper that reports another one would be decoded with
            // the wrong converter, so the track ends instead.
            return Err(CoreError::Decode(
                "resident restart: the reopened helper reports another format".into(),
            ));
        }
        if self.position_frames > 0 {
            let target = Duration::from_millis(resume_ms(spec, self.position_frames));
            let reached = fresh
                .seek(target)
                .map_err(|error| restart_failure(cause, &error))?;
            // The wire quantizes the resume to whole milliseconds, so the
            // reached position is the truth from here on.
            self.position_frames = Self::frames_at(spec, reached);
        }
        self.skipped_base += self.decoder.skipped_packets();
        self.decoder = fresh;
        Ok(())
    }
}

impl Decoder for RestartingDecoder {
    fn spec(&self) -> AudioSpec {
        self.decoder.spec()
    }

    fn read(&mut self, out: &mut [f32]) -> CoreResult<usize> {
        match self.decoder.read(out) {
            Ok(frames) => {
                self.position_frames += frames as u64;
                Ok(frames)
            }
            // A format change is a transition, not a failure: the resident is
            // alive and the engine adopts the new format (RFC 0002 C3).
            Err(error @ CoreError::FormatChanged { .. }) => Err(error),
            Err(error) => {
                if !self.can_restart() {
                    return Err(error);
                }
                self.restart(&error)?;
                match self.decoder.read(out) {
                    Ok(frames) => {
                        self.position_frames += frames as u64;
                        Ok(frames)
                    }
                    // The second failure ends the track: the budget is spent
                    // and the retry never loops.
                    Err(second) => Err(CoreError::Decode(format!(
                        "resident restart did not save the track: {}",
                        error_text(&second)
                    ))),
                }
            }
        }
    }

    fn seek(&mut self, to: Duration) -> CoreResult<Duration> {
        let reached = self.decoder.seek(to)?;
        self.position_frames = Self::frames_at(self.decoder.spec(), reached);
        Ok(reached)
    }

    fn duration(&self) -> Option<Duration> {
        self.decoder.duration()
    }

    fn metadata(&self) -> rustamp_core::meta::TrackMetadata {
        self.decoder.metadata()
    }

    fn stream_info(&self) -> StreamInfo {
        self.decoder.stream_info()
    }

    fn is_seekable(&self) -> bool {
        self.decoder.is_seekable()
    }

    fn skipped_packets(&self) -> u64 {
        self.skipped_base + self.decoder.skipped_packets()
    }
}

/// The wire position of a frame count, rounded to the nearest millisecond.
///
/// The wire speaks whole milliseconds, so a resume lands within half a
/// millisecond of the position the engine had reached — the same tolerance as
/// any seek.
fn resume_ms(spec: AudioSpec, frames: u64) -> u64 {
    let rate = u64::from(spec.sample_rate.max(1));
    (frames * 1000 + rate / 2) / rate
}

/// The message of a core error, without the `decode error: ` prefix.
fn error_text(error: &CoreError) -> String {
    match error {
        CoreError::Decode(message) => message.clone(),
        other => other.to_string(),
    }
}
/// A failed restart as a loud decode error: the track ends, the player does
/// not (RFC 0006 §8).
fn restart_failure(cause: &CoreError, error: &CoreError) -> CoreError {
    CoreError::Decode(format!(
        "resident restart failed after {}: {}",
        error_text(cause),
        error_text(error)
    ))
}

/// Reopens a decoder resident in a fresh helper process (RFC 0006 §8).
#[cfg(windows)]
struct ProcessReopen {
    config: ResidentConfig,
    location: MediaLocationWire,
}

#[cfg(windows)]
impl ResidentReopen for ProcessReopen {
    fn reopen(&mut self) -> CoreResult<Box<dyn Decoder>> {
        use rustamp_mediahost::decoder::DecoderProcess;

        let process = DecoderProcess::spawn(
            &self.config.helper,
            self.location.clone(),
            self.config.prefetch_frames,
            self.config.liveness,
        )
        .map_err(|error| CoreError::Decode(format!("resident restart: {error}")))?;
        let decoder =
            ProcessDecoder::new(process, self.config.read_timeout, self.config.seek_timeout)?;
        Ok(Box::new(decoder))
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
        // The request mapping does not depend on the platform: what cannot
        // cross the boundary is refused before anything is spawned.
        let location = host_location(&source)?;
        #[cfg(not(windows))]
        {
            let _ = location;
            return Err(CoreError::Unsupported(
                "resident decoding requires Windows named PCM mappings",
            ));
        }
        #[cfg(windows)]
        {
            use rustamp_mediahost::decoder::DecoderProcess;

            if self.config.prefetch_frames == 0 {
                return Err(CoreError::InvalidArgument(
                    "resident prefetch_frames must be above zero".into(),
                ));
            }
            let process = DecoderProcess::spawn(
                &self.config.helper,
                location.clone(),
                self.config.prefetch_frames,
                self.config.liveness,
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
            let decoder =
                ProcessDecoder::new(process, self.config.read_timeout, self.config.seek_timeout)?;
            // The restart policy (RFC 0006 §8): a helper that fails mid-track
            // comes back once, and the track continues where it stopped.
            let reopen = ProcessReopen {
                config: self.config.clone(),
                location,
            };
            Ok(Box::new(RestartingDecoder::new(
                Box::new(decoder),
                Box::new(reopen),
                self.config.restarts,
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
/// The engine's join of the decode thread is bounded by this too. The bound
/// covers the **whole** round trip, every lockstep chunk included, so a
/// block of any size fails the track after the same time.
pub const DEFAULT_DSP_BLOCK_TIMEOUT: Duration = Duration::from_secs(5);

/// Frames per lockstep chunk in one resident block round-trip.
///
/// Own choice, and the number the deadlock fix turns on. The resident
/// worker moves frames strictly in order from the in-ring to its own
/// back-ring (4096 frames, `rustamp-mediahost`'s capacity) and keeps at
/// most one block (1024 frames) in hand while it does so. A player that
/// writes a whole block before it reads a single frame back therefore
/// fills the resident's back-ring, the resident stops taking from the
/// in-ring, and the two sides wait for each other — the bounded-buffer
/// deadlock (W14 finding F1).
///
/// The chunk is the lockstep unit: the adapter writes one chunk, reads
/// that chunk back, and only then writes the next one. The frames in
/// flight (written, not yet read back) never exceed one chunk, so the
/// resident's back-ring holds at most one chunk plus the block it keeps
/// in hand — 2048 of its 4096 frames — and the resident never blocks on
/// its back-ring, never stops draining the in-ring, and the player's
/// write always finds room. Neither wait can be permanent.
///
/// 1024 is also the resident worker's own block size, so the chunk
/// boundaries are the boundaries the chain already saw: the frames the
/// chain processes, their order and the values it returns do not change.
/// The chunk is additionally clamped to the in-ring capacity the caller
/// chose, so one chunk is always one ring write.
pub const DSP_CHUNK_FRAMES: usize = 1024;

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

/// One bounded block round-trip through the resident, in lockstep chunks.
///
/// The block is split into chunks of at most `chunk_frames` frames: write
/// one chunk, read that chunk back, then write the next. The resident
/// processes strictly in order, so the frames a chunk's read-back waits
/// for are exactly that chunk, and the block comes back delayed by the
/// chain's latency — exactly the `DspStage` contract the decode thread
/// compensates (`dsp_trim` after every prepare/reset, `flush_dsp` at the
/// end of every track). `Ok(0)` from a read is the session's sticky EOS,
/// which the adapter never asks for mid-stream: it is a loud failure,
/// never silence.
///
/// The lockstep is what makes the round trip deadlock-free. A single
/// write of the whole block fills the resident's back-ring before a
/// single frame is read back; the resident then blocks on its back-ring,
/// stops taking from the in-ring, and the write waits for a resident
/// that waits for the read — the deadlock the W14 review found in this
/// adapter (finding F1, claim C6-2). See [`DSP_CHUNK_FRAMES`].
///
/// The bound covers the **whole** round trip, not one chunk: the deadline
/// is taken once, so a wedged resident fails the track after
/// `block_timeout` however many chunks the block had.
fn process_block<F: PcmRings>(
    session: &mut DspSession<F>,
    buf: &mut [f32],
    block_timeout: Duration,
    chunk_frames: usize,
) -> CoreResult<()> {
    let channels = usize::from(session.spec().channels.max(1));
    if buf.is_empty() || !buf.len().is_multiple_of(channels) {
        // The `DspStage` contract promises whole frames; a block that is
        // not one is the caller's mistake, and it is refused before the
        // block is split — a partial frame is never dropped silently.
        return Err(CoreError::InvalidArgument(
            "resident DSP block must hold whole frames".into(),
        ));
    }
    let chunk_frames = chunk_frames.max(1);
    let total_frames = buf.len() / channels;
    let deadline = Instant::now() + block_timeout;
    let mut frame = 0usize;
    while frame < total_frames {
        let end = (frame + chunk_frames).min(total_frames);
        let chunk = &mut buf[frame * channels..end * channels];
        // The fence: the write returns only once the resident has taken
        // these frames, so a successful chunk write is a delivery.
        session
            .write_timeout(
                chunk,
                Some(deadline.saturating_duration_since(Instant::now())),
            )
            .map_err(|error| dsp_error_to_core(error, "write"))?;
        let mut filled = 0;
        while filled < chunk.len() {
            let frames = session
                .read(
                    &mut chunk[filled..],
                    Some(deadline.saturating_duration_since(Instant::now())),
                )
                .map_err(|error| dsp_error_to_core(error, "read"))?;
            if frames == 0 {
                return Err(CoreError::Dsp(
                    "resident DSP closed its output mid-stream".into(),
                ));
            }
            filled += frames * channels;
        }
        frame = end;
    }
    Ok(())
}

/// Shared control state of a resident DSP chain: the stage list a reconfigure
/// sends, the ring capacity, the bounded round-trip and the sticky poisoned
/// state.
struct ChainControl {
    stages: Vec<DspStageDesc>,
    ring_frames: u32,
    block_timeout: Duration,
    /// The first failure sticks; `Some` is the sticky first failure's text:
    /// every later `process` returns it as a `Dsp` error (RFC 0014 §4.3 —
    /// resuming on a failed chain is an error, recovery is a replacement).
    /// The decode thread discards a `prepare` error, so a refused reconfigure
    /// is held here and surfaces from the next `process`. Every latched
    /// failure is a `Dsp` error, so kind and text survive the round trip.
    poisoned: Option<String>,
}

impl ChainControl {
    fn new(stages: Vec<DspStageDesc>, ring_frames: u32, block_timeout: Duration) -> Self {
        ChainControl {
            stages,
            ring_frames,
            block_timeout,
            poisoned: None,
        }
    }

    /// Latch a failure: the first one sticks.
    fn latch(&mut self, error: &CoreError) {
        if self.poisoned.is_none() {
            self.poisoned = Some(error.to_string());
        }
    }

    /// The lockstep chunk size in frames.
    ///
    /// The smaller of the in-ring capacity the caller chose and the
    /// ceiling that keeps the resident's back-ring free
    /// ([`DSP_CHUNK_FRAMES`]); at least one frame, because a chunk of
    /// zero would never advance.
    fn chunk_frames(&self) -> usize {
        let ring = usize::try_from(self.ring_frames).unwrap_or(usize::MAX);
        ring.clamp(1, DSP_CHUNK_FRAMES)
    }

    /// `DspStage::prepare`: reconfigure the resident on a format change,
    /// clear its stage state otherwise. The decode thread calls `prepare` at
    /// every track start, and the `DspStage` contract clears stage state at a
    /// track change (the engine's own stages leave `prepare` with fresh
    /// state), so a same-format prepare resets the resident's chain too.
    /// That reset rides the asynchronous `DspReset`; see `reset` below for
    /// the seam race it carries until the mediahost fence lands (A26).
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
            self.latch(&error);
            return Err(CoreError::Dsp(err_msg));
        }
        Ok(())
    }

    /// `DspStage::process`: one bounded block round-trip, in lockstep
    /// chunks. A failure is returned as `Err` (RFC 0014 §3.5) and
    /// latched: the chain is poisoned and returns the sticky first
    /// failure from then on, without touching the session or the buffer
    /// again.
    fn process<F: PcmRings>(
        &mut self,
        session: &mut DspSession<F>,
        buf: &mut [f32],
    ) -> CoreResult<()> {
        if let Some(message) = &self.poisoned {
            return Err(CoreError::Dsp(message.clone()));
        }
        if let Err(error) = process_block(session, buf, self.block_timeout, self.chunk_frames()) {
            self.latch(&error);
            return Err(error);
        }
        Ok(())
    }

    /// `DspStage::reset` (seek): clear the resident's stage state; the rings
    /// are untouched, and the decode thread drops the chain's start-up frames
    /// (`dsp_trim`) right after, so the seek stays exact.
    ///
    /// The reset is asynchronous and unfenced: the session documents the send
    /// as neither an acknowledgement nor an ordering fence for PCM already
    /// queued, and the control path crosses three more threads (writer,
    /// reader, monitor) than the PCM path, so the resident can apply
    /// `DspReset` at any of its control polls — including after the first
    /// block of the new track was already processed. The trim does not cover
    /// that landing: it drops only the first `latency` frames of the new
    /// track, and a reset landing later clears a chain that then holds the
    /// new track's real frames — a hold-back stage loses them and emits
    /// `latency` frames of silence in their place, outside the trim window.
    /// That is injected silence, not a stale prefix, and it is exactly what
    /// fails the engine seam test
    /// (`engine_gapless_seam_through_the_resident_chain_is_exact`, A26). The
    /// seek path survives the race only because its drain delays the first
    /// post-seek block long enough for the control path to land; the gapless
    /// seam has no such slack. No wall-clock wait is added here: a fixed
    /// duration only narrows the race instead of closing it. The fix is a
    /// session-level acknowledgement on the `DspReset` path — a true delivery
    /// fence — owned by `rustamp-mediahost` (`DspSession::reset` at
    /// `crates/rustamp-mediahost/src/dsp.rs:577-580`, the pump's reset sites
    /// at `dsp.rs:128` and `dsp.rs:368`, the wire variant at
    /// `crates/rustamp-ipc/src/message.rs:240`), handed over with the exact
    /// sites and design in `docs/tasks/A26-gapless-real.md`.
    fn reset<F: PcmRings>(&mut self, session: &mut DspSession<F>) {
        if self.poisoned.is_some() {
            return;
        }
        if let Err(error) = session
            .reset()
            .map_err(|error| dsp_error_to_core(error, "reset"))
        {
            self.latch(&error);
        }
    }

    /// `DspStage::latency_frames`: the resident's `DspReady` report, which the
    /// decode thread sums into its trim/flush compensation exactly like an
    /// in-process stage's (RFC 0006 §6.2). Zero once poisoned: the stage
    /// never processes again, so no end-of-track flush may push silence
    /// through a dead chain.
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
    /// handshake; `stages`/`ring_frames` are what a later reconfigure sends
    /// and `block_timeout` bounds every block round-trip.
    pub fn new(
        session: DspSession<F>,
        stages: Vec<DspStageDesc>,
        ring_frames: u32,
        block_timeout: Duration,
    ) -> Self {
        HostDspChain {
            session,
            control: ChainControl::new(stages, ring_frames, block_timeout),
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

    fn process(&mut self, buf: &mut [f32]) -> CoreResult<()> {
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
    ) -> Self {
        ProcessDspChain {
            process: std::sync::Mutex::new(process),
            control: ChainControl::new(stages, ring_frames, block_timeout),
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

    fn process(&mut self, buf: &mut [f32]) -> CoreResult<()> {
        let Self { process, control } = self;
        let Ok(process) = process.get_mut() else {
            return Err(CoreError::Dsp("resident DSP session lock poisoned".into()));
        };
        control.process(process.session(), buf)
    }

    fn reset(&mut self) {
        let Self { process, control } = self;
        let Ok(process) = process.get_mut() else {
            control.latch(&CoreError::Dsp("resident DSP session lock poisoned".into()));
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
    /// reconfigures the resident.
    ///
    /// A refusal at configure (unknown stage id, undecodable params) is a
    /// loud `Dsp` error here, before anything is installed; a missing helper
    /// binary degrades to "stage unavailable" (`Unsupported`), per RFC 0006
    /// §8 — never a silent in-process fallback.
    pub fn open(&self, spec: AudioSpec) -> CoreResult<Box<dyn DspStage>> {
        #[cfg(not(windows))]
        {
            let _ = spec;
            return Err(CoreError::Unsupported(
                "resident DSP requires Windows named PCM mappings",
            ));
        }
        #[cfg(windows)]
        {
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
        assert_eq!(negotiated_spec(wire).unwrap(), AudioSpec::new(44_100, 2));
    }

    #[test]
    fn a_format_the_wire_cannot_carry_is_refused_loudly() {
        // The negotiation is one-sided: the host owns the format. A format
        // with no channels or no sample rate must not become an `AudioSpec`
        // with a zero rate (the clock would divide by it).
        for wire in [
            PcmSpecWire {
                sample_rate_hz: 44_100,
                channels: 0,
            },
            PcmSpecWire {
                sample_rate_hz: 0,
                channels: 2,
            },
        ] {
            let error = negotiated_spec(wire).expect_err("a broken format must be refused");
            assert!(
                matches!(&error, CoreError::Decode(message) if message.contains("handshake")),
                "{error:?}"
            );
        }
    }

    #[test]
    fn the_request_mapping_crosses_the_boundary_only_for_paths_and_urls() {
        // RFC 0006 §5.1: the host opens what it can reach itself.
        let path = MediaSource::Path(PathBuf::from("C:/Music/track.mp3"));
        assert_eq!(
            host_location(&path).unwrap(),
            MediaLocationWire::File("C:/Music/track.mp3".into())
        );
        let url = MediaSource::Url("https://example.invalid/stream.mp3".into());
        assert_eq!(
            host_location(&url).unwrap(),
            MediaLocationWire::Url("https://example.invalid/stream.mp3".into())
        );
        let reader = MediaSource::Reader {
            reader: Box::new(std::io::Cursor::new(vec![0u8; 8])),
            extension: Some("mp3".into()),
        };
        assert!(
            matches!(host_location(&reader), Err(CoreError::Unsupported(_))),
            "a reader cannot cross the process boundary"
        );
    }

    #[test]
    fn the_liveness_policy_is_the_engine_s_own() {
        // RFC 0006 §8 puts the supervision policy on the engine side, and the
        // default must be a stall escape rather than the framework's 5 s
        // wall-clock trip wire (the flake class M24 proved in this guest).
        assert_eq!(
            ResidentConfig::new(PathBuf::from("helper.exe")).liveness,
            DEFAULT_RESIDENT_LIVENESS
        );
        assert!(DEFAULT_RESIDENT_LIVENESS.timeout > Duration::from_secs(5));
        let tightened = Liveness {
            interval: Duration::from_millis(50),
            timeout: Duration::from_secs(2),
        };
        let config = ResidentConfig::new(PathBuf::from("helper.exe")).with_liveness(tightened);
        assert_eq!(config.liveness, tightened);
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

#[cfg(test)]
mod restart_tests {
    //! The restart policy (RFC 0006 §8) on its own: a scripted decoder and a
    //! scripted reopener, no media host, no process, no ring. The engine level
    //! proof — a real resident that dies mid-track — is in `tests/resident.rs`.

    use std::sync::{Arc, Mutex};

    use super::*;

    const SPEC: AudioSpec = AudioSpec::new(1000, 1);

    /// What the scripted residents did.
    #[derive(Default)]
    struct Log {
        /// How many residents were opened.
        opens: usize,
        /// The positions the residents were seeked to, in order.
        seeks: Vec<Duration>,
    }

    /// What a scripted resident does.
    #[derive(Clone, Copy)]
    enum Script {
        /// Serves `frames` frames and then fails.
        Die(u64),
        /// Serves `frames` frames and then fails, and cannot be seeked: a
        /// restart would replay the track from its start.
        DieUnseekable(u64),
        /// Changes the format once and then serves.
        Format,
        /// Serves forever.
        Live,
    }

    /// A decoder that follows a [`Script`].
    struct Scripted {
        log: Arc<Mutex<Log>>,
        script: Script,
        /// Frames served so far.
        served: u64,
        changed: bool,
    }

    impl Scripted {
        fn new(log: Arc<Mutex<Log>>, script: Script) -> Self {
            log.lock().expect("the log is not poisoned").opens += 1;
            Scripted {
                log,
                script,
                served: 0,
                changed: false,
            }
        }
    }

    impl Decoder for Scripted {
        fn spec(&self) -> AudioSpec {
            if self.changed {
                AudioSpec::new(2000, 1)
            } else {
                SPEC
            }
        }

        fn read(&mut self, out: &mut [f32]) -> CoreResult<usize> {
            match self.script {
                Script::Format if !self.changed => {
                    self.changed = true;
                    return Err(CoreError::FormatChanged {
                        from: SPEC,
                        to: AudioSpec::new(2000, 1),
                    });
                }
                Script::Die(until) | Script::DieUnseekable(until) if self.served >= until => {
                    return Err(CoreError::Decode("the scripted resident died".into()));
                }
                _ => {}
            }
            let left = match self.script {
                Script::Die(until) | Script::DieUnseekable(until) => {
                    (until - self.served).min(out.len() as u64) as usize
                }
                _ => out.len(),
            };
            for sample in out[..left].iter_mut() {
                *sample = self.served as f32;
                self.served += 1;
            }
            Ok(left)
        }

        fn seek(&mut self, to: Duration) -> CoreResult<Duration> {
            self.log
                .lock()
                .expect("the log is not poisoned")
                .seeks
                .push(to);
            self.served = RestartingDecoder::frames_at(SPEC, to);
            Ok(to)
        }

        fn duration(&self) -> Option<Duration> {
            Some(SPEC.duration_of(self.served))
        }

        fn is_seekable(&self) -> bool {
            !matches!(self.script, Script::DieUnseekable(_))
        }
    }

    /// A reopener that hands out scripted residents.
    struct Reopen {
        log: Arc<Mutex<Log>>,
        script: Script,
    }

    impl ResidentReopen for Reopen {
        fn reopen(&mut self) -> CoreResult<Box<dyn Decoder>> {
            Ok(Box::new(Scripted::new(self.log.clone(), self.script)))
        }
    }

    /// A decoder that fails at `until`, wrapped in the restart policy.
    fn scripted(
        log: &Arc<Mutex<Log>>,
        first: Script,
        again: Script,
        restarts: u32,
    ) -> RestartingDecoder {
        RestartingDecoder::new(
            Box::new(Scripted::new(log.clone(), first)),
            Box::new(Reopen {
                log: log.clone(),
                script: again,
            }),
            restarts,
        )
    }

    #[test]
    fn a_failed_resident_is_restarted_once_at_the_last_known_position() {
        // RFC 0006 §8: "retry once, then fail the track". The retry resumes
        // at the last position the engine had taken from the resident.
        let log = Arc::new(Mutex::new(Log::default()));
        let mut decoder = scripted(&log, Script::Die(100), Script::Live, 1);
        let mut buf = [0.0f32; 10];
        for _ in 0..10 {
            assert_eq!(decoder.read(&mut buf).expect("the resident serves"), 10);
        }
        // The next read hits the failure: the helper comes back and the track
        // continues where it stopped.
        assert_eq!(
            decoder
                .read(&mut buf)
                .expect("the restarted resident serves"),
            10
        );
        let log = log.lock().expect("the log is not poisoned");
        assert_eq!(log.opens, 2, "the helper must come back exactly once");
        assert_eq!(
            log.seeks,
            vec![Duration::from_millis(100)],
            "the retry must resume at the last known position"
        );
    }

    #[test]
    fn a_second_failure_fails_the_track_and_the_retry_never_loops() {
        let log = Arc::new(Mutex::new(Log::default()));
        let mut decoder = scripted(&log, Script::Die(100), Script::Die(0), 1);
        let mut buf = [0.0f32; 10];
        for _ in 0..10 {
            decoder.read(&mut buf).expect("the resident serves");
        }
        // The retry fails as well: the track ends loudly, and the policy does
        // not spend a second restart.
        let error = decoder
            .read(&mut buf)
            .expect_err("the second failure must end the track");
        assert!(
            matches!(&error, CoreError::Decode(message) if message.contains("resident restart")),
            "{error}"
        );
        let log = log.lock().expect("the log is not poisoned");
        assert_eq!(log.opens, 2, "the retry must not loop");
    }

    #[test]
    fn a_healthy_resident_is_never_restarted() {
        let log = Arc::new(Mutex::new(Log::default()));
        let mut decoder = scripted(&log, Script::Live, Script::Live, 1);
        let mut buf = [0.0f32; 10];
        for _ in 0..5 {
            assert_eq!(decoder.read(&mut buf).expect("the resident serves"), 10);
        }
        let log = log.lock().expect("the log is not poisoned");
        assert_eq!(log.opens, 1, "a healthy resident is never replaced");
        assert!(log.seeks.is_empty(), "a healthy resident is never seeked");
    }

    #[test]
    fn a_resident_that_cannot_seek_is_not_restarted_after_it_delivered_audio() {
        // A restart would replay the track from its start, so the track ends
        // instead: the policy never trades a repetition for a gap.
        let log = Arc::new(Mutex::new(Log::default()));
        let mut decoder = scripted(&log, Script::DieUnseekable(100), Script::Live, 1);
        let mut buf = [0.0f32; 10];
        for _ in 0..10 {
            decoder.read(&mut buf).expect("the resident serves");
        }
        let error = decoder
            .read(&mut buf)
            .expect_err("a failed resident must fail the track");
        assert!(
            matches!(&error, CoreError::Decode(message) if message.contains("died")),
            "{error}"
        );
        let log = log.lock().expect("the log is not poisoned");
        assert_eq!(log.opens, 1, "an unseekable resident is never restarted");
    }

    #[test]
    fn a_failure_before_the_first_frame_restarts_without_a_seek() {
        // Nothing was played yet, so there is no position to resume at.
        let log = Arc::new(Mutex::new(Log::default()));
        let mut decoder = scripted(&log, Script::Die(0), Script::Live, 1);
        let mut buf = [0.0f32; 10];
        assert_eq!(
            decoder
                .read(&mut buf)
                .expect("the restarted resident serves"),
            10
        );
        let log = log.lock().expect("the log is not poisoned");
        assert_eq!(log.opens, 2);
        assert!(log.seeks.is_empty(), "there is no position to resume at");
    }

    #[test]
    fn a_format_change_is_a_transition_and_never_restarts() {
        // RFC 0002 C3: the engine adopts the new format and reads on.
        let log = Arc::new(Mutex::new(Log::default()));
        let mut decoder = scripted(&log, Script::Format, Script::Live, 1);
        let mut buf = [0.0f32; 10];
        let error = decoder
            .read(&mut buf)
            .expect_err("the format change is reported to the engine");
        assert!(
            matches!(&error, CoreError::FormatChanged { .. }),
            "{error:?}"
        );
        let log = log.lock().expect("the log is not poisoned");
        assert_eq!(log.opens, 1, "a format change never restarts the helper");
    }

    #[test]
    fn a_restart_budget_of_zero_switches_the_restart_off() {
        let log = Arc::new(Mutex::new(Log::default()));
        let mut decoder = scripted(&log, Script::Die(100), Script::Live, 0);
        let mut buf = [0.0f32; 10];
        for _ in 0..10 {
            decoder.read(&mut buf).expect("the resident serves");
        }
        let error = decoder
            .read(&mut buf)
            .expect_err("without a budget the failure ends the track");
        assert!(
            matches!(&error, CoreError::Decode(message) if message.contains("died")),
            "{error}"
        );
        let log = log.lock().expect("the log is not poisoned");
        assert_eq!(log.opens, 1, "a budget of zero never restarts");
    }

    #[test]
    fn the_restart_budget_is_the_engine_s_own() {
        // RFC 0006 §8: the restart policy is engine policy, not contract.
        assert_eq!(DEFAULT_RESIDENT_RESTARTS, 1);
        assert_eq!(
            ResidentConfig::new(PathBuf::from("helper.exe")).restarts,
            DEFAULT_RESIDENT_RESTARTS
        );
        let config = ResidentConfig::new(PathBuf::from("helper.exe")).with_restarts(0);
        assert_eq!(config.restarts, 0);
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
    /// Stall escape for every wait this module waits on (the M10 rule): a
    /// bound so generous that only a real stall reaches it. The old 5 s value
    /// was a wall-clock bound a healthy-but-slow VM schedule trips — the
    /// flake class M24 proved in this guest. The one deliberate fast bound
    /// (the wedged-resident probe below) passes its own 20 ms, unchanged.
    const WAIT: Duration = Duration::from_secs(30);
    /// Liveness policy for the in-process control peers — the same
    /// wall-clock trip wire the `dsp_hosted_tests` module documents (the
    /// framework's 5 s default flakes under this VM's load; see M24).
    const LIVENESS: Liveness = Liveness {
        interval: Duration::from_millis(100),
        timeout: Duration::from_secs(30),
    };

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
        let client = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
        let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
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
            HostDecoder::new(session, read_timeout, seek_timeout)
                .expect("the handshake the session validated is adoptable"),
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
        let client = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
        let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
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
        // Kept at the original 5 s (not the widened WAIT): the 20 ms read
        // bound is the claim; this only proves the bound fired, not a hang.
        assert!(start.elapsed() < Duration::from_secs(5));
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
    //! conversion, loud error mapping and the opt-in config. No rings, no
    //! session, no process.

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
    fn the_lockstep_chunk_is_the_smaller_of_the_ring_and_the_ceiling() {
        // The chunk bounds the frames in flight, so it is the smaller of
        // the in-ring capacity the caller chose and the ceiling that
        // keeps the resident's back-ring free.
        let control =
            ChainControl::new(Vec::new(), DEFAULT_DSP_RING_FRAMES, Duration::from_secs(5));
        assert_eq!(control.chunk_frames(), DSP_CHUNK_FRAMES);
        let control = ChainControl::new(Vec::new(), 64, Duration::from_secs(5));
        assert_eq!(control.chunk_frames(), 64, "a small ring bounds the chunk");
        let control = ChainControl::new(Vec::new(), 0, Duration::from_secs(5));
        assert_eq!(
            control.chunk_frames(),
            1,
            "a chunk of zero would never advance"
        );
        // The ceiling is the resident worker's own block size, so the
        // chain sees the same block boundaries as before the fix.
        assert_eq!(DSP_CHUNK_FRAMES, 1024);
    }

    #[test]
    fn dsp_zero_ring_frames_is_rejected_loudly() {
        let config =
            DspResidentConfig::new(PathBuf::from("helper.exe"), Vec::new()).with_ring_frames(0);
        // On non-Windows this is Unsupported (no spawn platform); on Windows
        // it is InvalidArgument before any process exists. Either way no
        // helper is spawned for a zero capacity.
        match config.open(AudioSpec::new(44_100, 2)) {
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Instant;

    use rustamp_core::audio::{
        AudioSpec, Confidence, Decoder, DecoderFactory, DecoderRegistry, MediaSource, ProbeHint,
    };
    use rustamp_core::bus::{EventBus, EventReceiver};
    use rustamp_core::error::{Error as CoreError, ErrorKind, Result as CoreResult};
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
    /// Stall escape for every wait this module waits on (the M10 rule): a
    /// bound so generous that only a real stall reaches it. The old 5 s value
    /// was a wall-clock bound a healthy-but-slow VM schedule trips — the
    /// flake class M24 proved in this guest; as the adapter's per-block
    /// round-trip bound it could also turn a slow start into a mid-start
    /// failure error before the engine's poll published the track start (the
    /// erasure path of M25 §1.1, which then fails `wait_track`). The one
    /// deliberate fast bound (the wedge test's 20 ms) is passed explicitly,
    /// unchanged.
    const WAIT: Duration = Duration::from_secs(30);
    /// Liveness policy for the in-process control peers. The framework
    /// default (`Liveness::default()`: 100 ms interval, 5 s timeout) is a
    /// wall-clock bound a healthy-but-slow VM schedule can trip — the flake
    /// class M24 fixed in rustamp-dechost's `real_decode` suite. A dead
    /// resident never answers a Ping and is caught by the control EOF
    /// regardless, and a wedged resident by the adapter's own bounded block
    /// round-trip, so no test here depends on a fast liveness expiry; only
    /// the trip wire moves, from 5 s to the stall escape.
    const LIVENESS: Liveness = Liveness {
        interval: Duration::from_millis(100),
        timeout: WAIT,
    };

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
        /// Armed crash trigger for `Kind::Die`: the chain panics on the next
        /// block once set. The engine-level death test arms it after the
        /// track start is published, so the crash is deterministically
        /// mid-run instead of racing the start publication (a call-count
        /// crash can latch the DSP failure before the engine's poll has
        /// processed the decode thread's already-queued `TrackStarted`, and
        /// the failure teardown discards that message).
        die: Option<Arc<AtomicBool>>,
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
                die: None,
            }
        }

        /// Arm the crash trigger of a `Kind::Die` chain.
        fn armed_to_die(mut self, die: Arc<AtomicBool>) -> Self {
            self.die = Some(die);
            self
        }
    }

    impl ResidentChain for FakeChain {
        fn latency_frames(&self) -> u32 {
            self.latency as u32
        }

        fn process(&mut self, buf: &mut [f32]) -> HostResult<()> {
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
                    if self.calls == 3
                        || self
                            .die
                            .as_ref()
                            .is_some_and(|armed| armed.load(Ordering::Relaxed))
                    {
                        panic!("fake resident crash");
                    }
                    for sample in buf.iter_mut() {
                        *sample *= self.factor;
                    }
                }
            }
            Ok(())
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

        fn process(&mut self, buf: &mut [f32]) -> HostResult<()> {
            for stage in &mut self.stages {
                stage.process(buf)?;
            }
            Ok(())
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
        /// Crash trigger handed to every `fake:die` chain this resident
        /// builds; `None` keeps the call-count crash (third block).
        die: Option<Arc<AtomicBool>>,
    }

    impl FakeDspResident {
        fn new() -> Self {
            FakeDspResident {
                opens: AtomicUsize::new(0),
                die: None,
            }
        }

        /// A resident whose `fake:die` chains crash once `die` is set.
        fn armed(die: Arc<AtomicBool>) -> Self {
            FakeDspResident {
                opens: AtomicUsize::new(0),
                die: Some(die),
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
                    "fake:die" => match &self.die {
                        Some(die) => FakeChain::new(Kind::Die, 2.0, 0, channels)
                            .armed_to_die(Arc::clone(die)),
                        None => FakeChain::new(Kind::Die, 2.0, 0, channels),
                    },
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
        open_session_with(stages, spec, ring_frames, FakeDspResident::new())
    }

    /// The same in-process session with a chosen fake resident — the
    /// engine-level death test passes an armed one so its crash trigger is
    /// under the test's control (see `FakeChain::die`).
    fn open_session_with(
        stages: Vec<DspStageDesc>,
        spec: PcmSpecWire,
        ring_frames: u32,
        resident: FakeDspResident,
    ) -> (DspSession<NamedPcmRings>, HostOutcome) {
        let ((ar, aw), (br, bw)) = streams();
        let client = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
        let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
        let (done, result) = mpsc::channel();
        thread::spawn(move || {
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
    ) -> (HostDspChain<NamedPcmRings>, HostOutcome) {
        let (session, host) = open_session(stages.clone(), spec, ring_frames);
        let chain = HostDspChain::new(session, stages, ring_frames, block_timeout);
        (chain, host)
    }

    fn reap(host: HostOutcome) {
        let _ = host.recv_timeout(WAIT);
    }

    #[test]
    fn dsp_gain_round_trip_is_bit_exact_with_zero_latency() {
        let (mut chain, host) = open_chain(vec![gain_desc(2.0)], SPEC_WIRE, 8, WAIT);
        assert_eq!(chain.name(), "resident-dsp");
        assert_eq!(chain.latency_frames(), 0);
        let input: Vec<f32> = (0..7)
            .flat_map(|i| [i as f32 / 4.0, -(i as f32 / 4.0)])
            .collect();
        let mut buf = input.clone();
        chain.process(&mut buf).unwrap();
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
        let (mut chain, host) = open_chain(vec![delay_desc(LATENCY)], SPEC_WIRE, 8, WAIT);
        assert_eq!(chain.latency_frames(), LATENCY);

        // The block comes back delayed by the chain's latency: the first
        // LATENCY frames are the chain's initial state (zeros).
        let input: Vec<f32> = (0..10).flat_map(|i| [i as f32, -(i as f32)]).collect();
        let mut expected = vec![0.0; LATENCY as usize * 2];
        expected.extend_from_slice(&input[..(10 - LATENCY as usize) * 2]);
        let mut buf = input.clone();
        chain.process(&mut buf).unwrap();
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
            chain.process(&mut buf).unwrap();
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
        let (mut chain, host) = open_chain(vec![gain_desc(2.0)], SPEC_WIRE, 8, WAIT);
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
        chain.process(&mut buf).unwrap();
        assert_eq!(buf, [0.5, -1.0, 0.25]);
        drop(chain);
        reap(host);
    }

    #[test]
    fn dsp_prepare_with_the_same_format_clears_state_without_reconfigure() {
        const LATENCY: u32 = 2;
        let (mut chain, host) = open_chain(vec![delay_desc(LATENCY)], SPEC_WIRE, 8, WAIT);
        let input: Vec<f32> = (0..6).flat_map(|i| [i as f32, -(i as f32)]).collect();
        let mut buf = input.clone();
        chain.process(&mut buf).unwrap();
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
            chain.process(&mut buf).unwrap();
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
        let (mut chain, host) = open_chain(vec![stage("fake:reconfig-refuse")], SPEC_WIRE, 8, WAIT);
        let error = chain.prepare(AudioSpec::new(48_000, 1)).unwrap_err();
        assert!(matches!(error, CoreError::Dsp(_)), "{error}");
        assert!(
            error.to_string().contains("refuses a reconfigure"),
            "{error}"
        );
        // The refusal is latched: the decode thread discards a `prepare`
        // error, so the poisoned stage returns the sticky first failure
        // from its next `process` (RFC 0014 §4.3).
        assert_eq!(
            chain.latency_frames(),
            0,
            "a poisoned chain reports no latency"
        );
        let input = [0.5f32, -0.25];
        let mut buf = input;
        let process_error = chain.process(&mut buf).unwrap_err();
        assert!(
            matches!(process_error, CoreError::Dsp(_)),
            "{process_error}"
        );
        assert!(
            process_error.to_string().contains("refuses a reconfigure"),
            "{process_error}"
        );
        assert_eq!(buf, input, "a poisoned chain must not corrupt the block");
        // The sticky failure is returned again, without touching the session.
        assert!(matches!(
            chain.prepare(SPEC).unwrap_err(),
            CoreError::Dsp(_)
        ));
        assert!(matches!(
            chain.process(&mut buf).unwrap_err(),
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
        let (mut chain, host) = open_chain(vec![stage("fake:die")], SPEC_WIRE, 8, WAIT);
        // Two good blocks (the fake crashes on its third process call).
        let mut buf = [0.5f32, -0.25, 0.125, 0.0625];
        chain.process(&mut buf).unwrap();
        assert_eq!(buf, [1.0, -0.5, 0.25, 0.125]);
        let mut buf = [0.5f32, -0.25, 0.125, 0.0625];
        chain.process(&mut buf).unwrap();
        assert_eq!(buf, [1.0, -0.5, 0.25, 0.125]);
        // The third block round-trip dies with the host: `process` returns a
        // loud `Dsp` error and leaves the buffer untouched.
        let input = [0.5f32, -0.25, 0.125, 0.0625];
        let mut buf = input;
        let error = chain.process(&mut buf).unwrap_err();
        assert!(matches!(error, CoreError::Dsp(_)), "{error}");
        assert!(error.to_string().contains("resident DSP"), "{error}");
        assert_eq!(buf, input, "a dead resident must not corrupt the block");
        assert_eq!(chain.latency_frames(), 0);
        // The sticky failure is returned again, without touching the session.
        assert!(matches!(
            chain.process(&mut buf).unwrap_err(),
            CoreError::Dsp(_)
        ));
        // The host thread died with the crash (bounded reap via the channel).
        let outcome = host.recv_timeout(WAIT).expect("host must finish");
        assert!(outcome.is_err(), "the crash panic, got {outcome:?}");
        drop(chain);
    }

    #[test]
    fn dsp_wedge_is_a_bounded_wait_and_poisons_the_chain() {
        let (mut chain, host) = open_chain(
            vec![stage("fake:slow")],
            SPEC_WIRE,
            8,
            Duration::from_millis(20),
        );
        let input = [0.5f32, -0.25, 0.125, 0.0625];
        let mut buf = input;
        let start = Instant::now();
        let error = chain.process(&mut buf).unwrap_err();
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(1),
            "the block round-trip must be bounded, took {elapsed:?}"
        );
        assert!(matches!(error, CoreError::Dsp(_)), "{error}");
        assert!(error.to_string().contains("timed out"), "{error}");
        assert_eq!(buf, input, "a wedged resident must not corrupt the block");
        assert_eq!(chain.latency_frames(), 0);
        // The sticky failure is returned again, without touching the session.
        assert!(matches!(
            chain.process(&mut buf).unwrap_err(),
            CoreError::Dsp(_)
        ));
        drop(chain);
        let outcome = host.recv_timeout(WAIT).expect("host must finish");
        assert!(outcome.is_ok(), "{outcome:?}");
    }

    // -----------------------------------------------------------------------
    // The lockstep round-trip: no deadlock by construction
    // -----------------------------------------------------------------------

    /// The in-ring capacity the deadlock tests use: far smaller than the
    /// block, so the block has to wrap the ring many times over.
    const DEADLOCK_RING_FRAMES: u32 = 64;

    /// The block the deadlock tests push through the resident.
    ///
    /// Larger than the resident's whole absorption: its 4096-frame
    /// back-ring plus the block it keeps in hand while it writes back (at
    /// most the resident's own 1024-frame block). A single write of that
    /// block cannot be taken back by the resident before the player reads,
    /// so the un-chunked round trip stalls — while the lockstep round
    /// trip, which never has more than one chunk in flight, completes.
    const DEADLOCK_BLOCK_FRAMES: usize = 8192;

    /// The resident worker's back-ring capacity in frames
    /// (`rustamp-mediahost`'s own constant). The coupling is pinned here
    /// on purpose: the chunk ceiling [`DSP_CHUNK_FRAMES`] must stay
    /// below it, and this assertion is what notices when it does not.
    const BACK_RING_FRAMES: u64 = 4096;

    /// A stereo block of `frames` frames with a per-frame ramp.
    ///
    /// The values are exact in `f32` (a frame index below 2^24 divided
    /// by four), so a gain of 2.0 is exact too and the comparison can be
    /// a bit-for-bit one.
    fn block(frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|frame| [frame as f32 / 4.0, -(frame as f32 / 4.0)])
            .collect()
    }

    /// The pre-fix round-trip: one write of the whole block, then the
    /// read-back.
    ///
    /// This is the pattern the W14 review found in the production adapter
    /// (finding F1, claim C6-2). It is kept here as the contrast case:
    /// the fix's evidence is that this pattern stalls on exactly the
    /// rings the lockstep path survives.
    fn un_chunked_round_trip(
        session: &mut DspSession<NamedPcmRings>,
        buf: &mut [f32],
        bound: Duration,
    ) -> HostResult<()> {
        session.write_timeout(buf, Some(bound))?;
        let channels = usize::from(session.spec().channels.max(1));
        let mut filled = 0;
        while filled < buf.len() {
            let frames = session.read(&mut buf[filled..], Some(bound))?;
            if frames == 0 {
                return Err(MediaHostError::OperationFailed(
                    "the resident closed its output mid-stream".into(),
                ));
            }
            filled += frames * channels;
        }
        Ok(())
    }

    #[test]
    fn the_lockstep_round_trip_completes_on_a_ring_smaller_than_the_block() {
        // The ring is far smaller than the block: 64 frames of in-ring
        // against 8192 frames of block. The lockstep round trip splits
        // the block into chunks of one ring's worth, so the resident
        // never stops taking and the block comes back whole and exact.
        let (mut chain, host) =
            open_chain(vec![gain_desc(2.0)], SPEC_WIRE, DEADLOCK_RING_FRAMES, WAIT);
        let input = block(DEADLOCK_BLOCK_FRAMES);
        let mut buf = input.clone();
        chain
            .process(&mut buf)
            .expect("the lockstep round trip must complete");
        for (index, (got, want)) in buf.iter().zip(input.iter()).enumerate() {
            assert_eq!(*got, want * 2.0, "sample {index}");
        }
        assert_eq!(chain.session().frames_dropped(), 0);
        drop(chain);
        reap(host);
    }

    #[test]
    fn the_un_chunked_round_trip_stalls_where_the_lockstep_one_completes() {
        // The pre-fix pattern on the same rings: one write of the whole
        // block, no read until it is done. The resident fills its
        // back-ring, stops taking from the in-ring, and the write waits
        // for a resident that waits for the read: the deadlock.
        //
        // A deadlock cannot be observed directly — the proof is a bound
        // expiring without progress — so the test pins three things: the
        // round trip does not finish inside the bound, it fails on the
        // write (not on the read), and the ring counters show why.
        let (mut session, host) =
            open_session(vec![gain_desc(2.0)], SPEC_WIRE, DEADLOCK_RING_FRAMES);
        let mut buf = block(DEADLOCK_BLOCK_FRAMES);
        let bound = Duration::from_secs(1);
        let start = Instant::now();
        let error = un_chunked_round_trip(&mut session, &mut buf, bound)
            .expect_err("the un-chunked round trip must not complete");
        let elapsed = start.elapsed();
        assert!(
            matches!(error, MediaHostError::TimedOut),
            "the un-chunked round trip must stall on the write, got {error}"
        );
        assert!(
            elapsed >= bound,
            "the stall must last the whole bound, took {elapsed:?}"
        );
        // The mechanism, observed: the resident published everything its
        // back-ring holds and the player read none of it, so the
        // resident stopped taking and the write can never complete.
        let counters = session.back_ring_counters();
        assert_eq!(
            counters.available(),
            BACK_RING_FRAMES,
            "the resident's back-ring must be full: {counters:?}"
        );
        assert_eq!(counters.read, 0, "the player must not have read anything");
        drop(session);
        reap(host);
    }

    // -----------------------------------------------------------------------
    // Engine level: the adapter installed as the engine's DSP chain
    // -----------------------------------------------------------------------

    /// The sample value at `frame` for track `seed`: never zero, so silence
    /// in the output is always detectable.
    ///
    /// The range is `0.1..=1.1`, so the pattern reaches above full scale: the
    /// converter's finishing clamps it (see [`finished`]).
    fn pattern(seed: u64, frame: u64) -> f32 {
        let x = frame
            .wrapping_mul(2_654_435_761)
            .wrapping_add(seed.wrapping_mul(7919))
            % 2001;
        0.1 + x as f32 / 2000.0
    }

    /// The converter's finishing, the last level decision of the chain (see
    /// `crate::convert`): a non-finite sample becomes silence, a subnormal
    /// becomes zero, and a sample above full scale is clamped to `±1.0`.
    /// The device receives the *finished* samples, so the expectations
    /// below carry the same rule.
    fn finished(x: f32) -> f32 {
        if !x.is_finite() || x.is_subnormal() {
            0.0
        } else {
            x.clamp(-1.0, 1.0)
        }
    }

    /// The samples the device receives for the source pattern when the chain
    /// leaves them alone: the pattern, put through the finishing.
    fn expected(seed: u64, from: u64, n: u64) -> Vec<f32> {
        (0..n).map(|i| finished(pattern(seed, from + i))).collect()
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
            Self::with_resident(names, stages, block_timeout, FakeDspResident::new())
        }

        /// A harness whose in-process fake resident is the given one — the
        /// engine-level death test passes an armed one so the resident's
        /// crash is triggered by the test, not by a call count.
        fn with_resident(
            names: &[&str],
            stages: Vec<DspStageDesc>,
            block_timeout: Duration,
            resident: FakeDspResident,
        ) -> DspHarness {
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
            let (session, host) =
                open_session_with(stages.clone(), SPEC_WIRE, DEFAULT_DSP_RING_FRAMES, resident);
            let chain = HostDspChain::new(session, stages, DEFAULT_DSP_RING_FRAMES, block_timeout);
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

        /// Poll and pump until `cond` holds or `timeout` elapses. The wait
        /// measures the run's own condition with a wall-clock stall escape:
        /// a pump only advances the run while the ring holds audio, so a
        /// pump-count budget can expire on underruns (the start drain, the
        /// StartDrained handshake, a lagging decode thread) without the run
        /// having made progress — which is why this harness waits on the
        /// condition, not on a count (the proxy M24 replaced). The escape is
        /// generous - only a wedged run reaches it.
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
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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
        assert_eq!(left[2999], finished(pattern(7, 2999)));
        assert_eq!(left[3000], finished(pattern(2, 0)));
        assert!(!left.contains(&0.0), "the seam must not contain silence");

        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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
        // The crash is armed, not counted. The old call-count trigger (the
        // fake panicked on its third block) raced the start publication; the
        // armed trigger makes the crash deterministically mid-run — a
        // stronger claim, with every assertion unchanged.
        let die = Arc::new(AtomicBool::new(false));
        let mut h = DspHarness::with_resident(
            &["track7_44100.tst"],
            vec![stage("fake:die")],
            WAIT,
            FakeDspResident::armed(Arc::clone(&die)),
        );
        h.engine.play_index(0).unwrap();
        assert!(h.wait_track(0));
        die.store(true, Ordering::Relaxed);
        // The fake crashes on the next block: the adapter returns a loud
        // `Dsp` error from `process`, the decode thread reports it and ends
        // the track (RFC 0014 §3.5; RFC 0006 §8: the resident's death fails
        // the track, never the player). The wait pumps the clock so the
        // decode thread keeps flowing to the armed crash: without a consumer
        // the engine ring fills and the decode thread parks in its write
        // loop before the next block.
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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
        assert_eq!(error.kind, ErrorKind::Dsp, "{error:?}");
        assert!(h.engine.last_error().is_some());

        // Resuming on the failed chain is an error (RFC 0014 §4.3): the
        // poisoned stage returns its sticky failure from `process`, so the
        // next run fails loudly the same way instead of playing through.
        h.engine.play_index(0).unwrap();
        assert_eq!(h.engine.transport(), TransportState::Playing);
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
            "the poisoned chain must fail the next run loudly"
        );
        let events = h.drain_events();
        let error_count = events
            .iter()
            .filter(|event| matches!(event, Event::Error(_)))
            .count();
        assert_eq!(error_count, 1, "one error event per run, got {error_count}");
        assert!(h.engine.last_error().is_some());

        // Recovery is an explicit replacement (RFC 0014 §4.3): a fresh chain
        // with fresh rings plays the track to its end, with no error.
        let (fresh, fresh_host) = open_chain(
            vec![gain_desc(2.0)],
            SPEC_WIRE,
            DEFAULT_DSP_RING_FRAMES,
            WAIT,
        );
        h.engine.set_dsp_chain(vec![Box::new(fresh)]);
        h.engine.play_index(0).unwrap();
        assert_eq!(h.engine.transport(), TransportState::Playing);
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
            "the fresh chain must finish the track"
        );
        assert_eq!(h.engine.position_frames(), 44_100);
        let events = h.drain_events();
        assert!(
            !events.iter().any(|event| matches!(event, Event::Error(_))),
            "a healthy fresh chain must not publish an error: {events:?}"
        );
        h.finish();
        reap(fresh_host);
    }

    #[test]
    fn engine_publishes_the_start_when_the_resident_dies_right_after_it() {
        // The wart this pins (M25 §1.1, A25): the decode thread queues
        // `TrackStarted` — which carries the duration — before its first
        // block, and a resident death on that first block used to race the
        // start publication through the engine's failure teardown, which
        // discarded the queued message. With RFC 0014 the failure rides the
        // decode thread's own report queue instead: `TrackStarted` is queued
        // first, the error second, and the engine processes both in order —
        // the erasure window is gone, and the start survives the failure.
        //
        // The crash is armed before the second run starts, so the resident
        // dies on the second track's first block — immediately after its
        // start was queued. The first track is two blocks long, so the
        // fake's third-call crash cannot fire during the first run.
        let die = Arc::new(AtomicBool::new(false));
        let mut h = DspHarness::with_resident(
            &["track7_2048.tst", "track2_44100.tst"],
            vec![stage("fake:die")],
            WAIT,
            FakeDspResident::armed(Arc::clone(&die)),
        );
        // A first run publishes the first track's duration and ends before
        // the second track opens ("stop after current"): the stale duration
        // the erasure would leave behind is exactly this one.
        h.engine.toggle_stop_after_current().unwrap();
        h.engine.play_index(0).unwrap();
        assert!(h.wait_track(0), "the first track must start");
        assert_eq!(h.engine.duration_frames(), Some(2048));
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
            "the first run must finish after its track"
        );
        assert_eq!(
            h.engine.duration_frames(),
            Some(2048),
            "the second track must not have started"
        );

        // The second track starts; the resident dies on its first block.
        die.store(true, Ordering::Relaxed);
        h.engine.play_index(1).unwrap();
        assert_eq!(h.engine.current_index(), Some(1));
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
            "the run must stop when the resident dies on the first block"
        );
        // The start the decode thread queued before the failure was
        // published: the window shows the started track's own duration.
        assert_eq!(
            h.engine.duration_frames(),
            Some(44_100),
            "the reported duration must be the started track's, not the previous track's"
        );
        assert_eq!(h.engine.current_index(), Some(1));
        assert_eq!(
            h.engine.position_frames(),
            0,
            "the timeline must freeze at the failure (nothing was pumped)"
        );
        let events = h.drain_events();
        let error_count = events
            .iter()
            .filter(|event| matches!(event, Event::Error(_)))
            .count();
        assert_eq!(error_count, 1, "the error must be published once");
        let error = events
            .iter()
            .find_map(|event| match event {
                Event::Error(info) => Some(info),
                _ => None,
            })
            .expect("a resident death must publish an error event");
        assert!(error.message.contains("resident DSP"), "{error:?}");
        assert!(h.engine.last_error().is_some());
        // The window shows the started track: its own duration, on the
        // track-change event too.
        let started = events.iter().find_map(|event| match event {
            Event::TrackChanged {
                index: Some(1),
                duration_ms,
                ..
            } => Some(*duration_ms),
            _ => None,
        });
        assert_eq!(
            started,
            Some(Some(1000)),
            "the track change must carry the started track's duration"
        );

        // The player survives: it accepts the next run, which fails loudly
        // on the poisoned chain (RFC 0014 §4.3 — recovery is a fresh chain,
        // pinned in the death test above).
        h.engine.play_index(1).unwrap();
        assert_eq!(h.engine.transport(), TransportState::Playing);
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
            "the poisoned chain must fail the next run loudly"
        );
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
        // (20 ms) turns the wedge into a loud `Dsp` error from `process`,
        // the decode thread reports it and ends the track — the engine can
        // never hang on a wedged resident.
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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
        assert_eq!(error.kind, ErrorKind::Dsp, "{error:?}");
        h.finish();
    }
}
