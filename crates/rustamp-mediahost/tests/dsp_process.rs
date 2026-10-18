//! Two-process DSP checks: ignored, headless and VM-only. Build/stage the
//! `dsp_fixture` example and set RUSTAMP_DSP_FIXTURE to its guest path.
//!
//! The round trip runs in the same lockstep as the in-process suite (see
//! docs/tasks/M20-roundtrip-rebuild.md): write one bounded block, read that
//! block back, repeat — the producer and the consumer progress together, so
//! the fixed 4096-frame back-ring never has to absorb the sweep without a
//! reader, and a wedged helper fails loudly instead of hanging the test.

#![cfg(windows)]

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
