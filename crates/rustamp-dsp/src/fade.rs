//! The output fade stage: a gain ramp to silence and back.
//!
//! # What is measured and what is chosen here
//!
//! **Nothing in this module is measured.** The spec that would describe the
//! reference player's output fades — S18 ("Output behaviors (out_ds-style
//! fades, buffer, prebuffer)") — **does not exist**. Every number, every law
//! and every rule below is therefore **our own engineering choice**, and none
//! of it may be cited as 2.95 behaviour.
//!
//! The only thing the frozen contracts fix is the *shape* of the request:
//! `rustamp-core`'s
//! [`AudioConfig::fadeout_ms`](rustamp_core::config::AudioConfig::fadeout_ms)
//! names a fade-out duration in milliseconds for `Command::StopWithFadeout`,
//! and this stage is what that duration drives. The duration itself is an
//! input, not a constant of this module: the caller passes it with every
//! request.
//!
//! # The ramp law (own choice)
//!
//! **Equal power.** At the fraction `t` of a ramp that goes from `from` to
//! `to`, the gain is
//!
//! ```text
//! g(t) = sqrt(from^2 + (to^2 - from^2) * t)
//! ```
//!
//! A fade-out (`from = 1`, `to = 0`) is `g(t) = sqrt(1 - t)`, a fade-in
//! (`from = 0`, `to = 1`) is `g(t) = sqrt(t)`. The law keeps the *power*
//! (`g^2`) linear between the two endpoints, so the perceived loudness falls
//! evenly over the ramp instead of dropping fast and then crawling — which is
//! what a linear-in-amplitude ramp does. A linear ramp is the other obvious
//! choice; it is not used here.
//!
//! The endpoints are exact: the first frame of a ramp carries `from`
//! unchanged, the last one carries `to` exactly. A ramp therefore starts and
//! ends without a step, which is the whole point of a fade.
//!
//! # Where the stage runs
//!
//! The stage is an ordinary [`DspStage`], but the engine runs it in the
//! **render callback** (the consumer side of the ring), not in the decode
//! thread's chain. The reason is the ring: the decode thread writes about
//! 370 ms ahead of the device, so a fade applied there would be heard only
//! after that backlog had been played at full level, and the transport would
//! have to wait for the backlog as well. On the consumer side the ramp
//! covers exactly the frames the device plays, so its duration is the
//! requested one and the silence arrives when the ramp says it does.
//!
//! The consequence is that the visualization tap — which sits on the producer
//! side, before the ring — sees the **un-faded** signal. That is the same
//! split the volume/balance gain has: the tap shows the track, the device
//! gets the output.
//!
//! # The bypass rule (own choice)
//!
//! While no ramp is running **and** the gain rests at unity, `process`
//! returns before touching the buffer, so the output is the input, bit for
//! bit. A ramp that ends at unity (a fade-in, or a reset) returns to that
//! bypass; a ramp that ends at silence keeps multiplying by zero, because
//! silence is the state the caller asked for.
//!
//! # Duration accuracy (own choice)
//!
//! A ramp covers `round(duration_ms * sample_rate / 1000)` frames and is
//! counted in **frames**, not in blocks, so it completes within the requested
//! duration no matter how the frames are split into buffers. A request of
//! `0` ms jumps to the target at once.
//!
//! # Threading
//!
//! The stage lives on the audio thread; the owner of the playback keeps an
//! [`Arc<FadeControl>`] and writes the requests into it. The control block is
//! lock-free: the owner publishes a request (target, duration) and bumps a
//! counter, the stage picks the request up at its next `process` and reports
//! the finished counter back. `process` allocates nothing and takes no lock.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use rustamp_core::audio::{AudioSpec, DspStage};
use rustamp_core::error::Result;

/// The gain of a finished ramp that rests at unity.
const UNITY: f32 = 1.0;

/// The gain a finished ramp rests at when it faded to silence.
const SILENCE: f32 = 0.0;

/// The equal-power gain at the fraction `t` of a ramp from `from` to `to`.
///
/// The power `g^2` is linear between the endpoints; see the module docs.
fn equal_power(from: f32, to: f32, t: f64) -> f32 {
    let from = f64::from(from);
    let to = f64::from(to);
    let power = from * from + (to * to - from * from) * t;
    power.max(0.0).sqrt() as f32
}

/// The requests of a [`Fade`] stage, shared with the owner of the playback.
///
/// The owner publishes a request and the stage carries it out; the stage
/// publishes the counter of the request it finished, so the owner can wait
/// for a ramp to be over without polling the audio path.
///
/// The counters are monotonic: a request that arrives while another one is
/// running replaces it, and the *newest* request is the one that counts.
#[derive(Debug)]
pub struct FadeControl {
    /// Counter of the requests; bumped on every request.
    request: AtomicU64,
    /// Target gain of the newest request (the bits of an `f32`).
    target: AtomicU32,
    /// Duration of the newest request, in milliseconds.
    duration_ms: AtomicU32,
    /// The newest request the stage has carried out.
    finished: AtomicU64,
}

