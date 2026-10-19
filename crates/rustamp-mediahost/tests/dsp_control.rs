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
//!
//! The end-of-transfer evidence (RFC 0006 §4/§6): the DSP path has no
//! end-of-stream marker, so the frame counts decide whether a closed
//! back-ring is a finished transfer or a truncated one. The scripted-host
//! pair at the end of this file drives both sides of that rule with a
//! healthy control channel, so nothing but the counts can tell them apart.
//!
//! The kill-the-host acceptance (the DSP half of M14 §4): the last two
//! tests kill a helper that is blocked in *reading* the in-ring. The
//! decoder path's kill is in the *write* direction, so the two paths are
//! not evidence for each other (M14 L2). The difference that matters here
//! is the end of the transfer: the DSP path has no `DecoderEos` marker, so
//! a helper that dies after returning exactly as many frames as it was
//! given leaves a back-ring that would look like a complete transfer. Only
//! the death on the control channel keeps that from being an end of
//! stream, and the fake helper pins exactly that.
//!
//! The **write** is fenced the same way (RFC 0006 §6): a write returns only
//! after the helper has *taken* the frames from the in-ring, so `Ok(())` is
//! a delivery fact and not a queueing fact. Without the fence a write
//! reports success for a frame nobody will ever process: the in-ring still
//! has room between the helper's death and the observation of the control
//! EOF, and it has room for as long as a helper stops draining. The stream
//! then fails loudly only later, on some other call. The fence waits for
//! the take (the frame counters are the evidence) and turns loud when the
//! control channel reports the helper's death.

#![cfg(windows)]

#[path = "support/frame_pattern.rs"]
mod frame_pattern;
#[path = "support/dsp_resident.rs"]
mod resident;
#[path = "support/streams.rs"]
mod streams;

use std::io::{Read, Write};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::Duration;

