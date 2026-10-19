//! The decode thread: decoder -> DSP -> format converter -> ring.
//!
//! One thread per playback run. It owns the decoder, so all blocking I/O and
//! all allocation stays off the audio thread. It talks to the engine over two
//! channels (control in, reports out) plus the atomics of
//! [`crate::ring::RingSync`] for the seek handshake.
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
//!   ahead of what has been written when the track opens.
//!
//! A non-finite sample (NaN, infinity) leaving the DSP chain is replaced by
//! silence before it reaches the vis tap, the converter or the ring. On the
//! resampled path the converter additionally holds a final `±1.0` (0 dBFS)
//! ceiling (see [`FormatConverter`]), so the ring never holds a sample above
//! full scale however the equalizer and the limiter are set.
//!
//! # DSP stage failures
//!
//! `DspStage::process` is fallible (RFC 0014): an `Err` is terminal for the
//! run. `emit` handles it exactly like a decoder error — report it, push the
//! chain's held-back audio out, end the track — and the first failure sticks:
//! one `Event::Error` per run, never a second one when the flush re-fails.
//! The NaN sanitizer above stays as defense in depth on the Ok path; an `Err`
//! bypasses sample post-processing entirely.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use rtrb::{Consumer, Producer, RingBuffer};
use rustamp_core::audio::{AudioSpec, Decoder, DecoderRegistry, DspStage, MediaSource};
use rustamp_core::error::{Error, ErrorKind};
use rustamp_core::meta::TrackMetadata;
use rustamp_core::state::StreamInfo;
use rustamp_core::vis::VisFrame;

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
    /// A network stream (opening it is the decoder's business; URL support
    /// itself is an M4 item).
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
    /// Set shuffle mode enabled/disabled, carrying the engine's active
    /// shuffle plan so live traversal follows it (spec S15).
    SetShuffle {
        /// Whether shuffle is on.
        on: bool,
        /// The engine's active plan when turning on; empty when turning off.
        plan: Vec<usize>,
        /// The engine's previous cycle plan when turning on; None when turning off.
        prev_plan: Option<Vec<usize>>,
    },
    /// Set repeat mode enabled/disabled (spec S15).
    SetRepeat(bool),
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
    /// The whole queue is done; `timeline_end` is the last frame written.
    EndOfStream {
        /// Absolute timeline position after the last written frame.
        timeline_end: u64,
    },
    /// A previously reported EndOfStream is revoked (e.g. repeat toggled on).
    EndOfStreamRevoked,
    /// The worker (re)planned the shuffle permutation on its own — a cycle
    /// boundary, or a fallback when the engine handed no usable plan. The
    /// engine adopts it so its published plan stays the traversed one (S15).
    ShufflePlanned {
        /// The new active plan.
        plan: Vec<usize>,
        /// The previous cycle's plan, if any.
        prev_plan: Option<Vec<usize>>,
    },
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
            DecodeOut::EndOfStream { timeline_end } => f
                .debug_struct("EndOfStream")
                .field("timeline_end", timeline_end)
                .finish(),
            DecodeOut::EndOfStreamRevoked => write!(f, "EndOfStreamRevoked"),
            DecodeOut::ShufflePlanned { plan, prev_plan } => f
                .debug_struct("ShufflePlanned")
                .field("len", &plan.len())
                .field("has_prev", &prev_plan.is_some())
                .finish(),
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
    /// The user's ReplayGain settings, snapshotted once per track (see
    /// [`crate::replaygain`]).
    pub replay_gain: Arc<ReplayGainControl>,
    /// Ring epoch the engine bumped at the start of the run because the ring
    /// still held a previous run's audio. The first write waits for its
    /// drain and the engine's clock re-base (see `crate::ring`).
    pub start_epoch: Option<u64>,
    /// Whether shuffle mode is active when the run starts (spec S15).
    pub shuffle: bool,
    /// Whether repeat mode is active when the run starts (spec S15).
    pub repeat: bool,
    /// Seed for the shuffle PRNG (spec S15).
    pub shuffle_seed: Option<u64>,
    /// The engine's active shuffle plan (spec S15). The engine owns the
    /// plan; the worker traverses it, so live playback follows the
    /// permutation the engine publishes through `Engine::shuffle_plan`.
    pub shuffle_plan: Vec<usize>,
    /// The previous cycle's shuffle plan, if any (spec S15 §3.2).
    pub prev_shuffle_plan: Option<Vec<usize>>,
}

