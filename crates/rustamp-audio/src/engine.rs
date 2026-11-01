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
//! The decode thread is joined with a budget (`join_decode`,
//! `SHUTDOWN_JOIN_TIMEOUT`), never unbounded: it can block in a source `read`
//! forever, and `stop()`/`Drop` must not hang on it. A join that misses its
//! budget leaves the thread alone — it still owns the ring producer and the
//! DSP chain — and the run ends loudly instead of being published as a clean
//! stop.
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
//! plus a preamp in dB). The decode thread reads it when it opens a track
//! and again after every successful seek, and scales the raw decoder output
//! before the DSP chain, so the limiter (-0.3 dBFS ceiling) and the
//! converter's final ±1.0 ceiling still bound the normalized signal (see
//! [`crate::replaygain`] for the placement and the policy). The mode defaults
//! to disabled — loudness never changes until the user asks — and a track
//! without a usable tag plays unadjusted whatever the settings say.
//!
//! The settings live engine-side, like the limiter switch: [`PlayerState`]
//! has no ReplayGain field, and changing the wire contract requires an RFC.
//! There is no [`Command`] for them either (the command enum is a frozen
//! `rustamp-core` contract); the application layer calls the setters below
//! directly, as it does for [`Engine::set_limiter_enabled`].
//!
//! # The output fade (own design; S18 §2.3's constraint)
//!
//! The engine owns a lock-free [`FadeControl`] that drives the output fade
//! stage ([`rustamp_dsp::Fade`]) in the **render callback**, the consumer side
//! of the ring:
//!
//! ```text
//! decoder -> preamp -> 10 bands -> [installed DspStage]* -> limiter -> vis tap
//!         -> FormatConverter -> ring -> volume/balance -> fade -> device
//! ```
//!
//! The fade runs there and not in the decode chain because of the ring: the
//! decode thread writes about 370 ms ahead of the device, so a fade applied
//! there would be heard only after that backlog had been played at full level.
//! On the consumer side the ramp covers exactly the frames the device plays,
//! so its duration is the requested one.
//!
//! **Own choices** (the reference player's behaviour is unmeasured; S18
//! describes the reference's **output plug-in**, not the player, and leaves
//! the player's own fade open — S18 §7, item 7 — so nothing here may be
//! cited as 2.95 behaviour):
//!
//! * the ramp law is **equal power** (`g = sqrt(from^2 + (to^2 - from^2) * t)`),
//!   so the perceived loudness falls evenly;
//! * the duration comes from [`EngineConfig::fade_ms`], whose default is the
//!   frozen [`AudioConfig::fadeout_ms`](rustamp_core::config::AudioConfig::fadeout_ms)
//!   default (500 ms);
//! * **only** [`Command::StopWithFadeout`] fades. A plain [`Command::Stop`]
//!   and a [`Command::Pause`] stay immediate, so the existing stop semantics
//!   are untouched;
//! * a transport that is not playing, or a duration of `0` ms, stops at once:
//!   there is nothing to fade;
//! * a [`Command::Play`] during a running fade-out **cancels** it and ramps
//!   the gain back up, so the fade is not a one-way door.
//!
//! The visualization tap sits before the ring, so it shows the un-faded
//! signal — the same split the volume/balance gain has.
//!
//! # The fade against the buffer (S18 §2.3)
//!
//! The audited spec S18 records the constraint the reference's DirectSound
//! output plug-in states in its own words: *"Some fade times are bigger than
//! buffer length; in order to get what you want, please increase buffer size
//! to `<N>` ms."* The spec marks the consequence as a **reading**: a fade
//! longer than the buffer cannot be realised inside that buffer, and the
//! plug-in answers the conflict by naming the buffer size it would need — it
//! does not shorten the fade silently.
//!
//! Our pipeline owns two buffers, and the engine resolves the conflict the
//! same way the reference does — **by saying so**:
//!
//! * the **device's own buffer** ([`OutputFormat::buffer_frames`], the buffer
//!   the device actually got) and the **engine's ring**
//!   ([`EngineConfig::ring_frames`], the buffer the audio waits in) are both
//!   compared against the frames the requested fade needs at the device's
//!   rate;
//! * **the fade is never shortened.** The ramp is counted in frames of the
//!   output, not in buffers: it crosses the device's callback boundary as
//!   often as it needs to, and the ring that feeds it is refilled by the
//!   decode thread while the ramp runs (measured: a 500 ms fade on a 371 ms
//!   ring reaches silence at exactly 500 ms, and the ring is refilled while
//!   the ramp runs);
//! * **the conflict is reported.** [`Engine::take_fade_note`] hands the caller
//!   a [`FadeNote`] with the requested milliseconds, the available
//!   milliseconds and the buffer length that would hold the whole fade. The
//!   caller shows it to the user; the player surfaces it as a note.
//!
//! See [`crate::fade_budget`] for the rule and for what it accepts in
//! exchange (a ramp that outlasts the audio ends above zero).
//!
//! # The crossfade (own choice; S18 §7, item 9)
//!
//! [`EngineConfig::crossfade_ms`] overlaps the end of a track with the start of
//! the next; `0`, the default, leaves the gapless seam as it is. See
//! [`crate::crossfade`] for the mix point and the rules.
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
//! on every path (A14; finding **F2** of
//! `docs/tasks/W14-wave-review.md` closed the equal-rate path's gap), so the
//! ring — and hence the device — never sees a sample above full scale
//! however the equalizer and the limiter are set. Volume and balance after
//! the converter only attenuate.
//! Because [`PlayerState`] has no field for the limiter (and changing the wire
//! contract requires an RFC), the control remains engine-side via
//! [`Engine::limiter_enabled`], [`Engine::set_limiter_enabled`], and
//! [`Engine::limiter_control`].
//!
//! # The playback order (S15)
//!
//! Shuffle and repeat are **not** implemented here: the traversal order lives
//! in one place, the [`PlaybackOrder`] model of `rustamp-playlist` (S15 §1–§4).
//! The model owns the permutation, the repeat boundaries and the mid-run
//! toggles; the engine drives it and publishes what it says.
//!
//! One instance is shared by the engine and the decode thread: the thread
//! that plays a track and the thread that answers "which track comes next"
//! must follow the *same* permutation, and a second copy of the model would
//! be a second, diverging truth. The lock is held only for the few
//! instructions a model call takes — never across a blocking operation — so
//! the audio path stays free of contention.
//!
//! What the engine does with the model:
//!
//! * [`Engine::set_queue`] and [`Engine::set_current_index`] keep it in sync
//!   with the queue (a mutation invalidates the permutation, S15 §4).
//! * [`Engine::next_track`], [`Engine::prev_track`] and [`Engine::skip_by`]
//!   ask the model for the next, previous or relative track instead of
//!   computing an index of their own.
//! * [`Engine::set_shuffle`] and [`Engine::set_repeat`] change the modes in
//!   the model; the published state ([`PlayerState::shuffle`],
//!   [`PlayerState::repeat`]) is read back from it, so the lights and the
//!   traversal cannot disagree.
//!
//! The decode thread follows the same model: it asks it for the next track at
//! every track boundary, which is what makes the gapless seam follow the
//! permutation.
//!
//! # Not in this milestone
//!
//! out_ds-style fades and prebuffering (S18), the S08
//! vis derivation, and the playlist model (the engine only holds a queue of
//! re-openable sources and reports indices). See the task card and the report
//! for task card A1.

use std::collections::VecDeque;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use rtrb::{Consumer, Producer, RingBuffer};
use rustamp_core::audio::{
    AudioSpec, DecoderRegistry, DspStage, OutputDevice, OutputFormat, RenderFn,
};
use rustamp_core::bus::{CommandReceiver, EventBus};
use rustamp_core::command::Command;
use rustamp_core::config::AudioConfig;
use rustamp_core::error::{Error, ErrorInfo, ErrorKind, Result};
use rustamp_core::event::Event;
use rustamp_core::meta::TrackMetadata;
use rustamp_core::state::{EQ_BANDS, EqState, PlayerState, StreamInfo, TransportState};
use rustamp_core::vis::VisFrame;
use rustamp_decoders::{HttpProgress, HttpProgressSnapshot};
use rustamp_dsp::{EqControl, Equalizer, FadeControl, Limiter, LimiterControl};
use rustamp_playlist::PlaybackOrder;
use rustamp_playlist::rng::DEFAULT_SEED;

use crate::crossfade::MAX_CROSSFADE_MS;
use crate::decode::{Config, DecodeMsg, DecodeOut, DecodeThread, TrackSource, spawn};
use crate::fade_budget::{FadeBudget, FadeBuffer, FadeNote};
use crate::replaygain::{IntoReplayGainValue, ReplayGainControl, ReplayGainMode};
use crate::ring::{GainState, RingSync, make_render};

