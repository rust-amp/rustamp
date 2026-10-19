//! Delivery must survive a wake signal that never arrives.
//!
//! RFC 0007 §6.1/§6.3 make the ring counters the source of truth and the wake
//! event a hint that may be lost, duplicated or spurious. Both conversations
//! in this crate are therefore required to make progress from the counters
//! alone — and until the `RING_POLL` slice (`src/pcm.rs`) they did not: a
//! player-side read or write, and the decoder worker's write, each spent its
//! whole deadline inside one `WaitForSingleObject`, so a signal that was
//! skipped cost the entire bound instead of one slice. That is the stall the
//! `dsp_roundtrip`/`real_decode` suites hit at their 30 s escape with "the
//! resident stopped delivering without a close or an error"
//! (docs/tasks/W19-twoproc-stall.md).
//!
//! The injection here is total rather than probabilistic: [`DeafRings`] gives
//! every endpoint its **own** pair of wake events, so no `SetEvent` any party
//! makes can ever reach a waiter — not a publish, not a free slot, not even a
//! `close()`. Everything these two tests complete, they complete from the
//! header counters. Each wait carries a short, loud bound: a regression fails
//! in seconds with the step named, it never hangs the binary.
//!
//! Headless, one process, no window, no audio, no file: anonymous shared
//! regions and channel byte streams, exactly like the `pcm_transfer` suite.

#[path = "support/dsp_resident.rs"]
mod resident;
#[path = "support/streams.rs"]
mod streams;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use rustamp_ipc::Message;
use rustamp_ipc::shm::{
    AnonymousRegion, RING_DATA_OFFSET, RingConsumer, RingDirection, RingError, RingKind,
    RingProducer, RingWake,
};
use rustamp_mediahost::control::{ControlPeer, Liveness};
use rustamp_mediahost::decoder::{
    DecoderInfo, DecoderResident, DecoderSession, ResidentDecoder, serve_decoder,
};
use rustamp_mediahost::dsp::{DspSession, serve_dsp};
use rustamp_mediahost::pcm::{CloseRing, PcmInput, PcmOffer, PcmRings, check_offer};
use rustamp_mediahost::{MediaHostError, Result};
use rustamp_proto::host::{DspStageDesc, MediaLocationWire, PcmSpecWire};

/// Bound for every wait in this file. Short on purpose: with the slice in
/// place a deaf ring costs at most one `RING_POLL` (20 ms) per handoff, so a
/// whole transfer needs well under a second. Without it the very first wait
/// runs to this bound, which is the regression failing loudly and fast.
const DEAF_WAIT: Duration = Duration::from_secs(5);
/// Liveness policy: the timeout is a stall escape, never a schedule bound
/// (the M10 flake class). The control plane is unaffected by a deaf ring —
/// it carries its own channels — so this only has to stay out of the way.
const LIVENESS: Liveness = Liveness {
    interval: Duration::from_millis(100),
    timeout: DEAF_WAIT,
};
const SPEC: PcmSpecWire = PcmSpecWire {
    sample_rate_hz: 1000,
    channels: 2,
};
/// Frames per run. Small by design: every handoff is a full slice wait, so
/// the cost is `frames / CAPACITY` slices and the run stays a fraction of a
/// second while still wrapping the ring many times over.
const FRAMES: usize = 64;
/// Ring capacity in frames; `FRAMES` is a whole multiple of it, so the ring
/// wraps exactly eight times and every handoff starts from an empty ring.
const CAPACITY: u32 = 8;

fn sweep(frame: usize) -> [f32; 2] {
    [frame as f32 / 8.0, -(frame as f32 / 8.0)]
}

// ---------------------------------------------------------------------------
// The deaf ring factory
// ---------------------------------------------------------------------------

/// Rings whose four endpoints hold four **disjoint** wake pairs.
///
/// `RingWake::create` makes a fresh pair of auto-reset events, so handing
/// each endpoint its own `create` (never `open`) leaves every `SetEvent`
/// without a listener. The shared `AnonymousRegion` — and with it the header
/// counters and the closed state — is the only thing the two ends have in
/// common, which is exactly the contract under test.
#[derive(Clone, Default)]
struct DeafRings(Arc<Mutex<HashMap<String, AnonymousRegion>>>);

