//! A look-ahead brickwall limiter stage.
//!
//! # Why it exists, and what is measured
//!
//! Nothing here is measured. The equalizer ([`crate::eq`]) states that boosts
//! can push samples past full scale and that what the reference player does
//! with such overs is unmeasured; the device path would otherwise clip them
//! hard. This stage is an **own-choice addition** that offers a clean
//! alternative to that hard clip. It is **enabled by default** (owner decision,
//! 2026-10-17); what the reference player does with overs is unmeasured, so
//! enabling by default is our own choice and it costs latency (the 5 ms
//! look-ahead). When disabled, it is a bit-exact bypass. None of its behaviour
//! may be cited as 2.95 behaviour.
//!
//! # Why not a crate
//!
//! A maintained third-party limiter was preferred and checked first; none fit
//! a [`DspStage`] (in place, interleaved, any channel count, no allocation in
//! `process`, guaranteed ceiling, bit-exact when idle). The candidates and the
//! reasons are recorded in the lane report; in short: `fundsp`'s limiter
//! smooths with a follower (no ceiling guarantee), allocates while filling its
//! look-ahead and fixes the channel count at compile time; `dsp-core` smooths
//! with a one-pole attack (overshoot unless attack ≪ look-ahead) and is a
//! first release; `bisque` is the closest design but a first, three-month-old
//! release of a large framework, planar with a fixed maximum block, and pins
//! `libm` to an exact version; `compressor` and `audio-processor-dynamics`
//! have no look-ahead; `dhvani` is GPL-3.0-only.
//!
//! # Topology (own engineering choice)
//!
//! Per frame, with `L` the look-ahead in frames and a window of `L + 1`
//! frames:
//!
//! 1. **Required gain.** The linked peak `p` of the incoming frame (the largest
//!    `|sample|` over all channels) asks for `r = min(1, ceiling / p)`. One gain
//!    for all channels keeps the stereo image where it is.
//! 2. **Hold.** `h` is the minimum of `r` over the last `L + 1` frames (a
//!    monotonic-wedge sliding minimum, `O(1)` amortised).
//! 3. **Release.** The envelope `e` follows `h` down instantly and back up
//!    with a one-pole of time constant [`RELEASE_SECONDS`], so `e <= h`.
//! 4. **Attack.** The applied gain `g` is the moving average of `e` over the
//!    last `L + 1` frames: the gain ramps down linearly across the look-ahead
//!    instead of jumping, and moves by at most `1 / (L + 1)` per frame.
//! 5. **Delay.** `g` multiplies the frame that entered `L` frames ago.
//!
//! **No overshoot, by construction:** every `e(u)` in the averaging window of
//! the frame `i` being output is a minimum over a window that contains `i`, so
//! each is at most `r(i)`, and so is their average: `|x(i)| * g <= ceiling`.
//! A final clamp to the ceiling only absorbs floating-point rounding in that
//! product; it never shapes the waveform.
//!
//! **Transparency:** while every frame in the window is at or below the
//! ceiling, `g` is exactly `1.0` (the running sum is replaced by its exact
//! value whenever no envelope entry is below one), so the output is the input
//! delayed by `L` frames, bit for bit. After a reduction the envelope snaps to
//! exactly `1.0` once it is within `1e-9`, so transparency returns.
//!
//! **Non-finite input:** a frame holding a NaN or an infinity asks for gain
//! `0`; such samples come out as `0.0` rather than reaching the device.
//!
//! # Own-choice parameters
//!
//! * [`CEILING_DB`] `= -0.3 dBFS`: a sample-peak ceiling just under full scale.
//!   The limiter sees samples, not the reconstructed waveform; it cannot by
//!   itself bound what the band-limited resampler after it rebuilds between
//!   samples (A6 finding 3). The [`FormatConverter`] in `rustamp-audio`
//!   enforces the final `±1.0` (0 dBFS) ceiling on the resampled path, so the
//!   ring never holds a sample above full scale; this −0.3 dB margin keeps
//!   ordinary program material out of that safety net.
//! * [`LOOKAHEAD_SECONDS`] `= 5 ms`: long enough that the attack ramp spans
//!   about one period of 200 Hz, so a transient is turned down over a few
//!   milliseconds instead of being clipped; short enough to keep the added
//!   latency (reported by [`DspStage::latency_frames`]) small.
//! * [`RELEASE_SECONDS`] `= 50 ms`: a common programme-limiter release; slow
//!   enough not to modulate bass waveforms within a period, fast enough not to
//!   leave audible holes after a single peak.
//!
//! # Place in the pipeline
//!
//! The intended position is the **last** stage of the decode thread's DSP
//! chain: after the equalizer and any other stage, before the format
//! converter. After it come the resampler and the volume/balance gains. The
//! volume/balance gains never raise the level above unity (attenuation only,
//! see `rustamp-audio`'s gain module); the resampler **can** rebuild
//! inter-sample peaks above this stage's ceiling, which is why the converter
//! holds a final `±1.0` ceiling on the resampled path. Inserting it is the
//! engine's decision (and an owner decision, since it changes the sound once
//! enabled); this module only provides the stage.
//!
//! # Switching
//!
//! [`LimiterControl::set_enabled`] is latched at the next
//! [`DspStage::prepare`] or [`DspStage::reset`] (track start, seek). Enabling
//! adds `L` frames of delay and disabling removes them, so switching inside a
//! running stream would insert or drop audio; at those points the history is
//! cleared anyway.
//!
//! # Threading
//!
//! The stage lives on the decode thread. `process` never allocates and never
//! locks; the control block is a single atomic flag.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustamp_core::audio::{AudioSpec, DspStage};
use rustamp_core::error::Result;

