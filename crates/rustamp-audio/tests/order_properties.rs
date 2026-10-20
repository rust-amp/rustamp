//! Property tests for the engine's playback order (S15).
//!
//! The traversal order is the `PlaybackOrder` model's job: it owns the
//! permutation, the repeat boundaries and the mid-run toggles. The engine
//! drives that model, and this file proves that the engine **follows** the
//! order it is given instead of computing an index of its own.
//!
//! # How the comparison works
//!
//! Every scenario builds two things that have to stay in step:
//!
//! * the **engine**, with a queue of synthetic tracks and a virtual device
//!   (`rustamp_output::NullOutput`), and
//! * a **reference**: a second `PlaybackOrder` with the same seed, driven
//!   with the same operations as the engine's own model.
//!
//! The reference is not a copy of the engine's model: it is an independent
//! instance that only ever sees the operations the scenario performs. If the
//! engine applied an extra step, skipped one, or re-planned the permutation
//! on its own, the two would drift apart and the scenario would fail.
//!
//! # The properties
//!
//! * **O1 - follows**: the sequence the engine visits equals the sequence the
//!   order names, step after step, in every mode combination.
//! * **O2 - published**: in shuffle mode the visited sequence is the
//!   permutation the engine publishes.
//! * **O3 - cycle**: a cycle visits every entry of the queue exactly once.
//! * **O4 - repeat off**: the traversal ends with the queue, and the run ends
//!   with `PlaylistEnded`.
//! * **O5 - repeat all**: the traversal wraps and starts a new cycle.
//! * **O6 - repeat one**: the mode that stops after the current track never
//!   advances the traversal.
//! * **O7 - back**: a step back returns to the entry it came from.
//! * **O8 - queue change**: a queue that changes mid-cycle keeps the engine
//!   and the order in step.
//! * **O9 - automatic**: the advance at a track end follows the order too.
//!
//! # The "repeat one" mode
//!
//! S15 defines two flags, shuffle and repeat, and the model has one boolean
//! for the repeat mode: there is **no** repeat-one mode in the contract
//! (S15 §2). The engine's mode that never advances the traversal is "stop
//! after current track" (RFC 0005 O8): the run ends at the end of the
//! current track instead of opening the next entry. That is the mode this
//! file tests under the name "repeat one"; a manual `Next` still moves on,
//! because O8 arms a *stop* at the end of the track, not a *repeat* of it.
//!
//! # The waits
//!
//! The manual scenarios play tracks that are far longer than the ring, so
//! the decode thread blocks in its write and a track cannot end inside a
//! scenario: the only advance the engine can make is the one the scenario
//! asks for. The waits measure the wall clock, because the decode thread
//! opens a file on the way and that can take a while on a loaded machine.

mod common;

use std::time::{Duration, Instant};

use common::{Harness, WAIT};
use rustamp_audio::TrackSource;
use rustamp_core::event::Event;
use rustamp_core::state::TransportState;
use rustamp_playlist::PlaybackOrder;
use rustamp_playlist::rng::DEFAULT_SEED;

/// The seed of every scenario in this file.
const SEED: u64 = 0x0A2E_0DE5_1EED_0001;

/// The number of scenarios per property.
const ROUNDS: usize = 8;

/// The length of the tracks the manual scenarios play.
///
/// The track is far longer than the ring (16384 frames), so the decode
/// thread blocks in its write and the track cannot end inside a scenario: the
/// only advance the engine can make is the one the scenario asks for.
const LONG_FRAMES: u64 = 200_000;

/// The length of the tracks the automatic scenarios play.
///
/// It fits into the ring, so the decoder produces the whole queue and the
/// engine can reach the end of it on its own.
const SHORT_FRAMES: u64 = 1_000;

/// The pump granularity of a wait, in device frames.
const PUMP: u64 = 256;

/// The frames a wait may pump.
///
/// The decode thread writes at most the ring plus the pumped frames, and the
/// track of the manual scenarios is longer than that: the track cannot end
/// inside a wait, so a wait can only see the manual advance.
const PUMP_BUDGET: u64 = 16_384;