impl Default for FadeControl {
    fn default() -> Self {
        FadeControl::new()
    }
}

impl FadeControl {
    /// A control block with no request pending: the stage stays at unity.
    pub fn new() -> Self {
        FadeControl {
            request: AtomicU64::new(0),
            target: AtomicU32::new(UNITY.to_bits()),
            duration_ms: AtomicU32::new(0),
            finished: AtomicU64::new(0),
        }
    }

    /// Request a ramp to `gain` over `duration_ms`.
    ///
    /// `gain` is clamped to `0.0..=1.0`; a non-finite gain is read as unity
    /// (a gain that cannot be applied honestly is no gain at all). A
    /// `duration_ms` of `0` jumps to the target at once.
    pub fn request(&self, gain: f32, duration_ms: u32) {
        let gain = if gain.is_finite() {
            gain.clamp(SILENCE, UNITY)
        } else {
            UNITY
        };
        self.target.store(gain.to_bits(), Ordering::Relaxed);
        self.duration_ms.store(duration_ms, Ordering::Relaxed);
        // Release: a reader that sees the new counter also sees the fields
        // above it.
        self.request.fetch_add(1, Ordering::Release);
    }

    /// Request a ramp to silence over `duration_ms`.
    pub fn fade_out(&self, duration_ms: u32) {
        self.request(SILENCE, duration_ms);
    }

    /// Request a ramp back to unity over `duration_ms`.
    pub fn fade_in(&self, duration_ms: u32) {
        self.request(UNITY, duration_ms);
    }

    /// Jump back to unity, without a ramp.
    pub fn reset(&self) {
        self.request(UNITY, 0);
    }

    /// The counter of the newest request.
    pub fn requested(&self) -> u64 {
        self.request.load(Ordering::Acquire)
    }

    /// The counter of the newest request the stage has carried out.
    pub(crate) fn finished(&self) -> u64 {
        self.finished.load(Ordering::Acquire)
    }

    /// Whether the request `request` has been carried out.
    ///
    /// A request that was never made (`0`) counts as carried out, so a caller
    /// that waits for a request it has published is never blocked by an idle
    /// control block.
    pub fn is_finished(&self, request: u64) -> bool {
        self.finished() >= request
    }

    /// The target gain of the newest request.
    pub(crate) fn target(&self) -> f32 {
        f32::from_bits(self.target.load(Ordering::Relaxed))
    }

    /// The duration of the newest request, in milliseconds.
    pub(crate) fn duration_ms(&self) -> u32 {
        self.duration_ms.load(Ordering::Relaxed)
    }

    /// Report that the stage carried out `request`.
    fn mark_finished(&self, request: u64) {
        // The counter never moves backwards: a stage that is replaced while a
        // ramp runs must not undo the report of the one before it.
        self.finished.fetch_max(request, Ordering::Release);
    }
}

/// The output fade as a [`DspStage`]. See the module docs for the ramp law,
/// for what is an own choice and for where the engine runs the stage.
#[derive(Debug)]
pub struct Fade {
    control: Arc<FadeControl>,
    /// The request counter the stage has taken over.
    seen: u64,
    /// Gain at the start of the running ramp.
    from: f32,
    /// Gain the running ramp heads for.
    to: f32,
    /// The gain the stage applies while no ramp runs.
    gain: f32,
    /// Frames the running ramp has left.
    left: u64,
    /// Frames the running ramp covers in total.
    span: u64,
    /// Frame index inside the running ramp.
    pos: u64,
    /// Sample rate of the prepared format.
    sample_rate: u32,
    /// Channel count of the prepared format.
    channels: usize,
}

impl Default for Fade {
    fn default() -> Self {
        Fade::new()
    }
}

impl Fade {
    /// A stage with its own control block, resting at unity.
    pub fn new() -> Self {
        Fade::with_control(Arc::new(FadeControl::new()))
    }

    /// A stage that follows a control block shared with its owner.
    pub fn with_control(control: Arc<FadeControl>) -> Self {
        Fade {
            control,
            seen: 0,
            from: UNITY,
            to: UNITY,
            gain: UNITY,
            left: 0,
            span: 0,
            pos: 0,
            sample_rate: 44_100,
            channels: 0,
        }
    }

    /// The control block this stage reads.
    pub fn control(&self) -> Arc<FadeControl> {
        Arc::clone(&self.control)
    }

