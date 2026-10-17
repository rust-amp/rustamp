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
//! have no look-ahead; `dhvani` is GPL-3.0-only. The true-peak detector is
//! hand-rolled for the same reasons: it is 64 precomputed coefficients and a
//! `2 * half`-float history per channel, it must be zero-phase and
//! DC-normalized, and no maintained crate ships a true-peak *detector*
//! decoupled from its own gain computer.
//!
//! # Topology (own engineering choice)
//!
//! Per frame, with `L` the look-ahead in frames and a window of `L + 1`
//! frames:
//!
//! 1. **Required gain.** The linked peak `p` of the incoming frame asks for
//!    `r = min(1, ceiling / p)`. `p` is the larger of the frame's **sample
//!    peak** (the largest `|sample|` over all channels) and its **true-peak
//!    estimate** (the next section). One gain for all channels keeps the
//!    stereo image where it is.
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
//! product; it never shapes the waveform. This is the **sample-peak**
//! guarantee; it is exact, and the true-peak criterion below leaves it
//! untouched (the linked peak is only ever larger than the sample peak).
//!
//! **Transparency:** while every frame's linked peak — sample peak and
//! true-peak estimate — is at or below the ceiling, `g` is exactly `1.0` (the
//! running sum is replaced by its exact value whenever no envelope entry is
//! below one), so the output is the input delayed by `L` frames, bit for
//! bit. After a reduction the envelope snaps to exactly `1.0` once it is
//! within `1e-9`, so transparency returns.
//!
//! **Non-finite input:** a frame holding a NaN or an infinity asks for gain
//! `0`; such samples come out as `0.0` rather than reaching the device. The
//! detector's history is sanitized the same way on the way in, so a
//! non-finite sample cannot poison the estimate with a NaN gain.
//!
//! # The true-peak criterion (own engineering choice)
//!
//! The sample peak of a frame is not the peak of the signal. The band-limited
//! waveform reconstructed *between* the samples — by the resampler after
//! this stage, or by the output side's DAC at equal rates — rises above it:
//! for a tone at a quarter of the sample rate that is sampled at its zero
//! crossings, by `√2` (+3 dB). That is the A14 measurement this stage now
//! answers (`docs/tasks/A14-dsp-correctness.md`): the tone reads 0.7071 at
//! the source rate, the converter rebuilds it to ≈0.99 (+40 %), and driven
//! to the ceiling it would exceed full scale by 2.7 dB — flat-topped at
//! exactly 1.0 by the converter's safety net. The sidechain therefore asks
//! for the peak of the *reconstructed* signal, estimated by oversampling:
//!
//! * **How it detects:** for every frame, a polyphase interpolator evaluates
//!   the band-limited reconstruction at [`TRUE_PEAK_OVERSAMPLE`]
//!   sub-positions of the newest interval whose kernel support is fully
//!   inside the detector's history. The linked (largest over channels)
//!   maximum of those values is the frame's true-peak estimate, and the
//!   required gain is `min(1, ceiling / max(sample peak, estimate))`.
//! * **How much oversampling, and why:** 4× — the factor ITU-R BS.1770-4
//!   specifies for true-peak measurement. The 4× grid under-reads a tone's
//!   true peak by at most `1 - cos(pi f / (4 fs))`: 0.17 dB at a quarter of
//!   the sample rate (the A14 anchor), growing to 0.67 dB at Nyquist, where
//!   the resampler's own transition band attenuates by more. 8× would halve
//!   the under-read at twice the cost; the −0.3 dB ceiling margin absorbs
//!   the in-band part of the 4× residual.
//! * **The kernel:** a symmetric Blackman-Harris-windowed sinc of half-length
//!   [`TRUE_PEAK_HALF_TAPS`] (16 taps per phase) — the same window family
//!   the converter's resampler uses — DC-normalized so every phase sums to
//!   one (DC passes at unity) and phase 0 reproduces the sample itself.
//!   Zero-phase, so the only delay is the detector's own.
//! * **The detector's delay:** a zero-phase estimate of the interval
//!   `[n, n+1)` needs samples up to `n + half`, so the estimate pushed with
//!   frame `f` describes the interval `[f - half, f - half + 1)` — `half`
//!   frames of detector delay, always inside the look-ahead (`half` is
//!   clamped to it). The delay costs no latency: the detector is sidechain
//!   only, the signal path is the same delay line as before, and
//!   [`DspStage::latency_frames`] is unchanged.
//! * **The cost:** sidechain CPU only. Per frame the detector adds
//!   `TRUE_PEAK_OVERSAMPLE * 2 * TRUE_PEAK_HALF_TAPS` f64 multiply-adds per
//!   channel — 64 per channel at the defaults, ≈6M f64 FMAs/s at 48 kHz
//!   stereo, a small fraction of one core. No benchmark was run (the host
//!   runs no tests): the op count is stated, the wall clock is not. State
//!   grows by `2 * half` f32 per channel plus the coefficient table;
//!   `prepare` allocates, `process` never does.
//!
//! # What the criterion cannot promise
//!
//! * The **sample-peak guarantee is exact and unchanged**: the proof above
//!   only uses `required <= min(1, ceiling / sample peak)`, which the larger
//!   linked peak preserves. The **true-peak bound is an estimate**: the 4×
//!   grid under-reads by up to 0.17 dB at fs/4 (0.67 dB at Nyquist) before
//!   the kernel's own error, and the detector's `half`-frame delay dilutes
//!   the attack average by up to `half / (L + 1)` of the gain deficit on
//!   step transients whose preceding `half` frames are quiet (sustained
//!   tones: no dilution — every term of the average carries the bound). The
//!   converter's final `±1.0` ceiling in `rustamp-audio` stays as the last
//!   resort for whatever the estimate under-reads; it is not removed here.
//! * **Downsampling over-reads.** The estimator models the full-band
//!   reconstruction at this stage's rate; when the device rate is lower,
//!   content the resampler will remove is counted, so the limiter may limit
//!   a little more than strictly needed. Conservative, never the other way;
//!   modelling the device rate would couple the stage to a spec it does not
//!   see.
//! * **Equal rates change the sound on hot full-band material.** The
//!   criterion now also bounds the inter-sample overshoot the output side's
//!   own reconstruction produces — correct for a true-peak limiter, but the
//!   sample-peak stage was transparent there. Quiet material is unaffected:
//!   its true peak is its sample peak, or below the ceiling.
//! * **Discontinuities.** Within `2 * half` frames of a stream start (or any
//!   discontinuity) the kernel's truncation error is a few percent, so the
//!   held true peak there is the ceiling ± a few percent; where the missing
//!   history inflates the estimate the stage limits a little more, never
//!   less. The converter's net still bounds the absolute output.
//!
//! # Own-choice parameters
//!
//! * [`CEILING_DB`] `= -0.3 dBFS`: a ceiling just under full scale, now on
//!   the **linked peak** — the sample peak and its true-peak estimate. The
//!   margin absorbs the estimator's in-band under-read (≤ 0.17 dB at fs/4
//!   for the 4× grid) and keeps ordinary program material out of the
//!   converter's final `±1.0` (0 dBFS) safety net, which stays in place as
//!   the last resort for what the estimate under-reads near Nyquist (the
//!   [`FormatConverter`] in `rustamp-audio`; not removed here).
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
//! inter-sample peaks above a *sample*-peak ceiling — this stage now sees
//! that rebuild at the source rate (the true-peak criterion above) and
//! limits it, which is what keeps the converter's final `±1.0` ceiling —
//! still in place — from having to flat-top the signal. Inserting it is the
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

