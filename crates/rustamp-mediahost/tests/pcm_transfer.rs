//! Headless in-process protocol/worker tests. Run in the test VM, not the host.
//! Channel byte streams exercise production framing and the real ring/wake API.

#[path = "support/resident.rs"]
mod resident;

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rustamp_ipc::shm::{
    AnonymousRegion, RING_DATA_OFFSET, RingConsumer, RingDirection, RingKind, RingProducer,
    RingWake,
};
use rustamp_ipc::{Message, StreamTransport, Transport};
use rustamp_mediahost::control::{ControlPeer, Liveness};
use rustamp_mediahost::decoder::{
    DecoderInfo, DecoderResident, DecoderSession, ResidentDecoder, serve_decoder,
};
use rustamp_mediahost::pcm::{CloseRing, PcmInput, PcmOffer, PcmRings};
use rustamp_mediahost::{MediaHostError, Result};
use rustamp_proto::host::{MediaLocationWire, PcmSpecWire};

/// Stall bound for every wait that waits on the host worker: the M10
/// flakiness note's sanctioned form — a bound so generous that only a real
/// stall reaches it, with the message saying which. A timeout is always a
/// loud failure that names the step, never a pass.
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

#[derive(Clone, Default)]
struct TestRings(Arc<Mutex<HashMap<String, AnonymousRegion>>>);

impl PcmRings for TestRings {
    type Region = AnonymousRegion;

    fn create(&self, spec: PcmSpecWire, capacity: u32) -> Result<PcmOffer<Self::Region>> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let size = RING_DATA_OFFSET + spec.frame_size() as usize * capacity as usize;
        let region = AnonymousRegion::new(size)?;
        let producer =
            RingProducer::create(region.clone(), RingKind::Pcm, spec.frame_size(), capacity)?;
        let name = format!(
            "mediahost-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let producer = producer.with_wake(RingWake::create(&name)?);
        let abort = RingConsumer::attach(region.clone(), producer.expect())?
            .with_wake(RingWake::open(&name)?);
        self.0.lock().unwrap().insert(name.clone(), region);
        let message = Message::RingOffer {
            name,
            kind: RingKind::Pcm,
            frame_size: spec.frame_size(),
            capacity,
            size: size as u64,
            generation: producer.generation(),
            direction: RingDirection::HostToMain,
        };
        Ok(PcmOffer {
            producer,
            message,
            abort: Box::new(CloseRing(abort)),
        })
    }

    fn open(&self, offer: &Message, spec: PcmSpecWire) -> Result<PcmInput<Self::Region>> {
        let Message::RingOffer {
            name, frame_size, ..
        } = offer
        else {
            panic!("expected offer")
        };
        assert_eq!(*frame_size, spec.frame_size());
        let region = self.0.lock().unwrap()[name].clone();
        let expect = offer.ring_expect().unwrap();
        let consumer =
            RingConsumer::attach(region.clone(), expect)?.with_wake(RingWake::open(name)?);
        let abort = RingConsumer::attach(region, expect)?.with_wake(RingWake::open(name)?);
        Ok(PcmInput {
            consumer,
            abort: Box::new(CloseRing(abort)),
        })
    }
}

fn start(
    mode: &str,
    capacity: u32,
) -> (
    DecoderSession<TestRings>,
    mpsc::Receiver<Result<()>>,
    TestRings,
) {
    start_resident(mode, capacity, resident::StubResident)
}

fn start_resident(
    mode: &str,
    capacity: u32,
    resident: impl DecoderResident + Send + 'static,
) -> (
    DecoderSession<TestRings>,
    mpsc::Receiver<Result<()>>,
    TestRings,
) {
    let ((ar, aw), (br, bw)) = streams();
    let client = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
    let rings = TestRings::default();
    let host_rings = rings.clone();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let outcome = serve_decoder(&host, &host_rings, &resident);
        let _ = done.send(outcome);
    });
    let session = DecoderSession::open(
        client,
        rings.clone(),
        MediaLocationWire::Url(mode.into()),
        capacity,
    )
    .unwrap();
    (session, result, rings)
}

struct GatedResident(Mutex<Option<mpsc::Receiver<()>>>);

struct GatedDecoder {
    inner: Box<dyn ResidentDecoder>,
    release: Option<mpsc::Receiver<()>>,
}

impl DecoderResident for GatedResident {
    fn open(&self, location: MediaLocationWire) -> Result<Box<dyn ResidentDecoder>> {
        Ok(Box::new(GatedDecoder {
            inner: resident::StubResident.open(location)?,
            release: self.0.lock().unwrap().take(),
        }))
    }
}