/// A scenario: the engine under test and the reference traversal it has to
/// follow.
///
/// The engine's own model is shared with its decode thread, so the reference
/// is what says whether the engine followed the order or computed an index of
/// its own.
struct Scenario {
    /// The engine, its virtual device and the event subscription.
    harness: Harness,
    /// The reference traversal: the same operations, applied to a second
    /// `PlaybackOrder` with the same seed.
    reference: PlaybackOrder,
    /// The directory of the queue that replaced the original one, kept alive
    /// for as long as the scenario runs.
    replacement: Option<tempfile::TempDir>,
}

impl Scenario {
    /// A scenario with `len` entries of `frames` frames each, the given modes
    /// and the given seed.
    fn new(len: usize, shuffle: bool, repeat: bool, seed: u64, frames: u64) -> Scenario {
        let names = track_names(len, frames);
        let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut harness = Harness::new(&borrowed);
        // The reference starts where the engine's own model starts: an empty
        // order with the default seed, which the queue then fills.
        let mut reference = PlaybackOrder::new(0, false, false, DEFAULT_SEED);
        reference.set_len(len);
        harness.engine.set_shuffle_seed(seed);
        reference.set_seed(seed);
        harness.engine.set_shuffle(shuffle).unwrap();
        reference.set_shuffle(shuffle);
        harness.engine.set_repeat(repeat).unwrap();
        reference.set_repeat(repeat);
        Scenario {
            harness,
            reference,
            replacement: None,
        }
    }

    /// Start playback at the head of the order.
    ///
    /// The engine takes the first entry of the order when nothing is
    /// selected yet, so the reference does the same.
    fn play(&mut self) -> Option<u32> {
        self.harness.engine.play().unwrap();
        self.reference.next_index().map(|index| index as u32)
    }

    /// Advance the engine by one entry; the reference says where it goes.
    fn next(&mut self) -> Option<u32> {
        self.harness.engine.next_track().unwrap();
        self.reference.next_index().map(|index| index as u32)
    }

    /// Step the engine back by one entry; the reference says where it goes.
    fn prev(&mut self) -> Option<u32> {
        self.harness.engine.prev_track().unwrap();
        self.reference.prev_index().map(|index| index as u32)
    }

    /// Replace the queue with `len` entries and keep the reference in step.
    ///
    /// The engine takes the new queue over when it owns the traversal, that
    /// is, when no run is live: a live run keeps the queue it started with,
    /// and the order is brought up to date when the run ends.
    fn set_queue(&mut self, len: usize) {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut sources = Vec::new();
        for name in track_names(len, LONG_FRAMES) {
            let path = dir.path().join(name);
            std::fs::write(&path, b"").expect("writing the placeholder track");
            sources.push(TrackSource::path(path));
        }
        self.harness.engine.set_queue(sources);
        // The engine brings its order in line with the queue: a new length
        // rebuilds the plan, the same length invalidates it.
        if self.reference.len() != len {
            self.reference.set_len(len);
        } else {
            self.reference.invalidate();
        }
        self.replacement = Some(dir);
    }

    /// The index the engine publishes as the current track.
    fn current(&self) -> Option<u32> {
        self.harness.engine.current_index()
    }

    /// The published modes and the order's modes must not disagree.
    fn assert_modes_agree(&self, what: &str) {
        assert_eq!(
            self.harness.engine.shuffle(),
            self.reference.shuffle(),
            "{what}: the engine and the order disagree about the shuffle mode"
        );
        assert_eq!(
            self.harness.engine.repeat(),
            self.reference.repeat(),
            "{what}: the engine and the order disagree about the repeat mode"
        );
    }
}

/// The names of `len` synthetic tracks of `frames` frames each.
fn track_names(len: usize, frames: u64) -> Vec<String> {
    (1..=len)
        .map(|i| format!("track{i}_{frames}.tst"))
        .collect()
}

