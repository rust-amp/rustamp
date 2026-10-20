// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Seeded property tests for the DSP stage math.
//!
//! The unit tests next to each stage pin hand-written cases. This file
//! states the invariants the stages promise as properties over randomly
//! drawn signals and settings, with a seeded generator, so that a failure
//! is reproducible and the crate needs no dependency.
//!
//! # The invariants
//!
//! * **Bypass is exact** (`eq`): a disabled stage, and an enabled but flat
//!   stage, return the input bit for bit.
//! * **The crossfade is exact** (`eq`): the wet/dry mix covers
//!   `FADE_SECONDS * rate` frames, from the old mix to the new one, both
//!   exactly; the first frame of a crossfade is the dry signal, bit for bit.
//! * **The fade ramp is exact** (`fade`): the first frame of a ramp carries
//!   `from` and the last one `to`, both exactly; the ramp covers
//!   `round(duration_ms * rate / 1000)` frames whatever the block size is;
//!   the gain follows the equal-power law and stays between the two
//!   endpoints.
//! * **Nothing non-finite leaves** (`eq`, `fade`, `limiter`): a finite input
//!   and an in-range setting give a finite output.
//! * **The gain moves at a bounded rate** (`eq`, `limiter`): the equalizer
//!   approaches a target at the documented slew, and the limiter's gain
//!   moves by at most `1 / (lookahead + 1)` per frame.
//! * **No blow-up** (`eq`): a long random signal stays inside the cascade's
//!   own magnitude bound.
//! * **The block size does not matter** (all three stages): the same signal
//!   through the same stage gives the same output, however the frames are
//!   split into buffers.
//!
//! # What is not claimed
//!
//! * The equalizer does **not** clip: a boost may push a sample past full
//!   scale, and that is documented (`eq`). The bounds here are bounds of the
//!   *cascade's* gain, not of full scale.
//! * The fade multiplies: it does not sanitize. A non-finite *input* stays
//!   non-finite on the way out, because the stage's policy is "a gain
//!   ramp", and the non-finite policy of the chain lives at its input (the
//!   decode thread). Every property below is stated for a finite input.
//! * `DspStage::process` is documented to get whole frames ("`buf.len()` is a
//!   multiple of the prepared channel count"), so every block below holds a
//!   whole number of frames.

use std::f64::consts::PI;
use std::sync::Arc;

use rustamp_core::audio::{AudioSpec, DspStage};
use rustamp_core::state::{EQ_BANDS, EqState};

use rustamp_dsp::eq::{
    BAND_CENTERS_HZ, BAND_Q, Equalizer, FADE_SECONDS, NYQUIST_GUARD, SLEW_SECONDS, SUB_BLOCK,
};
use rustamp_dsp::fade::{Fade, FadeControl};
use rustamp_dsp::limiter::{LOOKAHEAD_SECONDS, Limiter, LimiterControl};

/// How many random scenarios one property runs.
const SEEDS: u64 = 16;

/// The sample rates the properties draw from.
const RATES: [u32; 4] = [8_000, 44_100, 48_000, 96_000];

// ---------------------------------------------------------------------------
// The seeded generator
// ---------------------------------------------------------------------------

/// A seeded xorshift64 generator.
///
/// The property suite must be reproducible from its seed (a failure has to
/// be replayable) and the crate takes no dependency, so the generator lives
/// here. It is not a quality generator: it only has to spread the drawn
/// values evenly enough for the properties to mean something.
struct Rng(u64);

