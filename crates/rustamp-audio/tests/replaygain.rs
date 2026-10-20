//! ReplayGain on the audio path (A15).
//!
//! Every test drives a virtual device ([`rustamp_output::NullOutput`]) so no
//! window opens and no audio plays. The `rg_` tracks are the shared
//! [`pattern`](common::pattern) samples with ReplayGain tags in their
//! metadata (`rg_<track>_<album>_<seed>_<frames>`; `none` = untagged), so a
//! tagged track and its untagged twin differ only in the tags.
//!
//! What each test pins:
//!
//! * `track_gain_scales_samples_by_the_documented_factor` — a −6.02 dB tag
//!   moves every sample by exactly `10^(−6.02/20)` (≈ 0.5).
//! * `missing_tags_play_unadjusted` — no tag means gain 1.0, bit-exact, even
//!   with a preamp dialed in.
//! * `volume_balance_and_replaygain_combine_in_documented_order` — the
//!   documented order (ReplayGain on the decode side, volume/balance on the
//!   render side) as one combined factor per channel.
//! * `album_mode_uses_album_gain_and_falls_back_to_track` — the mode picks
//!   the source, and each mode falls back to the other tag.
//! * `hot_replaygain_is_held_at_the_limiter_ceiling` — a +6.02 dB correction
//!   whose peak times gain exceeds full scale is held at the limiter ceiling
//!   on the equal-rate path (no pre-attenuation of the tags).
//! * `hot_replaygain_is_held_at_full_scale_after_resampling` — the same hot
//!   correction with the limiter off is held at exactly 1.0 by the
//!   converter's final ceiling on the resampled path.
//! * `replaygain_is_disabled_by_default` — the player changes no loudness
//!   until the user asks.

mod common;

use std::collections::BTreeMap;

use common::{Harness, expected, finished, pattern};
use rustamp_audio::replaygain::{ReplayGainMode, apply_gain, effective_gain};
use rustamp_audio::{EngineConfig, FormatConverter};
use rustamp_core::audio::AudioSpec;

/// Record `frames` left-channel samples of the current run from its start.
fn record_left(h: &mut Harness, frames: u64) -> (u64, Vec<f32>) {
    let p0 = h
        .wait_refilled(4096)
        .expect("the ring must fill at the start of the run");
    let mut rec = Vec::new();
    h.clock.pump_record(frames, &mut rec);
    h.engine.poll().unwrap();
    (p0, rec.iter().step_by(2).copied().collect())
}

#[test]
fn track_gain_scales_samples_by_the_documented_factor() {
    let mut h = Harness::new(&["rg_-6.02_none_1_44100.tst"]);
    h.engine.set_limiter_enabled(false);
    h.engine.set_replaygain_mode(ReplayGainMode::Track);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    let factor = 10f32.powf(-6.02 / 20.0);
    assert!(
        (factor - 0.5).abs() < 1e-3,
        "−6.02 dB must halve the samples: {factor}"
    );

    let (p0, left) = record_left(&mut h, 1024);
    assert_eq!(left.len(), 1024);
    for (i, got) in left.iter().enumerate() {
        // The ReplayGain factor runs in the chain, before the converter: the
        // expectation is the gained source, finished.
        let want = finished(pattern(1, p0 + i as u64) * factor);
        assert!(
            (got - want).abs() < 1e-7,
            "frame {}: got {got}, want {want}",
            p0 + i as u64
        );
    }
}

#[test]
fn missing_tags_play_unadjusted() {
    let mut h = Harness::new(&["track1_44100.tst"]);
    h.engine.set_limiter_enabled(false);
    h.engine.set_replaygain_mode(ReplayGainMode::Track);
    // A preamp without a tag must not invent a gain.
    h.engine.set_replaygain_preamp_db(6.0);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    let (p0, left) = record_left(&mut h, 1024);
    assert_eq!(
        &left[..],
        &expected(1, p0, 1024)[..],
        "no tag means bit-exact passthrough"
    );
}

