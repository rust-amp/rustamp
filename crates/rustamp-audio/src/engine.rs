//! The audio engine: transport, gapless playback, the audio clock and the
//! visualization tap.
//!
//! # Threads
//!
//! * **Decode thread** ([`crate::decode`]) — decoder -> DSP -> converter ->
//!   ring. Owns all blocking I/O and allocation.
//! * **Output thread** — the device's own callback, which pulls from the ring.
//!   Created and owned by the device implementation (`rustamp-output`).
//! * **Caller thread** — this type. It applies commands, polls the decode
//!   thread and publishes events. The audio path takes no lock and allocates
//!   nothing.
//!
//! # The audio clock
//!
//! Position comes from [`OutputDevice::played_frames`], never from the decode
//! side. The engine keeps a pair `(clock_origin, timeline_origin)`:
//!
//! ```text
//! timeline          = timeline_origin + (played_frames() - clock_origin)
//! position_in_track = timeline - track_start
//! ```
//!
//! `timeline` is in **device frames** at the rate the device granted. A track's
//! `track_start` is the timeline value of its first frame, so a gapless join is
//! exactly continuous and no frame is dropped. A seek re-bases the pair in
//! [`Engine::poll`], after the decode thread has drained the ring and before it
//! writes the first post-seek frame, so the position lands exactly on the
//! target (frame-accurate, not time-rounded).
//!
//! # The equalizer (S07)
//!
//! The engine owns the equalizer state ([`EqState`], driven by the `SetBand`,
//! `SetPreamp`, `SetEqEnabled` and `SetEqAuto` commands and published in
//! [`PlayerState::eq`]) and a lock-free [`EqControl`] it writes that state
//! into. Every run puts a fresh [`Equalizer`] reading that control **first** in
//! the decode thread's DSP chain, so per block of decoded audio:
//!
//! ```text
//! decoder -> preamp -> 10 bands -> [installed DspStage]* -> vis tap
//!         -> FormatConverter -> ring -> volume/balance (render callback) -> device
//! ```
//!
//! A track's ReplayGain correction, when the mode is not disabled, is a
//! per-track constant scale on the raw decoder output, before the equalizer
//! (see [`crate::replaygain`]): the limiter and the converter's final
//! ceiling then see the normalized signal, and volume/balance keep their
//! attenuate-only promise on the render side.
//!
//! This is S07 §10's position "EQ on the decoded PCM before the final output
//! scaling". That any further DSP stage sees the **post-EQ** stream, and that
//! the vis tap sees the equalized signal, is the provisional choice named
//! here (S07 O11 stays open). A change reaches the ear after the ring's
//! buffered audio (the ring holds about 370 ms by default), because the stage
//! runs on the producer side of the ring. The default state (flat, disabled)
//! is a bit-exact bypass, so the stage is inert until the equalizer is used.
//!
//! # The look-ahead brickwall limiter
//!
//! The engine owns an [`Arc<LimiterControl>`] that drives the optional
//! look-ahead brickwall limiter ([`Limiter`]). Every run puts a fresh [`Limiter`]
//! reading that control **last** in the decode thread's DSP chain, after any
//! user-installed stages:
//!
//! ```text
//! decoder -> Equalizer -> [installed DspStage]* -> Limiter -> vis tap
//!         -> FormatConverter -> ring -> volume/balance (render callback) -> device
//! ```
//!
//! The limiter is placed last because it must see everything earlier stages
//! (such as EQ boosts or installed DSP plugins) produced, acting as the stage
//! closest to the output to catch any overs before the format converter and
//! the ring buffer.
//!
//! The limiter defaults to **enabled** (owner decision, 2026-10-17). That is
//! an own product choice, not 2.95 behaviour: what the reference player does
//! with overs is unmeasured. Disabled, it is a bit-exact bypass. Its
//! look-ahead ([`rustamp_dsp::LOOKAHEAD_SECONDS`], 5 ms) is reported through
//! `DspStage::latency_frames` and handled by the decode thread the way the
//! converter's is: the stage's start-up frames are dropped and its tail is
//! pushed out at the end of every track, so no frame is lost, no silence
//! enters a gapless seam and the position stays exact (see [`crate::decode`]).
//! The look-ahead therefore costs no timeline offset and no tail; it delays
//! only the decode thread's output by 5 ms of decoding, which the ring hides.
//! What remains is that the limiter is reset at every track change (the
//! `DspStage` contract), so its look-ahead never sees across a seam.
//!
//! # ReplayGain
//!
//! The engine owns a [`ReplayGainControl`](crate::replaygain::ReplayGainControl)
//! that selects the per-track correction ([`ReplayGainMode`](crate::replaygain::ReplayGainMode)
//! plus a preamp in dB). The decode thread snapshots it when it opens a
//! track and scales the raw decoder output before the DSP chain, so the
//! limiter (-0.3 dBFS ceiling) and the converter's final ±1.0 ceiling still
//! bound the normalized signal (see [`crate::replaygain`] for the placement
//! and the policy). The mode defaults to disabled — loudness never changes
//! until the user asks — and a track without a usable tag plays unadjusted
//! whatever the settings say.
//!
//! The settings live engine-side, like the limiter switch: [`PlayerState`]
//! has no ReplayGain field, and changing the wire contract requires an RFC.
//! There is no [`Command`] for them either (the command enum is a frozen
//! `rustamp-core` contract); the application layer calls the setters below
//! directly, as it does for [`Engine::set_limiter_enabled`].
//!
//! # Restarting with a non-empty ring (A8)
//!
//! A stopped run leaves its buffered audio in the ring. When a run starts with
//! the ring non-empty, the engine invalidates it (an epoch bump), the decode
//! thread waits for the render callback to drop it before its first write, and
//! the engine re-bases the clock at that drain, so no frame of the old run is
//! played and the drain's silent callback does not count as played time of the
//! new run. See [`crate::ring`] for the ordering.
//!
//! Device-buffer invalidation is separate from that ring drain. A confirmed
//! seek (decoder success + ring drain), stop/end or stopped-position change
//! requires [`OutputDevice::flush`]; a refused seek and pause/resume do not.
//! A flush refusal/failure closes the output and prevents another start on
//! this engine: the caller must construct a new engine/output lifecycle. This
//! is intentional for cpal, which has no portable endpoint reset; silently
//! resuming its retained stream would replay stale audio. Flush-capable
//! backends retain the existing paused-device stop/start lifecycle.
//!
//! The limiter's ceiling holds at its own output only. The resampler after it
//! (when the device rate differs from the track's) rebuilds inter-sample
//! peaks above the ceiling, and above full scale on hostile signals (A6
//! finding 3). The [`FormatConverter`] holds a final `±1.0` (0 dBFS) ceiling
//! on the resampled path (A14), so the ring — and hence the device — never
//! sees a sample above full scale however the equalizer and the limiter are
//! set. Volume and balance after the converter only attenuate.
//! Because [`PlayerState`] has no field for the limiter (and changing the wire
//! contract requires an RFC), the control remains engine-side via
//! [`Engine::limiter_enabled`], [`Engine::set_limiter_enabled`], and
//! [`Engine::limiter_control`].
//!
//! # Not in this milestone
//!
//! out_ds-style fades and prebuffering (S18), crossfade, the S08
//! vis derivation, and the playlist model (the engine only holds a queue of
//! re-openable sources and reports indices). See the task card and the report
//! for task card A1.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rtrb::{Consumer, Producer, RingBuffer};
use rustamp_core::audio::{
    AudioSpec, DecoderRegistry, DspStage, OutputDevice, OutputFormat, RenderFn,
};
use rustamp_core::bus::{CommandReceiver, EventBus};
use rustamp_core::command::Command;
use rustamp_core::error::{Error, ErrorInfo, ErrorKind, Result};
use rustamp_core::event::Event;
use rustamp_core::meta::TrackMetadata;
use rustamp_core::state::{EQ_BANDS, EqState, PlayerState, StreamInfo, TransportState};
use rustamp_core::vis::VisFrame;
use rustamp_dsp::{EqControl, Equalizer, Limiter, LimiterControl};

use crate::decode::{Config, DecodeMsg, DecodeOut, TrackSource, clamped_index, spawn};
use crate::replaygain::{ReplayGainControl, ReplayGainMode};
use crate::ring::{GainState, RingSync, make_render};

/// Engine configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct EngineConfig {
    /// Format requested from the output device. The device may grant another
    /// one; the engine converts to it.
    pub spec: AudioSpec,
    /// Ring capacity in device frames (about 370 ms of stereo at 44.1 kHz).
    pub ring_frames: usize,
    /// Vis frames the tap ring holds before it starts dropping them.
    pub vis_capacity: usize,
    /// Publish vis frames at all.
    pub vis_enabled: bool,
    /// Initial volume `0..=255`.
    pub volume: u8,
    /// Initial balance `-127..=127`.
    pub balance: i8,
    /// Minimum wall-clock distance between two `PositionChanged` events.
    pub position_interval: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        EngineConfig {
            spec: AudioSpec::new(44100, 2),
            ring_frames: 16384,
            vis_capacity: 8,
            vis_enabled: true,
            volume: 255,
            balance: 0,
            position_interval: Duration::from_millis(200),
        }
    }
}

