//! Integration tests for shuffle and repeat playback modes (S15).
//!
//! Tests cover:
//! 1. All four mode combinations (Seq+NoRep, Seq+Rep, Shuf+NoRep, Shuf+Rep)
//! 2. Queue end behavior under each mode
//! 3. Manual advance (Next, Prev, SkipBy) under each mode
//! 4. Mid-run toggling of shuffle and repeat
//! 5. Single-entry queue behavior
//!
//! The traversal itself is the [`rustamp_playlist::PlaybackOrder`] model's
//! job; these tests pin what the *engine* does with it: that the engine asks
//! the model for the next track, that the published permutation is the one
//! that is traversed, and that the published modes are the ones the model
//! holds.

mod common;

use std::time::Duration;

use common::{Harness, WAIT, track_frames};
use rustamp_core::event::Event;
use rustamp_core::state::TransportState;

const SHORT_A: &str = "track1_1000.tst";
const SHORT_B: &str = "track2_1000.tst";
const SHORT_C: &str = "track3_1000.tst";

const LONG_A: &str = "track1_44100.tst";
const LONG_B: &str = "track2_44100.tst";
const LONG_C: &str = "track3_44100.tst";
const LONG_D: &str = "track4_44100.tst";

/// Frames a manual advance may consume before the test gives up.
///
/// The long tracks are a second long, and the ring holds a fraction of that,
/// so a track cannot end — and the automatic advance cannot happen — within
/// this budget. What the test sees is therefore the *manual* advance.
const MANUAL_BUDGET: u64 = 8192;

/// Wait until the engine's current track is `index`, pumping at most
/// [`MANUAL_BUDGET`] frames.
fn wait_manual_advance(h: &mut Harness, index: u32) -> bool {
    let mut pumped = 0u64;
    while pumped <= MANUAL_BUDGET {
        h.engine.poll().unwrap();
        if h.engine.current_index() == Some(index) {
            return true;
        }
        h.clock.pump(256);
        pumped += 256;
        std::thread::sleep(Duration::from_millis(1));
    }
    false
}

#[test]
fn mode_sequential_no_repeat_queue_end() {
    let mut h = Harness::new(&[SHORT_A, SHORT_B]);
    h.engine.set_shuffle(false).unwrap();
    h.engine.set_repeat(false).unwrap();
    assert!(!h.engine.shuffle());
    assert!(!h.engine.repeat());

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    // Wait until both tracks have buffered into the pipeline
    assert!(h.wait_buffered());

    let total = track_frames(SHORT_A) + track_frames(SHORT_B);
    assert_eq!(h.engine.buffered_frames(), total);

    // Pump all audio frames through the virtual device
    h.clock.pump(total);

    // Playback must stop at queue end
    assert!(h.wait_idle(
        |e| e.transport() == TransportState::Stopped,
        Duration::from_secs(2)
    ));

    let events = h.drain_events();
    assert!(
        events.contains(&Event::PlaylistEnded),
        "Sequential No-Repeat must publish PlaylistEnded at queue end: {events:?}"
    );
    assert!(
        events.contains(&Event::StateChanged(TransportState::Stopped)),
        "Sequential No-Repeat must transition to Stopped at queue end"
    );
}

#[test]
fn mode_sequential_repeat_loops_indefinitely() {
    let mut h = Harness::new(&[SHORT_A, SHORT_B]);
    h.engine.set_shuffle(false).unwrap();
    h.engine.set_repeat(true).unwrap();
    assert!(h.engine.repeat());

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    let total = track_frames(SHORT_A) + track_frames(SHORT_B);

    // Pump through the entire first cycle
    h.clock.pump(total);
    h.engine.poll().unwrap();

    // Under repeat, transport MUST NOT stop after total frames
    assert_eq!(h.engine.transport(), TransportState::Playing);
    let events = h.drain_events();
    assert!(
        !events.contains(&Event::PlaylistEnded),
        "Sequential Repeat must not emit PlaylistEnded at end of cycle: {events:?}"
    );

    // Pump more frames into cycle 2: track 0 must play again
    h.clock.pump(track_frames(SHORT_A));
    h.engine.poll().unwrap();
    assert_eq!(h.engine.transport(), TransportState::Playing);
}

