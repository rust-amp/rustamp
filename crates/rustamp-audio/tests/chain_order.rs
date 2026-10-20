//! The DSP chain order, measured (C2-3 of the W14 wave review).
//!
//! The wave review recorded the claim "ReplayGain gain is applied once and
//! in the correct order" as **HOLDS**, with the evidence "Order is verified:
//! ReplayGain -> Equalizer -> installed DSP stages -> Limiter -> Converter
//! -> Output ring" (`docs/tasks/W14-wave-review.md`). That evidence is a
//! reading of the source, not a measurement: nothing in the tree pinned the
//! order end to end.
//!
//! This file measures it. Every test drives a virtual device
//! ([`rustamp_output::NullOutput`]) and the fixture decoders, so no window
//! opens and no audio plays.
//!
//! # The order under test
//!
//! ```text
//! decoder -> ReplayGain -> Equalizer -> [installed stages] -> Limiter
//!         -> FormatConverter -> ring -> volume/balance -> fade -> device
//! ```
//!
//! # The instruments
//!
//! * **The probe** is a [`DspStage`] the caller installs. The engine puts
//!   the equalizer first and the limiter last in the decode chain, so the
//!   probe sits between them: it measures the level the limiter is asked to
//!   limit, and the level the chain handed over.
//! * **The visualization tap** sits between the chain and the converter: it
//!   shows what left the chain, after the decode thread's sanitizer and
//!   before the format converter.
//! * **The device** is the virtual output: what the player would send to
//!   the sound card.
//!
//! # What each test pins
//!
//! * `the_replaygain_factor_is_applied_exactly_once` — the same track, the
//!   same source frames, one run with the gain off and one with it on: the
//!   output carries the factor exactly once, not twice and not zero times.
//! * `the_chain_runs_in_the_documented_order` — the probe measures what the
//!   limiter is asked to limit, and the device's peak says what came out:
//!   the limiter caught the boosted signal and the boost did not escape it.
//! * `the_bounds_hold_after_the_replaygain_factor` — a hot track stays
//!   inside the limiter's ceiling and, with the limiter off, inside the
//!   converter's final clamp, on the equal-rate and on the resampled path.
//! * `volume_and_balance_are_applied_after_the_chain` — the volume and the
//!   balance do not move the chain's level; they scale the device's output.
//! * `the_fade_is_an_output_stage_gain` — the fade ramps the device's
//!   output down and leaves the chain alone.
//! * `a_seeded_random_signal_stays_finite_and_bounded` — a seeded signal
//!   with infinities, NaNs and values far above full scale leaves the chain
//!   finite and inside full scale, with the limiter on and off.
//!
//! # What these tests cannot prove
//!
//! **The position of the ReplayGain factor relative to the equalizer is not
//! separable in the sample values.** The equalizer is a linear stage (ten
//! peaking sections, a linear preamp, a linear wet/dry mix) and the factor
//! is a constant, so "the gain before the equalizer" and "the gain after
//! the equalizer" produce the same samples — bit for bit, up to the rounding
//! of the multiply. What *is* separable, and is measured here, is that the
//! factor is inside the chain and before the limiter (the limiter is the one
//! non-linear stage, and it sees the product of the factor and the
//! equalizer's response) and that it is not applied after the chain, where
//! the render callback's volume/balance sit and where nothing would catch a
//! boost any more.
//!
//! Nothing here says how the chain sounds, and nothing here measures a real
//! device: the output is a virtual one.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use common::{Harness, SPEC, finished, pattern};
use rustamp_audio::EngineConfig;
use rustamp_audio::replaygain::{ReplayGainMode, apply_gain};
use rustamp_core::audio::{AudioSpec, DspStage};
use rustamp_core::command::Command;
use rustamp_core::error::Result;
use rustamp_core::state::EqState;
use rustamp_dsp::{EqControl, Equalizer, Limiter, LimiterControl};

/// The frames of the long fixture tracks (one second at 44.1 kHz).
const FRAMES: u64 = 44_100;

