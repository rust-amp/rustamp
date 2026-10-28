//! The decode thread: decoder -> DSP -> format converter -> ring.
//!
//! One thread per playback run. It owns the decoder, so all blocking I/O and
//! all allocation stays off the audio thread. It talks to the engine over two
//! channels (control in, reports out) plus the atomics of
//! [`crate::ring::RingSync`] for the seek handshake.
//!
//! # Where the decoder comes from
//!
//! A **path** is opened by the registry, which knows a source by its
//! extension and by its first bytes. A **URL** is opened by the HTTP decoder
//! (S19): the registry cannot open one, because a URL has no bytes to sniff
//! (see [`open_source`]).
//!
//! # What a stream changes
//!
//! An `http://` stream is not a file: it has no position to seek to, and its
//! length is often unknown. Both are reported, never papered over:
//!
//! * a seek on a stream is refused by the decoder
//!   ([`Error::Unsupported`]), and the engine publishes the refusal and
//!   leaves the position where it was;
//! * a stream whose container states no length leaves the length unknown
//!   (`None`), so the window shows no total time instead of a wrong one;
//! * a connection that dies in the middle of the body is a **truncated
//!   stream**: the decoder reports it as a decode failure, and the frames
//!   that did arrive are still played.
//!
//! What a stream is **doing** is observable too: the decode thread hands the
//! engine an observer of the transfer (S19 §2), and the engine publishes
//! what it sees — the phase of the exchange and the counters of the body.
//! The observer is handed over *before* the open, so an open that fails
//! still leaves the phase that says why.
//!
//! The decode order per block is decoder -> ReplayGain scale ->
//! DSP chain -> vis tap -> format converter -> ring (see
//! [`crate::replaygain`] for why the gain sits before every stage).
//!
//! Gapless: when a track ends the loop simply opens the next queue entry and
//! keeps writing, so the ring never runs dry at the seam and the timeline stays
//! continuous. Nothing is dropped and no silence is inserted. The one exception
//! is the "stop after current track" mode (RFC 0005 O8): when it is armed the
//! run ends at the end of the current track instead of opening the next entry.
//!
//! # Latency at the seams
//!
//! Two parts of the path hold audio back, and both are accounted for so that
//! every decoded frame reaches the ring exactly once, at the timeline position
//! its track implies:
//!
//! * **The DSP chain** (the limiter's look-ahead, or any stage that reports
//!   [`DspStage::latency_frames`]). After every `prepare` or `reset` the
//!   first `latency` frames the chain emits are the stages' initial state, not
//!   audio, and are dropped. At the end of a track, before a format change
//!   and after a decoder error, `latency` frames of silence are pushed through
//!   the chain so that the audio it still holds comes out. The contract resets
//!   stages at every track change, so this happens at every seam. The
//!   default chain (equalizer, limiter enabled) has the limiter's look-ahead
//!   as its latency; with the limiter disabled it is 0 and nothing is trimmed
//!   or pushed.
//! * **The format converter** (the resampler's look-ahead). When the next
//!   track has the same format, the converter is **not** flushed or reset at
//!   the seam: the two tracks are resampled as one stream
//!   ([`FormatConverter::continues`]). It is flushed at the end of the run, at
//!   a seam between two formats, before a mid-stream format change and before
//!   a run ends on an error. A track's `timeline_start` is where its first
//!   frame lands on the output timeline, which with a continued resampler is
//!   ahead of what has been written when the track opens. A rate pair with no
//!   resampler at all is an **error**, never silence: the converter flags it
//!   ([`FormatConverter::resample_unavailable`]), the worker reports it as an
//!   `Error::Unsupported` and ends the track instead of running it in silence.
//!
//! A non-finite sample (NaN, infinity) leaving the DSP chain is replaced by
//! silence before it reaches the vis tap, the converter or the ring. The
//! converter holds a final `±1.0` (0 dBFS) ceiling on every path (see
//! [`FormatConverter`]), so the ring never holds a sample above full scale
//! however the equalizer and the limiter are set.
//!
//! # DSP stage failures
//!
//! `DspStage::process` is fallible (RFC 0014): an `Err` is terminal for the
//! track. `emit` handles it exactly like a decoder error — report it, push the
//! chain's held-back audio out, end the track — and the first failure sticks:
//! one `Event::Error` per run, never a second one when a later stage fails as
//! well. The NaN sanitizer above stays as defense in depth on the Ok path; an
//! `Err` bypasses sample post-processing entirely.
//!
//! ## The engine's recovery policy: bypass
//!
//! RFC 0014 §4.3 poisons a failed chain: resuming on it is an error, and the
//! recovery is an explicit replacement with a fresh one. That is the
//! *contract*; what the engine does with a failed stage in the meantime is
//! engine policy, the way RFC 0006 §8 puts the restart policy on the engine
//! side. The engine's policy is **bypass**:
//!
//! * The failed stage is replaced by a pass-through ([`Bypass`]) at the
//!   moment it fails, so the audio keeps flowing and **the next track
//!   plays** — the player does not die with a poisoned chain (W14 finding
//!   F4). The bypass lasts until the caller installs a new chain
//!   ([`Engine::set_dsp_chain`](crate::engine::Engine::set_dsp_chain)); the
//!   engine hands the bypassed chain back to the caller with the run, so the
//!   bypass is visible in what the caller gets back and can be undone by
//!   installing the stage again.
//! * The failure is reported **once** per run, and the report names the
//!   stage, the cause and what was lost. The bypass is not silent: the
//!   `Event::Error` says that the stage is bypassed from now on.
//! * The track the failure happened in ends there — the stage's audio for
//!   that track is gone — but the queue keeps going.
//!
//! A *replacement* (a fresh resident, a fresh plugin) is the other possible
//! policy, and it is the app layer's to choose: only the caller knows how
//! the stage was built (which helper binary, which plugin), so only the
//! caller can build a new one. The engine owns the chain, not its
//! construction.
//!
//! ## What happens to the audio the failed stage held
//!
//! A stage that reports a latency holds that many frames back. When it
//! fails, those frames are gone with it: they live inside the stage (inside
//! the resident helper, for the resident chain), and nothing can get them
//! out. The engine does not hide that: the failure report names the frames
//! as lost.
//!
//! What *can* come out is the rest of the chain's hold-back: the stages that
//! are still alive keep their audio, and the end-of-track flush pushes it
//! out. That is why the failed stage is bypassed **before** the flush runs:
//! a dead stage in the chain would abort the flush and take the living
//! stages' held-back audio down with it (W14 finding F4).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rtrb::{Consumer, Producer, RingBuffer};
use rustamp_core::audio::{AudioSpec, Decoder, DecoderRegistry, DspStage, MediaSource};
use rustamp_core::error::{Error, ErrorKind, Result};
use rustamp_core::meta::TrackMetadata;
use rustamp_core::state::StreamInfo;
use rustamp_core::vis::VisFrame;
use rustamp_decoders::http_source::DEFAULT_TIMEOUT;
use rustamp_decoders::{HttpDecoder, HttpProgress};
use rustamp_playlist::PlaybackOrder;

use crate::convert::FormatConverter;
use crate::replaygain::ReplayGainControl;
use crate::ring::RingSync;
use crate::vistap::VisTap;

