//! Headless one-process control regressions; execute in the Hyper-V VM only.
//! All functional waits are conditions with a generous stall escape
//! (`STALL_WAIT`): a healthy host answers within milliseconds, so only a
//! wedged or dead host reaches the bound, and reaching it fails loudly with
//! the step named — never a pass. Writes go through `write_pcm`
//! (`write_timeout` with the same bound), so the producer side is closed
//! too. Only the explicit timeout tests request a short deadline (the PCM
//! read timeout, and the framework's `CONTROL_WAIT` reset-acknowledgement
//! bound); no wall-clock assertion decides whether work completed.
//!
//! The reset fence (RFC 0015): `DspSession::reset` waits for the host's
//! `DspResetAck`, so a reset that returns has landed. The fence tests assert
//! exactly that — the chain's own report is already observable when `reset`
//! returns, and the wait is still blocked while the host's callback is held
//! — plus the bounded, loud timeout when no acknowledgement ever comes.

#![cfg(windows)]

#[path = "support/dsp_resident.rs"]
mod resident;
#[path = "support/streams.rs"]
mod streams;

use std::io::{Read, Write};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::Duration;

use rustamp_ipc::shm::{RingDirection, RingKind};
use rustamp_ipc::{Message, StreamTransport, Transport};
use rustamp_mediahost::control::{ControlPeer, Liveness};
use rustamp_mediahost::dsp::{DspResident, DspSession, ResidentChain, serve_dsp};
use rustamp_mediahost::pcm::{NamedPcmRings, PcmRings};
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
/// Liveness policy for the in-process control peers. The framework default
/// (`Liveness::default()`: 100 ms interval, 5 s timeout) is a wall-clock
/// bound a healthy-but-slow VM schedule can trip — the flake class M24
/// proved in this guest and fixed in the dechost/audio/dsphost suites
/// (M24 §1.3, M25). A dead resident never answers a Ping and is caught by
/// the control EOF regardless, so only the trip wire moves.
const LIVENESS: Liveness = Liveness {
    interval: Duration::from_millis(100),
    timeout: STALL_WAIT,
};

