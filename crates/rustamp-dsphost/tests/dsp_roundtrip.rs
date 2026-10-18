//! DSP resident round-trip tests: the run loop against byte-stream control
//! pairs and real named rings in-process. Headless; one Windows process, no
//! window, no audio. Run in the Hyper-V test VM, never on the host.
//!
//! Round-trip shape (the rebuild of the M18 candidate-1 deadlock): the
//! producer and the consumer run in lockstep — write one bounded chunk, read
//! that chunk back, repeat — so neither direction can end up waiting on a
//! ring the other side has stopped draining (see `round_trip`).
//!
//! Waiting rule (see the M10 flakiness note): PCM reads and writes wait on
//! the condition (data, space, a drained close or a terminal failure wakes
//! them), never on a wall-clock bound that a healthy-but-slow schedule
//! could trip. Every such wait still carries a generous stall escape
//! (`STALL_WAIT`): only a wedged or dead host reaches it, and reaching it
//! fails loudly with the step named — a stall is never reported as success.
//! Only joining the host thread or a woken outcome uses the same generous
//! bound (`JOIN_WAIT`), which a healthy host reaches in milliseconds; the
//! message on expiry names the stall. `DspSession::open` and `reconfigure`
//! inherit the framework's 5 s `CONTROL_WAIT` for a single handshake
//! exchange.

#![cfg(windows)]

use std::io::{Cursor, Read, Write};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rustamp_dsphost::StageResident;
use rustamp_ipc::shm::RingDirection;
use rustamp_ipc::{Message, StreamTransport, Transport};
use rustamp_mediahost::control::{ControlPeer, Liveness};
use rustamp_mediahost::dsp::{DspSession, serve_dsp};
use rustamp_mediahost::pcm::{NamedPcmRings, PcmRings};
use rustamp_mediahost::{MediaHostError, Result};
use rustamp_proto::host::{DspStageDesc, PcmSpecWire};

const JOIN_WAIT: Duration = Duration::from_secs(30);
/// Stall bound for every PCM read and write (see the file docs): generous
/// enough that only a real stall reaches it, and a timeout is a loud
/// failure that names the step, never a pass.
const STALL_WAIT: Duration = Duration::from_secs(30);
/// Liveness policy for the test's control peers. The framework default
/// (`Liveness::default()`: 100 ms interval, 5 s timeout) is a wall-clock
/// bound a healthy-but-slow VM schedule can trip — the flake class M24
/// proved in this VM and fixed in rustamp-dechost's `real_decode` suite
/// (M24 §1.3), which also named this file's suites as still carrying it.
/// A dead resident never answers a Ping and is caught by the control EOF
/// regardless, and every PCM wait keeps its `STALL_WAIT` bound, so no test
/// here depends on a fast liveness expiry; only the trip wire moves.
const LIVENESS: Liveness = Liveness {
    interval: Duration::from_millis(100),
    timeout: STALL_WAIT,
};
const SPEC: PcmSpecWire = PcmSpecWire {
    sample_rate_hz: 44_100,
    channels: 2,
};
const TOTAL: usize = 4099;
/// Frames per lockstep step. While the producer runs ahead of the consumer
/// the back-ring holds at most one chunk, so this bounds the frames the
/// fixed 4096-frame back-ring must ever absorb without a reader (see
/// `round_trip`).
const CHUNK: usize = 257;

fn gain(factor: f32) -> DspStageDesc {
    DspStageDesc {
        id: "gain".into(),
        params: factor.to_le_bytes().to_vec(),
    }
}

fn passthrough() -> DspStageDesc {
    DspStageDesc {
        id: "passthrough".into(),
        params: Vec::new(),
    }
}

fn sweep(frame: usize) -> [f32; 2] {
    [frame as f32 / 8.0, -(frame as f32 / 8.0)]
}

struct ByteReader {
    rx: mpsc::Receiver<Vec<u8>>,
    bytes: Cursor<Vec<u8>>,
}

struct ByteWriter(mpsc::Sender<Vec<u8>>);

impl Read for ByteReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            let count = self.bytes.read(out)?;
            if count > 0 {
                return Ok(count);
            }
            match self.rx.recv() {
                Ok(bytes) => self.bytes = Cursor::new(bytes),
                Err(_) => return Ok(0),
            }
        }
    }
}

