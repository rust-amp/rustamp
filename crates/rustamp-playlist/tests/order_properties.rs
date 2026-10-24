//! Property tests for the S15 shuffle and repeat traversal model.
//!
//! [`PlaybackOrder`] decides every next/previous step the engine takes, so
//! its invariants are checked over many seeded scenarios instead of a handful
//! of hand-written examples.
//!
//! The scenarios are driven by the crate's own [`Rng`]: no new dependency is
//! needed, and every case is reproducible from the printed seed and step.

use std::time::{Duration, Instant};

use rustamp_playlist::{MAX_SKIP_STEPS, PlaybackOrder, Rng};

/// Seed of the random scenario stream.
const SEED: u64 = 0x5EED_2060_0000_0001;

/// Number of independent random scenarios per property.
const ROUNDS: usize = 500;

/// Number of operations per random scenario.
const STEPS: usize = 40;

/// The operations a scenario may apply to the model.
#[derive(Debug, Clone, Copy)]
enum Op {
    Next,
    Prev,
    Skip(i32),
    SetLen(usize),
    SetCurrent(Option<usize>),
    Invalidate,
    ToggleShuffle,
    ToggleRepeat,
    SetSeed(u64),
}

/// A random operation list. The queue length is not known here, so
/// `SetCurrent` may pick an index outside the queue: the model has to
/// reject it.
fn random_ops(rng: &mut Rng, count: usize) -> Vec<Op> {
    (0..count)
        .map(|_| match rng.below(10) {
            0 => Op::Next,
            1 => Op::Prev,
            2 => Op::Skip(rng.below(9) as i32 - 4),
            3 => Op::SetLen(rng.below(10)),
            4 => {
                if rng.below(5) == 0 {
                    Op::SetCurrent(None)
                } else {
                    Op::SetCurrent(Some(rng.below(12)))
                }
            }
            5 => Op::Invalidate,
            6 => Op::ToggleShuffle,
            7 => Op::ToggleRepeat,
            8 => Op::SetSeed(rng.next_u64()),
            _ => Op::Next,
        })
        .collect()
}

/// The operation list of the exhaustive search: one entry per way the model
/// can be pushed off its happy path.
fn exhaustive_ops(len: usize) -> Vec<Op> {
    vec![
        Op::Next,
        Op::Prev,
        Op::Skip(1),
        Op::Skip(-1),
        Op::SetLen(0),
        Op::SetLen(len + 1),
        Op::SetCurrent(Some(0)),
        Op::SetCurrent(Some(len)),
        Op::Invalidate,
        Op::ToggleShuffle,
        Op::ToggleRepeat,
    ]
}

/// Applies one operation and reports the index the model answered with, if
/// the operation navigates.
fn apply(order: &mut PlaybackOrder, op: Op) -> Option<usize> {
    match op {
        Op::Next => order.next_index(),
        Op::Prev => order.prev_index(),
        Op::Skip(delta) => order.skip_by(delta),
        Op::SetLen(len) => {
            order.set_len(len);
            None
        }
        Op::SetCurrent(index) => {
            order.set_current_index(index);
            order.current_index()
        }
        Op::Invalidate => {
            order.invalidate();
            None
        }
        Op::ToggleShuffle => {
            order.toggle_shuffle();
            None
        }
        Op::ToggleRepeat => {
            order.toggle_repeat();
            None
        }
        Op::SetSeed(seed) => {
            order.set_seed(seed);
            None
        }
    }
}

/// Asserts that `indices` holds every index of `0..len` exactly once.
fn assert_permutation(indices: &[usize], len: usize, what: &str) {
    let mut sorted = indices.to_vec();
    sorted.sort_unstable();
    assert_eq!(
        sorted,
        (0..len).collect::<Vec<_>>(),
        "{what}: {indices:?} is not a permutation of 0..{len}"
    );
}