/// The frames of the short noise track: exactly the width of the
/// visualization tap's window, so the tap's frame shows every sample that
/// left the chain.
const NOISE_FRAMES: u64 = 576;

/// A stall escape for the waits in this file: generous enough that only a
/// real stall reaches it.
const WAIT: Duration = Duration::from_secs(30);

/// The peak of a buffer.
fn peak(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0_f32, |m, s| m.max(s.abs()))
}

/// The source of the fixture track, as the decoder produces it.
fn source(seed: u64, frames: u64) -> Vec<f32> {
    (0..frames).map(|i| pattern(seed, i)).collect()
}

/// The same source, interleaved into the stereo frames the chain works on.
fn stereo(source: &[f32]) -> Vec<f32> {
    source.iter().flat_map(|v| [*v, *v]).collect()
}

/// The equalizer state the order tests use: a +12 dB preamp and flat bands.
///
/// The preamp is a plain gain, so the equalizer's response is known exactly:
/// the chain's level is the source's level times the preamp's factor.
fn preamp_eq() -> EqState {
    EqState {
        bands: [0.0; 10],
        preamp: 12.0,
        enabled: true,
        auto: false,
    }
}

// ---------------------------------------------------------------------------
// The probe: a stage that measures the level it sees
// ---------------------------------------------------------------------------

/// A stage that measures the level it sees.
///
/// The engine puts the equalizer first and the limiter last in the decode
/// chain, so a stage the caller installs sits between them: the probe
/// measures the level the limiter is asked to limit.
struct Probe {
    /// The peak of the finite samples seen so far, as f32 bits.
    peak: Arc<AtomicU32>,
    /// The peak of the last block, as f32 bits.
    last: Arc<AtomicU32>,
    /// The frames processed so far.
    frames: Arc<AtomicU64>,
    /// The non-finite samples seen so far.
    non_finite: Arc<AtomicU64>,
    /// The channel count the stage was prepared for.
    channels: u16,
}

/// The readings of a [`Probe`], from the test's thread.
#[derive(Clone)]
struct ProbeHandle {
    peak: Arc<AtomicU32>,
    last: Arc<AtomicU32>,
    frames: Arc<AtomicU64>,
    non_finite: Arc<AtomicU64>,
}

impl Probe {
    /// A probe and the handle that reads it.
    fn install() -> (Box<dyn DspStage>, ProbeHandle) {
        let peak = Arc::new(AtomicU32::new(0));
        let last = Arc::new(AtomicU32::new(0));
        let frames = Arc::new(AtomicU64::new(0));
        let non_finite = Arc::new(AtomicU64::new(0));
        let stage = Probe {
            peak: Arc::clone(&peak),
            last: Arc::clone(&last),
            frames: Arc::clone(&frames),
            non_finite: Arc::clone(&non_finite),
            channels: 1,
        };
        let handle = ProbeHandle {
            peak,
            last,
            frames,
            non_finite,
        };
        (Box::new(stage), handle)
    }
}

impl ProbeHandle {
    /// The peak of the finite samples the probe has seen.
    fn peak(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
    }

    /// The peak of the last block the probe processed.
    fn last_peak(&self) -> f32 {
        f32::from_bits(self.last.load(Ordering::Relaxed))
    }

    /// The frames the probe has processed.
    fn frames(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }

    /// The non-finite samples the probe has seen.
    fn non_finite(&self) -> u64 {
        self.non_finite.load(Ordering::Relaxed)
    }
}

impl DspStage for Probe {
    fn name(&self) -> &str {
        "probe"
    }

    fn prepare(&mut self, spec: AudioSpec) -> Result<()> {
        self.channels = spec.channels.max(1);
        Ok(())
    }