use rustamp_ipc::shm::{NamedRegion, RingDirection, RingError, RingKind};
use rustamp_ipc::{Message, StreamTransport, Transport};
use rustamp_mediahost::control::{ControlPeer, Liveness};
use rustamp_mediahost::dsp::{DspResident, DspSession, ResidentChain, serve_dsp};
use rustamp_mediahost::pcm::{NamedPcmRings, PcmInput, PcmOffer, PcmRings};
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
/// The bound of the explicit timeout tests: a write that waits for a
/// delivery that never comes must fail on its own bound, not on the
/// stall escape. It is a deadline, not a success criterion: the assertions
/// look at the error, never at the clock.
const STALL_BOUND: Duration = Duration::from_millis(200);
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

    fn process(&mut self, buf: &mut [f32]) -> Result<()> {
        for sample in buf {
            *sample += self.history;
        }
        self.history = 1.0;
        Ok(())
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

/// RFC 0014 acceptance 3 (M14 finding F1's smallest reproduction): a chain
/// that fails after block *n* with a message. Before RFC 0014 no
/// `ResidentChain` could express this at all; now `process` returns `Err`,
/// the pump propagates it, `serve_dsp` sends the existing `DspError` message
/// with the text, and the player side fails loud and sticky.
///
/// The chain passes every block through bit-exactly until block
/// `FAIL_AT_BLOCK + 1`, which fails with "clipping detected".
const FAIL_AT_BLOCK: usize = 1024;

struct FailingChain {
    blocks: usize,
}

impl ResidentChain for FailingChain {
    fn latency_frames(&self) -> u32 {
        0
    }

    fn process(&mut self, _buf: &mut [f32]) -> Result<()> {
        self.blocks += 1;
        if self.blocks > FAIL_AT_BLOCK {
            return Err(MediaHostError::OperationFailed("clipping detected".into()));
        }
        // The healthy blocks pass through bit-exactly.
        Ok(())
    }

    fn reset(&mut self) {}
}

struct FailingResident;

impl DspResident for FailingResident {
    fn open(&self, _spec: PcmSpecWire, _stages: &[DspStageDesc]) -> Result<Box<dyn ResidentChain>> {
        Ok(Box::new(FailingChain { blocks: 0 }))
    }
}

#[test]
fn midrun_chain_failure_is_a_loud_dsp_error_carrying_the_text() {
    let (mut session, done) = start(FailingResident);
    let mut out = [0.0; 2];
    // Write FAIL_AT_BLOCK + 1 frames one at a time. The in-ring holds a
    // single frame, so the host reads each frame as its own block: the
    // first FAIL_AT_BLOCK blocks pass through bit-exactly and pile up
    // unread in the back-ring, and block FAIL_AT_BLOCK + 1 fails. Keeping
    // the healthy backlog in flight across the failure is what makes the
    // terminal read deterministic: the drain-then-close order hands the
    // healthy frames over first, and by the time the ring is empty the
    // `DspError` has long landed (the session's drained-close wake race of
    // M19 §3 would only be reachable by a read blocked on an empty ring
    // exactly at the failure).
    for frame in 0..=FAIL_AT_BLOCK {
        let v = frame as f32 / 8.0;
        write_pcm(&mut session, &[v, -v], "the frames around the failure");
    }
    // The host failed on the last block and exited with an error; its
    // outcome also proves the DspError was sent and flushed.
    let outcome = done
        .recv_timeout(STALL_WAIT)
        .expect("stall: the host must publish its failure instead of hanging");
    let host_error = outcome.expect_err("a failed chain must end the host with Err");
    assert!(
        host_error.to_string().contains("clipping detected"),
        "the host's error must carry the chain's text: {host_error}"
    );
    // Read the healthy backlog back: every frame that comes back is the
    // bit-exact passthrough, and the stream ends in the loud failure —
    // never a drained-close `Ok(0)`.
    let mut drained = 0usize;
    loop {
        match session.read(&mut out, Some(STALL_WAIT)) {
            Ok(0) => panic!("a mid-run chain failure must not turn into EOS"),
            Ok(frames) => {
                assert_eq!(frames, 1, "one frame per read");
                let want = drained as f32 / 8.0;
                assert_eq!(out, [want, -want], "frame {drained} must be bit-exact");
                drained += 1;
            }
            Err(error) => {
                assert!(
                    matches!(error, MediaHostError::Disconnected(_)),
                    "the failure must be a disconnect, got {error:?}"
                );
                let message = error.to_string();
                assert!(
                    message.contains("resident DSP failed"),
                    "the DspError text must reach the player: {message}"
                );
                assert!(
                    message.contains("clipping detected"),
                    "the chain's diagnostic must survive: {message}"
                );
                break;
            }
        }
    }
    assert!(
        drained <= FAIL_AT_BLOCK,
        "the drain cannot exceed the healthy blocks, got {drained}"
    );
    // The failure is sticky: a later read fails loud again, never `Ok(0)`.
    let again = session
        .read(&mut out, Some(STALL_WAIT))
        .expect_err("the failure must stick, never EOS");
    assert!(again.to_string().contains("clipping detected"), "{again}");
    drop(session);
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

#[test]
fn back_ring_close_before_input_finished_is_sticky_failure_not_eos() {
    let ((ar, aw), (br, bw)) = streams::streams();
    let player = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, false).unwrap();
    let (closed, observed) = mpsc::channel();
    let (release, held) = mpsc::channel();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        assert!(matches!(
            host.recv(STALL_WAIT).unwrap(),
            Message::DspConfigure { .. }
        ));
        let offer = host.recv(STALL_WAIT).unwrap();
        let input = NamedPcmRings
            .open_directed(&offer, SPEC, RingDirection::MainToHost)
            .unwrap();
        host.send(Message::RingReady {
            kind: RingKind::Pcm,
            generation: input.consumer.generation(),
        })
        .unwrap();
        let mut back = NamedPcmRings.create(SPEC, 1).unwrap();
        let back_generation = back.producer.generation();
        host.send(back.message).unwrap();
        assert!(matches!(
            host.recv(STALL_WAIT).unwrap(),
            Message::RingReady { generation, kind: RingKind::Pcm }
                if generation == back_generation
        ));
        host.send(Message::DspReady { latency_frames: 0 }).unwrap();
        back.producer.close();
        closed.send(()).unwrap();
        // Keep the control channel healthy; EOF, DspError and a missing Pong
        // must not accidentally satisfy the failure assertion below.
        held.recv_timeout(STALL_WAIT)
            .expect("stall: the test never released the healthy control peer");
        drop(host);
        done.send(()).unwrap();
    });
    let mut session = DspSession::open(player, NamedPcmRings, SPEC, &[], 1).unwrap();
    observed
        .recv_timeout(STALL_WAIT)
        .expect("stall: the scripted host never closed the back-ring");
    let error = session.read(&mut [0.0; 2], Some(STALL_WAIT)).unwrap_err();
    assert!(matches!(error, MediaHostError::Disconnected(_)), "{error}");
    assert!(
        error.to_string().contains("before input finished"),
        "{error}"
    );
    let again = session.write(&[0.25, -0.5]).unwrap_err();
    assert!(matches!(again, MediaHostError::Disconnected(_)), "{again}");
    assert!(
        again.to_string().contains("before input finished"),
        "{again}"
    );
    release.send(()).unwrap();
    drop(session);
    result
        .recv_timeout(STALL_WAIT)
        .expect("stall: the scripted host did not finish");
}