/// A pending seek, waiting for the decode thread to report it applied.
#[derive(Debug, Clone, Copy)]
struct PendingSeek {
    seq: u32,
}

/// A run that started with a previous run's audio still in the ring, waiting
/// for the drain and the clock re-base (see [`crate::ring`]).
#[derive(Debug, Clone, Copy)]
struct StartDrain {
    /// Epoch bumped by `start_run`.
    epoch: u64,
    /// Device frames the ring held when the run started.
    stale_frames: u64,
}

/// The audio engine. One instance per player.
pub struct Engine {
    cfg: EngineConfig,
    registry: DecoderRegistry,
    output: Box<dyn OutputDevice>,
    device: AudioSpec,
    bus: EventBus,
    gain: Arc<GainState>,
    sync: Arc<RingSync>,
    /// Held until the device is started; the device owns it afterwards.
    render: Option<RenderFn>,
    started: bool,
    vis_rx: Option<Consumer<VisFrame>>,
    dsp: Vec<Box<dyn DspStage>>,
    /// Equalizer state as published in [`PlayerState::eq`].
    eq: EqState,
    /// Where [`Engine::eq`] reaches the running [`Equalizer`] stage.
    eq_control: Arc<EqControl>,
    /// Where [`Engine::limiter_enabled`] / [`Engine::set_limiter_enabled`] reaches
    /// the running [`Limiter`] stage.
    limiter_control: Arc<LimiterControl>,
    /// Where [`Engine::set_replaygain_mode`] / [`Engine::set_replaygain_preamp_db`]
    /// reaches the decode thread. Snapshotted once per track.
    replaygain_control: Arc<ReplayGainControl>,
    /// The ring's write end, parked here while no run is active.
    producer: Option<Producer<f32>>,

    to_decode: Option<Sender<DecodeMsg>>,
    from_decode: Option<Receiver<DecodeOut>>,
    handle: Option<JoinHandle<()>>,

    queue: Vec<TrackSource>,
    current: Option<u32>,
    transport: TransportState,
    volume: u8,
    balance: i8,
    /// "Stop after current track" is armed (RFC 0005 O8).
    stop_after_current: bool,

    clock_origin: u64,
    timeline_origin: u64,
    track_start: u64,
    duration_frames: Option<u64>,
    duration_ms: Option<u64>,
    src_spec: Option<AudioSpec>,
    stream: Option<StreamInfo>,
    metadata: TrackMetadata,
    end_of_stream: Option<u64>,
    seekable: bool,
    pending_seek: Option<PendingSeek>,
    start_drain: Option<StartDrain>,
    seek_seq: u32,
    start_at: Option<(u64, u32)>,
    last_position_event: Option<Instant>,
    skipped: u64,
    last_track_frames: u64,
    error: Option<ErrorInfo>,
    /// A failed flush destroyed the output's render callback. Only a fresh
    /// engine/output lifecycle may start again; stream_error alone is not a
    /// barrier, since close may clear a backend's failure latch.
    flush_failure: Option<String>,
    shuffle: bool,
    repeat: bool,
    shuffle_seed: u64,
    shuffle_plan: Vec<usize>,
    prev_shuffle_plan: Option<Vec<usize>>,
}

impl Engine {
    /// Open the output device and build the engine. The device is not started
    /// until the first [`Command::Play`].
    pub fn new(
        registry: DecoderRegistry,
        mut output: Box<dyn OutputDevice>,
        bus: EventBus,
        cfg: EngineConfig,
    ) -> Result<Engine> {
        let format: OutputFormat = output.open(cfg.spec)?;
        let device = format.spec;
        let ring_samples = cfg
            .ring_frames
            .max(256)
            .saturating_mul(usize::from(device.channels.max(1)));
        let (producer, consumer) = RingBuffer::<f32>::new(ring_samples);
        let sync = Arc::new(RingSync::new());
        let initial_volume = cfg.volume;
        let initial_balance = cfg.balance;
        let gain = Arc::new(GainState::new(initial_volume, initial_balance));
        let render = make_render(
            consumer,
            Arc::clone(&sync),
            Arc::clone(&gain),
            device.channels,
        );
        Ok(Engine {
            cfg,
            registry,
            output,
            device,
            bus,
            gain,
            sync,
            render: Some(render),
            started: false,
            vis_rx: None,
            dsp: Vec::new(),
            eq: EqState::default(),
            eq_control: Arc::new(EqControl::default()),
            limiter_control: Arc::new(LimiterControl::default()),
            replaygain_control: Arc::new(ReplayGainControl::default()),
            producer: Some(producer),
            to_decode: None,
            from_decode: None,
            handle: None,
            queue: Vec::new(),
            current: None,
            transport: TransportState::Stopped,
            volume: initial_volume,
            balance: initial_balance,
            stop_after_current: false,
            clock_origin: 0,
            timeline_origin: 0,
            track_start: 0,
            duration_frames: None,
            duration_ms: None,
            src_spec: None,
            stream: None,
            metadata: TrackMetadata::default(),
            end_of_stream: None,
            seekable: false,
            pending_seek: None,
            start_drain: None,
            seek_seq: 0,
            start_at: None,
            last_position_event: None,
            skipped: 0,
            last_track_frames: 0,
            error: None,
            flush_failure: None,
            shuffle: false,
            repeat: false,
            shuffle_seed: 0x9E37_79B9_7F4A_7C15,
            shuffle_plan: Vec::new(),
            prev_shuffle_plan: None,
        })
    }

    /// Install the DSP chain. Stages run on the decode thread; they are
    /// prepared for every track and on a mid-stream format change.
    ///
    /// `DspStage::process` is fallible (RFC 0014): an `Err` from a stage ends
    /// the track loudly — the decode thread reports it, flushes the chain and
    /// ends the track, and the engine publishes the error (the first failure
    /// per run sticks).
    ///
    /// To run the chain in a resident process (opt-in, RFC 0006 §6), build a
    /// stage with [`resident::DspResidentConfig::open`] (or wrap a session in
    /// [`resident::HostDspChain::new`]) and install it here. The default chain
    /// is empty and never spawns a process.
    pub fn set_dsp_chain(&mut self, dsp: Vec<Box<dyn DspStage>>) {
        self.dsp = dsp;
    }

    /// Replace the play queue. Entries must be re-openable (paths or URLs).
    pub fn set_queue(&mut self, queue: Vec<TrackSource>) {
        self.queue = queue;
        self.prev_shuffle_plan = None;
        if self.shuffle {
            self.replan_shuffle_engine(self.current.map(|i| i as usize));
        }
    }

    /// The current queue.
    pub fn queue(&self) -> &[TrackSource] {
        &self.queue
    }

    /// Queue index of the current track, if any.
    pub fn current_index(&self) -> Option<u32> {
        self.current
    }

    /// Transport state.
    pub fn transport(&self) -> TransportState {
        self.transport
    }

    /// Device name of the opened output.
    pub fn output_name(&self) -> &str {
        self.output.name()
    }

    /// Format the device granted.
    pub fn device_spec(&self) -> AudioSpec {
        self.device
    }

    /// Output latency reported by the device.
    pub fn latency(&self) -> Duration {
        self.output.latency()
    }

    /// Format of the current track's decoder, if a track is open.
    pub fn source_spec(&self) -> Option<AudioSpec> {
        self.src_spec
    }

    /// Metadata of the current track.
    pub fn metadata(&self) -> &TrackMetadata {
        &self.metadata
    }

    /// Last decoder or output error, if any.
    pub fn last_error(&self) -> Option<&ErrorInfo> {
        self.error.as_ref()
    }

    /// Packets the decoder skipped in the track that ended last (RFC 0002, C1).
    pub fn skipped_packets(&self) -> u64 {
        self.skipped
    }

    /// Frames the track that ended last produced, in device frames.
    pub fn last_track_frames(&self) -> u64 {
        self.last_track_frames
    }

    /// Total frames the device has consumed since it was opened.
    pub fn played_frames(&self) -> u64 {
        self.output.played_frames()
    }

    /// Absolute timeline position in device frames since the run started.
    pub fn timeline_frames(&self) -> u64 {
        self.timeline_origin.saturating_add(
            self.output
                .played_frames()
                .saturating_sub(self.clock_origin),
        )
    }

    /// Position inside the current track, in device frames.
    pub fn position_frames(&self) -> u64 {
        let pos = self.timeline_frames().saturating_sub(self.track_start);
        match self.duration_frames {
            Some(d) => pos.min(d),
            None => pos,
        }
    }

    /// Position inside the current track, in milliseconds.
    pub fn position_ms(&self) -> u64 {
        if self.device.sample_rate == 0 {
            return 0;
        }
        self.position_frames().saturating_mul(1000) / u64::from(self.device.sample_rate)
    }