    fn process(&mut self, buf: &mut [f32]) -> Result<()> {
        let mut peak = 0.0_f32;
        let mut non_finite = 0_u64;
        for sample in buf.iter() {
            if sample.is_finite() {
                peak = peak.max(sample.abs());
            } else {
                non_finite += 1;
            }
        }
        self.frames.fetch_add(
            (buf.len() / usize::from(self.channels)) as u64,
            Ordering::Relaxed,
        );
        self.non_finite.fetch_add(non_finite, Ordering::Relaxed);
        // The bits of a non-negative finite f32 order like the numbers they
        // hold, so the peak is a plain max on the bits.
        let bits = peak.to_bits();
        self.last.store(bits, Ordering::Relaxed);
        let mut current = self.peak.load(Ordering::Relaxed);
        while bits > current {
            match self.peak.compare_exchange_weak(
                current,
                bits,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => current = actual,
            }
        }
        Ok(())
    }

    fn reset(&mut self) {}

    fn latency_frames(&self) -> u32 {
        0
    }
}

// ---------------------------------------------------------------------------
// The visualization tap: what left the chain
// ---------------------------------------------------------------------------

/// What the visualization tap saw.
///
/// The tap sits between the chain and the converter, so its window holds
/// the samples the chain handed over — after the decode thread's sanitizer
/// and before the format converter.
#[derive(Default)]
struct Vis {
    /// The frames the engine published.
    frames: usize,
    /// The non-finite samples in them.
    non_finite: usize,
}

impl Vis {
    /// Take the frames the engine has published since the last call.
    ///
    /// The engine keeps only the newest frame, so the frames are collected
    /// as the run goes.
    fn take(&mut self, h: &mut Harness) {
        while let Some(frame) = h.engine.take_vis_frame() {
            self.frames += 1;
            for channel in frame.waveform.iter() {
                for sample in channel.iter() {
                    if !sample.is_finite() {
                        self.non_finite += 1;
                    }
                }
            }
        }
    }
}

/// What a run produced.
struct Played {
    /// The samples the device got.
    device: Vec<f32>,
    /// What the visualization tap saw.
    vis: Vis,
}

// ---------------------------------------------------------------------------
// The runs
// ---------------------------------------------------------------------------

/// A harness with a probe stage installed between the equalizer and the
/// limiter.
fn probed(track: &str) -> (Harness, ProbeHandle) {
    let mut h = Harness::new(&[track]);
    let (stage, probe) = Probe::install();
    h.engine.set_dsp_chain(vec![stage]);
    (h, probe)
}

/// Play the whole track and record what the device got.
///
/// The run ends when the decode thread has processed the whole track, which
/// is also when the probe has seen every frame of it.
fn play_whole(h: &mut Harness, probe: &ProbeHandle, frames: u64) -> Played {
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0), "the track must open");
    // Wait until the ring holds something, so the first callback is not an
    // underrun. A short track never fills the ring.
    let fill = (frames.min(4096) / 2).max(1);
    h.wait_refilled(fill).expect("the ring must fill");
    let mut device = Vec::new();
    let mut vis = Vis::default();
    let deadline = Instant::now() + WAIT;
    while probe.frames() < frames {
        assert!(
            Instant::now() < deadline,
            "the decode thread must reach the end of the track"
        );
        h.engine.poll().unwrap();
        vis.take(h);
        h.clock.pump_record(256, &mut device);
        std::thread::sleep(Duration::from_millis(1));
    }
    // The decode thread may still be inside the block it is processing: the
    // probe counts the frames before the chain hands them over, and the tap
    // publishes its window after that. Wait for the tap, so the run's frames
    // are seen.
    let deadline = Instant::now() + WAIT;
    while vis.frames == 0 {
        assert!(Instant::now() < deadline, "the tap must publish a frame");
        h.engine.poll().unwrap();
        vis.take(h);
        std::thread::sleep(Duration::from_millis(1));
    }
    // The device must have played the track: what the ring still holds is
    // played now. A short track is decoded in one go, so the loop above may
    // never have run at all.
    h.clock.pump_record(frames, &mut device);
    h.engine.poll().unwrap();
    vis.take(h);
    Played { device, vis }
}