/// Frames the scripted host takes from the in-ring before the transfer ends.
const SCRIPT_FRAMES: usize = 4;
/// Capacity of the scripted back-ring. It has to hold every frame the host
/// publishes without a reader, because the player reads only after the
/// transfer ended.
const SCRIPT_BACK_RING: u32 = 8;

/// A scripted host that completes the real two-ring handshake, takes
/// [`SCRIPT_FRAMES`] frames from the in-ring and publishes only the first
/// `returned` of them to the back-ring, which it then closes.
///
/// The control channel stays healthy on purpose: no `DspError`, no EOF and
/// no missing Pong may accidentally satisfy a failure assertion, so the
/// **frame counts** are the only evidence the player has for what the host
/// did with the frames it was given (RFC 0006 §6). Returns the channel that
/// announces the closed back-ring and the one that releases the host at the
/// end of the test.
fn scripted_host(host: ControlPeer, returned: usize) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (closed, observed) = mpsc::channel();
    let (release, held) = mpsc::channel();
    thread::spawn(move || {
        // The handshake, in the production order: configure, the player's
        // in-ring offer, the in-ring Ready, the back-ring offer, the back-ring
        // Ready, DspReady.
        assert!(matches!(
            host.recv(STALL_WAIT).unwrap(),
            Message::DspConfigure { .. }
        ));
        let offer = host.recv(STALL_WAIT).unwrap();
        let mut input = NamedPcmRings
            .open_directed(&offer, SPEC, RingDirection::MainToHost)
            .unwrap();
        host.send(Message::RingReady {
            kind: RingKind::Pcm,
            generation: input.consumer.generation(),
        })
        .unwrap();
        let mut back = NamedPcmRings.create(SPEC, SCRIPT_BACK_RING).unwrap();
        let back_generation = back.producer.generation();
        host.send(back.message).unwrap();
        assert!(matches!(
            host.recv(STALL_WAIT).unwrap(),
            Message::RingReady { generation, kind: RingKind::Pcm }
                if generation == back_generation
        ));
        host.send(Message::DspReady { latency_frames: 0 }).unwrap();

        // The transfer: take every frame the player wrote, publish only
        // `returned` of them.
        let mut buf = [0.0f32; 2];
        let mut taken = 0usize;
        let mut published = 0usize;
        while taken < SCRIPT_FRAMES {
            let count = input
                .consumer
                .read_into_blocking(&mut buf, Some(STALL_WAIT))
                .expect("stall: the scripted host never read the player's frames");
            assert!(count > 0, "stall: the scripted host read no frame");
            taken += count;
            if published < returned {
                back.producer
                    .write_from_blocking(&buf[..count * 2], Some(STALL_WAIT))
                    .expect("stall: the scripted host could not publish its frame");
                published += count;
            }
        }
        // The player closes the in-ring when it finished its writes; the
        // back-ring closes with the transfer.
        assert!(matches!(
            input
                .consumer
                .read_into_blocking(&mut buf, Some(STALL_WAIT)),
            Err(RingError::Closed)
        ));
        back.producer.close();
        closed.send(()).unwrap();
        held.recv_timeout(STALL_WAIT)
            .expect("stall: the test never released the scripted host");
        drop(host);
    });
    (observed, release)
}