    /// Duration of the current track in device frames, if known.
    pub fn duration_frames(&self) -> Option<u64> {
        self.duration_frames
    }

    /// Whether the current track can be seeked.
    pub fn is_seekable(&self) -> bool {
        self.seekable
    }

    /// Frames the decode thread has produced but the render callback has not
    /// yet read from the ring.
    ///
    /// This is the true ring backlog: `written - consumed`. It is zero only
    /// when every frame the decode thread produced has been consumed by the
    /// render callback. During an underrun the device clock may run ahead of
    /// `written`, but this value correctly stays at whatever is still in the
    /// ring — it never saturates a real backlog to zero.
    ///
    /// Audio of a previous run counts too (A8): while stopped, this is what
    /// the ring physically holds, and after a start it stays at that amount
    /// until the render callback has dropped it. Nothing of the new run can be
    /// in the ring before that drain (the decode thread waits for it).
    pub fn buffered_frames(&self) -> u64 {
        if let Some(start) = self.start_drain
            && !self.sync.drained(start.epoch)
        {
            return start.stale_frames;
        }
        if self.transport == TransportState::Stopped
            && let Some(producer) = self.producer.as_ref()
        {
            return self.ring_occupancy(producer);
        }
        let (written, consumed) = self.sync.counters();
        written.saturating_sub(consumed)
    }

    /// Whether a ring drain (of a seek, or of a run's start) has not been
    /// acknowledged by the render callback yet.
    pub fn is_drain_pending(&self) -> bool {
        self.sync.is_drain_pending()
    }

    /// Whether the current run started with a previous run's audio in the
    /// ring and has not finished its start yet: the drain or the clock re-base
    /// after it is still outstanding. The new run's position is exact only
    /// once this is `false`.
    pub fn is_start_pending(&self) -> bool {
        self.start_drain.is_some()
    }

    /// Device frames the ring holds, seen from its write end.
    fn ring_occupancy(&self, producer: &Producer<f32>) -> u64 {
        let channels = usize::from(self.device.channels.max(1));
        let unread = producer.buffer().capacity() - producer.slots();
        (unread / channels) as u64
    }

    /// The raw ring counters `(written, consumed)` in device frames, loaded
    /// consistently (see [`RingSync::counters`]). [`Engine::buffered_frames`]
    /// is their saturating difference, which hides `consumed > written`; this
    /// accessor lets a diagnostic or a test see such a misalignment.
    pub fn ring_counters(&self) -> (u64, u64) {
        self.sync.counters()
    }

    /// Absolute timeline position the decode thread reported as the end of the
    /// queue, once it has produced everything. `None` while it still has work.
    pub fn end_of_stream(&self) -> Option<u64> {
        self.end_of_stream
    }

    /// Snapshot of everything the UI needs.
    pub fn state(&self) -> PlayerState {
        PlayerState {
            transport: self.transport,
            position_ms: self.position_ms(),
            duration_ms: self.duration_ms,
            volume: self.volume,
            balance: self.balance,
            shuffle: self.shuffle,
            repeat: self.repeat,
            current_index: self.current,
            current_id: None,
            playlist_len: self.queue.len() as u32,
            stream: self.stream,
            eq: self.eq,
            stop_after_current: self.stop_after_current,
        }
    }

    /// Whether shuffle mode is enabled (spec S15).
    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    /// Whether repeat mode is enabled (spec S15).
    pub fn repeat(&self) -> bool {
        self.repeat
    }

    /// Set shuffle mode enabled or disabled (spec S15).
    pub fn set_shuffle(&mut self, shuffle: bool) -> Result<()> {
        if self.shuffle != shuffle {
            self.shuffle = shuffle;
            if self.shuffle && !self.queue.is_empty() {
                self.replan_shuffle_engine(self.current.map(|i| i as usize));
            } else {
                self.prev_shuffle_plan = None;
            }
            // The engine owns the plan (S15): hand it to the decode thread
            // so live traversal follows the published permutation.
            let (plan, prev_plan) = if self.shuffle {
                (self.shuffle_plan.clone(), self.prev_shuffle_plan.clone())
            } else {
                (Vec::new(), None)
            };
            self.send(DecodeMsg::SetShuffle {
                on: self.shuffle,
                plan,
                prev_plan,
            });
            self.bus.publish(Event::ModesChanged {
                shuffle: self.shuffle,
                repeat: self.repeat,
            });
        }
        Ok(())
    }

    /// Toggle shuffle mode (spec S15).
    pub fn toggle_shuffle(&mut self) -> Result<()> {
        self.set_shuffle(!self.shuffle)
    }

    /// Set repeat mode enabled or disabled (spec S15).
    pub fn set_repeat(&mut self, repeat: bool) -> Result<()> {
        if self.repeat != repeat {
            self.repeat = repeat;
            self.send(DecodeMsg::SetRepeat(self.repeat));
            self.bus.publish(Event::ModesChanged {
                shuffle: self.shuffle,
                repeat: self.repeat,
            });
        }
        Ok(())
    }

    /// Toggle repeat mode (spec S15).
    pub fn toggle_repeat(&mut self) -> Result<()> {
        self.set_repeat(!self.repeat)
    }

    /// Set the PRNG seed for shuffle permutations (spec S15; for tests).
    pub fn set_shuffle_seed(&mut self, seed: u64) {
        self.shuffle_seed = seed;
    }

    /// Active shuffle permutation plan.
    pub fn shuffle_plan(&self) -> &[usize] {
        &self.shuffle_plan
    }

