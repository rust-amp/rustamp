//! The whole DSP chain against the resampler, with numbers (A14, re-scoped by
//! A17).
//!
//! The defect this pins (A6 finding 3): the chain is
//! `decoder -> Equalizer -> [stages] -> Limiter -> FormatConverter -> ring`.
//! Since A17 the limiter is a true-peak limiter: its 4x-oversampled sidechain
//! estimates the reconstructed peak and limits that to the ceiling, so the
//! band-limited resampler after it has nothing left to rebuild above full
//! scale. The converter keeps its final `±1.0` (0 dBFS) ceiling on every
//! path as the last resort, so the ring never holds a sample above full
//! scale.
//!
//! What each test proves, and what it cannot:
//!
//! * `eq_boost_then_limiter_then_resampler_holds_0dbfs`: a music-like hot
//!   signal (+12 dB EQ on a 0.5 sine) is limited to the limiter ceiling before
//!   the resampler and stays at full scale after it.
//! * `limiter_stops_the_rebuild_and_the_converter_has_nothing_to_clamp`: the
//!   A6 torture tone (samples at the ceiling, true peak +2.7 dBFS) is pulled
//!   down by the true-peak criterion (gain ~= -3.01 dB, the rebuild itself)
//!   to a sample peak of ~= 0.683 and a true peak at the ceiling; after
//!   resampling the peak sits at ~= 0.966-0.968 with zero samples clamped at
//!   full scale — the flat-topping the old safety net caused is gone.
//! * `disabled_limiter_still_holds_0dbfs`: the guarantee does not depend on
//!   the limiter switch — with the limiter off, pre-converter peaks above
//!   full scale are still held at `1.0` after resampling.
//!
//! What they cannot prove: audibility on real material, behaviour on real
//! devices (whether a WASAPI mixer clips or soft-limits `> 1.0` floats is a
//! hardware question), or the estimator's residual near Nyquist — the
//! converter net stays as the last resort for exactly that residual.

use std::sync::Arc;

use rustamp_audio::FormatConverter;
use rustamp_core::audio::{AudioSpec, DspStage};
use rustamp_core::state::EqState;
use rustamp_dsp::{EqControl, Equalizer, Limiter, LimiterControl};

const SRC: u32 = 44_100;
const DST: u32 = 48_000;

/// A sine at `freq` Hz, `amplitude`, one second at 44.1 kHz.
fn sine(freq: f64, amplitude: f64) -> Vec<f32> {
    (0..SRC as usize)
        .map(|n| {
            (amplitude * (2.0 * std::f64::consts::PI * freq * n as f64 / f64::from(SRC))).sin()
                as f32
        })
        .collect()
}

/// The A6 tone: `S·sin(πn/2 + π/4)` has samples of exactly `±S·√2/2` and a
/// true peak of `S`.
fn a6_tone(s: f64) -> Vec<f32> {
    (0..SRC as usize)
        .map(|n| {
            (s * (std::f64::consts::FRAC_PI_4 + std::f64::consts::PI * n as f64 / 2.0).sin()) as f32
        })
        .collect()
}

fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
}

fn eq_state(band_1k_db: f32, preamp_db: f32) -> EqState {
    let mut bands = [0.0; 10];
    bands[4] = band_1k_db;
    EqState {
        bands,
        preamp: preamp_db,
        enabled: true,
        auto: false,
    }
}

/// Equalizer then limiter, the engine's chain order, on mono audio.
fn through_chain(input: &[f32], eq: &EqState, limiter_on: bool) -> Vec<f32> {
    let mut equalizer = Equalizer::with_control(Arc::new(EqControl::new(eq)));
    equalizer.prepare(AudioSpec::new(SRC, 1)).unwrap();
    let mut out = input.to_vec();
    equalizer.process(&mut out).unwrap();

    let mut limiter = Limiter::with_control(Arc::new(LimiterControl::new(limiter_on)));
    limiter.prepare(AudioSpec::new(SRC, 1)).unwrap();
    // A silent tail lets the look-ahead's worth of frames come out.
    out.extend(std::iter::repeat_n(0.0, 512));
    limiter.process(&mut out).unwrap();
    out
}