impl Rng {
    /// A generator for `seed`. The state must not be zero.
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A uniform value in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    /// A uniform value in `[lo, hi)`.
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// A uniform index in `0..n`.
    fn index(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The bit patterns of a buffer, for an exact comparison.
fn bits(buf: &[f32]) -> Vec<u32> {
    buf.iter().map(|s| s.to_bits()).collect()
}

/// The root mean square of a buffer.
fn rms(buf: &[f32]) -> f64 {
    (buf.iter()
        .map(|s| f64::from(*s) * f64::from(*s))
        .sum::<f64>()
        / buf.len() as f64)
        .sqrt()
}

/// The largest absolute value in a buffer.
fn peak(buf: &[f32]) -> f64 {
    buf.iter()
        .map(|s| f64::from(s.abs()))
        .fold(0.0_f64, f64::max)
}

/// The largest frame-to-frame movement in a buffer.
fn max_step(buf: &[f32]) -> f64 {
    buf.windows(2)
        .map(|w| (f64::from(w[1]) - f64::from(w[0])).abs())
        .fold(0.0_f64, f64::max)
}

/// A random signal in the audio domain, with the exact edge values mixed in.
fn random_signal(rng: &mut Rng, frames: usize, channels: usize) -> Vec<f32> {
    let mut out: Vec<f32> = (0..frames * channels)
        .map(|_| rng.range(-1.0, 1.0))
        .collect();
    for (i, edge) in [0.0_f32, -0.0, 1.0, -1.0, f32::MIN_POSITIVE]
        .into_iter()
        .enumerate()
    {
        if let Some(slot) = out.get_mut(i * 13) {
            *slot = edge;
        }
    }
    out
}

/// A random *finite* signal: the audio domain plus the largest and the
/// smallest finite float, so the "nothing non-finite leaves" properties
/// cover the whole finite range and not only the audio domain.
fn random_finite_signal(rng: &mut Rng, frames: usize, channels: usize) -> Vec<f32> {
    let mut out = random_signal(rng, frames, channels);
    for (i, edge) in [
        0.0_f32,
        -0.0,
        1.0,
        -1.0,
        f32::MIN_POSITIVE,
        f32::MAX,
        f32::MIN,
        -f32::MAX,
    ]
    .into_iter()
    .enumerate()
    {
        if let Some(slot) = out.get_mut(i * 13) {
            *slot = edge;
        }
    }
    out
}

/// Feed `buf` through `stage` in blocks of a random number of frames.
///
/// Every block holds whole frames, as `DspStage::process` requires.
fn process_in_random_blocks(
    stage: &mut dyn DspStage,
    buf: &mut [f32],
    channels: usize,
    rng: &mut Rng,
) {
    let mut at = 0;
    while at < buf.len() {
        let frames = (rng.index(32) + 1).min((buf.len() - at) / channels).max(1);
        let len = frames * channels;
        stage.process(&mut buf[at..at + len]).expect("process");
        at += len;
    }
}

/// A random equalizer state: every band and the preamp inside the engine's
/// range, and the on/off flag at random.
fn random_eq_state(rng: &mut Rng, enabled: Option<bool>) -> EqState {
    let mut bands = [0.0_f32; EQ_BANDS];
    for band in &mut bands {
        *band = rng.range(-EqState::MAX_GAIN_DB, EqState::MAX_GAIN_DB);
    }
    EqState {
        bands,
        preamp: rng.range(-EqState::MAX_GAIN_DB, EqState::MAX_GAIN_DB),
        enabled: enabled.unwrap_or_else(|| rng.index(2) == 0),
        auto: rng.index(2) == 0,
    }
}

// ---------------------------------------------------------------------------
// The oracle: the cascade's own magnitude response
// ---------------------------------------------------------------------------

/// The coefficients of one peaking section: `b0`, `b1`, `b2`, `a1`, `a2`.
///
/// The textbook "Audio EQ Cookbook" formula, written out here as an
/// independent oracle for the bound the stage's own coefficients must
/// respect. The stage computes the same formula in `f64`; this copy exists
/// so that the bound does not have to be taken on trust.
fn section_coefficients(db: f64, f0: f64, q: f64, rate: f64) -> (f64, f64, f64, f64, f64) {
    let a = 10.0_f64.powf(db / 40.0);
    let w0 = 2.0 * PI * f0 / rate;
    let (sin, cos) = w0.sin_cos();
    let alpha = sin / (2.0 * q);
    let a0 = 1.0 + alpha / a;
    (
        (1.0 + alpha * a) / a0,
        -2.0 * cos / a0,
        (1.0 - alpha * a) / a0,
        -2.0 * cos / a0,
        (1.0 - alpha / a) / a0,
    )
}

/// The magnitude of one peaking section at the frequency `f`.
fn section_magnitude(db: f64, f0: f64, q: f64, rate: f64, f: f64) -> f64 {
    let (b0, b1, b2, a1, a2) = section_coefficients(db, f0, q, rate);
    let w = 2.0 * PI * f / rate;
    let (s1, c1) = w.sin_cos();
    let (s2, c2) = (2.0 * w).sin_cos();
    let num_r = b0 + b1 * c1 + b2 * c2;
    let num_i = b1 * s1 + b2 * s2;
    let den_r = 1.0 + a1 * c1 + a2 * c2;
    let den_i = a1 * s1 + a2 * s2;
    (num_r * num_r + num_i * num_i).sqrt() / (den_r * den_r + den_i * den_i).sqrt()
}

/// The L1 norm of one section's impulse response.
///
/// The sound bound for the *peak* of the output: a filter cannot raise a
/// sample by more than the sum of the absolute values of its impulse
/// response. (The largest magnitude of the response is not a bound for the
/// peak: that one holds for the energy, not for the peak.)
fn section_l1(db: f64, f0: f64, q: f64, rate: f64) -> f64 {
    let (b0, b1, b2, a1, a2) = section_coefficients(db, f0, q, rate);
    let (mut x1, mut x2) = (0.0, 0.0);
    let (mut y1, mut y2) = (0.0, 0.0);
    let mut sum = 0.0;
    // The longest ringing of the bank is the 60 Hz section, whose pole
    // radius is 0.997 at 48 kHz: after 20 000 frames it has decayed below
    // any rounding error.
    for n in 0..20_000 {
        let x = if n == 0 { 1.0 } else { 0.0 };
        let y = b0 * x + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
        sum += y.abs();
        x2 = x1;
        x1 = x;
        y2 = y1;
        y1 = y;
    }
    sum
}

/// The magnitude of the whole cascade at the frequency `f`.
///
/// A band at exactly `0` dB is skipped by the stage, and a band at or above
/// the Nyquist guard never runs; the oracle skips them the same way.
fn cascade_magnitude(bands: &[f32; EQ_BANDS], rate: f64, f: f64) -> f64 {
    let mut magnitude = 1.0;
    for (band, db) in bands.iter().enumerate() {
        if *db == 0.0 {
            continue;
        }
        let f0 = BAND_CENTERS_HZ[band];
        if f0 >= NYQUIST_GUARD * rate {
            continue;
        }
        magnitude *= section_magnitude(f64::from(*db), f0, BAND_Q, rate, f);
    }
    magnitude
}

/// The largest magnitude the configured cascade can have.
///
/// The grid is dense enough that the peak of a one-octave section is found
/// to within a thousandth of a decibel; the extra percent covers the grid
/// and the rounding.
fn cascade_peak_gain(bands: &[f32; EQ_BANDS], preamp: f32, rate: f64) -> f64 {
    const STEPS: usize = 20_000;
    let mut peak = 0.0_f64;
    for step in 0..=STEPS {
        let f = step as f64 * rate / 2.0 / STEPS as f64;
        peak = peak.max(cascade_magnitude(bands, rate, f));
    }
    peak * 10.0_f64.powf(f64::from(preamp) / 20.0) * 1.01
}

/// The largest factor by which the cascade can raise a single sample: the
/// product of the sections' impulse-response norms.
fn cascade_l1(bands: &[f32; EQ_BANDS], preamp: f32, rate: f64) -> f64 {
    let mut norm = 1.0;
    for (band, db) in bands.iter().enumerate() {
        if *db == 0.0 {
            continue;
        }
        let f0 = BAND_CENTERS_HZ[band];
        if f0 >= NYQUIST_GUARD * rate {
            continue;
        }
        norm *= section_l1(f64::from(*db), f0, BAND_Q, rate);
    }
    norm * 10.0_f64.powf(f64::from(preamp) / 20.0)
}

// ---------------------------------------------------------------------------
// The equalizer
// ---------------------------------------------------------------------------

/// A disabled equalizer is a bit-exact bypass: whatever the bands, the
/// preamp and the block size are, the buffer comes out unchanged.
#[test]
fn a_disabled_equalizer_is_a_bit_exact_bypass() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xB1_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let channels = rng.index(2) + 1;
        let state = random_eq_state(&mut rng, Some(false));
        let input = random_finite_signal(&mut rng, 2048, channels);

        let mut eq = Equalizer::new();
        eq.control().set(&state);
        eq.prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        let mut out = input.clone();
        process_in_random_blocks(&mut eq, &mut out, channels, &mut rng);

        assert_eq!(
            bits(&out),
            bits(&input),
            "seed {seed}: a disabled equalizer must not touch the buffer"
        );
    }
}

