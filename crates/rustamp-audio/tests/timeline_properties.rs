//! Property tests for the engine's time and position math.
//!
//! The engine's clock is the *device* clock: the position comes from
//! [`OutputDevice::played_frames`], never from the decode side. The engine
//! keeps a pair of origins and reports
//!
//! ```text
//! timeline     = timeline_origin + (played_frames - clock_origin)
//! position     = min(timeline - track_start, duration_frames)
//! position_ms  = position * 1000 / rate
//! ```
//!
//! Every re-base of that pair - the start of a run, a seek, the end of a run -
//! is a place where the arithmetic can go wrong, and it did once: `finish_run`
//! re-based the device clock but left `timeline_origin` at its old value, so
//! the moment a run ended `timeline_frames()` collapsed to zero. The position
//! read 0 and `buffered_frames()` reported the whole stream as unplayed.
//!
//! These tests drive the engine over a virtual device
//! ([`rustamp_output::NullOutput`]), so time is the test's to spend, and check
//! the invariants that must hold for every operation sequence:
//!
//! * **M1 - monotone**: while the transport plays, the timeline never goes
//!   backwards. Only an explicit seek (and a stop, which resets the position)
//!   may move it.
//! * **F1 - frozen**: a paused and a stopped transport do not move the clock,
//!   however much time passes.
//! * **F2 - frozen at the end**: a run that ends freezes the timeline at the
//!   *content* end - the last frame the decoder produced - not at the device
//!   position and not at zero.
//! * **B1 - bounded**: the position never leaves the track it is in, not even
//!   when the device plays past the end of the content.
//! * **S1 - clamped**: a seek lands inside `[0, duration]`, and the position
//!   follows the target.
//! * **C1 - consistent**: `position_ms` is the frame count converted to
//!   milliseconds, and the published state carries the same number.
//! * **R1 - re-based**: a second run starts at zero: the timeline origin is
//!   re-based, so the new run does not continue the old one.
//!
//! # Two positions, two meanings
//!
//! The engine reports two positions and they do not mean the same thing:
//!
//! * `timeline_frames()` is the **absolute** position of the run, in device
//!   frames since the run started. It only ever moves forward while the
//!   transport plays, across a gapless seam included.
//! * `position_frames()` is the position **inside the current track**. The
//!   current track is the one the decode thread has opened, which runs ahead
//!   of the audible one, so the per-track position restarts at a track
//!   change. It is the number the window shows.
//!
//! The properties below state each invariant on the quantity it belongs to.
//!
//! The scenarios are driven by the seeded [`Rng`] of `rustamp-playlist`, a
//! dependency this crate already has: no new dependency is needed, and every
//! failing case is reproducible from its seed alone.

mod common;

use std::time::{Duration, Instant};

use common::{Harness, SPEC, WAIT};
use rustamp_audio::Engine;
use rustamp_core::state::TransportState;
use rustamp_playlist::Rng;

/// The seed of every scenario in this file.
const SEED: u64 = 0xA0D1_0F7E_5EED_0001;

/// The number of scenarios per property.
const ROUNDS: usize = 24;

/// The number of operations per scenario.
const STEPS: usize = 14;

/// The pump granularity of a scenario, in device frames.
///
/// A scenario advances the clock in multiples of this, so every position the
/// engine reports between two re-bases is congruent to the position at the
/// last re-base modulo `STEP`. A seek target that is *not* congruent to the
/// current position modulo `STEP` can therefore not be mistaken for a
/// position that merely passed by on its way to the re-base.
const STEP: u64 = 256;

/// The length of the track a scenario plays.
///
/// The track is longer than the ring (16384 frames) and longer than any
/// scenario can play, so the decoder cannot produce all of it and the run
/// cannot end while the scenario runs. The end of a run is a property of its
/// own.
const TRACK_FRAMES: u64 = 60_000;

/// The length of the track of the scenarios that need a run that can end.
///
/// It fits into the ring, so the decoder produces all of it and the engine can
/// finish the run.
const SHORT_TRACK_FRAMES: u64 = 2_000;