/// Asserts the invariants that must hold after every single operation.
fn assert_invariants(order: &PlaybackOrder, what: &str) {
    let len = order.len();
    if let Some(current) = order.current_index() {
        assert!(
            current < len,
            "{what}: the current index {current} is outside the queue of {len} entries"
        );
    }
    let plan = order.shuffle_plan();
    if order.shuffle() {
        assert_eq!(
            plan.len(),
            len,
            "{what}: the shuffle plan holds {} entries, the queue holds {len}",
            plan.len()
        );
        if len > 0 {
            assert_permutation(plan, len, what);
        }
        if !plan.is_empty() {
            assert!(
                order.shuffle_pos() < plan.len(),
                "{what}: the plan position {} is outside the plan of {} entries",
                order.shuffle_pos(),
                plan.len()
            );
            // The track that plays is part of the permutation it follows.
            if let Some(current) = order.current_index() {
                assert!(
                    plan.contains(&current),
                    "{what}: the current track {current} is not part of the plan {plan:?}"
                );
                assert_eq!(
                    plan.get(order.shuffle_pos()),
                    Some(&current),
                    "{what}: the plan position {} does not point at the current track {current}",
                    order.shuffle_pos()
                );
            }
        }
    } else {
        assert!(
            plan.is_empty(),
            "{what}: a sequential order must not keep a shuffle plan"
        );
    }
}

/// Walks the current shuffle cycle and returns the indices it plays.
///
/// The first track of the next cycle does not belong to the current one, so
/// the walk stops there. The cycle boundary is detected through the retained
/// previous plan, which only the boundary sets.
fn walk_cycle(order: &mut PlaybackOrder) -> Vec<usize> {
    let mut visited = Vec::new();
    while let Some(index) = order.next_index() {
        if order.prev_shuffle_plan().is_some() {
            break;
        }
        visited.push(index);
    }
    visited
}

/// Every random operation sequence stays inside the queue and keeps the plan
/// a permutation.
#[test]
fn random_operation_sequences_never_leave_the_queue() {
    let mut rng = Rng::new(SEED);
    let mut saw_plan = 0usize;
    let mut saw_boundary = 0usize;
    let mut saw_length_change = 0usize;
    let mut saw_rejected_index = 0usize;
    for round in 0..ROUNDS {
        let len = rng.below(10);
        let shuffle = rng.below(2) == 1;
        let repeat = rng.below(2) == 1;
        let seed = rng.next_u64();
        let mut order = PlaybackOrder::new(len, shuffle, repeat, seed);
        let ops = random_ops(&mut rng, STEPS);
        for (step, op) in ops.iter().copied().enumerate() {
            let what = format!(
                "round {round}, step {step}, op {op:?}, queue of {} entries",
                order.len()
            );
            let answered = apply(&mut order, op);
            if let Some(index) = answered {
                assert!(
                    index < order.len(),
                    "{what}: the model answered index {index}, outside the queue"
                );
            }
            if matches!(op, Op::SetLen(_)) {
                saw_length_change += 1;
            }
            if matches!(op, Op::SetCurrent(Some(_))) && order.current_index().is_none() {
                saw_rejected_index += 1;
            }
            if !order.shuffle_plan().is_empty() {
                saw_plan += 1;
            }
            if order.prev_shuffle_plan().is_some() {
                saw_boundary += 1;
            }
            assert_invariants(&order, &what);
        }
    }
    // The generator has to reach the interesting states, otherwise the
    // property above would hold trivially.
    assert!(saw_plan > 0, "no scenario ever built a shuffle plan");
    assert!(
        saw_boundary > 0,
        "no scenario ever crossed a shuffle cycle boundary"
    );
    assert!(
        saw_length_change > 0,
        "no scenario ever changed the queue length"
    );
    assert!(
        saw_rejected_index > 0,
        "no scenario ever tried an index outside the queue"
    );
}

/// The same holds for every operation sequence of a bounded length, checked
/// exhaustively for the small queues.
#[test]
fn exhaustive_operation_sequences_never_leave_the_queue() {
    const DEPTH: usize = 4;
    for len in 0..=3usize {
        for shuffle in [true, false] {
            for repeat in [true, false] {
                let ops = exhaustive_ops(len);
                let mut order = PlaybackOrder::new(len, shuffle, repeat, 7);
                explore(&mut order, &ops, DEPTH, "exhaustive scenario");
            }
        }
    }
}

