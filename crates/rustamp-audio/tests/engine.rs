//! Engine behaviour: the transport state machine and its events, gapless joins,
//! frame-accurate seeking, volume/balance and the vis tap.
//!
//! Every test drives a virtual device ([`rustamp_output::NullOutput`]) so that
//! time, and therefore the audio clock, is under the test's control.

mod common;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Harness, SPEC, TONE_AMPLITUDE, TONE_HZ, WAIT, expected, pattern, track_frames};
use rustamp_audio::EngineConfig;
use rustamp_core::audio::{AudioSpec, OutputDevice, OutputFormat, RenderFn};
use rustamp_core::bus::command_channel;
use rustamp_core::command::Command;
use rustamp_core::error::{Error, ErrorKind, Result};
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
    // The test asserts bit-exact source pattern across the seam; pattern values
    // exceed the limiter's -0.3 dBFS ceiling (0.1..=1.1), so disable the default-on
    // limiter to isolate the gapless seam behavior.
    h.engine.set_limiter_enabled(false);
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
    assert!(h.wait_idle(|e| e.transport() == TransportState::Stopped, WAIT));
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
    // Disable limiter to verify bit-exact target audio without overshoot limiting.
    h.engine.set_limiter_enabled(false);
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
    let deadline = Instant::now() + WAIT;
    let mut frame = None;
    while Instant::now() < deadline {
        h.engine.poll().unwrap();
        if let Some(f) = h.engine.take_vis_frame() {
            frame = Some(f);
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let frame = frame.expect("stall: no vis frame was published within WAIT");
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
        // Round 1 starts with round 0's audio in the ring. Until its drain and
        // the clock re-base are done, the device plays no audio of this run,
        // so the clock is pinned from there on (A8).
        assert!(h.settle_drain(), "round {round}");
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
    assert!(
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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

    assert!(
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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
    assert!(
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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
    // The EQ tests assert exact values against source pattern (e.g. flat EQ bit-transparency),
    // so disable the limiter to isolate the EQ stage from limiter overshoot attenuation.
    h.engine.set_limiter_enabled(false);
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

    fn process(&mut self, buf: &mut [f32]) -> rustamp_core::error::Result<()> {
        // Multiply by 0.5 and count samples processed
        self.processed_samples
            .fetch_add(buf.len(), std::sync::atomic::Ordering::SeqCst);
        for s in buf {
            *s *= 0.5;
        }
        Ok(())
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
    assert!(
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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
    // stall forever: wait_pumping pumps in steps and polls between them.
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
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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
    // The test asserts bit-exact post-seek audio; disable the limiter to isolate the seek drain.
    h.engine.set_limiter_enabled(false);
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

    // The raw counters, not the saturating `buffered_frames()` (A6 test
    // audit): the drain aligned `consumed` with the seek base, so it is at
    // least 1000, and it never runs ahead of `written`.
    let (written, consumed) = h.engine.ring_counters();
    assert!(consumed >= 1000, "consumed {consumed} behind the seek base");
    assert!(
        consumed <= written,
        "consumed {consumed} > written {written}"
    );

    // And the audio that follows is the source at the position: no post-seek
    // block was written before the drain and dropped by it.
    let p0 = h
        .wait_refilled(4096)
        .expect("the ring must refill after the seek");
    assert_eq!(p0, 1000, "nothing was played between the drain and here");
    let mut rec = Vec::new();
    h.clock.pump_record(1024, &mut rec);
    let left: Vec<f32> = rec.iter().step_by(2).copied().collect();
    assert_eq!(&left[..], &expected(1, p0, 1024)[..]);
    let (written, consumed) = h.engine.ring_counters();
    assert_eq!(
        consumed,
        1000 + 1024,
        "every played frame came from the ring"
    );
    assert!(consumed <= written);
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
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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

    // Pump to completion through wait_pumping (which runs poll() and invokes finish_run()).
    assert!(
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
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
    assert!(
        h.wait_idle(|e| e.buffered_frames() > 0, WAIT),
        "stall: decode thread never buffered frames in ring"
    );
    let initial_buffered = h.engine.buffered_frames();
    assert!(
        initial_buffered > 0,
        "decode thread must have buffered frames in ring"
    );

    // 2. Severe underrun: pump more frames than were buffered.
    // Device plays silence; consumed must only advance by what was actually
    // read. The raw counters are checked, because `buffered_frames()`
    // saturates and would read 0 for `consumed > written` too (A6 test audit).
    let (_, consumed_before) = h.engine.ring_counters();
    let pumped = initial_buffered + 4000;
    h.clock.pump(pumped);
    h.engine.poll().unwrap();
    let (written, consumed) = h.engine.ring_counters();
    assert!(
        consumed <= written,
        "consumed {consumed} ran ahead of written {written}"
    );
    assert!(
        consumed - consumed_before <= pumped,
        "consumed counts more frames than the device pulled"
    );
    assert_eq!(h.engine.buffered_frames(), written - consumed);
    assert_eq!(
        h.engine.transport(),
        TransportState::Playing,
        "underrun on long track must not end run prematurely"
    );

    // 3. Seek: seek to frame 5000.
    h.engine.seek_frames(5000).unwrap();
    assert!(h.settle_seek(5000), "seek must settle on frame 5000");

    // 4. Seek drain alignment: `consumed` was set to the seek base (5000),
    // not left at the pre-seek `written` (A6 finding 2: a backward seek would
    // leave it above `written`, a forward one below the base).
    h.engine.poll().unwrap();
    let (written, consumed) = h.engine.ring_counters();
    assert!(consumed >= 5000, "consumed {consumed} behind the seek base");
    assert!(
        consumed <= written,
        "consumed {consumed} > written {written} after the seek"
    );
    assert!(
        written - consumed <= 16384,
        "the backlog is bounded by the ring: {}",
        written - consumed
    );

    // 5. Clean stop: the run's counters and position reset. Whatever the
    // decode thread had buffered stays in the ring until the next run drains
    // it, and `buffered_frames()` reports it rather than 0 (A8; pinned by
    // `stopped_run_leaves_no_stale_audio_in_ring_on_restart`).
    h.engine.stop().unwrap();
    assert_eq!(h.engine.ring_counters(), (0, 0));
    assert!(h.engine.buffered_frames() <= 16384);
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

    fn process(&mut self, buf: &mut [f32]) -> rustamp_core::error::Result<()> {
        for s in buf {
            *s *= self.0;
        }
        Ok(())
    }

    fn reset(&mut self) {}
}

#[test]
fn default_engine_limiter_is_enabled_and_limits_overs() {
    let mut h = Harness::new(&[LONG_A]);
    let ceiling = h.engine.limiter_ceiling();
    // The engine's limiter defaults to enabled (owner decision, 2026-10-17).
    assert!(h.engine.limiter_enabled());
    assert!(h.engine.limiter_control().enabled());

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    let _p0 = h.wait_refilled(4096).expect("ring must fill with audio");
    let mut rec = Vec::new();
    h.clock.pump_record(1024, &mut rec);
    let left: Vec<f32> = rec.iter().step_by(2).copied().collect();

    // Default configuration limits overs: peak <= ceiling.
    let peak = left.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        peak <= ceiling,
        "peak {peak} must not exceed ceiling {ceiling}"
    );

    // Disabling returns to bit-exact bypass mode.
    h.engine.stop().unwrap();
    h.engine.set_limiter_enabled(false);
    assert!(!h.engine.limiter_enabled());
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    assert!(h.settle_drain());
    let p0_disabled = h.wait_refilled(4096).expect("ring must refill with audio");
    let mut rec_disabled = Vec::new();
    h.clock.pump_record(1024, &mut rec_disabled);
    let left_disabled: Vec<f32> = rec_disabled.iter().step_by(2).copied().collect();
    assert_eq!(&left_disabled[..], &expected(1, p0_disabled, 1024)[..]);
}

#[test]
fn limiter_limits_hot_signal_to_ceiling_and_disabling_returns_to_bit_exact() {
    // The limiter sits as the LAST stage in the DSP chain (order: Equalizer,
    // installed stages, Limiter). It must see everything earlier stages
    // produced, and is the stage closest to the output.
    //
    // Note: The limiter is enabled by default (owner decision, 2026-10-17);
    // what the reference player does with overs is unmeasured, so enabling
    // by default is our own product choice and it costs look-ahead latency
    // (LOOKAHEAD_SECONDS, 5 ms).
    let mut h = Harness::new(&[LONG_A]);
    let ceiling = h.engine.limiter_ceiling();

    // Install a 2.0x boost stage so the signal is hot and definitely exceeds
    // the ceiling (pattern samples 0.1..=1.1 become 0.2..=2.2).
    h.engine
        .set_dsp_chain(vec![Box::new(GainBoostDspStage(2.0))]);

    // --- Phase 1: Default is enabled ---
    assert!(h.engine.limiter_enabled());
    assert!(h.engine.limiter_control().enabled());
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    let p0 = h.wait_refilled(4096).expect("refill run 1");
    let mut rec_enabled = Vec::new();
    h.clock.pump_record(2048, &mut rec_enabled);
    let left_enabled: Vec<f32> = rec_enabled.iter().step_by(2).copied().collect();

    // Because the limiter sits LAST in the DSP chain (after the 2.0x boost stage),
    // it sees the hot audio and ensures no output sample exceeds the ceiling.
    let peak_enabled = left_enabled.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        peak_enabled <= ceiling,
        "limiter overshoot: peak {peak_enabled} exceeds ceiling {ceiling}"
    );
    // A17: the limiter is a true-peak limiter, so this is a sample-peak
    // assertion on a wideband signal. The linked true-peak estimate
    // (kernel-L1 bound ~= 3.0) exceeds the sample peak (2.2), so the output
    // sample peak can legitimately sit below 0.99·ceiling (analytic range ~=
    // [0.71, 1.00]·ceiling). The lower bound is relaxed to 0.7·ceiling; the
    // upper bound above is the exact guarantee.
    assert!(
        peak_enabled >= ceiling * 0.7,
        "peak {peak_enabled} should be held close to ceiling {ceiling}"
    );

    // --- Phase 2: Disable the limiter before the run ---
    h.engine.stop().unwrap();
    h.engine.set_limiter_enabled(false);
    assert!(!h.engine.limiter_enabled());
    assert!(!h.engine.limiter_control().enabled());

    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    assert!(h.settle_drain());
    let p0_disabled = h.wait_refilled(4096).expect("refill run 2");
    assert_eq!(p0_disabled, p0);
    let mut rec_disabled = Vec::new();
    h.clock.pump_record(2048, &mut rec_disabled);
    let left_disabled: Vec<f32> = rec_disabled.iter().step_by(2).copied().collect();

    // Disabling changes the rendered audio back to the un-limited hot signal.
    assert_ne!(left_enabled, left_disabled);
    let peak_disabled = left_disabled.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(
        peak_disabled > ceiling,
        "unlimited signal must exceed ceiling ({peak_disabled} > {ceiling})"
    );
    // Exact bypass: each sample is exactly 2.0 * source.
    for (out, src) in left_disabled.iter().zip(expected(1, p0_disabled, 2048)) {
        assert_eq!(*out, src * 2.0);
    }

    // --- Phase 3: The control round-trips back to enabled ---
    h.engine.set_limiter_enabled(true);
    assert!(h.engine.limiter_enabled());
    assert!(h.engine.limiter_control().enabled());
}

#[test]
fn limiter_toggle_latches_on_seek() {
    let mut h = Harness::new(&[LONG_A]);
    let ceiling = h.engine.limiter_ceiling();
    h.engine
        .set_dsp_chain(vec![Box::new(GainBoostDspStage(2.0))]);

    // Start playback with limiter explicitly disabled.
    h.engine.set_limiter_enabled(false);
    assert!(!h.engine.limiter_enabled());
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

// ---------------------------------------------------------------------------
// A7: resampled seams, DSP latency, format changes and errors
// ---------------------------------------------------------------------------

/// A device that grants 48 kHz while every synthetic track is 44.1 kHz: the
/// most common Windows shared-mode setup, and the one that resamples.
const DEVICE_48K: AudioSpec = AudioSpec::new(48_000, 2);

/// An engine on a 48 kHz device with a ring large enough to hold a whole
/// short queue, so a test can buffer everything before it plays.
fn harness_48k(names: &[&str]) -> Harness {
    Harness::with_config(
        names,
        EngineConfig {
            spec: DEVICE_48K,
            ring_frames: 1 << 17,
            ..EngineConfig::default()
        },
    )
}

/// Play `total` buffered frames, return the left channel, and let the run
/// finish.
fn play_out(h: &mut Harness, total: u64) -> Vec<f32> {
    let mut rec = Vec::new();
    assert_eq!(h.clock.pump_record(total, &mut rec), total);
    assert!(
        h.wait_idle(|e| e.transport() == TransportState::Stopped, WAIT),
        "the run must finish once the ring is empty"
    );
    rec.iter().step_by(2).copied().collect()
}

/// A6 finding 1. Two 44.1 kHz tracks that together are one unbroken 1 kHz
/// tone, on a 48 kHz device, with the seam on a crest of the tone.
///
/// What it proves: across a same-format gapless seam the engine's output is
/// the tone, resampled, within 5e-3 (about -40 dB of the tone) at every frame
/// away from the stream's own start and end; the sample step stays within 2 %
/// of the tone's own largest step; the level (RMS per period) is continuous
/// within 1 %; the seam lands exactly at 0.5 s and the queue has exactly the
/// length its duration implies. The flush-and-restart seam this replaces
/// misses the joined signal by about 0.03 next to the seam (estimated, see
/// `convert::a_continued_stream_has_no_seam`), six times the tolerance.
///
/// What it cannot prove: that a seam is inaudible on real material, that a
/// seam between two **different** formats is smooth (it is not: two rates
/// cannot share a resampler, so that seam is still flushed and restarted), or
/// anything about a real device. The tolerance covers the converter's
/// passband error (±0.05 dB is 3e-3 at this level); the true error is
/// expected to be far smaller, but it was not measured on this host.
#[test]
fn a_resampled_gapless_seam_is_continuous() {
    let mut h = harness_48k(&["tone0_22050.tst", "tone22050_22050.tst"]);
    assert_eq!(h.engine.device_spec(), DEVICE_48K);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_buffered(), "both tracks must be buffered");

    // 2 x 22050 frames at 44.1 kHz are exactly 2 x 24000 at 48 kHz.
    let seam = 24_000_u64;
    let total = 2 * seam;
    assert_eq!(h.engine.end_of_stream(), Some(total));
    assert_eq!(h.engine.ring_counters(), (total, 0));
    assert_eq!(h.engine.current_index(), Some(1));

    let left = play_out(&mut h, total);
    assert_eq!(left.len() as u64, total);
    // Track 2 starts exactly at the seam, and both tracks have their length.
    assert_eq!(h.engine.position_frames(), total - seam);
    assert_eq!(h.engine.last_track_frames(), seam);

    let rate = f64::from(DEVICE_48K.sample_rate);
    let ideal =
        |k: usize| TONE_AMPLITUDE * (2.0 * std::f64::consts::PI * TONE_HZ * k as f64 / rate).cos();
    // The stream's own edges (a tone switched on and off from silence) ring;
    // 10 ms is well past the filter's half length (about 3 ms).
    let edge = 480;
    let body = edge..total as usize - edge;

    let (worst_at, worst) = body
        .clone()
        .map(|k| (k, (f64::from(left[k]) - ideal(k)).abs()))
        .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
    assert!(
        worst < 5e-3,
        "the output leaves the resampled tone by {worst} at frame {worst_at} (seam at {seam})"
    );

    let tone_step = 2.0 * TONE_AMPLITUDE * (std::f64::consts::PI * TONE_HZ / rate).sin();
    let max_step = body
        .clone()
        .skip(1)
        .map(|k| (f64::from(left[k]) - f64::from(left[k - 1])).abs())
        .fold(0.0, f64::max);
    assert!(
        max_step < tone_step * 1.02,
        "step {max_step} > {tone_step} (the tone's own largest step)"
    );

    // Level across the seam: the RMS of every whole period around it.
    let period = (rate / TONE_HZ) as usize;
    let target = TONE_AMPLITUDE / 2.0_f64.sqrt();
    let seam = seam as usize;
    for start in (seam - 10 * period..seam + 10 * period).step_by(period / 4) {
        let rms = (left[start..start + period]
            .iter()
            .map(|v| f64::from(*v).powi(2))
            .sum::<f64>()
            / period as f64)
            .sqrt();
        assert!(
            (rms / target - 1.0).abs() < 0.01,
            "level {rms} at frame {start} (seam at {seam}, expected {target})"
        );
    }
}

/// A6 finding 4. With the limiter enabled and a signal it does not have to
/// touch, a gapless queue comes out bit for bit: no look-ahead silence at the
/// start or in the seam, no frame lost at the seam or at the end, and the
/// position is not shifted by the look-ahead.
#[test]
fn the_enabled_limiter_keeps_a_gapless_join_exact_and_undelayed() {
    let mut h = Harness::new(&[TRACK_A, TRACK_B]);
    // 0.5x keeps the pattern (0.1..=1.1) at 0.05..=0.55, below the ceiling,
    // and the product is exact.
    h.engine
        .set_dsp_chain(vec![Box::new(GainBoostDspStage(0.5))]);
    h.engine.set_limiter_enabled(true);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_buffered(), "both tracks must be buffered");

    let a = track_frames(TRACK_A);
    let b = track_frames(TRACK_B);
    let total = a + b;
    assert_eq!(h.engine.end_of_stream(), Some(total));
    assert_eq!(h.engine.ring_counters(), (total, 0));

    let left = play_out(&mut h, total);
    let want: Vec<f32> = expected(1, 0, a)
        .into_iter()
        .chain(expected(2, 0, b))
        .map(|s| s * 0.5)
        .collect();
    assert_eq!(left, want, "the delayed limiter output, re-aligned");
    assert_eq!(h.engine.last_track_frames(), b);
    assert_eq!(h.engine.position_frames(), b);
}

/// A6 finding 5. A mid-stream format change from 44.1 kHz (resampled) to
/// 48 kHz (copied) keeps the resampler's tail: the first part has its full
/// resampled length, and the second part follows it bit for bit.
#[test]
fn a_format_change_keeps_the_resampler_tail() {
    // 4410 frames at 44.1 kHz = 4800 at 48 kHz, then 2400 at 48 kHz.
    let mut h = harness_48k(&["fmt4410_2400.tst"]);
    // Disable limiter to verify bit-exact resampler tail without peak limiting.
    h.engine.set_limiter_enabled(false);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_buffered(), "the track must be buffered");

    let first = 4_800_u64;
    let second = 2_400_u64;
    let total = first + second;
    assert_eq!(
        h.engine.end_of_stream(),
        Some(total),
        "the tail before the change was dropped"
    );
    assert_eq!(h.engine.source_spec(), Some(common::CHANGED_SPEC));
    let left = play_out(&mut h, total);
    assert_eq!(&left[first as usize..], &expected(2, 0, second)[..]);
    assert_eq!(h.engine.last_track_frames(), total);
}

/// A6 finding 7. A decoder error ends the track but keeps what was decoded
/// before it, including the part the resampler was still holding.
#[test]
fn a_decoder_error_keeps_the_audio_decoded_before_it() {
    let mut h = harness_48k(&["err4410_0.tst"]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_buffered(), "the run must reach its end");
    assert_eq!(h.engine.end_of_stream(), Some(4_800));
    assert!(h.engine.last_error().is_some(), "the error is reported");
}

/// RFC 0014 §7.2's stub: a stage that fails loudly with a message after
/// `fail_after` blocks — the engine-side half of M14 finding F1's smallest
/// reproduction. It reports a look-ahead so the end-of-track flush also
/// runs the failing stage (the re-failure path).
struct FailingDspStage {
    fail_after: usize,
    blocks: usize,
}

impl rustamp_core::audio::DspStage for FailingDspStage {
    fn name(&self) -> &str {
        "failing-dsp"
    }

    fn prepare(&mut self, _: AudioSpec) -> rustamp_core::error::Result<()> {
        Ok(())
    }

    fn process(&mut self, buf: &mut [f32]) -> rustamp_core::error::Result<()> {
        self.blocks += 1;
        if self.blocks > self.fail_after {
            return Err(Error::Dsp("clipping detected".into()));
        }
        for s in buf {
            *s *= 0.5;
        }
        Ok(())
    }

    fn reset(&mut self) {}

    fn latency_frames(&self) -> u32 {
        64
    }
}

/// RFC 0014 acceptance 2: a stage returning `Err(Error::Dsp("…"))` mid-run
/// ends the track with `Event::Error` carrying `ErrorKind::Dsp` and the
/// message text, the transport leaves Playing, and no second error is
/// published when the flush path re-fails — the stage fails on every later
/// block, and it reports a look-ahead, so the end-of-track flush runs it
/// again.
#[test]
fn a_stage_error_ends_the_track_loudly_and_only_once() {
    let mut h = Harness::new(&[TRACK_A]);
    // The limiter stays off so the healthy block's halved samples are not
    // clamped; only the failure is under test.
    h.engine.set_limiter_enabled(false);
    h.engine.set_dsp_chain(vec![Box::new(FailingDspStage {
        fail_after: 1,
        blocks: 0,
    })]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    assert!(
        h.wait_buffered(),
        "the run must reach its end (the track ended at the failure)"
    );
    let total = track_frames(TRACK_A);
    let end = h
        .engine
        .end_of_stream()
        .expect("the run must report its end");
    assert!(
        end < total,
        "the track must end at the failure, before its full length ({end} of {total})"
    );
    assert!(
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
        "the transport must leave Playing"
    );
    let info = h
        .engine
        .last_error()
        .expect("the stage error must be reported");
    assert_eq!(info.kind, ErrorKind::Dsp, "{info:?}");
    assert!(info.message.contains("clipping detected"), "{info:?}");
    let events = h.drain_events();
    let errors: Vec<_> = events
        .iter()
        .filter(|event| matches!(event, Event::Error(_)))
        .collect();
    assert_eq!(
        errors.len(),
        1,
        "exactly one error event per run (the flush re-failure must not re-report): {events:?}"
    );
    let Event::Error(published) = errors[0] else {
        unreachable!("filtered to Event::Error")
    };
    assert_eq!(published.kind, ErrorKind::Dsp, "{published:?}");
    assert!(
        published.message.contains("clipping detected"),
        "{published:?}"
    );
}

/// A6 finding 8. With the limiter disabled, a NaN from a stage never reaches the
/// device; every finite sample passes untouched.
#[test]
fn a_non_finite_sample_never_reaches_the_device() {
    struct Poison;
    impl rustamp_core::audio::DspStage for Poison {
        fn name(&self) -> &str {
            "poison"
        }
        fn prepare(&mut self, _: AudioSpec) -> rustamp_core::error::Result<()> {
            Ok(())
        }
        fn process(&mut self, buf: &mut [f32]) -> rustamp_core::error::Result<()> {
            for (i, s) in buf.iter_mut().enumerate() {
                match i % 300 {
                    0 => *s = f32::NAN,
                    101 => *s = f32::INFINITY,
                    _ => {}
                }
            }
            // The stage stays healthy on the contract (RFC 0014 acceptance 4:
            // a stage emitting NaN *and* returning Ok still hits the
            // sanitizer) — the failure channel is pinned by
            // `a_stage_error_ends_the_track_loudly_and_only_once` below.
            Ok(())
        }
        fn reset(&mut self) {}
    }

    let mut h = Harness::new(&[TRACK_A]);
    // Disable limiter so finite samples pass untouched to match pattern values exactly.
    h.engine.set_limiter_enabled(false);
    h.engine.set_dsp_chain(vec![Box::new(Poison)]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_buffered());
    let total = track_frames(TRACK_A);
    let mut rec = Vec::new();
    h.clock.pump_record(total, &mut rec);
    assert!(rec.iter().all(|s| s.is_finite()), "a non-finite sample");
    let silenced = rec.iter().filter(|s| **s == 0.0).count();
    assert!(silenced > 0, "the poisoned samples are silenced");
    for (frame, pair) in rec.as_chunks::<2>().0.iter().enumerate() {
        let want = pattern(1, frame as u64);
        for s in pair {
            assert!(*s == 0.0 || *s == want, "frame {frame}: {s} != {want}");
        }
    }
}

// A14. With the default chain (limiter enabled) on a resampling device, a
// hot signal stays within 0 dBFS end to end, at the exact length the duration
// implies. The 4x boost runs before the limiter (the engine order is EQ ->
// installed stages -> limiter), so the limiter brings the 2.0 sine down to
// its ceiling and the resampler reconstructs it; the converter's final
// ceiling is the bound asserted here.
//
// What it cannot prove: that the safety net engaged (a resampled sine never
// exceeds its own amplitude, so nothing here reaches 1.0), or anything about
// a real device. Engagement is pinned at converter and chain level.
#[test]
fn a_hot_resampled_run_with_the_limiter_holds_0dbfs() {
    let mut h = harness_48k(&["tone0_22050.tst", "tone22050_22050.tst"]);
    h.engine
        .set_dsp_chain(vec![Box::new(GainBoostDspStage(4.0))]);
    assert!(h.engine.limiter_enabled(), "the default chain limits");
    h.engine.play_index(0).unwrap();
    assert!(h.wait_buffered(), "both tracks must be buffered");
    let total = 48_000_u64;
    assert_eq!(h.engine.end_of_stream(), Some(total));
    let left = play_out(&mut h, total);
    let peak = left.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak <= 1.0, "above 0 dBFS after resampling: {peak}");
}

// A14. The guarantee does not depend on the limiter switch: with the limiter
// off, the same hot signal still never exceeds 0 dBFS, and the safety net
// demonstrably engages (crest samples sit at exactly ±1.0).
#[test]
fn a_hot_resampled_run_without_the_limiter_holds_0dbfs() {
    let mut h = harness_48k(&["tone0_22050.tst", "tone22050_22050.tst"]);
    h.engine
        .set_dsp_chain(vec![Box::new(GainBoostDspStage(4.0))]);
    h.engine.set_limiter_enabled(false);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_buffered(), "both tracks must be buffered");
    let total = 48_000_u64;
    assert_eq!(h.engine.end_of_stream(), Some(total));
    let left = play_out(&mut h, total);
    let peak = left.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak <= 1.0, "above 0 dBFS with the limiter off: {peak}");
    let clamped = left.iter().filter(|s| s.abs() == 1.0).count();
    assert!(clamped > 0, "the safety net never engaged");
}

/// A6 finding 2, second variant. A seek requested before the output runs
/// (the initial seek of a run) is not written ahead of the drain, so the
/// audio after it is the source at the target and the counters line up.
#[test]
fn seek_before_play_plays_the_target_audio() {
    let mut h = Harness::new(&[LONG_A]);
    // Disable limiter to verify bit-exact target audio without peak limiting.
    h.engine.set_limiter_enabled(false);
    h.engine.seek_frames(1500).unwrap();
    h.engine.play().unwrap();
    // The drain happens in a render callback: pump until it has.
    let deadline = Instant::now() + WAIT;
    while h.engine.ring_counters().1 < 1500 {
        assert!(Instant::now() < deadline, "the drain never happened");
        h.engine.poll().unwrap();
        h.clock.pump(64);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        h.engine.ring_counters().1,
        1500,
        "consumed is the seek base"
    );
    let p0 = h
        .wait_refilled(4096)
        .expect("the ring must fill after the seek");
    assert_eq!(p0, 1500);
    let mut rec = Vec::new();
    h.clock.pump_record(1024, &mut rec);
    let left: Vec<f32> = rec.iter().step_by(2).copied().collect();
    assert_eq!(&left[..], &expected(1, 1500, 1024)[..]);
    let (written, consumed) = h.engine.ring_counters();
    assert_eq!(consumed, 1500 + 1024);
    assert!(consumed <= written);
}

/// A8: A stopped run leaves its audio in the ring.
///
/// Proves:
/// - after stop() then play_index(0), the recording contains NO frame of the
///   previous run, verified bit for bit against the same recording from a
///   fresh engine;
/// - nothing new is lost: the first frame the new run writes still reaches
///   the device;
/// - buffered_frames() cannot report 0 while the ring holds stale frames
///   from a previous run.
#[test]
fn stopped_run_leaves_no_stale_audio_in_ring_on_restart() {
    // 1. Fresh engine baseline:
    let mut h_fresh = Harness::new(&[LONG_A]);
    h_fresh.engine.set_limiter_enabled(false);
    h_fresh.engine.play_index(0).unwrap();
    assert!(h_fresh.wait_track(0));
    let p_fresh = h_fresh
        .wait_refilled(4096)
        .expect("refill fresh engine run");
    let mut rec_fresh = Vec::new();
    h_fresh.clock.pump_record(2048, &mut rec_fresh);

    // 2. Stopped-then-restarted engine:
    let mut h = Harness::new(&[LONG_A]);
    h.engine.set_limiter_enabled(false);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    let _ = h.wait_refilled(4096).expect("refill run 1");

    // Pump 1024 frames so the ring still holds frames 1024..4096 from the first run.
    h.clock.pump(1024);
    h.engine.poll().unwrap();
    h.engine.stop().unwrap();

    // While stopped, buffered_frames() must honestly report the stale backlog, NOT 0.
    let stale = h.engine.buffered_frames();
    assert!(
        stale > 0,
        "buffered_frames() must report unconsumed stale frames after stop, got {stale}"
    );
    assert_eq!(h.engine.transport(), TransportState::Stopped);

    // Restart the run.
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    assert!(h.engine.is_drain_pending());
    assert_eq!(
        h.engine.buffered_frames(),
        stale,
        "buffered_frames() must report stale frames until consumer acknowledges drain"
    );

    // Pump to execute the drain callback and await decode refill.
    assert!(h.settle_drain());
    assert!(!h.engine.is_drain_pending());

    let p_restarted = h.wait_refilled(4096).expect("refill restarted engine run");
    assert_eq!(p_restarted, p_fresh);
    let mut rec_restarted = Vec::new();
    h.clock.pump_record(2048, &mut rec_restarted);

    // The restarted recording contains NO stale audio from the previous run,
    // bit for bit identical to a completely fresh engine.
    assert_eq!(
        rec_restarted, rec_fresh,
        "restarted run audio must match fresh engine recording bit for bit"
    );

    // First frame written by the new run reached the device (exact source pattern).
    let left: Vec<f32> = rec_restarted.iter().step_by(2).copied().collect();
    assert_eq!(
        &left[..],
        &expected(1, 0, 2048)[..],
        "first frame the new run writes must reach the device"
    );
    let (written, consumed) = h.engine.ring_counters();
    assert_eq!(consumed, 2048);
    assert!(consumed <= written);
}

/// Job 2: Default-enabled limiter keeps a gapless join exact, undelayed,
/// loses no tail, and adds no silence into the seam.
#[test]
fn the_default_limiter_loses_no_tail_and_adds_no_seam_silence() {
    let mut h = Harness::new(&[TRACK_A, TRACK_B]);
    // Note: limiter is NOT explicitly enabled here; it defaults to enabled.
    assert!(h.engine.limiter_enabled());
    assert!(h.engine.limiter_control().enabled());

    // 0.5x boost keeps pattern samples (0.1..=1.1) in range 0.05..=0.55,
    // strictly below the limiter's ceiling, so the limiter stays linear.
    h.engine
        .set_dsp_chain(vec![Box::new(GainBoostDspStage(0.5))]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_buffered(), "both tracks must be buffered");

    let a = track_frames(TRACK_A);
    let b = track_frames(TRACK_B);
    let total = a + b;
    assert_eq!(
        h.engine.end_of_stream(),
        Some(total),
        "total buffered frames must equal the sum of both tracks"
    );
    assert_eq!(h.engine.ring_counters(), (total, 0));

    let left = play_out(&mut h, total);
    let want: Vec<f32> = expected(1, 0, a)
        .into_iter()
        .chain(expected(2, 0, b))
        .map(|s| s * 0.5)
        .collect();
    assert_eq!(
        left, want,
        "the default-enabled limiter must preserve gapless audio bit for bit with no tail lost and no seam silence"
    );
    assert_eq!(h.engine.last_track_frames(), b);
    assert_eq!(h.engine.position_frames(), b);
}

/// Record 2048 frames of `LONG_A` from the start of a run with the default
/// configuration (limiter enabled), either on a fresh engine or after a stop
/// that left the previous run's audio in the ring. Returns the position at
/// the start of the recording and the recording.
fn record_default_start(restart: bool) -> (u64, Vec<f32>) {
    let mut h = Harness::new(&[LONG_A]);
    assert!(h.engine.limiter_enabled());
    if restart {
        h.engine.play_index(0).unwrap();
        assert!(h.wait_track(0));
        h.wait_refilled(4096).expect("refill run 1");
        h.clock.pump(1024);
        h.engine.stop().unwrap();
        assert!(
            h.engine.buffered_frames() > 0,
            "the ring holds run 1's audio"
        );
    }
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    assert!(h.settle_drain());
    let p0 = h.wait_refilled(4096).expect("refill");
    let mut rec = Vec::new();
    h.clock.pump_record(2048, &mut rec);
    (p0, rec)
}

/// A8 with the default chain. The limiter's look-ahead does not shift a
/// restarted run either: the restart renders exactly what a fresh engine
/// renders, from position 0, within the ceiling.
///
/// What it cannot prove: anything about a real device, whose drain callback
/// and first callbacks are timed by the hardware.
#[test]
fn a_restart_with_the_default_limiter_matches_a_fresh_run() {
    let (p_fresh, fresh) = record_default_start(false);
    let (p_restarted, restarted) = record_default_start(true);
    assert_eq!(p_fresh, 0);
    assert_eq!(p_restarted, 0);
    assert_eq!(
        restarted, fresh,
        "the restart renders what a fresh run does"
    );
    let ceiling = 10.0_f64.powf(rustamp_dsp::limiter::CEILING_DB / 20.0) as f32;
    assert!(restarted.iter().all(|s| s.abs() <= ceiling));
}

/// A8: a restart that is paused before its drain happens writes nothing of
/// the new run until the drain, which happens on the first callback after the
/// resume; the run then starts exactly at its first frame.
#[test]
fn a_restart_paused_before_its_drain_starts_clean() {
    let mut h = Harness::new(&[LONG_A]);
    h.engine.set_limiter_enabled(false);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    h.wait_refilled(4096).expect("refill run 1");
    h.clock.pump(1024);
    h.engine.stop().unwrap();

    h.engine.play_index(0).unwrap();
    h.engine.pause().unwrap();
    assert_eq!(h.clock.pump(1024), 0, "a paused device pulls nothing");
    assert!(
        h.wait_idle(|e| e.is_drain_pending() && e.is_start_pending(), WAIT),
        "stall: drain and start were never pending after restart"
    );
    assert_eq!(
        h.engine.ring_counters(),
        (0, 0),
        "nothing of the new run is written before the drain"
    );

    h.engine.play().unwrap();
    assert!(h.settle_drain());
    let p0 = h.wait_refilled(4096).expect("refill after the resume");
    assert_eq!(p0, 0);
    let mut rec = Vec::new();
    h.clock.pump_record(1024, &mut rec);
    let left: Vec<f32> = rec.iter().step_by(2).copied().collect();
    assert_eq!(&left[..], &expected(1, 0, 1024)[..]);
    let (written, consumed) = h.engine.ring_counters();
    assert_eq!(consumed, 1024);
    assert!(consumed <= written);
}

/// A run that ended naturally leaves an empty ring, so the next start needs
/// no drain and buffers without a single callback.
#[test]
fn a_restart_after_a_natural_end_needs_no_drain() {
    let mut h = Harness::new(&[TRACK_A]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_buffered());
    assert!(h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT));
    assert_eq!(h.engine.buffered_frames(), 0);

    h.engine.play_index(0).unwrap();
    assert!(!h.engine.is_start_pending());
    assert!(!h.engine.is_drain_pending());
    assert!(h.wait_buffered(), "the run buffers without a callback");
}

