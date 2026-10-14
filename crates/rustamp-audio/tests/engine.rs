//! Engine behaviour: the transport state machine and its events, gapless joins,
//! frame-accurate seeking, volume/balance and the vis tap.
//!
//! Every test drives a virtual device ([`rustamp_output::NullOutput`]) so that
//! time, and therefore the audio clock, is under the test's control.

mod common;

use std::time::{Duration, Instant};

use common::{Harness, SPEC, expected, pattern, track_frames};
use rustamp_core::bus::command_channel;
use rustamp_core::command::Command;
use rustamp_core::event::Event;
use rustamp_core::state::TransportState;

const TRACK_A: &str = "track1_3000.tst";
const TRACK_B: &str = "track2_3000.tst";
/// Long enough that the decode thread never reaches the end during a test.
const LONG_A: &str = "track1_44100.tst";
const LONG_B: &str = "track2_44100.tst";

fn frames(ms: u64) -> u64 {
    ms * u64::from(SPEC.sample_rate) / 1000
}

#[test]
fn transport_state_machine_and_events() {
    // Two long tracks: the decode thread cannot finish them, so the queue stays
    // alive for `Next`.
    let mut h = Harness::new(&[LONG_A, LONG_B]);
    assert_eq!(h.engine.transport(), TransportState::Stopped);
    assert_eq!(h.engine.position_frames(), 0);

    // Play: the state changes immediately, the track report follows.
    h.engine.play_index(0).unwrap();
    assert_eq!(h.engine.transport(), TransportState::Playing);
    assert!(h.wait_track(0), "the first track must open");
    let events = h.drain_events();
    assert_eq!(
        events.first(),
        Some(&Event::StateChanged(TransportState::Playing))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::TrackChanged { index: Some(0), .. })),
        "TrackChanged for index 0: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::StreamInfoChanged(Some(info)) if info.sample_rate_hz == SPEC.sample_rate
        )),
        "StreamInfoChanged: {events:?}"
    );
    assert_eq!(h.engine.state().current_index, Some(0));
    assert_eq!(h.engine.duration_frames(), Some(track_frames(LONG_A)));

    // The audio clock follows the device.
    assert_eq!(h.clock.pump(1000), 1000);
    h.engine.poll().unwrap();
    assert_eq!(h.engine.position_frames(), 1000);

    // Pause freezes the clock.
    h.engine.pause().unwrap();
    assert_eq!(h.engine.transport(), TransportState::Paused);
    assert_eq!(h.clock.pump(500), 0, "a paused device pulls nothing");
    assert_eq!(h.engine.position_frames(), 1000);

    // Resume continues from there.
    h.engine.play().unwrap();
    assert_eq!(h.engine.transport(), TransportState::Playing);
    h.clock.pump(500);
    h.engine.poll().unwrap();
    assert_eq!(h.engine.position_frames(), 1500);

    // Seek: frame-exact, and a PositionChanged carries it.
    assert!(h.seek_and_settle(2000), "seek did not land on frame 2000");
    assert_eq!(h.engine.position_frames(), 2000);
    assert_eq!(
        h.engine.position_ms(),
        2000 * 1000 / u64::from(SPEC.sample_rate)
    );
    let events = h.drain_events();
    assert!(
        events.iter().any(|e| matches!(
            e,
            Event::PositionChanged { position_ms, .. } if *position_ms == 2000 * 1000 / u64::from(SPEC.sample_rate)
        )),
        "PositionChanged after the seek: {events:?}"
    );

    // Next: the decode thread moves on and reports the new track.
    h.engine.next_track().unwrap();
    assert!(h.wait_track(1), "the second track must open");
    assert_eq!(h.engine.current_index(), Some(1));
    let events = h.drain_events();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::TrackChanged { index: Some(1), .. })),
        "TrackChanged for index 1: {events:?}"
    );

    // Stop resets the position and says so.
    h.engine.stop().unwrap();
    assert_eq!(h.engine.transport(), TransportState::Stopped);
    assert_eq!(h.engine.position_frames(), 0);
    let events = h.drain_events();
    assert_eq!(
        events.first(),
        Some(&Event::StateChanged(TransportState::Stopped))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::PositionChanged { position_ms: 0, .. })),
        "PositionChanged(0): {events:?}"
    );
}