impl DeafRings {
    fn create_impl(
        &self,
        spec: PcmSpecWire,
        capacity: u32,
        direction: RingDirection,
    ) -> Result<PcmOffer<AnonymousRegion>> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let size = RING_DATA_OFFSET + spec.frame_size() as usize * capacity as usize;
        let region = AnonymousRegion::new(size)?;
        let producer =
            RingProducer::create(region.clone(), RingKind::Pcm, spec.frame_size(), capacity)?;
        let name = format!(
            "mediahost-deaf-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let producer = producer.with_wake(RingWake::create(&format!("{name}-producer"))?);
        let abort = RingConsumer::attach(region.clone(), producer.expect())?
            .with_wake(RingWake::create(&format!("{name}-producer-abort"))?);
        self.0.lock().unwrap().insert(name.clone(), region);
        let message = Message::RingOffer {
            name,
            kind: RingKind::Pcm,
            frame_size: spec.frame_size(),
            capacity,
            size: size as u64,
            generation: producer.generation(),
            direction,
        };
        Ok(PcmOffer {
            producer,
            message,
            abort: Box::new(CloseRing(abort)),
        })
    }

    fn open_impl(
        &self,
        offer: &Message,
        spec: PcmSpecWire,
        expected: RingDirection,
    ) -> Result<PcmInput<AnonymousRegion>> {
        // The same rules production enforces: an offer in another direction
        // or with a foreign frame size is refused before any mapping opens.
        check_offer(offer, expected, spec)?;
        let Message::RingOffer { name, .. } = offer else {
            // `check_offer` already refused every other message.
            return Err(MediaHostError::InvalidMessage(
                "expected PCM RingOffer".into(),
            ));
        };
        let region = self.0.lock().unwrap()[name].clone();
        let expect = offer.ring_expect().unwrap();
        let consumer = RingConsumer::attach(region.clone(), expect)?
            .with_wake(RingWake::create(&format!("{name}-consumer"))?);
        let abort = RingConsumer::attach(region, expect)?
            .with_wake(RingWake::create(&format!("{name}-consumer-abort"))?);
        Ok(PcmInput {
            consumer,
            abort: Box::new(CloseRing(abort)),
        })
    }
}

impl PcmRings for DeafRings {
    type Region = AnonymousRegion;

    fn create(&self, spec: PcmSpecWire, capacity: u32) -> Result<PcmOffer<Self::Region>> {
        self.create_impl(spec, capacity, RingDirection::HostToMain)
    }

    fn open(&self, offer: &Message, spec: PcmSpecWire) -> Result<PcmInput<Self::Region>> {
        self.open_impl(offer, spec, RingDirection::HostToMain)
    }

    fn create_directed(
        &self,
        spec: PcmSpecWire,
        capacity: u32,
        direction: RingDirection,
    ) -> Result<PcmOffer<Self::Region>> {
        self.create_impl(spec, capacity, direction)
    }

    fn open_directed(
        &self,
        offer: &Message,
        spec: PcmSpecWire,
        direction: RingDirection,
    ) -> Result<PcmInput<Self::Region>> {
        self.open_impl(offer, spec, direction)
    }
}

// ---------------------------------------------------------------------------
// The offer rules
// ---------------------------------------------------------------------------

/// The error of a refused open. [`PcmInput`] is not `Debug`, so a refused
/// result cannot be unwrapped with `unwrap_err`.
fn refused<T>(result: Result<T>) -> MediaHostError {
    match result {
        Ok(_) => panic!("the open must be refused, not succeed"),
        Err(error) => error,
    }
}

