//! Two-process checks: ignored, headless and VM-only. Build/stage the
//! decoder_fixture example and set RUSTAMP_DECODER_FIXTURE to its guest path.

#![cfg(windows)]

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use rustamp_mediahost::MediaHostError;
use rustamp_mediahost::control::Liveness;
use rustamp_mediahost::decoder::DecoderProcess;
use rustamp_proto::host::MediaLocationWire;

fn fixture() -> PathBuf {
    PathBuf::from(
        std::env::var_os("RUSTAMP_DECODER_FIXTURE")
            .expect("VM must stage decoder_fixture and set its absolute path"),
    )
}

#[test]
#[ignore = "two processes; Hyper-V VM only; requires RUSTAMP_DECODER_FIXTURE"]
fn named_pcm_two_process_transfer_and_seek() {
    let mut child = DecoderProcess::spawn(
        &fixture(),
        MediaLocationWire::Url("fixture:plain".into()),
        3,
        Liveness::default(),
    )
    .unwrap();
    assert_ne!(child.pid(), std::process::id());
    assert_eq!(
        child.session().seek(29, Duration::from_secs(5)).unwrap(),
        29
    );
    let mut frame = 29;
    let mut out = [0.0; 22];
    loop {
        let count = child.session().read(&mut out, None).unwrap();
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
            Liveness::default(),
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
