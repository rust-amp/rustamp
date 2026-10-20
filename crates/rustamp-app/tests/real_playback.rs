// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Real playback on a real output device: **VM-only, `#[ignore]`d**.
//!
//! Every other test drives the **null** output; this is the one test that
//! exercises the production device path (`OutputChoice::Default` →
//! `rustamp_output::default_output()`, i.e. WASAPI shared mode with a `cpal`
//! fallback). It opens the real device, plays the repo's own 1.5 s FLAC
//! fixture to its end, and asserts what can be asserted without a microphone:
//!
//! * the device opened, and it is **not** the null output;
//! * the granted format is sane (the engine converts to whatever it grants);
//! * the engine advanced: `played_frames` grew (only a live device thread
//!   advances that clock — the null clock would need pumping, which never
//!   happens here), the timeline reached the end of the track, and the ring
//!   was fully consumed;
//! * playback completed on its own (`Playing` → `Stopped`, no error) and the
//!   engine stops cleanly afterwards.
//!
//! What this test **cannot** prove is stated plainly: whether the engine's
//! output really reaches a speaker cannot be proven from software alone —
//! there is no microphone or loopback capture in the loop. What would prove
//! it: a WASAPI loopback client recording the endpoint mix while this test
//! runs (and comparing it against the fixture), or a human listening in the
//! VM. A passing run proves the whole software path up to the device buffer.
//!
//! If the VM has **no audio endpoint**, this test fails loudly (it never
//! passes vacuously and never skips silently): enabling audio in the VM — a
//! virtual audio device or the Hyper-V audio redirection — is what it needs.
//!
//! This test opens **no window**: the `App` is driven in-process and
//! `window::run` is never called. It does play **audible audio** at the
//! configured volume, which is why it is gated on the headless test VM like
//! the window tests. Run it only inside the headless test VM:
//!
//! ```text
//! cargo test -p rustamp-app --test real_playback -- --ignored
//! ```
//!
//! The fixture is the repo's own 1.5 s, 440 Hz FLAC (generated with ffmpeg,
//! never downloaded; see `crates/rustamp-decoders/tests/fixtures/FIXTURES.txt`
//! and the `fixture()` helper in `tests/headless.rs`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_core::command::Command;
use rustamp_core::config::Config;
use rustamp_core::playlist::MediaLocation;
use rustamp_core::state::TransportState;

// The shared test-VM gate lives in `rustamp_app::window::require_test_vm`:
// this file keeps no copy of it. The gate has deliberately **no** override:
// audible audio must never play on the host.

/// How long the 1.5 s fixture may take to finish on a loaded VM.
const DEADLINE: Duration = Duration::from_secs(30);

/// Pump interval while waiting for the transport to settle.
const PUMP_INTERVAL: Duration = Duration::from_millis(10);

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("rustamp-decoders")
        .join("tests")
        .join("fixtures")
        .join("sine.flac")
}

