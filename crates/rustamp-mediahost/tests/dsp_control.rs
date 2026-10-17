//! Headless one-process control regressions; execute in the Hyper-V VM only.
//! All functional waits are conditions. Only the explicit timeout test requests
//! a PCM deadline; no wall-clock assertion decides whether work completed.

#![cfg(windows)]

#[path = "support/dsp_resident.rs"]
mod resident;
#[path = "support/streams.rs"]
mod streams;

use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::Duration;

use rustamp_mediahost::control::{ControlPeer, Liveness};
use rustamp_mediahost::dsp::{DspResident, DspSession, ResidentChain, serve_dsp};
use rustamp_mediahost::pcm::NamedPcmRings;
use rustamp_mediahost::{MediaHostError, Result};
use rustamp_proto::host::{DspStageDesc, PcmSpecWire};

const SPEC: PcmSpecWire = PcmSpecWire {
    sample_rate_hz: 44_100,
    channels: 2,
};

fn start(
    resident: impl DspResident + Send + 'static,
) -> (DspSession<NamedPcmRings>, mpsc::Receiver<Result<()>>) {
    let ((ar, aw), (br, bw)) = streams::streams();
    let player = ControlPeer::start(ar, aw, Liveness::default(), false).unwrap();
    let host = ControlPeer::start(br, bw, Liveness::default(), true).unwrap();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(serve_dsp(&host, &NamedPcmRings, &resident));
    });
    let session = DspSession::open(player, NamedPcmRings, SPEC, &[], 1).unwrap();
    (session, result)
}

fn finish(session: DspSession<NamedPcmRings>, done: mpsc::Receiver<Result<()>>) {
    drop(session);
    done.recv()
        .expect("host must publish its shutdown outcome")
        .unwrap();
}

struct StatefulResident {
    resets: mpsc::Sender<u32>,
    release_first: Mutex<Option<mpsc::Receiver<()>>>,
}

struct StatefulChain {
    resets: mpsc::Sender<u32>,
    release_first: Option<mpsc::Receiver<()>>,
    count: u32,
    history: f32,
}

impl DspResident for StatefulResident {
    fn open(&self, _spec: PcmSpecWire, _stages: &[DspStageDesc]) -> Result<Box<dyn ResidentChain>> {
        Ok(Box::new(StatefulChain {
            resets: self.resets.clone(),
            release_first: self.release_first.lock().unwrap().take(),
            count: 0,
            history: 42.0,
        }))
    }
}

impl ResidentChain for StatefulChain {
    fn latency_frames(&self) -> u32 {
        0
    }

    fn process(&mut self, buf: &mut [f32]) {
        for sample in buf {
            *sample += self.history;
        }
        self.history = 1.0;
    }

    fn reset(&mut self) {
        self.history = 0.0;
        self.count += 1;
        self.resets.send(self.count).unwrap();
        if let Some(release) = self.release_first.take() {
            release.recv().expect("test releases the first reset");
        }
    }
}

#[test]
fn idle_reset_lands_without_pcm_and_clears_history_before_the_next_block() {
    let (resets, observed) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let (mut session, done) = start(StatefulResident {
        resets,
        release_first: Mutex::new(Some(held)),
    });
    let generations = session.generations();
    session.reset().unwrap();
    assert_eq!(observed.recv().unwrap(), 1);
    // A second reset is queued while the first callback is held. The old
    // one-command-then-infinite-read loop could never service it without PCM.
    session.reset().unwrap();
    release.send(()).unwrap();
    assert_eq!(observed.recv().unwrap(), 2);
    assert_eq!(session.generations(), generations);

    let mut out = [0.0; 2];
    session.write(&[0.25, -0.25]).unwrap();
    assert_eq!(session.read(&mut out, None).unwrap(), 1);
    assert_eq!(out, [0.25, -0.25]);
    session.reset().unwrap();
    assert_eq!(observed.recv().unwrap(), 3);
    session.write(&[0.5, -0.5]).unwrap();
    assert_eq!(session.read(&mut out, None).unwrap(), 1);
    assert_eq!(
        out,
        [0.5, -0.5],
        "old history must not process the first new block"
    );

    session.finish_writes().unwrap();
    assert_eq!(session.read(&mut out, None).unwrap(), 0);
    session.reset().unwrap();
    assert_eq!(
        observed.recv().unwrap(),
        4,
        "reset also lands after drained EOS"
    );
    assert_eq!(session.generations(), generations);
    assert_eq!(session.read(&mut out, None).unwrap(), 0);
    finish(session, done);
}

#[test]
fn bounded_dsp_read_timeout_is_not_eos_and_retry_delivers_pcm() {
    let (mut session, done) = start(resident::StubDspResident);
    let mut out = [0.0; 2];
    // No input is published until after the timeout: scheduling cannot make
    // this race against a sleeping fixture, and the host remains responsive.
    assert!(matches!(
        session.read(&mut out, Some(Duration::from_millis(10))),
        Err(MediaHostError::TimedOut)
    ));
    session.write(&[0.25, -0.5]).unwrap();
    assert_eq!(session.read(&mut out, None).unwrap(), 1);
    assert_eq!(out, [0.25, -0.5]);
    session.finish_writes().unwrap();
    assert_eq!(session.read(&mut out, None).unwrap(), 0);
    assert_eq!(session.read(&mut out, None).unwrap(), 0);
    finish(session, done);
}