    /// Advance the engine's shuffle PRNG one step and return its raw value
    /// (a xorshift64* step over the golden-ratio constant; S15 §1.2's own
    /// choice of algorithm).
    fn rng_step(&mut self) -> u64 {
        let mut seed = self.shuffle_seed;
        if seed == 0 {
            seed = 0x9E37_79B9_7F4A_7C15;
        }
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        self.shuffle_seed = seed;
        seed.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Re-plan the shuffle permutation. With `current`, the plan is anchored
    /// at that track (S15 §4.1: the playing track becomes the first entry);
    /// without one it is a full permutation of the queue (S15 §1.1).
    fn replan_shuffle_engine(&mut self, current: Option<usize>) {
        let len = self.queue.len();
        if len == 0 {
            self.shuffle_plan.clear();
            return;
        }
        let plan = if let Some(curr) = current.filter(|&c| c < len) {
            let mut remaining: Vec<usize> = (0..len).filter(|&i| i != curr).collect();
            for i in (1..remaining.len()).rev() {
                let j = (self.rng_step() % (i as u64 + 1)) as usize;
                remaining.swap(i, j);
            }
            let mut p = Vec::with_capacity(len);
            p.push(curr);
            p.extend(remaining);
            p
        } else {
            let mut p: Vec<usize> = (0..len).collect();
            for i in (1..p.len()).rev() {
                let j = (self.rng_step() % (i as u64 + 1)) as usize;
                p.swap(i, j);
            }
            p
        };
        self.shuffle_plan = plan;
        self.prev_shuffle_plan = None;
    }

    /// Generate the next shuffle cycle at a repeat boundary (S15 §2.1): a
    /// full permutation whose first entry is not the track the exhausted
    /// cycle ended on, so no track plays twice in a row across the boundary.
    fn replan_shuffle_new_cycle_engine(&mut self, last_track: Option<usize>) {
        let len = self.queue.len();
        if len == 0 {
            self.shuffle_plan.clear();
            self.prev_shuffle_plan = None;
            return;
        }
        let mut p: Vec<usize> = (0..len).collect();
        for i in (1..p.len()).rev() {
            let j = (self.rng_step() % (i as u64 + 1)) as usize;
            p.swap(i, j);
        }
        if len > 1 && Some(p[0]) == last_track {
            let swap_idx = 1 + (self.rng_step() % (len as u64 - 1)) as usize;
            p.swap(0, swap_idx);
        }
        self.prev_shuffle_plan = Some(std::mem::replace(&mut self.shuffle_plan, p));
    }

    /// Make the shuffle plan coherent with the queue and the current track
    /// (S15): re-plan only when the plan cannot be the one being traversed.
    fn ensure_shuffle_plan(&mut self) {
        if !self.shuffle || self.queue.is_empty() {
            return;
        }
        let usable = self.shuffle_plan.len() == self.queue.len()
            && self
                .current
                .map(|c| self.shuffle_plan.contains(&(c as usize)))
                .unwrap_or(true);
        if !usable {
            self.replan_shuffle_engine(self.current.map(|i| i as usize));
        }
    }

    /// Position of the current track in the active shuffle plan (S15). The
    /// plan is the single source of truth and the position is derived from
    /// the current track, so it can never go stale while a run advances it.
    fn shuffle_pos_of_current(&self) -> usize {
        match self.current {
            Some(c) => self
                .shuffle_plan
                .iter()
                .position(|&t| t as u32 == c)
                .unwrap_or(0),
            None => 0,
        }
    }

    /// Current equalizer state.
    pub fn eq(&self) -> EqState {
        self.eq
    }

    /// Replace the whole equalizer state (for example a loaded preset).
    ///
    /// Gains are clamped to `±`[`EqState::MAX_GAIN_DB`]; a non-finite gain
    /// keeps the previous value of that slot. The running stage picks the new
    /// state up on its next block and slews to it (see `rustamp_dsp::eq`).
    pub fn set_eq(&mut self, eq: EqState) {
        let mut next = self.eq;
        for (slot, gain) in next.bands.iter_mut().zip(eq.bands) {
            *slot = clamp_gain(gain).unwrap_or(*slot);
        }
        next.preamp = clamp_gain(eq.preamp).unwrap_or(next.preamp);
        next.enabled = eq.enabled;
        next.auto = eq.auto;
        if next != self.eq {
            self.eq = next;
            self.eq_control.set(&self.eq);
            self.bus.publish(Event::EqChanged(self.eq));
        }
    }

    /// Set one band's gain in dB (`band < EQ_BANDS`; others are ignored).
    pub fn set_eq_band(&mut self, band: u8, gain_db: f32) -> Result<()> {
        let band = usize::from(band);
        if band < EQ_BANDS {
            let mut eq = self.eq;
            eq.bands[band] = gain_db;
            self.set_eq(eq);
        }
        Ok(())
    }

    /// Whether the look-ahead brickwall limiter is enabled.
    ///
    /// The limiter defaults to enabled (owner decision, 2026-10-17; an own
    /// choice, since the reference player's behaviour with overs is
    /// unmeasured). Its look-ahead ([`rustamp_dsp::LOOKAHEAD_SECONDS`]) does not
    /// shift the timeline (see the module docs). Disabled, it is a bit-exact
    /// bypass. Because [`PlayerState`] has no field for the limiter (and
    /// changing the wire contract requires an RFC), the control remains
    /// engine-side.
    pub fn limiter_enabled(&self) -> bool {
        self.limiter_control.enabled()
    }

    /// Enable or disable the look-ahead brickwall limiter (enabled by default).
    ///
    /// Takes effect at the stage's next track start or seek (prepare/reset).
    pub fn set_limiter_enabled(&mut self, enabled: bool) {
        self.limiter_control.set_enabled(enabled);
    }

    /// The limiter's control block, shared with the running limiter stage on
    /// the decode thread.
    pub fn limiter_control(&self) -> Arc<LimiterControl> {
        Arc::clone(&self.limiter_control)
    }

    /// The output ceiling in linear amplitude below which limited samples are held.
    pub fn limiter_ceiling(&self) -> f32 {
        10.0_f64.powf(rustamp_dsp::limiter::CEILING_DB / 20.0) as f32
    }

    /// The ReplayGain mode: which tag normalizes a track, if any.
    ///
    /// Disabled by default (an own choice: loudness never changes until the
    /// user asks). Because [`PlayerState`] has no field for it (and changing
    /// the wire contract requires an RFC), the setting remains engine-side.
    pub fn replaygain_mode(&self) -> ReplayGainMode {
        self.replaygain_control.mode()
    }

    /// Select which ReplayGain value normalizes a track
    /// ([`ReplayGainMode::Disabled`], [`ReplayGainMode::Track`] or
    /// [`ReplayGainMode::Album`]).
    ///
    /// Takes effect from the next track (or seek): the decode thread
    /// snapshots the setting when it opens a track.
    pub fn set_replaygain_mode(&mut self, mode: ReplayGainMode) {
        self.replaygain_control.set_mode(mode);
    }

    /// The ReplayGain preamp in dB: a user offset added on top of the
    /// selected track/album gain (`0.0` by default).
    ///
    /// It only shifts a selected gain; a track without a usable tag plays
    /// unadjusted whatever the preamp says. Engine-side, like the mode.
    pub fn replaygain_preamp_db(&self) -> f32 {
        self.replaygain_control.preamp_db()
    }

    /// Set the ReplayGain preamp in dB (clamped to
    /// `±`[`crate::replaygain::MAX_PREAMP_DB`]; non-finite keeps the
    /// previous value). Takes effect from the next track (or seek).
    pub fn set_replaygain_preamp_db(&mut self, preamp_db: f32) {
        self.replaygain_control.set_preamp_db(preamp_db);
    }

    /// The ReplayGain control block, shared with the decode thread.
    pub fn replaygain_control(&self) -> Arc<ReplayGainControl> {
        Arc::clone(&self.replaygain_control)
    }

    /// Whether "stop after current track" is armed (RFC 0005 O8).
    pub fn stop_after_current(&self) -> bool {
        self.stop_after_current
    }

    /// Newest vis frame, if one was published since the last call. Older frames
    /// are dropped: the visualization only ever wants the newest.
    pub fn take_vis_frame(&mut self) -> Option<VisFrame> {
        let consumer = self.vis_rx.as_mut()?;
        let mut newest = None;
        while let Ok(frame) = consumer.pop() {
            newest = Some(frame);
        }
        newest
    }

    // -----------------------------------------------------------------------
    // Transport
    // -----------------------------------------------------------------------

    /// Apply one command. Commands that do not belong to the engine (playlist,
    /// windows) are ignored.
    pub fn apply(&mut self, command: &Command) -> Result<()> {
        match command {
            Command::Play => self.play(),
            Command::Pause => self.pause(),
            Command::TogglePause => {
                if self.transport == TransportState::Playing {
                    self.pause()
                } else {
                    self.play()
                }
            }
            Command::Stop | Command::StopWithFadeout => self.stop(),
            Command::Next => self.next_track(),
            Command::Prev => self.prev_track(),
            Command::Seek { to_ms } => self.seek_ms(*to_ms),
            Command::SeekRelative { delta_ms } => {
                let target = if *delta_ms < 0 {
                    self.position_ms().saturating_sub(delta_ms.unsigned_abs())
                } else {
                    self.position_ms().saturating_add(*delta_ms as u64)
                };
                self.seek_ms(target)
            }
            Command::SetVolume(v) => self.set_volume(*v),
            Command::SetBalance(b) => self.set_balance(*b),
            Command::PlayIndex(index) => self.play_index(*index),
            Command::SkipBy { delta } => self.skip_by(*delta),
            Command::PlayLast => self.play_last(),
            Command::ToggleStopAfterCurrent => self.toggle_stop_after_current(),
            Command::ToggleShuffle => self.toggle_shuffle(),
            Command::ToggleRepeat => self.toggle_repeat(),
            Command::SetShuffle(on) => self.set_shuffle(*on),
            Command::SetRepeat(on) => self.set_repeat(*on),
            Command::SetBand { band, gain_db } => self.set_eq_band(*band, *gain_db),
            Command::SetPreamp(gain_db) => {
                self.set_eq(EqState {
                    preamp: *gain_db,
                    ..self.eq
                });
                Ok(())
            }
            Command::SetEqEnabled(enabled) => {
                self.set_eq(EqState {
                    enabled: *enabled,
                    ..self.eq
                });
                Ok(())
            }
            Command::SetEqAuto(auto) => {
                self.set_eq(EqState {
                    auto: *auto,
                    ..self.eq
                });
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Apply every command that is queued in `rx`; returns how many were
    /// applied.
    pub fn drain_commands(&mut self, rx: &CommandReceiver) -> Result<usize> {
        let mut applied = 0;
        while let Some(cmd) = rx.try_recv()? {
            self.apply(&cmd)?;
            applied += 1;
        }
        Ok(applied)
    }

    /// Start or resume playback of the current track.
    pub fn play(&mut self) -> Result<()> {
        if let Some(message) = &self.flush_failure {
            return Err(Error::Output(message.clone()));
        }
        match self.transport {
            TransportState::Playing => Ok(()),
            TransportState::Paused => {
                if let Some(err) = self.output.stream_error() {
                    self.handle_output_error(err);
                    return Ok(());
                }
                self.send(DecodeMsg::Resume);
                self.output.resume()?;
                self.sync.set_active(true);
                self.transport = TransportState::Playing;
                self.publish_state();
                Ok(())
            }
            TransportState::Stopped => self.start_run(),
        }
    }

    /// Pause. The decode thread stops writing, the device stops pulling and the
    /// audio clock freezes.
    pub fn pause(&mut self) -> Result<()> {
        if self.transport != TransportState::Playing {
            return Ok(());
        }
        self.send(DecodeMsg::Pause);
        self.sync.set_active(false);
        self.output.pause()?;
        self.transport = TransportState::Paused;
        self.publish_state();
        self.publish_position(true);
        Ok(())
    }

    /// Stop, reset the position, drop the device buffer and leave the device
    /// open but paused. If flush is refused/fails, closes the output, reports
    /// the error and requires a fresh engine before playback can start again.
    pub fn stop(&mut self) -> Result<()> {
        self.send(DecodeMsg::Stop);
        self.sync.set_active(false);
        let _ = self.output.pause();
        let flushed = self.flush_output();
        let _ = self.join_decode();
        self.transport = TransportState::Stopped;
        self.clock_origin = self.output.played_frames();
        self.timeline_origin = 0;
        self.track_start = 0;
        self.pending_seek = None;
        self.start_drain = None;
        self.start_at = None;
        self.end_of_stream = None;
        // The run's counters end here. What it left in the ring stays there
        // (only the consumer can drop it) and is reported by
        // `buffered_frames()` until the next run drains it.
        self.sync.set_written(0);
        self.sync.set_consumed(0);
        self.bus
            .publish(Event::StateChanged(TransportState::Stopped));
        self.bus.publish(Event::PositionChanged {
            position_ms: 0,
            duration_ms: self.duration_ms,
        });
        if let Err(ref err) = flushed {
            self.report_output_error(err);
        }
        flushed
    }

    /// Play the queue entry at `index`.
    pub fn play_index(&mut self, index: u32) -> Result<()> {
        if index as usize >= self.queue.len() {
            return Err(Error::NotFound(format!("queue index {index}")));
        }
        if self.transport != TransportState::Stopped {
            self.stop()?;
        }
        self.current = Some(index);
        self.start_run()
    }

    /// Skip to the next queue entry. Under S15 modes, respects shuffle and repeat.
    pub fn next_track(&mut self) -> Result<()> {
        if self.transport == TransportState::Stopped || self.to_decode.is_none() {
            if self.queue.is_empty() {
                return Ok(());
            }
            let next = if !self.shuffle {
                let n = self.current.map(|i| i + 1).unwrap_or(0);
                if (n as usize) < self.queue.len() {
                    Some(n)
                } else if self.repeat {
                    Some(0)
                } else {
                    None
                }
            } else {
                self.ensure_shuffle_plan();
                if self.current.is_none() {
                    Some(self.shuffle_plan[0] as u32)
                } else {
                    let pos = self.shuffle_pos_of_current();
                    if pos + 1 < self.shuffle_plan.len() {
                        Some(self.shuffle_plan[pos + 1] as u32)
                    } else if self.repeat {
                        // A new cycle starts at its first entry, which is
                        // not the track the exhausted cycle ended on
                        // (S15 §2.1).
                        let last = self.current.map(|c| c as usize);
                        self.replan_shuffle_new_cycle_engine(last);
                        Some(self.shuffle_plan[0] as u32)
                    } else {
                        None
                    }
                }
            };
            if let Some(target) = next {
                return self.play_index(target);
            }
            return Ok(());
        }
        self.send(DecodeMsg::Next);
        Ok(())
    }

    /// Go back to the previous queue entry. Under S15 modes, respects shuffle and repeat.
    pub fn prev_track(&mut self) -> Result<()> {
        if self.transport == TransportState::Stopped || self.to_decode.is_none() {
            if self.queue.is_empty() {
                return Ok(());
            }
            let prev = if !self.shuffle {
                match self.current {
                    Some(0) if self.repeat => Some((self.queue.len() - 1) as u32),
                    Some(i) if i > 0 => Some(i - 1),
                    _ => Some(0),
                }
            } else {
                self.ensure_shuffle_plan();
                // Walk back along the plan (S15 §3.2): the entry before the
                // current one is the track played immediately before it.
                let pos = self.shuffle_pos_of_current();
                if pos > 0 {
                    Some(self.shuffle_plan[pos - 1] as u32)
                } else if self.repeat && self.queue.len() > 1 {
                    if let Some(prev) = self.prev_shuffle_plan.take() {
                        self.shuffle_plan = prev;
                        let last_idx = self.shuffle_plan.len() - 1;
                        Some(self.shuffle_plan[last_idx] as u32)
                    } else {
                        Some(self.shuffle_plan[self.shuffle_plan.len() - 1] as u32)
                    }
                } else {
                    self.current.or(Some(0))
                }
            };
            if let Some(target) = prev {
                return self.play_index(target);
            }
            return Ok(());
        }
        self.send(DecodeMsg::Prev);
        Ok(())
    }

    /// Move the current entry by `delta` positions in the order
    /// [`Engine::next_track`]/[`Engine::prev_track`] follow, clamped or wrapped
    /// according to the repeat mode (RFC 0005 `Command::SkipBy`, ids 40195 / 40197).
    pub fn skip_by(&mut self, delta: i32) -> Result<()> {
        if delta == 0 || self.queue.is_empty() {
            return Ok(());
        }
        if self.transport == TransportState::Stopped || self.to_decode.is_none() {
            let curr = self.current.unwrap_or(0);
            if !self.shuffle {
                let qlen = self.queue.len() as i32;
                if self.repeat {
                    let rem = (curr as i32 + delta).rem_euclid(qlen);
                    self.current = Some(rem as u32);
                } else {
                    self.current = Some(clamped_index(curr, delta, self.queue.len()));
                }
            } else {
                self.ensure_shuffle_plan();
                let plen = self.shuffle_plan.len() as i32;
                let pos = self.shuffle_pos_of_current() as i32;
                let target = if self.repeat {
                    (pos + delta).rem_euclid(plen)
                } else {
                    (pos + delta).clamp(0, plen - 1)
                } as usize;
                self.current = Some(self.shuffle_plan[target] as u32);
            }
            return Ok(());
        }
        self.send(DecodeMsg::SkipBy(delta));
        Ok(())
    }

    /// Play the last queue entry (RFC 0005 `Command::PlayLast`); a no-op when
    /// the queue is empty. Mirrors [`Engine::play_index`]`(0)` for the other end
    /// of the list: the engine resolves the last entry, so the caller needs no
    /// length.
    pub fn play_last(&mut self) -> Result<()> {
        match self.queue.len().checked_sub(1) {
            Some(last) => self.play_index(last as u32),
            None => Ok(()),
        }
    }

    /// Toggle "stop after current track" (RFC 0005 O8, id 40157). While armed,
    /// the end of the current track ends the run instead of advancing.
    ///
    /// The mode is a persistent toggle (the RFC's `Toggle*` convention); whether
    /// the reference player clears it after it fires is unmeasured, and the mode
    /// is observable through [`PlayerState::stop_after_current`].
    pub fn toggle_stop_after_current(&mut self) -> Result<()> {
        self.stop_after_current = !self.stop_after_current;
        self.send(DecodeMsg::SetStopAfterCurrent(self.stop_after_current));
        Ok(())
    }

    /// Seek inside the current track, in milliseconds.
    pub fn seek_ms(&mut self, ms: u64) -> Result<()> {
        let frames = ms.saturating_mul(u64::from(self.device.sample_rate)) / 1000;
        self.seek_frames(frames)
    }

    /// Seek inside the current track, in device frames (frame-accurate).
    pub fn seek_frames(&mut self, frames: u64) -> Result<()> {
        let frames = match self.duration_frames {
            Some(d) => frames.min(d),
            None => frames,
        };
        self.seek_seq = self.seek_seq.wrapping_add(1);
        let seq = self.seek_seq;
        if self.transport == TransportState::Stopped || self.to_decode.is_none() {
            if let Err(err) = self.flush_output() {
                // A decoder can already have exited while its buffered run is
                // paused. A refused flush must stop that case too, not leave a
                // Paused engine able to resume a now-closed stream.
                self.handle_output_error(Error::Output(err.to_string()));
                return Err(err);
            }
            // Not running: remember the target and apply it when playback
            // starts. The displayed position moves to the target right away.
            self.start_at = Some((frames, seq));
            self.timeline_origin = frames;
            self.track_start = 0;
            self.clock_origin = self.output.played_frames();
            self.publish_position(true);
            return Ok(());
        }
        let to = self.device.duration_of(frames);
        self.pending_seek = Some(PendingSeek { seq });
        self.send(DecodeMsg::Seek {
            to,
            track_frames: frames,
            seq,
        });
        Ok(())
    }

    /// Set the volume (`0..=255`).
    pub fn set_volume(&mut self, volume: u8) -> Result<()> {
        self.volume = volume;
        self.gain.set(self.volume, self.balance);
        self.bus.publish(Event::VolumeChanged {
            volume: self.volume,
            balance: self.balance,
        });
        Ok(())
    }

    /// Set the balance (`-127..=127`; values outside are clamped).
    pub fn set_balance(&mut self, balance: i8) -> Result<()> {
        self.balance = balance.clamp(-127, 127);
        self.gain.set(self.volume, self.balance);
        self.bus.publish(Event::VolumeChanged {
            volume: self.volume,
            balance: self.balance,
        });
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Polling
    // -----------------------------------------------------------------------

    /// Process everything the decode thread reported and publish the resulting
    /// events. Call this regularly (the CLI example does it every few ms).
    pub fn poll(&mut self) -> Result<()> {
        if self.transport == TransportState::Playing
            && let Some(err) = self.output.stream_error()
        {
            self.handle_output_error(err);
            return Ok(());
        }
        if let Some(rx) = self.from_decode.take() {
            let mut output_error = None;
            loop {
                match rx.try_recv() {
                    Ok(out) => {
                        if let Err(err) = self.on_decode_out(out) {
                            output_error = Some(err);
                            break;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
                }
            }
            self.from_decode = Some(rx);
            if let Some(err) = output_error {
                // Restore the receiver before teardown so join_decode recovers
                // Returned and does not leave queued seek reports to rebase a
                // stopped run on the next poll.
                self.handle_output_error(err);
                return Ok(());
            }
        }
        // Finish the run only when the ring is truly empty: every frame the
        // decode thread wrote has been consumed by the render callback. Using
        // the device clock here is wrong — it advances for silence frames
        // during underruns, which would trigger a premature finish. Using
        // `consumed >= written` instead guarantees we wait until the last
        // real audio frame leaves the ring, not merely until the device clock
        // passes the nominal end time.
        if let Some(end) = self.end_of_stream {
            let (written, consumed) = self.sync.counters();
            if written >= end && consumed >= written && self.transport == TransportState::Playing {
                self.finish_run();
            }
        }
        self.publish_position(false);
        Ok(())
    }

    fn on_decode_out(&mut self, out: DecodeOut) -> Result<()> {
        match out {
            DecodeOut::TrackStarted {
                index,
                src,
                duration_frames,
                duration_ms,
                metadata,
                stream,
                timeline_start,
                seekable,
            } => {
                self.current = Some(index);
                self.src_spec = Some(src);
                self.duration_frames = duration_frames;
                self.duration_ms = duration_ms;
                self.metadata = *metadata;
                self.stream = Some(*stream);
                self.track_start = timeline_start;
                self.seekable = seekable;
                self.end_of_stream = None;
                self.bus.publish(Event::TrackChanged {
                    id: None,
                    index: Some(index),
                    metadata: Some(Box::new(self.metadata.clone())),
                    duration_ms: self.duration_ms,
                });
                self.bus.publish(Event::StreamInfoChanged(self.stream));
                self.publish_position(true);
            }
            DecodeOut::Seeked {
                seq,
                epoch,
                timeline_pos,
            } => {
                if self.pending_seek.map(|p| p.seq) == Some(seq) {
                    self.pending_seek = None;
                }
                // An initial seek's drain also drops a previous run's audio,
                // and this re-base replaces the start's.
                self.start_drain = None;
                // Drop what the device still holds (pre-seek audio and the
                // drain callback's silence) so the new timeline plays clean.
                self.flush_output()?;
                // Re-base the audio clock exactly at the drain: every frame the
                // device plays from now on belongs to the new position.
                self.clock_origin = self.output.played_frames();
                self.timeline_origin = timeline_pos;
                self.sync.ack_engine(epoch);
                self.publish_position(true);
            }
            DecodeOut::SeekFailed { seq, initial } => {
                if self.pending_seek.map(|p| p.seq) == Some(seq) {
                    self.pending_seek = None;
                }
                if initial {
                    // Undo only the optimistic stopped-seek target, not
                    // elapsed playback or a pending previous-run drain.
                    self.timeline_origin = self.track_start;
                    self.publish_position(true);
                }
                // No flush, clock re-base, epoch ack or start-drain clear:
                // neither the decoder nor its still-playable buffers moved.
            }
            DecodeOut::StartDrained {
                epoch,
                timeline_pos,
            } => {
                // The previous run's audio is gone and the decode thread has
                // not written yet: everything the device played since the
                // start (the drain's silent callback included) was no audio
                // of this run, so the clock starts here.
                self.start_drain = None;
                self.clock_origin = self.output.played_frames();
                self.timeline_origin = timeline_pos;
                self.sync.ack_engine(epoch);
                self.publish_position(true);
            }
            DecodeOut::TrackEnded { skipped, frames } => {
                self.skipped = skipped;
                self.last_track_frames = frames;
            }
            DecodeOut::FormatChanged { from, to, stream } => {
                self.stream = Some(*stream);
                self.src_spec = Some(to);
                self.bus.publish(Event::Error(ErrorInfo {
                    kind: ErrorKind::FormatChanged,
                    message: format!(
                        "stream format changed: {} Hz/{} ch -> {} Hz/{} ch",
                        from.sample_rate, from.channels, to.sample_rate, to.channels
                    ),
                }));
                self.bus.publish(Event::StreamInfoChanged(self.stream));
            }
            DecodeOut::EndOfStream { timeline_end } => {
                self.end_of_stream = Some(timeline_end);
            }
            DecodeOut::ShufflePlanned { plan, prev_plan } => {
                // The worker reports only plans it generated itself (a cycle
                // boundary, or a fallback when the engine handed none);
                // adopt it so `shuffle_plan()` stays the plan being
                // traversed (S15).
                if plan.len() == self.queue.len() {
                    self.shuffle_plan = plan;
                    self.prev_shuffle_plan = prev_plan;
                }
            }
            DecodeOut::Error { kind, message } => {
                let info = ErrorInfo { kind, message };
                self.error = Some(info.clone());
                self.bus.publish(Event::Error(info));
            }
            DecodeOut::Stopped => {
                self.to_decode = None;
            }
            DecodeOut::Returned { producer, dsp } => {
                self.producer = Some(producer);
                self.dsp = dsp;
            }
        }
        Ok(())
    }

    fn finish_run(&mut self) {
        // Save end_of_stream before clearing it: that is the exact timeline
        // position of the last frame the decode thread produced. The device
        // clock (`timeline_frames()`) may be ahead of it when silence was
        // played during underruns, so we must NOT freeze at timeline_frames().
        let frozen_end = self.end_of_stream;
        self.end_of_stream = None;
        self.start_drain = None;
        self.sync.set_active(false);
        let _ = self.output.pause();
        let flushed = self.flush_output();
        let _ = self.join_decode();
        self.transport = TransportState::Stopped;
        // Freeze the timeline at the content end, not at the device position.
        // Re-basing the clock alone would reset `timeline_frames()` to the
        // timeline_origin (zero for a run without a seek), which would report a
        // full track as unplayed. Using the saved `end_of_stream` value ensures
        // position_frames() lands exactly at the track boundary.
        self.timeline_origin = frozen_end.unwrap_or_else(|| self.timeline_frames());
        self.clock_origin = self.output.played_frames();
        // track_start keeps its final value.
        self.bus
            .publish(Event::StateChanged(TransportState::Stopped));
        self.bus.publish(Event::PositionChanged {
            position_ms: self.position_ms(),
            duration_ms: self.duration_ms,
        });
        // "Stop after current track" (RFC 0005 O8) halts the run at the end of
        // the current track instead of advancing; that is a stop, not the end
        // of the playlist. `PlaylistEnded` fires iff the queue was actually
        // exhausted — the entry that just ended is the last one — so arming
        // the mode on the last track still ends the playlist. The RFC leaves
        // the exact interaction open (it rides on S02 open question 5); this
        // is the implemented reading.
        let queue_exhausted = !self.repeat
            && self
                .current
                .map(|index| {
                    if self.stop_after_current {
                        index as usize + 1 >= self.queue.len()
                    } else {
                        true
                    }
                })
                .unwrap_or(true);
        if queue_exhausted {
            self.bus.publish(Event::PlaylistEnded);
        }
        if let Err(err) = flushed {
            self.report_output_error(&err);
        }
    }

    /// Tear down playback after a fatal mid-run output device failure (RFC 0012).
    ///
    /// Stops decoding immediately, pauses and flushes the device, transitions
    /// transport to [`TransportState::Stopped`], freezes the timeline position,
    /// and publishes [`Event::StateChanged`], [`Event::PositionChanged`] and
    /// [`Event::Error`].
    ///
    /// A track start the decode thread already reported is published before
    /// the freeze (M25 §1.1, A25, M30): the join below holds the queued
    /// `TrackStarted` instead of discarding it, and delivering it here keeps
    /// the window on the track that actually started — its duration, not the
    /// previous track's.
    fn handle_output_error(&mut self, err: Error) {
        let frozen_timeline = self.timeline_frames();
        self.send(DecodeMsg::Stop);
        self.sync.set_active(false);
        let _ = self.output.pause();
        let _ = self.flush_output();
        // Deliver the held start before the freeze and the error
        // publication. The `TrackStarted` arm touches no timeline origin and
        // no output, so the freeze stays exact, the error is still published
        // once, and the sticky output-flush failure cannot be armed (M25 §1.1,
        // A25, M30).
        if let Some(start) = self.join_decode() {
            let _ = self.on_decode_out(start);
        }
        self.transport = TransportState::Stopped;
        self.timeline_origin = frozen_timeline;
        self.clock_origin = self.output.played_frames();
        self.pending_seek = None;
        self.start_drain = None;
        self.end_of_stream = None;
        self.sync.set_written(0);
        self.sync.set_consumed(0);
        self.bus
            .publish(Event::StateChanged(TransportState::Stopped));
        self.bus.publish(Event::PositionChanged {
            position_ms: self.position_ms(),
            duration_ms: self.duration_ms,
        });
        self.report_output_error(&err);
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    /// Never mistake a ring drain or an unsupported flush for an endpoint
    /// reset. Closing is fail-closed, not a claim to retract hardware audio.
    fn flush_output(&mut self) -> Result<()> {
        if let Some(message) = &self.flush_failure {
            return Err(Error::Output(message.clone()));
        }
        if let Err(err) = self.output.flush() {
            let frozen_timeline = self.timeline_frames();
            self.sync.set_active(false);
            let _ = self.output.pause();
            let closed = self.output.close();
            // close is not required to retain the device clock. Preserve the
            // timeline across it even on an implementation that resets it.
            self.timeline_origin = frozen_timeline;
            self.clock_origin = self.output.played_frames();
            let mut message = format!(
                "{}: output flush failed: {err}; output closed; recreate the engine/output before playback",
                self.output.name()
            );
            if let Err(close_err) = closed {
                message.push_str(&format!("; closing output also failed: {close_err}"));
            }
            self.flush_failure = Some(message.clone());
            return Err(Error::Output(message));
        }
        Ok(())
    }

    fn report_output_error(&mut self, err: &Error) {
        let info = ErrorInfo {
            kind: ErrorKind::Output,
            message: err.to_string(),
        };
        self.error = Some(info.clone());
        self.bus.publish(Event::Error(info));
    }

    fn start_run(&mut self) -> Result<()> {
        if let Some(message) = &self.flush_failure {
            return Err(Error::Output(message.clone()));
        }
        if let Some(err) = self.output.stream_error() {
            let info = ErrorInfo {
                kind: ErrorKind::Output,
                message: err.to_string(),
            };
            self.error = Some(info.clone());
            self.bus.publish(Event::Error(info));
            return Err(err);
        }
        let _ = self.join_decode();
        // S15: the engine owns the shuffle plan; make it coherent with the
        // queue and the current track before the run starts.
        self.ensure_shuffle_plan();
        // With shuffle on and no track selected, playback starts at the
        // first entry of the permutation (S15 §1.1: the permutation is
        // generated when playback starts with shuffle on).
        let index = if self.shuffle && self.current.is_none() && !self.shuffle_plan.is_empty() {
            self.shuffle_plan[0] as u32
        } else {
            self.current.unwrap_or(0)
        };
        if index as usize >= self.queue.len() {
            return Err(Error::NotFound(format!("queue index {index}")));
        }
        self.current = Some(index);
        let start_at = self.start_at.take();
        self.clock_origin = self.output.played_frames();
        // A run without an initial seek starts at zero. With one, keep the
        // target that `seek_frames` published while stopped, so the position
        // does not dip to zero between `play` and the decode thread's `Seeked`
        // (which re-bases the clock and reports the true position a moment
        // later). See the F1 finding in docs/tasks/A3-engine-lifecycle-review.md.
        self.timeline_origin = start_at.as_ref().map(|(frames, _)| *frames).unwrap_or(0);
        self.track_start = 0;
        self.end_of_stream = None;
        self.pending_seek = start_at.map(|(_, seq)| PendingSeek { seq });
        self.sync.set_written(0);
        // Reset the consumer-side counter alongside written so the invariant
        // consumed <= written holds for the new run. The render callback will
        // update it as it reads from the ring.
        self.sync.set_consumed(0);
        // A8: what a stopped run left in the ring must not reach the device.
        // Bump the epoch after the counter resets, so the drain aligns
        // `consumed` with this run's `written` (0); the decode thread waits for
        // the drain and the re-base before its first write. A seek of the
        // previous run that was never drained needs the same, or its drain
        // would drop this run's first frames.
        let stale_frames = self
            .producer
            .as_ref()
            .map_or(0, |producer| self.ring_occupancy(producer));
        let start_epoch = if stale_frames > 0 || self.sync.is_drain_pending() {
            let epoch = self.sync.request_epoch();
            self.start_drain = Some(StartDrain {
                epoch,
                stale_frames,
            });
            Some(epoch)
        } else {
            self.start_drain = None;
            None
        };
        let ring = self
            .producer
            .take()
            .ok_or_else(|| Error::Other("the ring producer was lost".into()))?;

        let (ctl_tx, ctl_rx) = std::sync::mpsc::channel::<DecodeMsg>();
        let (out_tx, out_rx) = std::sync::mpsc::channel::<DecodeOut>();
        let cfg = Config {
            registry: self.registry.clone(),
            queue: self.queue.clone(),
            device: self.device,
            sync: Arc::clone(&self.sync),
            vis_enabled: self.cfg.vis_enabled,
            vis_capacity: self.cfg.vis_capacity,
            stop_after_current: self.stop_after_current,
            replay_gain: Arc::clone(&self.replaygain_control),
            start_epoch,
            shuffle: self.shuffle,
            repeat: self.repeat,
            shuffle_seed: Some(self.shuffle_seed),
            shuffle_plan: self.shuffle_plan.clone(),
            prev_shuffle_plan: self.prev_shuffle_plan.clone(),
        };
        self.shuffle_seed = self.shuffle_seed.wrapping_add(1);
        // The equalizer runs first, on the decoded PCM (see the module docs).
        // A fresh stage per run starts from clean filter history; the shared
        // control carries the settings across runs.
        let mut dsp = std::mem::take(&mut self.dsp);
        dsp.insert(
            0,
            Box::new(Equalizer::with_control(Arc::clone(&self.eq_control))),
        );
        // The limiter runs last, after all installed stages (the order is:
        // equalizer, installed stages, limiter). It must see everything earlier
        // stages produced and is the stage closest to the output (see module docs).
        // A fresh stage per run starts from clean filter history; the shared
        // control carries the on/off state across runs.
        dsp.push(Box::new(Limiter::with_control(Arc::clone(
            &self.limiter_control,
        ))));
        let (handle, vis_rx) = spawn(cfg, dsp, ring, index, start_at, ctl_rx, out_tx);
        self.to_decode = Some(ctl_tx);
        self.from_decode = Some(out_rx);
        self.handle = Some(handle);
        self.vis_rx = Some(vis_rx);

        if !self.started {
            let render = self
                .render
                .take()
                .ok_or_else(|| Error::Output("render callback missing".into()))?;
            self.output.start(render)?;
            self.started = true;
        } else {
            self.output.resume()?;
        }
        self.sync.set_active(true);
        self.transport = TransportState::Playing;
        self.publish_state();
        Ok(())
    }

    fn send(&mut self, msg: DecodeMsg) {
        if let Some(tx) = self.to_decode.as_ref()
            && tx.send(msg).is_err()
        {
            self.to_decode = None;
        }
    }

    /// Join the decode thread and take the ring producer and DSP chain back.
    /// Returns a track start the decode thread queued but the engine never
    /// processed, held instead of discarded (M25 §1.1, A25, M30): the output
    /// failure teardown (`handle_output_error`) delivers it, so a start that
    /// already happened survives the failure. A DSP-stage failure needs no
    /// delivery — RFC 0014 queues `TrackStarted` ahead of the error on the
    /// decode thread's own report queue, so the start is published first.
    /// Every other caller discards it — the run it belongs to is over, and the
    /// next run publishes its own start.
    fn join_decode(&mut self) -> Option<DecodeOut> {
        // A thread waiting for a ring drain must not block the join; nothing is
        // consuming the ring at this point anyway.
        self.sync.set_active(false);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        // Drain any remaining messages from the decode thread. If `Returned`
        // was not handled in `poll`, recover both the ring producer and the
        // installed DSP chain here. On shutdown (`Drop`), this ensures that
        // stages are cleanly retrieved and dropped in order with the engine.
        // A queued `TrackStarted` is held for the caller: the last one wins,
        // naming the track the decode thread had open when the run ended.
        let mut held_start = None;
        if let Some(rx) = self.from_decode.take() {
            while let Ok(out) = rx.try_recv() {
                match out {
                    DecodeOut::Returned { producer, dsp } => {
                        self.producer = Some(producer);
                        self.dsp = dsp;
                    }
                    start @ DecodeOut::TrackStarted { .. } => {
                        held_start = Some(start);
                    }
                    // A plan the worker generated itself is the plan it ended
                    // on: adopt it so the engine's published plan stays
                    // coherent for stopped navigation and the next run (S15).
                    DecodeOut::ShufflePlanned { plan, prev_plan }
                        if plan.len() == self.queue.len() =>
                    {
                        self.shuffle_plan = plan;
                        self.prev_shuffle_plan = prev_plan;
                    }
                    _ => {}
                }
            }
        }
        self.to_decode = None;
        held_start
    }

    fn publish_state(&self) {
        self.bus.publish(Event::StateChanged(self.transport));
    }

    fn publish_position(&mut self, force: bool) {
        let due = self
            .last_position_event
            .map(|t| t.elapsed() >= self.cfg.position_interval)
            .unwrap_or(true);
        if !force && !due {
            return;
        }
        self.last_position_event = Some(Instant::now());
        self.bus.publish(Event::PositionChanged {
            position_ms: self.position_ms(),
            duration_ms: self.duration_ms,
        });
    }
}

/// Clamp an equalizer gain to the engine's range; `None` for a non-finite
/// gain, which cannot be applied honestly.
fn clamp_gain(gain_db: f32) -> Option<f32> {
    gain_db
        .is_finite()
        .then(|| gain_db.clamp(-EqState::MAX_GAIN_DB, EqState::MAX_GAIN_DB))
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.send(DecodeMsg::Quit);
        let _ = self.join_decode();
        let _ = self.output.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::audio::{
        AudioSpec, Confidence, Decoder, DecoderFactory, DecoderRegistry, MediaSource, ProbeHint,
    };
    use rustamp_core::bus::EventBus;
    use rustamp_core::error::{ErrorKind, Result as CoreResult};
    use rustamp_core::event::Event;
    use rustamp_output::{NullClock, NullOutput};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    const SPEC: AudioSpec = AudioSpec::new(44100, 2);

    struct PatternDecoder {
        total: u64,
        pos: u64,
    }

    impl Decoder for PatternDecoder {
        fn spec(&self) -> AudioSpec {
            SPEC
        }

        fn read(&mut self, out: &mut [f32]) -> CoreResult<usize> {
            let ch = usize::from(SPEC.channels);
            let want = out.len() / ch;
            let n = want.min(self.total.saturating_sub(self.pos) as usize);
            out[..n * ch].fill(0.1);
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
    }

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
                    return Err(rustamp_core::error::Error::Unsupported(
                        "fake decoder takes paths only",
                    ));
                }
            };
            let stem = name.rsplit(['/', '\\']).next().unwrap_or(name.as_str());
            let stem = stem.strip_suffix(".tst").unwrap_or(stem);
            let frames = stem
                .strip_prefix("track")
                .and_then(|rest| rest.split_once('_'))
                .map(|(_, f)| f.parse().unwrap_or(3000))
                .unwrap_or(3000);
            Ok(Box::new(PatternDecoder {
                total: frames,
                pos: 0,
            }))
        }
    }

    struct TestHarness {
        engine: Engine,
        clock: NullClock,
        events: rustamp_core::bus::EventReceiver,
        _dir: tempfile::TempDir,
    }

    impl TestHarness {
        fn new(names: &[&str]) -> Self {
            let dir = tempfile::tempdir().expect("temp dir");
            let (output, clock) = NullOutput::pair();
            let bus = EventBus::new();
            let events = bus.subscribe();
            let mut registry = DecoderRegistry::new();
            registry.register(Arc::new(PatternFactory));
            let mut engine = Engine::new(registry, Box::new(output), bus, EngineConfig::default())
                .expect("engine construction");
            engine.set_limiter_enabled(false);
            let queue = names
                .iter()
                .map(|name| {
                    let path = dir.path().join(name);
                    std::fs::write(&path, b"").expect("placeholder track");
                    TrackSource::path(path)
                })
                .collect();
            engine.set_queue(queue);
            TestHarness {
                engine,
                clock,
                events,
                _dir: dir,
            }
        }

        fn drain_events(&mut self) -> Vec<Event> {
            self.events.try_iter().collect()
        }

        fn wait_pumping(
            &mut self,
            mut cond: impl FnMut(&Engine) -> bool,
            timeout: Duration,
        ) -> bool {
            let deadline = std::time::Instant::now() + timeout;
            loop {
                self.engine.poll().expect("poll");
                if cond(&self.engine) {
                    return true;
                }
                if std::time::Instant::now() >= deadline {
                    return false;
                }
                self.clock.pump(256);
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    #[test]
    fn engine_publishes_the_start_when_output_fails_right_after_it() {
        let wait = Duration::from_secs(5);
        let mut h = TestHarness::new(&["track7_2048.tst", "track2_44100.tst"]);

        // A first run publishes the first track's duration and ends before
        // the second track opens ("stop after current"): the stale duration
        // the erasure would leave behind is exactly this one.
        h.engine.toggle_stop_after_current().unwrap();
        h.engine.play_index(0).unwrap();
        assert!(
            h.wait_pumping(|e| e.duration_frames().is_some(), wait),
            "the first track must start"
        );
        assert_eq!(h.engine.duration_frames(), Some(2048));
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, wait),
            "the first run must finish after its track"
        );
        assert_eq!(
            h.engine.duration_frames(),
            Some(2048),
            "the second track must not have started"
        );

        // The second track starts.
        h.engine.play_index(1).unwrap();
        assert_eq!(h.engine.current_index(), Some(1));

        // Wait until the decode thread has queued TrackStarted for track 1.
        // Once the decode thread begins writing into the ring, prepare_track
        // has definitely sent TrackStarted into the from_decode channel.
        let start_queued = {
            let deadline = std::time::Instant::now() + wait;
            loop {
                let (written, _) = h.engine.sync.counters();
                if written > 0 {
                    break true;
                }
                if std::time::Instant::now() >= deadline {
                    break false;
                }
                thread::sleep(Duration::from_millis(1));
            }
        };
        assert!(start_queued, "the decode thread must begin writing track 1");

        // The output device now fails mid-run before Engine::poll processes the queue.
        h.clock.fail();

        // One poll handles the failure: poll checks stream_error before
        // from_decode. The failure teardown must deliver the held start.
        h.engine.poll().unwrap();

        assert_eq!(
            h.engine.duration_frames(),
            Some(44_100),
            "the reported duration must be the started track's, not the previous track's"
        );
        assert_eq!(h.engine.current_index(), Some(1));
        assert_eq!(h.engine.transport(), TransportState::Stopped);
        assert_eq!(
            h.engine.position_frames(),
            0,
            "the timeline must freeze at the failure (nothing was pumped on track 1)"
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
            .expect("an output failure must publish an error event");
        assert_eq!(error.kind, ErrorKind::Output);
        assert!(
            error.message.contains("simulated device failure"),
            "{error:?}"
        );
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
    }

    #[test]
    fn stop_does_not_advance_to_prefetched_next_track_during_gapless() {
        // The stall escape follows the M10/M24 rule: generous enough that
        // only a real stall reaches it (the 5 s bounds this module carried
        // before are the flake class M24 proved in the test guest, and the
        // M29 liveness sweep never reached this private test module).
        let wait = Duration::from_secs(30);
        // Track 0 is longer than the ring (16384 frames), so the decode
        // thread blocks mid-track while the device has played only part of
        // it. That is the window in which track 0's start is published and
        // track 1's start is still queued (see the single poll below).
        let track0: u64 = 44_100;
        let mut h = TestHarness::new(&["track1_44100.tst", "track2_44100.tst"]);

        h.engine.play_index(0).unwrap();

        // Wait (without polling) until the decode thread writes: its first
        // write comes after prepare_track queued TrackStarted for track 0,
        // and the ring still holds the rest of track 0, so the worker cannot
        // have opened track 1 yet.
        let track0_started = {
            let deadline = std::time::Instant::now() + wait;
            loop {
                let (written, _) = h.engine.sync.counters();
                if written > 0 {
                    break true;
                }
                if std::time::Instant::now() >= deadline {
                    break false;
                }
                thread::sleep(Duration::from_millis(1));
            }
        };
        assert!(track0_started, "the decode thread must write track 0");

        // Publish track 0's start with one poll. This poll cannot consume
        // track 1's start: the worker is still mid-track-0 (the ring blocks
        // it long before track 0's 44100 frames are written), so TrackStarted
        // for track 1 is not queued yet.
        h.engine.poll().unwrap();
        assert_eq!(h.engine.duration_frames(), Some(track0));
        assert_eq!(h.engine.current_index(), Some(0));

        // Pump until the decode thread has finished track 0 and prefetched
        // into track 1 (written frames exceeding track 0's length): the
        // worker has then queued TrackStarted for track 1. No polling here —
        // the queued start must stay queued for the stop below.
        let prefetched = {
            let deadline = std::time::Instant::now() + wait;
            loop {
                h.clock.pump(256);
                let (written, _) = h.engine.sync.counters();
                if written > track0 {
                    break true;
                }
                if std::time::Instant::now() >= deadline {
                    break false;
                }
                thread::sleep(Duration::from_millis(1));
            }
        };
        assert!(prefetched, "decode thread should prefetch into track 1");

        // The user stops playback while the device has only partially played
        // track 0 (the ring still holds the rest of it).
        assert!(h.clock.frames() < track0);

        h.engine.stop().unwrap();

        assert_eq!(h.engine.transport(), TransportState::Stopped);
        // Current index must stay at track 0 (the track that was playing), not jump to track 1!
        assert_eq!(h.engine.current_index(), Some(0));

        let events = h.drain_events();
        // Event::TrackChanged must NOT have been emitted for track 1.
        let track1_changed = events
            .iter()
            .any(|e| matches!(e, Event::TrackChanged { index: Some(1), .. }));
        assert!(
            !track1_changed,
            "stop must not publish TrackChanged for prefetched track 1"
        );
    }
}
