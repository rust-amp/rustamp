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

use std::path::PathBuf;
use std::time::Duration;

use rustamp_core::audio::{AudioSpec, Confidence, Decoder, DecoderFactory, MediaSource, ProbeHint};
use rustamp_core::error::{Error as CoreError, Result as CoreResult};
use rustamp_core::state::StreamInfo;
use rustamp_mediahost::decoder::{DecoderSession, file_location};
use rustamp_mediahost::pcm::PcmRings;
use rustamp_mediahost::{MediaHostError, Result as HostResult};
use rustamp_proto::host::{MediaLocationWire, PcmSpecWire};

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
        match outcome {
            Err(MediaHostError::OperationFailed(message)) => {
                assert!(message.contains("refuses"), "{message}");
            }
            Err(error) => panic!("refusal must be OperationFailed, got {error}"),
            Ok(_) => panic!("refusal must be loud, never a session"),
        }
        result
            .recv_timeout(WAIT)
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
