//! The 10-band equalizer and preamp stage (spec S07).
//!
//! # What is measured and what is chosen here
//!
//! Fixed by audited evidence:
//!
//! * the value domain: [`EqState`] carries gains in dB, nominally
//!   `-12.0..=12.0` ([`EqState::MAX_GAIN_DB`]); the classic 0..63 slider scale
//!   maps onto it through the S09 anchors `0 = +12 dB`, `31 = 0 dB`,
//!   `63 = -12 dB` (`rustamp_proto::state::eq_classic_from_db` is the one
//!   conversion; this stage only ever sees dB);
//! * the chain position (S07 §10): the equalizer runs on the decoded PCM,
//!   **before** the output scaling that implements the volume and balance
//!   laws;
//! * the band count (ten) and the printed band labels of the base skin (S01:
//!   `60 170 310 600 1K 3K 6K 12K 14K 16K`).
//!
//! **Own engineering choices** (no spec fixes them; none is measured, and none
//! may be cited as 2.95 behaviour):
//!
//! * **Band centres** — [`BAND_CENTERS_HZ`] uses the frequencies the skin
//!   prints as band labels. That the reference filters are centred exactly on
//!   those labels is an assumption, not a measurement.
//! * **Topology** — ten second-order peaking filters in series, coefficients
//!   from the public-domain "Audio EQ Cookbook" (R. Bristow-Johnson) peaking
//!   formula, run in transposed direct form II with `f64` state. A peaking
//!   section has exactly the requested gain at its centre and unity far away,
//!   so a band set to `g` dB lifts its region by `g` dB and leaves the rest of
//!   the spectrum alone. The reference player's filter shapes (Q, overlap,
//!   shelving at the ends, any FFT-based design) are unmeasured.
//! * **Bandwidth** — one constant [`BAND_Q`] (one octave) for every band. The
//!   12k/14k/16k bands therefore overlap strongly; that is a consequence of the
//!   label spacing, not a modelled reference trait.
//! * **Band law** — gain in dB is linear in the 0..63 slider value (that is
//!   what the S09 anchors imply between their three points; the intermediate
//!   values are unmeasured).
//! * **Preamp** — a plain linear gain of `10^(dB/20)` applied to the input
//!   **before** the band filters.
//! * **Near Nyquist** — a band whose centre is at or above
//!   [`NYQUIST_GUARD`] × sample rate is skipped (the peaking formula degenerates
//!   there; at 22.05 kHz the three top bands are unreachable anyway).
//! * **No clipping or limiting.** Boosts can push samples past full scale; the
//!   stage does not clip, and what the reference does with overs is unmeasured.
//!
//! # Transparency and click-free switching
//!
//! * **Bypass is exact.** While the stage is disabled and its fade has finished,
//!   `process` returns before touching the buffer, so the output is the input,
//!   bit for bit. `auto` (preset autoloading) has no signal effect at all.
//! * **Flat is exact.** A band at exactly 0 dB is skipped rather than run as a
//!   unity filter, and a 0 dB preamp is the exact factor `1.0`, so an enabled
//!   but flat equalizer is also bit-transparent once it has settled.
//! * **Toggling does not click.** `enabled` drives a wet/dry mix that ramps
//!   linearly over [`FADE_SECONDS`]; the output is `dry + mix * (wet - dry)`, so
//!   switching on or off is a short crossfade instead of a step. Switching on
//!   from a full bypass clears the filter history first, and the start-up
//!   transient of the filters is inside the fade.
//! * **Parameter changes do not click.** Band and preamp targets are approached
//!   at a bounded slew (full range in [`SLEW_SECONDS`]); band coefficients are
//!   recomputed every [`SUB_BLOCK`] frames and the preamp factor is
//!   interpolated per frame inside each sub-block.
//!
//! # Signal hygiene (own choice)
//!
//! * **NaN and infinities become silence at the stage input.** A non-finite
//!   sample would otherwise lodge in the biquad history (`z1`/`z2` stay NaN
//!   once NaN enters, because every comparison against NaN is false) and
//!   poison every later frame. The decode thread already replaces non-finite
//!   samples before the chain; this is defense in depth for any other caller.
//! * **Subnormals pass through.** They are finite, so they cannot poison the
//!   history; subnormal *history* is flushed to zero instead
//!   ([`Biquad::flush_tiny`], −4000 dBFS threshold). The resampled path
//!   flushes subnormal outputs in the converter, where the bit-exact
//!   equal-rate promise does not apply.
//! * **DC passes.** Every peaking section is unity at DC by construction, so a
//!   constant is returned unchanged except for the preamp factor (pinned by
//!   `dc_passes_at_unity_through_boosted_bands`).
//!
//! # Threading
//!
//! The stage lives on the decode thread; the engine owns an [`EqControl`]
//! (shared through an [`Arc`]) and writes new [`EqState`] values into it
//! without locks. `process` checks the control's generation counter once per
//! call and never allocates.