impl ResidentDecoder for GatedDecoder {
    fn info(&self) -> DecoderInfo {
        self.inner.info()
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        if let Some(release) = self.release.take() {
            release
                .recv_timeout(STALL_WAIT)
                .expect("stall: the test never released the gated decode");
        }
        self.inner.read(out)
    }

    fn seek(&mut self, to_ms: u64) -> Result<u64> {
        self.inner.seek(to_ms)
    }
}

fn finish(session: DecoderSession<TestRings>, done: mpsc::Receiver<Result<()>>) {
    drop(session);
    done.recv_timeout(STALL_WAIT)
        .expect("stall: the host worker must publish its shutdown outcome instead of hanging")
        .unwrap();
}

/// Read PCM with a stall escape. A healthy host delivers within
/// milliseconds, so a timeout means it stopped answering without EOS or an
/// error; that is a stall and fails loudly, never a pass.
fn read_pcm(session: &mut DecoderSession<TestRings>, out: &mut [f32], step: &str) -> Result<usize> {
    match session.read(out, Some(STALL_WAIT)) {
        Err(MediaHostError::TimedOut) => panic!(
            "stall: {step} received no PCM within {STALL_WAIT:?} \
             (the resident stopped delivering without EOS or an error)"
        ),
        other => other,
    }
}

#[test]
fn stub_transfer_short_ring_is_exact_lossless_and_eos_is_drained() {
    let (mut session, done, _) = start("fixture:plain", 3);
    let mut received = Vec::new();
    let mut out = [0.0; 22];
    loop {
        let frames = read_pcm(&mut session, &mut out, "the stub transfer").unwrap();
        if frames == 0 {
            break;
        }
        received.extend_from_slice(&out[..frames * 2]);
    }
    let expected: Vec<f32> = (0..resident::TOTAL)
        .flat_map(|index| [index as f32 / 8.0, -(index as f32 / 8.0)])
        .collect();
    assert_eq!(
        received.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        expected.iter().map(|s| s.to_bits()).collect::<Vec<_>>()
    );
    assert_eq!(session.frames_dropped(), 0);
    assert_eq!(session.stats(), (7, Some(128)));
    assert_eq!(
        read_pcm(&mut session, &mut out, "the sticky EOS read").unwrap(),
        0
    );
    finish(session, done);
}

#[test]
fn format_change_drains_old_pcm_before_new_channel_count() {
    let (mut session, done, rings) = start("fixture:format", 3);
    let mut frame = 0;
    let mut changes = 0;
    let mut out = [0.0; 14];
    loop {
        let channels = usize::from(session.info().spec.channels);
        match read_pcm(&mut session, &mut out, "the format-change drain") {
            Ok(0) => break,
            Ok(frames) => {
                for samples in out[..frames * channels].chunks_exact(channels) {
                    assert_eq!(samples[0], frame as f32 / 8.0);
                    if channels == 2 {
                        assert_eq!(samples[1], -samples[0]);
                    }
                    frame += 1;
                }
            }
            Err(MediaHostError::FormatChanged) => {
                assert_eq!(frame, resident::CHANGE);
                assert_eq!(session.info().spec.channels, 1);
                changes += 1;
            }
            Err(error) => panic!("unexpected error: {error}"),
        }
    }
    assert_eq!(frame, resident::TOTAL);
    assert_eq!(changes, 1);
    assert_eq!(rings.0.lock().unwrap().len(), 2);
    assert_eq!(session.frames_dropped(), 0);
    finish(session, done);
}

#[test]
fn seek_interrupts_full_ring_and_discards_prefetch() {
    let (mut session, done, rings) = start("fixture:plain", 1);
    assert_eq!(session.seek(29, STALL_WAIT).unwrap(), 29);
    let mut out = [0.0; 2];
    assert_eq!(
        read_pcm(&mut session, &mut out, "the post-seek read").unwrap(),
        1
    );
    assert_eq!(out, [29.0 / 8.0, -29.0 / 8.0]);
    assert_eq!(rings.0.lock().unwrap().len(), 2);
    finish(session, done);
}

#[test]
fn bounded_read_timeout_is_not_eos_and_retry_delivers_pcm() {
    let (release, blocked) = mpsc::channel();
    let (mut session, done, _) =
        start_resident("fixture:plain", 1, GatedResident(Mutex::new(Some(blocked))));
    let mut out = [0.0; 2];
    assert!(matches!(
        session.read(&mut out, Some(Duration::from_millis(20))),
        Err(MediaHostError::TimedOut)
    ));
    release.send(()).unwrap();
    assert_eq!(
        read_pcm(
            &mut session,
            &mut out,
            "the retry after the bounded timeout"
        )
        .unwrap(),
        1
    );
    finish(session, done);
}