/// The clock state the engine is expected to be in.
///
/// The model is exact: the engine's timeline is the number of content frames
/// it has played since the last re-base, and its position is that timeline
/// clamped to the length of the current track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Clock {
    /// The absolute timeline, in device frames.
    timeline: u64,
    /// The position inside the current track, in device frames.
    position: u64,
    /// The length of the current track, in device frames.
    duration: u64,
}

impl Clock {
    /// The clock of a run that has just started at zero.
    fn new(duration: u64) -> Clock {
        Clock {
            timeline: 0,
            position: 0,
            duration,
        }
    }

    /// The clock after the device pulled `pulled` frames.
    fn advanced(&self, pulled: u64) -> Clock {
        let timeline = self.timeline + pulled;
        Clock {
            timeline,
            position: timeline.min(self.duration),
            duration: self.duration,
        }
    }

    /// The clock after a seek to `target`.
    ///
    /// The engine clamps the target to the length of the track and re-bases
    /// its origins there, so the frames the device played while the seek was
    /// on its way are discarded.
    fn seeked(&self, target: u64) -> Clock {
        let timeline = target.min(self.duration);
        Clock {
            timeline,
            position: timeline,
            duration: self.duration,
        }
    }

    /// The clock after a stop.
    ///
    /// A stop resets the position and re-bases the origins, so the run that
    /// follows starts from zero again.
    fn stopped(&self) -> Clock {
        Clock {
            timeline: 0,
            position: 0,
            duration: self.duration,
        }
    }
}

/// The operations a scenario applies to the engine.
#[derive(Debug, Clone, Copy)]
enum Op {
    /// Advance the device clock.
    Advance,
    /// Pause the transport.
    Pause,
    /// Resume the transport.
    Resume,
    /// Seek inside the current track.
    Seek,
    /// Stop the transport.
    Stop,
    /// Start the transport again.
    Restart,
}

/// A random operation list.
///
/// The seek target is drawn by the runner, not here: it depends on the
/// position the engine is in when the seek is issued.
fn random_ops(rng: &mut Rng, count: usize) -> Vec<Op> {
    (0..count)
        .map(|_| match rng.below(12) {
            0..=5 => Op::Advance,
            6 => Op::Pause,
            7 => Op::Resume,
            8 => Op::Seek,
            9 => Op::Stop,
            _ => Op::Restart,
        })
        .collect()
}