/// Write [`SCRIPT_FRAMES`] frames into the chain and end the write side.
fn write_script_frames(session: &mut DspSession<NamedPcmRings>) {
    for frame in 0..SCRIPT_FRAMES {
        let value = frame as f32 / 8.0;
        write_pcm(
            session,
            &[value, -value],
            "the frames before the transfer ends",
        );
    }
    session.finish_writes().unwrap();
}

/// Read the drained transfer: the frames that came back, then the terminal
/// outcome. `None` is the end of the stream, `Some` the failure that ended
/// it.
fn drain_transfer(session: &mut DspSession<NamedPcmRings>) -> (usize, Option<MediaHostError>) {
    let mut out = [0.0f32; 16];
    let mut received = 0usize;
    loop {
        match read_pcm(session, &mut out, "the drained transfer") {
            Ok(0) => return (received, None),
            Ok(frames) => received += frames,
            Err(error) => return (received, Some(error)),
        }
    }
}

/// A host that returns fewer frames than it was given must not look like a
/// finished transfer: RFC 0006 §6 says block *n* out is block *n* back, and
/// RFC 0006 §4 forbids a lost frame crossing the boundary unnoticed.
#[test]
fn a_truncated_transfer_after_finished_writes_is_a_loud_failure_not_eos() {
    let ((ar, aw), (br, bw)) = streams::streams();
    let player = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, false).unwrap();
    let (observed, release) = scripted_host(host, SCRIPT_FRAMES - 1);
    let mut session = DspSession::open(player, NamedPcmRings, SPEC, &[], 1).unwrap();
    write_script_frames(&mut session);
    observed
        .recv_timeout(STALL_WAIT)
        .expect("stall: the scripted host never closed the back-ring");

    let (received, failure) = drain_transfer(&mut session);
    assert_eq!(
        received,
        SCRIPT_FRAMES - 1,
        "the frames the host did return must arrive"
    );
    let error = failure.expect("a truncated transfer must not turn into EOS");
    assert!(matches!(error, MediaHostError::Disconnected(_)), "{error}");
    let message = error.to_string();
    assert!(message.contains("truncated"), "{message}");
    assert!(
        message.contains(&format!("{SCRIPT_FRAMES} frames written")),
        "{message}"
    );
    assert!(
        message.contains(&format!("{} returned", SCRIPT_FRAMES - 1)),
        "{message}"
    );
    // The failure is sticky: a later write fails loud too, never an EOS.
    let again = session.write(&[0.25, -0.5]).unwrap_err();
    assert!(matches!(again, MediaHostError::Disconnected(_)), "{again}");
    release.send(()).unwrap();
    drop(session);
}

/// The same scripted host that returns every frame it was given: the
/// transfer is complete, so the drained back-ring is the end of the stream
/// (RFC 0006 §6).
#[test]
fn a_complete_transfer_after_finished_writes_is_the_end_of_stream() {
    let ((ar, aw), (br, bw)) = streams::streams();
    let player = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, false).unwrap();
    let (observed, release) = scripted_host(host, SCRIPT_FRAMES);
    let mut session = DspSession::open(player, NamedPcmRings, SPEC, &[], 1).unwrap();
    write_script_frames(&mut session);
    observed
        .recv_timeout(STALL_WAIT)
        .expect("stall: the scripted host never closed the back-ring");

    let (received, failure) = drain_transfer(&mut session);
    assert_eq!(received, SCRIPT_FRAMES, "every frame must come back");
    assert!(
        failure.is_none(),
        "a complete transfer must not fail: {:?}",
        failure
    );
    // The end of the stream sticks.
    let mut out = [0.0f32; 2];
    assert_eq!(
        read_pcm(&mut session, &mut out, "the sticky EOS read").unwrap(),
        0
    );
    release.send(()).unwrap();
    drop(session);
}

// ---------------------------------------------------------------------------
// The kill-the-host acceptance: the helper dies while it waits for the
// next block of the in-ring.
// ---------------------------------------------------------------------------