/// Changes telemetry only on a read that produces no PCM. The normal fixture
/// changes it on every successful first block, which cannot catch lost final
/// packet/format telemetry.
struct BoundaryStatsResident;

struct BoundaryStatsDecoder {
    inner: Box<dyn ResidentDecoder>,
    boundaries: u64,
}

impl DecoderResident for BoundaryStatsResident {
    fn open(&self, location: MediaLocationWire) -> Result<Box<dyn ResidentDecoder>> {
        Ok(Box::new(BoundaryStatsDecoder {
            inner: resident::StubResident.open(location)?,
            boundaries: 0,
        }))
    }
}

impl ResidentDecoder for BoundaryStatsDecoder {
    fn info(&self) -> DecoderInfo {
        self.inner.info()
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let result = self.inner.read(out);
        if matches!(result, Ok(0) | Err(MediaHostError::FormatChanged)) {
            self.boundaries += 1;
        }
        result
    }

    fn seek(&mut self, to_ms: u64) -> Result<u64> {
        self.inner.seek(to_ms)
    }

    fn stats(&self) -> (u64, Option<u32>) {
        (self.boundaries, (self.boundaries > 0).then_some(64))
    }
}

#[test]
fn final_packet_statistics_arrive_before_eos_without_another_pcm_block() {
    let (mut session, done, _) = start_resident("fixture:plain", 3, BoundaryStatsResident);
    let mut out = [0.0; 22];
    let mut received = 0;
    loop {
        let frames = read_pcm(&mut session, &mut out, "the final statistics drain").unwrap();
        if frames == 0 {
            break;
        }
        received += frames;
    }
    assert_eq!(received, resident::TOTAL, "statistics must not drop PCM");
    assert_eq!(
        session.stats(),
        (1, Some(64)),
        "final read telemetry was lost"
    );
    finish(session, done);
}

#[test]
fn format_boundary_statistics_arrive_before_format_changed_is_returned() {
    let (mut session, done, _) = start_resident("fixture:format", 3, BoundaryStatsResident);
    let mut out = [0.0; 22];
    let mut received = 0;
    loop {
        match read_pcm(&mut session, &mut out, "the format statistics drain") {
            Ok(0) => panic!("EOS must not hide the format transition"),
            Ok(frames) => received += frames,
            Err(MediaHostError::FormatChanged) => break,
            Err(error) => panic!("unexpected error: {error}"),
        }
    }
    assert_eq!(received, resident::CHANGE);
    assert_eq!(session.info().spec.channels, 1);
    assert_eq!(
        session.stats(),
        (1, Some(64)),
        "format read telemetry was lost"
    );
    finish(session, done);
}

#[test]
fn decoder_error_is_loud_and_never_eos() {
    let (mut session, done, _) = start("fixture:fail", 3);
    let mut out = [0.0; 14];
    loop {
        match read_pcm(&mut session, &mut out, "the failing decode") {
            Ok(0) => panic!("failure must not turn into EOS"),
            Ok(_) => {}
            Err(error) => {
                assert!(
                    error.to_string().contains("fixture decode failure"),
                    "{error}"
                );
                break;
            }
        }
    }
    drop(session);
    assert!(
        done.recv_timeout(STALL_WAIT)
            .expect("stall: the failing host must publish its outcome instead of hanging")
            .is_err()
    );
}

#[test]
fn abrupt_player_loss_wakes_a_full_ring_decoder_writer() {
    let ((ar, aw), (br, bw)) = streams();
    let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
    let rings = TestRings::default();
    let host_rings = rings.clone();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(serve_decoder(&host, &host_rings, &resident::StubResident));
    });
    // Raw player: dropping this stream sends no Shutdown and closes no ring.
    let mut player = StreamTransport::new(ar, aw);
    player.send(&Message::hello()).unwrap();
    player
        .send(&Message::DecoderOpen {
            location: MediaLocationWire::Url("fixture:plain".into()),
            prefetch_frames: 1,
        })
        .unwrap();
    let mut spec = None;
    let input = loop {
        match player.recv().unwrap() {
            Message::Hello { .. } => {}
            Message::Ping(token) => player.send(&Message::Pong(token)).unwrap(),
            Message::DecoderOpened { spec: opened, .. } => spec = Some(opened),
            offer @ Message::RingOffer { .. } => {
                let input = rings.open(&offer, spec.unwrap()).unwrap();
                player
                    .send(&Message::RingReady {
                        kind: RingKind::Pcm,
                        generation: input.consumer.generation(),
                    })
                    .unwrap();
                break input;
            }
            message => panic!("unexpected handshake message: {message:?}"),
        }
    };
    // Wait on publication, not a sleep: with no consumer, capacity one leaves
    // the decoder's remaining block waiting for space forever.
    input
        .consumer
        .wait_readable(Some(STALL_WAIT))
        .expect("stall: the decoder never published into the full ring");
    assert_eq!(input.consumer.frames_available(), 1);
    assert!(!input.consumer.is_closed());
    drop(player);
    let outcome = result
        .recv_timeout(STALL_WAIT)
        .expect("stall: the host must publish its terminal outcome instead of hanging");
    assert!(matches!(outcome, Err(MediaHostError::Disconnected(_))));
    assert!(input.consumer.is_closed(), "EOF must abort the full ring");
}