/// An enabled but flat equalizer is bit-exact too: a band at exactly `0` dB
/// is skipped rather than run as a unity filter, and a `0` dB preamp is the
/// exact factor `1.0`.
#[test]
fn a_flat_equalizer_is_bit_exact() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xB2_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let channels = rng.index(2) + 1;
        let state = EqState {
            bands: [0.0; EQ_BANDS],
            preamp: 0.0,
            enabled: true,
            auto: rng.index(2) == 0,
        };
        let input = random_finite_signal(&mut rng, 2048, channels);

        let mut eq = Equalizer::new();
        eq.control().set(&state);
        eq.prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        let mut out = input.clone();
        process_in_random_blocks(&mut eq, &mut out, channels, &mut rng);

        assert_eq!(
            bits(&out),
            bits(&input),
            "seed {seed}: a flat equalizer must not touch the buffer"
        );
    }
}

/// Nothing non-finite leaves the equalizer: a finite input and an in-range
/// setting give a finite output, over the whole finite range of `f32` and
/// not only over the audio domain.
#[test]
fn the_equalizer_output_is_finite() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xB3_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let channels = rng.index(2) + 1;
        let state = random_eq_state(&mut rng, None);
        let input = random_finite_signal(&mut rng, 4096, channels);

        let mut eq = Equalizer::new();
        eq.control().set(&state);
        eq.prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        let mut out = input.clone();
        process_in_random_blocks(&mut eq, &mut out, channels, &mut rng);

        if let Some(bad) = out.iter().position(|s| !s.is_finite()) {
            panic!("seed {seed}: sample {bad} left as {}", out[bad]);
        }

        // The crossfade is exercised too: the switch makes the mix travel,
        // and the arithmetic of the mix is where the two ends of the `f32`
        // range meet.
        let mut switched = state;
        switched.enabled = !state.enabled;
        eq.control().set(&switched);
        let mut out = input.clone();
        process_in_random_blocks(&mut eq, &mut out, channels, &mut rng);

        if let Some(bad) = out.iter().position(|s| !s.is_finite()) {
            panic!(
                "seed {seed}: sample {bad} left as {} during the crossfade",
                out[bad]
            );
        }
    }
}