use std::f64::consts::{PI, SQRT_2};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use rustamp_core::audio::{AudioSpec, DspStage};
use rustamp_core::error::Result;
use rustamp_core::state::{EQ_BANDS, EqState};

use crate::sanitize_sample;

/// Band centre frequencies in Hz. **Own choice**: the base skin's printed band
/// labels (S01), not measured filter centres.
pub const BAND_CENTERS_HZ: [f64; EQ_BANDS] = [
    60.0, 170.0, 310.0, 600.0, 1_000.0, 3_000.0, 6_000.0, 12_000.0, 14_000.0, 16_000.0,
];

/// Quality factor of every band. **Own choice**: `sqrt(2)`, a one-octave
/// bandwidth for the peaking sections.
pub const BAND_Q: f64 = SQRT_2;

/// Bands centred at or above this fraction of the sample rate are skipped.
pub const NYQUIST_GUARD: f64 = 0.45;

/// Length of the enable/disable crossfade in seconds (own choice: 10 ms).
pub const FADE_SECONDS: f64 = 0.010;

/// Time a gain change across the whole `-12..=12` dB range takes (own choice:
/// 30 ms); smaller changes take proportionally less.
pub const SLEW_SECONDS: f64 = 0.030;

/// Frames between two coefficient updates while a gain is slewing.
pub const SUB_BLOCK: usize = 32;

/// Shared, lock-free parameter block between the engine (writer) and the
/// stage on the decode thread (reader).
///
/// The writer stores every field and then bumps the generation with release
/// ordering; the reader loads the generation with acquire ordering and then the
/// fields. A reader that races a second write may see a mix of the two, but it
/// also sees an out-of-date generation and re-reads on its next call, so it
/// always converges on the latest state.
#[derive(Debug)]
pub struct EqControl {
    bands: [AtomicU32; EQ_BANDS],
    preamp: AtomicU32,
    enabled: AtomicBool,
    generation: AtomicU64,
}

impl Default for EqControl {
    fn default() -> Self {
        EqControl::new(&EqState::default())
    }
}

impl EqControl {
    /// A control block holding `state`.
    pub fn new(state: &EqState) -> Self {
        EqControl {
            bands: std::array::from_fn(|i| AtomicU32::new(state.bands[i].to_bits())),
            preamp: AtomicU32::new(state.preamp.to_bits()),
            enabled: AtomicBool::new(state.enabled),
            generation: AtomicU64::new(0),
        }
    }

    /// Publish a new state. `auto` is not stored: it has no signal effect.
    pub fn set(&self, state: &EqState) {
        for (slot, gain) in self.bands.iter().zip(state.bands.iter()) {
            slot.store(gain.to_bits(), Ordering::Relaxed);
        }
        self.preamp.store(state.preamp.to_bits(), Ordering::Relaxed);
        self.enabled.store(state.enabled, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Release);
    }

    fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    fn bands_db(&self) -> [f32; EQ_BANDS] {
        std::array::from_fn(|i| f32::from_bits(self.bands[i].load(Ordering::Relaxed)))
    }

    fn preamp_db(&self) -> f32 {
        f32::from_bits(self.preamp.load(Ordering::Relaxed))
    }

    fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
}

/// Clamp a gain to the engine's range; a non-finite gain cannot be applied
/// honestly and is treated as flat.
fn sanitize_db(db: f32) -> f32 {
    if db.is_finite() {
        db.clamp(-EqState::MAX_GAIN_DB, EqState::MAX_GAIN_DB)
    } else {
        0.0
    }
}