/// The fade reports the engine keeps for the caller, at most.
///
/// The caller is expected to take them (see [`Engine::take_fade_note`]); the
/// cap keeps an engine whose caller never takes them from growing without
/// end. The oldest report is dropped when the cap is reached: a later report
/// is about the same configuration and carries the same numbers.
const MAX_FADE_NOTES: usize = 8;

/// How long [`Engine::join_decode`] waits for the decode thread to finish
/// before it treats it as stuck.
///
/// The decode thread runs a source `read` that can block indefinitely (a
/// stalled network read, a hung file system), so an unbounded join would hang
/// the caller — `stop()` and `Drop` included. Two seconds is far more than a
/// well-behaved thread needs to leave its loop (it polls its command channel
/// every few milliseconds), and short enough that a shutdown is not held up by
/// a dead source.
const SHUTDOWN_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

/// How long the natural end waits for an output device that still holds audio
/// after the ring is empty (a classic output plug-in buffers seconds of it).
/// Longer than any buffer the host opens a plug-in with, so a device whose clock
/// never arrives cannot hold the player.
const END_DRAIN_GRACE: Duration = Duration::from_secs(3);
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
    /// Duration of the fade-out that [`Command::StopWithFadeout`] runs, in
    /// milliseconds.
    ///
    /// The default is the frozen configuration's own default
    /// ([`AudioConfig::fadeout_ms`](rustamp_core::config::AudioConfig::fadeout_ms)),
    /// so the player setting and the engine agree until the application
    /// passes its configured value in. A duration of `0` stops at once.
    pub fade_ms: u32,
    /// Duration of the crossfade between two tracks, in milliseconds, clamped
    /// to [`MAX_CROSSFADE_MS`]. `0` (the default) is the gapless seam.
    pub crossfade_ms: u32,
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
            fade_ms: AudioConfig::default().fadeout_ms,
            crossfade_ms: AudioConfig::default().crossfade_ms,
        }
    }
}

/// A pending seek, waiting for the decode thread to report it applied.
#[derive(Debug, Clone, Copy)]
struct PendingSeek {
    seq: u32,
    /// Where the seek goes, in milliseconds inside the track.
    target_ms: u64,
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
    /// Where the output fade stage in the render callback takes its requests
    /// from (see the module docs).
    fade_control: Arc<FadeControl>,
    /// A fade-out that has not finished yet: the counter of the request it
    /// waits for. The transport stops when the ramp is over.
    pending_fade_stop: Option<u64>,
    /// When the ring first reached the end of the run while the device still
    /// held audio: the natural end waits for the device, but not without bound.
    end_drain_since: Option<std::time::Instant>,
    /// The fade reports the caller has not taken yet, oldest first (S18
    /// §2.3; see [`crate::fade_budget`]).
    fade_notes: VecDeque<FadeNote>,
    /// The buffer the device actually got, in frames. It is the counterpart
    /// of the reference's `cfg_buf_ms` (S18 §3) and the buffer a fade has to
    /// fit into.
    device_buffer_frames: u32,
    /// Where [`Engine::set_replaygain_mode`] / [`Engine::set_replaygain_preamp_db`]
    /// reaches the decode thread. Snapshotted once per track.
    replaygain_control: Arc<ReplayGainControl>,
    /// The ring's write end, parked here while no run is active.
    producer: Option<Producer<f32>>,

    to_decode: Option<Sender<DecodeMsg>>,
    from_decode: Option<Receiver<DecodeOut>>,
    /// The decode thread of the live run, see [`Engine::join_decode`].
    handle: Option<DecodeThread>,
    /// The last [`Engine::join_decode`] gave up on its thread: the run it
    /// belonged to is half dead, and the caller of the join reports that
    /// (`take_stuck_decode_error`).
    decode_stuck: bool,

    queue: Vec<TrackSource>,
    /// The same queue, as the decode thread reads it.
    shared_queue: Arc<Mutex<Vec<TrackSource>>>,
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
    /// A fatal mid-run stream error ended the run (RFC 0012 §4.3). The device
    /// object itself is still owned here, so the next explicit start gives it a
    /// fresh `close`/`open`/`start` lifecycle (RFC 0012 §4.3 item 8) instead of
    /// refusing forever. This is the engine's own half of the recovery split;
    /// `flush_failure` is the caller's half (see [`Engine::is_flush_failed`]).
    stream_lost: bool,
    /// The S15 playback order: the shuffle permutation and the repeat
    /// boundaries. Shared with the decode thread, which asks it for the next
    /// track (see the module docs).
    order: Arc<Mutex<PlaybackOrder>>,
    /// The queue changed while the order still describes the old one; the
    /// order is brought up to date when the engine owns it again.
    order_stale: bool,
    track_starts: Vec<(u32, u64)>,
    /// The observer of the stream transfer the run is on, if it is on one.
    ///
    /// The decode thread hands it over when it opens a URL, and it is
    /// dropped again when the run ends, when a file plays and when the next
    /// run starts — so a phase of an old transfer never shows (S19 §2). The
    /// observer is a handle, not a copy: the app can read it even after the
    /// open failed, which is when the phase says that the host could not be
    /// reached.
    stream_progress: Option<StreamObserver>,
}

/// The observer of the stream transfer the run is on.
///
/// The index says which track the transfer belongs to, in the queue **the run
/// started with**: the engine's own queue may already be a different one
/// while the run lives, so the index of the run is what a track report is
/// compared against.
#[derive(Debug)]
struct StreamObserver {
    /// The queue index of the track the transfer belongs to.
    index: u32,
    /// The observer of the transfer.
    progress: HttpProgress,
}

/// Ring capacity in samples for `cfg` on a device that granted `device`.
///
/// Shared by [`Engine::new`] and the RFC 0012 reopen, so a device that grants a
/// different channel count on its second open still gets a correctly sized ring.
fn ring_samples_for(cfg: &EngineConfig, device: AudioSpec) -> usize {
    cfg.ring_frames
        .max(256)
        .saturating_mul(usize::from(device.channels.max(1)))
}