/// The first `frames` frames the device plays, with the given output gains.
///
/// The chain is left alone (the limiter off, the equalizer flat), so the
/// device's output is the source, finished by the converter, times the
/// volume and the balance.
fn play_block(track: &str, volume: u8, balance: i8, frames: u64) -> (Vec<f32>, u64, f32) {
    let (mut h, probe) = probed(track);
    h.engine.set_limiter_enabled(false);
    h.engine.set_volume(volume).unwrap();
    h.engine.set_balance(balance).unwrap();
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0), "the track must open");
    let p0 = h.wait_refilled(4096).expect("the ring must fill");
    let mut rec = Vec::new();
    h.clock.pump_record(frames, &mut rec);
    h.engine.poll().unwrap();
    (rec, p0, probe.peak())
}

/// The chain the docs describe, built from the public stages: the ReplayGain
/// scale, the equalizer, the limiter.
///
/// With `rg_last` the scale is applied *after* the limiter instead — the
/// order the ReplayGain module rejects, and the one that lets a boost
/// escape the limiter.
fn reference_chain(source: &[f32], rg: f32, eq: &EqState, rg_last: bool) -> Vec<f32> {
    let mut buf = stereo(source);
    if !rg_last {
        apply_gain(&mut buf, rg);
    }
    let mut equalizer = Equalizer::with_control(Arc::new(EqControl::new(eq)));
    equalizer.prepare(SPEC).expect("the equalizer prepares");
    equalizer.process(&mut buf).expect("the equalizer runs");
    let mut limiter = Limiter::with_control(Arc::new(LimiterControl::new(true)));
    limiter.prepare(SPEC).expect("the limiter prepares");
    // A silent tail lets the look-ahead's worth of frames come out.
    buf.extend(std::iter::repeat_n(0.0_f32, 2 * 512));
    limiter.process(&mut buf).expect("the limiter runs");
    if rg_last {
        apply_gain(&mut buf, rg);
    }
    buf
}

/// The gain of the fade ramp at the frame the device played at counter
/// `played`.
///
/// The ramp starts at the first callback after the request, so the frame the
/// device played at counter `played` is the ramp's frame `played - start`.
/// The law is the equal-power one of `rustamp-dsp`'s fade: `sqrt(1 - t)`.
fn fade_gain(played: u64, start: u64, span: u64) -> f32 {
    let k = played.saturating_sub(start) as f64;
    let t = k / span.saturating_sub(1).max(1) as f64;
    (1.0 - t).max(0.0).sqrt() as f32
}

// ---------------------------------------------------------------------------
// The measurements
// ---------------------------------------------------------------------------

#[test]
fn the_replaygain_factor_is_applied_exactly_once() {
    // The same track, the same source frames, twice: once with the gain off
    // and once with the track gain in effect. The limiter is off and the
    // equalizer is flat, so the chain is transparent and the only thing that
    // can differ between the two runs is the factor.
    let track = "rg_-6.02_none_1_44100.tst";
    let factor = 10f32.powf(-6.02 / 20.0);
    assert!(
        (factor - 0.5).abs() < 1e-3,
        "−6.02 dB halves the samples: {factor}"
    );

    let (p0_off, off) = record_gain_run(track, ReplayGainMode::Off);
    let (p0_on, on) = record_gain_run(track, ReplayGainMode::Track);
    assert_eq!(
        p0_off, p0_on,
        "both runs must record the same source frames"
    );
    assert_eq!(off.len(), on.len());

    // The gain-off run is the source, finished by the converter. This pins
    // the alignment of the two blocks on the same source frames.
    for (i, got) in off.iter().enumerate() {
        let want = finished(pattern(1, p0_off + i as u64));
        assert!(
            (got - want).abs() < 1e-7,
            "frame {i}: got {got}, want {want} (the gain-off run)"
        );
    }

    // The factor appears once: the gain-on run is the gain-off run times the
    // factor, and neither twice the factor nor no factor at all.
    let mut compared = 0_usize;
    for (i, (off, on)) in off.iter().zip(on.iter()).enumerate() {
        if *off >= 1.0 {
            // The converter held this frame at full scale: the factor is not
            // visible in it.
            continue;
        }
        let want = off * factor;
        assert!(
            (on - want).abs() <= 1e-6,
            "frame {i}: got {on}, want {want} (the factor, once)"
        );
        assert!(
            (on - off * factor * factor).abs() > 0.05 * want,
            "frame {i}: the factor is applied twice: {on}"
        );
        assert!(
            (on - off).abs() > 0.05 * want,
            "frame {i}: the factor is not applied at all: {on}"
        );
        compared += 1;
    }
    assert!(compared > 1000, "only {compared} frames carried the factor");
}