#[test]
fn mode_shuffle_no_repeat_queue_end() {
    let mut h = Harness::new(&[SHORT_A, SHORT_B, SHORT_C]);
    h.engine.set_shuffle_seed(12345);
    h.engine.set_shuffle(true).unwrap();
    h.engine.set_repeat(false).unwrap();
    assert!(h.engine.shuffle());
    assert!(!h.engine.repeat());

    h.engine.play().unwrap();
    let plan = h.engine.shuffle_plan().to_vec();
    assert_eq!(plan.len(), 3);

    // Wait until all tracks are buffered
    assert!(h.wait_buffered());

    let total = track_frames(SHORT_A) + track_frames(SHORT_B) + track_frames(SHORT_C);
    h.clock.pump(total);

    // Playback must stop at the end of the shuffled cycle
    assert!(h.wait_idle(
        |e| e.transport() == TransportState::Stopped,
        Duration::from_secs(2)
    ));

    let events = h.drain_events();
    assert!(
        events.contains(&Event::PlaylistEnded),
        "Shuffle No-Repeat must emit PlaylistEnded after all tracks in permutation play"
    );
    assert_eq!(h.engine.transport(), TransportState::Stopped);
}

#[test]
fn mode_shuffle_repeat_anti_immediate_repeat() {
    let mut h = Harness::new(&[SHORT_A, SHORT_B, SHORT_C]);
    h.engine.set_shuffle_seed(42);
    h.engine.set_shuffle(true).unwrap();
    h.engine.set_repeat(true).unwrap();

    h.engine.play().unwrap();
    let cycle1_plan = h.engine.shuffle_plan().to_vec();
    assert_eq!(cycle1_plan.len(), 3);

    let total = track_frames(SHORT_A) + track_frames(SHORT_B) + track_frames(SHORT_C);

    // Pump through cycle 1
    h.clock.pump(total);
    h.engine.poll().unwrap();

    // Must still be playing
    assert_eq!(h.engine.transport(), TransportState::Playing);
    let events = h.drain_events();
    assert!(
        !events.contains(&Event::PlaylistEnded),
        "Shuffle Repeat must not emit PlaylistEnded at cycle boundary"
    );
}

#[test]
fn manual_advance_sequential_modes() {
    let mut h = Harness::new(&[LONG_A, LONG_B, LONG_C]);
    h.engine.set_shuffle(false).unwrap();
    h.engine.set_repeat(false).unwrap();

    // Start at 0
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    // Next -> 1
    h.engine.next_track().unwrap();
    assert!(h.wait_track(1));

    // Next -> 2
    h.engine.next_track().unwrap();
    assert!(h.wait_track(2));

    // Prev -> 1
    h.engine.prev_track().unwrap();
    assert!(h.wait_track(1));

    // Prev -> 0
    h.engine.prev_track().unwrap();
    assert!(h.wait_track(0));

    // Prev at start with repeat off stays at 0
    h.engine.prev_track().unwrap();
    assert_eq!(h.engine.current_index(), Some(0));

    // Now enable repeat and test wrapping
    h.engine.set_repeat(true).unwrap();

    // Prev at 0 with repeat wraps to last track (2)
    h.engine.prev_track().unwrap();
    assert!(h.wait_track(2));

    // Next at 2 with repeat wraps to first track (0)
    h.engine.next_track().unwrap();
    assert!(h.wait_track(0));
}

#[test]
fn manual_advance_shuffle_history() {
    let mut h = Harness::new(&[LONG_A, LONG_B, LONG_C, LONG_D]);
    h.engine.set_shuffle_seed(999);
    h.engine.set_shuffle(true).unwrap();
    h.engine.set_repeat(false).unwrap();

    h.engine.play().unwrap();
    let plan = h.engine.shuffle_plan().to_vec();
    assert_eq!(plan.len(), 4);

    let t0 = plan[0] as u32;
    let t1 = plan[1] as u32;
    let t2 = plan[2] as u32;

    assert!(h.wait_track(t0));

    // Next follows shuffle plan
    h.engine.next_track().unwrap();
    assert!(h.wait_track(t1));

    h.engine.next_track().unwrap();
    assert!(h.wait_track(t2));

    // Prev walks back the permutation history (t2 -> t1 -> t0)
    h.engine.prev_track().unwrap();
    assert!(h.wait_track(t1));

    h.engine.prev_track().unwrap();
    assert!(h.wait_track(t0));
}