impl Write for ByteWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .send(bytes.to_vec())
            .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn streams() -> ((ByteReader, ByteWriter), (ByteReader, ByteWriter)) {
    let (a_tx, b_rx) = mpsc::channel();
    let (b_tx, a_rx) = mpsc::channel();
    (
        (
            ByteReader {
                rx: a_rx,
                bytes: Cursor::new(Vec::new()),
            },
            ByteWriter(a_tx),
        ),
        (
            ByteReader {
                rx: b_rx,
                bytes: Cursor::new(Vec::new()),
            },
            ByteWriter(b_tx),
        ),
    )
}

/// Serve a chain on a host thread; return the player session plus the host
/// outcome. Ring capacities are deliberately small so every run wraps them.
fn start(
    stages: Vec<DspStageDesc>,
    capacity: u32,
) -> (DspSession<NamedPcmRings>, mpsc::Receiver<Result<()>>) {
    let ((ar, aw), (br, bw)) = streams();
    let client = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let outcome = serve_dsp(&host, &NamedPcmRings, &StageResident);
        let _ = done.send(outcome);
    });
    let session = DspSession::open(client, NamedPcmRings, SPEC, &stages, capacity).unwrap();
    (session, result)
}

/// Write PCM with a stall escape (see the file docs): `write_timeout` bounds
/// the whole call, so a resident that stops draining the in-ring fails
/// loudly with the step named instead of hanging the test — the
/// producer-side half of the deadlock this file once carried.
fn write_pcm(session: &mut DspSession<NamedPcmRings>, buf: &[f32], step: &str) {
    match session.write_timeout(buf, Some(STALL_WAIT)) {
        Err(MediaHostError::TimedOut) => panic!(
            "stall: {step} could not write within {STALL_WAIT:?} \
             (the resident stopped draining the in-ring without a close or an error)"
        ),
        other => other.unwrap(),
    }
}

/// Read PCM with a stall escape (see the file docs): a healthy host delivers
/// within milliseconds, so a timeout means it stopped answering without
/// data, a close or an error — a stall that fails loudly, never a pass.
fn read_pcm(session: &mut DspSession<NamedPcmRings>, out: &mut [f32], step: &str) -> usize {
    match session.read(out, Some(STALL_WAIT)) {
        Err(MediaHostError::TimedOut) => panic!(
            "stall: {step} received no PCM within {STALL_WAIT:?} \
             (the resident stopped delivering without a close or an error)"
        ),
        other => other.unwrap(),
    }
}

/// One interleaved producer/consumer round trip over the frames
/// `[from, to)`: write one bounded chunk, read that chunk back, repeat.
/// Returns the samples that came back, in order.
///
/// This is the rebuild of the write-then-read split that hung the VM (see
/// docs/tasks/M18-hang-hunt.md, candidate 1): the old `write_sweep` pushed
/// every frame before `read_all` read a single one, so the player could
/// block in `DspSession::write` on a full in-ring while the host waited on
/// a full 4096-frame back-ring, nobody reading — the bounded-buffer
/// deadlock, with `TOTAL = 4099 = 4096 + 3` leaving a 1–3-frame margin.
///
/// The invariant that now guarantees progress: **the producer never starts
/// chunk n+1 before the consumer has drained chunk n.** The host processes
/// strictly in order, so the frames a chunk's read-back waits for are
/// exactly that chunk, and the frames in flight (written but not yet read
/// back) never exceed one `CHUNK` (257). The back-ring therefore holds at
/// most 257 frames — 3839 below its 4096 capacity — so the host never
/// blocks on its back-ring write, never stops draining the in-ring, the
/// producer's write always finds room and the consumer's read always finds
/// data: neither wait can be permanent, and the deadlock boundary (4096 +
/// in-ring capacity + the host's in-hand block) is unreachable by
/// construction. The margin is no longer 1–3 frames but 3839, and the sweep
/// still flows 4099 frames through the 4096-frame back-ring, so the
/// longer-than-capacity, wrapping and lossless assertions keep their
/// meaning. Every wait is bounded by `STALL_WAIT` on top, so even a wedged
/// resident fails loudly with the step named instead of hanging the run.
fn round_trip(session: &mut DspSession<NamedPcmRings>, from: usize, to: usize) -> Vec<f32> {
    let mut received = Vec::with_capacity((to - from) * 2);
    let mut out = [0.0; CHUNK * 2];
    let mut next = from;
    while next < to {
        let end = (next + CHUNK).min(to);
        let count = end - next;
        let mut block = Vec::with_capacity(count * 2);
        for frame in next..end {
            block.extend_from_slice(&sweep(frame));
        }
        write_pcm(session, &block, "the lockstep write");
        next = end;
        // Read this chunk back; the host processes strictly in order.
        let mut got = 0;
        while got < count {
            let frames = read_pcm(session, &mut out, "the lockstep read");
            assert!(frames > 0, "host must not end the stream early");
            received.extend_from_slice(&out[..frames * 2]);
            got += frames;
        }
    }
    received
}