#[test]
fn volume_balance_and_replaygain_combine_in_documented_order() {
    let mut h = Harness::new(&["rg_-6.02_none_1_44100.tst"]);
    h.engine.set_limiter_enabled(false);
    h.engine.set_replaygain_mode(ReplayGainMode::Track);
    h.engine.set_volume(128).unwrap();
    h.engine.set_balance(-127).unwrap();
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    // Documented order: ReplayGain scales on the decode side (before every
    // stage), volume/balance scale on the render side. The left channel sees
    // both, the right channel is muted by the full-left balance.
    let replay = 10f32.powf(-6.02 / 20.0);
    let volume = 128.0f32 / 255.0;
    let p0 = h
        .wait_refilled(4096)
        .expect("the ring must fill at the start of the run");
    let mut rec = Vec::new();
    h.clock.pump_record(1024, &mut rec);
    h.engine.poll().unwrap();
    assert_eq!(rec.len(), 2048);
    for (i, frame) in rec.as_chunks::<2>().0.iter().enumerate() {
        // ReplayGain runs before the converter, the volume after it: the
        // expectation is the gained source, finished, times the volume.
        let want = finished(pattern(1, p0 + i as u64) * replay) * volume;
        assert!(
            (frame[0] - want).abs() < 1e-7,
            "left frame {}: got {}, want {want}",
            p0 + i as u64,
            frame[0]
        );
        assert_eq!(frame[1], 0.0, "full-left balance mutes the right channel");
    }
}

#[test]
fn album_mode_uses_album_gain_and_falls_back_to_track() {
    // Both tags present: each mode reads its own.
    for (mode, gain_db) in [(ReplayGainMode::Track, -2.0), (ReplayGainMode::Album, -5.0)] {
        let mut h = Harness::new(&["rg_-2.00_-5.00_1_44100.tst"]);
        h.engine.set_limiter_enabled(false);
        h.engine.set_replaygain_mode(mode);
        h.engine.play_index(0).unwrap();
        assert!(h.wait_track(0), "mode {mode:?}");

        let factor = 10f32.powf(gain_db / 20.0);
        let (p0, left) = record_left(&mut h, 1024);
        for (i, got) in left.iter().enumerate() {
            let want = finished(pattern(1, p0 + i as u64) * factor);
            assert!(
                (got - want).abs() < 1e-7,
                "mode {mode:?} frame {}: got {got}, want {want}",
                p0 + i as u64
            );
        }
    }

    // Album mode with only a track tag falls back to it.
    let mut h = Harness::new(&["rg_-6.02_none_1_44100.tst"]);
    h.engine.set_limiter_enabled(false);
    h.engine.set_replaygain_mode(ReplayGainMode::Album);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    let factor = 10f32.powf(-6.02 / 20.0);
    let (p0, left) = record_left(&mut h, 1024);
    for (i, got) in left.iter().enumerate() {
        let want = finished(pattern(1, p0 + i as u64) * factor);
        assert!(
            (got - want).abs() < 1e-7,
            "fallback frame {}: got {got}, want {want}",
            p0 + i as u64
        );
    }
}

#[test]
fn hot_replaygain_is_held_at_the_limiter_ceiling() {
    // +6.02 dB doubles the samples: the pattern peaks at 1.1, so peak times
    // gain is 2.2 — far above full scale. The policy is no pre-attenuation:
    // the limiter holds the boosted signal at its ceiling.
    let ceiling = {
        let h = Harness::new(&["track1_3000.tst"]);
        h.engine.limiter_ceiling()
    };

    let mut h = Harness::new(&["rg_+6.02_none_1_44100.tst"]);
    h.engine.set_replaygain_mode(ReplayGainMode::Track);
    assert!(
        h.engine.limiter_enabled(),
        "the limiter is on by default; this test needs it on"
    );
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    let (_, left) = record_left(&mut h, 4096);
    let peak = left.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        peak <= ceiling,
        "a +6.02 dB correction must not pass the ceiling {ceiling}: {peak}"
    );

    // Control: the same track with the limiter off is the doubled pattern,
    // held at full scale by the converter's ceiling — the ceiling above came
    // from the limiter, not from a quiet source or a hidden pre-attenuation.
    h.engine.stop().unwrap();
    h.engine.set_limiter_enabled(false);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    assert!(h.settle_drain());
    let factor = 10f32.powf(6.02 / 20.0);
    let (p0, left) = record_left(&mut h, 1024);
    let peak = left.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        peak > ceiling,
        "unlimited, the boosted signal must clear the ceiling: {peak}"
    );
    for (i, got) in left.iter().enumerate() {
        // The boosted source, finished: the converter's ceiling holds
        // everything above full scale at exactly 1.0.
        let want = finished(pattern(1, p0 + i as u64) * factor);
        assert!(
            (got - want).abs() < 1e-7,
            "unlimited frame {}: got {got}, want {want}",
            p0 + i as u64
        );
    }
}

