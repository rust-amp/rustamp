//! Two-process checks: ignored, headless and VM-only. Build/stage the
//! decoder_fixture example and set RUSTAMP_DECODER_FIXTURE to its guest path.

#![cfg(windows)]

#[path = "support/frame_pattern.rs"]
mod frame_pattern;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use rustamp_mediahost::MediaHostError;
use rustamp_mediahost::control::Liveness;
use rustamp_mediahost::decoder::{DecoderProcess, DecoderSession};
use rustamp_mediahost::pcm::NamedPcmRings;
use rustamp_proto::host::{MediaLocationWire, PcmSpecWire};

/// Stall bound for the functional PCM read: the fixture delivers within
/// milliseconds, so a timeout means the helper stopped answering without a
/// terminal signal — a stall that fails loudly, never a pass.
const STALL_WAIT: Duration = Duration::from_secs(30);
/// Liveness policy for the test's own control peer. The framework default
/// (`Liveness::default()`: 100 ms interval, 5 s timeout) is a wall-clock
/// bound a healthy-but-slow VM schedule can trip — the flake class M24
/// proved in this guest (M24 §1.3, M25). A dead helper never answers a
/// Ping and is caught by the control EOF regardless, so only the trip wire
/// moves.
const LIVENESS: Liveness = Liveness {
    interval: Duration::from_millis(100),
    timeout: STALL_WAIT,
};

fn fixture() -> PathBuf {
    PathBuf::from(
        std::env::var_os("RUSTAMP_DECODER_FIXTURE")
            .expect("VM must stage decoder_fixture and set its absolute path"),
    )
}

/// Read PCM with a stall escape. A healthy helper delivers within
/// milliseconds, so a timeout means it stopped answering without a terminal
/// signal; that is a stall that fails loudly, never a pass.
fn read_pcm(session: &mut DecoderSession<NamedPcmRings>, out: &mut [f32], step: &str) -> usize {
    match session.read(out, Some(STALL_WAIT)) {
        Err(MediaHostError::TimedOut) => panic!(
            "stall: {step} received no PCM within {STALL_WAIT:?} \
             (the resident stopped delivering without EOS or an error)"
        ),
        other => other.unwrap(),
    }
}

/// Verify that the frames the consumer received are whole and in order.
///
/// The fixture publishes frame `index` as `[index / 8, -index / 8]`. A helper
/// that died in the middle of writing a frame would leave a torn, missing or
/// reordered frame behind, and the pattern is what shows it.
fn check_frames(out: &[f32], frames: usize, delivered: &mut usize) {
    for chunk in out[..frames * 2].as_chunks::<2>().0 {
        assert_eq!(
            *chunk,
            frame_pattern::frame_samples(*delivered),
            "frame {delivered} was torn, missing or out of order"
        );
        *delivered += 1;
    }
}

#[test]
#[ignore = "two processes; Hyper-V VM only; requires RUSTAMP_DECODER_FIXTURE"]
fn named_pcm_two_process_transfer_and_seek() {
    let mut child = DecoderProcess::spawn(
        &fixture(),
        MediaLocationWire::Url("fixture:plain".into()),
        3,
        LIVENESS,
    )
    .unwrap();
    assert_ne!(child.pid(), std::process::id());
    assert_eq!(child.session().seek(29, STALL_WAIT).unwrap(), 29);
    let mut frame = 29;
    let mut out = [0.0; 22];
    loop {
        // Stall escape (see STALL_WAIT): a timeout is a loud failure that
        // names the step, never a pass.
        let count = match child.session().read(&mut out, Some(STALL_WAIT)) {
            Err(MediaHostError::TimedOut) => panic!(
                "stall: the fixture delivered no PCM within {STALL_WAIT:?} \
                 (it stopped answering without EOS or an error)"
            ),
            other => other.unwrap(),
        };
        if count == 0 {
            break;
        }
        for samples in out[..count * 2].as_chunks::<2>().0 {
            assert_eq!(*samples, [frame as f32 / 8.0, -(frame as f32 / 8.0)]);
            frame += 1;
        }
    }
    assert_eq!(frame, 4099);
    assert_eq!(child.session().frames_dropped(), 0);
}