/// The deaf ring double must report a foreign frame size the way production
/// does: with the typed [`RingError::FrameSizeMismatch`], never with a
/// panic.
///
/// Before the rule was shared, this double asserted the frame size with
/// `assert_eq!`, so a mismatched offer aborted the test binary instead of
/// reporting the error production returns.
#[test]
fn deaf_rings_refuse_the_frame_size_production_refuses() {
    let rings = DeafRings::default();
    // Mono: one channel is a 4-byte frame, the offer carries 8.
    let mono = PcmSpecWire {
        sample_rate_hz: SPEC.sample_rate_hz,
        channels: 1,
    };
    let offer = rings.create(SPEC, 1).unwrap();

    // The matching frame size opens.
    assert!(rings.open(&offer.message, SPEC).is_ok());

    // A spec with a different frame size is refused the way production
    // refuses it, naming the size it expected and the one it found.
    let error = refused(rings.open(&offer.message, mono));
    assert!(
        matches!(
            &error,
            MediaHostError::Ring(RingError::FrameSizeMismatch {
                expected: 4,
                found: 8
            })
        ),
        "a foreign frame size must be reported, not panicked: {error}"
    );
    assert_eq!(
        error.to_string(),
        "frame size mismatch: expected 4, found 8"
    );

    // An offer that lies about its frame size is refused with the same
    // error, naming the spec's size as the expected one.
    let mut lying = offer.message.clone();
    if let Message::RingOffer { frame_size, .. } = &mut lying {
        *frame_size = 16;
    }
    let error = refused(rings.open(&lying, SPEC));
    assert!(
        matches!(
            &error,
            MediaHostError::Ring(RingError::FrameSizeMismatch {
                expected: 8,
                found: 16
            })
        ),
        "a lying offer must be reported, not panicked: {error}"
    );
    assert_eq!(
        error.to_string(),
        "frame size mismatch: expected 8, found 16"
    );
}

// ---------------------------------------------------------------------------
// A decoder resident sized for this file
// ---------------------------------------------------------------------------

/// Deterministic resident: `FRAMES` frames of the same sweep the other
/// suites use, then EOS. No file, codec or device.
struct TinyResident;

struct TinyDecoder {
    frame: usize,
}

impl DecoderResident for TinyResident {
    fn open(&self, _location: MediaLocationWire) -> Result<Box<dyn ResidentDecoder>> {
        Ok(Box::new(TinyDecoder { frame: 0 }))
    }
}

impl ResidentDecoder for TinyDecoder {
    fn info(&self) -> DecoderInfo {
        DecoderInfo {
            spec: SPEC,
            duration_ms: Some(FRAMES as u64),
            seekable: false,
            metadata: Default::default(),
        }
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let frames = (FRAMES - self.frame).min(out.len() / 2);
        for (index, slot) in out[..frames * 2]
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .enumerate()
        {
            *slot = sweep(self.frame + index);
        }
        self.frame += frames;
        Ok(frames)
    }

    fn seek(&mut self, _to_ms: u64) -> Result<u64> {
        Err(MediaHostError::OperationFailed("not seekable".into()))
    }
}

// ---------------------------------------------------------------------------
// The two tests
// ---------------------------------------------------------------------------

#[test]
fn a_decoder_transfer_completes_with_every_wake_signal_lost() {
    let ((ar, aw), (br, bw)) = streams::streams();
    let client = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
    let rings = DeafRings::default();
    let host_rings = rings.clone();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(serve_decoder(&host, &host_rings, &TinyResident));
    });
    let mut session = DecoderSession::open(
        client,
        rings,
        MediaLocationWire::Url("deaf".into()),
        CAPACITY,
    )
    .unwrap();

    let mut received = Vec::new();
    let mut out = [0.0f32; CAPACITY as usize * 2];
    loop {
        match session.read(&mut out, Some(DEAF_WAIT)) {
            Ok(0) => break,
            Ok(frames) => received.extend_from_slice(&out[..frames * 2]),
            Err(MediaHostError::TimedOut) => {
                let counters = session.ring_counters();
                panic!(
                    "the deaf transfer stalled after {} frames: a ring wait outlived \
                     {DEAF_WAIT:?} although the counters are the contract \
                     (published {}, read {}, available {}, closed {})",
                    received.len() / 2,
                    counters.published,
                    counters.read,
                    counters.available(),
                    counters.closed
                )
            }
            Err(error) => panic!("unexpected error: {error}"),
        }
    }

    let expected: Vec<f32> = (0..FRAMES).flat_map(sweep).collect();
    assert_eq!(
        received.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        expected.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        "a deaf ring must still deliver every frame, in order and bit-exact"
    );
    assert_eq!(session.frames_dropped(), 0);
    // EOS came from the closed state in the header, not from the close signal.
    assert_eq!(session.read(&mut out, Some(DEAF_WAIT)).unwrap(), 0);
    drop(session);
    result
        .recv_timeout(DEAF_WAIT)
        .expect("the host must end on Shutdown instead of waiting for a signal")
        .unwrap();
}

