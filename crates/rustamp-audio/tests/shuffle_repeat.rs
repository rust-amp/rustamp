//! Integration tests for shuffle and repeat playback modes (S15).
//!
//! Tests cover:
//! 1. All four mode combinations (Seq+NoRep, Seq+Rep, Shuf+NoRep, Shuf+Rep)
//! 2. Queue end behavior under each mode
//! 3. Manual advance (Next, Prev, SkipBy) under each mode
//! 4. Mid-run toggling of shuffle and repeat
//! 5. Single-entry queue behavior

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