/// Engine-format frames decoded per iteration.
pub const READ_FRAMES: usize = 1024;

/// Sleep while paused or waiting for ring space.
const IDLE_SLEEP: Duration = Duration::from_millis(2);

/// A queue entry the engine can open again.
///
/// `MediaSource::Reader` cannot be re-opened (it owns its stream), so the
/// engine's queue only holds re-openable sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrackSource {
    /// A local file.
    Path(PathBuf),
    /// A network stream. It is opened by the HTTP decoder, which reads the
    /// body of an `http://` response (S19); the registry cannot open a URL,
    /// because it knows a source by its extension and by its first bytes and
    /// a URL has no bytes.
    Url(String),
}

impl TrackSource {
    /// A file path.
    pub fn path(path: impl Into<PathBuf>) -> Self {
        TrackSource::Path(path.into())
    }

    /// The [`MediaSource`] to hand to the registry.
    pub fn media_source(&self) -> MediaSource {
        match self {
            TrackSource::Path(p) => MediaSource::Path(p.clone()),
            TrackSource::Url(u) => MediaSource::Url(u.clone()),
        }
    }

    /// Text for logs, events and the window title.
    pub fn display(&self) -> String {
        match self {
            TrackSource::Path(p) => p.display().to_string(),
            TrackSource::Url(u) => u.clone(),
        }
    }

    /// Lower-case extension without the dot.
    pub fn extension(&self) -> Option<String> {
        self.media_source().extension()
    }
}

/// Opens the decoder for a queue entry.
///
/// A **path** goes through the registry, exactly as it always has: the
/// registry knows a source by its extension and by its first bytes, and a
/// file answers both.
///
/// A **URL** cannot go through the registry. The registry probes by extension
/// and sniffs bytes, and a URL has no bytes to sniff: a URL without an
/// extension is refused outright, and a URL with one reaches a factory that
/// opens files. The stream is opened here instead, by the decoder that reads
/// the body of an `http://` response (S19).
///
/// The registry's meaning is unchanged: a path still goes through it, and
/// only a URL takes the other way.
///
/// A URL is opened **with an observer**: the handle is handed to the engine
/// *before* the open, so an open that fails still leaves the phase that says
/// why (S19 §2). The engine shows what the transfer is doing; this crate
/// shows none of the texts (see `rustamp_decoders::http_progress`).
fn open_source(
    registry: &DecoderRegistry,
    source: &TrackSource,
    index: u32,
    tx: &Sender<DecodeOut>,
) -> Result<Box<dyn Decoder>> {
    match source {
        TrackSource::Path(_) => registry.open(source.media_source()),
        TrackSource::Url(url) => {
            let progress = HttpProgress::new();
            let _ = tx.send(DecodeOut::StreamStarted {
                index,
                progress: progress.clone(),
            });
            let decoder: Box<dyn Decoder> = Box::new(
                HttpDecoder::open_with_progress(url, DEFAULT_TIMEOUT, &progress)
                    .map_err(Error::from)?,
            );
            Ok(decoder)
        }
    }
}

/// Control messages, engine -> decode thread.
#[derive(Debug)]
pub(crate) enum DecodeMsg {
    /// Stop decoding but keep the thread alive.
    Pause,
    /// Continue after [`DecodeMsg::Pause`].
    Resume,
    /// End the playback run and exit the thread.
    Stop,
    /// End the playback run and exit the thread (engine shutting down).
    Quit,
    /// Seek to an absolute time in the current track.
    Seek {
        /// Target position inside the track.
        to: Duration,
        /// Target in device frames (the engine's bookkeeping).
        track_frames: u64,
        /// Engine-side sequence number, echoed in [`DecodeOut::Seeked`].
        seq: u32,
    },
    /// Abandon the current track and start the next one.
    Next,
    /// Abandon the current track and restart the previous one.
    Prev,
    /// Abandon the current track and jump `delta` entries (RFC 0005
    /// `Command::SkipBy`); the target is clamped to the queue.
    SkipBy(i32),
    /// Arm/disarm "stop after current track" (RFC 0005 O8): when armed, the run
    /// ends at the end of the current track instead of advancing.
    SetStopAfterCurrent(bool),
    /// The engine changed the playback modes (spec S15).
    ///
    /// The engine writes the modes into the shared model at once, so the
    /// published state is the current one. The message carries them as well,
    /// because the control channel is first-in, first-out: the modes a
    /// navigation command was issued under are the ones this thread saw
    /// before it, and the thread remembers them (see `Worker::in_issued_mode`).
    ModesChanged {
        /// Whether shuffle is on.
        shuffle: bool,
        /// Whether repeat is on.
        repeat: bool,
    },
}

/// Reports, decode thread -> engine.
pub(crate) enum DecodeOut {
    /// A track was opened and its first frame is about to be written.
    TrackStarted {
        /// Queue index.
        index: u32,
        /// Format the decoder produces.
        src: AudioSpec,
        /// Track length in device frames, if known.
        duration_frames: Option<u64>,
        /// Track length in ms, if known.
        duration_ms: Option<u64>,
        /// Decoder metadata.
        metadata: Box<TrackMetadata>,
        /// Technical info for the main window.
        stream: Box<StreamInfo>,
        /// Timeline position (device frames) of the first frame.
        timeline_start: u64,
        /// Whether the decoder supports seeking.
        seekable: bool,
    },
    /// The ring held a previous run's audio and the consumer has dropped it;
    /// the run's first frame is about to be written (see `crate::ring`).
    StartDrained {
        /// Ring epoch the drain belongs to.
        epoch: u64,
        /// Absolute timeline position of the run's first frame.
        timeline_pos: u64,
    },
    /// A seek has been applied and the ring drained.
    Seeked {
        /// Echo of the engine's sequence number.
        seq: u32,
        /// Ring epoch the drain belongs to.
        epoch: u64,
        /// New absolute timeline position in device frames.
        timeline_pos: u64,
    },
    /// The decoder refused the seek. No timeline, DSP, converter, ring or
    /// device discontinuity occurred; only the matching request is completed.
    SeekFailed {
        /// Echo of the engine's sequence number.
        seq: u32,
        /// Undo the optimistic stopped target even if a newer request is pending.
        initial: bool,
    },
    /// A track reached its end; the next one (if any) follows seamlessly.
    TrackEnded {
        /// Packets the decoder skipped (RFC 0002, C1).
        skipped: u64,
        /// Frames produced for this track, in device frames.
        frames: u64,
    },
    /// The stream changed format mid-stream and continues in the new one.
    FormatChanged {
        /// Format before the change.
        from: AudioSpec,
        /// Format after the change.
        to: AudioSpec,
        /// New technical info.
        stream: Box<StreamInfo>,
    },
    /// A stream transfer began: the observer of the transfer is handed to the
    /// engine, which shows what the transfer is doing (S19 §2).
    ///
    /// The handle is sent **before** the open, so an open that fails still
    /// leaves the phase that says why.
    StreamStarted {
        /// The queue index of the track the transfer belongs to, in the queue
        /// the run started with.
        index: u32,
        /// The observer of the transfer.
        progress: HttpProgress,
    },
    /// The whole queue is done; `timeline_end` is the last frame written.
    EndOfStream {
        /// Absolute timeline position after the last written frame.
        timeline_end: u64,
    },
    /// A previously reported EndOfStream is revoked (e.g. repeat toggled on).
    EndOfStreamRevoked,
    /// A decoder or output error.
    Error {
        /// Classification.
        kind: ErrorKind,
        /// Message.
        message: String,
    },
    /// The thread has exited.
    Stopped,
    /// The ring's write end and the user-installed DSP chain, handed back so
    /// the engine can start another run without losing installed stages.
    Returned {
        /// The ring's write end.
        producer: Producer<f32>,
        /// Installed DSP chain (with any per-run stages stripped).
        dsp: Vec<Box<dyn DspStage>>,
    },
}

