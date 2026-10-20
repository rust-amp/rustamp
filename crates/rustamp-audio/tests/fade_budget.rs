//! The fade against the buffer it is realised in: S18 §2.3's constraint.
//!
//! What this file pins:
//!
//! * a fade that **fits** the buffer is unchanged and produces no report;
//! * a fade that **does not fit** produces the report with **both numbers** —
//!   the requested milliseconds and the available milliseconds — and is
//!   **kept in full**: the ramp still lasts exactly the requested time;
//! * a **zero** fade is "no fade": it fits every buffer, reports nothing and
//!   stops the transport at once;
//! * the numbers are computed at the rate the device granted, so they are
//!   correct at 44.1 kHz and at 48 kHz;
//! * a fade that is longer than the engine's ring is realised in full: the
//!   ring is refilled while the ramp runs;
//! * the limit the rule accepts: a fade that outlasts the audio ends above
//!   zero - the music is cut, not faded out.

mod common;

use std::time::Duration;

use common::{Harness, finished, pattern};
use rustamp_audio::{EngineConfig, FadeBuffer};
use rustamp_core::audio::AudioSpec;
use rustamp_core::command::Command;
use rustamp_core::state::TransportState;
use rustamp_output::DEFAULT_CHUNK_FRAMES;

/// The frames the null device pulls per callback: its buffer.
const DEVICE_FRAMES: u64 = DEFAULT_CHUNK_FRAMES;

/// The rate the null device grants unless the engine asks for another one.
const RATE: u32 = 44_100;

/// A fade that fits the device's buffer: 10 ms is 441 frames at 44.1 kHz.
const FITS_MS: u32 = 10;

/// A fade that does not fit it: 500 ms is 22050 frames at 44.1 kHz.
const DOES_NOT_FIT_MS: u32 = 500;

/// A ring big enough to hold a whole fade, so that the ramp never runs on an
/// empty buffer.
const BIG_RING: usize = 200_000;

/// The frames the fade pumps before it is asked for.
const PRELUDE: u64 = 4096;

/// The frames a fade of `ms` covers at `rate`.
fn fade_frames(ms: u32, rate: u32) -> u64 {
    u64::from(ms) * u64::from(rate) / 1000
}

/// An engine with one long track and the given fade and ring.
fn harness(fade_ms: u32, ring_frames: usize) -> Harness {
    harness_with_track("track1_441000.tst", fade_ms, ring_frames)
}

/// An engine with the given track, fade and ring.
fn harness_with_track(track: &str, fade_ms: u32, ring_frames: usize) -> Harness {
    let cfg = EngineConfig {
        fade_ms,
        ring_frames,
        ..EngineConfig::default()
    };
    Harness::with_config(&[track], cfg)
}

/// Start playback and wait until the ring holds the frames the fade will
/// need, so that the first frame the device pulls is the track's frame 0.
fn start(h: &mut Harness, buffered: u64) {
    h.engine.set_limiter_enabled(false);
    h.engine.play_index(0).expect("play");
    assert!(
        h.wait_refilled(buffered).is_some(),
        "the ring must hold {buffered} frames"
    );
}

/// Pump `frames` frames, so that the device is at the track's frame
/// `PRELUDE` when the fade is asked for.
fn pump_prelude(h: &Harness) {
    h.clock.pump_record(PRELUDE, &mut Vec::new());
}

/// The gain the ramp applies at the frame `j` of a ramp over `span` frames.
fn ramp_gain(j: u64, span: u64) -> f32 {
    let t = j as f64 / (span - 1) as f64;
    (1.0 - t).sqrt() as f32
}

/// The gain the output carries at the frame `j` of the ramp: the output
/// divided by the source the ramp acts on.
fn measured_gain(during: &[f32], j: u64) -> f32 {
    let source = finished(pattern(1, PRELUDE + j));
    during[j as usize * 2] / source
}