fn db_to_linear(db: f32) -> f64 {
    10.0_f64.powf(f64::from(db) / 20.0)
}

fn step_toward(current: f32, target: f32, max_step: f32) -> f32 {
    let delta = target - current;
    if delta.abs() <= max_step {
        target
    } else {
        current + max_step.copysign(delta)
    }
}

/// Normalized biquad coefficients (`a0 = 1`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Coefs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Coefs {
    const IDENTITY: Coefs = Coefs {
        b0: 1.0,
        b1: 0.0,
        b2: 0.0,
        a1: 0.0,
        a2: 0.0,
    };

    /// Peaking EQ section ("Audio EQ Cookbook"): gain `db` at `f0`, unity far
    /// away, bandwidth set by `q`.
    fn peaking(f0: f64, sample_rate: f64, q: f64, db: f32) -> Coefs {
        let a = 10.0_f64.powf(f64::from(db) / 40.0);
        let w0 = 2.0 * PI * f0 / sample_rate;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha / a;
        Coefs {
            b0: (1.0 + alpha * a) / a0,
            b1: -2.0 * cos / a0,
            b2: (1.0 - alpha * a) / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha / a) / a0,
        }
    }
}

/// Transposed direct form II history of one section on one channel.
#[derive(Debug, Clone, Copy, Default)]
struct Biquad {
    z1: f64,
    z2: f64,
}

impl Biquad {
    #[inline]
    fn tick(&mut self, c: &Coefs, x: f64) -> f64 {
        let y = c.b0 * x + self.z1;
        self.z1 = c.b1 * x - c.a1 * y + self.z2;
        self.z2 = c.b2 * x - c.a2 * y;
        y
    }

    /// Flush a decayed history to zero so long silences never reach the slow
    /// subnormal range.
    fn flush_tiny(&mut self) {
        const TINY: f64 = 1e-200;
        if self.z1.abs() < TINY {
            self.z1 = 0.0;
        }
        if self.z2.abs() < TINY {
            self.z2 = 0.0;
        }
    }
}

/// The 10-band equalizer and preamp as a [`DspStage`]. See the module docs
/// for the topology and for which parts are own choices.
#[derive(Debug)]
pub struct Equalizer {
    control: Arc<EqControl>,
    /// Control generation the targets below were read at.
    seen_generation: u64,
    sample_rate: f64,
    channels: usize,
    target_db: [f32; EQ_BANDS],
    current_db: [f32; EQ_BANDS],
    target_preamp_db: f32,
    current_preamp_db: f32,
    /// Preamp factor at the start and at the end of the current sub-block.
    pre_from: f64,
    pre_to: f64,
    coefs: [Coefs; EQ_BANDS],
    /// Whether a band runs at all (non-zero gain and below the Nyquist guard).
    active: [bool; EQ_BANDS],
    /// Per-channel filter history (allocated in `prepare`).
    history: Vec<[Biquad; EQ_BANDS]>,
    /// Target of the wet/dry mix.
    enabled: bool,
    /// Wet/dry mix, `0.0` = bypass, `1.0` = fully equalized.
    mix: f32,
    mix_step: f32,
    slew_db: f32,
    /// Frame index inside the current sub-block.
    sub_pos: usize,
}

impl Default for Equalizer {
    fn default() -> Self {
        Equalizer::new()
    }
}

impl Equalizer {
    /// A stage with its own control block, holding the flat, disabled default.
    pub fn new() -> Self {
        Equalizer::with_control(Arc::new(EqControl::default()))
    }

    /// A stage that follows a control block shared with its owner.
    pub fn with_control(control: Arc<EqControl>) -> Self {
        let mut eq = Equalizer {
            control,
            seen_generation: 0,
            sample_rate: 44_100.0,
            channels: 0,
            target_db: [0.0; EQ_BANDS],
            current_db: [0.0; EQ_BANDS],
            target_preamp_db: 0.0,
            current_preamp_db: 0.0,
            pre_from: 1.0,
            pre_to: 1.0,
            coefs: [Coefs::IDENTITY; EQ_BANDS],
            active: [false; EQ_BANDS],
            history: Vec::new(),
            enabled: false,
            mix: 0.0,
            mix_step: 1.0,
            slew_db: 2.0 * EqState::MAX_GAIN_DB,
            sub_pos: 0,
        };
        eq.load_control();
        eq.mix = if eq.enabled { 1.0 } else { 0.0 };
        eq.snap();
        eq
    }