impl std::fmt::Debug for DecodeOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeOut::TrackStarted {
                index,
                src,
                duration_frames,
                duration_ms,
                metadata,
                stream,
                timeline_start,
                seekable,
            } => f
                .debug_struct("TrackStarted")
                .field("index", index)
                .field("src", src)
                .field("duration_frames", duration_frames)
                .field("duration_ms", duration_ms)
                .field("metadata", metadata)
                .field("stream", stream)
                .field("timeline_start", timeline_start)
                .field("seekable", seekable)
                .finish(),
            DecodeOut::StartDrained {
                epoch,
                timeline_pos,
            } => f
                .debug_struct("StartDrained")
                .field("epoch", epoch)
                .field("timeline_pos", timeline_pos)
                .finish(),
            DecodeOut::Seeked {
                seq,
                epoch,
                timeline_pos,
            } => f
                .debug_struct("Seeked")
                .field("seq", seq)
                .field("epoch", epoch)
                .field("timeline_pos", timeline_pos)
                .finish(),
            DecodeOut::SeekFailed { seq, initial } => f
                .debug_struct("SeekFailed")
                .field("seq", seq)
                .field("initial", initial)
                .finish(),
            DecodeOut::TrackEnded { skipped, frames } => f
                .debug_struct("TrackEnded")
                .field("skipped", skipped)
                .field("frames", frames)
                .finish(),
            DecodeOut::FormatChanged { from, to, stream } => f
                .debug_struct("FormatChanged")
                .field("from", from)
                .field("to", to)
                .field("stream", stream)
                .finish(),
            DecodeOut::StreamStarted { .. } => write!(f, "StreamStarted"),
            DecodeOut::EndOfStream { timeline_end } => f
                .debug_struct("EndOfStream")
                .field("timeline_end", timeline_end)
                .finish(),
            DecodeOut::EndOfStreamRevoked => write!(f, "EndOfStreamRevoked"),
            DecodeOut::Error { kind, message } => f
                .debug_struct("Error")
                .field("kind", kind)
                .field("message", message)
                .finish(),
            DecodeOut::Stopped => write!(f, "Stopped"),
            DecodeOut::Returned { dsp, .. } => f
                .debug_struct("Returned")
                .field("dsp_stages_len", &dsp.len())
                .finish_non_exhaustive(),
        }
    }
}

/// Everything the decode thread needs.
pub(crate) struct Config {
    pub registry: DecoderRegistry,
    pub queue: Vec<TrackSource>,
    pub device: AudioSpec,
    pub sync: Arc<RingSync>,
    pub vis_enabled: bool,
    pub vis_capacity: usize,
    /// The engine's "stop after current track" mode at the moment the run
    /// started (RFC 0005 O8); later toggles arrive as
    /// [`DecodeMsg::SetStopAfterCurrent`].
    pub stop_after_current: bool,
    /// The user's ReplayGain settings, read once per track and once per
    /// seek (see [`crate::replaygain`]).
    pub replay_gain: Arc<ReplayGainControl>,
    /// Ring epoch the engine bumped at the start of the run because the ring
    /// still held a previous run's audio. The first write waits for its
    /// drain and the engine's clock re-base (see `crate::ring`).
    pub start_epoch: Option<u64>,
    /// The shared playback order (spec S15): the permutation and the repeat
    /// boundaries. The engine owns the model and the worker follows it, so
    /// both traverse the same order.
    pub order: Arc<Mutex<PlaybackOrder>>,
}

/// The decode thread of a run: its handle and the flag that says whether it
/// has actually finished.
///
/// The thread owns the ring's producer and the DSP chain and hands both back
/// through [`DecodeOut::Returned`], so it cannot simply be detached when it
/// does not finish: a join needs a budget and a way to tell "finished" from
/// "still stuck in a source read" (see [`Engine::join_decode`]).
pub(crate) struct DecodeThread {
    handle: JoinHandle<()>,
    /// Set by the thread's drop guard as its last act, so it is true once the
    /// thread has left `run` — after the producer and the chain were handed
    /// back — and also when it unwound through a panic.
    finished: Arc<AtomicBool>,
}