/// Why playback is being torn down by [`Engine::handle_output_error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputFailure {
    /// The device latched a fatal mid-run stream error (RFC 0012): this output
    /// is dead and gets a fresh `open()` on the next explicit start, so its
    /// endpoint buffer is not worth flushing now.
    StreamLost,
    /// Something else on the output path failed while the device itself is
    /// still alive — a refused flush on a stopped seek, a decode report that
    /// could not be applied. The flush contract applies unchanged here.
    OnLiveDevice,
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
        let device_buffer_frames = format.buffer_frames;
        let (producer, consumer) = RingBuffer::<f32>::new(ring_samples_for(&cfg, device));
        let sync = Arc::new(RingSync::new());
        let initial_volume = cfg.volume;
        let initial_balance = cfg.balance;
        let gain = Arc::new(GainState::new(initial_volume, initial_balance));
        let fade_control = Arc::new(FadeControl::new());
        let render = make_render(
            consumer,
            Arc::clone(&sync),
            Arc::clone(&gain),
            device,
            Arc::clone(&fade_control),
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
            fade_control,
            pending_fade_stop: None,
            end_drain_since: None,
            fade_notes: VecDeque::new(),
            device_buffer_frames,
            replaygain_control: Arc::new(ReplayGainControl::default()),
            producer: Some(producer),
            to_decode: None,
            from_decode: None,
            handle: None,
            decode_stuck: false,
            queue: Vec::new(),
            shared_queue: Arc::new(Mutex::new(Vec::new())),
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
            stream_lost: false,
            order: Arc::new(Mutex::new(PlaybackOrder::new(
                0,
                false,
                false,
                DEFAULT_SEED,
            ))),
            order_stale: true,
            track_starts: Vec::new(),
            stream_progress: None,
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
    /// The failed stage is then **bypassed** (see [`crate::decode`]): the
    /// engine replaces it with a pass-through, so the next track plays
    /// without it instead of the player dying with the failed chain. The
    /// failure is reported with the stage's name and the frames that were
    /// lost, so the bypass is never silent. Installing a chain again undoes
    /// the bypass — that is how a caller replaces a failed resident or
    /// plugin with a fresh one.
    ///
    /// To run the chain in a resident process (opt-in, RFC 0006 §6), build a
    /// stage with [`resident::DspResidentConfig::open`] (or wrap a session in
    /// [`resident::HostDspChain::new`]) and install it here. The default chain
    /// is empty and never spawns a process.
    pub fn set_dsp_chain(&mut self, dsp: Vec<Box<dyn DspStage>>) {
        self.dsp = dsp;
    }

    /// Replaces the decoders the next run opens its tracks with.
    pub fn set_registry(&mut self, registry: DecoderRegistry) {
        self.registry = registry;
    }

    /// Replace the play queue. Entries must be re-openable (paths or URLs).
    ///
    /// The traversal order follows the queue: a new queue invalidates the
    /// permutation and the cursor (S15 §4), so the plan is regenerated around
    /// the current track.
    ///
    /// A live run follows at once: its decode thread reads the same queue, so
    /// an entry added while a track plays is the next one to play.
    pub fn set_queue(&mut self, queue: Vec<TrackSource>) {
        self.queue = queue;
        *self
            .shared_queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = self.queue.clone();
        // The run's history speaks in the old indices, and only the track that
        // is playing still counts.
        if self.track_starts.len() > 1 {
            self.track_starts.drain(..self.track_starts.len() - 1);
        }
        if let Some(current) = self.current
            && (current as usize) >= self.queue.len()
        {
            self.current = None;
        }
        self.order_stale = true;
        self.sync_order_to_queue();
    }

    /// Bring the traversal order in line with the queue.
    fn sync_order_to_queue(&mut self) {
        if !self.order_stale {
            return;
        }
        self.order_stale = false;
        let len = self.queue.len();
        let mut order = self.order();
        if order.len() != len {
            order.set_len(len);
        } else {
            order.invalidate();
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

    /// Set the current queue index without starting playback.
    ///
    /// The traversal order follows: in shuffle mode the active permutation is
    /// kept and the cursor moves into it, at the track that was selected
    /// (S15 §4). The traversal continues behind that track, and the entries
    /// before it in the permutation are deferred to the next cycle. The
    /// permutation itself is not rebuilt here; a mode toggle or a playlist
    /// mutation rebuilds it.
    ///
    /// The decode thread relies on that: it re-aligns its own place in a
    /// permutation the engine re-planned (S15 §4.1) without disturbing it.
    ///
    /// During a live run this says where the playing track sits after a
    /// playlist edit, so the run's bookkeeping moves with it.
    pub fn set_current_index(&mut self, index: Option<u32>) {
        self.sync_order_to_queue();
        let had_current = self.current.is_some();
        self.current = index.filter(|&i| (i as usize) < self.queue.len());
        self.order()
            .set_current_index(self.current.map(|i| i as usize));
        if let Some(current) = self.current
            && self.to_decode.is_some()
        {
            if let Some(last) = self.track_starts.last_mut() {
                last.0 = current;
            }
            self.send(DecodeMsg::CurrentMoved(current));
        }
        // The current entry is gone: nothing of it may stay on display.
        if self.current.is_none() && had_current && self.transport == TransportState::Stopped {
            self.duration_ms = None;
            self.duration_frames = None;
            self.metadata = TrackMetadata::default();
            self.stream = None;
            self.bus.publish(Event::TrackChanged {
                id: None,
                index: None,
                metadata: None,
                duration_ms: None,
            });
            self.bus.publish(Event::StreamInfoChanged(None));
        }
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

    /// Whether the output was closed after a terminal flush refusal or failure,
    /// requiring the caller to recreate the engine and output before playing again.
    ///
    /// This is the caller's half of the output recovery split. The engine's own
    /// half - a fatal mid-run stream error, which the engine recovers from by
    /// reopening the device it already holds - is [`Engine::is_stream_lost`].
    /// Stream-error teardown does not flush, so it cannot arm this latch.
    /// A later explicit stop/seek still obeys the strict flush contract.
    pub fn is_flush_failed(&self) -> bool {
        self.flush_failure.is_some()
    }

    /// Whether a fatal mid-run stream error ended the run and the output is
    /// waiting for the fresh `open()` that RFC 0012 §4.3 item 8 promises on the
    /// next explicit start.
    ///
    /// Unlike [`Engine::is_flush_failed`] this is **not** terminal and needs no
    /// caller-side recreation: the engine still owns the device object, so
    /// `play()`/`play_index()` close and reopen it. A reopen that fails leaves
    /// this set and reports an [`Error::Output`] naming the device, so the next
    /// explicit start tries again.
    pub fn is_stream_lost(&self) -> bool {
        self.stream_lost
    }

    /// Packets the decoder skipped in the track that ended last (RFC 0002, C1).
    pub fn skipped_packets(&self) -> u64 {
        self.skipped
    }

    /// What the stream transfer is doing right now, if the run is on one.
    ///
    /// The snapshot carries the phase of the exchange, the bytes of the body
    /// that came in and the length the server declared — the facts behind the
    /// status texts of S19 §2. The **rate** is not carried: the caller
    /// derives it from two snapshots and its own clock.
    ///
    /// `None` when the run is not on a stream: a file played, the run ended,
    /// or no stream was opened yet. The observer of a transfer that is over
    /// stays readable until the run ends, so the phase that says how the
    /// stream ended is not lost between two ticks.
    pub fn stream_snapshot(&self) -> Option<HttpProgressSnapshot> {
        self.stream_progress
            .as_ref()
            .map(|observer| observer.progress.snapshot())
    }

    /// Forgets the observer of the stream transfer.
    ///
    /// The rule is one rule with three places: a phase of a transfer that is
    /// over must not show (S19 §2). The run ended, a file played or a new
    /// run began — in all three the observer goes, and the next transfer
    /// hands a fresh one over.
    fn clear_stream_progress(&mut self) {
        self.stream_progress = None;
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
        // A seek while paused only reaches the clock on the resume, but it has
        // happened for whoever looks: Winamp 2.95 shows it at once.
        if self.transport == TransportState::Paused
            && let Some(pending) = self.pending_seek
        {
            return pending.target_ms;
        }
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
        let order = self.order();
        PlayerState {
            transport: self.transport,
            position_ms: self.position_ms(),
            duration_ms: self.duration_ms,
            volume: self.volume,
            balance: self.balance,
            shuffle: order.shuffle(),
            repeat: order.repeat(),
            current_index: self.current,
            current_id: None,
            playlist_len: self.queue.len() as u32,
            stream: self.stream,
            eq: self.eq,
            stop_after_current: self.stop_after_current,
        }
    }

    /// Index of the track currently being heard from the output device, or
    /// `self.current` if not playing.
    pub fn audible_index(&self) -> Option<u32> {
        if self.transport == TransportState::Stopped {
            return self.current;
        }
        let pos = self.sync.consumed();
        for &(idx, start) in self.track_starts.iter().rev() {
            if pos >= start {
                return Some(idx);
            }
        }
        self.current
    }

    /// If repeat was disabled mid-run while a cycle was playing, determine the
    /// timeline end of the currently audible cycle if subsequent cycles were
    /// already prebuffered into `track_starts` (spec S15 §4.3).
    fn current_cycle_end(&self, pos: u64) -> Option<u64> {
        let n = self.queue.len();
        if n == 0 {
            return None;
        }
        let curr_idx = self
            .track_starts
            .iter()
            .rposition(|&(_, start)| pos >= start)?;
        if !self.shuffle() {
            let last_track_index = (n - 1) as u32;
            let final_entry_idx = self.track_starts[curr_idx..]
                .iter()
                .position(|&(idx, _)| idx == last_track_index)
                .map(|offset| curr_idx + offset)?;
            if final_entry_idx + 1 < self.track_starts.len() {
                Some(self.track_starts[final_entry_idx + 1].1)
            } else {
                None
            }
        } else {
            let cycle = curr_idx / n;
            let final_entry_idx = cycle * n + (n - 1);
            if final_entry_idx + 1 < self.track_starts.len() {
                Some(self.track_starts[final_entry_idx + 1].1)
            } else {
                None
            }
        }
    }

    /// The shared playback order (S15).
    ///
    /// The lock is never held across a blocking operation, and a poisoned
    /// lock is recovered rather than propagated: the model is plain data and
    /// a panic in one thread must not disable the traversal for the other.
    fn order(&self) -> MutexGuard<'_, PlaybackOrder> {
        self.order
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Whether shuffle mode is enabled (spec S15).
    pub fn shuffle(&self) -> bool {
        self.order().shuffle()
    }

    /// Whether repeat mode is enabled (spec S15).
    pub fn repeat(&self) -> bool {
        self.order().repeat()
    }

    /// Set shuffle mode enabled or disabled (spec S15).
    ///
    /// Turning shuffle on mid-run re-plans the permutation around the track
    /// that is playing (S15 §4.1): the model puts that track first and
    /// permutes the rest around it. The track itself keeps playing.
    pub fn set_shuffle(&mut self, shuffle: bool) -> Result<()> {
        {
            let mut order = self.order();
            if order.shuffle() == shuffle {
                return Ok(());
            }
            if shuffle {
                // S15 §4.1: the track that is playing becomes the first entry
                // of the new permutation.
                if let Some(anchor) = self.audible_index().or(self.current) {
                    order.set_current_index(Some(anchor as usize));
                }
            }
            order.set_shuffle(shuffle);
        }
        // The decode thread follows the same model and applies the change in
        // the order the commands were queued.
        self.send(DecodeMsg::ModesChanged {
            shuffle,
            repeat: self.repeat(),
        });
        self.bus.publish(Event::ModesChanged {
            shuffle,
            repeat: self.repeat(),
        });
        Ok(())
    }

    /// Toggle shuffle mode (spec S15).
    pub fn toggle_shuffle(&mut self) -> Result<()> {
        self.set_shuffle(!self.shuffle())
    }

    /// Set repeat mode enabled or disabled (spec S15).
    ///
    /// Turning repeat off takes effect at the next queue boundary: the run
    /// then ends with the cycle it is playing instead of starting a new one
    /// (S15 §4.3).
    pub fn set_repeat(&mut self, repeat: bool) -> Result<()> {
        {
            let mut order = self.order();
            if order.repeat() == repeat {
                return Ok(());
            }
            order.set_repeat(repeat);
        }
        if repeat {
            self.end_of_stream = None;
        } else if self.transport == TransportState::Playing {
            let pos = self.sync.consumed();
            if let Some(cycle_end) = self.current_cycle_end(pos) {
                self.end_of_stream = Some(cycle_end);
            }
        }
        // The decode thread follows the same model and applies the change in
        // the order the commands were queued.
        self.send(DecodeMsg::ModesChanged {
            shuffle: self.shuffle(),
            repeat,
        });
        self.bus.publish(Event::ModesChanged {
            shuffle: self.shuffle(),
            repeat,
        });
        Ok(())
    }

    /// Toggle repeat mode (spec S15).
    pub fn toggle_repeat(&mut self) -> Result<()> {
        self.set_repeat(!self.repeat())
    }

    /// Set the PRNG seed for shuffle permutations (spec S15; for tests).
    ///
    /// The seed belongs to the model: the permutation that is published is
    /// the one the seed produces.
    pub fn set_shuffle_seed(&mut self, seed: u64) {
        self.order().set_seed(seed);
    }

    /// The active shuffle permutation: the order the traversal follows.
    pub fn shuffle_plan(&self) -> Vec<usize> {
        self.order().shuffle_plan().to_vec()
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
    /// ([`ReplayGainMode::Off`], [`ReplayGainMode::Track`] or
    /// [`ReplayGainMode::Album`]).
    ///
    /// Takes effect from the next track **or the next seek**: the decode
    /// thread reads the setting when it opens a track and derives the
    /// multiplier again after every successful seek, so a change made
    /// mid-track is heard from the next seek on.
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
    /// previous value). Takes effect from the next track **or the next
    /// seek**, like the mode.
    pub fn set_replaygain_preamp_db(&mut self, preamp_db: f32) {
        self.replaygain_control.set_preamp_db(preamp_db);
    }

    /// Set the ReplayGain mode and the current track's gain/peak values.
    ///
    /// The app calls this when a track starts, passing the values read from
    /// the track's metadata (see `rustamp-meta::replaygain::ReplayGainTags`).
    /// `None` (or a non-finite number) means the tag is absent.
    pub fn set_replaygain<TG, TP, AG, AP>(
        &mut self,
        mode: ReplayGainMode,
        track_gain_db: TG,
        track_peak: TP,
        album_gain_db: AG,
        album_peak: AP,
    ) where
        TG: IntoReplayGainValue,
        TP: IntoReplayGainValue,
        AG: IntoReplayGainValue,
        AP: IntoReplayGainValue,
    {
        self.replaygain_control.set_mode(mode);
        self.replaygain_control.set_track_values(
            track_gain_db.into_value(),
            track_peak.into_value(),
            album_gain_db.into_value(),
            album_peak.into_value(),
        );
    }

    /// The ReplayGain control block, shared with the decode thread.
    pub fn replaygain_control(&self) -> Arc<ReplayGainControl> {
        Arc::clone(&self.replaygain_control)
    }

    /// The crossfade duration in milliseconds; `0` is the gapless seam.
    pub fn crossfade_ms(&self) -> u32 {
        self.cfg.crossfade_ms.min(MAX_CROSSFADE_MS)
    }

    /// Set the crossfade duration in milliseconds (clamped to
    /// [`MAX_CROSSFADE_MS`]).
    ///
    /// A live run follows at once: the next end of a track fades with the new
    /// duration, as far as it can still hold that much of the track back.
    /// Because [`PlayerState`] has no field for it (and there is no
    /// [`Command`] for it), the setting remains engine-side, like the limiter
    /// switch.
    pub fn set_crossfade_ms(&mut self, ms: u32) {
        self.cfg.crossfade_ms = ms.min(MAX_CROSSFADE_MS);
        self.send(DecodeMsg::SetCrossfade(self.cfg.crossfade_ms));
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
            Command::Play => self.play_or_restart(),
            Command::Pause => self.pause(),
            Command::TogglePause => {
                if self.transport == TransportState::Playing {
                    self.pause()
                } else {
                    self.play()
                }
            }
            Command::Stop => self.stop(),
            Command::StopWithFadeout => self.stop_with_fadeout(),
            Command::Next => self.next_track(),
            Command::Prev => self.prev_track(),
            Command::Seek { to_ms } => self.seek_ms(*to_ms),
            Command::SeekRelative { delta_ms } => {
                // A seek that still waits for the device (a paused one) is
                // where the next step starts: the clock has not moved yet.
                let base = self
                    .pending_seek
                    .map_or_else(|| self.position_ms(), |pending| pending.target_ms);
                let target = if *delta_ms < 0 {
                    base.saturating_sub(delta_ms.unsigned_abs())
                } else {
                    base.saturating_add(*delta_ms as u64)
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
        if self.pending_fade_stop.take().is_some() {
            // A fade-out is still running: cancel it and ramp the gain back
            // up instead of stopping. The transport keeps playing.
            self.request_fade(1.0, self.cfg.fade_ms);
        }
        match self.transport {
            TransportState::Playing => Ok(()),
            TransportState::Paused => {
                if let Some(err) = self.output.stream_error() {
                    self.handle_output_error(err, OutputFailure::StreamLost);
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

    /// What `Command::Play` means: [`Engine::play`], and a player that is already
    /// playing starts the track over (S03 §5: "Click the play button: play /
    /// restart"). A running fade-out is only cancelled.
    fn play_or_restart(&mut self) -> Result<()> {
        match self.current {
            Some(current)
                if self.transport == TransportState::Playing
                    && self.pending_fade_stop.is_none() =>
            {
                self.play_index(current)
            }
            _ => self.play(),
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
        // A pending fade-out is over: the transport stops now, whatever the
        // ramp was doing.
        self.pending_fade_stop = None;
        self.send(DecodeMsg::Stop);
        self.sync.set_active(false);
        let _ = self.output.pause();
        let flushed = self.flush_output();
        let _ = self.join_decode();
        if let Some(err) = self.take_stuck_decode_error() {
            // The decode thread did not finish: the run is half dead, and a
            // clean stop over it would claim a teardown that did not happen.
            // End it through the output-failure path, which publishes the
            // error and the stopped state in one place (the frozen position is
            // the position playback reached, not zero).
            self.handle_output_error(err, OutputFailure::OnLiveDevice);
            return Err(stuck_decode_error());
        }
        self.transport = TransportState::Stopped;
        self.clock_origin = self.output.played_frames();
        self.timeline_origin = 0;
        self.track_start = 0;
        self.pending_seek = None;
        self.start_drain = None;
        self.start_at = None;
        self.end_of_stream = None;
        self.track_starts.clear();
        // The run is over: no phase of its stream may show any more.
        self.clear_stream_progress();
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

    /// Stop with a fade-out: the output gain ramps to silence over
    /// [`EngineConfig::fade_ms`] and the transport stops when the ramp is
    /// over (see the module docs).
    ///
    /// A transport that is not playing has nothing to fade, and a duration of
    /// `0` ms means "no fade", so both stop at once. A plain
    /// [`Command::Stop`] and a [`Command::Pause`] stay immediate: the fade is
    /// an own choice for the explicit fade-out command, and the reference
    /// player's behaviour is unmeasured (S18 describes the output plug-in and
    /// leaves the player's own fade open — S18 §7, item 7).
    ///
    /// The fade is **never shortened**: if it does not fit the buffer it is
    /// realised in, the engine keeps the requested duration and reports the
    /// conflict (see [`Engine::take_fade_note`]).
    pub fn stop_with_fadeout(&mut self) -> Result<()> {
        if self.transport != TransportState::Playing || self.cfg.fade_ms == 0 {
            return self.stop();
        }
        self.request_fade(0.0, self.cfg.fade_ms);
        // The counter of the request this stop waits for. The stage reports
        // it back when the ramp is over; `poll` then stops the transport.
        self.pending_fade_stop = Some(self.fade_control.requested());
        Ok(())
    }

    /// Take the oldest fade report the caller has not taken yet.
    ///
    /// A report is queued whenever a requested fade does not fit the buffer
    /// it is realised in (S18 §2.3's constraint; see [`crate::fade_budget`]
    /// for the rule). The engine does not log it and does not publish it as
    /// an error: **the caller shows it to the user**, so the caller takes it
    /// and turns it into a note.
    ///
    /// The wire contract (`rustamp-proto`'s [`Event`](rustamp_core::event::Event))
    /// is frozen, so a new event variant would need an RFC; the report
    /// travels through this accessor instead.
    pub fn take_fade_note(&mut self) -> Option<FadeNote> {
        self.fade_notes.pop_front()
    }

    /// The number of fade reports waiting to be taken.
    pub fn pending_fade_notes(&self) -> usize {
        self.fade_notes.len()
    }

    /// Play the queue entry at `index`.
    ///
    /// The traversal order follows: in shuffle mode the active permutation is
    /// kept and the cursor moves into it, at the track that is about to play
    /// (S15 §4). The traversal continues behind that track.
    pub fn play_index(&mut self, index: u32) -> Result<()> {
        if index as usize >= self.queue.len() {
            return Err(Error::NotFound(format!("queue index {index}")));
        }
        self.sync_order_to_queue();
        if self.transport != TransportState::Stopped {
            self.stop()?;
        }
        self.current = Some(index);
        {
            let mut order = self.order();
            // A navigation step has already moved the model to this track;
            // only an outside choice has to be told about it.
            if order.current_index() != Some(index as usize) {
                order.set_current_index(Some(index as usize));
            }
        }
        self.start_run()
    }

    /// Skip to the next queue entry. Under S15 modes, respects shuffle and repeat.
    ///
    /// While stopped this only moves the current entry (S02 §1).
    pub fn next_track(&mut self) -> Result<()> {
        if self.transport == TransportState::Stopped || self.to_decode.is_none() {
            if self.queue.is_empty() {
                return Ok(());
            }
            self.sync_order_to_queue();
            // The model owns the traversal: it knows the permutation and the
            // repeat boundary (S15 §3.1).
            let next = self.order().next_index();
            if let Some(target) = next {
                return self.move_or_play(target as u32);
            }
            return Ok(());
        }
        // A paused player at the end of the list stays where it is: ending the
        // run here would only show when the user resumes, as a stop instead of
        // the music they asked for.
        if self.transport == TransportState::Paused && !self.order().has_next() {
            return Ok(());
        }
        self.send(DecodeMsg::Next);
        Ok(())
    }

    /// A navigation target outside a run: while stopped the entry changes and
    /// playback does not start (S02 §1), and the marquee shows the new entry.
    fn move_or_play(&mut self, target: u32) -> Result<()> {
        if self.transport != TransportState::Stopped {
            return self.play_index(target);
        }
        self.move_while_stopped(target);
        Ok(())
    }

    /// Makes the first entry current when a stopped player has none, so the
    /// marquee shows it (measured against Winamp 2.95, 2026-10-31).
    pub fn select_first_if_none(&mut self) {
        if self.transport == TransportState::Stopped
            && self.current.is_none()
            && !self.queue.is_empty()
        {
            self.order().set_current_index(Some(0));
            self.move_while_stopped(0);
        }
    }

    /// Make `target` the current entry of a stopped player and say so.
    fn move_while_stopped(&mut self, target: u32) {
        self.current = Some(target);
        self.duration_ms = None;
        self.duration_frames = None;
        self.metadata = TrackMetadata::default();
        self.bus.publish(Event::TrackChanged {
            id: None,
            index: Some(target),
            metadata: None,
            duration_ms: None,
        });
    }

    /// Go back to the previous queue entry. Under S15 modes, respects shuffle and repeat.
    ///
    /// While stopped this only moves the current entry (S02 §1).
    pub fn prev_track(&mut self) -> Result<()> {
        if self.transport == TransportState::Stopped || self.to_decode.is_none() {
            if self.queue.is_empty() {
                return Ok(());
            }
            self.sync_order_to_queue();
            // In shuffle mode this walks back through the permutation, not
            // the playlist (S15 §3.2).
            let prev = self.order().prev_index();
            if let Some(target) = prev {
                return self.move_or_play(target as u32);
            }
            return Ok(());
        }
        self.send(DecodeMsg::Prev);
        Ok(())
    }

    /// Move the current entry by `delta` positions in the order
    /// [`Engine::next_track`]/[`Engine::prev_track`] follow, clamped or wrapped
    /// according to the repeat mode (RFC 0005 `Command::SkipBy`, ids 40195 / 40197).
    ///
    /// The walk is bounded: the model clamps the number of steps to
    /// [`MAX_SKIP_STEPS`](rustamp_playlist::MAX_SKIP_STEPS), so a delta that
    /// arrives over the wire cannot spin the audio thread.
    pub fn skip_by(&mut self, delta: i32) -> Result<()> {
        if delta == 0 || self.queue.is_empty() {
            return Ok(());
        }
        if self.transport == TransportState::Stopped || self.to_decode.is_none() {
            self.sync_order_to_queue();
            // The cursor sits on the first entry when nothing is selected
            // yet, so a relative move is relative to it.
            {
                let mut order = self.order();
                if order.current_index().is_none() {
                    order.set_current_index(Some(0));
                }
            }
            // The model moves by `delta` entries in the active traversal order
            // (S15 §3.3) and reports where that lands.
            let target = self.order().skip_by(delta);
            if let Some(target) = target {
                self.move_while_stopped(target as u32);
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
                self.handle_output_error(
                    Error::Output(err.to_string()),
                    OutputFailure::OnLiveDevice,
                );
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
        let target_ms = if self.device.sample_rate == 0 {
            0
        } else {
            frames.saturating_mul(1000) / u64::from(self.device.sample_rate)
        };
        self.pending_seek = Some(PendingSeek { seq, target_ms });
        self.send(DecodeMsg::Seek {
            to,
            track_frames: frames,
            seq,
        });
        Ok(())
    }

    /// Master volume (`0..=255`).
    pub fn volume(&self) -> u8 {
        self.volume
    }

    /// Pan balance (`-127..=127`).
    pub fn balance(&self) -> i8 {
        self.balance
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
            self.handle_output_error(err, OutputFailure::StreamLost);
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
                self.handle_output_error(err, OutputFailure::OnLiveDevice);
                return Ok(());
            }
        }
        // A fade-out that has run its course stops the transport. The stage
        // reports the finished request from the render callback, so the stop
        // happens exactly when the ramp reached silence.
        if let Some(request) = self.pending_fade_stop
            && self.fade_control.is_finished(request)
        {
            self.pending_fade_stop = None;
            // `stop` reports an output failure itself (see `Engine::stop`).
            let _ = self.stop();
        }
        // Finish the run only when the ring is truly empty: every frame the
        // decode thread wrote has been consumed by the render callback. Using
        // the device clock here is wrong — it advances for silence frames
        // during underruns, which would trigger a premature finish. Using
        // `consumed >= end` instead guarantees we wait until the last
        // real audio frame of the run leaves the ring, not merely until the device clock
        // passes the nominal end time.
        if !self.repeat()
            && self.transport == TransportState::Playing
            && self.end_of_stream.is_none()
        {
            let pos = self.sync.consumed();
            if let Some(cycle_end) = self.current_cycle_end(pos) {
                self.end_of_stream = Some(cycle_end);
            }
        }
        if let Some(end) = self.end_of_stream
            && self.transport == TransportState::Playing
        {
            let (written, consumed) = self.sync.counters();
            let ring_done = written >= end && consumed >= end;
            // The ring being empty is not the end of the sound: an output
            // device - above all a classic output plug-in, which buffers up to
            // seconds of it - still holds part of the run. The device's own
            // clock says when the last frame was played, so the run ends when
            // both agree. A device without a clock counts as playing what it was
            // given, so this asks nothing of it.
            let drained = self.output.played_frames() >= end;
            if ring_done && drained {
                self.finish_run();
                self.end_drain_since = None;
            } else if ring_done || drained {
                // One of the two is at the end and the other has a moment to
                // arrive: the device's buffer, or a ring a flush reset at the
                // end. The bound keeps a device whose clock never arrives from
                // holding the player.
                let since = *self
                    .end_drain_since
                    .get_or_insert_with(std::time::Instant::now);
                if since.elapsed() >= END_DRAIN_GRACE {
                    self.finish_run();
                    self.end_drain_since = None;
                }
            } else {
                self.end_drain_since = None;
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
                self.track_starts.push((index, timeline_start));
                self.current = Some(index);
                // A track that is not the one the observer belongs to played:
                // the phase of the last transfer is stale, and the observer
                // goes with it (S19 §2). The stream's own track keeps its
                // observer: it is the transfer that is running now.
                if self
                    .stream_progress
                    .as_ref()
                    .is_some_and(|observer| observer.index != index)
                {
                    self.stream_progress = None;
                }
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
            DecodeOut::StreamStarted { index, progress } => {
                // The observer of the transfer the decode thread is running:
                // the caller reads what it is doing out of it (S19 §2).
                self.stream_progress = Some(StreamObserver { index, progress });
            }
            DecodeOut::EndOfStream { timeline_end } => {
                self.end_of_stream = Some(match self.end_of_stream {
                    Some(existing) => existing.min(timeline_end),
                    None => timeline_end,
                });
            }
            DecodeOut::EndOfStreamRevoked => {
                self.end_of_stream = None;
            }
            DecodeOut::TrackSkipped { index } => {
                self.current = Some(index);
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
        // The run ended by itself: a pending fade-out has nothing left to
        // fade.
        self.pending_fade_stop = None;
        if let Some(end) = frozen_end {
            for &(idx, start) in self.track_starts.iter().rev() {
                if end > start || (end == start && start == 0) {
                    self.current = Some(idx);
                    break;
                }
            }
        }
        self.track_starts.clear();
        self.sync.set_active(false);
        let _ = self.output.pause();
        let flushed = self.flush_output();
        let _ = self.join_decode();
        let stuck = self.take_stuck_decode_error();
        self.transport = TransportState::Stopped;
        // The run is over: no phase of its stream may show any more, unless
        // nothing played and that phase is what says why (S19 §2).
        if self.sync.counters().0 > 0 {
            self.clear_stream_progress();
        }
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
        // exhausted — the entry that just ended is the last one of the
        // traversal — so arming the mode on the last track still ends the
        // playlist. The RFC leaves the exact interaction open (it rides on S02
        // open question 5); this is the implemented reading.
        let queue_exhausted =
            !self.repeat() && (!self.stop_after_current || self.traversal_exhausted());
        if queue_exhausted {
            self.bus.publish(Event::PlaylistEnded);
        }
        if let Some(err) = stuck {
            // The run ended while its decode thread was stuck in the source:
            // say so, on top of the stopped state, instead of letting the end
            // of the stream look clean.
            self.report_output_error(&err);
        }
        if let Err(err) = flushed {
            self.report_output_error(&err);
        }
    }

    /// Whether the traversal has reached the end of the queue.
    ///
    /// The end of the traversal belongs to the order (S15 §2), not to the
    /// index of the entry that just ended: under shuffle the last entry of a
    /// cycle is the last entry of the active permutation, which is not the
    /// entry with the highest index. A run that ends anywhere else did not
    /// exhaust the queue, whatever the index of the entry that just ended
    /// is — the halt of the "stop after current track" mode (RFC 0005 O8) is
    /// a stop, not the end of the playlist.
    fn traversal_exhausted(&self) -> bool {
        let Some(current) = self.current else {
            return true;
        };
        let order = self.order();
        if order.shuffle() {
            match order.shuffle_plan().last() {
                Some(&last) => current as usize == last,
                None => true,
            }
        } else {
            current as usize + 1 >= self.queue.len()
        }
    }

    /// Tear down playback after a fatal mid-run output device failure (RFC 0012).
    ///
    /// Stops decoding immediately, pauses the device, transitions
    /// transport to [`TransportState::Stopped`], freezes the timeline position,
    /// and publishes [`Event::StateChanged`], [`Event::PositionChanged`] and
    /// [`Event::Error`].
    ///
    /// A track start the decode thread already reported is published before
    /// the freeze (M25 §1.1, A25, M30): the join below holds the queued
    /// `TrackStarted` instead of discarding it, and delivering it here keeps
    /// the window on the track that actually started — its duration, not the
    /// previous track's.
    ///
    /// `cause` decides what happens to the output. [`OutputFailure::StreamLost`]
    /// arms the RFC 0012 §4.3 item 8 reopen and omits flushing, as §4.3
    /// specifies. Otherwise an unnecessary flush refusal would trigger caller
    /// recreation and lose the frozen position. [`OutputFailure::OnLiveDevice`]
    /// still flushes; a refusal remains terminal, exactly as before.
    fn handle_output_error(&mut self, err: Error, cause: OutputFailure) {
        let frozen_timeline = self.timeline_frames();
        if cause == OutputFailure::StreamLost {
            self.stream_lost = true;
        }
        self.send(DecodeMsg::Stop);
        self.sync.set_active(false);
        let _ = self.output.pause();
        if cause == OutputFailure::OnLiveDevice {
            let _ = self.flush_output();
        }
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
        self.pending_fade_stop = None;
        self.start_drain = None;
        self.end_of_stream = None;
        // The run is over: no phase of its stream may show any more.
        self.clear_stream_progress();
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

    /// Give the output the fresh lifecycle RFC 0012 §4.3 item 8 promises after a
    /// fatal mid-run stream error: `close()`, `open()`, and a rebuilt ring and
    /// render callback for the following `start()`.
    ///
    /// The engine can do this without a device factory because it still owns
    /// the very same [`OutputDevice`] object — that is what separates this from
    /// a terminal flush refusal, where the output has to be rebuilt from a
    /// device descriptor the engine does not have and the caller does
    /// (`crates/rustamp-core/src/audio.rs` `OutputDevice::flush`, W22).
    ///
    /// `close()` destroys the render callback the dead stream owned and takes
    /// the ring's consumer half with it, so both halves are made anew here.
    /// Both real backends release their stream before returning the old latched
    /// error from `close()`. Attempt `open()` even then; its result decides
    /// whether a new lifecycle exists. Keep the retry armed until `start()`
    /// succeeds, since cpal creates its actual stream there.
    ///
    /// A device that grants a different format on the second open is adopted:
    /// `self.device` becomes the granted spec, the ring is sized for it, and
    /// the next run's decode thread converts to it like any other run. Rescale
    /// frame-based state so a failed start retains the frozen time and an
    /// explicit stopped seek keeps its target under the new sample rate.
    fn reopen_output(&mut self) -> Result<()> {
        let frozen_timeline = self.timeline_frames();
        let _ = self.output.close();
        self.started = false;
        let opened = self.output.open(self.cfg.spec);
        // Even a failed open can reset the backend's clock (WASAPI does).
        // Preserve the snapshot until a new run actually starts.
        self.timeline_origin = frozen_timeline;
        self.clock_origin = self.output.played_frames();
        let format = opened?;
        let old_rate = u64::from(self.device.sample_rate);
        let new_rate = u64::from(format.spec.sample_rate);
        let rescale = |frames: u64| frames.saturating_mul(new_rate) / old_rate;
        self.timeline_origin = rescale(self.timeline_origin);
        self.track_start = rescale(self.track_start);
        self.duration_frames = self.duration_frames.map(rescale);
        self.start_at = self.start_at.map(|(frames, seq)| (rescale(frames), seq));
        self.device = format.spec;
        // The device may grant another buffer on its second open: the fade
        // budget follows the buffer the device actually got.
        self.device_buffer_frames = format.buffer_frames;
        let (producer, consumer) = RingBuffer::<f32>::new(ring_samples_for(&self.cfg, self.device));
        // A fresh handshake state alongside the fresh ring: the dead run's
        // epoch and counters describe a ring that no longer exists.
        self.sync = Arc::new(RingSync::new());
        // The new render callback starts with a fresh fade stage, which takes
        // the newest request over: the reset puts it at unity.
        self.fade_control.reset();
        self.render = Some(make_render(
            consumer,
            Arc::clone(&self.sync),
            Arc::clone(&self.gain),
            self.device,
            Arc::clone(&self.fade_control),
        ));
        self.producer = Some(producer);
        self.start_drain = None;
        Ok(())
    }

    /// Publish a fade request to `gain` over `duration_ms` and report the
    /// conflict with the buffer, if there is one.
    ///
    /// The request carries the duration the caller asked for: **a fade is
    /// never shortened** (see [`crate::fade_budget`] for why our ramp does
    /// not have to be). The report names the buffer the fade did not fit.
    fn request_fade(&mut self, gain: f32, duration_ms: u32) {
        self.fade_control.request(gain, duration_ms);
        self.report_fade_budget(duration_ms);
    }

    /// Compare a requested fade with the buffers it is realised in and queue
    /// the report when it does not fit one of them.
    ///
    /// The two buffers are the device's own (the buffer the device actually
    /// got) and the engine's ring (the buffer the audio waits in). The
    /// **binding** one is the shorter of the two — the buffer the fade
    /// exceeds first. The fade itself is not touched: it is played in full.
    fn report_fade_budget(&mut self, requested_ms: u32) {
        let rate = self.device.sample_rate;
        // The ring's real capacity: `ring_samples_for` sizes it in frames and
        // never goes below 256.
        let ring_frames = self.cfg.ring_frames.max(256) as u64;
        let ring = FadeBudget::new(requested_ms, rate, FadeBuffer::Ring, ring_frames);
        let device = FadeBudget::new(
            requested_ms,
            rate,
            FadeBuffer::Device,
            u64::from(self.device_buffer_frames),
        );
        let binding = if ring.available_frames() <= device.available_frames() {
            ring
        } else {
            device
        };
        let Some(note) = binding.note() else {
            // The fade fits: there is nothing to report.
            return;
        };
        if self.fade_notes.len() >= MAX_FADE_NOTES {
            // The caller is not taking the reports; the oldest one goes, so
            // the queue cannot grow without end.
            self.fade_notes.pop_front();
        }
        self.fade_notes.push_back(note);
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
        let _ = self.join_decode();
        if self.stream_lost {
            // RFC 0012 §4.3 item 8: an explicit start after a fatal mid-run
            // stream error opens the device again rather than refusing
            // forever. A successful `open()` clears the backend's failure
            // latch in every backend we have, so the health check below reads
            // the *new* lifecycle; a failing one keeps the retry armed and
            // reports an error naming the device, which is exactly what the
            // RFC promises Play to return while the device is still missing.
            // The pre-start check itself stays (RFC §8 Q3): it still rejects a
            // device that was already broken before any run existed, which no
            // teardown ever armed a reopen for.
            if let Err(err) = self.reopen_output() {
                self.report_output_error(&err);
                return Err(err);
            }
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
        let retry_snapshot =
            self.stream_lost
                .then_some((self.timeline_origin, self.track_start, self.start_at));
        // S15: the traversal order follows the queue. A queue that changed
        // while a run was live is taken over now: the run has ended, so the
        // engine owns the traversal again.
        self.sync_order_to_queue();
        // With shuffle on and no track selected, playback starts at the
        // first entry of the permutation (S15 §1.1: the permutation is
        // generated when playback starts with shuffle on).
        let index = match self.current {
            Some(index) => index,
            None => self.order().next_index().unwrap_or(0) as u32,
        };
        if index as usize >= self.queue.len() {
            return Err(Error::NotFound(format!("queue index {index}")));
        }
        self.current = Some(index);
        let start_at = self.start_at.take();
        // A new run starts with no stream: the observer of an old transfer is
        // stale, and the decode thread hands a fresh one over when it opens a
        // URL (S19 §2).
        self.clear_stream_progress();
        self.clock_origin = self.output.played_frames();
        // A run without an initial seek starts at zero. With one, keep the
        // target that `seek_frames` published while stopped, so the position
        // does not dip to zero between `play` and the decode thread's `Seeked`
        // (which re-bases the clock and reports the true position a moment
        // later). See the F1 finding in docs/tasks/A3-engine-lifecycle-review.md.
        self.timeline_origin = start_at.as_ref().map(|(frames, _)| *frames).unwrap_or(0);
        self.track_start = 0;
        self.end_of_stream = None;
        // The track starts are the decode thread's to report: it opens every
        // track of the run and reports the timeline position each one starts
        // at. The list therefore holds **one entry per track**, in play
        // order, and the cycle arithmetic of `current_cycle_end` counts on
        // that. A speculative entry for the track the run starts on would
        // duplicate it and shift every count by one.
        self.track_starts.clear();
        self.pending_seek = start_at.map(|(frames, seq)| PendingSeek {
            seq,
            target_ms: frames.saturating_mul(1000) / u64::from(self.device.sample_rate.max(1)),
        });
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
            queue: Arc::clone(&self.shared_queue),
            device: self.device,
            sync: Arc::clone(&self.sync),
            vis_enabled: self.cfg.vis_enabled,
            vis_capacity: self.cfg.vis_capacity,
            stop_after_current: self.stop_after_current,
            replay_gain: Arc::clone(&self.replaygain_control),
            start_epoch,
            order: Arc::clone(&self.order),
            crossfade_ms: self.crossfade_ms(),
            limiter: Arc::clone(&self.limiter_control),
        };
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
            // A run always starts at unity: a fade-out that latched the gain
            // at zero must not silence the next track. The reset is a
            // request, so it takes effect in the render callback that pulls
            // the run's first frame.
            self.fade_control.reset();
            let render = self
                .render
                .take()
                .ok_or_else(|| Error::Output("render callback missing".into()))?;
            if let Err(err) = self.output.start(render) {
                if let Some((timeline, track_start, start_at)) = retry_snapshot {
                    self.send(DecodeMsg::Stop);
                    self.sync.set_active(false);
                    let _ = self.output.pause();
                    let _ = self.join_decode();
                    self.timeline_origin = timeline;
                    self.track_start = track_start;
                    self.start_at = start_at;
                    self.clock_origin = self.output.played_frames();
                    self.pending_seek = None;
                    self.start_drain = None;
                    self.report_output_error(&err);
                }
                return Err(err);
            }
            self.started = true;
        } else {
            // The device is already running and only paused: the reset still
            // has to be requested, so the run's first frame is not attenuated.
            self.fade_control.reset();
            self.output.resume()?;
        }
        self.sync.set_active(true);
        if self.stream_lost {
            self.stream_lost = false;
            self.error = None;
        }
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
    ///
    /// The wait is **bounded** ([`SHUTDOWN_JOIN_TIMEOUT`]): the decode thread
    /// blocks in the source's `read`, and a source that never returns must not
    /// hang `stop()` or `Drop`. When the budget is missed, the thread is not
    /// joined (it is still alive and still owns the ring producer and the DSP
    /// chain) and [`Engine::take_stuck_decode_error`] reports the half-dead run
    /// instead of a clean stop. Leaving the thread alone is the only safe
    /// choice: detaching it would let it write into a ring the engine already
    /// gave to the next run, and the producer it holds cannot be recovered
    /// from any other place.
    fn join_decode(&mut self) -> Option<DecodeOut> {
        // A thread waiting for a ring drain must not block the join; nothing is
        // consuming the ring at this point anyway.
        self.sync.set_active(false);
        self.to_decode = None;
        self.decode_stuck = false;
        if let Some(thread) = self.handle.take() {
            if thread.wait_finished(SHUTDOWN_JOIN_TIMEOUT) {
                thread.join();
            } else {
                self.decode_stuck = true;
            }
        }
        // Drain any remaining messages from the decode thread. If `Returned`
        // was not handled in `poll`, recover both the ring producer and the
        // installed DSP chain here. On shutdown (`Drop`), this ensures that
        // stages are cleanly retrieved and dropped in order with the engine.
        // A queued `TrackStarted` is held for the caller: the last one wins,
        // naming the track the decode thread had open when the run ended.
        //
        // A stuck thread has not sent `Returned` and will not: its producer
        // stays with it (there is nothing to park here), so a later `start()`
        // fails with "the ring producer was lost" rather than silently
        // publishing a half-dead run.
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
                    _ => {}
                }
            }
        }
        self.to_decode = None;
        held_start
    }

    /// The error that reports a decode thread the last join gave up on, if
    /// there is one. Taking it clears the flag, so the run is reported once.
    fn take_stuck_decode_error(&mut self) -> Option<Error> {
        if !self.decode_stuck {
            return None;
        }
        self.decode_stuck = false;
        Some(stuck_decode_error())
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

/// The report for a decode thread that did not finish within
/// [`SHUTDOWN_JOIN_TIMEOUT`]: the thread is stuck in the source, so the run
/// cannot be torn down and the next one cannot get the ring back.
fn stuck_decode_error() -> Error {
    Error::Output(format!(
        "the decode thread did not finish within {} ms: the source is not returning, \
         so the run cannot be torn down; the thread still holds the ring and the DSP chain, \
         and playback needs a new engine",
        SHUTDOWN_JOIN_TIMEOUT.as_millis()
    ))
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
    use rustamp_decoders::HttpPhase;
    use rustamp_output::{NullClock, NullOutput};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    const SPEC: AudioSpec = AudioSpec::new(44100, 2);

    struct PatternDecoder {
        total: u64,
        pos: u64,
        /// The rate the decoder claims; [`SPEC`] unless the track name carries
        /// an `@<rate>` suffix (see `PatternFactory::open`).
        rate: u32,
    }

    impl Decoder for PatternDecoder {
        fn spec(&self) -> AudioSpec {
            AudioSpec::new(self.rate, SPEC.channels)
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
            let frames = (to.as_secs_f64() * f64::from(self.rate)).round() as u64;
            self.pos = frames.min(self.total);
            Ok(AudioSpec::new(self.rate, SPEC.channels).duration_of(self.pos))
        }

        fn duration(&self) -> Option<Duration> {
            Some(AudioSpec::new(self.rate, SPEC.channels).duration_of(self.total))
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
            // `track0_1000@31337.tst`: the decoder reports 31337 Hz, for the
            // converter tests that need a rate pair of their own.
            let (stem, rate) = match stem.split_once('@') {
                Some((stem, rate)) => (stem, rate.parse().unwrap_or(SPEC.sample_rate)),
                None => (stem, SPEC.sample_rate),
            };
            let frames = stem
                .strip_prefix("track")
                .and_then(|rest| rest.split_once('_'))
                .map(|(_, f)| f.parse().unwrap_or(3000))
                .unwrap_or(3000);
            Ok(Box::new(PatternDecoder {
                total: frames,
                pos: 0,
                rate,
            }))
        }
    }

    /// Control for a `.blk` track: its `read` blocks while `hold` is set.
    ///
    /// A real source can block in `read` without ever returning (a stalled
    /// network read, a hung file system). `MockDecoder` in `rustamp-core` does
    /// not, so the wrapper lives here.
    #[derive(Default)]
    struct HoldControl {
        /// The decode thread has entered the blocking read.
        entered: AtomicBool,
        /// Kept by the test: the read blocks while this is set.
        hold: AtomicBool,
    }

    /// A track whose `read` does not return while the test holds it.
    struct BlockedDecoder {
        control: Arc<HoldControl>,
    }

    impl Decoder for BlockedDecoder {
        fn spec(&self) -> AudioSpec {
            SPEC
        }

        fn read(&mut self, out: &mut [f32]) -> CoreResult<usize> {
            self.control.entered.store(true, Ordering::Release);
            while self.control.hold.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(1));
            }
            out.fill(0.0);
            Ok(0)
        }

        fn seek(&mut self, _to: Duration) -> CoreResult<Duration> {
            Err(rustamp_core::error::Error::Unsupported(
                "the test source does not seek",
            ))
        }

        fn duration(&self) -> Option<Duration> {
            Some(Duration::from_secs(1))
        }
    }

    struct BlockedFactory {
        control: Arc<HoldControl>,
    }

    impl DecoderFactory for BlockedFactory {
        fn name(&self) -> &str {
            "blocked"
        }

        fn extensions(&self) -> &[&str] {
            &["blk"]
        }

        fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
            if hint.extension == Some("blk") {
                Confidence::CONTENT
            } else {
                Confidence::NONE
            }
        }

        fn open(&self, _source: MediaSource) -> CoreResult<Box<dyn Decoder>> {
            Ok(Box::new(BlockedDecoder {
                control: Arc::clone(&self.control),
            }))
        }
    }

    struct TestHarness {
        engine: Engine,
        clock: NullClock,
        events: rustamp_core::bus::EventReceiver,
        /// Control for the `.blk` tracks (a source whose `read` can block).
        blocked: Arc<HoldControl>,
        _dir: tempfile::TempDir,
    }

    impl TestHarness {
        fn new(names: &[&str]) -> Self {
            let dir = tempfile::tempdir().expect("temp dir");
            let (output, clock) = NullOutput::pair();
            let bus = EventBus::new();
            let events = bus.subscribe();
            let blocked = Arc::new(HoldControl::default());
            let mut registry = DecoderRegistry::new();
            registry.register(Arc::new(PatternFactory));
            registry.register(Arc::new(BlockedFactory {
                control: Arc::clone(&blocked),
            }));
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
                blocked,
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

    /// A port that nothing listens on: the connect is refused at once.
    fn closed_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local listener");
        let port = listener.local_addr().expect("the address").port();
        drop(listener);
        port
    }

    /// A file run is not on a stream: there is no phase to show.
    #[test]
    fn a_file_run_shows_no_stream_phase() {
        let mut h = TestHarness::new(&["track7_2048.tst"]);
        h.engine.play_index(0).unwrap();
        assert!(
            h.wait_pumping(|e| e.duration_frames().is_some(), Duration::from_secs(5)),
            "the track must start"
        );
        assert_eq!(h.engine.stream_snapshot(), None, "a file is not a stream");
    }

    /// The observer survives a failed open: the phase that says the host
    /// could not be reached is still readable (S19 §2).
    #[test]
    fn a_failed_connect_leaves_the_phase_that_says_why() {
        let mut h = TestHarness::new(&[]);
        let url = format!("http://127.0.0.1:{}/stream.wav", closed_port());
        h.engine.set_queue(vec![TrackSource::Url(url)]);
        h.engine.play_index(0).unwrap();
        assert!(
            h.wait_pumping(
                |e| e
                    .stream_snapshot()
                    .is_some_and(|snapshot| snapshot.phase == HttpPhase::ConnectFailed),
                Duration::from_secs(10),
            ),
            "the phase of the failed connect must be readable"
        );
    }

    /// A track whose rate pair has no resampler must end with a reported error
    /// and not a single frame in the ring. Before the fix the converter's
    /// `convert` returned 0 frames and the track "played" as silence with the
    /// position running.
    #[test]
    fn an_unavailable_resampler_ends_the_track_with_an_error() {
        // The device grants 44.1 kHz (the harness's `EngineConfig`); this
        // track reports 31337 Hz, a pair no other test uses.
        const TRACK_RATE: u32 = 31_337;
        crate::convert::force_no_resampler(TRACK_RATE, SPEC.sample_rate);
        let mut h = TestHarness::new(&["track0_1000@31337.tst"]);

        h.engine.play_index(0).unwrap();
        let reported = h.wait_pumping(|e| e.last_error().is_some(), Duration::from_secs(10));
        // The forced pair only exists for this test; take it back before the
        // assertions so a failure cannot leak into another test.
        crate::convert::clear_forced_no_resampler();
        assert!(reported, "the unavailable resampler must be reported");

        let (written, _) = h.engine.sync.counters();
        assert_eq!(written, 0, "no frame may reach the ring");

        let events = h.drain_events();
        let error = events
            .iter()
            .find_map(|event| match event {
                Event::Error(info) => Some(info),
                _ => None,
            })
            .expect("the missing resampler must publish an error event");
        assert_eq!(error.kind, ErrorKind::Other);
        assert!(
            error.message.contains("no resampler"),
            "the report must name the cause: {error:?}"
        );
        // The run is still alive at the end of the stream, not silently playing.
        assert_eq!(h.engine.current_index(), Some(0));
        h.engine.stop().unwrap();
    }

    /// A decode thread stuck in a source read must not hang `stop()`: the join
    /// gives up after [`SHUTDOWN_JOIN_TIMEOUT`] and the half-dead run is
    /// reported instead of being published as a clean stop.
    #[test]
    fn a_stuck_decode_thread_does_not_hang_stop_and_is_reported() {
        let mut h = TestHarness::new(&["stuck.blk"]);
        h.blocked.hold.store(true, Ordering::Release);
        h.engine.play_index(0).unwrap();

        // Wait until the decode thread is inside the source read: from here on
        // it can neither see `Stop` nor reach the end of the run.
        let entered = {
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                if h.blocked.entered.load(Ordering::Acquire) {
                    break true;
                }
                if std::time::Instant::now() >= deadline {
                    break false;
                }
                h.clock.pump(256);
                thread::sleep(Duration::from_millis(1));
            }
        };
        assert!(entered, "the decode thread must reach the source read");

        let started = std::time::Instant::now();
        let stopped = h.engine.stop();
        let elapsed = started.elapsed();
        h.blocked.hold.store(false, Ordering::Release);

        assert!(
            stopped.is_err(),
            "a run whose thread never finished must not stop cleanly"
        );
        assert!(
            elapsed >= SHUTDOWN_JOIN_TIMEOUT - Duration::from_millis(50),
            "the join must wait for its budget: {elapsed:?}"
        );
        assert!(
            elapsed < SHUTDOWN_JOIN_TIMEOUT + Duration::from_secs(2),
            "the join must give up after its budget: {elapsed:?}"
        );
        assert_eq!(h.engine.transport(), TransportState::Stopped);

        let events = h.drain_events();
        let error = events
            .iter()
            .find_map(|event| match event {
                Event::Error(info) => Some(info),
                _ => None,
            })
            .expect("the stuck run must be reported");
        assert_eq!(error.kind, ErrorKind::Output);
        assert!(
            error.message.contains("did not finish"),
            "the report must name the stuck join: {error:?}"
        );
        assert!(h.engine.last_error().is_some());
    }
}