/// Record the first frames the device plays, with the ReplayGain mode set.
fn record_gain_run(track: &str, mode: ReplayGainMode) -> (u64, Vec<f32>) {
    let mut h = Harness::new(&[track]);
    h.engine.set_limiter_enabled(false);
    h.engine.set_replaygain_mode(mode);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0), "the track must open");
    let p0 = h.wait_refilled(4096).expect("the ring must fill");
    let mut rec = Vec::new();
    h.clock.pump_record(4096, &mut rec);
    h.engine.poll().unwrap();
    (p0, rec.iter().step_by(2).copied().collect())
}

#[test]
fn the_chain_runs_in_the_documented_order() {
    // The order under test:
    //
    //   decoder -> ReplayGain -> Equalizer -> [stages] -> Limiter
    //           -> converter -> ring -> volume/balance -> fade -> device
    //
    // The track carries a +6.02 dB track gain and the equalizer a +12 dB
    // preamp, so the chain's level is about 8.7 — far above the limiter's
    // ceiling, and the limiter has to act.
    let track = "rg_+6.02_none_1_44100.tst";
    let (mut h, probe) = probed(track);
    h.engine.set_replaygain_mode(ReplayGainMode::Track);
    h.engine.set_eq(preamp_eq());
    let played = play_whole(&mut h, &probe, FRAMES);

    let rg = 10f32.powf(6.02 / 20.0);
    let eq = 10f64.powf(12.0 / 20.0);
    let source_peak = peak(&source(1, FRAMES));

    // 1. The limiter sees the product: the factor and the equalizer's
    //    response are both in the signal at the probe, which sits before the
    //    limiter. A factor applied after the equalizer but before the
    //    limiter would look the same here — the equalizer is linear — but a
    //    factor applied after the limiter would not.
    let want_probe = (f64::from(source_peak * rg) * eq) as f32;
    let seen = probe.peak();
    assert!(
        (seen - want_probe).abs() <= 1e-3 * want_probe,
        "the level the limiter is asked to limit: got {seen}, want {want_probe}"
    );

    // 2. The limiter caught it: the device's peak is the limiter's ceiling,
    //    not the level the probe saw and not the converter's clamp.
    let ceiling = h.engine.limiter_ceiling();
    let device_peak = peak(&played.device);
    assert!(
        device_peak <= ceiling + 1e-3,
        "the limiter must hold the signal: {device_peak} > {ceiling}"
    );
    assert!(
        device_peak >= ceiling * 0.9,
        "the limiter must engage, not pass the signal: {device_peak}"
    );
    assert!(
        device_peak < 0.99,
        "the converter's clamp must not be what holds the signal: {device_peak}"
    );

    // 3. The measured value follows the documented order, and not the order
    //    the ReplayGain module rejects: the same source through the same
    //    stages, with the factor applied after the limiter instead.
    let documented = reference_chain(&source(1, FRAMES), rg, &preamp_eq(), false);
    let alternative = reference_chain(&source(1, FRAMES), rg, &preamp_eq(), true);
    let documented_peak = peak(&documented);
    let alternative_peak = peak(&alternative);
    assert!(
        (documented_peak - device_peak).abs() <= 0.02 * documented_peak,
        "the measured peak must follow the documented order: got {device_peak}, \
         want {documented_peak}"
    );
    assert!(
        alternative_peak > 1.5,
        "the alternative order must let the boost escape the limiter: {alternative_peak}"
    );
    assert!(
        device_peak < alternative_peak,
        "the measured peak must not follow the alternative order: {device_peak} vs \
         {alternative_peak}"
    );
}