/// Applies every operation sequence of the given depth and checks the
/// invariants after every step.
fn explore(order: &mut PlaybackOrder, ops: &[Op], depth: usize, what: &str) {
    if depth == 0 {
        return;
    }
    for op in ops {
        let answered = apply(order, *op);
        let what = format!("{what}, op {op:?}");
        if let Some(index) = answered {
            assert!(
                index < order.len(),
                "{what}: the model answered index {index}, outside the queue"
            );
        }
        assert_invariants(order, &what);
        explore(order, ops, depth - 1, &what);
    }
}

/// The same seed and the same operations produce the same answers.
#[test]
fn the_same_seed_and_operations_give_the_same_answers() {
    let mut rng = Rng::new(SEED ^ 0xD7);
    for round in 0..ROUNDS {
        let len = rng.below(10);
        let shuffle = rng.below(2) == 1;
        let repeat = rng.below(2) == 1;
        let seed = rng.next_u64();
        let ops = random_ops(&mut rng, STEPS);

        let mut a = PlaybackOrder::new(len, shuffle, repeat, seed);
        let mut b = PlaybackOrder::new(len, shuffle, repeat, seed);
        for (step, op) in ops.iter().copied().enumerate() {
            let answered_a = apply(&mut a, op);
            let answered_b = apply(&mut b, op);
            assert_eq!(
                answered_a, answered_b,
                "round {round}, step {step}, op {op:?}: the two runs disagree"
            );
            assert_eq!(
                a.current_index(),
                b.current_index(),
                "round {round}, step {step}, op {op:?}: the two runs disagree on the current track"
            );
            assert_eq!(
                a.shuffle_plan(),
                b.shuffle_plan(),
                "round {round}, step {step}, op {op:?}: the two runs disagree on the plan"
            );
            assert_eq!(
                a.shuffle_pos(),
                b.shuffle_pos(),
                "round {round}, step {step}, op {op:?}: the two runs disagree on the plan position"
            );
        }
    }
}

/// A full cycle plays every index of the queue exactly once, in both the
/// shuffled and the sequential order.
#[test]
fn a_full_cycle_plays_every_index_exactly_once() {
    for len in 0..=10usize {
        for shuffle in [true, false] {
            for repeat in [true, false] {
                for seed in [1u64, 2, 3, 0xDEAD_BEEF] {
                    let mut order = PlaybackOrder::new(len, shuffle, repeat, seed);
                    let mut visited = Vec::new();
                    for _ in 0..len {
                        let index = order.next_index().unwrap_or_else(|| {
                            panic!("len {len}, shuffle {shuffle}, repeat {repeat}: the cycle ended early")
                        });
                        visited.push(index);
                    }
                    if shuffle {
                        assert_permutation(&visited, len, "shuffled cycle");
                    } else {
                        assert_eq!(
                            visited,
                            (0..len).collect::<Vec<_>>(),
                            "the sequential order must run from the first to the last entry"
                        );
                    }

                    // The step past the cycle either ends the queue or starts
                    // the next cycle.
                    match (shuffle, repeat, len) {
                        (_, _, 0) => assert_eq!(order.next_index(), None),
                        (false, false, _) => {
                            assert_eq!(order.next_index(), None, "the queue ends without repeat")
                        }
                        (false, true, _) => assert_eq!(
                            order.next_index(),
                            Some(0),
                            "the sequential order wraps around with repeat"
                        ),
                        (true, false, _) => {
                            assert_eq!(order.next_index(), None, "the cycle ends without repeat")
                        }
                        (true, true, _) => {
                            let first = order
                                .next_index()
                                .unwrap_or_else(|| panic!("len {len}: repeat must continue"));
                            assert!(first < len);
                            assert_permutation(order.shuffle_plan(), len, "the next cycle");
                        }
                    }
                }
            }
        }
    }
}