/// The output ceiling in dBFS (own choice: -0.3 dBFS, a sample-peak ceiling).
pub const CEILING_DB: f64 = -0.3;

/// The linear ceiling: no output sample of an active limiter exceeds it.
/// Kept as a function (not a const) because `powf` is not const-evaluable;
/// the value is `10^(-0.3/20) ≈ 0.96605`.
pub fn ceiling_linear() -> f32 {
    10.0_f64.powf(CEILING_DB / 20.0) as f32
}

/// Look-ahead, and therefore attack time, in seconds (own choice: 5 ms).
pub const LOOKAHEAD_SECONDS: f64 = 0.005;

/// Release time constant in seconds (own choice: 50 ms).
pub const RELEASE_SECONDS: f64 = 0.050;

/// Distance below the target at which the released envelope snaps to it.
const SNAP: f64 = 1e-9;

/// The limiter's on/off switch, shared between the engine (writer) and the
/// stage on the decode thread (reader). Lock-free.
#[derive(Debug)]
pub struct LimiterControl {
    enabled: AtomicBool,
}

impl Default for LimiterControl {
    /// A control block in the default state (enabled; owner decision, 2026-10-17).
    fn default() -> Self {
        LimiterControl::new(true)
    }
}

impl LimiterControl {
    /// A control block in the given state.
    pub fn new(enabled: bool) -> Self {
        LimiterControl {
            enabled: AtomicBool::new(enabled),
        }
    }

    /// Request the limiter on or off. Takes effect at the stage's next
    /// `prepare` or `reset` (see the module docs).
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
    }

    /// The requested state.
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }
}

/// The look-ahead brickwall limiter as a [`DspStage`]. See the module docs
/// for the topology and for which parts are own choices.
#[derive(Debug)]
pub struct Limiter {
    control: Arc<LimiterControl>,
    /// The latched on/off state.
    active: bool,
    /// Linear ceiling.
    ceiling: f32,
    channels: usize,
    /// Look-ahead `L` in frames (>= 1): the delay of the signal path.
    lookahead: usize,
    /// `L + 1`: the hold and the attack window.
    window: usize,
    /// One-pole release coefficient per frame.
    release: f64,
    /// Interleaved delay line of `lookahead` frames.
    delay: Vec<f32>,
    delay_pos: usize,
    /// Sliding-minimum wedge (ring of `window + 1` entries): required gains
    /// in increasing order, with the frame index each was pushed at.
    wedge_val: Vec<f64>,
    wedge_idx: Vec<u64>,
    wedge_head: usize,
    wedge_len: usize,
    /// Frames pushed since the last reset.
    frame: u64,
    /// Released envelope `e`.
    envelope: f64,
    /// The last `window` envelope values, for the attack average.
    avg: Vec<f64>,
    avg_pos: usize,
    avg_sum: f64,
    /// Entries of `avg` below `1.0`.
    below: usize,
}

impl Default for Limiter {
    fn default() -> Self {
        Limiter::new()
    }
}