/// Blocks the fake helper takes from the in-ring before it stops draining
/// it. The consumer wrote and read back exactly this many frames, so the
/// two frame counts match at the moment of the death — the case in which a
/// closed back-ring would look like a complete transfer.
const KILL_BLOCKS: usize = 3;
/// Capacity of the fake helper's back-ring. It has to hold every block the
/// helper publishes, because the consumer reads them back.
const KILL_BACK_RING: u32 = 8;

/// The fake helper's transport: the raw wire, so a killed helper leaves the
/// rings open (a `ControlPeer` would close them when it is dropped).
type FakeTransport = StreamTransport<streams::ByteReader, streams::ByteWriter>;

/// The fake helper's two-ring handshake: the in-ring it reads and the
/// back-ring it publishes to, in the production order (the in-ring offer and
/// ready first, the back-ring offer second — see `serve_dsp` and
/// `DspSession::handshake`).
fn fake_dsp_handshake(
    transport: &mut FakeTransport,
    back_capacity: u32,
) -> (PcmInput<NamedRegion>, PcmOffer<NamedRegion>) {
    transport.send(&Message::hello()).unwrap();
    let mut spec = None;
    loop {
        match transport.recv().unwrap() {
            Message::Hello { .. } => {}
            Message::Ping(token) => transport.send(&Message::Pong(token)).unwrap(),
            Message::DspConfigure {
                spec: configured, ..
            } => spec = Some(configured),
            offer @ Message::RingOffer { .. } => {
                let spec = spec.expect("the configure must come before the ring offer");
                // The consumer's in-ring: the helper reads from it.
                let opened = NamedPcmRings
                    .open_directed(&offer, spec, RingDirection::MainToHost)
                    .unwrap();
                transport
                    .send(&Message::RingReady {
                        kind: RingKind::Pcm,
                        generation: opened.consumer.generation(),
                    })
                    .unwrap();
                // The helper's back-ring.
                let created = NamedPcmRings
                    .create_directed(spec, back_capacity, RingDirection::HostToMain)
                    .unwrap();
                let back_generation = created.producer.generation();
                transport.send(&created.message).unwrap();
                // The consumer's Ready for the back-ring.
                let ready = loop {
                    match transport.recv().unwrap() {
                        Message::Ping(token) => transport.send(&Message::Pong(token)).unwrap(),
                        Message::RingReady {
                            kind: RingKind::Pcm,
                            generation,
                        } => break generation,
                        message => {
                            panic!("unexpected message while the back-ring is opened: {message:?}")
                        }
                    }
                };
                assert_eq!(
                    ready, back_generation,
                    "the consumer must ready the ring the helper offered"
                );
                transport
                    .send(&Message::DspReady { latency_frames: 0 })
                    .unwrap();
                return (opened, created);
            }
            message => panic!("unexpected message: {message:?}"),
        }
    }
}

/// A fake DSP helper that reads from the in-ring, publishes every block it
/// read to the back-ring and then dies without a terminal message.
///
/// It speaks the raw wire on purpose: a `ControlPeer` would close the
/// rings when it is dropped, and a killed helper leaves them open (RFC 0006
/// §8: crash detection is the control channel's job). The helper reads
/// exactly `blocks` blocks and then stops draining the in-ring — the state
/// a kill lands in when the helper waits for the next block.
fn dying_dsp_helper(
    reader: streams::ByteReader,
    writer: streams::ByteWriter,
    blocks: usize,
    back_capacity: u32,
    killed: mpsc::Receiver<()>,
) {
    let mut transport = StreamTransport::new(reader, writer);
    let (mut input, mut back) = fake_dsp_handshake(&mut transport, back_capacity);
    // The transfer: the helper takes every block from the in-ring and
    // publishes it unchanged to the back-ring.
    let mut buf = [0.0f32; 2];
    for _ in 0..blocks {
        let count = input
            .consumer
            .read_into_blocking(&mut buf, Some(STALL_WAIT))
            .expect("stall: the fake helper never read a block");
        back.producer
            .write_from_blocking(&buf[..count * 2], Some(STALL_WAIT))
            .expect("stall: the fake helper could not publish its block");
    }
    // The helper stops draining the in-ring and waits for the kill: from
    // here on nobody reads the next block.
    killed
        .recv_timeout(STALL_WAIT)
        .expect("stall: the test never killed the fake helper");
    // The helper dies here: the control channel ends and both rings stay
    // open. A killed process leaves exactly this behind.
    drop(transport);
    drop(input);
    drop(back);
}

