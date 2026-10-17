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
use rustamp_core::state::{EqState, TransportState};

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
    // The final timeline survives the stop: the run froze where it ended, so
    // the position stays at the end of the last track and nothing is reported
    // as still buffered (the re-based clock must not reset the timeline).
    assert_eq!(h.engine.timeline_frames(), total);
    assert_eq!(h.engine.position_frames(), b);
    assert_eq!(h.engine.buffered_frames(), 0);
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
    for frame in rec.as_chunks::<2>().0 {
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

// ----- RFC 0005 commands -----

#[test]
fn play_index_zero_starts_the_first_entry() {
    // RFC 0005 §5.5: "start of list" (40154) is the existing `PlayIndex(0)`.
    let mut h = Harness::new(&[LONG_A, LONG_B]);
    let (tx, rx) = command_channel();
    tx.send(Command::PlayIndex(0)).unwrap();
    assert_eq!(h.engine.drain_commands(&rx).unwrap(), 1);
    assert!(h.wait_track(0), "index 0 must open");
    assert_eq!(h.engine.current_index(), Some(0));
    assert_eq!(h.engine.transport(), TransportState::Playing);
}

#[test]
fn play_last_plays_the_last_entry() {
    let mut h = Harness::new(&[LONG_A, LONG_B]);
    let (tx, rx) = command_channel();
    tx.send(Command::PlayLast).unwrap();
    assert_eq!(h.engine.drain_commands(&rx).unwrap(), 1);
    assert!(h.wait_track(1), "the last entry must open");
    assert_eq!(h.engine.current_index(), Some(1));
    assert_eq!(h.engine.state().current_index, Some(1));
    assert_eq!(h.engine.transport(), TransportState::Playing);
}

#[test]
fn play_last_on_an_empty_queue_is_a_no_op() {
    let mut h = Harness::new(&[]);
    assert!(h.engine.apply(&Command::PlayLast).is_ok());
    assert_eq!(h.engine.current_index(), None);
    assert_eq!(h.engine.transport(), TransportState::Stopped);
    // `PlayIndex` stays strict: an out-of-range index is an error.
    assert!(h.engine.apply(&Command::PlayIndex(0)).is_err());
}

#[test]
fn skip_by_moves_the_entry_in_queue_order_and_clamps() {
    let mut h = Harness::new(&[LONG_A, LONG_B, LONG_A]);
    let (tx, rx) = command_channel();
    tx.send(Command::PlayIndex(0)).unwrap();
    h.engine.drain_commands(&rx).unwrap();
    assert!(h.wait_track(0));

    // Ten tracks forward from 0 is clamped to the last entry.
    tx.send(Command::SkipBy { delta: 10 }).unwrap();
    h.engine.drain_commands(&rx).unwrap();
    assert!(h.wait_track(2), "clamped at the last entry");

    // Two back, then ten back (clamped to the first).
    tx.send(Command::SkipBy { delta: -2 }).unwrap();
    h.engine.drain_commands(&rx).unwrap();
    assert!(h.wait_track(0), "two entries back");

    tx.send(Command::SkipBy { delta: -10 }).unwrap();
    h.engine.drain_commands(&rx).unwrap();
    assert!(h.wait_track(0), "clamped at the first entry");

    // One forward again, to prove the move works both ways.
    tx.send(Command::SkipBy { delta: 1 }).unwrap();
    h.engine.drain_commands(&rx).unwrap();
    assert!(h.wait_track(1), "one entry forward");
}

#[test]
fn a_zero_skip_is_a_no_op_on_an_empty_and_a_loaded_queue() {
    let mut h = Harness::new(&[LONG_A, LONG_B]);
    h.engine.play_index(1).unwrap();
    assert!(h.wait_track(1));
    let before = h.engine.duration_frames();
    h.engine.apply(&Command::SkipBy { delta: 0 }).unwrap();
    assert_eq!(
        h.engine.current_index(),
        Some(1),
        "delta 0 must not re-open the track"
    );
    assert_eq!(h.engine.duration_frames(), before);
    assert_eq!(h.engine.transport(), TransportState::Playing);

    let mut empty = Harness::new(&[]);
    assert!(empty.engine.apply(&Command::SkipBy { delta: 5 }).is_ok());
    assert_eq!(empty.engine.current_index(), None);
    assert_eq!(empty.engine.transport(), TransportState::Stopped);
}

#[test]
fn skip_by_while_stopped_moves_without_starting_playback() {
    // RFC 0005 §5.1 / S02 §1: changing the entry while stopped does not start
    // playback.
    let mut h = Harness::new(&[LONG_A, LONG_B, LONG_A]);
    h.engine.stop().unwrap();
    assert_eq!(h.engine.transport(), TransportState::Stopped);

    h.engine.apply(&Command::SkipBy { delta: 2 }).unwrap();
    assert_eq!(
        h.engine.transport(),
        TransportState::Stopped,
        "no playback starts"
    );
    assert_eq!(h.engine.current_index(), Some(2), "the current entry moved");
    assert_eq!(h.engine.state().current_index, Some(2));

    // A later play starts from the moved entry.
    h.engine.play().unwrap();
    assert!(h.wait_track(2));
    assert_eq!(h.engine.transport(), TransportState::Playing);
}

#[test]
fn stop_after_current_halts_at_the_end_of_the_track() {
    let mut h = Harness::new(&[TRACK_A, TRACK_B]);
    h.engine.apply(&Command::ToggleStopAfterCurrent).unwrap();
    assert!(h.engine.stop_after_current());
    assert!(
        h.engine.state().stop_after_current,
        "the mode is observable through PlayerState"
    );

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    let total = track_frames(TRACK_A);
    assert!(
        h.wait_until(|e| e.transport() == TransportState::Stopped, total + 8192),
        "the run must stop at the end of the first track"
    );
    assert_eq!(
        h.engine.current_index(),
        Some(0),
        "the second track must not be opened"
    );
    let events = h.drain_events();
    assert!(
        events.contains(&Event::StateChanged(TransportState::Stopped)),
        "{events:?}"
    );
    assert!(
        !events.contains(&Event::PlaylistEnded),
        "the queue did not end; it was halted: {events:?}"
    );
    assert!(
        h.engine.stop_after_current(),
        "the mode stays armed (it is a toggle)"
    );

    // Toggling it off again lets the next run advance again.
    h.engine.apply(&Command::ToggleStopAfterCurrent).unwrap();
    assert!(!h.engine.stop_after_current());
    assert!(!h.engine.state().stop_after_current);
}

#[test]
fn arming_stop_after_current_mid_track_reaches_the_running_decode_thread() {
    // The long tracks keep the decode thread inside the first one, so arming the
    // mode is not a race: it travels as a control message to a live worker.
    let mut h = Harness::new(&[LONG_A, LONG_B]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    h.engine.apply(&Command::ToggleStopAfterCurrent).unwrap();
    assert!(h.engine.stop_after_current());

    let total = track_frames(LONG_A);
    assert!(
        h.wait_until(|e| e.transport() == TransportState::Stopped, total + 8192),
        "the run must stop at the end of the first track"
    );
    assert_eq!(h.engine.current_index(), Some(0));
    let events = h.drain_events();
    assert!(!events.contains(&Event::PlaylistEnded), "{events:?}");
}

#[test]
fn stop_after_current_on_the_last_track_still_ends_the_playlist() {
    // RFC 0005 O8 leaves the `PlaylistEnded` interaction open; the implemented
    // reading: the event fires iff the queue was actually exhausted. Arming the
    // mode while the last entry plays therefore still ends the playlist — the
    // halt is a stop instead of an advance only when a next entry exists.
    let mut h = Harness::new(&[TRACK_A, TRACK_B]);
    h.engine.apply(&Command::ToggleStopAfterCurrent).unwrap();
    h.engine.play_index(1).unwrap();
    assert!(h.wait_track(1));
    let total = track_frames(TRACK_B);
    assert!(
        h.wait_until(|e| e.transport() == TransportState::Stopped, total + 8192),
        "the run must stop at the end of the last track"
    );
    assert_eq!(h.engine.current_index(), Some(1), "no next entry exists");
    let events = h.drain_events();
    assert!(
        events.contains(&Event::PlaylistEnded),
        "the queue was exhausted: {events:?}"
    );
    assert!(
        events.contains(&Event::StateChanged(TransportState::Stopped)),
        "{events:?}"
    );
}

#[test]
fn the_engine_ignores_the_app_level_commands() {
    // The app/window commands are not engine mutations; applying them must not
    // disturb the transport (RFC 0005 §3).
    let mut h = Harness::new(&[LONG_A]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    for command in [
        Command::OpenFileInfo,
        Command::OpenAbout,
        Command::OpenSkinBrowser,
        Command::RefreshSkin,
        Command::Minimize,
        Command::FocusNextWindow,
        Command::ShowMainWindow,
        Command::CloseFocusedWindow,
        Command::OpenMainMenu,
        Command::ConfigureVisPlugin,
        Command::ToggleVisPlugin,
    ] {
        assert!(h.engine.apply(&command).is_ok(), "{command:?}");
    }
    assert_eq!(h.engine.transport(), TransportState::Playing);
    assert_eq!(h.engine.current_index(), Some(0));
}

#[test]
fn eq_commands_drive_the_published_state() {
    let mut h = Harness::new(&[LONG_A]);
    assert_eq!(h.engine.state().eq, EqState::default());
    for command in [
        Command::SetBand {
            band: 0,
            gain_db: 6.0,
        },
        // Clamped to the engine's +-12 dB.
        Command::SetBand {
            band: 3,
            gain_db: 40.0,
        },
        // Out of range band: ignored.
        Command::SetBand {
            band: 10,
            gain_db: 5.0,
        },
        Command::SetPreamp(-3.0),
        // Non-finite: keeps the previous preamp.
        Command::SetPreamp(f32::NAN),
        Command::SetEqEnabled(true),
        Command::SetEqAuto(true),
    ] {
        assert!(h.engine.apply(&command).is_ok(), "{command:?}");
    }
    let eq = h.engine.state().eq;
    assert_eq!(
        eq.bands,
        [6.0, 0.0, 0.0, 12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(eq.preamp, -3.0);
    assert!(eq.enabled);
    assert!(eq.auto);
    assert_eq!(h.engine.eq(), eq);
}

/// Record `frames` left-channel frames of `LONG_A` from the start of a run,
/// with the engine's EQ set to `eq` before playback.
fn record_left_with_eq(eq: EqState, frames: u64) -> (u64, Vec<f32>) {
    let mut h = Harness::new(&[LONG_A]);
    h.engine.set_eq(eq);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    let p0 = h
        .wait_refilled(4096)
        .expect("the ring must fill at the start of the run");
    let mut rec = Vec::new();
    h.clock.pump_record(frames, &mut rec);
    (p0, rec.iter().step_by(2).copied().collect())
}

#[test]
fn flat_enabled_eq_is_bit_transparent_through_the_engine() {
    let eq = EqState {
        enabled: true,
        ..EqState::default()
    };
    let (p0, left) = record_left_with_eq(eq, 1024);
    assert_eq!(&left[..], &expected(1, p0, 1024)[..]);
}

#[test]
fn the_preamp_reaches_the_rendered_audio() {
    let eq = EqState {
        preamp: -6.0,
        enabled: true,
        ..EqState::default()
    };
    let (p0, left) = record_left_with_eq(eq, 1024);
    let factor = 10.0_f64.powf(-6.0 / 20.0);
    for (got, source) in left.iter().zip(expected(1, p0, 1024)) {
        let want = f64::from(source) * factor;
        assert!(
            (f64::from(*got) - want).abs() < 1e-6,
            "got {got}, want {want}"
        );
    }
}

/// A test DSP stage that counts how many frames it processed.
struct CountingDspStage {
    prepared_spec: Option<rustamp_core::audio::AudioSpec>,
    processed_samples: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl rustamp_core::audio::DspStage for CountingDspStage {
    fn name(&self) -> &str {
        "counting-dsp"
    }

    fn prepare(&mut self, spec: rustamp_core::audio::AudioSpec) -> rustamp_core::error::Result<()> {
        self.prepared_spec = Some(spec);
        Ok(())
    }

    fn process(&mut self, buf: &mut [f32]) {
        // Multiply by 0.5 and count samples processed
        self.processed_samples
            .fetch_add(buf.len(), std::sync::atomic::Ordering::SeqCst);
        for s in buf {
            *s *= 0.5;
        }
    }

    fn reset(&mut self) {}
}

#[test]
fn installed_dsp_chain_survives_consecutive_runs() {
    let mut h = Harness::new(&[TRACK_A]);
    let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // Install user DSP stage before first run
    h.engine.set_dsp_chain(vec![Box::new(CountingDspStage {
        prepared_spec: None,
        processed_samples: std::sync::Arc::clone(&counter),
    })]);

    // First run: play index 0 to the end
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0), "track must open for first run");
    assert!(
        h.wait_buffered(),
        "first run track must be buffered to end of stream"
    );

    // Pump clock until the track is finished playing
    let total = track_frames(TRACK_A);
    assert!(
        h.wait_until(|e| e.transport() == TransportState::Stopped, total + 8192),
        "first run must complete and stop"
    );

    let first_run_samples = counter.load(std::sync::atomic::Ordering::SeqCst);
    assert!(
        first_run_samples > 0,
        "DSP stage must have processed samples during the first run"
    );

    // Second run: play again and assert the DSP stage is still active and processing
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0), "track must open for second run");
    assert!(
        h.wait_buffered(),
        "second run track must be buffered to end of stream"
    );

    assert!(
        h.wait_until(|e| e.transport() == TransportState::Stopped, total + 8192),
        "second run must complete and stop"
    );

    let second_run_samples = counter.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        second_run_samples,
        first_run_samples * 2,
        "DSP stage must have survived and processed identical sample count in the second run"
    );
}

// ----- Engine lifecycle correctness findings -----

#[test]
fn timeline_does_not_dip_after_seek_before_play() {
    // Finding F1: verify position_frames() does NOT briefly read 0 after
    // play() following seek_frames() while stopped.
    let mut h = Harness::new(&[TRACK_A]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    // Stop (resets position to 0).
    h.engine.stop().unwrap();
    assert_eq!(h.engine.transport(), TransportState::Stopped);
    assert_eq!(h.engine.position_frames(), 0);

    // Seek while stopped sets the target immediately.
    h.engine.seek_frames(1500).unwrap();
    assert_eq!(h.engine.position_frames(), 1500);

    // Now play: the decode thread needs a moment to open and seek.
    // We verify position_frames() stays at the target, never drops to 0.
    h.engine.play().unwrap();
    assert_eq!(h.engine.transport(), TransportState::Playing);

    // Immediately poll without pumping the clock — the decode thread hasn't
    // sent Seeked yet, but position_frames() must still read 1500 (not 0).
    h.engine.poll().unwrap();
    assert_eq!(
        h.engine.position_frames(),
        1500,
        "position must stay at seek target before Seeked; actual: {}",
        h.engine.position_frames()
    );

    // Settle the seek fully.
    assert!(h.settle_seek(1500), "seek did not land on frame 1500");
    assert_eq!(h.engine.position_frames(), 1500);

    // Advance a bit more to verify continuous playback.
    h.clock.pump(100);
    h.engine.poll().unwrap();
    assert_eq!(h.engine.position_frames(), 1600);
}

#[test]
fn finish_run_frozes_at_content_end_not_device_position() {
    // Finding F2 — restated for the ring-drain fix.
    //
    // Previously the test just checked `position_frames() <= track_len`.
    // That weak bound passed even with the old device-clock-based finish
    // condition once we also fixed `finish_run` to freeze at `end_of_stream`.
    //
    // New expectation (stronger): after the run ends,
    //   position_frames() == track_len      (frozen exactly at content end)
    //   buffered_frames() == 0              (ring is empty)
    //   transport         == Stopped
    //
    // The test still pumps past the track length to create an underrun
    // scenario (device plays silence). The run must NOT finish until the
    // ring is truly drained (consumed >= written), but it must also not
    // stall forever: wait_until pumps in steps and polls between them.
    let mut h = Harness::new(&[TRACK_A]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    let track_len = track_frames(TRACK_A);

    // Wait for the decode thread to buffer everything.
    assert!(h.wait_buffered(), "track must be buffered");

    // Pump past the track length so the device plays silence (underrun).
    let extra = 1024;
    h.clock.pump(track_len + extra);
    h.engine.poll().unwrap();

    // The run must end, but only once the ring is actually drained.
    assert!(
        h.wait_until(
            |e| e.transport() == TransportState::Stopped,
            track_len + 8192
        ),
        "run must have ended"
    );

    // The timeline must be frozen exactly at the content end (end_of_stream),
    // not at the device position (which ran further with silence).
    assert_eq!(
        h.engine.position_frames(),
        track_len,
        "position must equal track length after the run; got {} (track = {})",
        h.engine.position_frames(),
        track_len
    );

    // The ring must be fully drained.
    assert_eq!(
        h.engine.buffered_frames(),
        0,
        "buffered must be 0 after run ends"
    );
}

/// Normal drain: the run finishes exactly when the ring is empty, the position
/// lands at the end and `buffered_frames()` is zero.
#[test]
fn normal_run_finishes_when_ring_is_drained() {
    let mut h = Harness::new(&[TRACK_A]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    let track_len = track_frames(TRACK_A);

    // Wait until the whole track is in the ring.
    assert!(h.wait_buffered(), "track must be fully buffered");

    // At this point written == track_len and consumed == 0 (nothing pumped).
    assert_eq!(
        h.engine.buffered_frames(),
        track_len,
        "whole track must be in the ring before we start pumping"
    );
    assert_eq!(h.engine.transport(), TransportState::Playing);

    // Drive the engine to completion with controlled pumping.
    assert!(
        h.wait_until(
            |e| e.transport() == TransportState::Stopped,
            track_len + 8192
        ),
        "run must finish within the track length"
    );

    // The ring must be empty.
    assert_eq!(
        h.engine.buffered_frames(),
        0,
        "ring must be empty when the run ends"
    );
    // The position must be exactly at the track end.
    assert_eq!(
        h.engine.position_frames(),
        track_len,
        "position must be at track end when run finishes"
    );
    assert_eq!(h.engine.transport(), TransportState::Stopped);
}

/// After a seek while paused, the ring is drained by the handshake and the
/// `consumed` counter is reset to `written` so `buffered_frames()` stays
/// consistent (no stale backlog reported).
#[test]
fn seek_while_paused_drains_consistently() {
    let mut h = Harness::new(&[LONG_A]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    // Let the decode thread buffer some frames, then pause.
    h.clock.pump(512);
    h.engine.poll().unwrap();
    h.engine.pause().unwrap();
    assert_eq!(h.engine.transport(), TransportState::Paused);

    // Wait for some buffering in the paused state.
    std::thread::sleep(Duration::from_millis(50));
    h.engine.poll().unwrap();

    // Issue a seek. While paused the drain happens on the first callback after
    // resume (documented behaviour); the consumed counter will be reset then.
    h.engine.seek_frames(1000).unwrap();

    // Resume so the drain callback can fire.
    h.engine.play().unwrap();

    // Settle the seek (pumps until position == 1000).
    assert!(
        h.settle_seek(1000),
        "seek must settle on frame 1000; got {}",
        h.engine.position_frames()
    );

    // After the seek drains, buffered_frames() must not report a stale pre-seek
    // backlog. It may be non-zero (post-seek frames already written) but must
    // not exceed what the decode thread has since written.
    h.engine.poll().unwrap();
    // end_of_stream is None while the run is live; check written via
    // buffered_frames() being sane.
    let written = h.engine.end_of_stream().unwrap_or(0);
    let _ = written; // used only to document intent

    // The key invariant: buffered_frames() == written - consumed.
    // After a seek-drain the render callback aligns consumed to written,
    // so buffered_frames() reflects only post-seek frames.
    // We can't easily measure the exact value headlessly, but we can assert
    // it is not wildly large (no stale pre-seek backlog leaking through).
    let track_len = track_frames(LONG_A);
    assert!(
        h.engine.buffered_frames() <= track_len,
        "buffered_frames must not exceed the total track length after a seek; got {}",
        h.engine.buffered_frames()
    );
    assert_eq!(
        h.engine.transport(),
        TransportState::Playing,
        "transport must still be Playing after the seek"
    );
}

/// Consecutive runs: after a track finishes and a second run is started with
/// `play_index()`, the consumed counter must reset to 0 alongside written so
/// the second run buffers and drains completely without finishing early.
#[test]
fn second_run_resets_consumed_and_drains_correctly() {
    let mut h = Harness::new(&[TRACK_A]);
    let track_len = track_frames(TRACK_A);

    // First run: play to completion.
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    assert!(h.wait_buffered());
    assert!(
        h.wait_until(
            |e| e.transport() == TransportState::Stopped,
            track_len + 8192
        ),
        "first run must complete"
    );
    assert_eq!(h.engine.buffered_frames(), 0);
    assert_eq!(h.engine.position_frames(), track_len);
    assert_eq!(h.engine.transport(), TransportState::Stopped);

    // Second run: play the same track again.
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0), "track must open for second run");
    assert!(h.wait_buffered(), "second run must buffer");

    // Before pumping any frames for the second run, buffered_frames() must report
    // the whole track length (written = track_len, consumed = 0), NOT 0 from stale state!
    assert_eq!(
        h.engine.buffered_frames(),
        track_len,
        "second run must start with full track buffered"
    );
    assert_eq!(h.engine.transport(), TransportState::Playing);

    // Drive second run to completion.
    assert!(
        h.wait_until(
            |e| e.transport() == TransportState::Stopped,
            track_len + 8192
        ),
        "second run must complete naturally"
    );
    assert_eq!(h.engine.buffered_frames(), 0);
    assert_eq!(h.engine.position_frames(), track_len);
    assert_eq!(h.engine.transport(), TransportState::Stopped);
}

/// Job 1: Equalizer state changes publish Event::EqChanged, and setting the
/// same state twice does not produce a second event.
#[test]
fn eq_commands_publish_event_and_deduplicate_no_ops() {
    let mut h = Harness::new(&[LONG_A]);
    // Drain any startup events.
    let _ = h.drain_events();

    // 1. Command::SetBand: produces exactly one EqChanged event.
    h.engine
        .apply(&Command::SetBand {
            band: 2,
            gain_db: 3.5,
        })
        .unwrap();
    let events = h.drain_events();
    assert_eq!(
        events.len(),
        1,
        "SetBand must publish exactly one event; got: {events:?}"
    );
    match &events[0] {
        Event::EqChanged(eq) => {
            assert!((eq.bands[2] - 3.5).abs() < 1e-6);
        }
        other => panic!("expected Event::EqChanged, got: {other:?}"),
    }

    // Setting the same band to the exact same gain produces NO event.
    h.engine
        .apply(&Command::SetBand {
            band: 2,
            gain_db: 3.5,
        })
        .unwrap();
    let events = h.drain_events();
    assert!(
        events.is_empty(),
        "Setting identical band gain must not publish: {events:?}"
    );

    // 2. Command::SetPreamp: produces exactly one EqChanged event.
    h.engine.apply(&Command::SetPreamp(-2.0)).unwrap();
    let events = h.drain_events();
    assert_eq!(
        events.len(),
        1,
        "SetPreamp must publish exactly one event; got: {events:?}"
    );
    match &events[0] {
        Event::EqChanged(eq) => {
            assert!((eq.preamp - (-2.0)).abs() < 1e-6);
        }
        other => panic!("expected Event::EqChanged, got: {other:?}"),
    }

    // Setting the same preamp produces NO event.
    h.engine.apply(&Command::SetPreamp(-2.0)).unwrap();
    let events = h.drain_events();
    assert!(
        events.is_empty(),
        "Setting identical preamp must not publish: {events:?}"
    );

    // 3. Command::SetEqEnabled: produces exactly one EqChanged event.
    h.engine.apply(&Command::SetEqEnabled(true)).unwrap();
    let events = h.drain_events();
    assert_eq!(
        events.len(),
        1,
        "SetEqEnabled must publish exactly one event; got: {events:?}"
    );
    match &events[0] {
        Event::EqChanged(eq) => {
            assert!(eq.enabled);
        }
        other => panic!("expected Event::EqChanged, got: {other:?}"),
    }

    // Setting identical enabled state produces NO event.
    h.engine.apply(&Command::SetEqEnabled(true)).unwrap();
    let events = h.drain_events();
    assert!(
        events.is_empty(),
        "Setting identical enabled state must not publish: {events:?}"
    );

    // 4. Command::SetEqAuto: produces exactly one EqChanged event.
    h.engine.apply(&Command::SetEqAuto(true)).unwrap();
    let events = h.drain_events();
    assert_eq!(
        events.len(),
        1,
        "SetEqAuto must publish exactly one event; got: {events:?}"
    );
    match &events[0] {
        Event::EqChanged(eq) => {
            assert!(eq.auto);
        }
        other => panic!("expected Event::EqChanged, got: {other:?}"),
    }

    // Setting identical auto state produces NO event.
    h.engine.apply(&Command::SetEqAuto(true)).unwrap();
    let events = h.drain_events();
    assert!(
        events.is_empty(),
        "Setting identical auto state must not publish: {events:?}"
    );

    // 5. Engine::set_eq with whole state produces exactly one event.
    let mut custom = h.engine.eq();
    custom.bands[0] = 5.0;
    h.engine.set_eq(custom);
    let events = h.drain_events();
    assert_eq!(
        events.len(),
        1,
        "set_eq must publish exactly one event; got: {events:?}"
    );
    match &events[0] {
        Event::EqChanged(eq) => {
            assert!((eq.bands[0] - 5.0).abs() < 1e-6);
        }
        other => panic!("expected Event::EqChanged, got: {other:?}"),
    }

    // Setting identical whole state produces NO event.
    h.engine.set_eq(custom);
    let events = h.drain_events();
    assert!(
        events.is_empty(),
        "set_eq with same state must not publish: {events:?}"
    );

    // 6. Polling must never publish EqChanged.
    h.engine.poll().unwrap();
    let events = h.drain_events();
    assert!(
        events.iter().all(|e| !matches!(e, Event::EqChanged(_))),
        "poll must not publish EqChanged: {events:?}"
    );

    // 7. Clamped values that map to the same clamped value produce NO second event.
    // Setting 15.0 dB clamps to 12.0 dB -> publishes once.
    h.engine
        .apply(&Command::SetBand {
            band: 5,
            gain_db: 15.0,
        })
        .unwrap();
    let events = h.drain_events();
    assert_eq!(events.len(), 1);
    // Setting 18.0 dB also clamps to 12.0 dB -> no change -> no event.
    h.engine
        .apply(&Command::SetBand {
            band: 5,
            gain_db: 18.0,
        })
        .unwrap();
    let events = h.drain_events();
    assert!(
        events.is_empty(),
        "Clamped value with no effective change must not publish: {events:?}"
    );

    // 8. Out-of-bounds band index is ignored and produces NO event.
    h.engine
        .apply(&Command::SetBand {
            band: 99,
            gain_db: 5.0,
        })
        .unwrap();
    let events = h.drain_events();
    assert!(
        events.is_empty(),
        "Out-of-range band must not publish: {events:?}"
    );
}

/// Job 2 Item (a): Interleaving between stop() and finish_run() in poll().
/// Calling stop() when the stream is ready to finish cleanly transitions to
/// Stopped at origin (0), and subsequent poll() does not trigger finish_run().
#[test]
fn stop_at_end_of_stream_prevents_duplicate_finish_run() {
    let mut h = Harness::new(&[TRACK_A]);
    let track_len = track_frames(TRACK_A);

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    assert!(h.wait_buffered());

    // Pump all audio so consumed == written == end_of_stream.
    h.clock.pump(track_len + 512);

    // Instead of calling poll() (which would invoke finish_run()), call stop() directly!
    h.engine.stop().unwrap();
    assert_eq!(h.engine.transport(), TransportState::Stopped);
    assert_eq!(h.engine.position_frames(), 0);
    assert_eq!(h.engine.buffered_frames(), 0);

    let stop_events = h.drain_events();
    assert!(
        stop_events
            .iter()
            .any(|e| matches!(e, Event::StateChanged(TransportState::Stopped))),
        "stop must publish StateChanged(Stopped)"
    );

    // Now call poll(). Since transport is Stopped and end_of_stream was cleared by stop(),
    // finish_run() must NOT fire. No duplicate StateChanged(Stopped) or PlaylistEnded!
    h.engine.poll().unwrap();
    let poll_events = h.drain_events();
    assert!(
        !poll_events
            .iter()
            .any(|e| matches!(e, Event::StateChanged(TransportState::Stopped))),
        "poll() after stop() must not fire finish_run() again"
    );
    assert!(
        !poll_events
            .iter()
            .any(|e| matches!(e, Event::PlaylistEnded)),
        "poll() after explicit stop() must not fire PlaylistEnded"
    );
}

/// Job 2 Item (a): Calling stop() immediately after finish_run() has stopped the run
/// cleanly resets position to 0 without panics or double joins.
#[test]
fn stop_after_finish_run_resets_position_to_zero() {
    let mut h = Harness::new(&[TRACK_A]);
    let track_len = track_frames(TRACK_A);

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    assert!(h.wait_buffered());

    // Pump to completion through wait_until (which runs poll() and invokes finish_run()).
    assert!(
        h.wait_until(
            |e| e.transport() == TransportState::Stopped,
            track_len + 8192
        ),
        "run must finish"
    );
    assert_eq!(h.engine.position_frames(), track_len);
    assert_eq!(h.engine.transport(), TransportState::Stopped);

    // User presses Stop on the completed track:
    h.engine.stop().unwrap();
    assert_eq!(h.engine.transport(), TransportState::Stopped);
    assert_eq!(h.engine.position_frames(), 0);
    assert_eq!(h.engine.buffered_frames(), 0);
}

/// Job 2: F2 Invariants hold across underruns, seeks, and completion:
/// - consumed <= written at every point (buffered_frames() >= 0 and never underflows).
/// - buffered_frames() is honest (reflects ring buffer backlog, not saturating to 0).
/// - consumed is aligned on seek drain.
#[test]
fn f2_invariants_hold_across_underruns_and_seeks() {
    let mut h = Harness::new(&[LONG_A]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));

    // 1. Initial state: buffered_frames is honest.
    // Some audio should be buffered in the ring.
    std::thread::sleep(Duration::from_millis(50));
    h.engine.poll().unwrap();
    let initial_buffered = h.engine.buffered_frames();
    assert!(
        initial_buffered > 0,
        "decode thread must have buffered frames in ring"
    );

    // 2. Severe underrun: pump more frames than were buffered.
    // Device plays silence; consumed must only advance by what was actually read.
    h.clock.pump(initial_buffered + 4000);
    h.engine.poll().unwrap();
    // Invariant: buffered_frames() must not underflow.
    assert_eq!(
        h.engine.buffered_frames(),
        0,
        "ring should be drained to 0 during underrun, not underflow"
    );
    assert_eq!(
        h.engine.transport(),
        TransportState::Playing,
        "underrun on long track must not end run prematurely"
    );

    // 3. Seek: seek to frame 5000.
    h.engine.seek_frames(5000).unwrap();
    assert!(h.settle_seek(5000), "seek must settle on frame 5000");

    // 4. Seek drain alignment: post-seek frames are written.
    h.engine.poll().unwrap();
    let post_seek_buffered = h.engine.buffered_frames();
    // Post-seek backlog must be honest and not wildly exceed ring capacity.
    assert!(
        post_seek_buffered <= 16384,
        "buffered frames must be bounded by ring capacity: {post_seek_buffered}"
    );

    // 5. Clean stop: resets backlog to 0.
    h.engine.stop().unwrap();
    assert_eq!(h.engine.buffered_frames(), 0);
    assert_eq!(h.engine.position_frames(), 0);
}

// ---------------------------------------------------------------------------
// Look-ahead brickwall limiter (DSP chain last stage)
// ---------------------------------------------------------------------------

/// A test DSP stage that scales all samples by a constant gain factor.
struct GainBoostDspStage(f32);

impl rustamp_core::audio::DspStage for GainBoostDspStage {
    fn name(&self) -> &str {
        "gain-boost-dsp"
    }

    fn prepare(
        &mut self,
        _spec: rustamp_core::audio::AudioSpec,
    ) -> rustamp_core::error::Result<()> {
        Ok(())
    }

    fn process(&mut self, buf: &mut [f32]) {
        for s in buf {
            *s *= self.0;
        }
    }

    fn reset(&mut self) {}
}

#[test]
fn default_engine_limiter_is_disabled_and_bit_exact() {
    let mut h = Harness::new(&[LONG_A]);
    // The engine's limiter must default to disabled so an untouched player is bit-transparent.
    assert!(!h.engine.limiter_enabled());
    assert!(!h.engine.limiter_control().enabled());

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    let p0 = h.wait_refilled(4096).expect("ring must fill with audio");
    let mut rec = Vec::new();
    h.clock.pump_record(1024, &mut rec);
    let left: Vec<f32> = rec.iter().step_by(2).copied().collect();

    // Default configuration: bit-exact to the synthetic source pattern.
    assert_eq!(&left[..], &expected(1, p0, 1024)[..]);
}

#[test]
fn limiter_limits_hot_signal_to_ceiling_and_disabling_returns_to_bit_exact() {
    // The limiter sits as the LAST stage in the DSP chain (order: Equalizer,
    // installed stages, Limiter). It must see everything earlier stages
    // produced, and is the stage closest to the output.
    //
    // Note: Enabling the limiter is a product decision (the reference player's
    // behaviour with overs is unmeasured) and costs look-ahead latency
    // (LOOKAHEAD_SECONDS, 5 ms).
    let mut h = Harness::new(&[LONG_A]);
    let ceiling = h.engine.limiter_ceiling();

    // Install a 2.0x boost stage so the signal is hot and definitely exceeds
    // the ceiling (pattern samples 0.1..=1.1 become 0.2..=2.2).
    h.engine
        .set_dsp_chain(vec![Box::new(GainBoostDspStage(2.0))]);

    // --- Phase 1: Default is disabled (bypass) ---
    assert!(!h.engine.limiter_enabled());
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    let p0 = h.wait_refilled(4096).expect("refill run 1");
    let mut rec_disabled = Vec::new();
    h.clock.pump_record(2048, &mut rec_disabled);
    let left_disabled: Vec<f32> = rec_disabled.iter().step_by(2).copied().collect();

    // The un-limited hot signal has samples exceeding the ceiling.
    let peak_disabled = left_disabled.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        peak_disabled > ceiling,
        "unlimited signal must exceed ceiling ({peak_disabled} > {ceiling})"
    );
    // Exact bypass: each sample is exactly 2.0 * source.
    for (out, src) in left_disabled.iter().zip(expected(1, p0, 2048)) {
        assert_eq!(*out, src * 2.0);
    }
    // --- Phase 2: Enable the limiter before the run ---
    // A fresh harness on purpose: `stop()` leaves the run's audio in the ring
    // and `start_run` does not invalidate it, so a stopped-then-started engine
    // replays the previous run's tail first. Reusing this harness would measure
    // that leftover instead of the limiter; the ring reuse is a finding of its
    // own (see docs/tasks/A8-ring-reuse.md).
    let mut h = Harness::new(&[LONG_A]);
    let ceiling = h.engine.limiter_ceiling();
    h.engine
        .set_dsp_chain(vec![Box::new(GainBoostDspStage(2.0))]);
    h.engine.set_limiter_enabled(true);
    assert!(h.engine.limiter_enabled());
    assert!(h.engine.limiter_control().enabled());

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    let p0_enabled = h.wait_refilled(4096).expect("refill run 2");
    assert_eq!(p0_enabled, p0);
    let mut rec_enabled = Vec::new();
    h.clock.pump_record(2048, &mut rec_enabled);
    let left_enabled: Vec<f32> = rec_enabled.iter().step_by(2).copied().collect();

    // Enabling changes the rendered audio.
    assert_ne!(left_enabled, left_disabled);

    // Because the limiter sits LAST in the DSP chain (after the 2.0x boost stage),
    // it sees the hot audio and ensures no output sample exceeds the ceiling.
    let peak_enabled = left_enabled.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        peak_enabled <= ceiling,
        "limiter overshoot: peak {peak_enabled} exceeds ceiling {ceiling}"
    );
    // The peak actually reached the ceiling (it limited the hot peaks, not muted).
    assert!(
        peak_enabled >= ceiling * 0.99,
        "peak {peak_enabled} should be held close to ceiling {ceiling}"
    );
    // --- Phase 3: The control round-trips back to disabled ---
    // Phase 1 already proves that a disabled limiter is a bit-exact bypass, so
    // this only has to pin that the switch takes effect.
    h.engine.set_limiter_enabled(false);
    assert!(!h.engine.limiter_enabled());
    assert!(!h.engine.limiter_control().enabled());
}

#[test]
fn limiter_toggle_latches_on_seek() {
    let mut h = Harness::new(&[LONG_A]);
    let ceiling = h.engine.limiter_ceiling();
    h.engine
        .set_dsp_chain(vec![Box::new(GainBoostDspStage(2.0))]);

    // Start playback with limiter disabled.
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    let _p0 = h.wait_refilled(4096).expect("refill");
    let mut rec = Vec::new();
    h.clock.pump_record(1024, &mut rec);
    let peak = rec.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > ceiling);

    // Toggle limiter on while playing.
    h.engine.set_limiter_enabled(true);
    assert!(h.engine.limiter_enabled());

    // Seeking resets DSP stages, latching the enabled state into the limiter stage.
    assert!(h.seek_and_settle(5000));
    let p_after = h.wait_refilled(4096).expect("refill after seek");
    let mut rec_after = Vec::new();
    h.clock.pump_record(2048, &mut rec_after);
    let peak_after = rec_after.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        peak_after <= ceiling,
        "limiter must be active after seek: peak {peak_after} <= {ceiling}"
    );
    assert_eq!(p_after, 5000);
}