    /// The control block this stage reads.
    pub fn control(&self) -> Arc<EqControl> {
        Arc::clone(&self.control)
    }

    /// Read the targets from the control block.
    fn load_control(&mut self) {
        self.seen_generation = self.control.generation();
        self.target_db = self.control.bands_db().map(sanitize_db);
        self.target_preamp_db = sanitize_db(self.control.preamp_db());
        self.enabled = self.control.enabled();
    }

    /// Pick up a new control state, if one was published.
    fn poll_control(&mut self) {
        if self.control.generation() == self.seen_generation {
            return;
        }
        self.load_control();
        if self.mix == 0.0 {
            // Fully bypassed: nothing audible depends on the filters, so jump
            // straight to the targets and start any fade-in from clean history.
            self.snap();
        }
    }

    /// Jump every smoothed value to its target and clear the history.
    fn snap(&mut self) {
        self.current_db = self.target_db;
        self.current_preamp_db = self.target_preamp_db;
        for band in 0..EQ_BANDS {
            self.update_band(band);
        }
        self.pre_to = db_to_linear(self.current_preamp_db);
        self.pre_from = self.pre_to;
        self.clear_history();
        self.sub_pos = 0;
    }

    fn clear_history(&mut self) {
        for channel in &mut self.history {
            *channel = [Biquad::default(); EQ_BANDS];
        }
    }

    fn update_band(&mut self, band: usize) {
        let db = self.current_db[band];
        let f0 = BAND_CENTERS_HZ[band];
        let active = db != 0.0 && f0 < NYQUIST_GUARD * self.sample_rate;
        if self.active[band] && !active {
            // A band that settles back to flat is skipped from now on; its
            // history must not leak into a later reactivation.
            for channel in &mut self.history {
                channel[band] = Biquad::default();
            }
        }
        self.active[band] = active;
        self.coefs[band] = if active {
            Coefs::peaking(f0, self.sample_rate, BAND_Q, db)
        } else {
            Coefs::IDENTITY
        };
    }

    /// Start a new sub-block: move every gain one slew step toward its target.
    fn advance_params(&mut self) {
        for band in 0..EQ_BANDS {
            if self.current_db[band] != self.target_db[band] {
                self.current_db[band] =
                    step_toward(self.current_db[band], self.target_db[band], self.slew_db);
                self.update_band(band);
            }
        }
        if self.current_preamp_db != self.target_preamp_db {
            self.current_preamp_db =
                step_toward(self.current_preamp_db, self.target_preamp_db, self.slew_db);
        }
        self.pre_from = self.pre_to;
        self.pre_to = db_to_linear(self.current_preamp_db);
        for channel in &mut self.history {
            for section in channel.iter_mut() {
                section.flush_tiny();
            }
        }
    }
}

impl DspStage for Equalizer {
    fn name(&self) -> &str {
        "equalizer"
    }

    fn prepare(&mut self, spec: AudioSpec) -> Result<()> {
        self.sample_rate = f64::from(spec.sample_rate.max(1));
        self.channels = usize::from(spec.channels);
        self.history = vec![[Biquad::default(); EQ_BANDS]; self.channels];
        let fade_frames = (FADE_SECONDS * self.sample_rate).max(1.0);
        self.mix_step = (1.0 / fade_frames) as f32;
        let sub_blocks = (SLEW_SECONDS * self.sample_rate / SUB_BLOCK as f64).max(1.0);
        self.slew_db = (2.0 * f64::from(EqState::MAX_GAIN_DB) / sub_blocks) as f32;
        self.load_control();
        self.mix = if self.enabled { 1.0 } else { 0.0 };
        // The band activity depends on the sample rate: recompute everything.
        self.active = [false; EQ_BANDS];
        self.snap();
        Ok(())
    }