/// Poll the engine until `cond` holds, advancing the clock by [`STEP`] frames
/// per round. Returns whether the condition held.
///
/// The bound is the harness' stall escape: generous enough that only a real
/// stall reaches it.
fn pump_until(h: &mut Harness, mut cond: impl FnMut(&Engine) -> bool) -> bool {
    let deadline = Instant::now() + WAIT;
    loop {
        h.engine.poll().expect("poll");
        if cond(&h.engine) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        h.clock.pump(STEP);
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Assert that the engine reports the clock the model expects.
fn assert_clock(h: &Harness, clock: &Clock, what: &str) {
    assert_eq!(
        h.engine.timeline_frames(),
        clock.timeline,
        "{what}: the timeline"
    );
    assert_eq!(
        h.engine.position_frames(),
        clock.position,
        "{what}: the position"
    );
}

/// A seek target that the positions between two re-bases cannot hit.
///
/// The positions the engine reports before the seek is applied are all
/// congruent to the current position modulo [`STEP`], so a target with a
/// different residue can only be the result of the re-base.
fn seek_target(rng: &mut Rng, position: u64, duration: u64) -> Option<u64> {
    let residue = (position % STEP + 1) % STEP;
    // Keep a margin of one step below the end: a target at the very end would
    // also be the position the clamp parks at.
    let room = duration.checked_sub(STEP + residue)?;
    let count = (room / STEP + 1) as usize;
    Some(residue + rng.below(count) as u64 * STEP)
}

/// Run one random scenario and check the invariants after every operation.
fn run_scenario(rng: &mut Rng, round: usize) {
    let name = format!("track1_{TRACK_FRAMES}.tst");
    let mut h = Harness::new(&[name.as_str()]);
    h.engine.play_index(0).expect("play");
    assert!(h.wait_track(0), "round {round}: the track must open");
    let duration = h.engine.duration_frames().expect("the track's length");
    assert_eq!(duration, TRACK_FRAMES, "round {round}: the track's length");

    let mut clock = Clock::new(duration);
    assert_clock(&h, &clock, &format!("round {round}: the start of the run"));

    let ops = random_ops(rng, STEPS);
    for (step, op) in ops.iter().copied().enumerate() {
        let what = format!("round {round}, step {step}, op {op:?}");
        match op {
            Op::Advance => {
                let frames = rng.below(8) as u64 * STEP;
                let pulled = h.clock.pump(frames);
                h.engine.poll().expect("poll");
                clock = clock.advanced(pulled);
            }
            Op::Pause => {
                h.engine.pause().expect("pause");
                let pulled = h.clock.pump(STEP);
                assert_eq!(pulled, 0, "{what}: a paused device pulls nothing");
                h.engine.poll().expect("poll");
            }
            Op::Resume | Op::Restart => {
                let stopped = h.engine.transport() == TransportState::Stopped;
                h.engine.play().expect("play");
                if stopped {
                    // The new run starts with a drain of the ring the previous
                    // run left behind; the clock is re-based when the drain is
                    // done, so the run starts at zero.
                    assert!(
                        pump_until(&mut h, |e| !e.is_start_pending()),
                        "{what}: the start drain must complete"
                    );
                    clock = clock.stopped();
                }
                h.engine.poll().expect("poll");
            }
            Op::Seek => {
                if h.engine.transport() == TransportState::Playing
                    && let Some(target) = seek_target(rng, clock.position, duration)
                {
                    h.engine.seek_frames(target).expect("seek");
                    assert!(
                        pump_until(&mut h, |e| e.position_frames() == target),
                        "{what}: the seek must land on {target}, not on {}",
                        h.engine.position_frames()
                    );
                    clock = clock.seeked(target);
                }
            }
            Op::Stop => {
                h.engine.stop().expect("stop");
                let pulled = h.clock.pump(STEP);
                assert_eq!(pulled, 0, "{what}: a stopped device pulls nothing");
                h.engine.poll().expect("poll");
                clock = clock.stopped();
            }
        }
        assert_clock(&h, &clock, &what);
    }
}

/// Every random operation sequence keeps the clock where the model expects it.
///
/// This is the whole suite in one test: M1, F1, S1, C1 and R1 at once, over a
/// seeded random script of advances, pauses, resumes, seeks, stops and
/// restarts.
#[test]
fn random_operation_sequences_keep_the_clock_where_it_belongs() {
    let mut rng = Rng::new(SEED);
    for round in 0..ROUNDS {
        run_scenario(&mut rng, round);
    }
}

/// M1: the timeline never goes backwards while the transport plays.
///
/// The timeline is the absolute position of the run, so it only ever moves
/// forward. The position *inside* the track follows it, clamped to the length
/// of the track.
#[test]
fn the_position_never_goes_backwards_while_playing() {
    let mut rng = Rng::new(SEED ^ 0x11);
    for round in 0..ROUNDS {
        let name = format!("track1_{TRACK_FRAMES}.tst");
        let mut h = Harness::new(&[name.as_str()]);
        h.engine.play_index(0).expect("play");
        assert!(h.wait_track(0), "round {round}: the track must open");

        let mut last_timeline = h.engine.timeline_frames();
        let mut last_position = h.engine.position_frames();
        for step in 0..STEPS {
            let pulled = h.clock.pump(rng.below(8) as u64 * STEP);
            h.engine.poll().expect("poll");
            let timeline = h.engine.timeline_frames();
            let position = h.engine.position_frames();
            assert!(
                timeline >= last_timeline,
                "round {round}, step {step}: the timeline went backwards ({last_timeline} -> {timeline})"
            );
            assert!(
                position >= last_position,
                "round {round}, step {step}: the position went backwards ({last_position} -> {position})"
            );
            assert_eq!(
                timeline - last_timeline,
                pulled,
                "round {round}, step {step}: the timeline must follow the device clock"
            );
            last_timeline = timeline;
            last_position = position;
        }
    }
}

/// M1: the timeline is continuous across a gapless seam.
///
/// The second track starts exactly where the first one ended: the absolute
/// timeline follows the device clock frame by frame across the join, and the
/// seam itself sits on the content boundary - the first frame of the second
/// track is the frame right after the last frame of the first one.
#[test]
fn the_timeline_is_continuous_across_a_gapless_seam() {
    let mut rng = Rng::new(SEED ^ 0x22);
    for round in 0..ROUNDS {
        // Short tracks, so the scenario can walk the seam frame by frame and
        // the whole queue fits into the ring at once.
        let first = 1_000u64 + rng.below(1_000) as u64;
        let second = 1_000u64 + rng.below(1_000) as u64;
        let names = [
            format!("track1_{first}.tst"),
            format!("track2_{second}.tst"),
        ];
        let mut h = Harness::new(&[names[0].as_str(), names[1].as_str()]);
        h.engine.play_index(0).expect("play");
        assert!(
            h.wait_buffered(),
            "round {round}: both tracks must be decoded"
        );

        let mut last_timeline = h.engine.timeline_frames();
        let mut last_audible = h.engine.audible_index();
        let mut seam = None;
        // The run ends after the whole queue has been played, and the engine
        // freezes the timeline at the content end.
        for _ in 0..(first + second + 2 * STEP) {
            h.clock.pump(1);
            h.engine.poll().expect("poll");
            let timeline = h.engine.timeline_frames();
            if h.engine.transport() != TransportState::Playing {
                // The run ended: the engine re-based the timeline onto the
                // content end. That is the one place where the timeline can
                // move backwards, and it lands on the content - never on
                // zero.
                assert_eq!(
                    timeline,
                    first + second,
                    "round {round}: the timeline must freeze at the content end"
                );
                break;
            }
            // The absolute timeline follows the device clock frame by frame -
            // across the seam included. This is the continuity claim.
            assert_eq!(
                timeline,
                last_timeline + 1,
                "round {round}: the timeline must follow the device clock"
            );
            // The per-track position never leaves the track it is in.
            if let Some(duration) = h.engine.duration_frames() {
                assert!(
                    h.engine.position_frames() <= duration,
                    "round {round}: the position left the track"
                );
            }
            // The seam sits exactly on the content boundary.
            let audible = h.engine.audible_index();
            if audible != last_audible {
                assert!(
                    seam.is_none(),
                    "round {round}: the seam must be crossed once"
                );
                seam = Some(timeline);
                assert_eq!(
                    timeline, first,
                    "round {round}: the seam must sit on the content boundary"
                );
            }
            last_audible = audible;
            last_timeline = timeline;
        }
        assert!(
            seam.is_some(),
            "round {round}: the seam must have been reached"
        );
        // The run ended: the timeline froze at the content end and the
        // position at the end of the last track.
        assert_eq!(
            h.engine.timeline_frames(),
            first + second,
            "round {round}: the frozen timeline"
        );
        assert_eq!(
            h.engine.position_frames(),
            second,
            "round {round}: the frozen position"
        );
    }
}

/// F1: a paused and a stopped transport do not move the clock.
#[test]
fn a_paused_and_a_stopped_transport_freeze_the_clock() {
    let mut rng = Rng::new(SEED ^ 0x33);
    for round in 0..ROUNDS {
        let name = format!("track1_{TRACK_FRAMES}.tst");
        let mut h = Harness::new(&[name.as_str()]);
        h.engine.play_index(0).expect("play");
        assert!(h.wait_track(0), "round {round}: the track must open");
        h.clock.pump(rng.below(4) as u64 * STEP);
        h.engine.poll().expect("poll");

        // Paused: the device pulls nothing and the clock stands still.
        h.engine.pause().expect("pause");
        let timeline = h.engine.timeline_frames();
        let position = h.engine.position_frames();
        for _ in 0..4 {
            assert_eq!(
                h.clock.pump(STEP),
                0,
                "round {round}: a paused device pulls nothing"
            );
            h.engine.poll().expect("poll");
            assert_eq!(
                h.engine.timeline_frames(),
                timeline,
                "round {round}: the timeline moved"
            );
            assert_eq!(
                h.engine.position_frames(),
                position,
                "round {round}: the position moved"
            );
        }

        // Resumed: the clock continues where it stood.
        h.engine.play().expect("resume");
        let pulled = h.clock.pump(STEP);
        h.engine.poll().expect("poll");
        assert_eq!(
            h.engine.timeline_frames(),
            timeline + pulled,
            "round {round}: the clock must continue where it stopped"
        );

        // Stopped: the position is reset and stays there.
        h.engine.stop().expect("stop");
        assert_eq!(
            h.engine.position_frames(),
            0,
            "round {round}: a stop resets the position"
        );
        assert_eq!(
            h.engine.timeline_frames(),
            0,
            "round {round}: a stop re-bases the timeline"
        );
        for _ in 0..4 {
            assert_eq!(
                h.clock.pump(STEP),
                0,
                "round {round}: a stopped device pulls nothing"
            );
            h.engine.poll().expect("poll");
            assert_eq!(
                h.engine.position_frames(),
                0,
                "round {round}: the position moved"
            );
            assert_eq!(
                h.engine.timeline_frames(),
                0,
                "round {round}: the timeline moved"
            );
        }
    }
}

/// F2: a run that ends freezes the timeline at the content end.
///
/// This is the regression: the run ended, the engine re-based the device
/// clock but not the timeline origin, and the timeline collapsed to zero.
#[test]
fn a_run_that_ends_freezes_the_timeline_at_the_content_end() {
    let mut rng = Rng::new(SEED ^ 0x44);
    for round in 0..ROUNDS {
        // A track that fits into the ring, so the decoder produces all of it
        // and the run can end.
        let frames = SHORT_TRACK_FRAMES + rng.below(2_000) as u64;
        let name = format!("track1_{frames}.tst");
        let mut h = Harness::new(&[name.as_str()]);
        h.engine.play_index(0).expect("play");
        assert!(
            h.wait_buffered(),
            "round {round}: the decoder must produce the track"
        );

        // The device pulls the whole track and more: the extra frames are
        // silence, and the run must not end before the content is drained.
        h.clock.pump(frames + 2 * STEP);
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
            "round {round}: the run must end"
        );

        assert_eq!(
            h.engine.timeline_frames(),
            frames,
            "round {round}: the timeline must freeze at the content end"
        );
        assert_eq!(
            h.engine.position_frames(),
            frames,
            "round {round}: the position must freeze at the content end"
        );
        assert_eq!(
            h.engine.buffered_frames(),
            0,
            "round {round}: the ring must be empty"
        );

        // And it must stay there: the run is over, the device is parked.
        for _ in 0..4 {
            h.clock.pump(STEP);
            h.engine.poll().expect("poll");
            assert_eq!(
                h.engine.timeline_frames(),
                frames,
                "round {round}: the frozen timeline moved"
            );
            assert_eq!(
                h.engine.position_frames(),
                frames,
                "round {round}: the frozen position moved"
            );
        }
    }
}

/// B1: the position never leaves the track it is in.
///
/// The device can play past the end of the content - the ring runs dry and
/// the device plays silence - and the position must still stay inside the
/// track, not run past its end.
#[test]
fn the_position_never_exceeds_the_track_length() {
    let mut rng = Rng::new(SEED ^ 0x55);
    for round in 0..ROUNDS {
        let frames = SHORT_TRACK_FRAMES + rng.below(2_000) as u64;
        let name = format!("track1_{frames}.tst");
        let mut h = Harness::new(&[name.as_str()]);
        h.engine.play_index(0).expect("play");
        assert!(
            h.wait_buffered(),
            "round {round}: the decoder must produce the track"
        );
        let duration = h.engine.duration_frames().expect("the track's length");
        assert_eq!(duration, frames, "round {round}: the track's length");

        // The device plays the whole track and then silence: the ring is
        // empty and the device keeps pulling.
        h.clock.pump(frames + 2 * STEP);
        assert!(
            h.engine.position_frames() <= duration,
            "round {round}: the position {} left the track of {duration}",
            h.engine.position_frames()
        );

        // And the run ends at the content end, not at the device position.
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
            "round {round}: the run must end"
        );
        assert_eq!(
            h.engine.position_frames(),
            frames,
            "round {round}: the position must freeze at the content end"
        );
    }
}