#[test]
fn abrupt_control_loss_wakes_infinite_pcm_wait_without_eos() {
    let ((ar, aw), (br, bw)) = streams();
    let peer = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let rings = TestRings::default();
    let host_rings = rings.clone();
    let (die, death) = mpsc::channel();
    thread::spawn(move || {
        // Deliberately no ControlPeer or close guard at the fake host end.
        let mut transport = StreamTransport::new(br, bw);
        transport.send(&Message::hello()).unwrap();
        let mut offer = None;
        loop {
            match transport.recv().unwrap() {
                Message::Hello { .. } => {}
                Message::Ping(token) => transport.send(&Message::Pong(token)).unwrap(),
                Message::DecoderOpen {
                    prefetch_frames, ..
                } => {
                    let spec = PcmSpecWire {
                        sample_rate_hz: 1000,
                        channels: 2,
                    };
                    transport
                        .send(&Message::DecoderOpened {
                            spec,
                            duration_ms: None,
                            seekable: false,
                            metadata: Default::default(),
                        })
                        .unwrap();
                    let ring = host_rings.create(spec, prefetch_frames).unwrap();
                    transport.send(&ring.message).unwrap();
                    offer = Some(ring);
                }
                Message::RingReady { .. } => {
                    let mut ring = offer.take().unwrap();
                    ring.producer
                        .write_from_blocking(&[0.25, -0.25], Some(STALL_WAIT))
                        .unwrap();
                    death
                        .recv_timeout(STALL_WAIT)
                        .expect("stall: the test never released the fake host");
                    // RingProducer::Drop does NOT close the ring.
                    break;
                }
                message => panic!("unexpected {message:?}"),
            }
        }
    });
    let mut session =
        DecoderSession::open(peer, rings, MediaLocationWire::Url("fake".into()), 1).unwrap();
    let mut out = [0.0; 2];
    assert_eq!(
        read_pcm(&mut session, &mut out, "the fake host's first block").unwrap(),
        1
    );
    die.send(()).unwrap();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(session.read(&mut out, None));
    });
    let outcome = result
        .recv_timeout(STALL_WAIT)
        .expect("control EOF must wake an infinite PCM wait");
    assert!(matches!(outcome, Err(MediaHostError::Disconnected(_))));
}

#[test]
fn missing_pong_is_bounded_even_with_an_open_control_stream() {
    let ((ar, aw), (br, bw)) = streams();
    let policy = Liveness {
        interval: Duration::from_millis(20),
        timeout: Duration::from_millis(200),
    };
    let peer = ControlPeer::start(ar, aw, policy, false).unwrap();
    let rings = TestRings::default();
    let spec = PcmSpecWire {
        sample_rate_hz: 1000,
        channels: 2,
    };
    let offer = rings.create(spec, 1).unwrap();
    let mut input = rings.open(&offer.message, spec).unwrap();
    peer.set_abort(input.abort).unwrap();
    let (woke, waiter) = mpsc::channel();
    thread::spawn(move || {
        let _ = woke.send(input.consumer.read_into_blocking(&mut [0.0; 2], None));
    });
    let mut unresponsive = StreamTransport::new(br, bw);
    unresponsive.send(&Message::hello()).unwrap();
    let start = Instant::now();
    let outcome = peer.recv(STALL_WAIT);
    assert!(matches!(outcome, Err(MediaHostError::Disconnected(_))));
    // Kept at the original 5 s (not the widened stall escape): the 200 ms
    // policy above is the claim; this only proves the liveness fired, it
    // did not hang.
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(matches!(
        waiter.recv_timeout(STALL_WAIT).unwrap(),
        Err(rustamp_ipc::shm::RingError::Closed)
    ));
    assert!(
        outcome
            .unwrap_err()
            .to_string()
            .contains("liveness deadline")
    );
}

#[test]
fn old_hello_is_rejected_before_decoder_open() {
    let ((ar, aw), (br, bw)) = streams();
    let peer = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let mut old = StreamTransport::new(br, bw);
    old.send(&Message::Hello {
        protocol_version: 2,
        pid: 1,
    })
    .unwrap();
    assert!(matches!(
        peer.recv(STALL_WAIT),
        Err(MediaHostError::Disconnected(_))
    ));
}