#[test]
fn the_bounds_hold_after_the_replaygain_factor() {
    // The same hot track as the order test: the track gain and the
    // equalizer's preamp put the chain's level at about 8.7.
    let track = "rg_+6.02_none_1_44100.tst";

    // With the limiter on, the device's peak is the limiter's ceiling.
    let (mut h, probe) = probed(track);
    h.engine.set_replaygain_mode(ReplayGainMode::Track);
    h.engine.set_eq(preamp_eq());
    let played = play_whole(&mut h, &probe, FRAMES);
    let ceiling = h.engine.limiter_ceiling();
    let hot_limited = peak(&played.device);
    assert!(
        hot_limited <= ceiling + 1e-3,
        "a hot track must not pass the ceiling: {hot_limited} > {ceiling}"
    );
    assert!(
        probe.peak() > 2.0,
        "the chain's level must have been hot: {}",
        probe.peak()
    );

    // With the limiter off, the converter's final clamp holds it at full
    // scale.
    let (mut h, probe) = probed(track);
    h.engine.set_limiter_enabled(false);
    h.engine.set_replaygain_mode(ReplayGainMode::Track);
    h.engine.set_eq(preamp_eq());
    let played = play_whole(&mut h, &probe, FRAMES);
    let hot = peak(&played.device);
    assert!(
        hot <= 1.0,
        "nothing above full scale may reach the device: {hot}"
    );
    assert!(
        (hot - 1.0).abs() < 1e-6,
        "the converter's safety net must have engaged: {hot}"
    );

    // The same on the resampled path, where the converter rebuilds the
    // peaks it is given.
    let cfg = EngineConfig {
        spec: AudioSpec::new(48_000, 2),
        ..EngineConfig::default()
    };
    let mut h = Harness::with_config(&[track], cfg);
    let (stage, probe) = Probe::install();
    h.engine.set_dsp_chain(vec![stage]);
    h.engine.set_limiter_enabled(false);
    h.engine.set_replaygain_mode(ReplayGainMode::Track);
    h.engine.set_eq(preamp_eq());
    let played = play_whole(&mut h, &probe, FRAMES);
    let hot = peak(&played.device);
    assert!(
        hot <= 1.0,
        "nothing above full scale may reach the device: {hot}"
    );
    assert!(
        hot > 0.99,
        "the converter's safety net must have engaged: {hot}"
    );
}

#[test]
fn volume_and_balance_are_applied_after_the_chain() {
    // The limiter off and the equalizer flat: the chain leaves the source
    // alone, so the device's output is the source, finished by the
    // converter, times the volume and the balance.
    let track = "track1_44100.tst";
    let (unity, p0, chain_unity) = play_block(track, 255, 0, 4096);
    let (quiet, p1, chain_quiet) = play_block(track, 128, 0, 4096);
    let (left, p2, _) = play_block(track, 128, -127, 4096);
    assert_eq!(p0, p1);
    assert_eq!(p0, p2);

    // The chain's level does not move with the volume: the volume is not a
    // chain gain.
    assert_eq!(
        chain_unity, chain_quiet,
        "the volume must not change the chain's level"
    );
    assert!(
        chain_unity > 1.0,
        "the chain's level must have been the source's: {chain_unity}"
    );

    let volume = 128.0_f32 / 255.0;
    for (i, frame) in unity.as_chunks::<2>().0.iter().enumerate() {
        // With the volume at unity the device gets the finished source.
        let want = finished(pattern(1, p0 + i as u64));
        assert!(
            (frame[0] - want).abs() < 1e-7,
            "frame {i}: got {}, want {want}",
            frame[0]
        );
        assert_eq!(frame[1], want, "the two channels carry the same source");

        // With the volume at 128 the device gets the same, scaled.
        let scaled = want * volume;
        assert!(
            (quiet[i * 2] - scaled).abs() < 1e-7,
            "frame {i}: got {}, want {scaled}",
            quiet[i * 2]
        );
        assert_eq!(quiet[i * 2 + 1], scaled, "the right channel too");

        // The full-left balance mutes the right channel and leaves the left
        // at the volume's gain.
        assert!(
            (left[i * 2] - scaled).abs() < 1e-7,
            "frame {i}: got {}, want {scaled}",
            left[i * 2]
        );
        assert_eq!(
            left[i * 2 + 1],
            0.0,
            "the full-left balance mutes the right"
        );
    }
}