#[test]
#[ignore = "abrupt resident-process exit; Hyper-V VM only; requires RUSTAMP_DECODER_FIXTURE"]
fn host_exits_mid_stream_and_infinite_read_survives() {
    let executable = fixture();
    let (done, result) = mpsc::channel();
    std::thread::spawn(move || {
        let mut child = DecoderProcess::spawn(
            &executable,
            MediaLocationWire::Url("fixture:exit".into()),
            3,
            LIVENESS,
        )
        .unwrap();
        let mut received = 0;
        let mut out = [0.0; 14];
        loop {
            match child.session().read(&mut out, None) {
                Ok(0) => panic!("unexpected process exit is not EOS"),
                Ok(count) => received += count,
                Err(error) => {
                    assert!(matches!(error, MediaHostError::Disconnected(_)));
                    assert!(received > 0 && received < 4099);
                    let _ = done.send(());
                    break;
                }
            }
        }
    });
    result
        .recv_timeout(Duration::from_secs(10))
        .expect("dead host must not hang the player worker");
}

/// The kill-the-host acceptance: the helper is terminated from the outside
/// while it streams.
///
/// The kill is a `TerminateProcess` from the parent: no clean exit, no
/// terminal message, no ring close - the helper leaves no goodbye at all.
/// The consumer must then report a loud, typed error - never a silent end of
/// stream, never a hang - and every frame it did receive must be whole: a
/// helper that died in the middle of writing a frame must not leave a torn
/// one behind.
#[test]
#[ignore = "two processes; Hyper-V VM only; requires RUSTAMP_DECODER_FIXTURE"]
fn killing_the_helper_mid_stream_is_a_loud_error_and_never_a_torn_frame() {
    // One frame of prefetch, so the helper is inside its publish path while
    // it streams and the kill lands mid-stream.
    let mut child = DecoderProcess::spawn(
        &fixture(),
        MediaLocationWire::Url("fixture:plain".into()),
        1,
        LIVENESS,
    )
    .unwrap();
    assert_ne!(child.pid(), std::process::id());
    // The handshake is complete when spawn returns: the helper answered with
    // its format, offered the ring and received the RingReady.
    assert_eq!(
        child.session().info().spec,
        PcmSpecWire {
            sample_rate_hz: 1000,
            channels: 2,
        }
    );

    // Take a few frames while the helper streams.
    let mut out = [0.0; 8];
    let mut delivered = 0usize;
    while delivered < 4 {
        let frames = read_pcm(child.session(), &mut out, "the blocks before the kill");
        check_frames(&out, frames, &mut delivered);
    }

    // Kill the helper from the outside, mid-stream.
    child.terminate().unwrap();

    // The consumer must now fail loudly: a typed error, never a silent end of
    // stream and never a hang. The bounded read turns a hang into a loud
    // failure that names the step, never into a pass.
    loop {
        match child.session().read(&mut out, Some(STALL_WAIT)) {
            Ok(0) => panic!("the killed helper must not look like end of stream"),
            Ok(frames) => check_frames(&out, frames, &mut delivered),
            Err(MediaHostError::TimedOut) => panic!(
                "stall: the resident stopped delivering without EOS or an error \
                 (the killed helper was not noticed within {STALL_WAIT:?})"
            ),
            Err(error) => {
                let MediaHostError::Disconnected(reason) = &error else {
                    panic!("the killed helper must report a loud, typed error: {error}");
                };
                assert!(
                    reason.contains("control stream ended"),
                    "the death must be seen on the control channel: {error}"
                );
                break;
            }
        }
    }

    // The failure must stick: a later read may not turn into end of stream.
    let again = match child.session().read(&mut out, Some(STALL_WAIT)) {
        Ok(frames) => panic!(
            "the failure must stick: the killed helper turned into end of stream ({frames} frames)"
        ),
        Err(error) => error,
    };
    let MediaHostError::Disconnected(_) = &again else {
        panic!("the failure must stick as the same loud error: {again}");
    };
    assert!(
        delivered >= 4,
        "the helper must have streamed before the kill, not died before it"
    );
    assert_eq!(child.session().frames_dropped(), 0);
}