#[test]
fn commands_drive_the_transport() {
    let mut h = Harness::new(&[LONG_A]);
    let (tx, rx) = command_channel();

    tx.send(Command::Play).unwrap();
    assert_eq!(h.engine.drain_commands(&rx).unwrap(), 1);
    assert!(h.wait_track(0));

    tx.send(Command::SetVolume(128)).unwrap();
    tx.send(Command::SetBalance(-127)).unwrap();
    h.engine.drain_commands(&rx).unwrap();
    assert_eq!(h.engine.state().volume, 128);
    assert_eq!(h.engine.state().balance, -127);
    let events = h.drain_events();
    assert!(events.iter().any(|e| matches!(
        e,
        Event::VolumeChanged {
            volume: 128,
            balance: -127
        }
    )));

    tx.send(Command::SeekRelative { delta_ms: 20 }).unwrap();
    h.engine.drain_commands(&rx).unwrap();
    // The command went through the engine; settle the same way a direct seek
    // does.
    assert!(
        h.settle_seek(frames(20)),
        "SeekRelative landed on {} instead of {}",
        h.engine.position_frames(),
        frames(20)
    );

    tx.send(Command::TogglePause).unwrap();
    h.engine.drain_commands(&rx).unwrap();
    assert_eq!(h.engine.transport(), TransportState::Paused);
    tx.send(Command::TogglePause).unwrap();
    h.engine.drain_commands(&rx).unwrap();
    assert_eq!(h.engine.transport(), TransportState::Playing);

    tx.send(Command::Stop).unwrap();
    h.engine.drain_commands(&rx).unwrap();
    assert_eq!(h.engine.transport(), TransportState::Stopped);
}

#[test]
fn gapless_join_has_no_silence_and_no_dropped_frames() {
    let mut h = Harness::new(&[TRACK_A, TRACK_B]);
    h.engine.play_index(0).unwrap();
    // The decode thread may report both tracks before the first poll, so wait
    // for the whole queue rather than for a specific index.
    assert!(h.wait_buffered(), "both tracks must be buffered");

    // The decode thread has produced both tracks back to back, so the ring
    // holds exactly track 1 followed by track 2: the seam is frame-exact.
    assert_eq!(h.engine.timeline_frames(), 0);
    let a = track_frames(TRACK_A);
    let b = track_frames(TRACK_B);
    let total = a + b;
    assert_eq!(
        h.engine.buffered_frames(),
        total,
        "the whole queue must be buffered, with nothing dropped"
    );
    let mut rec = Vec::new();
    let mut pumped = 0u64;
    while pumped < total {
        let step = 500.min(total - pumped);
        h.clock.pump_record(step, &mut rec);
        pumped += step;
        assert_eq!(
            h.engine.timeline_frames(),
            pumped,
            "the audio clock must advance one frame per frame played"
        );
        if pumped < total {
            // Polling while frames are still to come; polling after the last
            // frame ends the run.
            h.engine.poll().unwrap();
        }
    }

    let left: Vec<f32> = rec.iter().step_by(2).copied().collect();
    assert_eq!(left.len() as u64, total, "one stereo pair per frame");
    assert_eq!(
        &left[..a as usize],
        &expected(1, 0, a)[..],
        "track 1 content up to the seam"
    );
    assert_eq!(
        &left[a as usize..],
        &expected(2, 0, b)[..],
        "track 2 content right after the seam"
    );
    assert!(!left.contains(&0.0), "the seam must not contain silence");
    // The seam itself: last frame of track 1, then first frame of track 2.
    assert_eq!(left[a as usize - 1], pattern(1, a - 1));
    assert_eq!(left[a as usize], pattern(2, 0));

    // Position continuity across the seam: the per-track position is the
    // timeline minus the track's start, so it keeps running in track 2.
    assert_eq!(h.engine.timeline_frames(), total);
    assert_eq!(h.engine.current_index(), Some(1));
    assert_eq!(
        h.engine.position_frames(),
        total - a,
        "the per-track position continues inside track 2"
    );

    // The end of the queue stops playback and says so.
    assert!(h.wait_idle(
        |e| e.transport() == TransportState::Stopped,
        Duration::from_secs(2)
    ));
    let events = h.drain_events();
    assert!(events.contains(&Event::PlaylistEnded), "{events:?}");
    assert!(events.contains(&Event::StateChanged(TransportState::Stopped)));
}