/// The new cycle never starts with the track the exhausted cycle ended on.
#[test]
fn a_new_cycle_does_not_repeat_the_last_track() {
    for len in 2..=10usize {
        for seed in 0..24u64 {
            let mut order = PlaybackOrder::new(len, true, true, seed.wrapping_mul(0x9E37_79B9) ^ 1);
            let mut cycle = Vec::new();
            for _ in 0..len {
                cycle.push(order.next_index().expect("the cycle yields a track"));
            }
            let mut last = *cycle.last().expect("the cycle is not empty");
            for round in 0..4 {
                let first = order.next_index().expect("repeat continues");
                assert_ne!(
                    first, last,
                    "len {len}, seed {seed}, cycle {round}: the new cycle starts with the last track of the previous one"
                );
                for _ in 1..len {
                    order.next_index().expect("the cycle yields a track");
                }
                last = order.current_index().expect("the cycle ends on a track");
            }
        }
    }
}

/// The same guarantee holds when the traversal steps back across the cycle
/// boundary and forward again.
#[test]
fn a_new_cycle_does_not_repeat_the_last_track_after_stepping_back() {
    for len in 2..=10usize {
        for seed in 0..24u64 {
            let mut order = PlaybackOrder::new(len, true, true, seed.wrapping_mul(0x9E37_79B9) ^ 1);
            let mut cycle = Vec::new();
            for _ in 0..len {
                cycle.push(order.next_index().expect("the cycle yields a track"));
            }
            let last = *cycle.last().expect("the cycle is not empty");

            // Cross the boundary, then step back onto the last track of the
            // exhausted cycle (S15 §3.2).
            let first = order.next_index().expect("repeat continues");
            assert_ne!(
                first, last,
                "len {len}, seed {seed}: the boundary repeats a track"
            );
            let restored = order.prev_index().expect("the previous cycle is restored");
            assert_eq!(
                restored, last,
                "len {len}, seed {seed}: the wrong track restored"
            );

            // Forward again: the cycle that starts now must not repeat the
            // track the restored cycle ended on.
            let again = order.next_index().expect("repeat continues");
            assert_ne!(
                again, last,
                "len {len}, seed {seed}: the new cycle repeats the last track of the restored one"
            );
        }
    }
}

/// Toggling shuffle on mid-run keeps the current track playing and heads the
/// new plan with it; the rest of the queue follows in one cycle (S15 §4.1).
#[test]
fn toggling_shuffle_on_mid_run_keeps_the_current_track_first() {
    let mut rng = Rng::new(SEED ^ 0x4A);
    for _round in 0..ROUNDS {
        let len = 1 + rng.below(10);
        let mut order = PlaybackOrder::new(len, false, false, rng.next_u64());
        let _ = order.next_index();
        let current = rng.below(len);
        order.set_current_index(Some(current));
        order.set_shuffle(true);

        assert_eq!(
            order.current_index(),
            Some(current),
            "the track that plays keeps playing"
        );
        assert_eq!(
            order.shuffle_plan().first(),
            Some(&current),
            "the track that plays heads the plan"
        );

        let mut visited = vec![current];
        while let Some(index) = order.next_index() {
            visited.push(index);
        }
        assert_permutation(&visited, len, "the cycle after the mid-run toggle");
    }
}

/// Toggling shuffle off mid-run discards the plan and resumes the queue
/// order behind the current track (S15 §4.2).
#[test]
fn toggling_shuffle_off_mid_run_resumes_the_queue_order() {
    for len in 1..=10usize {
        for repeat in [true, false] {
            let mut order = PlaybackOrder::new(len, true, repeat, 31);
            let _ = order.next_index();
            let current = order.current_index().expect("a track plays");
            order.set_shuffle(false);

            assert_eq!(
                order.current_index(),
                Some(current),
                "the track that plays keeps playing"
            );
            assert!(
                order.shuffle_plan().is_empty(),
                "the shuffle plan is discarded"
            );

            let next = order.next_index();
            if current + 1 < len {
                assert_eq!(next, Some(current + 1), "the queue order resumes");
            } else if repeat {
                assert_eq!(next, Some(0), "the queue order wraps around");
            } else {
                assert_eq!(next, None, "the queue ends");
            }
        }
    }
}