/// A long random signal stays bounded through the equalizer: the output's
/// energy and its peak stay inside the cascade's own bounds, so the filter
/// cannot blow up.
#[test]
fn a_long_random_signal_stays_bounded_through_the_equalizer() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xB4_0000 + seed);
        let rate = 48_000_u32;
        let channels = 2_usize;
        let state = random_eq_state(&mut rng, Some(true));
        let input = random_signal(&mut rng, 2 * rate as usize, channels);

        let mut eq = Equalizer::new();
        eq.control().set(&state);
        eq.prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        let mut out = input.clone();
        eq.process(&mut out).expect("process");

        assert!(
            out.iter().all(|s| s.is_finite()),
            "seed {seed}: the output must stay finite"
        );
        // The energy: a linear filter cannot make a signal louder than its
        // largest gain.
        let gain = cascade_peak_gain(&state.bands, state.preamp, f64::from(rate));
        let energy_bound = rms(&input) * gain;
        assert!(
            rms(&out) <= energy_bound + 1e-6,
            "seed {seed}: the energy {} exceeds the bound {energy_bound}",
            rms(&out)
        );
        // The peak: bounded by the impulse response's norm, which is the
        // sound bound for the peak of the output.
        let peak_bound = peak(&input) * cascade_l1(&state.bands, state.preamp, f64::from(rate));
        assert!(
            peak(&out) <= peak_bound + 1e-6,
            "seed {seed}: the peak {} exceeds the bound {peak_bound}",
            peak(&out)
        );
    }
}

/// The output does not depend on the block size: the stage counts in
/// frames, not in buffers.
#[test]
fn the_equalizer_output_does_not_depend_on_the_block_size() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xB5_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let channels = rng.index(2) + 1;
        let state = random_eq_state(&mut rng, Some(true));
        let input = random_signal(&mut rng, 4096, channels);

        let mut whole = Equalizer::new();
        whole.control().set(&state);
        whole
            .prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        let mut reference = input.clone();
        whole.process(&mut reference).expect("process");

        let mut split = Equalizer::new();
        split.control().set(&state);
        split
            .prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        let mut got = input.clone();
        process_in_random_blocks(&mut split, &mut got, channels, &mut rng);

        assert_eq!(
            bits(&got),
            bits(&reference),
            "seed {seed}: the block size must not change the output"
        );
    }
}

/// The preamp approaches its target at the documented slew: the applied
/// factor moves by at most the documented rate per frame.
///
/// A DC probe reads the factor off the output: with every band flat the
/// bands are skipped, so the output is the input times the preamp factor.
#[test]
fn the_equalizer_slews_the_preamp_at_the_documented_rate() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xB6_0000 + seed);
        let rate = 48_000_u32;
        let target_db = rng.range(-EqState::MAX_GAIN_DB, EqState::MAX_GAIN_DB);
        let flat = EqState {
            bands: [0.0; EQ_BANDS],
            preamp: 0.0,
            enabled: true,
            auto: false,
        };

        let mut eq = Equalizer::new();
        eq.control().set(&flat);
        eq.prepare(AudioSpec::new(rate, 1)).expect("prepare");
        let frames = rate as usize;
        let mut out = vec![1.0_f32; frames];
        let at = frames / 2;
        eq.process(&mut out[..at]).expect("process");
        eq.control().set(&EqState {
            preamp: target_db,
            ..flat
        });
        eq.process(&mut out[at..]).expect("process");

        // The slew step of one sub-block, and the rate it allows: the factor
        // is interpolated linearly inside the sub-block, so its largest
        // relative step per frame is at the small end of the ramp.
        let sub_blocks = (SLEW_SECONDS * f64::from(rate) / SUB_BLOCK as f64).max(1.0);
        let slew_db = (2.0 * f64::from(EqState::MAX_GAIN_DB) / sub_blocks) as f32;
        let relative = 10.0_f64.powf(f64::from(slew_db) / 20.0) - 1.0;
        let bound = 20.0 / 10.0_f64.ln() * (1.0 + relative / SUB_BLOCK as f64).ln() + 1e-6;

        for (n, w) in out.windows(2).enumerate() {
            let a = 20.0 * f64::from(w[0]).log10();
            let b = 20.0 * f64::from(w[1]).log10();
            assert!(
                (b - a).abs() <= bound + 1e-6,
                "seed {seed}: frame {}: the preamp moved {b} - {a} dB, the slew allows {bound}",
                n + 1
            );
        }
    }
}