/// Sticky-EOS read with the same stall escape (see the file docs).
fn read_sticky(session: &mut DspSession<NamedPcmRings>, out: &mut [f32], step: &str) -> usize {
    match session.read(out, Some(STALL_WAIT)) {
        Err(MediaHostError::TimedOut) => panic!(
            "stall: {step} received no drained close within {STALL_WAIT:?} \
             (the resident stopped answering without closing the ring)"
        ),
        other => other.unwrap(),
    }
}

#[test]
fn passthrough_round_trip_is_bit_exact_and_eos_is_sticky() {
    let (mut session, done) = start(vec![passthrough()], 3);
    // Handshake pins the contract: the echoed spec and zero chain latency.
    assert_eq!(session.spec(), SPEC);
    assert_eq!(session.latency_frames(), 0);
    let received = round_trip(&mut session, 0, TOTAL);
    session.finish_writes().unwrap();
    let expected: Vec<f32> = (0..TOTAL).flat_map(sweep).collect();
    assert_eq!(
        received.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        expected.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
    );
    assert_eq!(session.frames_dropped(), 0);
    // Sticky end of stream only after the drain.
    let mut out = [0.0; 2];
    assert_eq!(
        read_sticky(&mut session, &mut out, "the first sticky EOS read"),
        0
    );
    assert_eq!(
        read_sticky(&mut session, &mut out, "the second sticky EOS read"),
        0
    );
    drop(session);
    done.recv_timeout(JOIN_WAIT)
        .expect("graceful Shutdown must end the host")
        .unwrap();
}

#[test]
fn gain_chain_changes_audio_measurably() {
    let (mut session, done) = start(vec![passthrough(), gain(2.0)], 7);
    assert_eq!(session.latency_frames(), 0);
    let received = round_trip(&mut session, 0, TOTAL);
    session.finish_writes().unwrap();
    let input: Vec<f32> = (0..TOTAL).flat_map(sweep).collect();
    assert_ne!(received, input, "a x2 gain must be measurable");
    for (index, sample) in received.iter().enumerate() {
        assert_eq!(*sample, input[index] * 2.0, "frame {}", index / 2);
    }
    assert_eq!(session.frames_dropped(), 0);
    let mut out = [0.0; 2];
    assert_eq!(
        read_sticky(&mut session, &mut out, "the sticky EOS read"),
        0
    );
    drop(session);
    done.recv_timeout(JOIN_WAIT)
        .expect("graceful Shutdown must end the host")
        .unwrap();
}

#[test]
fn unknown_stage_is_a_loud_error_before_any_ring() {
    let ((ar, aw), (br, bw)) = streams();
    let client = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let outcome = serve_dsp(&host, &NamedPcmRings, &StageResident);
        let _ = done.send(outcome);
    });
    let error = DspSession::open(
        client,
        NamedPcmRings,
        SPEC,
        &[DspStageDesc {
            id: "fuzz".into(),
            params: Vec::new(),
        }],
        3,
    )
    .err()
    .expect("unknown stage must fail the configure");
    assert!(error.to_string().contains("unknown DSP stage"), "{error}");
    // The refusal carries no EOS: the host thread ends loud, never drained.
    assert!(
        result
            .recv_timeout(JOIN_WAIT)
            .expect("refused host must terminate")
            .is_err()
    );
}