/// Skipping is the same as advancing step by step, in both directions
/// (S15 §3.3).
#[test]
fn skipping_matches_repeated_advancing() {
    for len in 1..=8usize {
        for shuffle in [true, false] {
            for repeat in [true, false] {
                for delta in -6i32..=6 {
                    let mut skipped = PlaybackOrder::new(len, shuffle, repeat, 23);
                    let mut stepped = PlaybackOrder::new(len, shuffle, repeat, 23);
                    let answer = skipped.skip_by(delta);
                    let mut expected = None;
                    if delta > 0 {
                        for _ in 0..delta {
                            match stepped.next_index() {
                                Some(index) => expected = Some(index),
                                None => break,
                            }
                        }
                    } else {
                        for _ in 0..delta.unsigned_abs() {
                            match stepped.prev_index() {
                                Some(index) => expected = Some(index),
                                None => break,
                            }
                        }
                    }
                    assert_eq!(
                        answer, expected,
                        "len {len}, shuffle {shuffle}, repeat {repeat}, delta {delta}: skipping and stepping disagree"
                    );
                }
            }
        }
    }
}

/// An explicit selection moves the cursor into the active permutation
/// instead of rebuilding it: the traversal continues behind the selected
/// entry, and the entries before it in the permutation are deferred to the
/// next cycle.
///
/// This is not a weakening of the permutation property but its precise
/// scope: a cycle that starts at the head of the permutation plays every
/// entry of the queue exactly once. A selection is an outside choice that
/// re-points the cursor *inside* the running permutation, so the entries
/// before the cursor are not part of the cycle that is left. The engine's
/// decode thread relies on that: it re-aligns its own place in a
/// permutation the engine re-planned (S15 §4.1) without disturbing it.
#[test]
fn selecting_a_track_moves_the_cursor_into_the_permutation() {
    let mut rng = Rng::new(SEED ^ 0x5E);
    for _round in 0..ROUNDS {
        let len = 2 + rng.below(8);
        let repeat = rng.below(2) == 1;
        let mut order = PlaybackOrder::new(len, true, repeat, rng.next_u64());
        // Play a few entries of the cycle.
        let _ = order.next_index();
        let _ = order.next_index();

        let plan = order.shuffle_plan().to_vec();
        let target_pos = rng.below(len);
        let target = plan[target_pos];
        order.set_current_index(Some(target));

        assert_eq!(order.current_index(), Some(target));
        assert_eq!(
            order.shuffle_plan(),
            &plan[..],
            "the permutation is kept, the cursor moves into it"
        );
        assert_eq!(
            order.shuffle_pos(),
            target_pos,
            "the cursor sits at the selected entry"
        );

        // The traversal continues behind the selected entry.
        if target_pos + 1 < len {
            assert_eq!(
                order.next_index(),
                Some(plan[target_pos + 1]),
                "the traversal continues behind the selected entry"
            );
        } else if repeat {
            let next = order.next_index().expect("repeat continues");
            assert!(next < len);
            assert_permutation(order.shuffle_plan(), len, "the cycle after the selection");
        } else {
            assert_eq!(order.next_index(), None, "the cycle ends");
        }
    }
}

/// Stepping back returns the track that was played before the current one.
#[test]
fn stepping_back_returns_the_previously_played_track() {
    for len in 1..=10usize {
        for repeat in [true, false] {
            for seed in 0..12u64 {
                let mut order =
                    PlaybackOrder::new(len, true, repeat, seed.wrapping_mul(0x9E37_79B9) ^ 1);
                let mut played = Vec::new();
                for _ in 0..len {
                    played.push(order.next_index().expect("the cycle yields a track"));
                }
                // The last track is the current one, so the first step back
                // has to land on the one before it.
                for expected in played.iter().rev().skip(1) {
                    let back = order.prev_index().expect("stepping back yields a track");
                    assert_eq!(
                        back, *expected,
                        "len {len}, repeat {repeat}, seed {seed}: stepping back returned the wrong track"
                    );
                }
            }
        }
    }
}