use std::f64::consts::PI;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rustamp_core::audio::{AudioSpec, DspStage};
use rustamp_core::error::Result;

use crate::sanitize_sample;

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

/// Oversampling factor of the true-peak sidechain (own choice: 4×, the
/// factor ITU-R BS.1770-4 specifies for true-peak measurement; see the
/// module docs for the residual it leaves and why 4× is the trade-off).
pub const TRUE_PEAK_OVERSAMPLE: usize = 4;

/// Half-length of the true-peak detector's interpolation kernel, in input
/// frames (own choice: 8, i.e. 16 taps per phase — the same window family
/// the converter's resampler uses). Clamped to the look-ahead in `prepare`
/// so the detector's delay never exceeds it.
pub const TRUE_PEAK_HALF_TAPS: usize = 8;

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
    /// True-peak detector: kernel half-length in frames (>= 1, <= lookahead).
    half_taps: usize,
    /// Detector taps: `TRUE_PEAK_OVERSAMPLE` rows of `2 * half_taps`
    /// coefficients, one row per sub-position.
    taps: Vec<f64>,
    /// Detector history: the last `2 * half_taps` input frames, sanitized
    /// (a non-finite sample enters as `0.0`).
    hist: Vec<f32>,
    /// Ring position (in frames) the next input frame is written to.
    hist_pos: usize,
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
            half_taps: 1,
            taps: Vec::new(),
            hist: Vec::new(),
            hist_pos: 0,
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

    /// Push `frame` into the detector's history and return the linked (largest
    /// over channels) true-peak estimate of the newest interval whose kernel
    /// support is fully inside the history: the interval
    /// `[f - half_taps, f - half_taps + 1)` for the frame `f` just pushed.
    /// Always finite: the history is sanitized on the way in.
    fn detector_peak(&mut self, frame: &[f32]) -> f64 {
        let channels = self.channels;
        let half = self.half_taps;
        let frames = 2 * half; // history length in frames
        let width = 2 * half; // taps per phase
        // Push the frame (sanitized: a non-finite sample is silence and must
        // not poison the kernel's history with a NaN estimate).
        let at = self.hist_pos * channels;
        for (slot, sample) in self.hist[at..at + channels].iter_mut().zip(frame.iter()) {
            *slot = sanitize_sample(*sample);
        }
        self.hist_pos += 1;
        if self.hist_pos == frames {
            self.hist_pos = 0;
        }
        // The FIR for the interval [n, n + 1) reads the samples
        // [n - half + 1, n + half]; with n = f - half those are exactly the
        // ring's frames, oldest first, at slots hist_pos + k (mod frames).
        let mut peak = 0.0_f64;
        for taps in self.taps.chunks_exact(width) {
            for channel in 0..channels {
                let mut acc = 0.0_f64;
                for (k, tap) in taps.iter().enumerate() {
                    let slot = ((self.hist_pos + k) % frames) * channels + channel;
                    acc += tap * f64::from(self.hist[slot]);
                }
                let a = acc.abs();
                if a > peak {
                    peak = a;
                }
            }
        }
        peak
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

/// The polyphase coefficient table of the true-peak detector: `oversample`
/// rows of `2 * half` taps. Row `j` estimates the band-limited
/// reconstruction at the sub-position `j / oversample` of an interval, from
/// a symmetric Blackman-Harris-windowed sinc kernel of half-length `half`
/// (the window family the converter's resampler uses). Every row is
/// normalized to sum to one, so DC passes at unity and row 0 reproduces the
/// sample itself.
fn true_peak_taps(oversample: usize, half: usize) -> Vec<f64> {
    let width = 2 * half;
    let mut taps = vec![0.0; oversample * width];
    for (phase, row) in taps.chunks_exact_mut(width).enumerate() {
        let offset = phase as f64 / oversample as f64;
        for (k, tap) in row.iter_mut().enumerate() {
            // Kernel argument: u = half - 1 + offset - k, with |u| < half.
            let u = half as f64 - 1.0 + offset - k as f64;
            *tap = windowed_sinc(u, half);
        }
        let sum: f64 = row.iter().sum();
        // A sane kernel sums to ~1; a broken one must not silently become
        // the criterion. The bounds hold for any real rate and half-length.
        debug_assert!((0.5..1.5).contains(&sum), "kernel row sums to {sum}");
        for tap in row.iter_mut() {
            *tap /= sum;
        }
    }
    taps
}

/// `sinc(u)` times a four-term Blackman-Harris window over `|u| < half`.
fn windowed_sinc(u: f64, half: usize) -> f64 {
    let limit = half as f64;
    if u.abs() >= limit {
        return 0.0;
    }
    // Window position across the support, 0..1.
    let x = (u + limit) / (2.0 * limit);
    let window = 0.35875 - 0.48829 * (2.0 * PI * x).cos() + 0.14128 * (4.0 * PI * x).cos()
        - 0.01168 * (6.0 * PI * x).cos();
    let sinc = if u == 0.0 {
        1.0
    } else {
        (PI * u).sin() / (PI * u)
    };
    sinc * window
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
        // The detector's kernel half-length, clamped to the look-ahead so its
        // delay never exceeds it (see the module docs).
        self.half_taps = TRUE_PEAK_HALF_TAPS.clamp(1, self.lookahead);
        self.taps = true_peak_taps(TRUE_PEAK_OVERSAMPLE, self.half_taps);
        self.hist = vec![0.0; 2 * self.half_taps * self.channels];
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
            // 1. Linked peak of the incoming frame: the larger of its sample
            //    peak (NaN counts as infinite) and its true-peak estimate.
            let mut sample_peak = 0.0_f32;
            for sample in frame.iter() {
                let a = sample.abs();
                if a.is_nan() {
                    sample_peak = f32::INFINITY;
                } else if a > sample_peak {
                    sample_peak = a;
                }
            }
            let true_peak = self.detector_peak(frame);
            let peak = f64::from(sample_peak).max(true_peak);
            let required = if peak <= f64::from(ceiling) {
                1.0
            } else if peak.is_finite() {
                f64::from(ceiling) / peak
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
        self.hist.fill(0.0);
        self.hist_pos = 0;
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
        stage_at(enabled, channels, FS)
    }

    /// A stage prepared at an arbitrary sample rate.
    fn stage_at(enabled: bool, channels: u16, rate: u32) -> Limiter {
        let mut limiter = Limiter::with_control(Arc::new(LimiterControl::new(enabled)));
        limiter.prepare(AudioSpec::new(rate, channels)).unwrap();
        limiter
    }

    /// The look-ahead in frames at `rate` (the stage's own formula).
    fn lookahead_at(rate: u32) -> usize {
        ((LOOKAHEAD_SECONDS * f64::from(rate)).round() as usize).max(1)
    }

    fn bits(buf: &[f32]) -> Vec<u32> {
        buf.iter().map(|s| s.to_bits()).collect()
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
    }

    /// The A14 band-edge tone at any rate: `S·sin(πn/2 + π/4)` has samples of
    /// exactly `±S·√2/2` and a true peak of `S` — a rebuild of `√2` (+3 dB)
    /// over the sample peak that a sample-peak criterion cannot see.
    fn a6_tone(s: f64, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|n| (s * (std::f64::consts::FRAC_PI_4 + PI * n as f64 / 2.0)).sin() as f32)
            .collect()
    }

    /// Test-local reference true-peak estimate: the ideal band-limited
    /// reconstruction evaluated at `sub` sub-positions per frame with a
    /// 65-tap Blackman-Harris-windowed sinc (the module's own kernel at four
    /// times the half-length and four times the sub-positions of the stage's
    /// detector), so its own error is negligible against every bound
    /// asserted here. Zero outside the buffer, like the stage's zero-primed
    /// history.
    fn reference_true_peak(buf: &[f32], sub: usize) -> f64 {
        const HALF: usize = 4 * TRUE_PEAK_HALF_TAPS; // 65 taps
        let width = 2 * HALF + 1;
        // Kernel table: sub-position j, tap distance d in -HALF..=HALF.
        let kernel: Vec<f64> = (0..sub)
            .flat_map(|j| {
                (-(HALF as isize)..=(HALF as isize))
                    .map(move |d| windowed_sinc(d as f64 + j as f64 / sub as f64, HALF))
            })
            .collect();
        let mut peak = 0.0_f64;
        for n in 0..buf.len() {
            for j in 0..sub {
                let row = &kernel[j * width..(j + 1) * width];
                let mut acc = 0.0_f64;
                for (d, tap) in row.iter().enumerate() {
                    let d = d as isize - HALF as isize;
                    let m = n as isize + d;
                    if let Ok(m) = usize::try_from(m)
                        && let Some(sample) = buf.get(m)
                    {
                        acc += tap * f64::from(*sample);
                    }
                }
                peak = peak.max(acc.abs());
            }
        }
        peak
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
        // "Quiet" now means: the *linked* peak — sample peak and true-peak
        // estimate — is below the -0.3 dBFS ceiling. A band-limited sine's
        // true peak is its amplitude; white noise's reconstruction peaks
        // above its samples, so the noise is scaled until the kernel's own
        // L1 bound (max |sample| * sum |taps| ≈ 0.46) is far below the
        // ceiling. The output is the input delayed by the look-ahead, bit
        // for bit.
        let mut limiter = stage(true, 2);
        assert_eq!(limiter.latency_frames(), LA as u32);
        let frames = 4 * FS as usize;
        let left = sine(1_000.0, 0.5, frames);
        let right: Vec<f32> = noise(frames, 3).iter().map(|s| s * 0.25).collect();
        let input: Vec<f32> = left
            .iter()
            .zip(right.iter())
            .flat_map(|(l, r)| [*l, *r])
            .collect();
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
        let frames = FS as usize;
        let left = sine(1_000.0, 3.0, frames);
        // The right channel's noise stays far below the sine in *true* peak
        // (its estimate is bounded by max |sample| * sum |taps| ≈ 0.46), so
        // the linked criterion is set by the sine, as its sample peak set it
        // before.
        let right: Vec<f32> = noise(frames, 4).iter().map(|s| s * 0.5).collect();
        let mut input: Vec<f32> = left
            .iter()
            .zip(right.iter())
            .flat_map(|(l, r)| [*l, *r])
            .collect();
        // A silent tail lets the last look-ahead's worth of frames come out.
        input.extend(std::iter::repeat_n(0.0, 2 * LA * 2));
        let mut out = input.clone();
        process_in_blocks(&mut limiter, &mut out, 2);
        let peak = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        assert!(peak <= ceiling, "overshoot: {peak} > {ceiling}");
        // The limiter brought the signal *to* the ceiling, not below it. The
        // sample peak of the output is ceiling * (sample peak / linked peak);
        // for a 1 kHz sine at 48 kHz the samples hit the sine's peak exactly,
        // so the linked peak is the sample peak up to the kernel's own error
        // (≤ 0.1 %), and the output sample peak lands within 0.1 % of the
        // ceiling. The true-peak criterion's own target — the estimated true
        // peak held at the ceiling — is pinned by the band-edge tests below.
        assert!(peak >= ceiling * 0.99, "not brought to the ceiling: {peak}");
    }

    #[test]
    fn transparency_returns_after_a_burst() {
        let mut limiter = stage(true, 1);
        // A hot burst (±8) forces a reduction; the tail is quiet in *true*
        // peak (±0.125 white noise, L1-bound ≈ 0.23), so the released
        // envelope snaps back to exactly 1.0 and the tail passes bit for bit.
        let mut input: Vec<f32> = noise(2 * FS as usize, 5).iter().map(|s| s * 0.25).collect();
        input[..FS as usize / 10]
            .iter_mut()
            .for_each(|s| *s *= 64.0);
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

    /// The A14 anchor, S = 1.0: `x[n] = sin(πn/2 + π/4)` at 44.1 kHz — samples
    /// of exactly `±√2/2`, a true peak of 1.0. The sample peak (0.7071) is far
    /// below the ceiling, so the old sample-peak criterion passed the tone
    /// untouched and the converter rebuilt it to ≈0.99 (+40 %). The true-peak
    /// criterion sees the rebuild at the source rate and limits it.
    #[test]
    fn the_true_peak_criterion_catches_the_band_edge_tone() {
        let rate = 44_100_u32;
        let la = lookahead_at(rate);
        let mut limiter = stage_at(true, 1, rate);
        // The criterion adds no latency: the detector is sidechain only.
        assert_eq!(limiter.latency_frames(), la as u32);
        let ceiling = limiter.ceiling();

        let input = a6_tone(1.0, rate as usize);
        let sample_peak = peak(&input);
        // The sample-peak criterion is blind: 0.7071 is far below the ceiling.
        assert!((sample_peak - std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6);
        assert!(sample_peak < ceiling);
        // ... and the true peak is a full 1.0: the +3 dB the sample peak
        // misses (the number the converter used to rebuild).
        let true_peak = reference_true_peak(&input, 16);
        assert!(
            (true_peak - 1.0).abs() < 0.03,
            "reference true peak {true_peak}"
        );
        assert!(
            true_peak > 1.3 * f64::from(sample_peak),
            "rebuild ratio {true_peak} / {sample_peak}"
        );

        // A silent tail lets the look-ahead's worth of frames come out.
        let mut input = input;
        input.extend(std::iter::repeat_n(0.0, 2 * la));
        let mut out = input.clone();
        process_in_blocks(&mut limiter, &mut out, 1);

        // The guarantee that never changed: no output sample exceeds the
        // ceiling, exactly.
        assert!(out.iter().all(|s| s.abs() <= ceiling));
        // The criterion engaged: the output is attenuated although every
        // input sample was below the ceiling.
        let out_peak = peak(&out);
        assert!(out_peak < sample_peak, "no engagement: {out_peak}");
        // The settled output: the tone pulled down so that its *true* peak
        // sits at the ceiling — the gain is ceiling / true peak ≈ 0.966, so
        // the sample peak lands at 0.7071 * 0.966 ≈ 0.683.
        let settled = &out[la + 64..out.len() - la - 64];
        let settled_peak = peak(settled);
        assert!(
            (0.66..0.70).contains(&settled_peak),
            "settled sample peak {settled_peak}"
        );
        let gain = f64::from(settled_peak) / f64::from(sample_peak);
        assert!((0.94..0.99).contains(&gain), "settled gain {gain}");
        // The limited signal's true peak sits at the ceiling and below full
        // scale: the converter's safety net cannot engage on this signal.
        let settled_tp = reference_true_peak(settled, 16);
        assert!(
            (0.94..0.99).contains(&settled_tp),
            "settled true peak {settled_tp}, ceiling {ceiling}"
        );
    }

    /// The A14 torture anchor, S = 1.3662: samples at exactly the ceiling
    /// (`±0.96605`), a true peak of 1.3662 (+2.7 dBFS). The old criterion
    /// passed the tone bit for bit and the converter flat-topped the rebuild
    /// at exactly 1.0; the true-peak criterion pulls the tone down by the
    /// rebuild itself (gain ≈ 0.707, −3.0 dB) so the resampled peak stays
    /// below full scale and nothing is flat-topped.
    #[test]
    fn the_limiter_pulls_the_torture_tone_down_instead_of_passing_it() {
        let rate = 44_100_u32;
        let la = lookahead_at(rate);
        let mut limiter = stage_at(true, 1, rate);
        let ceiling = limiter.ceiling();

        let input = a6_tone(1.3662, rate as usize);
        let sample_peak = peak(&input);
        // The sample-peak criterion passes the tone: its samples sit at the
        // ceiling (the bound the A14 converter test pins as well).
        assert!(
            (sample_peak - ceiling).abs() <= 1e-3,
            "sample peak {sample_peak}, ceiling {ceiling}"
        );
        let true_peak = reference_true_peak(&input, 16);
        assert!(
            (true_peak - 1.3662).abs() < 0.03,
            "reference true peak {true_peak}"
        );
        assert!(true_peak > f64::from(ceiling), "true peak {true_peak}");

        let mut input = input;
        input.extend(std::iter::repeat_n(0.0, 2 * la));
        let mut out = input.clone();
        process_in_blocks(&mut limiter, &mut out, 1);

        assert!(out.iter().all(|s| s.abs() <= ceiling));
        // Engagement: the tone is attenuated although its samples sat exactly
        // at the ceiling.
        let out_peak = peak(&out);
        assert!(out_peak < sample_peak, "no engagement: {out_peak}");
        // The settled output: the sample peak is pulled from 0.966 to ≈0.683,
        // the true peak is held at the ceiling, and nothing reaches full
        // scale — the converter's net has nothing to flat-top. (The chain
        // test in `rustamp-audio` measures the resampled peak and the clamp
        // count; expected ≈0.967 and 0, against +2.7 dBFS and ~47 % of
        // samples clamped before this change.)
        let settled = &out[la + 64..out.len() - la - 64];
        let settled_peak = peak(settled);
        assert!(
            (0.66..0.70).contains(&settled_peak),
            "settled sample peak {settled_peak}"
        );
        let gain = f64::from(settled_peak) / f64::from(sample_peak);
        assert!((0.69..0.73).contains(&gain), "settled gain {gain}");
        let settled_tp = reference_true_peak(settled, 16);
        assert!(
            (0.94..0.99).contains(&settled_tp),
            "settled true peak {settled_tp}, ceiling {ceiling}"
        );
    }

    /// The detector's kernel, pinned analytically: every phase sums to one
    /// (DC passes at unity), phase 0 reproduces the sample itself, and the
    /// symmetric kernel mirrors between the phases `j` and `K - j`.
    #[test]
    fn the_detector_kernel_is_dc_normalized_and_sample_exact() {
        let taps = true_peak_taps(TRUE_PEAK_OVERSAMPLE, TRUE_PEAK_HALF_TAPS);
        let width = 2 * TRUE_PEAK_HALF_TAPS;
        assert_eq!(taps.len(), TRUE_PEAK_OVERSAMPLE * width);
        for (phase, row) in taps.chunks_exact(width).enumerate() {
            let sum: f64 = row.iter().sum();
            assert!((sum - 1.0).abs() < 1e-12, "phase {phase} sums to {sum}");
            if phase == 0 {
                // The sinc is zero at every integer offset but the centre,
                // so phase 0 is the sample itself.
                for (k, tap) in row.iter().enumerate() {
                    let want = if k == TRUE_PEAK_HALF_TAPS - 1 {
                        1.0
                    } else {
                        0.0
                    };
                    assert!((tap - want).abs() < 1e-12, "phase 0 tap {k}: {tap}");
                }
            }
        }
        for j in 1..TRUE_PEAK_OVERSAMPLE {
            let a = &taps[j * width..(j + 1) * width];
            let b =
                &taps[(TRUE_PEAK_OVERSAMPLE - j) * width..(TRUE_PEAK_OVERSAMPLE - j + 1) * width];
            for (x, y) in a.iter().zip(b.iter().rev()) {
                assert!(
                    (x - y).abs() < 1e-12,
                    "phases {j} and {} mirror",
                    TRUE_PEAK_OVERSAMPLE - j
                );
            }
        }
    }

    /// The true-peak criterion is linked over channels: a band-edge tone on
    /// one channel and silence on the other is limited by the tone's
    /// inter-sample peak.
    #[test]
    fn the_true_peak_criterion_is_linked_over_channels() {
        let rate = 44_100_u32;
        let la = lookahead_at(rate);
        let mut limiter = stage_at(true, 2, rate);
        let ceiling = limiter.ceiling();
        let tone = a6_tone(1.3662, rate as usize);
        let mut input: Vec<f32> = tone.iter().flat_map(|s| [*s, 0.0]).collect();
        input.extend(std::iter::repeat_n(0.0, 2 * la * 2));
        let mut out = input.clone();
        process_in_blocks(&mut limiter, &mut out, 2);

        assert!(out.iter().all(|s| s.abs() <= ceiling));
        // Silence in, silence out on the right channel.
        assert!(out.iter().skip(1).step_by(2).all(|s| *s == 0.0));
        // The left channel's settled output is held at the ceiling's true
        // peak, exactly as in the mono torture test.
        let left: Vec<f32> = out.iter().step_by(2).copied().collect();
        let settled = &left[la + 64..left.len() - la - 64];
        let settled_tp = reference_true_peak(settled, 16);
        assert!(
            (0.94..0.99).contains(&settled_tp),
            "linked true peak {settled_tp}, ceiling {ceiling}"
        );
    }

    /// A NaN in the input silences its frame (the sample-peak rule) and must
    /// not poison the detector's history either: the estimate stays finite
    /// and the criterion still engages on the tone around it.
    #[test]
    fn a_nan_does_not_poison_the_true_peak_detector() {
        let rate = 44_100_u32;
        let la = lookahead_at(rate);
        let mut limiter = stage_at(true, 1, rate);
        let ceiling = limiter.ceiling();
        let mut input = a6_tone(1.3662, rate as usize);
        input[1000] = f32::NAN;
        input.extend(std::iter::repeat_n(0.0, 2 * la));
        let mut out = input.clone();
        process_in_blocks(&mut limiter, &mut out, 1);

        assert!(out.iter().all(|s| s.is_finite() && s.abs() <= ceiling));
        // The tone is still limited (the sanitized history keeps the
        // estimate finite, so the criterion still sees the rebuild).
        let settled = &out[la + 64..out.len() - la - 64];
        let settled_peak = peak(settled);
        assert!(
            (0.66..0.70).contains(&settled_peak),
            "settled sample peak {settled_peak}"
        );
    }

    /// At a rate whose look-ahead is shorter than the kernel's half-length
    /// the half-length clamps to it; the sample-peak guarantee does not
    /// depend on the kernel at all.
    #[test]
    fn a_low_rate_prepare_still_holds_the_ceiling() {
        let mut limiter = stage_at(true, 1, 1_000);
        assert_eq!(limiter.latency_frames(), 5);
        let input: Vec<f32> = noise(4096, 21).iter().map(|s| s * 8.0).collect();
        let mut out = input.clone();
        limiter.process(&mut out);
        assert!(out.iter().all(|s| s.abs() <= limiter.ceiling()));
    }
}
