//! The output fade: what the engine does with `Command::StopWithFadeout`.
//!
//! The ramp law itself is unit-tested in `rustamp-dsp`; this file pins the
//! engine's side of it:
//!
//! * the ramp goes into the **output** (the render callback, the consumer
//!   side of the ring), so it covers exactly the frames the device plays;
//! * its shape is the equal-power law and it lasts exactly the configured
//!   duration;
//! * the transport stops when the ramp is over, and not before;
//! * a plain `Stop` and a `Pause` stay immediate, and the run after a
//!   fade-out is at unity again.

mod common;

use common::{Harness, SPEC, pattern};
use rustamp_audio::EngineConfig;
use rustamp_core::command::Command;
use rustamp_core::state::TransportState;

/// The fade duration the tests use: short enough to keep the test quick, long
/// enough to see the ramp.
const FADE_MS: u32 = 50;

/// The frames the fade covers at the test's sample rate.
fn fade_frames() -> u64 {
    u64::from(FADE_MS) * u64::from(SPEC.sample_rate) / 1000
}

/// The frames the test pumps before it asks for the fade.
const PRELUDE: u64 = 4096;

/// The ring capacity in frames ([`EngineConfig::ring_frames`]'s default). The
/// tests wait for the ring to be full before they pump, so that the decode
/// thread cannot fall behind and the device cannot run dry: every pumped
/// frame is then a frame of the track, in order.
const RING_FRAMES: u64 = 16384;

/// A harness with one long track and a short fade.
fn harness() -> Harness {
    let cfg = EngineConfig {
        fade_ms: FADE_MS,
        ..EngineConfig::default()
    };
    Harness::with_config(&["track1_44100.tst"], cfg)
}

/// Start playback and wait until the ring holds the whole track's first
/// stretch, so that the first frame the device pulls is the track's frame 0.
fn start(h: &mut Harness) {
    h.engine.set_limiter_enabled(false);
    h.engine.play_index(0).expect("play");
    assert!(
        h.wait_refilled(RING_FRAMES).is_some(),
        "the ring must be full"
    );
}

#[test]
fn the_fade_out_ramps_the_output_down_and_stops_the_transport() {
    let mut h = harness();
    start(&mut h);

    // Before the fade the output is the source, bit for bit.
    let mut before = Vec::new();
    h.clock.pump_record(PRELUDE, &mut before);
    for (i, frame) in before.as_chunks::<2>().0.iter().enumerate() {
        assert_eq!(
            frame[0],
            pattern(1, i as u64),
            "frame {i} must be the source"
        );
    }

    // The fade-out: the transport keeps playing while the ramp runs.
    h.engine.apply(&Command::StopWithFadeout).expect("fade out");
    assert_eq!(
        h.engine.transport(),
        TransportState::Playing,
        "the transport stops when the ramp is over, not before"
    );

    let mut during = Vec::new();
    h.clock.pump_record(fade_frames(), &mut during);

    // The ramp is the equal-power law over the requested duration: the gain
    // falls from unity to silence, frame by frame.
    let span = fade_frames();
    for j in 0..span as usize {
        let t = j as f64 / (span - 1) as f64;
        let expected = (1.0 - t).sqrt() as f32;
        let source = pattern(1, PRELUDE + j as u64);
        let actual = during[j * 2] / source;
        assert!(
            (actual - expected).abs() < 1e-5,
            "frame {j}: the gain is {actual}, expected {expected}"
        );
    }

    // The ramp is over: the transport stops.
    h.engine.poll().expect("poll");
    assert_eq!(h.engine.transport(), TransportState::Stopped);
}

#[test]
fn a_plain_stop_is_immediate() {
    let mut h = harness();
    start(&mut h);
    h.engine.apply(&Command::Stop).expect("stop");
    assert_eq!(h.engine.transport(), TransportState::Stopped);
    assert_eq!(h.clock.pump(1024), 0, "a stopped device pulls nothing");
}

#[test]
fn a_pause_is_immediate_and_does_not_fade() {
    let mut h = harness();
    start(&mut h);
    h.engine.apply(&Command::Pause).expect("pause");
    assert_eq!(h.engine.transport(), TransportState::Paused);
    assert_eq!(h.clock.pump(1024), 0, "a paused device pulls nothing");

    // The pause did not attenuate anything: after the resume the output is
    // the source again.
    h.engine.apply(&Command::Play).expect("resume");
    let mut after = Vec::new();
    h.clock.pump_record(1024, &mut after);
    for (i, frame) in after.as_chunks::<2>().0.iter().enumerate() {
        assert_eq!(
            frame[0],
            pattern(1, i as u64),
            "frame {i} must be the source"
        );
    }
}

#[test]
fn the_run_after_a_fade_out_is_at_unity() {
    let mut h = harness();
    start(&mut h);
    h.engine.apply(&Command::StopWithFadeout).expect("fade out");
    h.clock.pump_record(fade_frames(), &mut Vec::new());
    h.engine.poll().expect("poll");
    assert_eq!(h.engine.transport(), TransportState::Stopped);

    // The next run is not silent: the fade that latched the gain at zero is
    // reset when the run starts.
    h.engine.play_index(0).expect("play");
    // The stopped run left its audio in the ring, so the new run starts with
    // the drain that drops it.
    assert!(h.settle_drain(), "the start drain must complete");
    assert!(h.wait_refilled(RING_FRAMES).is_some());
    let mut again = Vec::new();
    h.clock.pump_record(4096, &mut again);
    // The source is a pseudo-random pattern in 0.1..=1.1 with a mean of 0.6;
    // at unity the output has the same mean, at the latched silence it has
    // none.
    let mean = again.iter().map(|s| s.abs()).sum::<f32>() / again.len() as f32;
    assert!(
        (0.55..0.65).contains(&mean),
        "the new run must be at unity, but its mean level is {mean}"
    );
}

#[test]
fn a_play_during_the_fade_cancels_it() {
    let mut h = harness();
    start(&mut h);
    // Play the first stretch, so that the frames the device pulls are the
    // track's frames in order.
    h.clock.pump_record(PRELUDE, &mut Vec::new());

    h.engine.apply(&Command::StopWithFadeout).expect("fade out");
    // The first half of the ramp.
    h.clock.pump_record(fade_frames() / 2, &mut Vec::new());

    // A play cancels the fade-out: the transport keeps playing and the gain
    // ramps back up.
    h.engine.apply(&Command::Play).expect("play");
    assert_eq!(h.engine.transport(), TransportState::Playing);

    let mut back = Vec::new();
    h.clock.pump_record(fade_frames(), &mut back);
    h.engine.poll().expect("poll");
    assert_eq!(
        h.engine.transport(),
        TransportState::Playing,
        "the cancelled fade must not stop the transport"
    );

    // The last frame of the fade-in is at unity again.
    let last = back.len() / 2 - 1;
    let source = pattern(1, PRELUDE + fade_frames() / 2 + last as u64);
    assert_eq!(back[last * 2], source, "the gain is back at unity");
}