#[test]
fn toggle_shuffle_and_repeat_mid_run() {
    let mut h = Harness::new(&[LONG_A, LONG_B, LONG_C, LONG_D]);
    h.engine.set_shuffle(false).unwrap();
    h.engine.set_repeat(false).unwrap();

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    // Toggle shuffle ON mid-run while playing track 0
    h.engine.toggle_shuffle().unwrap();
    assert!(h.engine.shuffle());

    let events = h.drain_events();
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::ModesChanged {
                shuffle: true,
                repeat: false
            }
        )),
        "ModesChanged event must be emitted on shuffle toggle"
    );

    // Next must pick one of the remaining unplayed tracks [1, 2, 3]. The
    // skip is asynchronous (the worker abandons track 0 and opens the next
    // one before its start is published), so wait for the engine to land on
    // the new track instead of reading the index right after the command.
    h.engine.next_track().unwrap();
    assert!(
        h.wait_pumping(
            |e| matches!(e.current_index(), Some(i) if (1..=3).contains(&i)),
            WAIT
        ),
        "mid-run shuffle toggle must advance to an unplayed track"
    );
    let next_idx = h.engine.current_index().expect("current track index");
    assert!(
        [1, 2, 3].contains(&next_idx),
        "Mid-run shuffle plan must visit remaining unplayed tracks: got {next_idx}"
    );

    // Toggle shuffle OFF mid-run
    h.engine.toggle_shuffle().unwrap();
    assert!(!h.engine.shuffle());

    // Toggle repeat mid-run
    h.engine.toggle_repeat().unwrap();
    assert!(h.engine.repeat());

    let events = h.drain_events();
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::ModesChanged {
                shuffle: false,
                repeat: true
            }
        )),
        "ModesChanged event must be emitted on repeat toggle"
    );
}

#[test]
fn single_entry_queue_all_modes() {
    // 1. Single entry, No repeat, Sequential
    {
        let mut h = Harness::new(&[SHORT_A]);
        h.engine.set_shuffle(false).unwrap();
        h.engine.set_repeat(false).unwrap();

        h.engine.play_index(0).unwrap();
        assert!(h.wait_buffered());
        h.clock.pump(track_frames(SHORT_A));

        assert!(h.wait_idle(
            |e| e.transport() == TransportState::Stopped,
            Duration::from_secs(2)
        ));
        let events = h.drain_events();
        assert!(events.contains(&Event::PlaylistEnded));
    }

    // 2. Single entry, Repeat, Sequential
    {
        let mut h = Harness::new(&[SHORT_A]);
        h.engine.set_shuffle(false).unwrap();
        h.engine.set_repeat(true).unwrap();

        h.engine.play_index(0).unwrap();
        h.clock.pump(track_frames(SHORT_A));
        h.engine.poll().unwrap();

        assert_eq!(h.engine.transport(), TransportState::Playing);
        assert_eq!(h.engine.current_index(), Some(0));
    }

    // 3. Single entry, Shuffle, No Repeat
    {
        let mut h = Harness::new(&[SHORT_A]);
        h.engine.set_shuffle(true).unwrap();
        h.engine.set_repeat(false).unwrap();

        h.engine.play_index(0).unwrap();
        assert!(h.wait_buffered());
        h.clock.pump(track_frames(SHORT_A));

        assert!(h.wait_idle(
            |e| e.transport() == TransportState::Stopped,
            Duration::from_secs(2)
        ));
        let events = h.drain_events();
        assert!(events.contains(&Event::PlaylistEnded));
    }

    // 4. Single entry, Shuffle, Repeat
    {
        let mut h = Harness::new(&[SHORT_A]);
        h.engine.set_shuffle(true).unwrap();
        h.engine.set_repeat(true).unwrap();

        h.engine.play_index(0).unwrap();
        h.clock.pump(track_frames(SHORT_A));
        h.engine.poll().unwrap();

        assert_eq!(h.engine.transport(), TransportState::Playing);
        assert_eq!(h.engine.current_index(), Some(0));
    }
}