/// S1: a seek inside the track lands exactly, and the position follows it.
#[test]
fn a_seek_lands_inside_the_track_and_the_position_follows_it() {
    let mut rng = Rng::new(SEED ^ 0x66);
    for round in 0..ROUNDS {
        let name = format!("track1_{TRACK_FRAMES}.tst");
        let mut h = Harness::new(&[name.as_str()]);
        h.engine.play_index(0).expect("play");
        assert!(h.wait_track(0), "round {round}: the track must open");

        // A target inside the track is landed on exactly.
        let position = h.engine.position_frames();
        let Some(target) = seek_target(&mut rng, position, TRACK_FRAMES) else {
            continue;
        };
        h.engine.seek_frames(target).expect("seek");
        assert!(
            pump_until(&mut h, |e| e.position_frames() == target),
            "round {round}: the seek must land on {target}, not on {}",
            h.engine.position_frames()
        );
        assert!(
            h.engine.position_frames() <= TRACK_FRAMES,
            "round {round}: the position must stay inside the track"
        );

        // The position follows the target from there.
        let pulled = h.clock.pump(STEP);
        h.engine.poll().expect("poll");
        assert_eq!(
            h.engine.position_frames(),
            target + pulled,
            "round {round}: the position must follow the seek"
        );

        // A seek to the start of the track lands on zero.
        h.clock.pump(STEP);
        h.engine.poll().expect("poll");
        assert!(
            h.engine.position_frames() > 0,
            "round {round}: the position must have moved"
        );
        h.engine.seek_frames(0).expect("seek");
        assert!(
            pump_until(&mut h, |e| e.position_frames() == 0),
            "round {round}: the seek to zero must land on zero, not on {}",
            h.engine.position_frames()
        );
    }
}