    fn process(&mut self, buf: &mut [f32]) {
        self.poll_control();
        let channels = self.channels;
        if channels == 0 {
            return;
        }
        for frame in buf.chunks_exact_mut(channels) {
            if !self.enabled && self.mix == 0.0 {
                // Exact bypass: the rest of the buffer is left untouched.
                return;
            }
            if self.sub_pos == 0 {
                self.advance_params();
            }
            let t = self.sub_pos as f64 / SUB_BLOCK as f64;
            let preamp = self.pre_from + (self.pre_to - self.pre_from) * t;
            self.sub_pos = (self.sub_pos + 1) % SUB_BLOCK;
            let mix = self.mix;
            for (sample, history) in frame.iter_mut().zip(self.history.iter_mut()) {
                // Hostile input becomes silence here; subnormals and DC pass
                // through (see the module docs). Full bypass above stays
                // bit-exact because it returns before reaching this line.
                let dry = sanitize_sample(*sample);
                // Preamp first, then the bands in ascending order.
                let mut y = f64::from(dry) * preamp;
                for ((section, coefs), active) in history
                    .iter_mut()
                    .zip(self.coefs.iter())
                    .zip(self.active.iter())
                {
                    if *active {
                        y = section.tick(coefs, y);
                    }
                }
                let wet = y as f32;
                *sample = if mix >= 1.0 {
                    wet
                } else {
                    dry + (wet - dry) * mix
                };
            }
            if self.enabled {
                if self.mix < 1.0 {
                    self.mix = (self.mix + self.mix_step).min(1.0);
                }
            } else if self.mix > 0.0 {
                self.mix = (self.mix - self.mix_step).max(0.0);
            }
        }
    }

