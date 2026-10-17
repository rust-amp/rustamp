//! Two-process DSP checks: ignored, headless and VM-only. Build/stage the
//! `dsp_fixture` example and set RUSTAMP_DSP_FIXTURE to its guest path.

#![cfg(windows)]

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use rustamp_mediahost::MediaHostError;
use rustamp_mediahost::control::Liveness;
use rustamp_mediahost::dsp::DspProcess;
use rustamp_proto::host::{DspStageDesc, PcmSpecWire};

const SPEC: PcmSpecWire = PcmSpecWire {
    sample_rate_hz: 44_100,
    channels: 2,
};

/// Stall bound for the functional PCM read: the fixture delivers within
/// milliseconds, so a timeout means the helper stopped answering without a
/// terminal signal — a stall that fails loudly, never a pass.
const STALL_WAIT: Duration = Duration::from_secs(30);

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

#[test]
#[ignore = "two processes; Hyper-V VM only; requires RUSTAMP_DSP_FIXTURE"]
fn named_dsp_two_process_gain_round_trip_then_sticky_eos() {
    let mut child =
        DspProcess::spawn(&fixture(), SPEC, vec![gain(2.0)], 3, Liveness::default()).unwrap();
    assert_ne!(child.pid(), std::process::id());
    assert_eq!(child.session().latency_frames(), 0);
    let total = 4099;
    for block in 0..total / 257 {
        let start = block * 257;
        let mut samples = Vec::with_capacity(257 * 2);
        for frame in start..start + 257 {
            samples.push(frame as f32 / 8.0);
            samples.push(-(frame as f32 / 8.0));
        }
        child.session().write(&samples).unwrap();
    }
    child.session().finish_writes().unwrap();
    let mut received = Vec::new();
    let mut out = [0.0; 22];
    loop {
        // Stall escape (see STALL_WAIT): a timeout is a loud failure that
        // names the step, never a pass.
        let frames = match child.session().read(&mut out, Some(STALL_WAIT)) {
            Err(MediaHostError::TimedOut) => panic!(
                "stall: the fixture delivered no PCM within {STALL_WAIT:?} \
                 (it stopped answering without a close or an error)"
            ),
            other => other.unwrap(),
        };
        if frames == 0 {
            break;
        }
        received.extend_from_slice(&out[..frames * 2]);
    }
    assert_eq!(received.len(), (total / 257) * 257 * 2);
    for (index, sample) in received.iter().enumerate() {
        let frame = index / 2;
        let sign = if index % 2 == 0 { 1.0 } else { -1.0 };
        assert_eq!(*sample, sign * frame as f32 / 4.0);
    }
    assert_eq!(child.session().frames_dropped(), 0);
    assert_eq!(
        match child.session().read(&mut out, Some(STALL_WAIT)) {
            Err(MediaHostError::TimedOut) =>
                panic!("stall: the sticky EOS read never drained within {STALL_WAIT:?}"),
            other => other.unwrap(),
        },
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
            Liveness::default(),
        )
        .unwrap();
        child.session().write(&[0.25, -0.25]).unwrap();
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