#[test]
fn gain_with_bad_params_is_refused() {
    let ((ar, aw), (br, bw)) = streams();
    let client = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let outcome = serve_dsp(&host, &NamedPcmRings, &StageResident);
        let _ = done.send(outcome);
    });
    let error = DspSession::open(
        client,
        NamedPcmRings,
        SPEC,
        &[DspStageDesc {
            id: "gain".into(),
            params: vec![0u8; 3],
        }],
        3,
    )
    .err()
    .expect("bad params must fail the configure");
    assert!(error.to_string().contains("gain params"), "{error}");
    assert!(
        result
            .recv_timeout(JOIN_WAIT)
            .expect("refused host must terminate")
            .is_err()
    );
}

#[test]
fn reset_keeps_rings_and_continues() {
    let (mut session, done) = start(vec![gain(2.0)], 3);
    let before = session.generations();
    let mut received = round_trip(&mut session, 0, 1000);
    session.reset().unwrap();
    assert_eq!(session.generations(), before, "reset must not touch rings");
    assert_eq!(session.latency_frames(), 0);
    // Continue the sweep where it stopped; the stream must stay exact.
    received.extend(round_trip(&mut session, 1000, TOTAL));
    session.finish_writes().unwrap();
    for (index, sample) in received.iter().enumerate() {
        let frame = index / 2;
        let sign = if index % 2 == 0 { 1.0 } else { -1.0 };
        assert_eq!(*sample, sign * frame as f32 / 4.0, "frame {frame}");
    }
    drop(session);
    done.recv_timeout(JOIN_WAIT)
        .expect("graceful Shutdown must end the host")
        .unwrap();
}

#[test]
fn reconfigure_reoffers_both_rings_with_new_sound() {
    let (mut session, done) = start(vec![gain(2.0)], 3);
    let before = session.generations();
    let first = round_trip(&mut session, 0, 500);
    for (index, sample) in first.iter().enumerate() {
        let frame = index / 2;
        let sign = if index % 2 == 0 { 1.0 } else { -1.0 };
        assert_eq!(*sample, sign * frame as f32 / 4.0, "frame {frame}");
    }
    session.reconfigure(SPEC, &[gain(0.5)], 3).unwrap();
    assert_ne!(
        session.generations(),
        before,
        "reconfigure must bump both rings"
    );
    let rest = round_trip(&mut session, 500, TOTAL);
    session.finish_writes().unwrap();
    for (index, sample) in rest.iter().enumerate() {
        let frame = index / 2 + 500;
        let sign = if index % 2 == 0 { 1.0 } else { -1.0 };
        assert_eq!(*sample, sign * frame as f32 / 16.0, "frame {frame}");
    }
    let mut out = [0.0; 2];
    assert_eq!(
        read_sticky(&mut session, &mut out, "the sticky EOS read"),
        0
    );
    drop(session);
    done.recv_timeout(JOIN_WAIT)
        .expect("graceful Shutdown must end the host")
        .unwrap();
}

#[test]
fn dropping_the_player_stops_the_host_cleanly() {
    let (session, done) = start(vec![gain(2.0)], 3);
    drop(session);
    done.recv_timeout(JOIN_WAIT)
        .expect("dropped player must end the host")
        .unwrap();
}