#[test]
fn the_fade_is_an_output_stage_gain() {
    // The fade runs in the render callback, after the chain: it ramps the
    // device's output down and leaves the chain's level alone.
    const FADE_MS: u32 = 500;
    let cfg = EngineConfig {
        fade_ms: FADE_MS,
        ..EngineConfig::default()
    };
    let mut h = Harness::with_config(&["track1_44100.tst"], cfg);
    let (stage, probe) = Probe::install();
    h.engine.set_dsp_chain(vec![stage]);
    h.engine.set_limiter_enabled(false);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0), "the track must open");
    h.wait_refilled(4096).expect("the ring must fill");
    let chain_before = probe.peak();

    // The fade-out starts; the ramp begins at the next callback the device
    // runs, which is the frame at the current position of the clock.
    h.engine.apply(&Command::StopWithFadeout).expect("fade out");
    let ramp_start = h.clock.frames();
    let p1 = h.engine.position_frames();
    let s1 = h.clock.frames();
    let mut during = Vec::new();
    h.clock.pump_record(1024, &mut during);
    h.engine.poll().unwrap();

    // The device's output is the source, finished, times the ramp's gain.
    let span = u64::from(FADE_MS) * u64::from(SPEC.sample_rate) / 1000;
    for (j, frame) in during.as_chunks::<2>().0.iter().enumerate() {
        let want = fade_gain(s1 + j as u64, ramp_start, span) * finished(pattern(1, p1 + j as u64));
        assert!(
            (frame[0] - want).abs() < 1e-6,
            "frame {j}: got {}, want {want}",
            frame[0]
        );
        assert_eq!(frame[1], frame[0], "the two channels carry the same source");
    }

    // The chain's level did not move: the fade is not a chain gain.
    assert!(
        probe.peak() >= chain_before,
        "the chain's level must not fall with the fade"
    );
    assert!(
        probe.last_peak() > 0.9,
        "the chain must still carry the source: {}",
        probe.last_peak()
    );
    // The device's output is well below the chain's level: the ramp is
    // between the chain and the device.
    let device = peak(&during);
    assert!(
        device < chain_before * 0.99,
        "the device's output must be ramped down: {device} vs {chain_before}"
    );
}

#[test]
fn a_seeded_random_signal_stays_finite_and_bounded() {
    // The seeded noise track: infinities, NaNs and values far above full
    // scale, mixed with ordinary ones. It is exactly as long as the
    // visualization tap's window, so the tap's frame shows every sample
    // that left the chain.
    let track = "noise7_576.tst";
    let hostile = (0..NOISE_FRAMES)
        .filter(|i| !common::noise_sample(7, *i).is_finite())
        .count();
    assert!(hostile > 0, "the fixture must carry non-finite samples");

    for limiter in [true, false] {
        let (mut h, probe) = probed(track);
        h.engine.set_limiter_enabled(limiter);
        let played = play_whole(&mut h, &probe, NOISE_FRAMES);

        // The values really entered the chain: the probe sits before the
        // limiter and saw them.
        assert!(
            probe.non_finite() > 0,
            "the chain's input must have carried them (limiter {limiter})"
        );

        // And nothing non-finite left it: the tap's window is what the
        // chain handed over, after the decode thread's sanitizer.
        assert!(
            played.vis.frames > 0,
            "the tap must have published a frame (limiter {limiter})"
        );
        assert_eq!(
            played.vis.non_finite, 0,
            "no non-finite sample may leave the chain (limiter {limiter})"
        );

        // The device's output is finite and inside full scale.
        assert!(
            !played.device.is_empty(),
            "the device must have played the track (limiter {limiter})"
        );
        for (i, sample) in played.device.iter().enumerate() {
            assert!(
                sample.is_finite(),
                "sample {i} is not finite (limiter {limiter}): {sample}"
            );
            assert!(
                sample.abs() <= 1.0,
                "sample {i} is above full scale (limiter {limiter}): {sample}"
            );
        }
    }
}
