//! Headless one-process control regressions; execute in the Hyper-V VM only.
//! All functional waits are conditions with a generous stall escape
//! (`STALL_WAIT`): a healthy host answers within milliseconds, so only a
//! wedged or dead host reaches the bound, and reaching it fails loudly with
//! the step named — never a pass. Writes go through `write_pcm`
//! (`write_timeout` with the same bound), so the producer side is closed
//! too. Only the explicit timeout test requests a short PCM deadline; no
//! wall-clock assertion decides whether work completed.

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

/// Stall bound for every wait that waits on the host worker (see the file
/// docs): generous enough that only a real stall reaches it, and a timeout is
/// a loud failure that names the step, never a pass.
const STALL_WAIT: Duration = Duration::from_secs(30);

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
    done.recv_timeout(STALL_WAIT)
        .expect("stall: the host must publish its shutdown outcome instead of hanging")
        .unwrap();
}

/// Read PCM with a stall escape (see the file docs).
fn read_pcm(session: &mut DspSession<NamedPcmRings>, out: &mut [f32], step: &str) -> Result<usize> {
    match session.read(out, Some(STALL_WAIT)) {
        Err(MediaHostError::TimedOut) => panic!(
            "stall: {step} received no PCM within {STALL_WAIT:?} \
             (the resident stopped delivering without a close or an error)"
        ),
        other => other,
    }
}

/// Write PCM with a stall escape (see the file docs): `write_timeout` bounds
/// the whole call, so a resident that stops draining the in-ring fails
/// loudly with the step named instead of hanging the test.
fn write_pcm(session: &mut DspSession<NamedPcmRings>, buf: &[f32], step: &str) {
    match session.write_timeout(buf, Some(STALL_WAIT)) {
        Err(MediaHostError::TimedOut) => panic!(
            "stall: {step} could not write within {STALL_WAIT:?} \
             (the resident stopped draining the in-ring without a close or an error)"
        ),
        other => other.unwrap(),
    }
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
            release
                .recv_timeout(STALL_WAIT)
                .expect("stall: the test never released the first reset");
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
    assert_eq!(
        observed
            .recv_timeout(STALL_WAIT)
            .expect("stall: the host did not service the first reset instead of hanging"),
        1
    );
    // A second reset is queued while the first callback is held. The old
    // one-command-then-infinite-read loop could never service it without PCM.
    session.reset().unwrap();
    release.send(()).unwrap();
    assert_eq!(
        observed
            .recv_timeout(STALL_WAIT)
            .expect("stall: the host did not service the second reset instead of hanging"),
        2
    );
    assert_eq!(session.generations(), generations);

    let mut out = [0.0; 2];
    write_pcm(
        &mut session,
        &[0.25, -0.25],
        "the block after the idle resets",
    );
    assert_eq!(
        read_pcm(&mut session, &mut out, "the block after the idle resets").unwrap(),
        1
    );
    assert_eq!(out, [0.25, -0.25]);
    session.reset().unwrap();
    assert_eq!(
        observed
            .recv_timeout(STALL_WAIT)
            .expect("stall: the host did not service the mid-stream reset instead of hanging"),
        3
    );
    write_pcm(
        &mut session,
        &[0.5, -0.5],
        "the block after the mid-stream reset",
    );
    assert_eq!(
        read_pcm(
            &mut session,
            &mut out,
            "the block after the mid-stream reset"
        )
        .unwrap(),
        1
    );
    assert_eq!(
        out,
        [0.5, -0.5],
        "old history must not process the first new block"
    );

    session.finish_writes().unwrap();
    assert_eq!(
        read_pcm(&mut session, &mut out, "the drained EOS read").unwrap(),
        0
    );
    session.reset().unwrap();
    assert_eq!(
        observed
            .recv_timeout(STALL_WAIT)
            .expect("stall: the host did not service the post-EOS reset instead of hanging"),
        4,
        "reset also lands after drained EOS"
    );
    assert_eq!(session.generations(), generations);
    assert_eq!(
        read_pcm(&mut session, &mut out, "the sticky EOS read").unwrap(),
        0
    );
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
    write_pcm(
        &mut session,
        &[0.25, -0.5],
        "the block after the bounded read timeout",
    );
    assert_eq!(
        read_pcm(
            &mut session,
            &mut out,
            "the retry after the bounded timeout"
        )
        .unwrap(),
        1
    );
    assert_eq!(out, [0.25, -0.5]);
    session.finish_writes().unwrap();
    assert_eq!(
        read_pcm(&mut session, &mut out, "the drained EOS read").unwrap(),
        0
    );
    assert_eq!(
        read_pcm(&mut session, &mut out, "the sticky EOS read").unwrap(),
        0
    );
    finish(session, done);
}