    /// The gain the stage applies right now (`1.0` = unity).
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Take over a request that was published since the last call.
    fn poll_control(&mut self) {
        let request = self.control.requested();
        if request == self.seen {
            return;
        }
        self.seen = request;
        self.from = self.gain;
        self.to = self.control.target();
        self.span = self.ramp_frames(self.control.duration_ms());
        self.pos = 0;
        self.left = self.span;
        if self.left == 0 || self.from == self.to {
            // Nothing to ramp: the target is already reached.
            self.gain = self.to;
            self.left = 0;
            self.span = 0;
            self.control.mark_finished(self.seen);
        }
    }

    /// The number of frames a ramp of `duration_ms` covers.
    fn ramp_frames(&self, duration_ms: u32) -> u64 {
        if duration_ms == 0 {
            return 0;
        }
        let frames = f64::from(duration_ms) * f64::from(self.sample_rate) / 1000.0;
        frames.round().max(1.0) as u64
    }

    /// The gain of the next frame, advancing the ramp.
    fn next_gain(&mut self) -> f32 {
        if self.left == 0 {
            return self.gain;
        }
        self.left -= 1;
        let gain = if self.left == 0 {
            // The last frame of the ramp takes the target exactly.
            self.to
        } else {
            // The first frame of the ramp takes the start exactly and the
            // last one the target, so the ramp is spread over its frames.
            let t = self.pos as f64 / (self.span - 1).max(1) as f64;
            equal_power(self.from, self.to, t)
        };
        self.pos += 1;
        self.gain = gain;
        if self.left == 0 {
            self.control.mark_finished(self.seen);
        }
        gain
    }
}

impl DspStage for Fade {
    fn name(&self) -> &str {
        "fade"
    }

    fn prepare(&mut self, spec: AudioSpec) -> Result<()> {
        self.sample_rate = spec.sample_rate.max(1);
        self.channels = usize::from(spec.channels.max(1));
        // A request that was published before the stage existed is taken over
        // here, with the sample rate of the format it is prepared for.
        self.poll_control();
        Ok(())
    }

    fn process(&mut self, buf: &mut [f32]) -> Result<()> {
        self.poll_control();
        if self.left == 0 && self.gain == UNITY {
            // Exact bypass: the buffer is not touched at all.
            return Ok(());
        }
        let channels = self.channels.max(1);
        for frame in buf.chunks_exact_mut(channels) {
            let gain = self.next_gain();
            for sample in frame.iter_mut() {
                *sample *= gain;
            }
        }
        Ok(())
    }