/// Plays the reference file on the real device, to its end.
///
/// Honest by construction: every assertion below would fail with no device,
/// with the null output, or with a stalled stream. Nothing here can pass
/// without the device pulling the engine's audio to completion.
#[test]
#[ignore = "VM-only: plays audible audio on the real output device; run inside the headless test VM"]
fn the_real_device_plays_a_reference_file_to_its_end() {
    // The gate, before any device exists.
    rustamp_app::window::require_test_vm();

    let fixture = fixture();
    assert!(
        fixture.exists(),
        "missing fixture {} (generated with ffmpeg; see crates/rustamp-decoders/tests/fixtures/FIXTURES.txt)",
        fixture.display()
    );

    // The production output choice: WASAPI shared mode, `cpal` fallback.
    // This is the path no other test takes — everything else sets
    // `OutputChoice::Null`.
    let options = AppOptions {
        config: Config::default(),
        skin: rustamp_app::skin::builtin(),
        output: OutputChoice::Default,
        volume: 200,
        media: Some(MediaLocation::File(fixture.clone())),
        seconds: None,
    };
    let mut app = match App::new(options) {
        Ok(app) => app,
        Err(error) => panic!(
            "NO AUDIO ENDPOINT: the real output device could not be opened ({error}). \
             This test needs an audio endpoint in the headless test VM (a virtual audio \
             device or the Hyper-V audio redirection). It fails loudly by design: without \
             a device there is nothing to verify, and passing anyway would be a lie."
        ),
    };

    // Not the null output: with `OutputChoice::Default` no virtual clock
    // exists, so `played_frames` below can only advance on a live device.
    assert!(
        !app.headless(),
        "the app must not be headless here: `OutputChoice::Default` has no virtual clock"
    );
    let name = app.engine().output_name().to_owned();
    assert_ne!(
        name, "null",
        "the device must be real (`default_output()`), not the null output"
    );
    assert!(
        !name.is_empty(),
        "the opened device must report a name (got an empty one)"
    );

    // The granted format: WASAPI grants exactly what the engine asks for
    // (44.1 kHz stereo); the `cpal` fallback grants its own configuration and
    // the engine converts. Either way the spec must be usable.
    let granted = app.engine().device_spec();
    assert!(
        granted.sample_rate > 0,
        "the device granted a zero sample rate on {name}"
    );
    assert!(
        (1..=2).contains(&granted.channels),
        "the device granted {} channels on {name}; the engine expects 1 or 2",
        granted.channels
    );

    // Start playback of the queued fixture.
    app.submit(Command::Play)
        .expect("starting playback on the real device");
    assert_eq!(
        app.engine().transport(),
        TransportState::Playing,
        "the engine must be Playing right after Play"
    );

    // Let the device pull: the callback runs on the device's own thread, so
    // the test only pumps commands/events until the engine finishes the run
    // by itself (`Playing` → `Stopped` at the end of the queue).
    let start = Instant::now();
    while app.engine().transport() != TransportState::Stopped {
        app.pump().expect("pumping the app while the device plays");
        if start.elapsed() > DEADLINE {
            panic!(
                "the real device never finished the 1.5 s fixture within {DEADLINE:?} \
                 (transport {:?}, played {} frames, timeline {} frames, position {} ms, \
                 last error {:?}, device {name} @ {} Hz/{} ch). A stalled device is a \
                 failure, not a skip: check the VM's audio endpoint.",
                app.engine().transport(),
                app.engine().played_frames(),
                app.engine().timeline_frames(),
                app.engine().position_ms(),
                app.engine().last_error(),
                granted.sample_rate,
                granted.channels,
            );
        }
        std::thread::sleep(PUMP_INTERVAL);
    }
    app.pump().expect("draining the final events");

    // The device pulled real audio: only a running device thread advances
    // this clock, and nothing in this test pumps a virtual one.
    let played = app.engine().played_frames();
    assert!(
        played > 0,
        "the device consumed no frames on {name}: the engine's audio never reached it"
    );

    // The decode thread opened the real fixture and reported its length, and
    // the timeline ran to (at least) the end of the track.
    let duration = app.engine().duration_frames().unwrap_or_else(|| {
        panic!(
            "the engine never learned the fixture's length: the decode of {} produced no track",
            fixture.display()
        )
    });
    assert!(duration > 0, "a zero-length track proves nothing");
    assert!(
        app.engine().timeline_frames() >= duration,
        "the timeline stopped at {} of {duration} frames: playback did not complete",
        app.engine().timeline_frames()
    );

    // The position sits at the end of the track, and the ring holds nothing
    // anymore: every decoded frame was rendered through the device.
    let state = app.engine().state();
    let expected_ms = state.duration_ms.unwrap_or(1500);
    assert!(
        app.engine().position_ms() + 25 >= expected_ms,
        "position {} ms never reached the end ({expected_ms} ms)",
        app.engine().position_ms()
    );
    assert_eq!(
        app.engine().buffered_frames(),
        0,
        "the ring still holds frames after the run: they were decoded but never rendered"
    );

    // The fixture decoded without damage and the engine stops cleanly. (The
    // `App`/`Engine` drop closes the device; `Stop` exercises the same pause,
    // decode-thread join and state reset the close path relies on, through the
    // public command path — `App` exposes the engine only by shared reference.)
    assert!(
        app.engine().last_error().is_none(),
        "the real decode failed: {:?}",
        app.engine().last_error()
    );
    app.submit(Command::Stop)
        .expect("stopping the engine after real playback");
    assert_eq!(app.engine().transport(), TransportState::Stopped);
    assert_eq!(app.engine().position_frames(), 0);
}