/// The consumer of a dead helper: the player side of a session whose
/// helper has just been killed, plus the channel that kills it.
fn dying_dsp_session() -> (DspSession<NamedPcmRings>, mpsc::Sender<()>) {
    let ((ar, aw), (br, bw)) = streams::streams();
    let peer = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let (kill, killed) = mpsc::channel();
    thread::spawn(move || dying_dsp_helper(br, bw, KILL_BLOCKS, KILL_BACK_RING, killed));
    let session = DspSession::open(peer, NamedPcmRings, SPEC, &[], 1).unwrap();
    (session, kill)
}

/// A fake DSP helper that completes the handshake and then never takes a
/// frame from the in-ring: every frame the player writes is lost.
///
/// It watches the in-ring instead of reading it, so it can announce that
/// the player's frame is in the ring (the write reached the ring) without
/// taking it — the take is the delivery the write has to wait for. The kill
/// then ends the control channel and leaves both rings open, exactly like a
/// killed process.
fn stalled_dsp_helper(
    reader: streams::ByteReader,
    writer: streams::ByteWriter,
    back_capacity: u32,
    seen: mpsc::Sender<()>,
    killed: mpsc::Receiver<()>,
) {
    let mut transport = StreamTransport::new(reader, writer);
    let (input, _back) = fake_dsp_handshake(&mut transport, back_capacity);
    // The helper watches the in-ring and never takes the frame: the wait
    // ends when the player's frame is published, not when it is read.
    match input.consumer.wait_readable(Some(STALL_WAIT)) {
        Ok(_) => {}
        Err(error) => panic!("stall: the fake helper never saw the player's frame: {error}"),
    }
    seen.send(()).unwrap();
    killed
        .recv_timeout(STALL_WAIT)
        .expect("stall: the test never killed the fake helper");
    // The helper dies here: the control channel ends and both rings stay
    // open. A killed process leaves exactly this behind.
    drop(transport);
    drop(input);
}

/// The player side of a session whose helper ignores the in-ring, plus the
/// channel that announces the player's frame and the one that kills the
/// helper.
fn stalled_dsp_session() -> (
    DspSession<NamedPcmRings>,
    mpsc::Receiver<()>,
    mpsc::Sender<()>,
) {
    let ((ar, aw), (br, bw)) = streams::streams();
    let peer = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let (watching, seen) = mpsc::channel();
    let (kill, killed) = mpsc::channel();
    thread::spawn(move || stalled_dsp_helper(br, bw, KILL_BACK_RING, watching, killed));
    let session = DspSession::open(peer, NamedPcmRings, SPEC, &[], 1).unwrap();
    (session, seen, kill)
}

/// The DSP write gap: a write must not report success for a frame that
/// nobody will ever process.
///
/// The helper watches the in-ring, sees the player's frame and dies without
/// taking it: the frame is lost, and the control EOF is the only news about
/// it. A write that returned `Ok(())` would report a delivered frame for a
/// transfer that never happened — the stream would only fail later, on
/// some other call. The write must fail loudly instead: the helper's death
/// ends the wait, and the failure names the control stream.
#[test]
fn a_write_never_reports_success_for_a_frame_the_dead_helper_will_not_take() {
    let (mut session, seen, kill) = stalled_dsp_session();
    let frame = frame_pattern::frame_samples(0);
    // The write blocks until the helper takes the frame — which never
    // happens — so it runs on its own thread and the death wakes it. The
    // session comes back with the result, so the rings stay open while the
    // helper watches the in-ring.
    let handle = thread::spawn(move || {
        let result = session.write_timeout(&frame, Some(STALL_WAIT));
        (session, result)
    });
    // The helper sees the frame in the in-ring and never takes it.
    seen.recv_timeout(STALL_WAIT)
        .expect("stall: the fake helper never saw the player's frame");
    // The helper dies: the control channel ends and the rings stay open.
    kill.send(()).unwrap();

    let (_session, result) = handle.join().expect("the write thread must not panic");
    let error = match result {
        Ok(()) => panic!("a write must not report success for a frame nobody will take"),
        Err(MediaHostError::TimedOut) => panic!(
            "stall: the dead helper was not noticed within {STALL_WAIT:?} \
             (the write waited for a frame nobody will ever take)"
        ),
        Err(error) => error,
    };
    let MediaHostError::Disconnected(reason) = &error else {
        panic!("the write must fail with a loud, typed error: {error}");
    };
    assert!(
        reason.contains("control stream ended"),
        "the death must be seen on the control channel: {error}"
    );
}