/// A parameter change does not click: the output's frame-to-frame movement
/// stays inside what the input's own movement and the gain ramp allow.
#[test]
fn a_parameter_change_does_not_click() {
    /// The transient a coefficient change adds, as a fraction of the signal
    /// level. The analytic worst case of a correctly slewed change (one
    /// sub-block, at most 0.6 dB of gain) moves the filter's direct term by
    /// about 2.5 % of the signal level; 5 % is a factor of two over that.
    const TRANSIENT: f64 = 0.05;

    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xB7_0000 + seed);
        let rate = 48_000_u32;
        let band = rng.index(EQ_BANDS);
        let target_db = rng.range(-EqState::MAX_GAIN_DB, EqState::MAX_GAIN_DB);
        let flat = EqState {
            bands: [0.0; EQ_BANDS],
            preamp: 0.0,
            enabled: true,
            auto: false,
        };

        let mut eq = Equalizer::new();
        eq.control().set(&flat);
        eq.prepare(AudioSpec::new(rate, 1)).expect("prepare");

        // A probe at the band's own centre: the band is the only thing that
        // moves, and the probe shows its gain.
        let f0 = BAND_CENTERS_HZ[band];
        let amplitude = 0.25_f32;
        let frames = rate as usize;
        let input: Vec<f32> = (0..frames)
            .map(|n| {
                (f64::from(amplitude) * (2.0 * PI * f0 * n as f64 / f64::from(rate)).sin()) as f32
            })
            .collect();
        let mut out = input.clone();
        let at = frames / 2;
        eq.process(&mut out[..at]).expect("process");
        let mut bands = [0.0_f32; EQ_BANDS];
        bands[band] = target_db;
        eq.control().set(&EqState { bands, ..flat });
        eq.process(&mut out[at..]).expect("process");

        let gain = cascade_peak_gain(&bands, 0.0, f64::from(rate));
        let input_step = f64::from(amplitude) * 2.0 * PI * f0 / f64::from(rate);
        let bound = input_step * gain + TRANSIENT * f64::from(amplitude) * gain;
        let step = max_step(&out);
        assert!(
            step <= bound,
            "seed {seed}: band {band} to {target_db} dB: the output moved {step}, the bound is {bound}"
        );
    }
}

/// Switching the equalizer on and off crossfades over `FADE_SECONDS`
/// instead of clicking: the wet/dry mix ramps linearly from silence to the
/// full signal and back.
///
/// A DC probe reads the mix off the output: with a preamp of `-12` dB the
/// output is `dry + mix * (wet - dry)`, so the mix follows from the sample.
#[test]
fn the_equalizer_crossfades_over_the_documented_fade() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xB8_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let preamp = -EqState::MAX_GAIN_DB;
        let factor = 10.0_f64.powf(f64::from(preamp) / 20.0);
        let fade_frames = (FADE_SECONDS * f64::from(rate)).max(1.0) as u64;

        let off = EqState {
            bands: [0.0; EQ_BANDS],
            preamp,
            enabled: false,
            auto: false,
        };
        let on = EqState {
            enabled: true,
            ..off
        };
        let mut eq = Equalizer::new();
        eq.control().set(&off);
        eq.prepare(AudioSpec::new(rate, 1)).expect("prepare");

        // Switch on, then off again, and read the mix out of the output.
        // The mix is updated after the frame it belongs to, so the fade-in
        // ends one frame before the mix reaches one, and the fade-out
        // needs one frame more to come back to silence.
        let frames = 3 * fade_frames as usize + 64;
        let mut out = vec![1.0_f32; frames];
        let at = fade_frames as usize;
        eq.process(&mut out[..at]).expect("process");
        eq.control().set(&on);
        eq.process(&mut out[at..2 * at]).expect("process");
        eq.control().set(&off);
        eq.process(&mut out[2 * at..3 * at]).expect("process");

        let mix = |sample: f32| (f64::from(sample) - 1.0) / (factor - 1.0);
        // The ramp covers `fade_frames` frames, from the old mix to the new
        // one, so its per-frame step is one part in `fade_frames - 1`.
        let step = 1.0 / (fade_frames - 1).max(1) as f64;
        // While the stage is off, the mix is silence.
        for (n, sample) in out[..at].iter().enumerate() {
            assert!(
                mix(*sample).abs() < 1e-6,
                "seed {seed}: frame {n} must still be dry"
            );
        }
        // The fade-in ramps the mix up, linearly and without a jump.
        let mut previous = mix(out[at]);
        assert!(
            previous.abs() < 1e-6,
            "seed {seed}: the fade-in must start dry"
        );
        for (n, sample) in out[at..2 * at].iter().enumerate().skip(1) {
            let m = mix(*sample);
            assert!(
                m >= previous - 1e-6,
                "seed {seed}: frame {n}: the mix went back to {m}"
            );
            assert!(
                m - previous <= step + 1e-6,
                "seed {seed}: frame {n}: the mix jumped by {}",
                m - previous
            );
            previous = m;
        }
        assert!(
            previous >= 1.0 - 2.0 * step - 1e-6,
            "seed {seed}: the fade-in ended at {previous}"
        );
        // The fade-out ramps the mix back down to silence.
        previous = mix(out[2 * at]);
        assert!(
            (previous - 1.0).abs() < 1e-6,
            "seed {seed}: the fade-out must start at the full mix"
        );
        for (n, sample) in out[2 * at..3 * at].iter().enumerate().skip(1) {
            let m = mix(*sample);
            assert!(
                m <= previous + 1e-6,
                "seed {seed}: frame {n}: the mix went up to {m}"
            );
            assert!(
                previous - m <= step + 1e-6,
                "seed {seed}: frame {n}: the mix jumped by {}",
                previous - m
            );
            previous = m;
        }
        assert!(
            previous <= 2.0 * step + 1e-6,
            "seed {seed}: the fade-out ended at {previous}"
        );
        for (n, sample) in out[3 * at..].iter().enumerate() {
            assert!(
                mix(*sample).abs() < 1e-6,
                "seed {seed}: frame {n} must be dry again"
            );
        }
    }
}

