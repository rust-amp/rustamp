//! Two-process DSP checks: ignored, headless and VM-only. Build/stage the
//! `dsp_fixture` example and set RUSTAMP_DSP_FIXTURE to its guest path.
//!
//! The round trip runs in the same lockstep as the in-process suite (see
//! docs/tasks/M20-roundtrip-rebuild.md): write one bounded block, read that
//! block back, repeat — the producer and the consumer progress together, so
//! the fixed 4096-frame back-ring never has to absorb the sweep without a
//! reader, and a wedged helper fails loudly instead of hanging the test.
//!
//! The kill-the-host acceptance (the DSP half of M14 §4): the helper is
//! terminated from the outside while it waits for the next block of the
//! in-ring. The decoder path's kill is in the *write* direction, so the two
//! paths are not evidence for each other (M14 L2).

#![cfg(windows)]

#[path = "support/frame_pattern.rs"]
mod frame_pattern;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use rustamp_mediahost::MediaHostError;
use rustamp_mediahost::control::Liveness;
use rustamp_mediahost::dsp::{DspProcess, DspSession};
use rustamp_mediahost::pcm::NamedPcmRings;
use rustamp_proto::host::{DspStageDesc, PcmSpecWire};

const SPEC: PcmSpecWire = PcmSpecWire {
    sample_rate_hz: 44_100,
    channels: 2,
};

/// Stall bound for the functional PCM waits: the fixture delivers within
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
        std::env::var_os("RUSTAMP_DSP_FIXTURE")
            .expect("VM must stage dsp_fixture and set its absolute path"),
    )
}

fn gain(factor: f32) -> DspStageDesc {
    DspStageDesc {
        id: "gain".into(),
        params: factor.to_le_bytes().to_vec(),
    }
}

/// A chain that changes nothing, so the frames that come back are the
/// frames that went in — the shared frame pattern stays visible.
fn passthrough() -> DspStageDesc {
    DspStageDesc {
        id: "passthrough".into(),
        params: Vec::new(),
    }
}

/// Write PCM with a stall escape (see STALL_WAIT).
fn write_pcm(session: &mut DspSession<NamedPcmRings>, buf: &[f32], step: &str) {
    match session.write_timeout(buf, Some(STALL_WAIT)) {
        Err(MediaHostError::TimedOut) => panic!(
            "stall: {step} could not write within {STALL_WAIT:?} \
             (the fixture stopped draining the in-ring without a close or an error)"
        ),
        other => other.unwrap(),
    }
}

/// Read PCM with a stall escape (see STALL_WAIT).
fn read_pcm(session: &mut DspSession<NamedPcmRings>, out: &mut [f32], step: &str) -> usize {
    match session.read(out, Some(STALL_WAIT)) {
        Err(MediaHostError::TimedOut) => panic!(
            "stall: {step} received no PCM within {STALL_WAIT:?} \
             (the fixture stopped answering without a close or an error)"
        ),
        other => other.unwrap(),
    }
}

#[test]
#[ignore = "two processes; Hyper-V VM only; requires RUSTAMP_DSP_FIXTURE"]
fn named_dsp_two_process_gain_round_trip_then_sticky_eos() {
    let mut child = DspProcess::spawn(&fixture(), SPEC, vec![gain(2.0)], 3, LIVENESS).unwrap();
    assert_ne!(child.pid(), std::process::id());
    assert_eq!(child.session().latency_frames(), 0);
    let total = 4099;
    let mut received = Vec::new();
    let mut out = [0.0; 257 * 2];
    for block in 0..total / 257 {
        let start = block * 257;
        let end = start + 257;
        let mut samples = Vec::with_capacity(257 * 2);
        for frame in start..end {
            samples.push(frame as f32 / 8.0);
            samples.push(-(frame as f32 / 8.0));
        }
        // Lockstep: the bounded write and the read-back of the same block
        // keep producer and consumer progressing together.
        write_pcm(child.session(), &samples, "the lockstep write");
        let mut got = 0;
        while got < 257 {
            let frames = read_pcm(child.session(), &mut out, "the lockstep read");
            assert!(frames > 0, "fixture must not end the stream early");
            received.extend_from_slice(&out[..frames * 2]);
            got += frames;
        }
    }
    child.session().finish_writes().unwrap();
    assert_eq!(received.len(), (total / 257) * 257 * 2);
    for (index, sample) in received.iter().enumerate() {
        let frame = index / 2;
        let sign = if index % 2 == 0 { 1.0 } else { -1.0 };
        assert_eq!(*sample, sign * frame as f32 / 4.0);
    }
    assert_eq!(child.session().frames_dropped(), 0);
    // Sticky end of stream only after the drain: the first read after
    // finish_writes is the drained close, the second sticks there.
    assert_eq!(
        read_pcm(child.session(), &mut out, "the drained EOS read"),
        0
    );
    assert_eq!(
        read_pcm(child.session(), &mut out, "the sticky EOS read"),
        0
    );
}