#[test]
fn a_fade_that_fits_the_buffer_produces_no_report() {
    let mut h = harness(FITS_MS, BIG_RING);
    start(&mut h, fade_frames(FITS_MS, RATE) + PRELUDE);
    pump_prelude(&h);

    h.engine.apply(&Command::StopWithFadeout).expect("fade out");
    assert_eq!(
        h.engine.pending_fade_notes(),
        0,
        "a fade that fits must report nothing"
    );
    assert!(h.engine.take_fade_note().is_none());

    // The fade itself is unchanged: the ramp covers exactly the requested
    // frames.
    let span = fade_frames(FITS_MS, RATE);
    let mut during = Vec::new();
    h.clock.pump_record(span, &mut during);
    for j in 0..span {
        let actual = measured_gain(&during, j);
        let expected = ramp_gain(j, span);
        assert!(
            (actual - expected).abs() < 1e-5,
            "frame {j}: the gain is {actual}, expected {expected}"
        );
    }
}

#[test]
fn a_fade_that_does_not_fit_produces_the_report_with_both_numbers() {
    let mut h = harness(DOES_NOT_FIT_MS, BIG_RING);
    start(&mut h, fade_frames(DOES_NOT_FIT_MS, RATE) + PRELUDE);
    pump_prelude(&h);

    h.engine.apply(&Command::StopWithFadeout).expect("fade out");

    let note = h.engine.take_fade_note().expect("a report");
    assert_eq!(note.requested_ms, DOES_NOT_FIT_MS);
    // The device's own buffer is the shorter one: 1024 frames are 23 ms at
    // 44.1 kHz.
    assert_eq!(note.buffer, FadeBuffer::Device);
    assert_eq!(note.available_ms, 23);
    assert_eq!(note.needed_ms, u64::from(DOES_NOT_FIT_MS));
    assert_eq!(
        h.engine.pending_fade_notes(),
        0,
        "the report must be taken once"
    );

    // The report reaches the user as a text with the numbers in it.
    let message = note.message();
    assert!(message.contains("500 ms"), "{message}");
    assert!(message.contains("23 ms"), "{message}");
    assert!(message.contains("device's buffer"), "{message}");
}

#[test]
fn a_fade_that_does_not_fit_is_kept_in_full() {
    let mut h = harness(DOES_NOT_FIT_MS, BIG_RING);
    start(&mut h, fade_frames(DOES_NOT_FIT_MS, RATE) + PRELUDE);
    pump_prelude(&h);

    h.engine.apply(&Command::StopWithFadeout).expect("fade out");
    assert!(h.engine.take_fade_note().is_some(), "a report");

    // The ramp is not shortened: it covers exactly the requested frames and
    // reaches silence at the last one.
    let span = fade_frames(DOES_NOT_FIT_MS, RATE);
    let mut during = Vec::new();
    h.clock.pump_record(span, &mut during);
    for j in 0..span {
        let actual = measured_gain(&during, j);
        let expected = ramp_gain(j, span);
        assert!(
            (actual - expected).abs() < 1e-5,
            "frame {j}: the gain is {actual}, expected {expected}"
        );
    }
    h.engine.poll().expect("poll");
    assert_eq!(
        h.engine.transport(),
        TransportState::Stopped,
        "the transport stops when the ramp is over"
    );
}

#[test]
fn the_engine_buffer_binds_when_it_is_the_shorter_one() {
    // A ring of 512 frames is 11 ms at 44.1 kHz - shorter than the device's
    // buffer of 1024 frames.
    let mut h = harness(50, 512);
    start(&mut h, 512);
    pump_prelude(&h);

    h.engine.apply(&Command::StopWithFadeout).expect("fade out");

    let note = h.engine.take_fade_note().expect("a report");
    assert_eq!(note.buffer, FadeBuffer::Ring);
    assert_eq!(note.requested_ms, 50);
    assert_eq!(note.available_ms, 11);
    assert_eq!(note.needed_ms, 50);
}

#[test]
fn a_fade_that_outlasts_the_audio_ends_above_zero() {
    // The track is one second, the fade asks for two. The ramp runs past the
    // end of the audio, so the music is cut at the gain the ramp had reached
    // - not at silence. That is the limit the rule accepts, and the report
    // says so.
    const LONG_MS: u32 = 2000;
    let mut h = harness_with_track("track1_44100.tst", LONG_MS, BIG_RING);
    // The whole track is 44100 frames; the ring holds it, so the ramp never
    // runs on an empty buffer.
    start(&mut h, 44_100);
    pump_prelude(&h);

    h.engine.apply(&Command::StopWithFadeout).expect("fade out");
    assert!(h.engine.take_fade_note().is_some(), "a report");

    let span = fade_frames(LONG_MS, RATE);
    let mut during = Vec::new();
    h.clock.pump_record(span, &mut during);
    // The last frame that carries audio is the track's last one; after it the
    // ramp runs on silence.
    let last = during
        .chunks(2)
        .rposition(|frame| frame[0].abs() > 1e-6)
        .expect("the track must be audible");
    let expected = ramp_gain(last as u64, span);
    assert!(
        expected > 0.1,
        "the ramp must still be above zero where the audio ends, got {expected}"
    );
    assert!(
        expected < 0.9,
        "the ramp must not have reached silence before the audio ended, got {expected}"
    );
}