fn start(
    resident: impl DspResident + Send + 'static,
) -> (DspSession<NamedPcmRings>, mpsc::Receiver<Result<()>>) {
    let ((ar, aw), (br, bw)) = streams::streams();
    let player = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
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

/// Read one `DspResetAck` from a raw player transport, answering liveness
/// pings on the way; anything else is a protocol violation.
fn recv_ack<R: Read + Send, W: Write + Send>(player: &mut StreamTransport<R, W>) {
    loop {
        match player.recv().unwrap() {
            Message::Ping(token) => player.send(&Message::Pong(token)).unwrap(),
            Message::DspResetAck => return,
            message => panic!("expected DspResetAck, got {message:?}"),
        }
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
fn reset_returns_only_after_the_host_applied_it() {
    let (resets, observed) = mpsc::channel();
    let (mut session, done) = start(StatefulResident {
        resets,
        release_first: Mutex::new(None),
    });
    // No PCM, no hold: the fenced wait returns once the chain state is
    // cleared — the chain's own report is already there when it does,
    // with no re-send and no further wait.
    session.reset().unwrap();
    assert_eq!(
        observed.try_recv(),
        Ok(1),
        "the fence: the reset must be applied when reset returns"
    );
    let generations = session.generations();
    session.reset().unwrap();
    assert_eq!(
        observed.try_recv(),
        Ok(2),
        "the fence: the reset must be applied when reset returns"
    );
    assert_eq!(
        session.generations(),
        generations,
        "the fence never touches the rings"
    );
    // The acknowledgement is consumed by the wait: the stream is undisturbed.
    let mut out = [0.0; 2];
    write_pcm(
        &mut session,
        &[0.25, -0.25],
        "the block after the fenced resets",
    );
    assert_eq!(
        read_pcm(&mut session, &mut out, "the block after the fenced resets").unwrap(),
        1
    );
    assert_eq!(out, [0.25, -0.25]);
    finish(session, done);
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

    // The first reset is serviced with no PCM at all: the pump polls commands
    // between its bounded read slices. Its callback is held, so the fenced
    // wait blocks — a reset is not acknowledged before it is applied.
    let handle = thread::spawn(move || {
        session.reset().unwrap();
        session
    });
    assert_eq!(
        observed
            .recv_timeout(STALL_WAIT)
            .expect("stall: the host did not enter the first reset instead of hanging"),
        1
    );
    assert!(
        !handle.is_finished(),
        "the fence: reset must not return while the host's callback is held"
    );
    release.send(()).unwrap();
    let mut session = handle
        .join()
        .expect("the fenced wait must return once the host applies the reset");

    // The second reset is serviced in order; when it returns, the chain has
    // already applied it — the count is there with no further wait.
    session.reset().unwrap();
    assert_eq!(
        observed.try_recv(),
        Ok(2),
        "the fence: the reset must be applied when reset returns"
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

    // A mid-stream reset clears the history the running chain accumulated.
    session.reset().unwrap();
    assert_eq!(
        observed.try_recv(),
        Ok(3),
        "the fence: the mid-stream reset must be applied when reset returns"
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
    // The post-EOS idle loop answers the fence too.
    session.reset().unwrap();
    assert_eq!(
        observed.try_recv(),
        Ok(4),
        "the fence: reset also lands after drained EOS"
    );
    assert_eq!(session.generations(), generations);
    assert_eq!(
        read_pcm(&mut session, &mut out, "the sticky EOS read").unwrap(),
        0
    );
    finish(session, done);
}

/// The host-side half of the fence: a player may queue several resets
/// behind a callback that is still running; the pump drains them in order
/// and answers each with exactly one `DspResetAck` (RFC 0015). Driven over
/// a raw transport because the session's fenced wait serializes resets by
/// design — a burst exists only below the session.
#[test]
fn queued_resets_are_each_acknowledged_in_order_while_the_host_is_busy() {
    let (resets, observed) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let resident = StatefulResident {
        resets,
        release_first: Mutex::new(Some(held)),
    };
    let ((ar, aw), (br, bw)) = streams::streams();
    let mut player = StreamTransport::new(ar, aw);
    player.send(&Message::hello()).unwrap();
    player
        .send(&Message::DspConfigure {
            spec: SPEC,
            stages: Vec::new(),
        })
        .unwrap();
    let offer = NamedPcmRings
        .create_directed(SPEC, 1, RingDirection::MainToHost)
        .unwrap();
    let in_generation = offer.producer.generation();
    player.send(&offer.message).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(serve_dsp(&host, &NamedPcmRings, &resident));
    });
    // The host's ControlPeer queues its own Hello first; consume it (and
    // answer pings) before the handshake replies, exactly like the raw
    // decoder mirrors in the pcm_transfer suite.
    let mut saw_in_ready = false;
    let back = loop {
        match player.recv().unwrap() {
            Message::Hello { .. } => {}
            Message::Ping(token) => player.send(&Message::Pong(token)).unwrap(),
            Message::RingReady {
                kind: RingKind::Pcm,
                generation,
            } => {
                assert_eq!(generation, in_generation, "in-ring ready generation");
                saw_in_ready = true;
            }
            offer @ Message::RingOffer { .. } => {
                assert!(saw_in_ready, "back-ring offer before the in-ring was ready");
                break offer;
            }
            message => panic!("unexpected handshake message: {message:?}"),
        }
    };
    let back_input = NamedPcmRings
        .open_directed(&back, SPEC, RingDirection::HostToMain)
        .unwrap();
    player
        .send(&Message::RingReady {
            kind: RingKind::Pcm,
            generation: back_input.consumer.generation(),
        })
        .unwrap();
    loop {
        match player.recv().unwrap() {
            Message::Ping(token) => player.send(&Message::Pong(token)).unwrap(),
            Message::DspReady { .. } => break,
            message => panic!("unexpected post-handshake message: {message:?}"),
        }
    }

    // The first reset's callback is held; a second reset queued behind it
    // must still be serviced, in order, each with exactly one ack.
    player.send(&Message::DspReset).unwrap();
    assert_eq!(
        observed
            .recv_timeout(STALL_WAIT)
            .expect("stall: the host did not enter the first reset instead of hanging"),
        1
    );
    player.send(&Message::DspReset).unwrap();
    release.send(()).unwrap();
    recv_ack(&mut player);
    recv_ack(&mut player);
    assert_eq!(
        observed
            .recv_timeout(STALL_WAIT)
            .expect("stall: the host did not service the queued reset instead of hanging"),
        2
    );

    player.send(&Message::Shutdown).unwrap();
    drop(back_input);
    drop(offer.producer);
    drop(player);
    result
        .recv_timeout(STALL_WAIT)
        .expect("stall: the host must publish its shutdown outcome instead of hanging")
        .unwrap();
}

#[test]
fn reset_without_an_acknowledgement_is_a_loud_bounded_timeout() {
    // A resident whose reset never returns: the pump is held inside the
    // callback, so no DspResetAck can come. The session's bounded wait must
    // fail loudly (TimedOut) — never a silent pass — and the failure must be
    // sticky: a late acknowledgement can never surface as an "unexpected
    // message" on a later write or read.
    let (resets, observed) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let (mut session, done) = start(StatefulResident {
        resets,
        release_first: Mutex::new(Some(held)),
    });
    assert!(
        matches!(session.reset(), Err(MediaHostError::TimedOut)),
        "a missing acknowledgement is a loud TimedOut, never a silent pass"
    );
    // The host did receive and enter the reset — the wait failed because
    // the acknowledgement never came, not because the message was lost.
    assert_eq!(
        observed.try_recv(),
        Ok(1),
        "the host must have entered the reset whose acknowledgement never came"
    );
    // The failure is sticky and names the fence: later calls fail with the
    // disconnect, never a race against the late acknowledgement.
    let error = session.write(&[0.0, 0.0]).unwrap_err();
    assert!(
        matches!(error, MediaHostError::Disconnected(_)),
        "a timed-out reset fails the conversation sticky, got {error:?}"
    );
    assert!(
        error.to_string().contains("reset acknowledgement"),
        "the sticky failure must name the reset acknowledgement, got {error}"
    );
    assert!(matches!(
        session.read(&mut [0.0, 0.0], Some(STALL_WAIT)),
        Err(MediaHostError::Disconnected(_))
    ));
    // Release the wedged callback: the host then sends the late ack into the
    // failed conversation and ends loud on the player's hang-up.
    release.send(()).unwrap();
    drop(session);
    let outcome = done
        .recv_timeout(STALL_WAIT)
        .expect("stall: the host must publish its outcome instead of hanging");
    assert!(
        outcome.is_err(),
        "the player hung up on a failed conversation: the host ends loud, never clean, got {outcome:?}"
    );
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