#[test]
fn shuffle_repeat_prev_across_cycle_boundary() {
    let mut h = Harness::new(&[SHORT_A, SHORT_B, SHORT_C]);
    h.engine.set_shuffle(true).unwrap();
    h.engine.set_repeat(true).unwrap();

    let plan1 = h.engine.shuffle_plan().to_vec();
    assert_eq!(plan1.len(), 3);

    // Start playback on first track of cycle 1 (S15 §1.1)
    h.engine.play().unwrap();
    assert!(h.wait_track(plan1[0] as u32));

    // Advance through the remaining tracks of cycle 1: pos 1, then pos 2
    h.engine.next_track().unwrap();
    assert!(h.wait_pumping(|e| e.current_index() == Some(plan1[1] as u32), WAIT));

    h.engine.next_track().unwrap();
    assert!(h.wait_pumping(|e| e.current_index() == Some(plan1[2] as u32), WAIT));

    // Next track wraps across the cycle boundary to cycle 2 track 0
    h.engine.next_track().unwrap();
    assert!(h.wait_pumping(
        |e| {
            let plan2 = e.shuffle_plan();
            plan2.len() == 3 && e.current_index() == Some(plan2[0] as u32)
        },
        WAIT
    ));

    let plan2 = h.engine.shuffle_plan().to_vec();
    assert_eq!(h.engine.current_index(), Some(plan2[0] as u32));

    // Now call prev_track() while at track 0 of cycle 2:
    // S15 §3.2 requires jumping to the last track of cycle 1 (plan1[2])!
    h.engine.prev_track().unwrap();
    assert!(h.wait_pumping(
        |e| e.current_index() == Some(plan1[2] as u32) && e.shuffle_plan() == plan1,
        WAIT
    ));
    assert_eq!(
        h.engine.shuffle_plan(),
        plan1,
        "shuffle plan must be restored to cycle 1"
    );
}

#[test]
fn regression_late_repeat_enablement_at_queue_end() {
    let mut h = Harness::new(&[SHORT_A, SHORT_B]);
    h.engine.set_shuffle(false).unwrap();
    h.engine.set_repeat(false).unwrap();

    h.engine.play_index(0).unwrap();

    let total = track_frames(SHORT_A) + track_frames(SHORT_B);
    // A fast worker can prefetch both tracks before the first poll, so the
    // decoder's current_index may already be 1: wait_track(0) can miss it.
    // This regression needs EOS with the entire first cycle still unconsumed,
    // not the next cycle's audio (repeat is still off). Never pump this wait.
    assert!(h.wait_idle(
        |e| e.buffered_frames() == total && e.end_of_stream() == Some(total),
        WAIT
    ));
    assert_eq!(h.engine.ring_counters(), (total, 0));
    assert_eq!(h.engine.current_index(), Some(1));
    assert_eq!(h.engine.audible_index(), Some(0));

    // Audio is still unconsumed in the ring (buffered_frames == total).
    // Now toggle repeat ON: this MUST revoke EOS and keep the run alive.
    h.engine.set_repeat(true).unwrap();
    assert!(h.engine.repeat());
    assert!(h.engine.end_of_stream().is_none());

    // Pump through the remaining frames of the first cycle
    h.clock.pump(total);
    h.engine.poll().unwrap();

    // The defect would have terminated the worker, so the engine would have
    // stopped playback once the buffer drained.
    // With the fix, repeat is honoured: transport stays Playing and loops.
    assert_eq!(
        h.engine.transport(),
        TransportState::Playing,
        "Transport must remain Playing after buffer drain when repeat was re-enabled"
    );

    // Pump into cycle 2: track 0 must play again
    assert!(h.wait_pumping(
        |e| e.transport() == TransportState::Playing && e.current_index() == Some(0),
        WAIT
    ));
}