/// The sequential order steps back to the entry before the current one.
#[test]
fn the_sequential_order_steps_back_to_the_previous_entry() {
    for len in 1..=10usize {
        for repeat in [true, false] {
            let mut order = PlaybackOrder::new(len, false, repeat, 13);
            let mut played = Vec::new();
            for _ in 0..len {
                played.push(order.next_index().expect("the queue yields a track"));
            }
            for expected in played.iter().rev().skip(1) {
                let back = order.prev_index().expect("stepping back yields a track");
                assert_eq!(
                    back, *expected,
                    "len {len}, repeat {repeat}: stepping back returned the wrong entry"
                );
            }
            // At the first entry the order either clamps or wraps around.
            let first = order.prev_index().expect("stepping back yields a track");
            if repeat {
                assert_eq!(first, len - 1, "len {len}: the order must wrap around");
            } else {
                assert_eq!(first, 0, "len {len}: the order must clamp");
            }
        }
    }
}

/// After a length change the next cycle is a permutation of the new length,
/// and after an invalidation the next cycle covers the queue again.
#[test]
fn invalidation_rebuilds_the_cycle_for_the_current_length() {
    for shuffle in [true, false] {
        for repeat in [true, false] {
            for len in 0..=6usize {
                for new_len in 0..=6usize {
                    for seed in [3u64, 17] {
                        let mut order = PlaybackOrder::new(len, shuffle, repeat, seed);
                        // Move away from the start, so the invalidation has a
                        // current track to anchor to.
                        let _ = order.next_index();
                        let _ = order.next_index();

                        order.set_len(new_len);
                        assert_invariants(&order, "after set_len");
                        assert_cycle_covers_the_queue(
                            &mut order,
                            new_len,
                            shuffle,
                            "after set_len",
                        );

                        order.invalidate();
                        assert_invariants(&order, "after invalidate");
                        assert_cycle_covers_the_queue(
                            &mut order,
                            new_len,
                            shuffle,
                            "after invalidate",
                        );
                    }
                }
            }
        }
    }
}

/// Asserts that the cycle the model is about to play stays inside the queue
/// of `len` entries, and that it covers every entry exactly once.
///
/// The shuffled order starts a cycle at the head of its permutation, so the
/// cycle is a permutation of the whole queue. The sequential order continues
/// behind the track that plays, so its cycle is the rest of the queue; only
/// from a stopped state is it the whole queue.
fn assert_cycle_covers_the_queue(order: &mut PlaybackOrder, len: usize, shuffle: bool, what: &str) {
    if shuffle {
        let current = order.current_index();
        let visited = walk_cycle(order);
        for index in &visited {
            assert!(
                *index < len,
                "{what}: the cycle plays the stale index {index} of a queue of {len} entries"
            );
        }
        let mut cycle = visited;
        if let Some(current) = current {
            cycle.push(current);
        }
        assert_permutation(&cycle, len, what);
    } else {
        // The sequential order plays the entries from the current track on.
        // From a stopped state that is the whole queue.
        let current = order.current_index();
        if current.is_none() {
            let mut visited = Vec::new();
            for _ in 0..len {
                let index = order.next_index().expect("the queue yields a track");
                assert!(index < len, "{what}: the stale index {index} was played");
                visited.push(index);
            }
            assert_permutation(&visited, len, what);
        } else {
            for _ in 0..=len {
                if let Some(index) = order.next_index() {
                    assert!(index < len, "{what}: the stale index {index} was played");
                }
            }
        }
    }
}