/// S1: a seek past the end of the track is clamped to the track.
///
/// The clamp is the engine's own: it refuses to seek past the length of the
/// track, so the run ends at the content end instead of running past it.
#[test]
fn a_seek_past_the_end_is_clamped_to_the_track() {
    let mut rng = Rng::new(SEED ^ 0x77);
    for round in 0..ROUNDS {
        // A track that fits into the ring, so the decoder produces all of it
        // and the run can end.
        let frames = SHORT_TRACK_FRAMES + rng.below(2_000) as u64;
        let name = format!("track1_{frames}.tst");
        let mut h = Harness::new(&[name.as_str()]);
        h.engine.play_index(0).expect("play");
        assert!(
            h.wait_buffered(),
            "round {round}: the decoder must produce the track"
        );

        let over = frames + 1 + rng.below(10_000) as u64;
        h.engine.seek_frames(over).expect("seek");
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
            "round {round}: the run must end at the end of the track"
        );
        assert_eq!(
            h.engine.position_frames(),
            frames,
            "round {round}: the position must be clamped to the track"
        );
        assert_eq!(
            h.engine.timeline_frames(),
            frames,
            "round {round}: the timeline must be clamped to the track"
        );
    }
}

/// S1: a seek while the transport is stopped is clamped to the track too.
///
/// The engine positions itself at the seek target right away, so its own view
/// of where it is must not point past the end of the track.
#[test]
fn a_seek_while_stopped_is_clamped_to_the_track() {
    let mut rng = Rng::new(SEED ^ 0x88);
    for round in 0..ROUNDS {
        let name = format!("track1_{TRACK_FRAMES}.tst");
        let mut h = Harness::new(&[name.as_str()]);
        h.engine.play_index(0).expect("play");
        assert!(h.wait_track(0), "round {round}: the track must open");
        let duration = h.engine.duration_frames().expect("the track's length");
        h.engine.stop().expect("stop");

        let over = duration + 1 + rng.below(10_000) as u64;
        h.engine.seek_frames(over).expect("seek");
        assert!(
            h.engine.timeline_frames() <= duration,
            "round {round}: the timeline {} points past the track of {duration}",
            h.engine.timeline_frames()
        );
        assert!(
            h.engine.position_frames() <= duration,
            "round {round}: the position {} points past the track of {duration}",
            h.engine.position_frames()
        );
    }
}