impl DecodeThread {
    /// Wait up to `budget` for the thread to finish. `true` when it did; then
    /// [`Self::join`] returns without blocking.
    pub(crate) fn wait_finished(&self, budget: Duration) -> bool {
        let deadline = Instant::now() + budget;
        loop {
            if self.finished.load(Ordering::Acquire) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Join the thread. Only call it after [`Self::wait_finished`] returned
    /// `true` — otherwise this blocks exactly as long as the thread does.
    pub(crate) fn join(self) {
        let _ = self.handle.join();
    }
}

/// Sets a [`DecodeThread`]'s completion flag when the decode run ends,
/// including on a panic, so a waiting join can never mistake a finished (or
/// unwound) thread for a stuck one.
struct FinishGuard(Arc<AtomicBool>);

impl Drop for FinishGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// Start the decode thread. Returns its handle and the vis consumer.
pub(crate) fn spawn(
    cfg: Config,
    dsp: Vec<Box<dyn DspStage>>,
    ring: Producer<f32>,
    start_index: u32,
    start_at: Option<(u64, u32)>,
    rx: Receiver<DecodeMsg>,
    tx: Sender<DecodeOut>,
) -> (DecodeThread, Consumer<VisFrame>) {
    let (vis, vis_rx) = VisTap::new(cfg.vis_capacity);
    let stop_after_current = cfg.stop_after_current;
    let start_epoch = cfg.start_epoch;
    let mut worker = Worker {
        cfg,
        ring,
        vis,
        tx,
        rx,
        conv: FormatConverter::new(AudioSpec::new(44100, 2), AudioSpec::new(44100, 2)),
        in_buf: Vec::new(),
        out_buf: Vec::new(),
        dsp,
        src: AudioSpec::new(44100, 2),
        timeline: 0,
        track_start: 0,
        stream_base: 0,
        dsp_trim: 0,
        paused: false,
        quitting: false,
        stopped: false,
        rg_gain: 1.0,
        stop_after_current,
        shuffle: false,
        repeat: false,
        current_index: start_index,
        pending_seek: None,
        pending_skip: None,
        drain_epoch: start_epoch,
        report_start: start_epoch.is_some(),
        stage_failed: false,
    };
    // The modes live in the shared model; the worker remembers them only to
    // notice a change.
    let (shuffle, repeat) = {
        let order = worker.order();
        (order.shuffle(), order.repeat())
    };
    worker.sync_modes(shuffle, repeat);
    let finished = Arc::new(AtomicBool::new(false));
    let thread_finished = Arc::clone(&finished);
    let handle = std::thread::Builder::new()
        .name("rustamp-decode".into())
        .spawn(move || {
            // Dropped last, so the flag turns true only after the producer and
            // the user's stages were sent back to the engine.
            let _finished = FinishGuard(thread_finished);
            worker.run(start_index, start_at);
            let producer = worker.take_ring();
            // Drop the per-run stages (Equalizer at index 0, Limiter at the end)
            // inserted by start_run so only the caller's installed DSP stages are
            // returned to the engine.
            let dsp = worker.take_user_dsp();
            let _ = worker.tx.send(DecodeOut::Stopped);
            let _ = worker.tx.send(DecodeOut::Returned { producer, dsp });
        })
        .expect("spawning the decode thread");
    (DecodeThread { handle, finished }, vis_rx)
}

/// What the per-track loop decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The decoder reached the end of the stream.
    Ended,
    /// The engine asked for the next track.
    SkipNext,
    /// The engine asked for the previous track.
    SkipPrev,
    /// The engine asked to move `delta` entries (`Command::SkipBy`).
    SkipBy(i32),
    /// Playback was stopped.
    Stop,
    /// The engine is shutting down.
    Quit,
}

/// What one block run through the DSP chain did.
enum Emit {
    /// The block was processed and written (or trimmed); the track continues.
    Continue,
    /// The write was cut short; the track must be abandoned.
    Abandoned,
    /// The track ends loudly: a DSP stage failed (RFC 0014 §3.5) or the
    /// converter has no resampler for the stream's rate pair. The error was
    /// already reported; a failing stage's chain was flushed in `emit`, and a
    /// stage that fails during that flush ends the track through this same arm
    /// without a second report.
    StageFailed,
}

/// The report for a rate pair the converter cannot resample.
///
/// The converter would emit 0 frames for it (see
/// [`FormatConverter::resample_unavailable`]), so the track cannot play at all
/// — the run reports that instead of letting silence run with the position.
fn resample_unavailable_error() -> Error {
    Error::Unsupported(
        "the track's sample rate cannot be converted to the output device's rate; \
         no resampler is available for this rate pair, so the track cannot be played",
    )
}

/// The pass-through a failed stage is replaced with.
///
/// The engine's recovery policy on a DSP failure is to bypass the failed
/// stage (see the module docs): the stage is dropped and this placeholder
/// takes its place, so the audio keeps flowing and the next track plays.
///
/// The placeholder keeps the chain's shape — the equalizer stays first, the
/// limiter last — and holds nothing back, so the latency bookkeeping (the
/// trim after a `prepare`, the flush at the end of a track) stays exact.
struct Bypass;

impl DspStage for Bypass {
    fn name(&self) -> &str {
        "bypass"
    }

    fn prepare(&mut self, _spec: AudioSpec) -> rustamp_core::error::Result<()> {
        Ok(())
    }

    fn process(&mut self, _buf: &mut [f32]) -> rustamp_core::error::Result<()> {
        Ok(())
    }

    fn reset(&mut self) {}

    fn latency_frames(&self) -> u32 {
        0
    }
}

struct Worker {
    cfg: Config,
    ring: Producer<f32>,
    vis: VisTap,
    tx: Sender<DecodeOut>,
    rx: Receiver<DecodeMsg>,
    conv: FormatConverter,
    /// Engine-format scratch buffer for one read.
    in_buf: Vec<f32>,
    /// Device-format scratch buffer for one write.
    out_buf: Vec<f32>,
    dsp: Vec<Box<dyn DspStage>>,
    /// Decoder format of the current track.
    src: AudioSpec,
    /// Absolute timeline position (device frames) of the next frame to write.
    timeline: u64,
    /// Timeline position where the current track started.
    track_start: u64,
    /// Timeline position of the converter's last reset: the converter's
    /// [`FormatConverter::stream_frames`] count from here.
    stream_base: u64,
    /// Frames the DSP chain still emits from its initial state after the last
    /// `prepare`/`reset`; they are dropped (see the module docs).
    dsp_trim: usize,
    paused: bool,
    quitting: bool,
    stopped: bool,
    /// The current track's ReplayGain multiplier (1.0 when the mode is
    /// disabled or the track carries no usable tag).
    ///
    /// Derived in one place, [`Worker::refresh_replaygain`]: a new track
    /// derives it in `prepare_track`, a seek derives it again, so a mode or
    /// preamp change takes effect at the next track **or seek** (the
    /// engine's contract, see [`crate::engine`]).
    rg_gain: f32,
    /// "Stop after current track" is armed (RFC 0005 O8): the run ends at the
    /// end of the current track instead of advancing.
    stop_after_current: bool,
    /// The shuffle mode as the worker last saw it. The model is the truth;
    /// this is what a change is noticed against.
    shuffle: bool,
    /// The repeat mode as the worker last saw it (see `shuffle`).
    repeat: bool,
    current_index: u32,
    pending_seek: Option<(Duration, u64, u32)>,
    pending_skip: Option<Outcome>,
    /// Ring epoch the consumer still has to drain before post-seek (or, at
    /// the start of a run, any) frames may be written.
    drain_epoch: Option<u64>,
    /// The pending drain is the run's start drain: once it is done, report
    /// [`DecodeOut::StartDrained`] and wait for the engine's re-base. A seek
    /// before the first write takes this over with its own report.
    report_start: bool,
    /// A DSP stage failure was already reported this run (RFC 0014 §4.2,
    /// first failure sticks): the worker is one run, so the flag is per run.
    stage_failed: bool,
}

impl Worker {
    fn run(&mut self, start_index: u32, start_at: Option<(u64, u32)>) {
        let mut index = start_index;
        let mut start_at = start_at;
        self.current_index = index;
        // The previous track ran to its end and the converter still holds its
        // tail: a next track in the same format continues that stream.
        let mut carry = false;
        loop {
            if self.quitting || self.stopped {
                break;
            }
            self.current_index = index;
            let Some(source) = self.cfg.queue.get(index as usize).cloned() else {
                self.flush_converter();
                let _ = self.tx.send(DecodeOut::EndOfStream {
                    timeline_end: self.timeline,
                });
                break;
            };
            let mut decoder = match open_source(&self.cfg.registry, &source, index, &self.tx) {
                Ok(d) => d,
                Err(e) => {
                    self.flush_converter();
                    self.report_error(&e);
                    break;
                }
            };
            let continues = carry && self.conv.continues(decoder.spec(), self.cfg.device);
            if carry && !continues {
                // A seam between two formats: the first stream ends here.
                self.flush_converter();
            }
            let prepared = self.prepare_track(index, decoder.as_ref(), continues);
            let start = start_at.take();
            let outcome = if prepared {
                if let Some((frames, seq)) = start {
                    let to = self.frames_to_duration(frames);
                    self.apply_seek(&mut decoder, to, seq, true);
                }
                self.track_loop(&mut decoder)
            } else {
                // The converter refused this track's rate pair; the error was
                // reported in `prepare_track`. Nothing of the track can reach
                // the device format, so it ends here (the same shape as a
                // failed DSP stage) instead of running silent.
                Outcome::Ended
            };
            // A track that never prepared its stream has nothing for the next
            // one to continue.
            carry = prepared && outcome == Outcome::Ended;
            if outcome == Outcome::Ended && self.stop_after_current {
                // "Stop after current track" (RFC 0005 O8): the run ends here
                // instead of opening the next entry. Reporting the end of the
                // stream is what lets the engine stop the transport and park the
                // device, exactly as it does at the end of the queue.
                self.flush_converter();
                let _ = self.tx.send(DecodeOut::EndOfStream {
                    timeline_end: self.timeline,
                });
                break;
            }

            // The traversal order (spec S15) decides what comes next: the
            // permutation and the repeat boundary live in the model the
            // engine shares with this thread.
            let next_target = match outcome {
                Outcome::Ended => self.advance_in_order(),
                Outcome::SkipNext => self.next_in_order(),
                Outcome::SkipPrev => self.prev_in_order(),
                Outcome::SkipBy(delta) => self.skip_in_order(delta),
                Outcome::Stop | Outcome::Quit => None,
            };

            let next_idx = match next_target {
                Some(idx) => idx,
                None => {
                    if matches!(outcome, Outcome::Ended | Outcome::SkipNext) {
                        self.flush_converter();
                        let _ = self.tx.send(DecodeOut::EndOfStream {
                            timeline_end: self.timeline,
                        });
                        match self.wait_at_eos() {
                            Some(idx) => {
                                carry = false;
                                idx
                            }
                            None => break,
                        }
                    } else {
                        break;
                    }
                }
            };
            index = next_idx;
        }
    }