    fn reset(&mut self) {
        self.poll_control();
        self.mix = if self.enabled { 1.0 } else { 0.0 };
        self.snap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_proto::state::eq_classic_from_db;

    const FS: u32 = 48_000;

    /// Deterministic white-ish noise in `-0.5..0.5`, with a few exact edge
    /// values (signed zeros, full scale) mixed in.
    fn noise(samples: usize) -> Vec<f32> {
        let mut seed: u32 = 0x1234_5678;
        let mut out: Vec<f32> = (0..samples)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect();
        for (i, edge) in [0.0, -0.0, 1.0, -1.0].into_iter().enumerate() {
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

    fn stage(state: &EqState, channels: u16) -> Equalizer {
        let mut eq = Equalizer::new();
        eq.control().set(state);
        eq.prepare(AudioSpec::new(FS, channels)).unwrap();
        eq
    }

    fn enabled(bands: [f32; EQ_BANDS], preamp: f32) -> EqState {
        EqState {
            bands,
            preamp,
            enabled: true,
            auto: false,
        }
    }

    fn bits(buf: &[f32]) -> Vec<u32> {
        buf.iter().map(|s| s.to_bits()).collect()
    }

    fn rms(buf: &[f32]) -> f64 {
        (buf.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / buf.len() as f64).sqrt()
    }

    /// Steady-state gain in dB of a mono sine at `freq` through `state`: one
    /// second of signal, measured over the last half second (an integer number
    /// of periods for every frequency used below).
    fn gain_db(state: &EqState, freq: f64) -> f64 {
        let input = sine(freq, 0.25, FS as usize);
        let mut out = input.clone();
        stage(state, 1).process(&mut out);
        let half = FS as usize / 2;
        20.0 * (rms(&out[half..]) / rms(&input[half..])).log10()
    }

    #[test]
    fn disabled_stage_is_bit_transparent_whatever_the_settings() {
        let input = noise(4096 * 2);
        for state in [
            EqState::default(),
            EqState {
                bands: [12.0, -12.0, 6.0, -6.0, 3.0, -3.0, 12.0, -12.0, 9.0, -9.0],
                preamp: 7.5,
                enabled: false,
                auto: true,
            },
        ] {
            let mut eq = stage(&state, 2);
            let mut out = input.clone();
            eq.process(&mut out);
            assert_eq!(bits(&out), bits(&input), "disabled must be a bypass");
        }
    }

    #[test]
    fn flat_enabled_stage_is_bit_transparent() {
        let input = noise(4096 * 2);
        let mut eq = stage(&enabled([0.0; EQ_BANDS], 0.0), 2);
        let mut out = input.clone();
        eq.process(&mut out);
        assert_eq!(bits(&out), bits(&input));
    }

    #[test]
    fn bypass_after_a_fade_out_equals_the_input_exactly() {
        let mut eq = stage(&enabled([12.0; EQ_BANDS], -6.0), 2);
        let mut warm = noise(4096 * 2);
        eq.process(&mut warm);
        eq.control().set(&EqState {
            enabled: false,
            ..enabled([12.0; EQ_BANDS], -6.0)
        });
        // Run the fade out (10 ms = 480 frames) to completion.
        let mut fade = noise(2048 * 2);
        eq.process(&mut fade);
        let input = noise(4096 * 2);
        let mut out = input.clone();
        eq.process(&mut out);
        assert_eq!(bits(&out), bits(&input));
    }

    #[test]
    fn s09_anchors_give_plus_12_0_and_minus_12_db() {
        for (db, classic) in [(12.0_f32, 0), (0.0, 31), (-12.0, 63)] {
            // The measured surface conversion: dB -> classic 0..63.
            assert_eq!(eq_classic_from_db(db), classic);

            // Preamp at the anchor: an exact linear factor of 10^(dB/20).
            let input = noise(1024);
            let mut out = input.clone();
            stage(&enabled([0.0; EQ_BANDS], db), 1).process(&mut out);
            let factor = 10.0_f64.powf(f64::from(db) / 20.0);
            for (x, y) in input.iter().zip(out.iter()) {
                let expected = (f64::from(*x) * factor) as f32;
                assert_eq!(y.to_bits(), expected.to_bits(), "preamp {db} dB");
            }

            // One band at the anchor: exactly that gain at its centre.
            let mut bands = [0.0; EQ_BANDS];
            bands[4] = db;
            let measured = gain_db(&enabled(bands, 0.0), BAND_CENTERS_HZ[4]);
            assert!(
                (measured - f64::from(db)).abs() < 0.05,
                "band at classic {classic}: measured {measured:.3} dB, want {db} dB"
            );
        }
    }

    #[test]
    fn a_boosted_band_lifts_its_region_and_leaves_the_rest() {
        let flat = enabled([0.0; EQ_BANDS], 0.0);
        let mut bands = [0.0; EQ_BANDS];
        bands[1] = 12.0;
        let boosted = enabled(bands, 0.0);

        let flat_170 = gain_db(&flat, 170.0);
        let boost_170 = gain_db(&boosted, 170.0);
        let boost_60 = gain_db(&boosted, 60.0);
        let boost_6k = gain_db(&boosted, 6_000.0);
        assert!(flat_170.abs() < 1e-6, "flat: {flat_170}");
        assert!(boost_170 - flat_170 > 11.9, "centre lift {boost_170}");
        // 60 Hz is 1.5 octaves below the centre: still on the skirt (about
        // +1.1 dB for a one-octave section), far below the centre lift.
        assert!(boost_60 > 0.5 && boost_60 < boost_170, "skirt {boost_60}");
        assert!(boost_6k.abs() < 0.1, "far region moved: {boost_6k}");
    }

    #[test]
    fn channels_are_filtered_independently() {
        let mut bands = [0.0; EQ_BANDS];
        bands[4] = 12.0;
        let mut eq = stage(&enabled(bands, 6.0), 2);
        let left = noise(2048);
        let mut buf: Vec<f32> = left.iter().flat_map(|l| [*l, 0.0]).collect();
        eq.process(&mut buf);
        assert!(buf.iter().skip(1).step_by(2).all(|r| *r == 0.0));
        assert!(buf.iter().step_by(2).any(|l| *l != 0.0));
    }

    #[test]
    fn toggling_enabled_crossfades_instead_of_clicking() {
        // A -12 dB preamp halves the amplitude twice over: switched hard, the
        // output would jump by up to 0.375 between two samples.
        let state = EqState {
            enabled: false,
            ..enabled([0.0; EQ_BANDS], -12.0)
        };
        let mut eq = stage(&state, 1);
        let input = sine(100.0, 0.5, FS as usize);
        let mut out = input.clone();
        for (block, chunk) in out.chunks_mut(512).enumerate() {
            if block == 30 {
                eq.control().set(&EqState {
                    enabled: true,
                    ..state
                });
            }
            if block == 60 {
                eq.control().set(&state);
            }
            eq.process(chunk);
        }
        let max_step = out
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0_f32, f32::max);
        // The sine itself moves by at most 0.5 * 2 pi * 100 / 48000 = 0.0065.
        assert!(max_step < 0.01, "click: largest step {max_step}");
        // The switch did take effect, and switching off is exact again.
        let on = &out[50 * 512..58 * 512];
        let on_db = 20.0 * (rms(on) / rms(&input[50 * 512..58 * 512])).log10();
        assert!((on_db + 12.0).abs() < 0.01, "enabled level {on_db}");
        let tail = 70 * 512;
        assert_eq!(bits(&out[tail..]), bits(&input[tail..]));
    }

    #[test]
    fn gain_changes_are_slewed() {
        let mut eq = stage(&enabled([0.0; EQ_BANDS], 0.0), 1);
        let input = sine(60.0, 0.5, FS as usize / 2);
        let mut out = input.clone();
        for (block, chunk) in out.chunks_mut(256).enumerate() {
            if block == 20 {
                let mut bands = [0.0; EQ_BANDS];
                bands[0] = 12.0;
                eq.control().set(&enabled(bands, -12.0));
            }
            eq.process(chunk);
        }
        let max_step = out
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0_f32, f32::max);
        // The 60 Hz sine moves by at most 0.5 * 2 pi * 60 / 48000 = 0.0039.
        assert!(max_step < 0.01, "click: largest step {max_step}");
    }

    #[test]
    fn reset_restores_a_fresh_stage() {
        let mut bands = [0.0; EQ_BANDS];
        bands[0] = 12.0;
        bands[9] = -9.0;
        let state = enabled(bands, 3.0);

        let mut used = stage(&state, 2);
        let mut warm = noise(8192 * 2);
        used.process(&mut warm);
        used.reset();

        let mut fresh = stage(&state, 2);
        let mut impulse = vec![0.0_f32; 2048 * 2];
        impulse[0] = 1.0;
        impulse[1] = -1.0;
        let mut a = impulse.clone();
        let mut b = impulse;
        used.process(&mut a);
        fresh.process(&mut b);
        assert_eq!(bits(&a), bits(&b));
    }

    #[test]
    fn bands_above_the_nyquist_guard_are_skipped() {
        let mut bands = [0.0; EQ_BANDS];
        bands[7] = 12.0;
        bands[8] = 12.0;
        bands[9] = 12.0;
        let mut eq = Equalizer::new();
        eq.control().set(&enabled(bands, 0.0));
        eq.prepare(AudioSpec::new(22_050, 2)).unwrap();
        let input = noise(4096 * 2);
        let mut out = input.clone();
        eq.process(&mut out);
        assert_eq!(bits(&out), bits(&input));
        assert!(out.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn out_of_range_and_non_finite_gains_are_sanitized() {
        let mut bands = [f32::NAN; EQ_BANDS];
        bands[4] = 40.0;
        let measured = gain_db(&enabled(bands, f32::INFINITY), BAND_CENTERS_HZ[4]);
        assert!((measured - 12.0).abs() < 0.05, "measured {measured}");
    }

    /// Every band hits its own centre gain at both anchors, not just band 4:
    /// the peaking section has exactly the requested gain at its centre by
    /// construction. Tolerance matches `s09_anchors_...` (±0.05 dB).
    #[test]
    fn all_bands_hit_their_centre_gain_at_both_anchors() {
        for band in 0..EQ_BANDS {
            for db in [12.0_f32, -12.0] {
                let mut bands = [0.0; EQ_BANDS];
                bands[band] = db;
                let measured = gain_db(&enabled(bands, 0.0), BAND_CENTERS_HZ[band]);
                assert!(
                    (measured - f64::from(db)).abs() < 0.05,
                    "band {band} at {db} dB: measured {measured:.3} dB"
                );
            }
        }
    }

    /// The intermediate law is linear in dB (own choice): ±6 dB land halfway
    /// between the anchors, at the centre and through the preamp.
    #[test]
    fn intermediate_gains_are_linear_in_db() {
        let mut bands = [0.0; EQ_BANDS];
        bands[4] = 6.0;
        let measured = gain_db(&enabled(bands, 0.0), BAND_CENTERS_HZ[4]);
        assert!((measured - 6.0).abs() < 0.05, "measured {measured}");

        // Preamp at +6 dB: the exact linear factor, bit for bit.
        let input = noise(1024);
        let mut out = input.clone();
        stage(&enabled([0.0; EQ_BANDS], 6.0), 1).process(&mut out);
        let factor = 10.0_f64.powf(6.0 / 20.0);
        for (x, y) in input.iter().zip(out.iter()) {
            let expected = (f64::from(*x) * factor) as f32;
            assert_eq!(y.to_bits(), expected.to_bits());
        }
    }

    /// DC is unity through the bands: a constant returns unchanged (times the
    /// exact preamp factor), even with every band at full boost. Peaking
    /// sections are unity at DC by construction; the last half second is
    /// measured so the step transient has settled.
    #[test]
    fn dc_passes_at_unity_through_boosted_bands() {
        let dc = vec![0.5_f32; FS as usize];
        for preamp in [0.0_f32, 12.0, -12.0] {
            let mut out = dc.clone();
            stage(&enabled([12.0; EQ_BANDS], preamp), 1).process(&mut out);
            let factor = 10.0_f64.powf(f64::from(preamp) / 20.0) as f32;
            let half = FS as usize / 2;
            for (i, y) in out[half..].iter().enumerate() {
                let want = 0.5 * factor;
                assert!(
                    (y - want).abs() <= 1e-5 * want.abs().max(1e-6),
                    "preamp {preamp} dB, frame {}: {y} != {want}",
                    half + i
                );
            }
        }
    }

    /// A hostile block (NaN, ±infinity) comes out as silence, stays finite,
    /// and leaves no poison behind: the next block matches a stage that saw
    /// zeros instead, bit for bit (the sanitizer maps non-finite to exactly
    /// `0.0` before the filters).
    #[test]
    fn non_finite_input_becomes_silence_without_poisoning_history() {
        let state = enabled(
            [12.0, -12.0, 6.0, -6.0, 3.0, -3.0, 9.0, -9.0, 12.0, 0.0],
            6.0,
        );
        let mut hostile = noise(2048);
        hostile[0] = f32::NAN;
        hostile[500] = f32::INFINITY;
        hostile[1000] = f32::NEG_INFINITY;
        hostile[1500] = f32::NAN;

        let mut used = stage(&state, 1);
        let mut out = hostile.clone();
        used.process(&mut out);
        assert!(
            out.iter().all(|s| s.is_finite()),
            "nothing non-finite leaves"
        );

        // What follows the poison must equal what follows zeros.
        let probe = sine(440.0, 0.25, 2048);
        let mut a = probe.clone();
        used.process(&mut a);
        let mut fresh = stage(&state, 1);
        let mut zeros = vec![0.0; 2048];
        fresh.process(&mut zeros);
        let mut b = probe.clone();
        fresh.process(&mut b);
        assert_eq!(
            bits(&a),
            bits(&b),
            "history after poison == history after zeros"
        );
    }

    /// Subnormal signal values pass through the processing path unchanged:
    /// they are finite and cannot poison the history. (The resampled path
    /// flushes them in the converter; full bypass never touches the buffer.)
    #[test]
    fn subnormal_signal_passes_through_unchanged() {
        let input: Vec<f32> = noise(2048).iter().map(|s| s * 1e-40).collect();
        assert!(input.iter().all(|s| s.is_subnormal() || *s == 0.0));
        let mut out = input.clone();
        // An enabled, flat stage: the processing loop runs (no early bypass)
        // but no band and no preamp touches the samples.
        stage(&enabled([0.0; EQ_BANDS], 0.0), 1).process(&mut out);
        assert_eq!(bits(&out), bits(&input), "subnormals pass through");
    }
}