/// C1: the frames and the milliseconds of the position agree.
#[test]
fn the_frames_and_the_milliseconds_of_the_position_agree() {
    let mut rng = Rng::new(SEED ^ 0x99);
    let rate = u64::from(SPEC.sample_rate);
    for round in 0..ROUNDS {
        let name = format!("track1_{TRACK_FRAMES}.tst");
        let mut h = Harness::new(&[name.as_str()]);
        h.engine.play_index(0).expect("play");
        assert!(h.wait_track(0), "round {round}: the track must open");

        for step in 0..STEPS {
            h.clock.pump(rng.below(8) as u64 * STEP);
            h.engine.poll().expect("poll");
            let position = h.engine.position_frames();
            let ms = h.engine.position_ms();
            let what = format!("round {round}, step {step}");
            assert_eq!(
                ms,
                position * 1000 / rate,
                "{what}: {position} frames must be {ms} ms"
            );
            assert_eq!(
                h.engine.state().position_ms,
                ms,
                "{what}: the published position"
            );
            // The conversion truncates, so the exact value is below the
            // reported one by less than a millisecond.
            let exact = position as f64 * 1000.0 / rate as f64;
            assert!(
                (ms as f64 - exact).abs() < 1.0,
                "{what}: {ms} ms must be within a millisecond of {exact}"
            );
            assert!(
                position <= TRACK_FRAMES,
                "{what}: the position must stay inside the track"
            );
        }
    }
}