/// A length change never leaves a stale plan behind: the plan is rebuilt for
/// the new length as soon as the queue changes.
#[test]
fn a_length_change_never_keeps_a_stale_plan() {
    for len in 0..=8usize {
        for new_len in 0..=8usize {
            for seed in [5u64, 99] {
                let mut order = PlaybackOrder::new(len, true, true, seed);
                let _ = order.next_index();
                order.set_len(new_len);
                assert_eq!(
                    order.shuffle_plan().len(),
                    new_len,
                    "len {len} -> {new_len}: the plan was not rebuilt"
                );
                assert_permutation(order.shuffle_plan(), new_len, "the rebuilt plan");

                // Growing the queue again must not resurrect the old plan.
                order.set_len(new_len + 3);
                assert_eq!(order.shuffle_plan().len(), new_len + 3);
                assert_permutation(order.shuffle_plan(), new_len + 3, "the grown plan");
            }
        }
    }
}

/// The delta a skip is clamped to: the documented bound in the direction of
/// the delta.
fn clamped_delta(delta: i32) -> i32 {
    let bound = MAX_SKIP_STEPS as i32;
    match delta {
        d if d > bound => bound,
        d if d < -bound => -bound,
        d => d,
    }
}

/// A delta from the wire cannot turn the walk into an unbounded loop.
///
/// The model clamps the number of steps to [`MAX_SKIP_STEPS`], so even the
/// largest delta returns at once and lands where the clamped semantics say:
/// exactly where the same walk of the clamped delta lands, with the same
/// permutation and the same place in it (S15 §3.3).
#[test]
fn a_huge_delta_is_clamped_to_the_documented_bound() {
    const LEN: usize = 7;
    for shuffle in [true, false] {
        for repeat in [true, false] {
            for delta in [i32::MAX, i32::MIN, 3, -3] {
                let mut clamped = PlaybackOrder::new(LEN, shuffle, repeat, 99);
                clamped.set_current_index(Some(0));
                let expected = clamped.skip_by(clamped_delta(delta));

                let mut huge = PlaybackOrder::new(LEN, shuffle, repeat, 99);
                huge.set_current_index(Some(0));
                let started = Instant::now();
                let answer = huge.skip_by(delta);
                let elapsed = started.elapsed();

                assert_eq!(
                    answer, expected,
                    "shuffle {shuffle}, repeat {repeat}, delta {delta}: the clamped walk lands somewhere else"
                );
                assert_eq!(
                    huge.shuffle_plan(),
                    clamped.shuffle_plan(),
                    "shuffle {shuffle}, repeat {repeat}, delta {delta}: the clamped walk took another number of steps"
                );
                assert_eq!(
                    huge.shuffle_pos(),
                    clamped.shuffle_pos(),
                    "shuffle {shuffle}, repeat {repeat}, delta {delta}: the clamped walk ended somewhere else in the permutation"
                );
                assert!(
                    elapsed < Duration::from_secs(1),
                    "shuffle {shuffle}, repeat {repeat}, delta {delta}: the walk took {elapsed:?}"
                );
            }
        }
    }
}

/// The sequential order walks the queue from the first to the last entry and
/// wraps around only with repeat.
#[test]
fn the_sequential_order_runs_in_queue_order() {
    for len in 0..=8usize {
        for repeat in [true, false] {
            let mut order = PlaybackOrder::new(len, false, repeat, 11);
            let mut visited = Vec::new();
            for _ in 0..len {
                let index = order
                    .next_index()
                    .unwrap_or_else(|| panic!("len {len}, repeat {repeat}: the queue ended early"));
                visited.push(index);
            }
            if len == 0 {
                assert!(visited.is_empty());
                assert_eq!(order.next_index(), None, "an empty queue yields no track");
                continue;
            }
            assert_eq!(visited, (0..len).collect::<Vec<_>>());
            if repeat {
                assert_eq!(
                    order.next_index(),
                    Some(0),
                    "the sequential order wraps around with repeat"
                );
            } else {
                assert_eq!(
                    order.next_index(),
                    None,
                    "the queue ends after the last entry without repeat"
                );
            }
        }
    }
}