#[test]
fn abrupt_player_loss_stops_the_host_without_hanging() {
    // A player that vanishes without Shutdown: raw transport, a manual
    // handshake, one block, then the pipes are dropped (control EOF).
    let ((ar, aw), (br, bw)) = streams();
    let mut player = StreamTransport::new(ar, aw);
    player.send(&Message::hello()).unwrap();
    player
        .send(&Message::DspConfigure {
            spec: SPEC,
            stages: vec![gain(1.0)],
        })
        .unwrap();
    let offer = NamedPcmRings
        .create_directed(SPEC, 3, RingDirection::MainToHost)
        .unwrap();
    let in_generation = offer.producer.generation();
    player.send(&offer.message).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let outcome = serve_dsp(&host, &NamedPcmRings, &StageResident);
        let _ = done.send(outcome);
    });
    // The host's ControlPeer queues its own Hello first; consume it (and
    // answer pings) before the handshake replies, exactly like the decoder
    // mirror in rustamp-mediahost's pcm_transfer suite.
    let mut saw_in_ready = false;
    let back = loop {
        match player.recv().unwrap() {
            Message::Hello { .. } => {}
            Message::Ping(token) => player.send(&Message::Pong(token)).unwrap(),
            Message::RingReady {
                kind: rustamp_ipc::shm::RingKind::Pcm,
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
    let mut back_input = NamedPcmRings
        .open_directed(&back, SPEC, RingDirection::HostToMain)
        .unwrap();
    player
        .send(&Message::RingReady {
            kind: rustamp_ipc::shm::RingKind::Pcm,
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
    let mut producer = offer.producer;
    producer
        .write_from_blocking(&sweep(0), Some(STALL_WAIT))
        .unwrap();
    let mut one = [0.0; 2];
    back_input
        .consumer
        .read_into_blocking(&mut one, Some(STALL_WAIT))
        .unwrap();
    // Vanish: drop the transport without Shutdown, then the rings.
    drop(player);
    drop(producer);
    drop(back_input);
    assert!(
        result
            .recv_timeout(JOIN_WAIT)
            .expect("vanished player must end the host")
            .is_err(),
        "a vanished player is a loud disconnect, never a clean exit"
    );
}

#[test]
fn abrupt_host_loss_is_loud_and_never_eos() {
    let ((ar, aw), (br, bw)) = streams();
    let peer = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let (die, death) = mpsc::channel();
    thread::spawn(move || {
        // Deliberately no ControlPeer or close guard at the fake host end.
        let mut transport = StreamTransport::new(br, bw);
        transport.send(&Message::hello()).unwrap();
        let mut back = None;
        loop {
            match transport.recv().unwrap() {
                Message::Hello { .. } => {}
                Message::Ping(token) => transport.send(&Message::Pong(token)).unwrap(),
                Message::DspConfigure { spec, .. } => {
                    // Buffer the back-ring offer: the production order is
                    // in-ring Ready first, then the back-ring offer (see
                    // `serve_dsp` and `DspSession::handshake`). Sending the
                    // back offer here would put it ahead of the in-ring
                    // Ready and fail the handshake.
                    let offer = NamedPcmRings
                        .create_directed(spec, 1, RingDirection::HostToMain)
                        .unwrap();
                    back = Some(offer);
                }
                Message::RingOffer { generation, .. } => {
                    transport
                        .send(&Message::RingReady {
                            kind: rustamp_ipc::shm::RingKind::Pcm,
                            generation,
                        })
                        .unwrap();
                    // In-ring is live: now offer the back-ring.
                    let offer = back.as_ref().expect("back-ring before configure");
                    transport.send(&offer.message).unwrap();
                }
                Message::RingReady { .. } => {
                    let mut offer = back.take().unwrap();
                    offer
                        .producer
                        .write_from_blocking(&[0.5, -0.5], Some(STALL_WAIT))
                        .unwrap();
                    transport
                        .send(&Message::DspReady { latency_frames: 0 })
                        .unwrap();
                    death.recv_timeout(Duration::from_secs(30)).unwrap();
                    // RingProducer::Drop does NOT close the ring.
                    break;
                }
                message => panic!("unexpected {message:?}"),
            }
        }
    });
    let mut session = DspSession::open(peer, NamedPcmRings, SPEC, &[gain(1.0)], 1).unwrap();
    write_pcm(&mut session, &[0.25, -0.25], "the fake host's first block");
    let mut out = [0.0; 2];
    // Stall escape (see the file docs) on the first read: the fake host
    // publishes its block before DspReady, so a timeout is a stall, not a
    // slow schedule.
    let first = read_pcm(&mut session, &mut out, "the fake host's first block");
    assert_eq!(first, 1);
    assert_eq!(out, [0.5, -0.5]);
    die.send(()).unwrap();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(session.read(&mut out, None));
    });
    let outcome = result
        .recv_timeout(JOIN_WAIT)
        .expect("control EOF must wake an infinite DSP wait");
    assert!(matches!(outcome, Err(MediaHostError::Disconnected(_))));
}