    /// The shared playback order (spec S15).
    fn order(&self) -> MutexGuard<'_, PlaybackOrder> {
        self.cfg
            .order
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The next track in the active playback order, for the *automatic*
    /// advance at a track end (S15 §3.1).
    ///
    /// It takes the mode as it is now: a mode change takes effect at the
    /// next queue boundary check (S15 §4.3).
    fn advance_in_order(&mut self) -> Option<u32> {
        self.order().next_index().map(|index| index as u32)
    }

    /// The next track in the active playback order, for a manual `Next`
    /// (S15 §3.1).
    fn next_in_order(&mut self) -> Option<u32> {
        self.in_issued_mode(|order| order.next_index().map(|index| index as u32))
    }

    /// The previous track in the active playback order (S15 §3.2): in shuffle
    /// mode the track that was played immediately before the current one.
    fn prev_in_order(&mut self) -> Option<u32> {
        self.in_issued_mode(|order| order.prev_index().map(|index| index as u32))
    }

    /// The track `delta` entries away in the active playback order
    /// (S15 §3.3).
    fn skip_in_order(&mut self, delta: i32) -> Option<u32> {
        self.in_issued_mode(|order| order.skip_by(delta).map(|index| index as u32))
    }

    /// Evaluate a navigation command with the repeat mode it was issued
    /// under.
    ///
    /// The engine applies a mode change to the model at once, so that the
    /// published plan is the one the traversal follows. A navigation command
    /// that was queued before that change must not be wrapped by it, so the
    /// repeat mode of the command is the one this thread last saw in the
    /// command stream. The model keeps the published mode afterwards.
    fn in_issued_mode<T>(&mut self, navigate: impl FnOnce(&mut PlaybackOrder) -> T) -> T {
        let mut order = self.order();
        let published = order.repeat();
        order.set_repeat(self.repeat);
        let result = navigate(&mut order);
        order.set_repeat(published);
        result
    }

    /// Take over the modes the engine published, and report what changed.
    ///
    /// The modes live in the model; this thread remembers the ones it last
    /// saw in the command stream, so that a change can be noticed and a
    /// navigation command can be evaluated with the mode it was issued
    /// under.
    fn sync_modes(&mut self, shuffle: bool, repeat: bool) -> (bool, bool) {
        let changed = (shuffle != self.shuffle, repeat != self.repeat);
        self.shuffle = shuffle;
        self.repeat = repeat;
        changed
    }

    /// Keep the worker's own place in the permutation after the engine
    /// re-planned it around the track that is playing (S15 §4.1): the track
    /// this thread is decoding stays where it is in the new plan, so it is
    /// not abandoned half-way.
    fn realign_order(&mut self) {
        let mut order = self.order();
        order.set_current_index(Some(self.current_index as usize));
    }

    /// Hand the ring's write end back (used when the thread exits).
    fn take_ring(&mut self) -> Producer<f32> {
        std::mem::replace(&mut self.ring, RingBuffer::<f32>::new(1).0)
    }

    /// Hand the user-installed DSP stages back, removing the per-run Equalizer
    /// (inserted at index 0 by `Engine::start_run`) and Limiter (inserted at the
    /// end of the chain by `Engine::start_run`).
    fn take_user_dsp(&mut self) -> Vec<Box<dyn DspStage>> {
        let mut dsp = std::mem::take(&mut self.dsp);
        if !dsp.is_empty() {
            dsp.remove(0);
        }
        if !dsp.is_empty() {
            dsp.pop();
        }
        dsp
    }

    /// Configure the DSP chain, the converter and the tap for a new track.
    /// With `continues`, the converter keeps running from the previous track
    /// (a same-format gapless seam, see the module docs).
    ///
    /// Returns `false` when the converter has no resampler for this track's
    /// rate pair: the error was already reported and the track must end here.
    /// Resampling is not optional — without it the converter can only emit 0
    /// frames, and a track that "plays" as silence with the position running
    /// is the one failure a user cannot tell from a working player.
    fn prepare_track(&mut self, index: u32, decoder: &dyn Decoder, continues: bool) -> bool {
        let src = decoder.spec();
        self.src = src;
        // The track's ReplayGain multiplier, before every stage: the limiter
        // and the converter ceiling below see the normalized signal, so a hot
        // correction is limited and clamped like any other hot signal.
        let metadata = decoder.metadata();
        let tags = rustamp_meta::replaygain::ReplayGainTags::from_metadata(&metadata);
        self.cfg.replay_gain.set_track_values(
            tags.track_gain_db,
            tags.track_peak,
            tags.album_gain_db,
            tags.album_peak,
        );
        self.refresh_replaygain();
        for stage in &mut self.dsp {
            let _ = stage.prepare(src);
        }
        self.dsp_trim = self.dsp_latency();
        if !continues {
            self.conv.prepare(src, self.cfg.device);
            if self.conv.resample_unavailable() {
                self.report_error(&resample_unavailable_error());
                return false;
            }
            self.stream_base = self.timeline;
        }
        self.vis.set_enabled(self.cfg.vis_enabled);
        self.vis.prepare(src);
        self.in_buf = vec![0.0; src.samples_for(READ_FRAMES)];
        // The resampler holds a tail back that is emitted at the end of the
        // track, and that tail needs more room than one read block, so the
        // buffer covers both uses.
        let out_frames = self
            .conv
            .max_out_frames(READ_FRAMES)
            .max(self.conv.max_out_frames(2 * self.conv.latency_frames()));
        self.out_buf = vec![0.0; self.cfg.device.samples_for(out_frames)];
        // Where this track's first frame lands: with a continued converter the
        // previous track's tail is still inside it, so this is ahead of
        // `timeline`.
        self.track_start = self.stream_pos();
        let duration_frames = decoder
            .duration()
            .map(|d| (d.as_secs_f64() * f64::from(self.cfg.device.sample_rate)).round() as u64);
        let _ = self.tx.send(DecodeOut::TrackStarted {
            index,
            src,
            duration_frames,
            duration_ms: decoder.duration().map(|d| d.as_millis() as u64),
            metadata: Box::new(metadata),
            stream: Box::new(decoder.stream_info()),
            timeline_start: self.track_start,
            seekable: decoder.is_seekable(),
        });
        true
    }