#[test]
#[ignore = "abrupt resident-process exit; Hyper-V VM only; requires RUSTAMP_DSP_FIXTURE"]
fn host_exits_mid_stream_and_infinite_read_survives() {
    let executable = fixture();
    let (done, result) = mpsc::channel();
    std::thread::spawn(move || {
        let mut child = DspProcess::spawn(
            &executable,
            SPEC,
            vec![DspStageDesc {
                id: "fixture:exit".into(),
                params: Vec::new(),
            }],
            3,
            LIVENESS,
        )
        .unwrap();
        write_pcm(
            child.session(),
            &[0.25, -0.25],
            "the block before the fixture exit",
        );
        let mut out = [0.0; 2];
        loop {
            match child.session().read(&mut out, None) {
                Ok(0) => panic!("unexpected process exit is not EOS"),
                Ok(_) => {}
                Err(error) => {
                    assert!(matches!(error, MediaHostError::Disconnected(_)));
                    let _ = done.send(());
                    break;
                }
            }
        }
    });
    result
        .recv_timeout(Duration::from_secs(10))
        .expect("dead DSP host must not hang the player worker");
}

/// The kill-the-host acceptance for the DSP path: the helper is terminated
/// from the outside while it waits for the next block of the in-ring.
///
/// The kill is a `TerminateProcess` from the parent: no clean exit, no
/// terminal message, no ring close - the helper leaves no goodbye at all.
/// The consumer must then report a loud, typed error - never a silent end
/// of stream, never a hang - and every frame it did receive must be whole.
///
/// The DSP path has no end-of-stream marker (RFC 0006 §6): the end of the
/// transfer is inferred from the frame counts, and the helper returned
/// exactly as many frames as it was given. A closed back-ring would
/// therefore look like a complete transfer, so the death has to be seen on
/// the control channel first - that is what this test pins, and the
/// decoder path cannot have it: `DecoderEos` is a marker message.
///
/// The write direction is pinned too: after the kill a write must fail
/// loudly. A write that returned `Ok(())` would report a delivered frame
/// for a transfer that never happened - the in-ring still has room, and
/// nobody will ever read the frame from it.
#[test]
#[ignore = "two processes; Hyper-V VM only; requires RUSTAMP_DSP_FIXTURE"]
fn killing_the_dsp_helper_mid_stream_is_a_loud_error_and_never_a_torn_frame() {
    // A one-frame in-ring, so the helper is inside its read wait when the
    // kill lands: it has taken every frame the consumer wrote and waits
    // for the next one.
    let mut child = DspProcess::spawn(&fixture(), SPEC, vec![passthrough()], 1, LIVENESS).unwrap();
    assert_ne!(child.pid(), std::process::id());
    // The handshake is complete when spawn returns: the helper answered
    // with its latency and both rings are live.
    assert_eq!(child.session().latency_frames(), 0);

    // A few frames through the chain while the helper waits for the next
    // block.
    let mut out = [0.0; 2];
    let mut delivered = 0usize;
    while delivered < 4 {
        write_pcm(
            child.session(),
            &frame_pattern::frame_samples(delivered),
            "the frames before the kill",
        );
        let frames = read_pcm(child.session(), &mut out, "the frames before the kill");
        assert_eq!(frames, 1, "one frame per round trip");
        assert_eq!(
            out,
            frame_pattern::frame_samples(delivered),
            "frame {delivered} was torn, missing or out of order"
        );
        delivered += 1;
    }

    // Kill the helper from the outside, mid-stream.
    child.terminate().unwrap();

    // The consumer must now fail loudly: a typed error, never a silent end
    // of stream and never a hang. The bounded read turns a hang into a
    // loud failure that names the step, never into a pass.
    loop {
        match child.session().read(&mut out, Some(STALL_WAIT)) {
            Ok(0) => panic!("the killed helper must not look like end of stream"),
            Ok(frames) => {
                // A frame that was still in flight is whole: a helper that
                // dies mid-frame must not leave a torn one behind.
                for chunk in out[..frames * 2].as_chunks::<2>().0 {
                    assert_eq!(
                        *chunk,
                        frame_pattern::frame_samples(delivered),
                        "frame {delivered} was torn, missing or out of order"
                    );
                    delivered += 1;
                }
            }
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

    // The write direction: the in-ring still has room, but nobody will ever
    // read the frame from it. The write must not report success for it.
    let write_error = match child
        .session()
        .write_timeout(&frame_pattern::frame_samples(delivered), Some(STALL_WAIT))
    {
        Ok(()) => panic!("a killed DSP helper must not accept a frame"),
        Err(MediaHostError::TimedOut) => panic!(
            "stall: the killed helper was not noticed within {STALL_WAIT:?} \
             (the write waited for a frame nobody will ever take)"
        ),
        Err(error) => error,
    };
    let MediaHostError::Disconnected(_) = &write_error else {
        panic!("the write must fail with a loud, typed error: {write_error}");
    };
    assert!(
        write_error.to_string().contains("control stream ended"),
        "the death must be seen on the control channel: {write_error}"
    );

    assert!(
        delivered >= 4,
        "the helper must have streamed before the kill, not died before it"
    );
    assert_eq!(child.session().frames_dropped(), 0);
}