impl Limiter {
    /// A stage with its own control block, enabled by default (owner decision, 2026-10-17).
    pub fn new() -> Self {
        Limiter::with_control(Arc::new(LimiterControl::default()))
    }

    /// A stage that follows a control block shared with its owner.
    pub fn with_control(control: Arc<LimiterControl>) -> Self {
        let active = control.enabled();
        Limiter {
            control,
            active,
            ceiling: ceiling_linear(),
            channels: 0,
            lookahead: 1,
            window: 2,
            release: 1.0,
            delay: Vec::new(),
            delay_pos: 0,
            wedge_val: Vec::new(),
            wedge_idx: Vec::new(),
            wedge_head: 0,
            wedge_len: 0,
            frame: 0,
            envelope: 1.0,
            avg: Vec::new(),
            avg_pos: 0,
            avg_sum: 0.0,
            below: 0,
        }
    }

    /// The control block this stage reads.
    pub fn control(&self) -> Arc<LimiterControl> {
        Arc::clone(&self.control)
    }

    /// The linear ceiling no output sample exceeds while the stage is active.
    pub fn ceiling(&self) -> f32 {
        self.ceiling
    }

    /// Push the required gain of the newest frame and return the minimum over
    /// the last `window` frames.
    fn hold(&mut self, required: f64) -> f64 {
        let cap = self.wedge_val.len();
        // Older entries that are not smaller can never be the minimum again.
        while self.wedge_len > 0 {
            let back = (self.wedge_head + self.wedge_len - 1) % cap;
            if self.wedge_val[back] >= required {
                self.wedge_len -= 1;
            } else {
                break;
            }
        }
        let slot = (self.wedge_head + self.wedge_len) % cap;
        self.wedge_val[slot] = required;
        self.wedge_idx[slot] = self.frame;
        self.wedge_len += 1;
        // Entries that have left the window.
        while self.wedge_idx[self.wedge_head] + self.window as u64 <= self.frame {
            self.wedge_head = (self.wedge_head + 1) % cap;
            self.wedge_len -= 1;
        }
        self.frame += 1;
        self.wedge_val[self.wedge_head]
    }

    /// Push the newest envelope value and return the attack-smoothed gain.
    fn attack(&mut self, envelope: f64) -> f64 {
        let old = std::mem::replace(&mut self.avg[self.avg_pos], envelope);
        if old < 1.0 {
            self.below -= 1;
        }
        if envelope < 1.0 {
            self.below += 1;
        }
        self.avg_sum += envelope - old;
        self.avg_pos += 1;
        if self.avg_pos == self.window {
            self.avg_pos = 0;
            // Re-sum once per window so rounding in the running sum cannot
            // accumulate.
            self.avg_sum = self.avg.iter().sum();
        }
        if self.below == 0 {
            self.avg_sum = self.window as f64;
            1.0
        } else {
            (self.avg_sum / self.window as f64).clamp(0.0, 1.0)
        }
    }
}

/// Keep a limited sample inside the ceiling despite rounding, and silence a
/// non-finite one.
#[inline]
fn guard(sample: f32, ceiling: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-ceiling, ceiling)
    } else {
        0.0
    }
}

impl DspStage for Limiter {
    fn name(&self) -> &str {
        "limiter"
    }

    fn prepare(&mut self, spec: AudioSpec) -> Result<()> {
        let rate = f64::from(spec.sample_rate.max(1));
        self.channels = usize::from(spec.channels);
        self.lookahead = ((LOOKAHEAD_SECONDS * rate).round() as usize).max(1);
        self.window = self.lookahead + 1;
        self.release = 1.0 - (-1.0 / (RELEASE_SECONDS * rate)).exp();
        self.delay = vec![0.0; self.lookahead * self.channels];
        self.wedge_val = vec![1.0; self.window + 1];
        self.wedge_idx = vec![0; self.window + 1];
        self.avg = vec![1.0; self.window];
        self.reset();
        Ok(())
    }