    /// The track's ReplayGain multiplier for the mode the user set.
    ///
    /// The one place the multiplier is derived, so the two paths that can
    /// change it cannot drift apart: a new track derives it in
    /// [`Worker::prepare_track`], a seek derives it again in
    /// [`Worker::apply_seek`]. The control holds the track's tag values
    /// (snapshotted from the decoder's metadata in `prepare_track`) next to
    /// the mode and the preamp the user set, and the multiplier follows from
    /// them — so a mode that needs a tag the track does not carry selects
    /// nothing and the track plays unadjusted, exactly as the policy in
    /// [`crate::replaygain`] says.
    fn refresh_replaygain(&mut self) {
        self.rg_gain = self.cfg.replay_gain.effective_gain_current();
    }

    /// Frames the DSP chain holds back, at the current track's rate.
    fn dsp_latency(&self) -> usize {
        self.dsp
            .iter()
            .map(|stage| stage.latency_frames() as usize)
            .sum()
    }

    /// Timeline position the next input frame maps to, whether or not its
    /// output has left the converter yet.
    fn stream_pos(&self) -> u64 {
        self.stream_base + self.conv.stream_frames()
    }

    fn track_loop(&mut self, decoder: &mut Box<dyn Decoder>) -> Outcome {
        loop {
            if self.quitting {
                return Outcome::Quit;
            }
            if self.stopped {
                return Outcome::Stop;
            }
            if self.check_control() {
                return self.pending_skip.take().unwrap_or(Outcome::Stop);
            }
            // A skip that arrived while a seek drain was pending.
            if let Some(skip) = self.pending_skip.take() {
                return skip;
            }
            if let Some((to, _frames, seq)) = self.pending_seek.take() {
                self.apply_seek(decoder, to, seq, false);
                continue;
            }
            if self.paused {
                std::thread::sleep(IDLE_SLEEP);
                continue;
            }
            let src_ch = usize::from(self.src.channels);
            let want = READ_FRAMES * src_ch;
            let read = decoder.read(&mut self.in_buf[..want]);
            let frames = match read {
                Ok(0) => {
                    // The decoder is exhausted: emit what the DSP chain still
                    // holds. The converter's tail stays in it; the run loop
                    // either continues it with the next track or flushes it.
                    match self.flush_dsp() {
                        Emit::Continue => {}
                        Emit::Abandoned => return self.abandoned(),
                        // A stage failing during the end-of-track flush ends
                        // the track loudly (RFC 0014 §3.5).
                        Emit::StageFailed => return Outcome::Ended,
                    }
                    let _ = self.tx.send(DecodeOut::TrackEnded {
                        frames: self.stream_pos().saturating_sub(self.track_start),
                        skipped: decoder.skipped_packets(),
                    });
                    return Outcome::Ended;
                }
                Ok(n) => n,
                Err(Error::FormatChanged { from, to }) => {
                    match self.on_format_changed(decoder.as_ref(), from, to) {
                        Emit::Continue => {}
                        Emit::Abandoned => return self.abandoned(),
                        Emit::StageFailed => return Outcome::Ended,
                    }
                    continue;
                }
                Err(e) => {
                    self.report_error(&e);
                    // Keep the audio decoded before the error: the DSP
                    // chain's part now, the converter's in the run loop.
                    match self.flush_dsp() {
                        Emit::Continue => {}
                        Emit::Abandoned => return self.abandoned(),
                        Emit::StageFailed => return Outcome::Ended,
                    }
                    return Outcome::Ended;
                }
            };
            match self.emit(frames) {
                Emit::Continue => {}
                Emit::Abandoned => return self.abandoned(),
                // A stage error ends the track loudly (RFC 0014 §3.5): the
                // error was reported and the chain flushed in `emit`.
                Emit::StageFailed => return Outcome::Ended,
            }
        }
    }

    /// The outcome when a write was cut short: a pending skip, else a stop.
    fn abandoned(&mut self) -> Outcome {
        if self.quitting {
            return Outcome::Quit;
        }
        self.pending_skip.take().unwrap_or(Outcome::Stop)
    }

    /// Run the first `frames` frames of `in_buf` through the DSP chain, drop
    /// the chain's start-up frames, and convert and write the rest.
    fn emit(&mut self, frames: usize) -> Emit {
        let src_ch = usize::from(self.src.channels);
        let samples = frames * src_ch;
        // ReplayGain first: a per-track constant scale on the raw decoder
        // output, before the equalizer, every installed stage, the limiter
        // and the converter (see `crate::replaygain`). A gain of 1.0 is a
        // bit-exact no-op.
        let rg_gain = self.rg_gain;
        crate::replaygain::apply_gain(&mut self.in_buf[..samples], rg_gain);
        let mut index = 0;
        while index < self.dsp.len() {
            let outcome = self.dsp[index].process(&mut self.in_buf[..samples]);
            if let Err(error) = outcome {
                // RFC 0014 §3.5: a stage error ends the track. The engine's
                // own policy on top of that is to bypass the failed stage
                // (see the module docs): it is replaced by a pass-through,
                // so the audio keeps flowing and the next track plays
                // instead of the player dying with the poisoned chain.
                //
                // The stage is dropped here: a failed resident chain owns a
                // helper process that is dead or wedged, and it must not
                // outlive its failure.
                let failed = std::mem::replace(&mut self.dsp[index], Box::new(Bypass));
                let name = failed.name().to_owned();
                let held = failed.latency_frames();
                drop(failed);
                // The first failure sticks (§4.2): a stage that fails later
                // is bypassed too, but it is not reported a second time.
                if !self.stage_failed {
                    self.stage_failed = true;
                    self.report_stage_failure(&error, &name, held);
                }
                // The rest of the chain is still alive and may still hold
                // audio back: flush it, so that audio comes out. The failed
                // stage is out of the chain by now, so the flush is not
                // aborted by it — that is the whole point of bypassing it
                // first. What the failed stage itself held is lost with it,
                // and the report above says so.
                match self.flush_dsp() {
                    Emit::Continue => {}
                    Emit::Abandoned => return Emit::Abandoned,
                    Emit::StageFailed => {}
                }
                return Emit::StageFailed;
            }
            index += 1;
        }
        let skip = self.dsp_trim.min(frames);
        self.dsp_trim -= skip;
        if skip == frames {
            return Emit::Continue;
        }
        let block = &mut self.in_buf[skip * src_ch..samples];
        // Without the limiter nothing else stops a NaN or an infinity from a
        // decoder or a stage; on the resampled path one would also spread
        // over a whole FFT block. Finite samples are left untouched. This is
        // defense in depth on the Ok path only (RFC 0014 §4.5): an `Err`
        // bypasses sample post-processing entirely.
        for sample in block.iter_mut() {
            if !sample.is_finite() {
                *sample = 0.0;
            }
        }
        self.vis.push(block);
        let out_frames = self.conv.convert(block, &mut self.out_buf);
        let written = self.write_all(self.cfg.device.samples_for(out_frames));
        if written == out_frames {
            Emit::Continue
        } else {
            Emit::Abandoned
        }
    }