/// Start the decode thread. Returns its handle and the vis consumer.
pub(crate) fn spawn(
    mut cfg: Config,
    dsp: Vec<Box<dyn DspStage>>,
    ring: Producer<f32>,
    start_index: u32,
    start_at: Option<(u64, u32)>,
    rx: Receiver<DecodeMsg>,
    tx: Sender<DecodeOut>,
) -> (JoinHandle<()>, Consumer<VisFrame>) {
    let (vis, vis_rx) = VisTap::new(cfg.vis_capacity);
    let stop_after_current = cfg.stop_after_current;
    let start_epoch = cfg.start_epoch;
    let shuffle = cfg.shuffle;
    let repeat = cfg.repeat;
    let rng_state = cfg.shuffle_seed.unwrap_or(0x9E37_79B9_7F4A_7C15);
    let shuffle_plan = std::mem::take(&mut cfg.shuffle_plan);
    let prev_shuffle_plan = cfg.prev_shuffle_plan.take();
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
        shuffle,
        repeat,
        shuffle_plan,
        shuffle_pos: 0,
        prev_shuffle_plan,
        rng_state,
        current_index: start_index,
        pending_seek: None,
        pending_skip: None,
        drain_epoch: start_epoch,
        report_start: start_epoch.is_some(),
        stage_failed: false,
        cycle_boundaries: Vec::new(),
    };
    let handle = std::thread::Builder::new()
        .name("rustamp-decode".into())
        .spawn(move || {
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
    (handle, vis_rx)
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

/// The queue index `delta` entries away from `index`, clamped to `0..len`.
///
/// `len` is never zero here: the run loop has already returned when the queue is
/// empty.
pub(crate) fn clamped_index(index: u32, delta: i32, len: usize) -> u32 {
    let last = len.saturating_sub(1) as i64;
    (i64::from(index) + i64::from(delta)).clamp(0, last) as u32
}

/// What one block run through the DSP chain did.
enum Emit {
    /// The block was processed and written (or trimmed); the track continues.
    Continue,
    /// The write was cut short; the track must be abandoned.
    Abandoned,
    /// A DSP stage failed (RFC 0014 §3.5): the track ends loudly. The first
    /// failure was reported and the chain flushed in `emit`; a stage that
    /// keeps failing during that flush ends the track through the same arm
    /// without a second report.
    StageFailed,
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
    /// disabled or the track carries no usable tag). Snapshotted in
    /// `prepare_track`; a seek keeps the track's gain, a new track reads
    /// its own.
    rg_gain: f32,
    /// "Stop after current track" is armed (RFC 0005 O8): the run ends at the
    /// end of the current track instead of advancing.
    stop_after_current: bool,
    shuffle: bool,
    repeat: bool,
    shuffle_plan: Vec<usize>,
    shuffle_pos: usize,
    prev_shuffle_plan: Option<Vec<usize>>,
    rng_state: u64,
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
    /// Timeline positions of repeat cycle loop seams for cycles that have been
    /// prebuffered while repeat was ON.
    cycle_boundaries: Vec<u64>,
}

impl Worker {
    fn run(&mut self, start_index: u32, start_at: Option<(u64, u32)>) {
        let mut index = start_index;
        let mut start_at = start_at;
        self.current_index = index;
        if self.shuffle && !self.cfg.queue.is_empty() {
            if self.shuffle_plan.len() != self.cfg.queue.len()
                || !self.shuffle_plan.contains(&(start_index as usize))
            {
                // The engine handed no usable plan: plan one anchored at the
                // track this run starts at (S15 §4.1) and report it, so the
                // engine's published plan matches the traversal.
                self.init_shuffle_plan(start_index as usize);
            } else {
                // Traverse the engine's plan (the engine owns it — S15);
                // the position is where this run's first track sits in it.
                self.shuffle_pos = self
                    .shuffle_plan
                    .iter()
                    .position(|&t| t == start_index as usize)
                    .unwrap_or(0);
            }
        }
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
            let mut decoder = match self.cfg.registry.open(source.media_source()) {
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
            self.prepare_track(index, decoder.as_ref(), continues);
            if let Some((frames, seq)) = start_at.take() {
                let to = self.frames_to_duration(frames);
                self.apply_seek(&mut decoder, to, seq, true);
            }
            let outcome = self.track_loop(&mut decoder);
            carry = outcome == Outcome::Ended;
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

            let next_target = match outcome {
                Outcome::Ended | Outcome::SkipNext => {
                    if !self.shuffle {
                        let next = index.saturating_add(1);
                        if (next as usize) < self.cfg.queue.len() {
                            Some(next)
                        } else if self.repeat && !self.cfg.queue.is_empty() {
                            self.cycle_boundaries.push(self.timeline);
                            Some(0)
                        } else {
                            None
                        }
                    } else {
                        if self.shuffle_plan.is_empty() {
                            self.init_shuffle_plan(index as usize);
                        }
                        if self.shuffle_pos + 1 < self.shuffle_plan.len() {
                            self.shuffle_pos += 1;
                            Some(self.shuffle_plan[self.shuffle_pos] as u32)
                        } else if self.repeat && !self.cfg.queue.is_empty() {
                            self.cycle_boundaries.push(self.timeline);
                            let last = self.shuffle_plan.last().copied();
                            self.replan_shuffle_new_cycle(last);
                            Some(self.shuffle_plan[0] as u32)
                        } else {
                            None
                        }
                    }
                }
                Outcome::SkipPrev => {
                    if !self.shuffle {
                        if index > 0 {
                            Some(index - 1)
                        } else if self.repeat && !self.cfg.queue.is_empty() {
                            Some((self.cfg.queue.len() - 1) as u32)
                        } else {
                            Some(0)
                        }
                    } else {
                        if self.shuffle_plan.is_empty() {
                            self.init_shuffle_plan(index as usize);
                        }
                        if self.shuffle_pos > 0 {
                            self.shuffle_pos -= 1;
                            Some(self.shuffle_plan[self.shuffle_pos] as u32)
                        } else if self.repeat && self.cfg.queue.len() > 1 {
                            if let Some(prev) = self.prev_shuffle_plan.take() {
                                self.shuffle_plan = prev;
                                self.shuffle_pos = self.shuffle_plan.len() - 1;
                                let _ = self.tx.send(DecodeOut::ShufflePlanned {
                                    plan: self.shuffle_plan.clone(),
                                    prev_plan: None,
                                });
                                Some(self.shuffle_plan[self.shuffle_pos] as u32)
                            } else {
                                self.shuffle_pos = self.shuffle_plan.len() - 1;
                                Some(self.shuffle_plan[self.shuffle_pos] as u32)
                            }
                        } else {
                            Some(self.shuffle_plan[0] as u32)
                        }
                    }
                }
                Outcome::SkipBy(delta) => {
                    if delta == 0 || self.cfg.queue.is_empty() {
                        Some(index)
                    } else if !self.shuffle {
                        let qlen = self.cfg.queue.len() as i32;
                        if self.repeat {
                            let curr = index as i32;
                            let rem = (curr + delta).rem_euclid(qlen);
                            Some(rem as u32)
                        } else {
                            Some(clamped_index(index, delta, self.cfg.queue.len()))
                        }
                    } else {
                        if self.shuffle_plan.is_empty() {
                            self.init_shuffle_plan(index as usize);
                        }
                        let plen = self.shuffle_plan.len() as i32;
                        if self.repeat {
                            let curr = self.shuffle_pos as i32;
                            let rem = (curr + delta).rem_euclid(plen);
                            self.shuffle_pos = rem as usize;
                            Some(self.shuffle_plan[self.shuffle_pos] as u32)
                        } else {
                            let curr = self.shuffle_pos as i32;
                            let target = (curr + delta).clamp(0, plen - 1) as usize;
                            self.shuffle_pos = target;
                            Some(self.shuffle_plan[self.shuffle_pos] as u32)
                        }
                    }
                }
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

    fn rng_next(&mut self) -> u64 {
        let mut x = self.rng_state;
        if x == 0 {
            x = 0x9E37_79B9_7F4A_7C15;
        }
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng_state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn rng_below(&mut self, upper: usize) -> usize {
        if upper <= 1 {
            return 0;
        }
        let n = upper as u64;
        let limit = u64::MAX / n * n;
        loop {
            let value = self.rng_next();
            if value < limit {
                return (value % n) as usize;
            }
        }
    }

    fn shuffle_slice(&mut self, items: &mut [usize]) {
        for i in (1..items.len()).rev() {
            let j = self.rng_below(i + 1);
            items.swap(i, j);
        }
    }

    fn init_shuffle_plan(&mut self, start_index: usize) {
        let len = self.cfg.queue.len();
        if len == 0 {
            self.shuffle_plan.clear();
            self.shuffle_pos = 0;
            return;
        }
        let mut remaining: Vec<usize> = (0..len).filter(|&i| i != start_index).collect();
        self.shuffle_slice(&mut remaining);
        let mut plan = Vec::with_capacity(len);
        plan.push(start_index);
        plan.extend(remaining);
        self.shuffle_plan = plan;
        self.shuffle_pos = 0;
        self.prev_shuffle_plan = None;
        // The engine owns the published plan; report one this worker had to
        // generate itself so the engine adopts it (S15).
        let _ = self.tx.send(DecodeOut::ShufflePlanned {
            plan: self.shuffle_plan.clone(),
            prev_plan: None,
        });
    }

    fn replan_shuffle_new_cycle(&mut self, last_track: Option<usize>) {
        let len = self.cfg.queue.len();
        if len == 0 {
            return;
        }
        let mut plan: Vec<usize> = (0..len).collect();
        self.shuffle_slice(&mut plan);
        if len > 1 && last_track.is_some() && Some(plan[0]) == last_track {
            let swap_idx = 1 + self.rng_below(len - 1);
            plan.swap(0, swap_idx);
        }
        self.prev_shuffle_plan = Some(std::mem::replace(&mut self.shuffle_plan, plan));
        self.shuffle_pos = 0;
        // Report the new cycle: the engine adopts it so its published plan
        // stays the one being traversed (S15).
        let _ = self.tx.send(DecodeOut::ShufflePlanned {
            plan: self.shuffle_plan.clone(),
            prev_plan: self.prev_shuffle_plan.clone(),
        });
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
    fn prepare_track(&mut self, index: u32, decoder: &dyn Decoder, continues: bool) {
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
        self.rg_gain = self.cfg.replay_gain.effective_gain_current();
        for stage in &mut self.dsp {
            let _ = stage.prepare(src);
        }
        self.dsp_trim = self.dsp_latency();
        if !continues {
            self.conv.prepare(src, self.cfg.device);
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
            let (_written, consumed) = self.cfg.sync.counters();
            self.cycle_boundaries
                .retain(|&boundary| consumed < boundary);
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
        for stage in &mut self.dsp {
            if let Err(e) = stage.process(&mut self.in_buf[..samples]) {
                // RFC 0014 §3.5: a stage error is terminal for the run and is
                // handled exactly like the decoder-error arm: report it, keep
                // the audio the chain still holds, end the track. The first
                // failure sticks (§4.2): a stage that keeps failing during
                // the latency flush ends the track through this same arm
                // without a second report.
                if !self.stage_failed {
                    self.stage_failed = true;
                    self.report_error(&e);
                    if matches!(self.flush_dsp(), Emit::Abandoned) {
                        return Emit::Abandoned;
                    }
                }
                return Emit::StageFailed;
            }
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
                Ok(DecodeMsg::SetShuffle {
                    on,
                    plan,
                    prev_plan,
                }) => {
                    if self.shuffle != on {
                        self.shuffle = on;
                        if on && !self.cfg.queue.is_empty() {
                            if plan.len() == self.cfg.queue.len() {
                                // The engine owns the plan (S15): adopt the
                                // one it published, anchored at the track
                                // that is playing.
                                self.shuffle_plan = plan;
                                self.prev_shuffle_plan = prev_plan;
                                self.shuffle_pos = self
                                    .shuffle_plan
                                    .iter()
                                    .position(|&t| t == self.current_index as usize)
                                    .unwrap_or(0);
                            } else {
                                // No usable plan from the engine: plan on our
                                // own and report it; the engine adopts it on
                                // its next poll.
                                self.init_shuffle_plan(self.current_index as usize);
                            }
                        } else {
                            self.shuffle_plan.clear();
                            self.prev_shuffle_plan = None;
                            self.shuffle_pos = 0;
                        }
                    }
                }
                Ok(DecodeMsg::SetRepeat(on)) => {
                    self.repeat = on;
                    if !on {
                        let (_written, consumed) = self.cfg.sync.counters();
                        if let Some(&boundary) =
                            self.cycle_boundaries.iter().find(|&&b| consumed < b)
                        {
                            self.cycle_boundaries.clear();
                            self.flush_converter();
                            let _ = self.tx.send(DecodeOut::EndOfStream {
                                timeline_end: boundary,
                            });
                            self.pending_skip = Some(Outcome::Stop);
                            return true;
                        }
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
                Ok(DecodeMsg::SetRepeat(on)) => {
                    self.repeat = on;
                    if on && !self.cfg.queue.is_empty() {
                        let _ = self.tx.send(DecodeOut::EndOfStreamRevoked);
                        if !self.shuffle {
                            self.cycle_boundaries.push(self.timeline);
                            return Some(0);
                        } else {
                            if self.shuffle_plan.is_empty() {
                                self.init_shuffle_plan(0);
                            }
                            let last = self.shuffle_plan.last().copied();
                            self.replan_shuffle_new_cycle(last);
                            self.cycle_boundaries.push(self.timeline);
                            return Some(self.shuffle_plan[0] as u32);
                        }
                    }
                }
                Ok(DecodeMsg::SetShuffle {
                    on,
                    plan,
                    prev_plan,
                }) => {
                    if self.shuffle != on {
                        self.shuffle = on;
                        if on && !self.cfg.queue.is_empty() {
                            if plan.len() == self.cfg.queue.len() {
                                self.shuffle_plan = plan;
                                self.prev_shuffle_plan = prev_plan;
                                self.shuffle_pos = 0;
                            } else {
                                self.init_shuffle_plan(0);
                            }
                        } else {
                            self.shuffle_plan.clear();
                            self.prev_shuffle_plan = None;
                            self.shuffle_pos = 0;
                        }
                    }
                }
                Ok(DecodeMsg::Next) => {
                    if self.repeat && !self.cfg.queue.is_empty() {
                        let _ = self.tx.send(DecodeOut::EndOfStreamRevoked);
                        if !self.shuffle {
                            self.cycle_boundaries.push(self.timeline);
                            return Some(0);
                        } else {
                            let last = self.shuffle_plan.last().copied();
                            self.replan_shuffle_new_cycle(last);
                            self.cycle_boundaries.push(self.timeline);
                            return Some(self.shuffle_plan[0] as u32);
                        }
                    }
                }
                Ok(DecodeMsg::Prev) => {
                    if !self.cfg.queue.is_empty() {
                        let _ = self.tx.send(DecodeOut::EndOfStreamRevoked);
                        if !self.shuffle {
                            if self.repeat {
                                return Some((self.cfg.queue.len() - 1) as u32);
                            } else {
                                return Some(0);
                            }
                        } else {
                            if self.shuffle_plan.is_empty() {
                                self.init_shuffle_plan(0);
                            }
                            if self.repeat
                                && self.cfg.queue.len() > 1
                                && let Some(prev) = self.prev_shuffle_plan.take()
                            {
                                self.shuffle_plan = prev;
                                self.shuffle_pos = self.shuffle_plan.len() - 1;
                                let _ = self.tx.send(DecodeOut::ShufflePlanned {
                                    plan: self.shuffle_plan.clone(),
                                    prev_plan: None,
                                });
                                return Some(self.shuffle_plan[self.shuffle_pos] as u32);
                            }
                            return Some(self.shuffle_plan[0] as u32);
                        }
                    }
                }
                Ok(DecodeMsg::SkipBy(delta)) => {
                    if delta != 0 && !self.cfg.queue.is_empty() && self.repeat {
                        let _ = self.tx.send(DecodeOut::EndOfStreamRevoked);
                        if !self.shuffle {
                            let qlen = self.cfg.queue.len() as i32;
                            let rem = delta.rem_euclid(qlen);
                            return Some(rem as u32);
                        } else {
                            if self.shuffle_plan.is_empty() {
                                self.init_shuffle_plan(0);
                            }
                            let plen = self.shuffle_plan.len() as i32;
                            let rem = delta.rem_euclid(plen) as usize;
                            self.shuffle_pos = rem;
                            return Some(self.shuffle_plan[self.shuffle_pos] as u32);
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
    fn apply_seek(
        &mut self,
        decoder: &mut Box<dyn Decoder>,
        to: Duration,
        seq: u32,
        initial: bool,
    ) {
        match decoder.seek(to) {
            Ok(reached) => {
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
}
