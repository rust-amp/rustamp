//! Integration tests for the output device mid-run error hook (RFC 0012).
//!
//! Tests headless detection and handling of fatal output device mid-run failures:
//! - A failure is observed by the engine within one poll
//! - The run ends cleanly with transport transitioning to Stopped
//! - Event::StateChanged(TransportState::Stopped) and Event::Error are published
//! - The failure is reported once, not per poll
//! - A healthy device is unaffected
//! - Position freezes at the point of failure
//! - Resume while paused detects device loss and stops cleanly

mod common;

use common::Harness;
use rustamp_core::error::ErrorKind;
use rustamp_core::event::Event;
use rustamp_core::state::TransportState;

const LONG_TRACK: &str = "track1_44100.tst";

#[test]
fn engine_stops_and_publishes_error_when_output_fails_mid_run() {
    let mut h = Harness::new(&[LONG_TRACK]);
    h.engine.play().expect("playback start");
    assert!(h.wait_track(0), "track opens");

    // Pump audio frames and verify playback is active.
    let pumped = h.clock.pump(2048);
    assert_eq!(pumped, 2048);
    h.engine.poll().expect("poll");
    assert_eq!(h.engine.transport(), TransportState::Playing);
    let _ = h.drain_events();

    // Trigger mid-run device failure.
    h.clock.fail();
    assert_eq!(
        h.engine.transport(),
        TransportState::Playing,
        "engine still reports Playing before poll"
    );

    // Call poll: engine observes device failure within this single poll.
    h.engine.poll().expect("poll after failure");

    // Transport transitions to Stopped.
    assert_eq!(h.engine.transport(), TransportState::Stopped);

    // Drained events must contain StateChanged(Stopped), PositionChanged, and Error(Output).
    let events = h.drain_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::StateChanged(TransportState::Stopped))),
        "StateChanged(Stopped) missing: {events:?}"
    );

    let error_event = events
        .iter()
        .find(|e| matches!(e, Event::Error(info) if info.kind == ErrorKind::Output));
    assert!(
        error_event.is_some(),
        "Event::Error(Output) missing: {events:?}"
    );
    if let Some(Event::Error(info)) = error_event {
        assert!(
            info.message.contains("simulated device failure"),
            "error message should contain simulated failure details: {}",
            info.message
        );
    }

    // Engine error status is accessible.
    assert!(h.engine.last_error().is_some());
    assert_eq!(h.engine.last_error().unwrap().kind, ErrorKind::Output);

    // Clock and position are frozen: further clock pumping does not advance.
    let frozen_pos = h.engine.position_ms();
    let frozen_frames = h.clock.frames();
    assert_eq!(h.clock.pump(1024), 0, "pump on failed device returns 0");
    assert_eq!(h.clock.frames(), frozen_frames);
    assert_eq!(h.engine.position_ms(), frozen_pos);
}

#[test]
fn failure_is_reported_once_not_per_poll() {
    let mut h = Harness::new(&[LONG_TRACK]);
    h.engine.play().expect("playback start");
    assert!(h.wait_track(0), "track opens");

    h.clock.pump(1024);
    h.engine.poll().expect("poll");
    let _ = h.drain_events();

    // Trigger failure.
    h.clock.fail();

    // First poll catches the failure and publishes the error.
    h.engine.poll().expect("first poll");
    let events = h.drain_events();
    let error_count = events
        .iter()
        .filter(|e| matches!(e, Event::Error(_)))
        .count();
    assert_eq!(error_count, 1, "failure must be reported in first poll");

    // Subsequent polls must NOT report the failure again.
    for i in 2..=5 {
        h.engine.poll().expect("subsequent poll");
        let later_events = h.drain_events();
        assert!(
            !later_events.iter().any(|e| matches!(e, Event::Error(_))),
            "poll {i} published unexpected error: {later_events:?}"
        );
        assert!(
            !later_events
                .iter()
                .any(|e| matches!(e, Event::StateChanged(_))),
            "poll {i} published unexpected state change: {later_events:?}"
        );
    }
}

#[test]
fn healthy_device_is_unaffected() {
    let mut h = Harness::new(&[LONG_TRACK]);
    h.engine.play().expect("playback start");
    assert!(h.wait_track(0), "track opens");

    // Multiple pumps and polls with a healthy device.
    for _ in 0..5 {
        let n = h.clock.pump(512);
        assert_eq!(n, 512);
        h.engine.poll().expect("poll");
        assert_eq!(h.engine.transport(), TransportState::Playing);
    }

    let events = h.drain_events();
    assert!(
        !events.iter().any(|e| matches!(e, Event::Error(_))),
        "healthy device must never publish Event::Error: {events:?}"
    );
    assert!(h.engine.last_error().is_none());
}

#[test]
fn failure_observed_within_one_poll() {
    let mut h = Harness::new(&[LONG_TRACK]);
    h.engine.play().expect("playback start");
    assert!(h.wait_track(0), "track opens");
    h.clock.pump(1024);
    h.engine.poll().expect("poll");
    let _ = h.drain_events();

    // Trigger failure and assert immediately that transport is STILL Playing prior to poll.
    h.clock.fail();
    assert_eq!(h.engine.transport(), TransportState::Playing);
    assert!(h.engine.last_error().is_none());

    // Exactly one poll.
    h.engine.poll().expect("single poll");

    // Immediately after that single poll, the engine must have reacted.
    assert_eq!(h.engine.transport(), TransportState::Stopped);
    assert!(h.engine.last_error().is_some());
}

#[test]
fn device_failure_while_paused_prevents_resume() {
    let mut h = Harness::new(&[LONG_TRACK]);
    h.engine.play().expect("playback start");
    assert!(h.wait_track(0), "track opens");
    h.clock.pump(1024);
    h.engine.poll().expect("poll");

    // Pause playback.
    h.engine.pause().expect("pause");
    assert_eq!(h.engine.transport(), TransportState::Paused);
    let _ = h.drain_events();

    // Device fails while paused.
    h.clock.fail();

    // Attempting to resume (via play) detects the dead device and transitions to Stopped.
    h.engine.play().expect("play while paused with dead device");
    assert_eq!(h.engine.transport(), TransportState::Stopped);

    let events = h.drain_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::StateChanged(TransportState::Stopped))),
        "StateChanged(Stopped) expected: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Error(info) if info.kind == ErrorKind::Output)),
        "Error(Output) expected: {events:?}"
    );
}

#[test]
fn device_failure_before_play_errors_immediately() {
    let mut h = Harness::new(&[LONG_TRACK]);
    assert_eq!(h.engine.transport(), TransportState::Stopped);

    // Fail the device before starting playback.
    h.clock.fail();

    // play() should immediately return Err and publish Event::Error.
    let result = h.engine.play();
    assert!(result.is_err());
    assert_eq!(h.engine.transport(), TransportState::Stopped);

    let events = h.drain_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Error(info) if info.kind == ErrorKind::Output)),
        "Error(Output) expected: {events:?}"
    );
}