    /// Push `latency` frames of silence through the DSP chain so that the
    /// audio it holds back comes out (see the module docs).
    fn flush_dsp(&mut self) -> Emit {
        let src_ch = usize::from(self.src.channels);
        let mut left = self.dsp_latency();
        while left > 0 {
            let n = left.min(READ_FRAMES);
            self.in_buf[..n * src_ch].fill(0.0);
            match self.emit(n) {
                Emit::Continue => left -= n,
                other => return other,
            }
        }
        Emit::Continue
    }

    /// End the converter's stream: emit the tail the resampler is still
    /// holding into the ring and start the next stream at the current
    /// timeline position. Emits nothing when the rates are equal (the
    /// converter holds nothing back then).
    fn flush_converter(&mut self) {
        let out_frames = self.conv.flush(&mut self.out_buf);
        if out_frames > 0 {
            self.write_all(self.cfg.device.samples_for(out_frames));
        }
        self.stream_base = self.timeline;
    }

    /// Write up to `samples` device-format samples from `out_buf` into the
    /// ring, waiting for space and servicing control messages, and advance the
    /// timeline. Returns the number of **frames** written (always
    /// frame-aligned); fewer than requested when the track must be abandoned.
    fn write_all(&mut self, samples: usize) -> usize {
        let channels = usize::from(self.cfg.device.channels.max(1));
        let mut written = 0;
        // A pending drain must complete before the first post-seek frame is
        // written, or the consumer would drop it.
        if self.await_drain() || self.pending_skip.is_some() {
            return 0;
        }
        while written < samples {
            if self.check_control() || self.quitting || self.stopped {
                break;
            }
            if self.paused {
                std::thread::sleep(IDLE_SLEEP);
                continue;
            }
            let available = self.ring.slots();
            if available < channels {
                std::thread::sleep(IDLE_SLEEP);
                continue;
            }
            let want = ((samples - written).min(available) / channels) * channels;
            if want == 0 {
                std::thread::sleep(IDLE_SLEEP);
                continue;
            }
            match self.ring.write_chunk(want) {
                Ok(mut chunk) => {
                    let n = chunk.len();
                    {
                        let (first, second) = chunk.as_mut_slices();
                        let a = first.len();
                        first.copy_from_slice(&self.out_buf[written..written + a]);
                        second.copy_from_slice(&self.out_buf[written + a..written + n]);
                    }
                    // Publish before committing: the render callback must
                    // never read a frame that `written` does not count yet,
                    // or `consumed` would run ahead of it (see `crate::ring`).
                    self.timeline += (n / channels) as u64;
                    self.cfg.sync.set_written(self.timeline);
                    chunk.commit_all();
                    written += n;
                }
                Err(_) => std::thread::sleep(IDLE_SLEEP),
            }
        }
        written / channels
    }