#[test]
fn regression_late_repeat_disablement_near_cycle_boundary() {
    let mut h = Harness::new(&[SHORT_A, SHORT_B]);
    h.engine.set_shuffle(false).unwrap();
    h.engine.set_repeat(true).unwrap();

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    // Under repeat, decoder decodes track A, track B, and starts prebuffering Cycle 2 track A.
    let len_a = track_frames(SHORT_A);
    let len_b = track_frames(SHORT_B);
    assert!(h.wait_idle(|e| e.buffered_frames() > len_a + len_b, WAIT));

    // Pump only track A so track B is currently playing/audible.
    h.clock.pump(len_a);
    h.engine.poll().unwrap();
    assert_eq!(h.engine.audible_index(), Some(1));

    // Now turn repeat OFF while track B of Cycle 1 is still audible
    h.engine.set_repeat(false).unwrap();
    assert!(!h.engine.repeat());

    // Pump the rest of Cycle 1 (track B)
    h.clock.pump(len_b);

    // Playback must stop cleanly at the end of Cycle 1 (S15 §4.3)
    assert!(h.wait_idle(|e| e.transport() == TransportState::Stopped, WAIT));
    let events = h.drain_events();
    assert!(
        events.contains(&Event::PlaylistEnded),
        "Turning repeat off before cycle end must stop playback with PlaylistEnded: {events:?}"
    );
}

#[test]
fn regression_shuffle_mid_run_replan_anchoring_to_audible_track() {
    let mut h = Harness::new(&[SHORT_A, SHORT_B, SHORT_C]);
    h.engine.set_shuffle(false).unwrap();
    h.engine.set_repeat(false).unwrap();

    h.engine.play_index(0).unwrap();

    let total = track_frames(SHORT_A) + track_frames(SHORT_B) + track_frames(SHORT_C);
    // Do not wait for the transient decoder index 0: a fast worker may have
    // already prefetched track 2 before the first poll. The discriminator is
    // decoder index 2 versus audible index 0, with all audio still in the ring.
    // Unlike late repeat enablement, this replan does not require EOS.
    assert!(h.wait_idle(
        |e| e.buffered_frames() == total && e.current_index() == Some(2),
        WAIT
    ));
    assert_eq!(h.engine.ring_counters(), (total, 0));

    // Device has consumed 0 frames, so Track 0 is currently audible
    assert_eq!(h.engine.audible_index(), Some(0));

    // Enable shuffle mid-run: S15 §4.1 mandates that track K (the audible track 0)
    // is set as the first entry (pos 0) in the active shuffle plan.
    h.engine.set_shuffle(true).unwrap();
    let plan = h.engine.shuffle_plan();
    assert_eq!(plan.len(), 3);
    assert_eq!(
        plan[0], 0,
        "Shuffle replan must anchor to audible track 0, not decoder prefetched track"
    );
}

#[test]
fn regression_single_track_repeat_toggle_during_drain() {
    let mut h = Harness::new(&[SHORT_A]);
    h.engine.set_shuffle(false).unwrap();
    h.engine.set_repeat(false).unwrap();

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    // Wait until track 0 is fully decoded into the ring
    assert!(h.wait_buffered());
    let len = track_frames(SHORT_A);
    assert_eq!(h.engine.buffered_frames(), len);

    // Toggle repeat ON before ring drains
    h.engine.set_repeat(true).unwrap();
    assert!(h.engine.repeat());
    assert!(h.engine.end_of_stream().is_none());

    // Pump past the end of the first iteration
    h.clock.pump(len);
    h.engine.poll().unwrap();

    // Transport must not stop; it must continue playing track 0
    assert_eq!(
        h.engine.transport(),
        TransportState::Playing,
        "Single-track queue must loop when repeat is turned on during drain"
    );
    assert!(h.wait_pumping(
        |e| e.transport() == TransportState::Playing && e.current_index() == Some(0),
        WAIT
    ));
}

// --- The engine drives the S15 order model ---

/// The permutation the engine publishes is the permutation the traversal
/// follows (S15 §1.1, §3.1): the engine asks the model for the next track
/// instead of computing an index of its own.
#[test]
fn shuffle_traversal_follows_the_published_plan() {
    let mut h = Harness::new(&[LONG_A, LONG_B, LONG_C, LONG_D]);
    h.engine.set_shuffle_seed(2024);
    h.engine.set_shuffle(true).unwrap();
    h.engine.set_repeat(false).unwrap();

    let plan = h.engine.shuffle_plan();
    assert_eq!(plan.len(), 4);
    let mut sorted = plan.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![0, 1, 2, 3], "the plan is a permutation");
    assert_ne!(plan, vec![0, 1, 2, 3], "the plan is not the queue order");

    h.engine.play().unwrap();
    assert!(
        h.wait_track(plan[0] as u32),
        "the first entry of the permutation plays first"
    );

    for want in plan.iter().skip(1) {
        h.engine.next_track().unwrap();
        let want = *want as u32;
        assert!(
            wait_manual_advance(&mut h, want),
            "the manual advance must follow the permutation to {want}"
        );
    }
}