/// The same gap without the death: the helper keeps the control channel
/// healthy but never takes the frame. The write must still not report
/// success — the frame is lost — and the bounded write has to say so with
/// a loud stall instead of accepting a frame nobody will process.
#[test]
fn a_write_never_reports_success_while_the_helper_stops_draining() {
    let (mut session, seen, kill) = stalled_dsp_session();
    let frame = frame_pattern::frame_samples(0);
    let error = match session.write_timeout(&frame, Some(STALL_BOUND)) {
        Ok(()) => panic!("a write must not report success for a frame nobody will take"),
        Err(error) => error,
    };
    assert!(
        matches!(error, MediaHostError::TimedOut),
        "a helper that never takes the frame is a loud stall, not a silent success: {error:?}"
    );
    // The helper is alive and healthy: it saw the frame and never took it.
    seen.recv_timeout(STALL_WAIT)
        .expect("stall: the fake helper never saw the player's frame");
    kill.send(()).unwrap();
}

/// The regression for the write fence: a helper that takes the frame makes
/// the write succeed, and the frame comes back through the back-ring. The
/// fence waits for the take, it never eats the frame.
#[test]
fn a_live_helper_takes_the_frame_the_write_reports() {
    let (mut session, done) = start(resident::StubDspResident);
    let mut out = [0.0; 2];
    let frame = frame_pattern::frame_samples(7);
    write_pcm(&mut session, &frame, "the frame a live helper takes");
    assert_eq!(
        read_pcm(&mut session, &mut out, "the frame a live helper takes").unwrap(),
        1,
        "the frame the write reported must come back"
    );
    assert_eq!(out, frame, "the frame must survive the round trip");
    finish(session, done);
}

/// A helper that dies mid-stream must leave the consumer with whole frames
/// only, and then with a loud, typed error.
///
/// The fake helper reads the frames the consumer writes, publishes them
/// unchanged and then vanishes without a terminal message and without
/// closing its rings — a crash surrogate, the same shape as a helper that
/// is killed from the outside. The consumer must receive exactly those
/// frames, each whole and in order (a torn, missing or reordered frame
/// breaks the shared frame pattern), and then report `Disconnected`: never
/// a silent end of stream, never a hang. The failure must also stick: a
/// later read may not turn into end of stream.
///
/// The DSP path has no end-of-stream marker (RFC 0006 §6): the end of the
/// transfer is inferred from the frame counts. The helper returned exactly
/// as many frames as it was given, so a closed back-ring would look like a
/// complete transfer — the end of the stream is only avoided because the
/// death is seen on the control channel first. That is the trap this test
/// pins, and the decoder path cannot have it: `DecoderEos` is a marker.
#[test]
fn a_dead_dsp_helper_delivers_whole_frames_and_then_fails_loudly() {
    let (mut session, kill) = dying_dsp_session();

    // The frames that made the round trip: whole, in order and nothing else.
    let mut out = [0.0; 2];
    for index in 0..KILL_BLOCKS {
        write_pcm(
            &mut session,
            &frame_pattern::frame_samples(index),
            "the frames before the kill",
        );
        let frames = read_pcm(&mut session, &mut out, "the frames before the kill").unwrap();
        assert_eq!(frames, 1, "one frame per round trip");
        assert_eq!(
            out,
            frame_pattern::frame_samples(index),
            "frame {index} was torn, missing or out of order"
        );
    }

    // The helper dies: the control channel ends and the rings stay open.
    kill.send(()).unwrap();

    // The consumer must now fail loudly: a typed error, never a silent end
    // of stream and never a hang.
    let error = match session.read(&mut out, Some(STALL_WAIT)) {
        Ok(0) => panic!("a dead DSP helper must not look like end of stream"),
        Ok(frames) => panic!("a dead DSP helper must not deliver more frames ({frames})"),
        Err(MediaHostError::TimedOut) => panic!(
            "stall: the dead helper was not noticed within {STALL_WAIT:?} \
             (the consumer waited for a frame that never came)"
        ),
        Err(error) => error,
    };
    let MediaHostError::Disconnected(reason) = &error else {
        panic!("a dead DSP helper must report a loud, typed error: {error}");
    };
    assert!(
        reason.contains("control stream ended"),
        "the death must be seen on the control channel: {error}"
    );

    // The failure must stick: a later read may not turn into end of stream.
    let again = match session.read(&mut out, Some(STALL_WAIT)) {
        Ok(frames) => panic!(
            "the failure must stick: the dead helper turned into end of stream ({frames} frames)"
        ),
        Err(error) => error,
    };
    let MediaHostError::Disconnected(_) = &again else {
        panic!("the failure must stick as the same loud error: {again}");
    };

    // The write direction is the one the decoder path does not have: the
    // consumer pushes frames into the in-ring. A dead helper never reads
    // them, so the write must fail loudly too — never hang and never look
    // like a delivered frame.
    let frame = frame_pattern::frame_samples(KILL_BLOCKS);
    let write_error = match session.write_timeout(&frame, Some(STALL_WAIT)) {
        Ok(()) => panic!("a dead DSP helper must not accept a frame"),
        Err(MediaHostError::TimedOut) => {
            panic!("stall: the write waited for space that never came")
        }
        Err(error) => error,
    };
    let MediaHostError::Disconnected(_) = &write_error else {
        panic!("the write must fail with the same loud error: {write_error}");
    };
}