/// The first frame of a crossfade is the dry signal, bit for bit: the mix
/// starts at zero, so nothing of the wet signal is mixed in yet.
#[test]
fn the_first_frame_of_a_crossfade_is_the_dry_signal() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xB9_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let channels = rng.index(2) + 1;
        let mut state = random_eq_state(&mut rng, Some(false));
        let mut input = random_finite_signal(&mut rng, 64, channels);
        // A negative zero in the first frame: the dry end of the mix has to
        // hand it back unchanged.
        for sample in input.iter_mut().take(channels) {
            *sample = -0.0;
        }

        let mut eq = Equalizer::new();
        eq.control().set(&state);
        eq.prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        state.enabled = true;
        eq.control().set(&state);

        let mut out = input.clone();
        eq.process(&mut out).expect("process");
        assert_eq!(
            bits(&out[..channels]),
            bits(&input[..channels]),
            "seed {seed}: the first frame of the crossfade must be the dry signal"
        );
    }
}

// ---------------------------------------------------------------------------
// The output fade
// ---------------------------------------------------------------------------

/// The fade ramp is exact: the first frame carries `from`, the last one
/// `to`, both exactly; the gain follows the equal-power law in between and
/// never leaves the two endpoints.
#[test]
fn a_fade_ramp_reaches_its_endpoints_exactly() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xFA_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let channels = rng.index(2) + 1;
        let from = rng.range(0.0, 1.0);
        let to = rng.range(0.0, 1.0);
        let duration_ms = rng.index(200) as u32 + 1;

        let control = Arc::new(FadeControl::new());
        let mut fade = Fade::with_control(Arc::clone(&control));
        fade.prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        // Put the stage at `from` first: a jump to the target, no ramp.
        control.request(from, 0);
        let span = ((f64::from(duration_ms) * f64::from(rate) / 1000.0).round() as u64).max(1);
        // Let the stage take the jump over, so that the ramp starts there.
        let mut latch = vec![1.0_f32; channels];
        fade.process(&mut latch).expect("process");
        assert_eq!(
            latch[0].to_bits(),
            from.to_bits(),
            "seed {seed}: the stage must sit at {from} before the ramp"
        );

        control.request(to, duration_ms);
        let mut buf = vec![1.0_f32; (span as usize + 8) * channels];
        fade.process(&mut buf).expect("process");
        let gains: Vec<f32> = buf.chunks_exact(channels).map(|f| f[0]).collect();

        if span > 1 {
            assert_eq!(
                gains[0].to_bits(),
                from.to_bits(),
                "seed {seed}: the ramp must start at {from}"
            );
        }
        assert_eq!(
            gains[span as usize - 1].to_bits(),
            to.to_bits(),
            "seed {seed}: the ramp must end at {to}"
        );
        for (n, gain) in gains.iter().skip(span as usize).enumerate() {
            assert_eq!(
                gain.to_bits(),
                to.to_bits(),
                "seed {seed}: frame {} must hold the target {to}",
                span as usize + n
            );
        }
        // The law: the power falls evenly between the endpoints.
        for (n, gain) in gains.iter().take(span as usize).enumerate() {
            let t = n as f64 / (span as f64 - 1.0).max(1.0);
            let power = f64::from(from) * f64::from(from)
                + (f64::from(to) * f64::from(to) - f64::from(from) * f64::from(from)) * t;
            let want = power.max(0.0).sqrt();
            assert!(
                (f64::from(*gain) - want).abs() < 1e-6,
                "seed {seed}: frame {n}: the gain is {}, the law says {want}",
                *gain
            );
            // The gain stays between the two endpoints.
            let low = f64::from(from).min(f64::from(to));
            let high = f64::from(from).max(f64::from(to));
            let got = f64::from(*gain);
            assert!(
                got >= low - 1e-6 && got <= high + 1e-6,
                "seed {seed}: frame {n}: the gain {got} left [{low}, {high}]"
            );
        }
    }
}