/// Repeat changes the order the engine selects: at the end of the queue the
/// run ends without it (S15 §2) and wraps to the first track with it
/// (S15 §3.1).
#[test]
fn repeat_changes_the_live_selection_at_the_queue_end() {
    // Without repeat the run ends at the last track.
    {
        let mut h = Harness::new(&[LONG_A, LONG_B]);
        h.engine.set_shuffle(false).unwrap();
        h.engine.set_repeat(false).unwrap();

        h.engine.play_index(0).unwrap();
        assert!(h.wait_track(0));
        h.engine.next_track().unwrap();
        assert!(h.wait_track(1));

        h.engine.next_track().unwrap();
        assert!(
            h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
            "the run must end at the queue end without repeat"
        );
        let events = h.drain_events();
        assert!(
            events.contains(&Event::PlaylistEnded),
            "the run must publish PlaylistEnded: {events:?}"
        );
    }

    // With repeat the traversal wraps to the first track.
    {
        let mut h = Harness::new(&[LONG_A, LONG_B]);
        h.engine.set_shuffle(false).unwrap();
        h.engine.set_repeat(true).unwrap();

        h.engine.play_index(0).unwrap();
        assert!(h.wait_track(0));
        h.engine.next_track().unwrap();
        assert!(h.wait_track(1));

        h.engine.next_track().unwrap();
        assert!(
            wait_manual_advance(&mut h, 0),
            "repeat must wrap the manual advance to the first track"
        );
    }
}

/// A shuffle toggle mid-run re-plans the permutation around the track that
/// is playing, and the traversal follows the new plan (S15 §4.1).
#[test]
fn a_mid_run_shuffle_toggle_is_honoured() {
    let mut h = Harness::new(&[LONG_A, LONG_B, LONG_C, LONG_D]);
    h.engine.set_shuffle(false).unwrap();
    h.engine.set_repeat(false).unwrap();

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    // Nothing has been consumed yet, so track 0 is the audible one.
    assert_eq!(h.engine.audible_index(), Some(0));

    h.engine.set_shuffle(true).unwrap();
    // The published state follows the model, so the light and the traversal
    // cannot disagree.
    assert!(h.engine.shuffle());
    assert!(h.engine.state().shuffle);

    let plan = h.engine.shuffle_plan();
    assert_eq!(plan.len(), 4);
    assert_eq!(
        plan[0], 0,
        "the new permutation is anchored at the audible track"
    );
    assert_ne!(
        plan,
        vec![0, 1, 2, 3],
        "the new permutation is shuffled, not the queue order"
    );
    let mut sorted = plan.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![0, 1, 2, 3], "the new plan is a permutation");

    h.engine.next_track().unwrap();
    let want = plan[1] as u32;
    assert!(
        wait_manual_advance(&mut h, want),
        "the manual advance must follow the new permutation to {want}"
    );
}

/// A single track with repeat on loops: the same track plays again and the
/// run never ends (S15 §6.1).
#[test]
fn repeat_of_one_loops_the_same_track() {
    let mut h = Harness::new(&[SHORT_A]);
    h.engine.set_shuffle(false).unwrap();
    h.engine.set_repeat(true).unwrap();

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    let len = track_frames(SHORT_A);
    // The decoder wraps to the same track, so the ring holds more than one
    // iteration of it.
    assert!(
        h.wait_pumping(|e| e.buffered_frames() > len, WAIT),
        "the single track must loop"
    );

    h.clock.pump(len);
    h.engine.poll().unwrap();
    assert_eq!(
        h.engine.transport(),
        TransportState::Playing,
        "the run must not end while repeat is on"
    );
    assert_eq!(h.engine.current_index(), Some(0));
    let events = h.drain_events();
    assert!(
        !events.contains(&Event::PlaylistEnded),
        "a looping run must not publish PlaylistEnded: {events:?}"
    );
}