    fn reset(&mut self) {
        // The engine runs this stage in the render callback, where a track
        // change is not a stage reset, so nothing calls `reset` in the
        // product. It is here for the trait contract: the stage returns to
        // unity, the running ramp is dropped and the request that was
        // pending is reported as carried out.
        self.gain = UNITY;
        self.left = 0;
        self.span = 0;
        self.pos = 0;
        self.seen = self.control.requested();
        self.control.mark_finished(self.seen);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FS: u32 = 48_000;

    fn spec(channels: u16) -> AudioSpec {
        AudioSpec::new(FS, channels)
    }

    /// A stage prepared for `channels` channels, following `control`.
    fn stage(control: &Arc<FadeControl>, channels: u16) -> Fade {
        let mut fade = Fade::with_control(Arc::clone(control));
        fade.prepare(spec(channels)).expect("prepare");
        fade
    }

    /// The gain the stage applies to each frame of `frames` constant frames.
    fn gains(fade: &mut Fade, frames: usize, channels: u16) -> Vec<f32> {
        let mut buf = vec![1.0f32; frames * usize::from(channels)];
        fade.process(&mut buf).expect("process");
        buf.chunks_exact(usize::from(channels))
            .map(|frame| frame[0])
            .collect()
    }

    #[test]
    fn the_bypass_does_not_touch_the_buffer() {
        let control = Arc::new(FadeControl::new());
        let mut fade = stage(&control, 2);
        // Odd values a multiplication would change: a signed zero, a
        // subnormal and a NaN.
        let mut buf = vec![0.0f32, -0.0, f32::MIN_POSITIVE, f32::NAN, 1.0, -1.0];
        let before: Vec<u32> = buf.iter().map(|s| s.to_bits()).collect();
        fade.process(&mut buf).expect("process");
        let after: Vec<u32> = buf.iter().map(|s| s.to_bits()).collect();
        assert_eq!(before, after, "the bypass must be bit exact");
    }

    #[test]
    fn a_fade_out_is_equal_power_and_ends_at_silence() {
        let control = Arc::new(FadeControl::new());
        let mut fade = stage(&control, 1);
        control.fade_out(10);
        let span = 480; // 10 ms at 48 kHz
        let applied = gains(&mut fade, span + 1, 1);
        assert_eq!(applied.len(), span + 1);
        assert_eq!(applied[0], 1.0, "the ramp starts at unity");
        assert_eq!(applied[span - 1], 0.0, "the ramp ends at silence");
        assert_eq!(applied[span], 0.0, "the silence is latched");
        for (i, gain) in applied.iter().take(span).enumerate() {
            let t = i as f64 / (span - 1) as f64;
            let expected = (1.0 - t).sqrt();
            assert!(
                (f64::from(*gain) - expected).abs() < 1e-6,
                "frame {i}: {} != {expected}",
                *gain
            );
            // The power falls evenly: it is affine in the frame index.
            let power = f64::from(*gain) * f64::from(*gain);
            assert!(
                (power - (1.0 - t)).abs() < 1e-6,
                "frame {i}: the power does not fall evenly ({power})"
            );
        }
    }

    #[test]
    fn the_ramp_is_the_same_however_the_frames_are_split() {
        let control = Arc::new(FadeControl::new());
        let mut reference = stage(&control, 2);
        control.fade_out(10);
        let frames = 480;
        let expected = gains(&mut reference, frames, 2);

        for block in [1usize, 3, 7, 64, 1024] {
            let control = Arc::new(FadeControl::new());
            let mut fade = stage(&control, 2);
            control.fade_out(10);
            let mut got = Vec::new();
            let mut left = frames;
            while left > 0 {
                let n = left.min(block);
                got.extend(gains(&mut fade, n, 2));
                left -= n;
            }
            assert_eq!(got, expected, "block size {block}");
        }
    }

    #[test]
    fn the_ramp_completes_within_the_requested_duration() {
        let control = Arc::new(FadeControl::new());
        let mut fade = stage(&control, 1);
        control.fade_out(250);
        // 250 ms at 48 kHz is 12000 frames.
        let applied = gains(&mut fade, 12_000, 1);
        assert_eq!(applied[11_999], 0.0, "the ramp is over after 250 ms");
        assert!(control.is_finished(control.requested()));
        // The frames after the ramp keep the silence.
        let after = gains(&mut fade, 10, 1);
        assert!(after.iter().all(|g| *g == 0.0), "silence is latched");
    }

    #[test]
    fn a_fade_in_brings_the_gain_back_to_unity() {
        let control = Arc::new(FadeControl::new());
        let mut fade = stage(&control, 1);
        control.fade_out(10);
        gains(&mut fade, 480, 1);
        assert_eq!(fade.gain(), 0.0);
        control.fade_in(10);
        let applied = gains(&mut fade, 480, 1);
        assert_eq!(applied[0], 0.0, "the ramp starts at silence");
        assert_eq!(applied[479], 1.0, "the ramp ends at unity");
        // Back in the bypass.
        let mut buf = vec![0.25f32, -0.25];
        fade.process(&mut buf).expect("process");
        assert_eq!(buf, vec![0.25, -0.25], "the bypass is bit exact again");
    }

    #[test]
    fn a_request_without_a_duration_jumps_at_once() {
        let control = Arc::new(FadeControl::new());
        let mut fade = stage(&control, 1);
        control.reset();
        let applied = gains(&mut fade, 4, 1);
        assert!(applied.iter().all(|g| *g == 1.0), "unity is a no-op");
        assert!(control.is_finished(control.requested()));
    }

    #[test]
    fn a_request_while_the_gain_is_already_at_the_target_is_immediate() {
        let control = Arc::new(FadeControl::new());
        let mut fade = stage(&control, 1);
        control.fade_out(0);
        gains(&mut fade, 1, 1);
        assert_eq!(fade.gain(), 0.0);
        // A second fade-out has nothing to ramp.
        control.fade_out(500);
        let applied = gains(&mut fade, 1, 1);
        assert_eq!(applied[0], 0.0);
        assert!(control.is_finished(control.requested()));
    }

    #[test]
    fn the_report_never_moves_backwards() {
        let control = Arc::new(FadeControl::new());
        let mut fade = stage(&control, 1);
        control.fade_out(10);
        gains(&mut fade, 480, 1);
        let finished = control.finished();
        assert!(finished > 0);
        // A stage that is replaced reports its own request; the counter stays
        // at the highest one.
        control.mark_finished(finished - 1);
        assert_eq!(control.finished(), finished);
    }

    #[test]
    fn the_request_counter_survives_a_stage_that_joins_late() {
        let control = Arc::new(FadeControl::new());
        control.fade_out(10);
        // The stage is created after the request was published: it takes the
        // request over instead of ignoring it.
        let mut fade = stage(&control, 1);
        let applied = gains(&mut fade, 480, 1);
        assert_eq!(applied[0], 1.0);
        assert_eq!(applied[479], 0.0);
    }
}