/// The ramp covers exactly the requested number of frames, whatever the
/// block size is, and the same signal gives the same ramp.
#[test]
fn a_fade_ramp_is_frame_accurate_for_any_block_size() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xFA_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let channels = rng.index(2) + 1;
        let to = rng.range(0.0, 1.0);
        let duration_ms = rng.index(200) as u32 + 1;
        let span = ((f64::from(duration_ms) * f64::from(rate) / 1000.0).round() as u64).max(1);

        let control = Arc::new(FadeControl::new());
        let mut fade = Fade::with_control(Arc::clone(&control));
        fade.prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        control.request(to, duration_ms);

        let frames = span as usize + 16;
        let mut buf = vec![1.0_f32; frames * channels];
        fade.process(&mut buf).expect("process");
        let gains: Vec<f32> = buf.chunks_exact(channels).map(|f| f[0]).collect();

        // The ramp is over after exactly `span` frames.
        assert_eq!(
            gains[span as usize - 1].to_bits(),
            to.to_bits(),
            "seed {seed}: the ramp must be over after {span} frames"
        );
        for (n, gain) in gains.iter().skip(span as usize).enumerate() {
            assert_eq!(
                gain.to_bits(),
                to.to_bits(),
                "seed {seed}: frame {} must hold the target",
                span as usize + n
            );
        }

        // The same ramp, split into random blocks.
        let control = Arc::new(FadeControl::new());
        let mut split = Fade::with_control(Arc::clone(&control));
        split
            .prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        control.request(to, duration_ms);
        let mut got = vec![1.0_f32; frames * channels];
        process_in_random_blocks(&mut split, &mut got, channels, &mut rng);
        let got: Vec<f32> = got.chunks_exact(channels).map(|f| f[0]).collect();
        assert_eq!(
            bits(&got),
            bits(&gains),
            "seed {seed}: the block size must not change the ramp"
        );
    }
}

/// The fade multiplies: the output never exceeds the input by more than the
/// ramp's own gain, and nothing non-finite leaves.
#[test]
fn a_fade_output_is_bounded_by_its_ramp() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xFA_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let channels = rng.index(2) + 1;
        let from = rng.range(0.0, 1.0);
        let to = rng.range(0.0, 1.0);
        let duration_ms = rng.index(200) as u32 + 1;

        let control = Arc::new(FadeControl::new());
        let mut fade = Fade::with_control(Arc::clone(&control));
        fade.prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        control.request(from, 0);
        // Let the stage take the jump over, so that the ramp starts there.
        let mut latch = vec![1.0_f32; channels];
        fade.process(&mut latch).expect("process");
        control.request(to, duration_ms);

        let input = random_finite_signal(&mut rng, 4096, channels);
        let mut out = input.clone();
        process_in_random_blocks(&mut fade, &mut out, channels, &mut rng);

        assert!(
            out.iter().all(|s| s.is_finite()),
            "seed {seed}: the output must stay finite"
        );
        let high = f64::from(from).max(f64::from(to));
        for (n, (x, y)) in input.iter().zip(out.iter()).enumerate() {
            assert!(
                f64::from(y.abs()) <= high * f64::from(x.abs()) + 1e-6,
                "seed {seed}: sample {n}: {} is more than the ramp allows",
                *y
            );
        }
    }
}

/// An interrupted fade continues from the gain it had reached: the new
/// ramp starts exactly there and ends exactly at its new target, so the
/// output never steps.
#[test]
fn an_interrupted_fade_continues_from_the_current_gain() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0xFA_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let duration_ms = rng.index(200) as u32 + 1;
        let first = rng.range(0.0, 1.0);
        let second = rng.range(0.0, 1.0);
        let span = ((f64::from(duration_ms) * f64::from(rate) / 1000.0).round() as u64).max(1);

        let control = Arc::new(FadeControl::new());
        let mut fade = Fade::with_control(Arc::clone(&control));
        fade.prepare(AudioSpec::new(rate, 1)).expect("prepare");
        control.request(first, duration_ms);

        // Run the first half of the ramp.
        let half = (span / 2).max(1) as usize;
        let mut buf = vec![1.0_f32; half];
        fade.process(&mut buf).expect("process");
        let reached = buf[half - 1];

        // Change the target in the middle of the ramp.
        control.request(second, duration_ms);
        let mut buf = vec![1.0_f32; span as usize + 4];
        fade.process(&mut buf).expect("process");
        assert_eq!(
            buf[0].to_bits(),
            reached.to_bits(),
            "seed {seed}: the new ramp must start at the gain the old one reached"
        );
        assert_eq!(
            buf[span as usize - 1].to_bits(),
            second.to_bits(),
            "seed {seed}: the new ramp must end at {second}"
        );
    }
}

// ---------------------------------------------------------------------------
// The limiter
// ---------------------------------------------------------------------------

/// The limiter holds the ceiling: no output sample exceeds it, and nothing
/// non-finite leaves.
#[test]
fn the_limiter_holds_its_ceiling() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0x11_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let channels = rng.index(2) + 1;
        let scale = rng.range(0.5, 8.0);
        let mut input = random_finite_signal(&mut rng, 4096, channels);
        for sample in &mut input {
            *sample *= scale;
        }

        let mut limiter = Limiter::with_control(Arc::new(LimiterControl::new(true)));
        limiter
            .prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        let ceiling = limiter.ceiling();
        let mut out = input.clone();
        process_in_random_blocks(&mut limiter, &mut out, channels, &mut rng);

        for (n, sample) in out.iter().enumerate() {
            assert!(
                sample.is_finite(),
                "seed {seed}: sample {n} left as {sample}"
            );
            assert!(
                sample.abs() <= ceiling,
                "seed {seed}: sample {n} is {sample}, the ceiling is {ceiling}"
            );
        }
    }
}