/// A stopped run flushes the output device so that the device drops what
/// was buffered below the ring (F3).
#[test]
fn stopped_run_flushes_output_device() {
    let mut h = Harness::new(&[LONG_A]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    h.clock.pump(512);
    h.engine.poll().unwrap();
    assert_eq!(h.clock.flushes(), 0, "no flush during normal playback");

    h.engine.stop().unwrap();
    assert!(
        h.clock.flushes() >= 1,
        "stop() must flush the output device"
    );
}

/// A seek drops what was buffered in the device before the seek target (F3).
#[test]
fn seek_flushes_output_device() {
    let mut h = Harness::new(&[LONG_A]);
    h.engine.set_limiter_enabled(false);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    h.clock.pump(512);
    h.engine.poll().unwrap();
    let flushes_before = h.clock.flushes();

    h.engine.seek_frames(1500).unwrap();
    assert!(h.settle_seek(1500));
    assert!(
        h.clock.flushes() > flushes_before,
        "a seek must flush the output device on drain"
    );
}

/// A seek while stopped flushes the output device so any stale buffer is dropped.
#[test]
fn seek_while_stopped_flushes_output_device() {
    let mut h = Harness::new(&[LONG_A]);
    assert_eq!(h.clock.flushes(), 0);
    h.engine.seek_frames(500).unwrap();
    assert_eq!(h.clock.flushes(), 1);
}

/// When a run ends naturally, the output device is flushed when entering Stopped state.
#[test]
fn natural_end_flushes_output_device() {
    let mut h = Harness::new(&[TRACK_A]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_buffered());
    assert!(h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT));
    assert!(
        h.clock.flushes() >= 1,
        "finishing a run must flush the output device"
    );
}