    fn process(&mut self, buf: &mut [f32]) {
        let channels = self.channels;
        if !self.active || channels == 0 {
            // Exact bypass: the buffer is not touched.
            return;
        }
        let ceiling = self.ceiling;
        for frame in buf.chunks_exact_mut(channels) {
            // 1. Linked peak of the incoming frame; NaN counts as infinite.
            let mut peak = 0.0_f32;
            for sample in frame.iter() {
                let a = sample.abs();
                if a.is_nan() {
                    peak = f32::INFINITY;
                } else if a > peak {
                    peak = a;
                }
            }
            let required = if peak <= ceiling {
                1.0
            } else if peak.is_finite() {
                f64::from(ceiling) / f64::from(peak)
            } else {
                0.0
            };
            // 2. Hold over the look-ahead window.
            let held = self.hold(required);
            // 3. Instant down, one-pole release up (e <= h).
            self.envelope = if held <= self.envelope {
                held
            } else {
                let next = self.envelope + (held - self.envelope) * self.release;
                if held - next < SNAP { held } else { next }
            };
            // 4. Linear attack ramp across the look-ahead.
            let gain = self.attack(self.envelope);
            // 5. Apply to the frame that entered `lookahead` frames ago.
            let at = self.delay_pos * channels;
            for (sample, slot) in frame.iter_mut().zip(&mut self.delay[at..at + channels]) {
                let delayed = std::mem::replace(slot, *sample);
                // Always guard, never bypass on `gain == 1.0`: the gain can be
                // exactly 1.0 while the delayed sample is still hot (the attack
                // ramp's fast path when nothing is currently below 1.0), and a
                // bypass there would let a sample above the ceiling through.
                // `clamp` is exact for a sample already inside the ceiling, so
                // a quiet signal stays bit-exact.
                *sample = guard((f64::from(delayed) * gain) as f32, ceiling);
            }
            self.delay_pos += 1;
            if self.delay_pos == self.lookahead {
                self.delay_pos = 0;
            }
        }
    }

    fn reset(&mut self) {
        self.active = self.control.enabled();
        self.delay.fill(0.0);
        self.delay_pos = 0;
        self.wedge_head = 0;
        self.wedge_len = 0;
        self.frame = 0;
        self.envelope = 1.0;
        self.avg.fill(1.0);
        self.avg_pos = 0;
        self.avg_sum = self.window as f64;
        self.below = 0;
    }