    /// Service control messages. `true` when the current track must be
    /// abandoned; the reason is in `pending_skip` (or `stopped`).
    fn check_control(&mut self) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(DecodeMsg::Quit) => {
                    self.quitting = true;
                    return true;
                }
                Ok(DecodeMsg::Stop) => {
                    self.stopped = true;
                    return true;
                }
                Ok(DecodeMsg::Pause) => self.paused = true,
                Ok(DecodeMsg::Resume) => self.paused = false,
                Ok(DecodeMsg::Next) => {
                    self.pending_skip = Some(Outcome::SkipNext);
                    return true;
                }
                Ok(DecodeMsg::Prev) => {
                    self.pending_skip = Some(Outcome::SkipPrev);
                    return true;
                }
                Ok(DecodeMsg::SkipBy(delta)) => {
                    self.pending_skip = Some(Outcome::SkipBy(delta));
                    return true;
                }
                Ok(DecodeMsg::SetStopAfterCurrent(armed)) => self.stop_after_current = armed,
                Ok(DecodeMsg::ModesChanged { shuffle, repeat }) => {
                    let (shuffle_changed, _) = self.sync_modes(shuffle, repeat);
                    if shuffle_changed && shuffle {
                        // The engine re-planned the permutation around the
                        // track that is playing (S15 §4.1); this thread keeps
                        // its own place in it.
                        self.realign_order();
                    }
                }
                Ok(DecodeMsg::Seek {
                    to,
                    track_frames,
                    seq,
                }) => self.pending_seek = Some((to, track_frames, seq)),
                Err(std::sync::mpsc::TryRecvError::Empty) => return false,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.quitting = true;
                    return true;
                }
            }
        }
    }

    /// Wait at end-of-stream until repeat is enabled, navigation is requested,
    /// or playback is stopped/terminated.
    fn wait_at_eos(&mut self) -> Option<u32> {
        loop {
            if self.quitting || self.stopped {
                return None;
            }
            match self.rx.recv() {
                Ok(DecodeMsg::Quit) => {
                    self.quitting = true;
                    return None;
                }
                Ok(DecodeMsg::Stop) => {
                    self.stopped = true;
                    return None;
                }
                Ok(DecodeMsg::Pause) => {
                    self.paused = true;
                }
                Ok(DecodeMsg::Resume) => {
                    self.paused = false;
                }
                Ok(DecodeMsg::ModesChanged { shuffle, repeat }) => {
                    // The shuffle change needs no action here: the engine
                    // re-planned the permutation around the track that is
                    // playing (S15 §4.1) and this thread keeps the place it
                    // had in it.
                    let (_, repeat_changed) = self.sync_modes(shuffle, repeat);
                    if repeat_changed && repeat && !self.cfg.queue.is_empty() {
                        // Repeat was turned on at the end of the queue: the
                        // traversal continues with the next cycle (S15 §2).
                        let _ = self.tx.send(DecodeOut::EndOfStreamRevoked);
                        if let Some(next) = self.advance_in_order() {
                            return Some(next);
                        }
                    }
                }
                Ok(DecodeMsg::Next) => {
                    if self.repeat && !self.cfg.queue.is_empty() {
                        let _ = self.tx.send(DecodeOut::EndOfStreamRevoked);
                        if let Some(next) = self.next_in_order() {
                            return Some(next);
                        }
                    }
                }
                Ok(DecodeMsg::Prev) => {
                    if !self.cfg.queue.is_empty() {
                        let _ = self.tx.send(DecodeOut::EndOfStreamRevoked);
                        if let Some(prev) = self.prev_in_order() {
                            return Some(prev);
                        }
                    }
                }
                Ok(DecodeMsg::SkipBy(delta)) => {
                    if delta != 0 && !self.cfg.queue.is_empty() && self.repeat {
                        let _ = self.tx.send(DecodeOut::EndOfStreamRevoked);
                        if let Some(target) = self.skip_in_order(delta) {
                            return Some(target);
                        }
                    }
                }
                Ok(DecodeMsg::SetStopAfterCurrent(armed)) => {
                    self.stop_after_current = armed;
                }
                Ok(DecodeMsg::Seek { .. }) => {}
                Err(_) => {
                    return None;
                }
            }
        }
    }

    /// Seek, drain the ring, report the new timeline position and wait for the
    /// engine to re-base its clock (see [`crate::ring`]).
    ///
    /// The order matters: the consumer must have dropped the pre-seek frames
    /// *before* the engine re-bases the clock and *before* the first post-seek
    /// frame is written, so no frame is lost and the position lands exactly on
    /// the target.
    ///
    /// A successful seek also re-derives the track's ReplayGain multiplier
    /// (see [`Worker::refresh_replaygain`]): the mode and the preamp may have
    /// changed since the track was prepared, and a change takes effect at the
    /// next track **or seek**. A refused seek changes nothing — the decoder
    /// did not move, so the audio keeps the gain it had.
    fn apply_seek(
        &mut self,
        decoder: &mut Box<dyn Decoder>,
        to: Duration,
        seq: u32,
        initial: bool,
    ) {
        match decoder.seek(to) {
            Ok(reached) => {
                // A seek re-derives the ReplayGain multiplier: the mode or the
                // preamp may have changed since the track was prepared, and
                // the engine promises that a change takes effect at the next
                // track *or seek*. The new multiplier applies to the first
                // block after the seek — the seek is a block boundary — so no
                // block ever carries two multipliers.
                //
                // The transition cannot click: the seek drops the frames the
                // ring still holds and the engine flushes the device, so the
                // audio is discontinuous across it either way. The multiplier
                // starts at the discontinuity, with no waveform to ramp
                // against (see `crate::replaygain`).
                self.refresh_replaygain();
                for stage in &mut self.dsp {
                    stage.reset();
                }
                self.dsp_trim = self.dsp_latency();
                self.conv.reset();
                self.vis.reset();
                let frames =
                    (reached.as_secs_f64() * f64::from(self.cfg.device.sample_rate)).round() as u64;
                self.timeline = self.track_start + frames;
                self.stream_base = self.timeline;
                // `written` first, the epoch second (A6 finding 2): the drain
                // aligns `consumed` with the post-seek `written`.
                let epoch = self.cfg.sync.begin_seek(self.timeline);
                self.drain_epoch = Some(epoch);
                // This drain also covers a pending start drain (the consumer
                // acks the newest epoch), and `Seeked` re-bases the clock, so
                // the start report is not needed any more.
                self.report_start = false;
                if self.await_drain() {
                    return;
                }
                let _ = self.tx.send(DecodeOut::Seeked {
                    seq,
                    epoch,
                    timeline_pos: self.timeline,
                });
                if self.cfg.sync.is_active() {
                    let _ = self.cfg.sync.wait_for_engine(epoch);
                }
            }
            Err(e) => {
                // Only Ok above authorizes a discontinuity. In particular the
                // producer's write cursor is NOT the audible position: rebasing
                // to it on a refusal would skip the still-playable backlog.
                let _ = self.tx.send(DecodeOut::SeekFailed { seq, initial });
                self.report_error(&e);
            }
        }
    }

    /// Wait until the render side has dropped the pre-seek frames. Returns
    /// `true` when the run is stopping or quitting instead.
    ///
    /// Control messages are serviced while waiting, so `Stop`, `Quit` and
    /// `Pause` always work. A skip is remembered in `pending_skip` and acted on
    /// once the drain is done. The wait does **not** end while the output is
    /// paused or not yet started: every frame written before the drain would
    /// be dropped by it, and a dropped frame that `written` already counts
    /// would leave `consumed` behind for good (A6 finding 2, second variant).
    /// The first callback after the resume drains and the decoder carries on.
    ///
    /// For the start drain of a run (`report_start`), the run's start position
    /// is reported once the drain is done and the engine's re-base awaited, in
    /// the same order as steps 2 and 3 of a seek (see [`crate::ring`]).
    fn await_drain(&mut self) -> bool {
        let Some(epoch) = self.drain_epoch else {
            return false;
        };
        loop {
            if self.cfg.sync.drained(epoch) {
                self.drain_epoch = None;
                if std::mem::take(&mut self.report_start) {
                    let _ = self.tx.send(DecodeOut::StartDrained {
                        epoch,
                        timeline_pos: self.timeline,
                    });
                    if self.cfg.sync.is_active() {
                        let _ = self.cfg.sync.wait_for_engine(epoch);
                    }
                }
                return false;
            }
            if self.check_control() && (self.quitting || self.stopped) {
                self.drain_epoch = None;
                return true;
            }
            std::thread::sleep(IDLE_SLEEP);
        }
    }

    /// Switch to a new decoder format mid-stream. What the old format still
    /// holds (the DSP chain's delay, then the converter's tail) is emitted
    /// first.
    fn on_format_changed(&mut self, decoder: &dyn Decoder, from: AudioSpec, to: AudioSpec) -> Emit {
        match self.flush_dsp() {
            Emit::Continue => {}
            other => return other,
        }
        self.flush_converter();
        if self.quitting || self.stopped || self.pending_skip.is_some() {
            return Emit::Abandoned;
        }
        self.src = to;
        self.conv.prepare(to, self.cfg.device);
        if self.conv.resample_unavailable() {
            // The new format needs a resampler that does not exist for this
            // rate pair: the rest of the track would be silence. Report it and
            // end the track loudly (the stream ended, nothing can follow it).
            self.report_error(&resample_unavailable_error());
            return Emit::StageFailed;
        }
        for stage in &mut self.dsp {
            let _ = stage.prepare(to);
        }
        self.dsp_trim = self.dsp_latency();
        self.vis.prepare(to);
        self.in_buf = vec![0.0; to.samples_for(READ_FRAMES)];
        // Same sizing as `prepare_track`: room for the resampler's tail too.
        let out_frames = self
            .conv
            .max_out_frames(READ_FRAMES)
            .max(self.conv.max_out_frames(2 * self.conv.latency_frames()));
        self.out_buf = vec![0.0; self.cfg.device.samples_for(out_frames)];
        let _ = self.tx.send(DecodeOut::FormatChanged {
            from,
            to,
            stream: Box::new(decoder.stream_info()),
        });
        Emit::Continue
    }

    fn frames_to_duration(&self, frames: u64) -> Duration {
        self.cfg.device.duration_of(frames)
    }

    fn report_error(&self, e: &Error) {
        let info = rustamp_core::error::ErrorInfo::from(e);
        let _ = self.tx.send(DecodeOut::Error {
            kind: info.kind,
            message: info.message,
        });
    }

    /// Report a stage failure: the stage's name, the cause, what is lost and
    /// that the stage is bypassed from now on.
    ///
    /// The report is the only place the user learns what the failure cost,
    /// so it names all three: the stage that failed (the chain or the
    /// plugin behind it), the frames that are gone, and the bypass that
    /// follows. `held` is the failed stage's own hold-back: the frames it
    /// still held are lost with it, because they live inside it.
    fn report_stage_failure(&self, error: &Error, name: &str, held: u32) {
        // The cause without the `dsp error: ` prefix the report adds itself.
        let cause = match error {
            Error::Dsp(message) => message.clone(),
            other => other.to_string(),
        };
        let lost = if held > 0 {
            format!(
                "the block it was processing and the audio it held back (up to {held} frames) are lost"
            )
        } else {
            "the block it was processing is lost".to_owned()
        };
        self.report_error(&Error::Dsp(format!(
            "stage '{name}' failed: {cause}; the track ends here and the stage is bypassed from now on; {lost}"
        )));
    }
}
