//! DSP resident round-trip tests: the run loop against byte-stream control
//! pairs and real named rings in-process. Headless; one Windows process, no
//! window, no audio. Run in the Hyper-V test VM, never on the host.
//!
//! Waiting rule (see the M10 flakiness note): PCM reads wait on the condition
//! (`None`: data, a drained close or a terminal failure wakes them), never on
//! a wall-clock bound. Only joining the host thread or a woken outcome uses a
//! generous 30 s bound, which a healthy host reaches in milliseconds; the
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
const SPEC: PcmSpecWire = PcmSpecWire {
    sample_rate_hz: 44_100,
    channels: 2,
};
const TOTAL: usize = 4099;

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
    let client = ControlPeer::start(ar, aw, Liveness::default(), false).unwrap();
    let host = ControlPeer::start(br, bw, Liveness::default(), true).unwrap();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let outcome = serve_dsp(&host, &NamedPcmRings, &StageResident);
        let _ = done.send(outcome);
    });
    let session = DspSession::open(client, NamedPcmRings, SPEC, &stages, capacity).unwrap();
    (session, result)
}

fn write_sweep(session: &mut DspSession<NamedPcmRings>, frames: usize) {
    let mut block = Vec::with_capacity(257 * 2);
    let mut written = 0;
    while written < frames {
        block.clear();
        for frame in written..(written + 257).min(frames) {
            block.extend_from_slice(&sweep(frame));
        }
        session.write(&block).unwrap();
        written += block.len() / 2;
    }
}

fn read_all(session: &mut DspSession<NamedPcmRings>, frames: usize) -> Vec<f32> {
    let mut received = Vec::with_capacity(frames * 2);
    let mut out = [0.0; 514];
    while received.len() < frames * 2 {
        let count = session.read(&mut out, None).unwrap();
        assert!(count > 0, "host must not end the stream early");
        received.extend_from_slice(&out[..count * 2]);
    }
    received
}

#[test]
fn passthrough_round_trip_is_bit_exact_and_eos_is_sticky() {
    let (mut session, done) = start(vec![passthrough()], 3);
    // Handshake pins the contract: the echoed spec and zero chain latency.
    assert_eq!(session.spec(), SPEC);
    assert_eq!(session.latency_frames(), 0);
    write_sweep(&mut session, TOTAL);
    session.finish_writes().unwrap();
    let received = read_all(&mut session, TOTAL);
    let expected: Vec<f32> = (0..TOTAL).flat_map(sweep).collect();
    assert_eq!(
        received.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        expected.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
    );
    assert_eq!(session.frames_dropped(), 0);
    // Sticky end of stream only after the drain.
    let mut out = [0.0; 2];
    assert_eq!(session.read(&mut out, None).unwrap(), 0);
    assert_eq!(session.read(&mut out, None).unwrap(), 0);
    drop(session);
    done.recv_timeout(JOIN_WAIT)
        .expect("graceful Shutdown must end the host")
        .unwrap();
}

#[test]
fn gain_chain_changes_audio_measurably() {
    let (mut session, done) = start(vec![passthrough(), gain(2.0)], 7);
    assert_eq!(session.latency_frames(), 0);
    write_sweep(&mut session, TOTAL);
    session.finish_writes().unwrap();
    let received = read_all(&mut session, TOTAL);
    let input: Vec<f32> = (0..TOTAL).flat_map(sweep).collect();
    assert_ne!(received, input, "a x2 gain must be measurable");
    for (index, sample) in received.iter().enumerate() {
        assert_eq!(*sample, input[index] * 2.0, "frame {}", index / 2);
    }
    assert_eq!(session.frames_dropped(), 0);
    let mut out = [0.0; 2];
    assert_eq!(session.read(&mut out, None).unwrap(), 0);
    drop(session);
    done.recv_timeout(JOIN_WAIT)
        .expect("graceful Shutdown must end the host")
        .unwrap();
}

#[test]
fn unknown_stage_is_a_loud_error_before_any_ring() {
    let ((ar, aw), (br, bw)) = streams();
    let client = ControlPeer::start(ar, aw, Liveness::default(), false).unwrap();
    let host = ControlPeer::start(br, bw, Liveness::default(), true).unwrap();
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
    let client = ControlPeer::start(ar, aw, Liveness::default(), false).unwrap();
    let host = ControlPeer::start(br, bw, Liveness::default(), true).unwrap();
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
    write_sweep(&mut session, 1000);
    session.reset().unwrap();
    assert_eq!(session.generations(), before, "reset must not touch rings");
    assert_eq!(session.latency_frames(), 0);
    // Continue the sweep where it stopped; the stream must stay exact.
    let mut block = Vec::with_capacity(2);
    for frame in 1000..TOTAL {
        block.clear();
        block.extend_from_slice(&sweep(frame));
        session.write(&block).unwrap();
    }
    session.finish_writes().unwrap();
    let received = read_all(&mut session, TOTAL);
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
    write_sweep(&mut session, 500);
    let first = read_all(&mut session, 500);
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
    let mut block = Vec::with_capacity(2);
    for frame in 500..TOTAL {
        block.clear();
        block.extend_from_slice(&sweep(frame));
        session.write(&block).unwrap();
    }
    session.finish_writes().unwrap();
    let rest = read_all(&mut session, TOTAL - 500);
    for (index, sample) in rest.iter().enumerate() {
        let frame = index / 2 + 500;
        let sign = if index % 2 == 0 { 1.0 } else { -1.0 };
        assert_eq!(*sample, sign * frame as f32 / 16.0, "frame {frame}");
    }
    let mut out = [0.0; 2];
    assert_eq!(session.read(&mut out, None).unwrap(), 0);
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
    let host = ControlPeer::start(br, bw, Liveness::default(), true).unwrap();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let outcome = serve_dsp(&host, &NamedPcmRings, &StageResident);
        let _ = done.send(outcome);
    });
    assert_eq!(
        player.recv().unwrap(),
        Message::RingReady {
            kind: rustamp_ipc::shm::RingKind::Pcm,
            generation: in_generation,
        }
    );
    let back = match player.recv().unwrap() {
        offer @ Message::RingOffer { .. } => offer,
        message => panic!("expected back-ring offer, got {message:?}"),
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
    assert!(matches!(player.recv().unwrap(), Message::DspReady { .. }));
    let mut producer = offer.producer;
    producer
        .write_from_blocking(&sweep(0), Some(Duration::from_secs(5)))
        .unwrap();
    let mut one = [0.0; 2];
    back_input
        .consumer
        .read_into_blocking(&mut one, Some(Duration::from_secs(5)))
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
    let peer = ControlPeer::start(ar, aw, Liveness::default(), false).unwrap();
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
                    let offer = NamedPcmRings
                        .create_directed(spec, 1, RingDirection::HostToMain)
                        .unwrap();
                    transport.send(&offer.message).unwrap();
                    back = Some(offer);
                }
                Message::RingOffer { generation, .. } => {
                    transport
                        .send(&Message::RingReady {
                            kind: rustamp_ipc::shm::RingKind::Pcm,
                            generation,
                        })
                        .unwrap();
                }
                Message::RingReady { .. } => {
                    let mut offer = back.take().unwrap();
                    offer
                        .producer
                        .write_from_blocking(&[0.5, -0.5], Some(Duration::from_secs(5)))
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
    session.write(&[0.25, -0.25]).unwrap();
    let mut out = [0.0; 2];
    assert_eq!(session.read(&mut out, None).unwrap(), 1);
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