#[test]
fn a_zero_fade_is_no_fade_and_reports_nothing() {
    let mut h = harness(0, BIG_RING);
    start(&mut h, 16_384);
    pump_prelude(&h);

    h.engine.apply(&Command::StopWithFadeout).expect("fade out");

    // A duration of 0 ms means "no fade": the transport stops at once.
    assert_eq!(h.engine.transport(), TransportState::Stopped);
    assert_eq!(
        h.engine.pending_fade_notes(),
        0,
        "a fade of 0 ms fits every buffer and reports nothing"
    );
}

#[test]
fn the_numbers_are_rate_correct_at_48_kilohertz() {
    let cfg = EngineConfig {
        spec: AudioSpec::new(48_000, 2),
        fade_ms: DOES_NOT_FIT_MS,
        ring_frames: BIG_RING,
        ..EngineConfig::default()
    };
    let mut h = Harness::with_config(&["track1_441000.tst"], cfg);
    start(&mut h, fade_frames(DOES_NOT_FIT_MS, 48_000) + PRELUDE);
    pump_prelude(&h);

    h.engine.apply(&Command::StopWithFadeout).expect("fade out");

    let note = h.engine.take_fade_note().expect("a report");
    assert_eq!(note.requested_ms, DOES_NOT_FIT_MS);
    // The same 1024 frames are 21 ms at 48 kHz (21.33 ms).
    assert_eq!(note.available_ms, 21);
    // 500 ms is 24000 frames at 48 kHz, and 24000 frames are exactly 500 ms.
    assert_eq!(note.needed_ms, u64::from(DOES_NOT_FIT_MS));
}

/// Pump `frames` frames in device-sized chunks, paced like a real device, so
/// that the decode thread has the wall-clock time to refill the ring while
/// the ramp runs.
fn pump_paced(h: &Harness, frames: u64, out: &mut Vec<f32>) {
    let mut left = frames;
    while left > 0 {
        let n = left.min(DEVICE_FRAMES);
        h.clock.pump_record(n, out);
        left -= n;
        std::thread::sleep(Duration::from_micros(n * 1_000_000 / u64::from(RATE)));
    }
}

#[test]
fn a_fade_longer_than_the_engine_ring_is_realised_in_full() {
    // The ring holds 371 ms; the fade asks for 500 ms. The ramp has to
    // cross the ring's boundary, and the ring has to be refilled while it
    // runs.
    const LONG_MS: u32 = 500;
    let mut h = harness(LONG_MS, 16_384);
    start(&mut h, 16_384);
    pump_paced(&h, PRELUDE, &mut Vec::new());

    h.engine.apply(&Command::StopWithFadeout).expect("fade out");
    let note = h.engine.take_fade_note().expect("a report");
    assert_eq!(note.requested_ms, LONG_MS);

    let span = fade_frames(LONG_MS, RATE);
    let mut during = Vec::new();
    pump_paced(&h, span, &mut during);
    assert_eq!(during.len() / 2, span as usize);

    // The ramp reaches silence at exactly the requested frame: it was not
    // shortened to the buffer's length.
    let last = during[(span as usize - 1) * 2].abs();
    assert!(last < 1e-3, "the ramp must end at silence, got {last}");
    // The ring was refilled while the ramp ran: the ramp still had audio to
    // fade in its last tenth, which lies beyond the ring's first 371 ms.
    let late = span * 9 / 10;
    let actual = measured_gain(&during, late);
    let expected = ramp_gain(late, span);
    assert!(
        (actual - expected).abs() < 1e-3,
        "the ring must be refilled while the ramp runs: the gain is {actual}, expected {expected}"
    );
}