/// Simulates a device with an internal hardware/driver buffer (such as the WASAPI
/// endpoint buffer) to prove headlessly that without `flush`, a stop followed
/// by a play would replay old audio below the ring, and that calling `flush`
/// on stop drops the device buffer.
#[test]
fn buffered_device_drops_tail_on_stop_preventing_stale_replay() {
    struct BufferedDeviceInner {
        spec: Option<AudioSpec>,
        render: Option<RenderFn>,
        device_buf: Vec<f32>,
        paused: bool,
        flushes: u64,
        played: u64,
    }

    struct BufferedOutput {
        inner: Arc<Mutex<BufferedDeviceInner>>,
    }

    struct BufferedClock {
        inner: Arc<Mutex<BufferedDeviceInner>>,
    }

    impl OutputDevice for BufferedOutput {
        fn name(&self) -> &str {
            "buffered-mock"
        }
        fn open(&mut self, spec: AudioSpec) -> Result<OutputFormat> {
            let mut g = self.inner.lock().unwrap();
            g.spec = Some(spec);
            g.device_buf.clear();
            g.flushes = 0;
            g.played = 0;
            Ok(OutputFormat {
                spec,
                buffer_frames: 256,
            })
        }
        fn start(&mut self, render: RenderFn) -> Result<()> {
            let mut g = self.inner.lock().unwrap();
            g.render = Some(render);
            g.paused = false;
            Ok(())
        }
        fn pause(&mut self) -> Result<()> {
            let mut g = self.inner.lock().unwrap();
            g.paused = true;
            // Crucial: pause does NOT drop the device buffer (retained, like WASAPI)
            Ok(())
        }
        fn resume(&mut self) -> Result<()> {
            let mut g = self.inner.lock().unwrap();
            g.paused = false;
            Ok(())
        }
        fn flush(&mut self) -> Result<()> {
            let mut g = self.inner.lock().unwrap();
            g.flushes += 1;
            g.device_buf.clear();
            Ok(())
        }
        fn latency(&self) -> Duration {
            Duration::from_millis(10)
        }
        fn played_frames(&self) -> u64 {
            self.inner.lock().unwrap().played
        }
        fn close(&mut self) -> Result<()> {
            let mut g = self.inner.lock().unwrap();
            g.render = None;
            Ok(())
        }
    }

    impl BufferedClock {
        /// Pull `frames` frames from the mock hardware buffer to the speaker.
        /// If the buffer has fewer frames than requested, it pulls from `render`
        /// to top it up (like WASAPI's render thread topping up its endpoint buffer).
        fn pull_hardware(&self, frames: usize, out: &mut Vec<f32>) -> usize {
            let mut g = self.inner.lock().unwrap();
            if g.paused || g.render.is_none() {
                return 0;
            }
            let channels = usize::from(g.spec.unwrap().channels);
            let samples_wanted = frames * channels;
            // Top up device buffer to at least samples_wanted
            if g.device_buf.len() < samples_wanted {
                let needed = samples_wanted - g.device_buf.len();
                let mut scratch = vec![0.0; needed];
                if let Some(render) = g.render.as_mut() {
                    render(&mut scratch);
                }
                g.device_buf.extend_from_slice(&scratch);
            }
            let take_samples = samples_wanted.min(g.device_buf.len());
            let taken: Vec<f32> = g.device_buf.drain(..take_samples).collect();
            let frames_taken = taken.len() / channels;
            g.played += frames_taken as u64;
            out.extend_from_slice(&taken);
            frames_taken
        }
    }

    let inner = Arc::new(Mutex::new(BufferedDeviceInner {
        spec: None,
        render: None,
        device_buf: Vec::new(),
        paused: false,
        flushes: 0,
        played: 0,
    }));
    let output = BufferedOutput {
        inner: Arc::clone(&inner),
    };
    let clock = BufferedClock {
        inner: Arc::clone(&inner),
    };

    let dir = tempfile::tempdir().expect("temp dir");
    let path_a = dir.path().join("track1_44100.tst");
    let path_b = dir.path().join("track2_44100.tst");
    std::fs::write(&path_a, b"").unwrap();
    std::fs::write(&path_b, b"").unwrap();
    let mut reg = rustamp_core::audio::DecoderRegistry::new();
    reg.register(common::TestFactory::arc());
    let bus = rustamp_core::bus::EventBus::new();
    let mut engine = rustamp_audio::Engine::new(
        reg,
        Box::new(output),
        bus,
        rustamp_audio::EngineConfig::default(),
    )
    .unwrap();
    engine.set_limiter_enabled(false);
    engine.set_queue(vec![
        rustamp_audio::TrackSource::path(path_a.display().to_string()),
        rustamp_audio::TrackSource::path(path_b.display().to_string()),
    ]);

    // 1. Play track 1
    engine.play_index(0).unwrap();
    // Every wait in this test is a bounded condition (the file's Harness
    // helpers use the same 30 s deadline): a condition that never becomes
    // true must fail loudly with the step named, never spin forever.
    let deadline = std::time::Instant::now() + WAIT;
    while engine.current_index() != Some(0) || engine.duration_frames().is_none() {
        engine.poll().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "stall: track 1 never opened"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    // Pull 256 frames of track 1 audio through hardware
    let mut rec1 = Vec::new();
    assert_eq!(clock.pull_hardware(256, &mut rec1), 256);
    engine.poll().unwrap();
    // Device buffer now holds top-up audio of track 1:
    {
        let mut g = clock.inner.lock().unwrap();
        let mut topup = vec![0.0; 256 * 2];
        if let Some(render) = g.render.as_mut() {
            render(&mut topup);
        }
        g.device_buf.extend_from_slice(&topup);
        assert!(
            !g.device_buf.is_empty(),
            "device buffer holds track 1 audio"
        );
    }

    // 2. Stop playback!
    engine.stop().unwrap();
    // Stop() called output.flush(), which cleared the device buffer!
    assert_eq!(inner.lock().unwrap().flushes, 1);
    assert!(
        inner.lock().unwrap().device_buf.is_empty(),
        "device buffer must be cleared by flush on stop"
    );

    // 3. Play track 2
    engine.play_index(1).unwrap();
    let deadline = std::time::Instant::now() + WAIT;
    while engine.current_index() != Some(1) || engine.duration_frames().is_none() {
        engine.poll().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "stall: track 2 never opened"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    // Wait for decode thread to refill the ring
    let deadline = std::time::Instant::now() + WAIT;
    while engine.buffered_frames() < 1024 {
        engine.poll().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "stall: the ring never refilled after the restart"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    // A8: the new run starts by draining what the stopped run left in the ring,
    // and the drain callback is silent by design; the decode thread writes the
    // new run only after it. Pulling that callback and proving it silent is the
    // stronger half of this test's claim: a callback that still carried track 1
    // audio would be exactly the stale replay the test exists to catch.
    let deadline = std::time::Instant::now() + WAIT;
    let mut drained = 0usize;
    while engine.is_start_pending() {
        let mut chunk = Vec::new();
        assert_eq!(clock.pull_hardware(256, &mut chunk), 256);
        assert!(
            chunk.iter().all(|sample| *sample == 0.0),
            "the start drain callback must be silent, never stale track 1 audio"
        );
        drained += chunk.len();
        engine.poll().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "the start drain never settled"
        );
    }
    assert!(
        drained > 0,
        "the stopped run left audio in the ring, so a start drain was due"
    );
    // Only now does the new run's own audio reach the device.
    let deadline = std::time::Instant::now() + WAIT;
    while engine.buffered_frames() < 256 {
        engine.poll().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "stall: the decode thread wrote nothing after the start drain"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    // Pull audio of track 2 from the device
    let mut rec2 = Vec::new();
    assert_eq!(clock.pull_hardware(256, &mut rec2), 256);
    let left2: Vec<f32> = rec2.iter().step_by(2).copied().collect();

    // Verify: the audio heard from the new run belongs to track 2, NOT track 1!
    let track1_expected = expected(1, 0, 256);
    let track2_expected = expected(2, 0, 256);
    assert_ne!(left2, track1_expected, "must NOT replay track 1 audio!");
    assert_eq!(left2, track2_expected, "must play track 2 audio cleanly!");
}
