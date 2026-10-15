//! The decode thread: decoder -> DSP -> format converter -> ring.
//!
//! One thread per playback run. It owns the decoder, so all blocking I/O and
//! all allocation stays off the audio thread. It talks to the engine over two
//! channels (control in, reports out) plus the atomics of
//! [`crate::ring::RingSync`] for the seek handshake.
//!
//! Gapless: when a track ends the loop simply opens the next queue entry and
//! keeps writing, so the ring never runs dry at the seam and the timeline stays
//! continuous. Nothing is dropped and no silence is inserted. The one exception
//! is the "stop after current track" mode (RFC 0005 O8): when it is armed the
//! run ends at the end of the current track instead of opening the next entry.

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
}

/// Reports, decode thread -> engine.
#[derive(Debug)]
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
    /// A seek has been applied and the ring drained.
    Seeked {
        /// Echo of the engine's sequence number.
        seq: u32,
        /// Ring epoch the drain belongs to.
        epoch: u64,
        /// New absolute timeline position in device frames.
        timeline_pos: u64,
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
    /// A decoder or output error.
    Error {
        /// Classification.
        kind: ErrorKind,
        /// Message.
        message: String,
    },
    /// The thread has exited.
    Stopped,
    /// The ring's write end, handed back so the engine can start another run.
    Returned(Producer<f32>),
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
) -> (JoinHandle<()>, Consumer<VisFrame>) {
    let (vis, vis_rx) = VisTap::new(cfg.vis_capacity);
    let stop_after_current = cfg.stop_after_current;
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
        paused: false,
        quitting: false,
        stopped: false,
        stop_after_current,
        pending_seek: None,
        pending_skip: None,
        drain_epoch: None,
    };
    let handle = std::thread::Builder::new()
        .name("rustamp-decode".into())
        .spawn(move || {
            worker.run(start_index, start_at);
            let producer = worker.take_ring();
            let _ = worker.tx.send(DecodeOut::Stopped);
            let _ = worker.tx.send(DecodeOut::Returned(producer));
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
    paused: bool,
    quitting: bool,
    stopped: bool,
    /// "Stop after current track" is armed (RFC 0005 O8): the run ends at the
    /// end of the current track instead of advancing.
    stop_after_current: bool,
    pending_seek: Option<(Duration, u64, u32)>,
    pending_skip: Option<Outcome>,
    /// Ring epoch the consumer still has to drain before post-seek frames may
    /// be written.
    drain_epoch: Option<u64>,
}

impl Worker {
    fn run(&mut self, start_index: u32, start_at: Option<(u64, u32)>) {
        let mut index = start_index;
        let mut start_at = start_at;
        loop {
            if self.quitting || self.stopped {
                break;
            }
            let Some(source) = self.cfg.queue.get(index as usize).cloned() else {
                let _ = self.tx.send(DecodeOut::EndOfStream {
                    timeline_end: self.timeline,
                });
                break;
            };
            let mut decoder = match self.cfg.registry.open(source.media_source()) {
                Ok(d) => d,
                Err(e) => {
                    self.report_error(&e);
                    break;
                }
            };
            self.prepare_track(index, decoder.as_ref());
            if let Some((frames, seq)) = start_at.take() {
                let to = self.frames_to_duration(frames);
                self.apply_seek(&mut decoder, to, frames, seq);
            }
            let outcome = self.track_loop(&mut decoder);
            if outcome == Outcome::Ended && self.stop_after_current {
                // "Stop after current track" (RFC 0005 O8): the run ends here
                // instead of opening the next entry. Reporting the end of the
                // stream is what lets the engine stop the transport and park the
                // device, exactly as it does at the end of the queue.
                let _ = self.tx.send(DecodeOut::EndOfStream {
                    timeline_end: self.timeline,
                });
                break;
            }
            match outcome {
                Outcome::Ended | Outcome::SkipNext => index = index.saturating_add(1),
                Outcome::SkipPrev => index = index.saturating_sub(1),
                Outcome::SkipBy(delta) => {
                    index = clamped_index(index, delta, self.cfg.queue.len());
                }
                Outcome::Stop | Outcome::Quit => break,
            }
        }
    }

    /// Hand the ring's write end back (used when the thread exits).
    fn take_ring(&mut self) -> Producer<f32> {
        std::mem::replace(&mut self.ring, RingBuffer::<f32>::new(1).0)
    }

    /// Configure the DSP chain, the converter and the tap for a new track.
    fn prepare_track(&mut self, index: u32, decoder: &dyn Decoder) {
        let src = decoder.spec();
        self.src = src;
        for stage in &mut self.dsp {
            let _ = stage.prepare(src);
        }
        self.conv.prepare(src, self.cfg.device);
        self.vis.set_enabled(self.cfg.vis_enabled);
        self.vis.prepare(src);
        self.in_buf = vec![0.0; src.samples_for(READ_FRAMES)];
        self.out_buf = vec![
            0.0;
            self.cfg
                .device
                .samples_for(self.conv.max_out_frames(READ_FRAMES))
        ];
        self.track_start = self.timeline;
        let duration_frames = decoder
            .duration()
            .map(|d| (d.as_secs_f64() * f64::from(self.cfg.device.sample_rate)).round() as u64);
        let _ = self.tx.send(DecodeOut::TrackStarted {
            index,
            src,
            duration_frames,
            duration_ms: decoder.duration().map(|d| d.as_millis() as u64),
            metadata: Box::new(decoder.metadata()),
            stream: Box::new(decoder.stream_info()),
            timeline_start: self.timeline,
            seekable: decoder.is_seekable(),
        });
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
            if let Some((to, frames, seq)) = self.pending_seek.take() {
                self.apply_seek(decoder, to, frames, seq);
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
                    let _ = self.tx.send(DecodeOut::TrackEnded {
                        frames: self.timeline.saturating_sub(self.track_start),
                        skipped: decoder.skipped_packets(),
                    });
                    return Outcome::Ended;
                }
                Ok(n) => n,
                Err(Error::FormatChanged { from, to }) => {
                    self.on_format_changed(decoder.as_ref(), from, to);
                    continue;
                }
                Err(e) => {
                    self.report_error(&e);
                    return Outcome::Ended;
                }
            };
            let samples = frames * src_ch;
            for stage in &mut self.dsp {
                stage.process(&mut self.in_buf[..samples]);
            }
            self.vis.push(&self.in_buf[..samples]);
            let out_frames = self
                .conv
                .convert(&self.in_buf[..samples], &mut self.out_buf);
            let out_samples = self.cfg.device.samples_for(out_frames);
            let written = self.write_all(out_samples);
            self.timeline += written as u64;
            self.cfg.sync.set_written(self.timeline);
            if written < out_frames {
                return self.pending_skip.take().unwrap_or(Outcome::Stop);
            }
        }
    }

    /// Write up to `samples` device-format samples into the ring, waiting for
    /// space and servicing control messages. Returns the number of **frames**
    /// written (always frame-aligned); fewer than requested when the track must
    /// be abandoned.
    fn write_all(&mut self, samples: usize) -> usize {
        let channels = usize::from(self.cfg.device.channels.max(1));
        let mut written = 0;
        // A pending drain must complete before the first post-seek frame is
        // written, or the consumer would drop it.
        self.await_drain();
        if self.quitting || self.stopped {
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
        track_frames: u64,
        seq: u32,
    ) {
        match decoder.seek(to) {
            Ok(reached) => {
                for stage in &mut self.dsp {
                    stage.reset();
                }
                self.conv.reset();
                self.vis.reset();
                let epoch = self.cfg.sync.request_epoch();
                self.drain_epoch = Some(epoch);
                let frames =
                    (reached.as_secs_f64() * f64::from(self.cfg.device.sample_rate)).round() as u64;
                self.timeline = self.track_start + frames;
                self.cfg.sync.set_written(self.timeline);
                self.await_drain();
                if self.quitting || self.stopped {
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
                // Keep playing from where we are; report and carry on.
                let _ = self.tx.send(DecodeOut::Seeked {
                    seq,
                    epoch: self.cfg.sync.epoch(),
                    timeline_pos: self.timeline,
                });
                self.report_error(&e);
                let _ = track_frames;
            }
        }
    }

    /// Wait until the render side has dropped the pre-seek frames.
    ///
    /// Control messages are serviced while waiting, so `Stop`, `Quit` and
    /// `Pause` always work. While the output is paused there is no consumer to
    /// drain, so the wait is skipped and the drain stays pending: it happens on
    /// the first callback after the resume (the first block of new audio is then
    /// dropped, which is the documented price of seeking while paused).
    fn await_drain(&mut self) {
        let Some(epoch) = self.drain_epoch else {
            return;
        };
        loop {
            if self.cfg.sync.drained(epoch) {
                self.drain_epoch = None;
                return;
            }
            if !self.cfg.sync.is_active() {
                return;
            }
            if self.check_control() {
                self.drain_epoch = None;
                return;
            }
            std::thread::sleep(IDLE_SLEEP);
        }
    }

    fn on_format_changed(&mut self, decoder: &dyn Decoder, from: AudioSpec, to: AudioSpec) {
        self.src = to;
        self.conv.prepare(to, self.cfg.device);
        for stage in &mut self.dsp {
            let _ = stage.prepare(to);
        }
        self.vis.prepare(to);
        self.in_buf = vec![0.0; to.samples_for(READ_FRAMES)];
        self.out_buf = vec![
            0.0;
            self.cfg
                .device
                .samples_for(self.conv.max_out_frames(READ_FRAMES))
        ];
        let _ = self.tx.send(DecodeOut::FormatChanged {
            from,
            to,
            stream: Box::new(decoder.stream_info()),
        });
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