/// R1: a second run starts at zero: the timeline origin is re-based.
#[test]
fn a_second_run_starts_at_zero() {
    let mut rng = Rng::new(SEED ^ 0xAA);
    for round in 0..ROUNDS {
        let name = format!("track1_{TRACK_FRAMES}.tst");
        let mut h = Harness::new(&[name.as_str()]);

        // The first run: play a little, then stop it. The stop leaves the
        // unwritten rest of the track in the ring, so the second run starts
        // with a drain that drops it.
        h.engine.play_index(0).expect("play");
        assert!(h.wait_track(0), "round {round}: the track must open");
        h.clock.pump(rng.below(4) as u64 * STEP);
        h.engine.poll().expect("poll");
        h.engine.stop().expect("stop");
        assert_eq!(
            h.engine.position_frames(),
            0,
            "round {round}: a stop resets the position"
        );

        // The second run: the timeline origin is re-based, so the run starts
        // at zero - and not at the end of the first one.
        h.engine.play().expect("play");
        assert!(
            pump_until(&mut h, |e| !e.is_start_pending()),
            "round {round}: the start drain must complete"
        );
        assert_eq!(
            h.engine.timeline_frames(),
            0,
            "round {round}: the second run starts at zero"
        );
        assert_eq!(
            h.engine.position_frames(),
            0,
            "round {round}: the second run starts at zero"
        );

        // And it plays on from there.
        let pulled = h.clock.pump(STEP);
        h.engine.poll().expect("poll");
        assert_eq!(
            h.engine.timeline_frames(),
            pulled,
            "round {round}: the second run must play on from zero"
        );
    }
}

/// R1: a run that ended by itself is followed by a run that starts at zero.
#[test]
fn a_run_after_a_finished_run_starts_at_zero() {
    let mut rng = Rng::new(SEED ^ 0xBB);
    for round in 0..ROUNDS {
        let frames = SHORT_TRACK_FRAMES + rng.below(2_000) as u64;
        let name = format!("track1_{frames}.tst");
        let mut h = Harness::new(&[name.as_str()]);
        h.engine.play_index(0).expect("play");
        assert!(
            h.wait_buffered(),
            "round {round}: the decoder must produce the track"
        );
        h.clock.pump(frames + 2 * STEP);
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
            "round {round}: the run must end"
        );
        assert_eq!(
            h.engine.timeline_frames(),
            frames,
            "round {round}: the frozen timeline"
        );

        // The next run starts at zero, not at the end of the finished one.
        h.engine.play().expect("play");
        assert!(
            pump_until(&mut h, |e| !e.is_start_pending()),
            "round {round}: the start drain must complete"
        );
        assert_eq!(
            h.engine.timeline_frames(),
            0,
            "round {round}: the second run starts at zero"
        );
        assert_eq!(
            h.engine.position_frames(),
            0,
            "round {round}: the second run starts at zero"
        );
    }
}