#[test]
fn seek_is_frame_accurate_and_plays_the_right_audio() {
    // A long track, so the seek target has room after it.
    let mut h = Harness::new(&[LONG_A]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    h.clock.pump(1000);
    h.engine.poll().unwrap();
    assert_eq!(h.engine.position_frames(), 1000);

    let target = 20000u64;
    assert!(
        h.seek_and_settle(target),
        "seek landed on {} instead of {target}",
        h.engine.position_frames()
    );

    // Wait for the decode thread to refill the ring *without pumping*, so no
    // underrun silence sneaks in between the position and the audio: every
    // rendered frame from here on is the source frame at the position.
    let p0 = h
        .wait_refilled(4096)
        .expect("the ring must refill after the seek");
    let mut rec = Vec::new();
    h.clock.pump_record(1024, &mut rec);
    h.engine.poll().unwrap();
    assert_eq!(h.engine.position_frames(), p0 + 1024);

    let left: Vec<f32> = rec.iter().step_by(2).copied().collect();
    assert_eq!(left.len(), 1024);
    assert_eq!(
        &left[..],
        &expected(1, p0, 1024)[..],
        "the audio after the seek must be the source at the current position"
    );
}

#[test]
fn seek_before_play_starts_at_the_target() {
    let mut h = Harness::new(&[TRACK_A]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    h.engine.stop().unwrap();
    h.engine.seek_frames(1500).unwrap();
    assert_eq!(h.engine.position_frames(), 1500, "the seek is remembered");
    h.engine.play().unwrap();
    assert!(
        h.settle_seek(1500),
        "playback must start at frame 1500 (at {})",
        h.engine.position_frames()
    );
    h.clock.pump(100);
    h.engine.poll().unwrap();
    assert_eq!(h.engine.position_frames(), 1600);
}

#[test]
fn vis_tap_is_filled_without_the_consumer_running() {
    let mut h = Harness::new(&[TRACK_A]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    // The tap runs on the decode thread, so frames appear without any pump.
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut frame = None;
    while Instant::now() < deadline {
        h.engine.poll().unwrap();
        if let Some(f) = h.engine.take_vis_frame() {
            frame = Some(f);
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let frame = frame.expect("a vis frame must be published");
    assert!(frame.timestamp_us > 0, "the tap has an audio clock");
    let peak = frame
        .waveform
        .iter()
        .flat_map(|row| row.iter())
        .fold(0.0f32, |a, v| a.max(v.abs()));
    assert!(peak > 0.0, "the waveform must carry the audio");
    assert_eq!(
        frame.waveform[0], frame.waveform[1],
        "the synthetic track is the same in both channels"
    );
    assert!(
        frame.spectrum[0].iter().all(|v| *v == 0.0),
        "the spectrum derivation is spec S08 and must stay untouched"
    );
}

#[test]
fn volume_and_balance_reach_the_render_callback() {
    let mut h = Harness::new(&[TRACK_A]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    h.engine.set_volume(255).unwrap();
    h.engine.set_balance(-127).unwrap();
    assert!(h.wait_buffered());

    let mut rec = Vec::new();
    h.clock.pump_record(256, &mut rec);
    for frame in rec.chunks_exact(2) {
        assert_eq!(frame[1], 0.0, "balance -127 mutes the right channel");
    }
    assert!(rec[0] != 0.0, "the left channel still carries audio");

    let mut rec = Vec::new();
    h.engine.set_volume(0).unwrap();
    h.clock.pump_record(256, &mut rec);
    assert!(rec.iter().all(|v| *v == 0.0), "volume 0 is silence");
}

#[test]
fn engine_survives_a_second_run() {
    let mut h = Harness::new(&[TRACK_A]);
    for round in 0..2 {
        h.engine.play_index(0).unwrap();
        assert!(h.wait_track(0), "round {round}");
        h.clock.pump(512);
        h.engine.poll().unwrap();
        assert_eq!(h.engine.position_frames(), 512, "round {round}");
        h.engine.stop().unwrap();
        assert_eq!(h.engine.position_frames(), 0);
    }
}

#[test]
fn output_format_is_negotiated_not_assumed() {
    let mut h = Harness::new(&[TRACK_A]);
    assert_eq!(h.engine.device_spec(), SPEC);
    assert_eq!(h.engine.output_name(), "null");
    let clock = h.clock.clone();
    assert_eq!(clock.spec(), Some(SPEC));
    assert!(!clock.is_running());
    h.engine.play_index(0).unwrap();
    assert!(clock.is_running());
    h.engine.stop().unwrap();
    assert_eq!(h.engine.device_spec(), SPEC);
    assert_eq!(h.engine.played_frames(), 0, "nothing was pulled");
}

#[test]
fn unknown_queue_index_is_an_error() {
    let mut h = Harness::new(&[TRACK_A]);
    assert!(h.engine.play_index(7).is_err());
    assert!(h.engine.play_index(0).is_ok());
    assert_eq!(h.engine.state().playlist_len, 1);
}