/// The write direction of the kill: a frame that has to wait for space in
/// the in-ring must be woken by the helper's death, not hang.
///
/// The helper stops draining the in-ring, so the ring fills up. The next
/// write has to wait for space that never comes — the wait the decoder
/// path does not have, because there the consumer never writes PCM. The
/// death must end that wait with a loud, typed error.
///
/// The write is larger than the in-ring (one frame), so it fills the ring
/// with its first frame and waits for the space that never comes; a write
/// that fitted would return as soon as the helper took it (the write
/// fence), which is a different wait. The write blocks, so it runs on its
/// own thread and the death wakes it.
#[test]
fn a_dead_dsp_helper_wakes_a_write_that_waits_for_space() {
    let (mut session, kill) = dying_dsp_session();

    // The frames that made the round trip.
    let mut out = [0.0; 2];
    for index in 0..KILL_BLOCKS {
        write_pcm(
            &mut session,
            &frame_pattern::frame_samples(index),
            "the frames before the kill",
        );
        read_pcm(&mut session, &mut out, "the frames before the kill").unwrap();
    }
    // The next write has to wait for space: the helper has stopped draining
    // the in-ring, so the first frame fills it and the second one waits.
    let mut frames = [0.0f32; 4];
    frames[..2].copy_from_slice(&frame_pattern::frame_samples(KILL_BLOCKS));
    frames[2..].copy_from_slice(&frame_pattern::frame_samples(KILL_BLOCKS + 1));
    let handle = thread::spawn(move || session.write_timeout(&frames, Some(STALL_WAIT)));
    // The helper dies with the in-ring full.
    kill.send(()).unwrap();

    // The death must wake the wait: a silent hang and a delivered frame are
    // both wrong.
    let error = match handle.join().expect("the write thread must not panic") {
        Ok(()) => panic!("a dead DSP helper must not accept a frame"),
        Err(MediaHostError::TimedOut) => panic!(
            "stall: the dead helper was not noticed within {STALL_WAIT:?} \
             (the write waited for space in a ring nobody reads)"
        ),
        Err(error) => error,
    };
    let MediaHostError::Disconnected(reason) = &error else {
        panic!("the blocked write must fail with a loud, typed error: {error}");
    };
    assert!(
        reason.contains("control stream ended"),
        "the death must be seen on the control channel: {error}"
    );
}