/// The seed of a scenario: reproducible from the round and the mode.
fn scenario_seed(round: usize, len: usize, shuffle: bool, repeat: bool) -> u64 {
    SEED ^ (round as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (len as u64).wrapping_mul(0x1234_5678_9ABC_DEF1)
        ^ ((shuffle as u64) << 32)
        ^ ((repeat as u64) << 40)
}

/// Wait until the engine publishes `index` as the current track.
///
/// The advance is asynchronous: the decode thread abandons the current track
/// and opens the next one, and the engine learns about it when it polls the
/// thread's reports.
///
/// The wait measures the wall clock, not a pump count, because the decode
/// thread opens a file on the way and that can take a while on a loaded
/// machine. The pump is bounded by [`PUMP_BUDGET`] frames: it keeps the
/// pipeline moving, and the budget keeps the track far from its end (the
/// decode thread writes at most the ring plus the pumped frames, and the
/// track of the manual scenarios is longer than that). The wait can
/// therefore only see the manual advance.
///
/// Returns the index the engine published last, so a failure can say what
/// the engine published instead of the entry it was asked for.
fn wait_current(harness: &mut Harness, index: u32) -> Option<u32> {
    let deadline = Instant::now() + WAIT;
    let mut pumped = 0;
    loop {
        harness.engine.poll().unwrap();
        let current = harness.engine.current_index();
        if current == Some(index) {
            return Some(index);
        }
        if Instant::now() >= deadline {
            return current;
        }
        if pumped < PUMP_BUDGET {
            harness.clock.pump(PUMP);
            pumped += PUMP;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// The indices of the tracks the engine reported a start for, in order.
fn visited_tracks(events: &[Event]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::TrackChanged {
                index: Some(index), ..
            } => Some(*index),
            _ => None,
        })
        .collect()
}

/// **O1** - the engine visits the entries the order names, step after step,
/// in every mode combination.
///
/// The reference is a second model with the same seed, so the visited
/// sequence and the predicted sequence cannot drift apart.
#[test]
fn the_engine_visits_the_order_the_model_predicts() {
    for len in 2..=5usize {
        for shuffle in [false, true] {
            for repeat in [false, true] {
                for round in 0..ROUNDS {
                    let seed = scenario_seed(round, len, shuffle, repeat);
                    let mut s = Scenario::new(len, shuffle, repeat, seed, LONG_FRAMES);
                    let mut visited: Vec<u32> = Vec::new();
                    let mut predicted: Vec<u32> = Vec::new();

                    // The traversal starts at the head of the order.
                    let start = s.play().expect("the order names a first track");
                    assert_eq!(
                        wait_current(&mut s.harness, start),
                        Some(start),
                        "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                         the first track did not start"
                    );
                    visited.push(s.current().expect("a track plays"));
                    predicted.push(start);

                    // Every step has to land on the entry the order names.
                    for step in 0..(len + 1) {
                        let Some(want) = s.next() else { break };
                        assert_eq!(
                            wait_current(&mut s.harness, want),
                            Some(want),
                            "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}, step {step}: \
                             the engine did not reach the entry {want} the order names"
                        );
                        visited.push(s.current().expect("a track plays"));
                        predicted.push(want);
                    }

                    assert_eq!(
                        visited, predicted,
                        "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                         the engine visited another sequence than the order names"
                    );
                    s.assert_modes_agree("after the traversal");

                    // **O3** - a cycle covers the queue exactly once.
                    let cycle: Vec<u32> = visited.iter().take(len).copied().collect();
                    assert_eq!(
                        cycle.len(),
                        len,
                        "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                         the cycle is shorter than the queue"
                    );
                    if shuffle {
                        let mut sorted = cycle.clone();
                        sorted.sort_unstable();
                        assert_eq!(
                            sorted,
                            (0..len as u32).collect::<Vec<_>>(),
                            "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                             the shuffled cycle must cover the queue exactly once"
                        );
                    } else {
                        assert_eq!(
                            cycle,
                            (0..len as u32).collect::<Vec<_>>(),
                            "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                             the sequential order must run from the first to the last entry"
                        );
                    }

                    if repeat {
                        // **O5** - the traversal wraps and starts a new cycle.
                        assert_eq!(
                            visited.len(),
                            len + 2,
                            "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                             the traversal must continue past the end of the queue"
                        );
                        if shuffle {
                            // S15 §2.1: the new cycle does not start with the
                            // track the exhausted cycle ended on.
                            assert_ne!(
                                visited[len],
                                cycle[len - 1],
                                "len {len}, seed {seed}: the new cycle starts with the last \
                                 track of the exhausted one"
                            );
                        }
                    } else {
                        // **O4** - the traversal ends with the queue.
                        assert_eq!(
                            visited.len(),
                            len,
                            "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                             the traversal must stop at the end of the queue"
                        );
                    }
                }
            }
        }
    }
}

/// **O2** - in shuffle mode the traversal is the permutation the engine
/// publishes.
///
/// The published plan is the order the player is given: the window shows it,
/// and the traversal has to be that permutation, entry after entry.
#[test]
fn the_engine_follows_the_published_shuffle_plan() {
    for len in 2..=6usize {
        for round in 0..ROUNDS {
            let seed = scenario_seed(round, len, true, false);
            let mut s = Scenario::new(len, true, false, seed, LONG_FRAMES);
            let plan = s.harness.engine.shuffle_plan();
            assert_eq!(plan.len(), len, "the published plan must cover the queue");
            let mut sorted = plan.clone();
            sorted.sort_unstable();
            assert_eq!(
                sorted,
                (0..len).collect::<Vec<_>>(),
                "the published plan must be a permutation of the queue"
            );

            let mut visited: Vec<u32> = Vec::new();
            let start = s.play().expect("the order names a first track");
            assert_eq!(
                wait_current(&mut s.harness, start),
                Some(start),
                "len {len}, seed {seed}: the first track did not start"
            );
            visited.push(s.current().expect("a track plays"));
            for step in 1..len {
                let want = s.next().expect("the order names another track");
                assert_eq!(
                    wait_current(&mut s.harness, want),
                    Some(want),
                    "len {len}, seed {seed}, step {step}: \
                     the engine did not reach the entry {want} of the published plan"
                );
                visited.push(s.current().expect("a track plays"));
            }
            assert_eq!(
                visited,
                plan.iter().map(|index| *index as u32).collect::<Vec<_>>(),
                "len {len}, seed {seed}: the traversal must be the published permutation"
            );
        }
    }
}

/// **O9** - the advance at a track end follows the order as well.
///
/// The tracks are short, so the decode thread runs into the next entry by
/// itself: the traversal is the model's, not the engine's.
#[test]
fn the_engine_advances_by_itself_in_the_order_the_model_predicts() {
    for len in 2..=5usize {
        for shuffle in [false, true] {
            for round in 0..ROUNDS {
                let seed = scenario_seed(round, len, shuffle, false);
                let mut s = Scenario::new(len, shuffle, false, seed, SHORT_FRAMES);
                s.harness.drain_events();
                let start = s.play().expect("the order names a first track");
                assert!(
                    s.harness
                        .wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
                    "len {len}, shuffle {shuffle}, seed {seed}: the run must end at the end of the queue"
                );

                let events = s.harness.drain_events();
                let visited = visited_tracks(&events);
                assert_eq!(
                    visited.len(),
                    len,
                    "len {len}, shuffle {shuffle}, seed {seed}: the run must play every entry once"
                );

                let mut predicted = vec![start];
                for _ in 1..len {
                    predicted.push(
                        s.reference
                            .next_index()
                            .expect("the order names another track") as u32,
                    );
                }
                assert_eq!(
                    visited, predicted,
                    "len {len}, shuffle {shuffle}, seed {seed}: \
                     the engine played another sequence than the order names"
                );
                // **O4** - the run ends with the queue.
                assert!(
                    events.contains(&Event::PlaylistEnded),
                    "len {len}, shuffle {shuffle}, seed {seed}: \
                     the run must publish PlaylistEnded: {events:?}"
                );
                // The run heard the whole queue: the frozen timeline is the
                // end of the last track, not a track boundary in the middle
                // of it.
                assert_eq!(
                    s.harness.engine.timeline_frames(),
                    len as u64 * SHORT_FRAMES,
                    "len {len}, shuffle {shuffle}, seed {seed}: \
                     the run must end at the end of the last track"
                );
            }
        }
    }
}

/// **O6** - the mode that stops after the current track never advances the
/// traversal.
///
/// The run ends at the end of the current track instead of opening the next
/// entry, so the engine visits exactly one entry and stops there.
#[test]
fn the_stop_after_current_mode_never_advances_the_traversal() {
    for len in 1..=4usize {
        for shuffle in [false, true] {
            for round in 0..ROUNDS {
                let seed = scenario_seed(round, len, shuffle, false);
                let mut s = Scenario::new(len, shuffle, false, seed, SHORT_FRAMES);
                // Arm the mode before the run starts, so the decode thread
                // takes it over with the run.
                s.harness.engine.toggle_stop_after_current().unwrap();
                s.harness.drain_events();
                let start = s.play().expect("the order names a first track");
                assert!(
                    s.harness
                        .wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
                    "len {len}, shuffle {shuffle}, seed {seed}: \
                     the run must end at the end of the current track"
                );

                let events = s.harness.drain_events();
                let visited = visited_tracks(&events);
                assert_eq!(
                    visited,
                    vec![start],
                    "len {len}, shuffle {shuffle}, seed {seed}: \
                     the traversal must not advance: {visited:?}"
                );
                if len > 1 {
                    // The queue was not exhausted, so the playlist did not end.
                    assert!(
                        !events.contains(&Event::PlaylistEnded),
                        "len {len}, shuffle {shuffle}, seed {seed}: \
                         the queue was not exhausted: {events:?}"
                    );
                }
            }
        }
    }
}

/// **O7** - a step back returns to the entry it came from.
///
/// S15 §3.2: the step back walks the traversal backwards, so it lands on the
/// entry the traversal came from - in shuffle mode not on the entry before it
/// in the playlist, but on the one before it in the permutation.
#[test]
fn previous_after_next_returns_to_the_entry_it_came_from() {
    for len in 2..=5usize {
        for shuffle in [false, true] {
            for repeat in [false, true] {
                for round in 0..ROUNDS {
                    let seed = scenario_seed(round, len, shuffle, repeat);
                    let mut s = Scenario::new(len, shuffle, repeat, seed, LONG_FRAMES);
                    let start = s.play().expect("the order names a first track");
                    assert_eq!(
                        wait_current(&mut s.harness, start),
                        Some(start),
                        "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                         the first track did not start"
                    );

                    // One step forward, then one step back.
                    let forward = s.next().expect("the order names another track");
                    assert_eq!(
                        wait_current(&mut s.harness, forward),
                        Some(forward),
                        "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                         the engine did not reach the entry {forward} the order names"
                    );
                    let back = s.prev().expect("the order names the entry before it");
                    assert_eq!(
                        wait_current(&mut s.harness, back),
                        Some(back),
                        "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                         the engine did not step back to the entry {back} the order names"
                    );
                    assert_eq!(
                        s.current(),
                        Some(back),
                        "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                         the engine published another entry after the step back"
                    );
                    assert_eq!(
                        back, start,
                        "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                         the step back did not return to the entry it came from"
                    );
                }
            }
        }
    }
}

/// **O8** - a queue that changes mid-cycle keeps the engine and the order in
/// step.
///
/// The engine takes the new queue over when it owns the traversal, that is,
/// when no run is live. The order is brought up to date with the new queue
/// and the traversal continues where the order says.
#[test]
fn a_queue_change_mid_cycle_keeps_the_engine_consistent_with_the_order() {
    for len in 3..=5usize {
        for shuffle in [false, true] {
            for repeat in [false, true] {
                for round in 0..ROUNDS {
                    let seed = scenario_seed(round, len, shuffle, repeat);
                    let mut s = Scenario::new(len, shuffle, repeat, seed, LONG_FRAMES);
                    let start = s.play().expect("the order names a first track");
                    assert_eq!(
                        wait_current(&mut s.harness, start),
                        Some(start),
                        "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                         the first track did not start"
                    );
                    // Two steps into the cycle.
                    for step in 0..2 {
                        let want = s.next().expect("the order names another track");
                        assert_eq!(
                            wait_current(&mut s.harness, want),
                            Some(want),
                            "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}, step {step}: \
                             the engine did not reach the entry {want} the order names"
                        );
                    }

                    // The engine owns the traversal again.
                    s.harness.engine.stop().unwrap();

                    // The queue changes: an entry is added or removed.
                    let new_len = if round % 2 == 0 { len + 1 } else { len - 1 };
                    s.set_queue(new_len);
                    s.assert_modes_agree("after the queue change");
                    assert_eq!(
                        s.harness.engine.shuffle_plan().len(),
                        if shuffle { new_len } else { 0 },
                        "the published plan must describe the new queue"
                    );

                    // The traversal continues where the order says.
                    let before = s.current();
                    match s.next() {
                        Some(want) => {
                            assert_eq!(
                                wait_current(&mut s.harness, want),
                                Some(want),
                                "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                                 the engine did not reach the entry {want} the order names \
                                 after the queue change"
                            );
                            assert_eq!(
                                s.current(),
                                Some(want),
                                "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                                 the engine published another entry after the queue change"
                            );
                        }
                        None => {
                            // The order ends the traversal here: the engine
                            // must stay on the entry it is on and must not
                            // start a run.
                            assert_eq!(
                                s.current(),
                                before,
                                "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                                 the engine left the entry the order ended on"
                            );
                            assert_eq!(
                                s.harness.engine.transport(),
                                TransportState::Stopped,
                                "len {len}, shuffle {shuffle}, repeat {repeat}, seed {seed}: \
                                 the engine started a run although the order ended the traversal"
                            );
                        }
                    }
                }
            }
        }
    }
}

/// A shuffle toggle in the middle of a run re-plans the order around the
/// track that plays, and the traversal follows the new plan (S15 §4.1).
#[test]
fn a_mid_run_shuffle_toggle_re_plans_the_order_and_the_traversal_follows_it() {
    for len in 2..=5usize {
        for round in 0..ROUNDS {
            let seed = scenario_seed(round, len, false, false);
            let mut s = Scenario::new(len, false, false, seed, LONG_FRAMES);
            let start = s.play().expect("the order names a first track");
            assert_eq!(
                wait_current(&mut s.harness, start),
                Some(start),
                "len {len}, seed {seed}: the first track did not start"
            );

            // The engine anchors the new plan at the track that plays.
            let anchor = s
                .harness
                .engine
                .audible_index()
                .or(s.harness.engine.current_index());
            s.harness.engine.set_shuffle(true).unwrap();
            if let Some(anchor) = anchor {
                s.reference.set_current_index(Some(anchor as usize));
            }
            s.reference.set_shuffle(true);
            s.assert_modes_agree("after the shuffle toggle");
            assert_eq!(
                s.harness.engine.shuffle_plan(),
                s.reference.shuffle_plan(),
                "len {len}, seed {seed}: the engine must publish the plan the order built"
            );
            let plan = s.harness.engine.shuffle_plan();
            assert_eq!(
                plan.first(),
                Some(&(anchor.expect("a track plays") as usize)),
                "len {len}, seed {seed}: the new plan must start at the track that plays"
            );

            // The traversal follows the new plan.
            for step in 1..len {
                let want = s.next().expect("the order names another track");
                assert_eq!(
                    wait_current(&mut s.harness, want),
                    Some(want),
                    "len {len}, seed {seed}, step {step}: \
                     the engine did not reach the entry {want} of the new plan"
                );
                assert_eq!(s.current(), Some(want));
            }
        }
    }
}