#[test]
fn hot_replaygain_is_held_at_full_scale_after_resampling() {
    // Chain level, limiter off: a +12 dB correction on a 0.5 sine peaks at
    // ~2.0 before the converter, and the converter's final ceiling holds it
    // at exactly 1.0 after resampling — the same guarantee A14 pins for the
    // equalizer, now for a ReplayGain boost.
    let mut extra = BTreeMap::new();
    extra.insert("REPLAYGAIN_TRACK_GAIN".to_owned(), "+12.00 dB".to_owned());
    let gain = effective_gain(&extra, ReplayGainMode::Track, 0.0);
    assert!((gain - 3.981).abs() < 1e-3, "+12 dB is x4: {gain}");

    let input: Vec<f32> = (0..44_100)
        .map(|n| (0.5 * (2.0 * std::f64::consts::PI * 1_000.0 * n as f64 / 44_100.0).sin()) as f32)
        .collect();
    let mut boosted = input.clone();
    apply_gain(&mut boosted, gain);
    let pre = boosted.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(pre > 1.5, "the correction must overshoot first: {pre}");

    let mut converter = FormatConverter::new(AudioSpec::new(44_100, 1), AudioSpec::new(48_000, 1));
    let mut out = Vec::new();
    let mut buf = vec![0.0; converter.max_out_frames(1024)];
    for chunk in boosted.chunks(1024) {
        let n = converter.convert(chunk, &mut buf);
        out.extend_from_slice(&buf[..n]);
    }
    let mut tail = vec![0.0; converter.max_out_frames(2 * converter.latency_frames())];
    let n = converter.flush(&mut tail);
    out.extend_from_slice(&tail[..n]);
    assert_eq!(out.len(), 48_000);

    let post = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(post <= 1.0, "above 0 dBFS after resampling: {post}");
    assert!(
        (post - 1.0).abs() < 1e-6,
        "the safety net must have engaged: {post}"
    );
}

#[test]
fn replaygain_is_disabled_by_default() {
    let h = Harness::new(&["rg_-6.02_none_1_44100.tst"]);
    assert_eq!(
        h.engine.replaygain_mode(),
        ReplayGainMode::Off,
        "loudness never changes until the user asks"
    );
    assert_eq!(h.engine.replaygain_preamp_db(), 0.0);
}

#[test]
fn a_resampled_device_makes_no_difference_to_the_documented_order() {
    // Equal rates take the bit-exact path while resampling rebuilds peaks
    // (A14); the ReplayGain scale sits before both, so the documented order
    // holds either way. This run just pins that a 48 kHz device plays the
    // corrected track end to end with the limiter on and the peak bounded.
    let cfg = EngineConfig {
        spec: AudioSpec::new(48_000, 2),
        ..EngineConfig::default()
    };
    let mut h = Harness::with_config(&["rg_-6.02_none_1_44100.tst"], cfg);
    assert_eq!(h.engine.device_spec(), AudioSpec::new(48_000, 2));
    h.engine.set_replaygain_mode(ReplayGainMode::Track);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    let ceiling = h.engine.limiter_ceiling();
    let (_, left) = record_left(&mut h, 4096);
    let peak = left.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        peak <= 1.0,
        "nothing above full scale may reach the device: {peak}"
    );
    assert!(
        peak <= ceiling + 0.05,
        "a −6.02 dB cut stays near the limiter ceiling {ceiling}: {peak}"
    );
}