    fn latency_frames(&self) -> u32 {
        if self.active {
            self.lookahead as u32
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    const FS: u32 = 48_000;

    /// Look-ahead in frames at [`FS`].
    const LA: usize = 240;

    /// Deterministic white-ish noise in `-0.5..0.5`, with a few exact edge
    /// values (signed zeros) mixed in.
    fn noise(samples: usize, seed: u32) -> Vec<f32> {
        let mut seed = seed;
        let mut out: Vec<f32> = (0..samples)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect();
        for (i, edge) in [0.0, -0.0].into_iter().enumerate() {
            if let Some(slot) = out.get_mut(i * 7) {
                *slot = edge;
            }
        }
        out
    }

    fn sine(freq: f64, amplitude: f64, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| (amplitude * (2.0 * PI * freq * n as f64 / f64::from(FS)).sin()) as f32)
            .collect()
    }

    fn stage(enabled: bool, channels: u16) -> Limiter {
        let mut limiter = Limiter::with_control(Arc::new(LimiterControl::new(enabled)));
        limiter.prepare(AudioSpec::new(FS, channels)).unwrap();
        limiter
    }

    fn bits(buf: &[f32]) -> Vec<u32> {
        buf.iter().map(|s| s.to_bits()).collect()
    }

    /// Process `buf` in blocks of varying size.
    fn process_in_blocks(limiter: &mut Limiter, buf: &mut [f32], channels: usize) {
        let sizes = [1, 7, 100, 4096, 333];
        let mut at = 0;
        let mut i = 0;
        while at < buf.len() {
            let len = (sizes[i % sizes.len()] * channels).min(buf.len() - at);
            limiter.process(&mut buf[at..at + len]);
            at += len;
            i += 1;
        }
    }

    #[test]
    fn disabled_is_a_bit_exact_bypass() {
        let mut limiter = stage(false, 2);
        assert_eq!(limiter.latency_frames(), 0);
        let input: Vec<f32> = noise(8192 * 2, 1).iter().map(|s| s * 8.0).collect();
        let mut out = input.clone();
        process_in_blocks(&mut limiter, &mut out, 2);
        assert_eq!(bits(&out), bits(&input));
    }

    #[test]
    fn the_default_stage_is_enabled() {
        let mut limiter = Limiter::new();
        limiter.prepare(AudioSpec::new(FS, 2)).unwrap();
        assert_eq!(limiter.latency_frames(), LA as u32);
        assert!(limiter.control().enabled());
        let input: Vec<f32> = noise(1024 * 2, 2).iter().map(|s| s * 8.0).collect();
        let mut out = input.clone();
        limiter.process(&mut out);
        assert!(out.iter().all(|s| s.abs() <= limiter.ceiling()));
    }

    #[test]
    fn a_quiet_signal_passes_bit_exact_when_enabled() {
        // Peaks of 0.5 are below the -0.3 dBFS ceiling: the output is the
        // input delayed by the look-ahead, bit for bit.
        let mut limiter = stage(true, 2);
        assert_eq!(limiter.latency_frames(), LA as u32);
        let input = noise(8192 * 2, 3);
        let mut out = input.clone();
        process_in_blocks(&mut limiter, &mut out, 2);
        assert!(out[..LA * 2].iter().all(|s| *s == 0.0));
        assert_eq!(bits(&out[LA * 2..]), bits(&input[..input.len() - LA * 2]));
    }

    #[test]
    fn a_hot_signal_is_held_at_the_ceiling() {
        let mut limiter = stage(true, 2);
        let ceiling = limiter.ceiling();
        assert!((f64::from(ceiling) - 10.0_f64.powf(CEILING_DB / 20.0)).abs() < 1e-7);
        let left = sine(1_000.0, 3.0, FS as usize);
        let right = noise(FS as usize, 4);
        let mut input: Vec<f32> = left
            .iter()
            .zip(right.iter())
            .flat_map(|(l, r)| [l + r, r * 4.0])
            .collect();
        // A silent tail lets the last look-ahead's worth of frames come out.
        input.extend(std::iter::repeat_n(0.0, 2 * LA * 2));
        let mut out = input.clone();
        process_in_blocks(&mut limiter, &mut out, 2);
        let peak = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(peak <= ceiling, "overshoot: {peak} > {ceiling}");
        assert!(
            peak >= ceiling * 0.999,
            "not brought to the ceiling: {peak}"
        );
    }

    #[test]
    fn transparency_returns_after_a_burst() {
        let mut limiter = stage(true, 1);
        let mut input = noise(2 * FS as usize, 5);
        input[..FS as usize / 10].iter_mut().for_each(|s| *s *= 8.0);
        let mut out = input.clone();
        process_in_blocks(&mut limiter, &mut out, 1);
        // The release (50 ms time constant) has long settled 1.5 s later.
        let tail = 3 * FS as usize / 2;
        assert_eq!(
            bits(&out[tail..]),
            bits(&input[tail - LA..input.len() - LA])
        );
    }

    #[test]
    fn look_ahead_shapes_the_transient_instead_of_clipping_it() {
        // A 100 Hz sine at 0.25 jumps to 4.0 (12 dB over) at a zero crossing.
        let onset = FS as usize / 2;
        let input: Vec<f32> = sine(100.0, 1.0, FS as usize)
            .iter()
            .enumerate()
            .map(|(n, s)| s * if n < onset { 0.25 } else { 4.0 })
            .collect();
        let mut limiter = stage(true, 1);
        let ceiling = limiter.ceiling();
        let mut out = input.clone();
        process_in_blocks(&mut limiter, &mut out, 1);
        assert!(out.iter().all(|s| s.abs() <= ceiling), "overshoot");

        // The applied gain, where it can be read off reliably.
        let gain = |n: usize| -> Option<f64> {
            let x = f64::from(input[n - LA]);
            (x.abs() > 0.05).then(|| f64::from(out[n]) / x)
        };
        // A gain envelope, not waveform clipping: the gain moves by at most
        // one window step per frame (a clipper's gain jumps within a period).
        let step = 1.0 / (LA + 1) as f64 + 1e-4;
        for n in LA + 1..out.len() {
            if let (Some(a), Some(b)) = (gain(n - 1), gain(n)) {
                assert!((b - a).abs() <= step, "gain jump at {n}: {a} -> {b}");
            }
        }
        // Look-ahead: 50 frames before the onset reaches the output, the gain
        // is already well on its way down.
        let early = gain(onset - 50 + LA).unwrap();
        assert!(early < 0.9, "no look-ahead: gain {early}");
        // The sample step stays bounded by the input's own step plus what the
        // gain ramp can add.
        let input_step = input
            .windows(2)
            .map(|w| f64::from((w[1] - w[0]).abs()))
            .fold(0.0, f64::max);
        let bound = input_step + 4.0 / (LA + 1) as f64 + 1e-4;
        let out_step = out
            .windows(2)
            .map(|w| f64::from((w[1] - w[0]).abs()))
            .fold(0.0, f64::max);
        assert!(out_step <= bound, "step {out_step} > {bound}");
    }

    #[test]
    fn non_finite_samples_are_silenced() {
        let mut limiter = stage(true, 2);
        let mut input = noise(4096 * 2, 6);
        input[1000] = f32::NAN;
        input[2001] = f32::INFINITY;
        input[3000] = f32::NEG_INFINITY;
        let mut out = input.clone();
        limiter.process(&mut out);
        assert!(
            out.iter()
                .all(|s| s.is_finite() && s.abs() <= limiter.ceiling())
        );
        assert_eq!(out[1000 + LA * 2], 0.0);
        // No poison stays behind: a clean block afterwards is finite too.
        let mut clean = noise(4096 * 2, 16);
        limiter.process(&mut clean);
        assert!(clean.iter().all(|s| s.is_finite()));
        assert!(clean.iter().all(|s| s.abs() <= limiter.ceiling()));
    }

    #[test]
    fn reset_clears_the_history() {
        let mut used = stage(true, 2);
        let mut warm: Vec<f32> = noise(8192 * 2, 7).iter().map(|s| s * 6.0).collect();
        used.process(&mut warm);
        used.reset();

        let mut fresh = stage(true, 2);
        let probe: Vec<f32> = noise(4096 * 2, 8).iter().map(|s| s * 3.0).collect();
        let mut a = probe.clone();
        let mut b = probe;
        used.process(&mut a);
        fresh.process(&mut b);
        assert_eq!(bits(&a), bits(&b));
    }

    #[test]
    fn switching_is_latched_until_reset() {
        let mut limiter = stage(false, 1);
        limiter.control().set_enabled(true);
        let input: Vec<f32> = noise(2048, 9).iter().map(|s| s * 8.0).collect();
        let mut out = input.clone();
        limiter.process(&mut out);
        assert_eq!(bits(&out), bits(&input), "still a bypass until reset");
        assert_eq!(limiter.latency_frames(), 0);

        limiter.reset();
        assert_eq!(limiter.latency_frames(), LA as u32);
        let mut out = input.clone();
        limiter.process(&mut out);
        assert!(out.iter().all(|s| s.abs() <= limiter.ceiling()));
    }

    /// DC below the ceiling is delayed bit-exact; DC above it is held at the
    /// ceiling. The first `lookahead` frames are the zero priming of the
    /// delay line.
    #[test]
    fn dc_is_delayed_exact_and_hot_dc_is_held() {
        let mut limiter = stage(true, 1);
        let ceiling = limiter.ceiling();
        assert!((ceiling - ceiling_linear()).abs() < 1e-9);

        let quiet = vec![0.5_f32; 4 * LA];
        let mut out = quiet.clone();
        limiter.process(&mut out);
        assert!(out[..LA].iter().all(|s| *s == 0.0));
        assert_eq!(bits(&out[LA..]), bits(&quiet[..quiet.len() - LA]));

        let mut limiter = stage(true, 1);
        let hot = vec![3.0_f32; 8 * LA];
        let mut out = hot.clone();
        limiter.process(&mut out);
        assert!(out[..LA].iter().all(|s| *s == 0.0));
        let settled = &out[4 * LA..];
        let peak = settled.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(peak <= ceiling, "overshoot: {peak} > {ceiling}");
        assert!(
            peak >= ceiling * 0.999,
            "settled {peak}, expected the ceiling {ceiling}"
        );
    }

    /// Subnormal signal values pass through an active limiter unchanged
    /// (delayed): they are finite, so no gain is asked and the exact `1.0`
    /// fast path keeps them bit for bit.
    #[test]
    fn subnormal_signal_passes_through_unchanged() {
        let mut limiter = stage(true, 1);
        let input: Vec<f32> = noise(4096, 17).iter().map(|s| s * 1e-40).collect();
        assert!(input.iter().all(|s| s.is_subnormal() || *s == 0.0));
        let mut out = input.clone();
        limiter.process(&mut out);
        assert!(out[..LA].iter().all(|s| *s == 0.0));
        assert_eq!(bits(&out[LA..]), bits(&input[..input.len() - LA]));
    }
}