/// The chain output through the 44.1 kHz -> 48 kHz converter.
fn resample(input: &[f32]) -> Vec<f32> {
    let mut c = FormatConverter::new(AudioSpec::new(SRC, 1), AudioSpec::new(DST, 1));
    let mut all = Vec::new();
    let mut buf = vec![0.0; c.max_out_frames(1024)];
    for chunk in input.chunks(1024) {
        let n = c.convert(chunk, &mut buf);
        all.extend_from_slice(&buf[..n]);
    }
    let mut tail = vec![0.0; c.max_out_frames(2 * c.latency_frames())];
    let n = c.flush(&mut tail);
    all.extend_from_slice(&tail[..n]);
    all
}

#[test]
fn eq_boost_then_limiter_then_resampler_holds_0dbfs() {
    // A 1 kHz sine at 0.5, +12 dB at the 1 kHz band: the limiter must work.
    let limited = through_chain(&sine(1_000.0, 0.5), &eq_state(12.0, 0.0), true);
    let ceiling = Limiter::with_control(Arc::new(LimiterControl::new(true))).ceiling();
    let pre = peak(&limited);
    assert!(pre <= ceiling, "limiter output {pre} > {ceiling}");

    let out = resample(&limited);
    assert_eq!(
        out.len(),
        (limited.len() * 160).div_ceil(147),
        "the converter tail included"
    );
    let post = peak(&out);
    assert!(post <= 1.0, "above 0 dBFS after resampling: {post}");
}

#[test]
fn limiter_stops_the_rebuild_and_the_converter_has_nothing_to_clamp() {
    // A17: the limiter is a true-peak limiter now, so it can stop the
    // rebuild the old sample-peak criterion was blind to. The A6 torture
    // tone (samples at the ceiling, true peak 1.3662 = +2.7 dBFS) is pulled
    // down by ~= -3.01 dB (the rebuild itself): the settled sample peak is
    // ~= 0.683 and the true peak sits at the ceiling.
    let ceiling = Limiter::with_control(Arc::new(LimiterControl::new(true))).ceiling();
    assert!((f64::from(ceiling) - 10.0_f64.powf(-0.3 / 20.0)).abs() < 1e-7);
    let limited = through_chain(&a6_tone(1.3662), &eq_state(0.0, 0.0), true);
    let pre = peak(&limited);
    // The limiter engaged (far below the ceiling it used to pass through)
    // and settled where A17 measured it: ~= 0.683, inside [0.66, 0.70].
    assert!(
        pre < ceiling,
        "the true-peak limiter must engage: {pre} vs ceiling {ceiling}"
    );
    assert!(
        (0.66..=0.70).contains(&pre),
        "limiter output {pre}, want ~= 0.683 (A17 §3)"
    );

    let out = resample(&limited);
    let post = peak(&out);
    // The resampled true peak sits at the ceiling within the estimator's
    // error: ceiling·0.99 <= post <= 1.0 is the safe form (A17 §6.1).
    // Before A17 this was exactly 1.0 with ~= 47 % of samples clamped.
    assert!(
        post >= ceiling * 0.99,
        "resampled peak {post} should reach the ceiling {ceiling}"
    );
    assert!(post <= 1.0, "above 0 dBFS after resampling: {post}");
    // The flat-topping is gone: nothing sits at full scale.
    let clamped = out.iter().filter(|s| s.abs() == 1.0).count();
    assert_eq!(clamped, 0, "the safety net must have nothing to clamp");
}

#[test]
fn disabled_limiter_still_holds_0dbfs() {
    // Limiter off, +12 dB EQ on a 0.5 sine: the pre-converter signal reaches
    // about 2.0, far above full scale.
    let hot = through_chain(&sine(1_000.0, 0.5), &eq_state(12.0, 0.0), false);
    let pre = peak(&hot);
    assert!(pre > 1.5, "expected a hot pre-converter signal, got {pre}");

    let out = resample(&hot);
    let post = peak(&out);
    assert!(post <= 1.0, "above 0 dBFS with the limiter off: {post}");
    // The safety net engaged (not a vacuous bound on a quiet signal).
    assert!((post - 1.0).abs() < 1e-6, "post {post}, want exactly 1.0");
}