/// A disabled limiter is a bit-exact bypass.
#[test]
fn a_disabled_limiter_is_a_bit_exact_bypass() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0x11_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let channels = rng.index(2) + 1;
        let input = random_finite_signal(&mut rng, 4096, channels);

        let mut limiter = Limiter::with_control(Arc::new(LimiterControl::new(false)));
        limiter
            .prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        assert_eq!(limiter.latency_frames(), 0);
        let mut out = input.clone();
        process_in_random_blocks(&mut limiter, &mut out, channels, &mut rng);

        assert_eq!(
            bits(&out),
            bits(&input),
            "seed {seed}: a disabled limiter must not touch the buffer"
        );
    }
}

/// A signal below the ceiling passes through bit for bit, delayed by the
/// look-ahead.
#[test]
fn a_quiet_signal_passes_the_limiter_bit_exact() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0x11_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let channels = rng.index(2) + 1;
        let lookahead = ((LOOKAHEAD_SECONDS * f64::from(rate)).round() as usize).max(1);
        // Quiet enough that the true-peak estimate stays far below the
        // ceiling: the estimate is bounded by the kernel's L1 norm times the
        // largest sample.
        let input: Vec<f32> = (0..4096 * channels).map(|_| rng.range(-0.1, 0.1)).collect();

        let mut limiter = Limiter::with_control(Arc::new(LimiterControl::new(true)));
        limiter
            .prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        assert_eq!(limiter.latency_frames(), lookahead as u32);
        let mut out = input.clone();
        process_in_random_blocks(&mut limiter, &mut out, channels, &mut rng);

        assert!(
            out[..lookahead * channels].iter().all(|s| *s == 0.0),
            "seed {seed}: the delay line is primed with silence"
        );
        assert_eq!(
            bits(&out[lookahead * channels..]),
            bits(&input[..input.len() - lookahead * channels]),
            "seed {seed}: a quiet signal must pass bit for bit"
        );
    }
}

/// The limiter's gain moves by at most `1 / (lookahead + 1)` per frame: the
/// attack ramps down linearly across the look-ahead instead of jumping.
#[test]
fn the_limiter_moves_its_gain_by_at_most_one_window_step_per_frame() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0x11_0000 + seed);
        let rate = 48_000_u32;
        let lookahead = ((LOOKAHEAD_SECONDS * f64::from(rate)).round() as usize).max(1);
        let freq = f64::from(rng.range(200.0, 2_000.0));
        let amplitude = f64::from(rng.range(1.5, 4.0));

        let mut limiter = Limiter::with_control(Arc::new(LimiterControl::new(true)));
        limiter.prepare(AudioSpec::new(rate, 1)).expect("prepare");
        let ceiling = limiter.ceiling();

        let frames = 2 * rate as usize;
        let input: Vec<f32> = (0..frames)
            .map(|n| (amplitude * (2.0 * PI * freq * n as f64 / f64::from(rate)).sin()) as f32)
            .collect();
        let mut out = input.clone();
        limiter.process(&mut out).expect("process");

        // The limiter is a pure gain on the delayed frame, so the applied
        // gain can be read off the output wherever the sample is not too
        // small to divide by and the result was not clamped to the ceiling.
        let mut last: Option<(usize, f64)> = None;
        for n in lookahead..frames {
            let x = f64::from(input[n - lookahead]);
            let y = f64::from(out[n]);
            if x.abs() < 0.05 * amplitude {
                continue;
            }
            if y.abs() >= f64::from(ceiling) * (1.0 - 1e-6) {
                continue;
            }
            let gain = y / x;
            if let Some((previous, before)) = last {
                let gap = (n - previous) as f64;
                let bound = gap / (lookahead + 1) as f64 + 1e-5;
                assert!(
                    (gain - before).abs() <= bound,
                    "seed {seed}: frame {n}: the gain moved {} -> {gain}, {gap} frames allow {bound}",
                    before
                );
            }
            last = Some((n, gain));
        }
    }
}

/// The limiter's output does not depend on the block size.
#[test]
fn the_limiter_output_does_not_depend_on_the_block_size() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(0x11_0000 + seed);
        let rate = RATES[rng.index(RATES.len())];
        let channels = rng.index(2) + 1;
        let input = random_finite_signal(&mut rng, 4096, channels);

        let mut whole = Limiter::with_control(Arc::new(LimiterControl::new(true)));
        whole
            .prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        let mut reference = input.clone();
        whole.process(&mut reference).expect("process");

        let mut split = Limiter::with_control(Arc::new(LimiterControl::new(true)));
        split
            .prepare(AudioSpec::new(rate, channels as u16))
            .expect("prepare");
        let mut got = input.clone();
        process_in_random_blocks(&mut split, &mut got, channels, &mut rng);

        assert_eq!(
            bits(&got),
            bits(&reference),
            "seed {seed}: the block size must not change the output"
        );
    }
}