#[test]
fn a_dsp_round_trip_completes_with_every_wake_signal_lost() {
    let ((ar, aw), (br, bw)) = streams::streams();
    let client = ControlPeer::start(ar, aw, LIVENESS, false).unwrap();
    let host = ControlPeer::start(br, bw, LIVENESS, true).unwrap();
    let rings = DeafRings::default();
    let host_rings = rings.clone();
    let (done, result) = mpsc::channel();
    thread::spawn(move || {
        let _ = done.send(serve_dsp(&host, &host_rings, &resident::StubDspResident));
    });
    let stages = vec![DspStageDesc {
        id: "gain".into(),
        params: 2.0f32.to_le_bytes().to_vec(),
    }];
    let mut session = DspSession::open(client, rings, SPEC, &stages, CAPACITY).unwrap();

    // Lockstep, exactly like the `dsp_roundtrip` suite: write one chunk, read
    // that chunk back. Both directions of the in-ring and the back-ring's
    // data direction are deaf, so every one of these waits has to resolve
    // itself from the counters.
    let chunk = CAPACITY as usize;
    let mut received = Vec::with_capacity(FRAMES * 2);
    let mut out = [0.0f32; CAPACITY as usize * 2];
    let mut next = 0;
    while next < FRAMES {
        let end = (next + chunk).min(FRAMES);
        let block: Vec<f32> = (next..end).flat_map(sweep).collect();
        match session.write_timeout(&block, Some(DEAF_WAIT)) {
            Ok(()) => {}
            Err(MediaHostError::TimedOut) => {
                let counters = session.in_ring_counters();
                panic!(
                    "the deaf in-ring write stalled at frame {next}: a wait outlived \
                     {DEAF_WAIT:?} with capacity {CAPACITY} \
                     (published {}, read {}, available {}, closed {})",
                    counters.published,
                    counters.read,
                    counters.available(),
                    counters.closed
                )
            }
            Err(error) => panic!("unexpected write error: {error}"),
        }
        let mut got = 0;
        while got < end - next {
            match session.read(&mut out, Some(DEAF_WAIT)) {
                Ok(0) => panic!("the host must not end the stream early"),
                Ok(frames) => {
                    received.extend_from_slice(&out[..frames * 2]);
                    got += frames;
                }
                Err(MediaHostError::TimedOut) => {
                    let counters = session.back_ring_counters();
                    panic!(
                        "the deaf back-ring read stalled at frame {}: a wait outlived \
                         {DEAF_WAIT:?} although the counters are the contract \
                         (published {}, read {}, available {}, closed {})",
                        next + got,
                        counters.published,
                        counters.read,
                        counters.available(),
                        counters.closed
                    )
                }
                Err(error) => panic!("unexpected read error: {error}"),
            }
        }
        next = end;
    }

    let expected: Vec<f32> = (0..FRAMES)
        .flat_map(|frame| {
            let [left, right] = sweep(frame);
            [left * 2.0, right * 2.0]
        })
        .collect();
    assert_eq!(
        received.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        expected.iter().map(|s| s.to_bits()).collect::<Vec<_>>(),
        "a deaf pair of rings must still deliver every processed frame"
    );
    assert_eq!(session.frames_dropped(), 0);
    session.finish_writes().unwrap();
    drop(session);
    result
        .recv_timeout(DEAF_WAIT)
        .expect("the host must end on Shutdown instead of waiting for a signal")
        .unwrap();
}
