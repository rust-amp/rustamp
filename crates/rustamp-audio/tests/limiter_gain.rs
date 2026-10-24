//! The limiter's ceiling must come from its gain, not from its final clamp.
//!
//! A6 test audit: `a_hot_signal_is_held_at_the_ceiling` and
//! `look_ahead_shapes_the_transient_instead_of_clipping_it` (in
//! `rustamp-dsp`) check `peak <= ceiling` on samples that all went through
//! `guard(..).clamp(±ceiling)`, so a broken hold window or a misaligned attack
//! average still passes them: the clamp hides the overshoot. This test reads
//! the gain itself. The limiter applies one gain to all channels of a frame,
//! so with a hot left channel and a quieter right one, `out / in` must be the
//! same on both sides. Wherever the construction let the hot side overshoot,
//! the clamp would cut only that side, and its ratio would fall below the
//! other side's.
//!
//! It lives here, not next to the limiter, because this lane may not edit
//! `rustamp-dsp`; it uses only the stage's public API.

use std::f64::consts::PI;
use std::sync::Arc;

use rustamp_core::audio::{AudioSpec, DspStage};
use rustamp_dsp::{Limiter, LimiterControl};

const FS: u32 = 48_000;

/// Deterministic white-ish noise in `-0.5..0.5`.
fn noise(n: usize, mut seed: u32) -> Vec<f32> {
    (0..n)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        })
        .collect()
}

#[test]
fn the_ceiling_comes_from_the_linked_gain_not_from_the_clamp() {
    let mut limiter = Limiter::with_control(Arc::new(LimiterControl::new(true)));
    limiter.prepare(AudioSpec::new(FS, 2)).unwrap();
    let la = limiter.latency_frames() as usize;
    assert!(la > 0, "the enabled limiter reports its look-ahead");
    let ceiling = limiter.ceiling();

    // Left: a 1 kHz sine at 3.0 with bursts of noise, far over the ceiling.
    // Right: noise of at most 0.5, which only the linked gain turns down.
    let frames = FS as usize;
    let hot = noise(frames, 11);
    let quiet = noise(frames, 12);
    let mut input: Vec<f32> = (0..frames)
        .flat_map(|n| {
            let burst = if (n / 2_000) % 3 == 0 { 2.0 } else { 0.0 };
            let sine = 3.0 * (2.0 * PI * 1_000.0 * n as f64 / f64::from(FS)).sin();
            [sine as f32 + burst * hot[n], quiet[n]]
        })
        .collect();
    // A silent tail lets the last look-ahead's worth of frames come out.
    input.extend(std::iter::repeat_n(0.0, 2 * la * 2));
    let mut out = input.clone();
    for block in out.chunks_mut(2 * 1_000) {
        limiter.process(block).unwrap();
    }

    let peak = out.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    assert!(peak <= ceiling, "overshoot: {peak} > {ceiling}");

    let mut compared = 0;
    let mut limited = 0;
    for (i, frame) in out.as_chunks::<2>().0.iter().enumerate().skip(la) {
        let src = &input[(i - la) * 2..(i - la) * 2 + 2];
        // Ratios only where both inputs are large enough to read one.
        if src[0].abs() < 0.05 || src[1].abs() < 0.05 {
            continue;
        }
        let left = f64::from(frame[0]) / f64::from(src[0]);
        let right = f64::from(frame[1]) / f64::from(src[1]);
        compared += 1;
        if right < 0.99 {
            limited += 1;
        }
        // f32 rounding of each product is below 1e-7 relative; a clamp that
        // shaped the hot side would show up far above that.
        assert!(
            (left - right).abs() <= 1e-6 * right.max(1e-3),
            "frame {i}: gain {left} on the hot side, {right} on the quiet side"
        );
    }
    assert!(compared > frames / 4, "too few frames compared: {compared}");
    assert!(
        limited > frames / 4,
        "the limiter barely limited: {limited}"
    );
}
