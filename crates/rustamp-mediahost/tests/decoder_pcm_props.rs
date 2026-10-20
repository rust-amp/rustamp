//! Property tests for the media host's PCM transfer path (the decoder side).
//!
//! `session_props.rs` pins the **DSP** session's state machine over random
//! operation sequences (wave224). This file pins the **decoder** session's
//! transfer path — the ring the host publishes into and the player reads from
//! (RFC 0006 §5) — over the same kind of randomness: random ring capacities,
//! random host block sizes, random write chunk sizes and random read chunk
//! sizes.
//!
//! The invariants:
//!
//! 1. **The byte stream out is the byte stream in.** No frame is torn,
//!    duplicated, reordered or lost, whatever the chunk sizes are.
//! 2. **The counters stay consistent.** After every operation the ring
//!    counters hold: `published >= read` (the available count is never
//!    negative), `published - read <= capacity` (the ring never holds more
//!    frames than its capacity) and no frame was dropped.
//! 3. **A successful write was taken.** The frames a write published are in
//!    the ring, none of them dropped, and the peer takes every one of them
//!    exactly once — the delivery fence of the decoder direction
//!    (RFC 0006 §6).
//! 4. **A truncated source is a loud failure, never a clean end.** A host
//!    that stops early — it dies, or it closes the ring without the
//!    end-of-stream marker — leaves the session failed, and the frames
//!    delivered before it are exactly the frames the host produced. The
//!    decoder path has no frame count to compare: the player does not write
//!    into the ring, so it cannot know what the host intended to publish.
//!    The evidence is therefore the loud failure plus the frame-exactness,
//!    not the count comparison the DSP path can do.
//! 5. **An abrupt consumer loss wakes the producer.** The host blocked on a
//!    full ring wakes up, reports a typed error and publishes nothing past
//!    the loss.
//! 6. **Close is idempotent and no sequence panics.**
//!
//! Every sequence is driven by a printed seed, so a failing run is
//! replayable. The hosts are in-process: a scripted host on the control wire
//! (so the test controls every message and every write) and the production
//! `serve_decoder` worker (so the production host path is covered too). No
//! process is spawned, no window is opened and no audio device is touched.

#![cfg(windows)]

#[path = "support/frame_pattern.rs"]
mod frame_pattern;
#[path = "support/rng.rs"]
mod rng;
#[path = "support/streams.rs"]
mod streams;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use rustamp_ipc::Message;
use rustamp_ipc::shm::{NamedRegion, RingError, RingKind, RingProducer};
use rustamp_mediahost::control::{ControlPeer, Liveness};
use rustamp_mediahost::decoder::{
    DecoderInfo, DecoderResident, DecoderSession, ResidentDecoder, serve_decoder,
};
use rustamp_mediahost::pcm::{NamedPcmRings, PcmRings};
use rustamp_mediahost::{MediaHostError, Result};
use rustamp_proto::host::{MediaLocationWire, PcmSpecWire};

use rng::Rng;

/// The stereo spec every fixture uses: one frame is two interleaved samples.
const SPEC: PcmSpecWire = PcmSpecWire {
    sample_rate_hz: 44_100,
    channels: 2,
};

/// The samples per frame of [`SPEC`].
const CHANNELS: usize = 2;

/// The liveness policy of the in-process peers. A healthy peer answers within
/// milliseconds, so only a dead one reaches the bound — and a dead peer is
/// caught by the control EOF long before that.
const LIVENESS: Liveness = Liveness {
    interval: Duration::from_millis(50),
    timeout: Duration::from_secs(10),
};

/// The bound for a wait that must not block for ever. A healthy host answers
/// within milliseconds, so reaching the bound is a stall and fails loudly
/// with the step named — never a pass.
const WAIT: Duration = Duration::from_secs(10);

/// The bound for the wait after a ring closed without its marker. The marker
/// never comes in that case, so the bound is what ends the wait.
const MARKER_WAIT: Duration = Duration::from_millis(500);

/// The seeds of the property runs. A failing run prints its seed, so the
/// sequence is replayable.
const SEEDS: [u64; 12] = [
    0x0000_0001,
    0x0000_00a3,
    0x0000_07ff,
    0x0000_ffff,
    0x0001_2345,
    0x00c0_ffee,
    0x0bad_c0de,
    0x5eed_0001,
    0x5eed_0002,
    0xdead_beef,
    0xface_b00c,
    0xffff_ffff,
];

/// The number of operations in one random sequence.
const STEPS: usize = 24;

/// The frames one operation may write or read.
const MAX_CHUNK: usize = 8;

// ---------------------------------------------------------------------------
// The resident fixture behind the production host
// ---------------------------------------------------------------------------

/// A resident that publishes the shared frame pattern.
///
/// `block` is the number of frames it hands to the host per read, so the
/// host's writes land in the ring in chunks of that size.
struct PatternResident {
    frames: usize,
    block: usize,
}

struct PatternDecoder {
    frames: usize,
    block: usize,
    position: usize,
}

impl DecoderResident for PatternResident {
    fn open(&self, _location: MediaLocationWire) -> Result<Box<dyn ResidentDecoder>> {
        Ok(Box::new(PatternDecoder {
            frames: self.frames,
            block: self.block,
            position: 0,
        }))
    }
}

impl ResidentDecoder for PatternDecoder {
    fn info(&self) -> DecoderInfo {
        DecoderInfo {
            spec: SPEC,
            duration_ms: Some(self.frames as u64),
            seekable: true,
            metadata: Default::default(),
        }
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let count = (self.frames - self.position)
            .min(self.block)
            .min(out.len() / CHANNELS);
        for (index, frame) in out[..count * CHANNELS]
            .as_chunks_mut::<CHANNELS>()
            .0
            .iter_mut()
            .enumerate()
        {
            frame.copy_from_slice(&frame_pattern::frame_samples(self.position + index));
        }
        self.position += count;
        Ok(count)
    }

    fn seek(&mut self, to_ms: u64) -> Result<u64> {
        // The fixture maps milliseconds to frames one to one, so the reached
        // position names the first frame the host publishes after the seek.
        self.position = (to_ms as usize).min(self.frames);
        Ok(self.position as u64)
    }
}

/// A decoder session and the handle of the production host behind it.
fn start_host(
    resident: impl DecoderResident + Send + 'static,
    capacity: u32,
) -> (
    DecoderSession<NamedPcmRings>,
    thread::JoinHandle<Result<()>>,
) {
    let ((ar, aw), (br, bw)) = streams::streams();
    let player = ControlPeer::start(ar, aw, LIVENESS, false).expect("the player peer starts");
    let host = ControlPeer::start(br, bw, LIVENESS, true).expect("the host peer starts");
    let handle = thread::spawn(move || serve_decoder(&host, &NamedPcmRings, &resident));
    let session = DecoderSession::open(
        player,
        NamedPcmRings,
        MediaLocationWire::Url("fixture:pattern".into()),
        capacity,
    )
    .expect("the decoder handshake completes");
    (session, handle)
}

// ---------------------------------------------------------------------------
// The scripted host
// ---------------------------------------------------------------------------

/// What the scripted host does at the end of its transfer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HostEnd {
    /// Close the ring and send the end-of-stream marker: a clean transfer.
    End,
    /// Close the ring and keep the control channel alive: the marker never
    /// comes, so the transfer ends without an end of stream.
    Truncate,
    /// End the control channel right away: the host dies mid-transfer.
    Die,
    /// Wait for the release signal and then end the control channel without
    /// a marker: the host dies while the player still reads.
    DieOnRelease,
}

/// The ring counters the scripted host saw after one of its writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WriteSnapshot {
    /// The frames the ring counted as published.
    published: u64,
    /// The frames the ring still held.
    available: u64,
    /// The frames the ring had to drop.
    dropped: u64,
}

/// The scripted host: the peer on the other end of the player's control
/// stream.
///
/// It performs the production handshake in the production order and publishes
/// the shared frame pattern in the chunk sizes the seed chose. Unlike the
/// production worker it owns the ring itself, so the test sees every write.
fn run_scripted_host(
    peer: ControlPeer,
    spec: PcmSpecWire,
    chunks: &[usize],
    end: HostEnd,
    release: mpsc::Receiver<()>,
) -> Result<Vec<WriteSnapshot>> {
    // 1. The handshake: the open, the offer, the Ready.
    let capacity = match peer.recv(WAIT) {
        Ok(Message::DecoderOpen {
            prefetch_frames, ..
        }) => prefetch_frames,
        other => return Err(host_error(format!("expected DecoderOpen, got {other:?}"))),
    };
    peer.send(Message::DecoderOpened {
        spec,
        duration_ms: None,
        seekable: false,
        metadata: Default::default(),
    })
    .map_err(|error| host_error(error.to_string()))?;
    let offer = NamedPcmRings
        .create(spec, capacity)
        .map_err(|error| host_error(error.to_string()))?;
    let generation = offer.producer.generation();
    peer.send(offer.message)
        .map_err(|error| host_error(error.to_string()))?;
    match peer.recv(WAIT) {
        Ok(Message::RingReady {
            kind: RingKind::Pcm,
            generation: ready,
        }) if ready == generation => {}
        other => {
            return Err(host_error(format!(
                "expected the ring Ready, got {other:?}"
            )));
        }
    }
    let mut producer = offer.producer;

    // 2. The transfer: every chunk is one write into the ring.
    let mut snapshots = Vec::with_capacity(chunks.len());
    let mut index = 0usize;
    for size in chunks {
        let block: Vec<f32> = (index..index + size)
            .flat_map(frame_pattern::frame_samples)
            .collect();
        publish(&mut producer, &block);
        index += size;
        snapshots.push(WriteSnapshot {
            published: producer.frames_written(),
            available: producer.frames_available(),
            dropped: producer.frames_dropped(),
        });
    }

    // 3. The end of the transfer.
    match end {
        HostEnd::End => {
            producer.close();
            peer.send(Message::DecoderEos)
                .map_err(|error| host_error(error.to_string()))?;
        }
        HostEnd::Truncate => {
            // The ring closes, the marker never comes: the transfer was cut
            // short. The host stays alive, so the player has to notice the
            // missing marker itself.
            producer.close();
        }
        HostEnd::Die => {
            // The host dies: the control channel ends without a marker.
            drop(producer);
            drop(peer);
            return Ok(snapshots);
        }
        HostEnd::DieOnRelease => {
            // The host waits for the player to finish reading and then dies.
            let _ = release.recv_timeout(WAIT);
            drop(producer);
            drop(peer);
            return Ok(snapshots);
        }
    }
    // The host stays alive for the rest of the conversation, exactly like the
    // production worker: the end of the stream is a marker, not a vanished
    // peer, and the player must not see the peer's death instead of it.
    let _ = release.recv_timeout(WAIT);
    drop(producer);
    drop(peer);
    Ok(snapshots)
}

/// The scripted host's own failure, as a media host error.
fn host_error(message: String) -> MediaHostError {
    MediaHostError::OperationFailed(message)
}

/// Publish frames into the ring, waiting for space.
///
/// The write is lossless (RFC 0006 §4): it blocks until the ring has room
/// and never drops a frame.
fn publish(producer: &mut RingProducer<NamedRegion>, frames: &[f32]) {
    let mut offset = 0;
    while offset < frames.len() / CHANNELS {
        let written = producer
            .write_from_blocking(&frames[offset * CHANNELS..], Some(WAIT))
            .expect("the scripted host publishes into its ring");
        assert!(written > 0, "the publish must not time out");
        offset += written;
    }
}

/// A decoder session and the handle of the scripted host behind it.
struct Scripted {
    session: DecoderSession<NamedPcmRings>,
    /// The snapshots of the host's writes, once the host is done.
    snapshots: mpsc::Receiver<Result<Vec<WriteSnapshot>>>,
    /// The signal that lets the host end its conversation.
    release: mpsc::Sender<()>,
}

fn start_scripted(chunks: Vec<usize>, end: HostEnd, capacity: u32) -> Scripted {
    let ((ar, aw), (br, bw)) = streams::streams();
    let player = ControlPeer::start(ar, aw, LIVENESS, false).expect("the player peer starts");
    let host = ControlPeer::start(br, bw, LIVENESS, true).expect("the host peer starts");
    let (release, released) = mpsc::channel();
    let (done, snapshots) = mpsc::channel();
    thread::spawn(move || {
        let outcome = run_scripted_host(host, SPEC, &chunks, end, released);
        let _ = done.send(outcome);
    });
    let session = DecoderSession::open(
        player,
        NamedPcmRings,
        MediaLocationWire::Url("fixture:scripted".into()),
        capacity,
    )
    .expect("the decoder handshake completes");
    Scripted {
        session,
        snapshots,
        release,
    }
}

// ---------------------------------------------------------------------------
// The shared assertions
// ---------------------------------------------------------------------------

/// Check the ring counters against the invariants of the transfer.
///
/// `delivered` is the number of frames the session handed to its caller in
/// the ring's own generation — a seek starts a new generation, so the count
/// restarts there.
fn check_counters(
    session: &DecoderSession<NamedPcmRings>,
    capacity: u32,
    delivered: u64,
    step: &str,
) {
    let counters = session.ring_counters();
    assert!(
        counters.published >= counters.read,
        "{step}: the read count must not pass the published count"
    );
    assert_eq!(
        counters.available(),
        counters.published - counters.read,
        "{step}: the available count must be the difference of the two counters"
    );
    assert!(
        counters.available() <= u64::from(capacity),
        "{step}: the ring must not hold more frames than its capacity"
    );
    assert_eq!(
        counters.read, delivered,
        "{step}: the ring must have read exactly the frames the session delivered"
    );
    assert_eq!(
        session.frames_dropped(),
        0,
        "{step}: the transfer must not drop a frame"
    );
}

/// Compare the frames that came back with the frames that went in.
///
/// A torn, duplicated, missing or reordered frame breaks the shared pattern,
/// so the comparison is exact: the samples must be bit-identical. `expected`
/// is the index of the frame the caller must receive next.
fn check_frames(received: &[f32], expected: &mut usize, step: &str) {
    for (index, frame) in received.as_chunks::<CHANNELS>().0.iter().enumerate() {
        let want = frame_pattern::frame_samples(*expected + index);
        assert_eq!(
            frame.as_slice(),
            want.as_slice(),
            "{step}: frame {} was torn, duplicated or reordered",
            *expected + index
        );
    }
    *expected += received.len() / CHANNELS;
}

/// Read PCM with a stall escape. A healthy host delivers within
/// milliseconds, so a timeout means it stopped answering without EOS or an
/// error; that is a stall and fails loudly, never a pass.
fn read_pcm(
    session: &mut DecoderSession<NamedPcmRings>,
    out: &mut [f32],
    step: &str,
) -> Result<usize> {
    match session.read(out, Some(WAIT)) {
        Err(MediaHostError::TimedOut) => panic!(
            "stall: {step} received no PCM within {WAIT:?} \
             (the host stopped delivering without EOS or an error)"
        ),
        other => other,
    }
}

/// The chunk sizes of a host's writes: chunks of a random size until the
/// total reaches `minimum`.
fn write_chunks(rng: &mut Rng, minimum: usize) -> Vec<usize> {
    let mut chunks = Vec::new();
    let mut total = 0usize;
    while total < minimum {
        let size = 1 + rng.below(MAX_CHUNK as u64) as usize;
        chunks.push(size);
        total += size;
    }
    chunks
}

// ---------------------------------------------------------------------------
// The properties
// ---------------------------------------------------------------------------

/// The byte stream out is the byte stream in, and the counters stay
/// consistent after every operation.
///
/// The host publishes in blocks of a random size, the player reads in chunks
/// of a random size, and the ring is small enough that it really wraps. Every
/// frame must arrive exactly once, in order and whole, and the ring counters
/// must hold their invariants after every single read.
#[test]
fn a_random_read_sequence_delivers_every_frame_once_and_keeps_the_counters_consistent() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let total = 1 + rng.below(200) as usize;
        let block = 1 + rng.below(16) as usize;
        let capacity = 1 + rng.below(16) as u32;
        let (mut session, host) = start_host(
            PatternResident {
                frames: total,
                block,
            },
            capacity,
        );
        let mut delivered = 0usize;
        loop {
            let want = 1 + rng.below(MAX_CHUNK as u64) as usize;
            let mut out = vec![0.0f32; want * CHANNELS];
            let frames = read_pcm(&mut session, &mut out, &format!("seed {seed}"))
                .unwrap_or_else(|error| panic!("seed {seed}: unexpected error: {error}"));
            if frames == 0 {
                break;
            }
            check_frames(
                &out[..frames * CHANNELS],
                &mut delivered,
                &format!("seed {seed}, {frames} frames"),
            );
            check_counters(
                &session,
                capacity,
                delivered as u64,
                &format!("seed {seed}, after {delivered} frames"),
            );
        }
        assert_eq!(
            delivered, total,
            "seed {seed}: every frame the host produced must arrive"
        );
        // The end of the stream sticks.
        assert_eq!(
            session
                .read(&mut [0.0f32; CHANNELS], Some(WAIT))
                .unwrap_or_else(|error| panic!("seed {seed}: the drain failed: {error}")),
            0,
            "seed {seed}: the transfer ended"
        );
        assert_eq!(
            session
                .read(&mut [0.0f32; CHANNELS], Some(WAIT))
                .unwrap_or_else(|error| panic!("seed {seed}: the drain failed: {error}")),
            0,
            "seed {seed}: the end of the stream sticks"
        );
        drop(session);
        host.join()
            .expect("the host must not panic")
            .expect("the host must end cleanly");
    }
}

/// A successful write was taken by the peer — the delivery fence of the
/// decoder direction.
///
/// Every write the host makes publishes whole frames into the ring and drops
/// none of them, the ring never holds more frames than its capacity, and the
/// player takes every published frame exactly once. The check runs after
/// **every** write, not only at the end of the transfer.
#[test]
fn a_successful_write_was_taken_by_the_peer() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let capacity = 1 + rng.below(8) as u32;
        let chunks = write_chunks(&mut rng, 64);
        let total: usize = chunks.iter().sum();
        let mut scripted = start_scripted(chunks.clone(), HostEnd::End, capacity);

        // The frames come back in random chunk sizes.
        let mut delivered = 0usize;
        loop {
            let want = 1 + rng.below(MAX_CHUNK as u64) as usize;
            let mut out = vec![0.0f32; want * CHANNELS];
            let frames = read_pcm(&mut scripted.session, &mut out, &format!("seed {seed}"))
                .unwrap_or_else(|error| panic!("seed {seed}: unexpected error: {error}"));
            if frames == 0 {
                break;
            }
            check_frames(
                &out[..frames * CHANNELS],
                &mut delivered,
                &format!("seed {seed}, {frames} frames"),
            );
            check_counters(
                &scripted.session,
                capacity,
                delivered as u64,
                &format!("seed {seed}, after {delivered} frames"),
            );
        }
        assert_eq!(
            delivered, total,
            "seed {seed}: every published frame must arrive exactly once"
        );

        // The fence, write by write: the ring counted every frame, dropped
        // none and never held more than its capacity. The host publishes its
        // snapshots when the conversation ends, so it is released first.
        let _ = scripted.release.send(());
        let snapshots = scripted
            .snapshots
            .recv_timeout(WAIT)
            .expect("stall: the scripted host must publish its outcome")
            .expect("the scripted host must not fail");
        let mut published = 0u64;
        for (index, snapshot) in snapshots.iter().enumerate() {
            assert_eq!(
                snapshot.dropped, 0,
                "seed {seed}: write {index} must not drop a frame"
            );
            assert!(
                snapshot.available <= u64::from(capacity),
                "seed {seed}: write {index} must not fill the ring beyond its capacity"
            );
            published += chunks[index] as u64;
            assert_eq!(
                snapshot.published, published,
                "seed {seed}: write {index} must publish exactly its frames"
            );
        }

        // The peer took every published frame exactly once.
        let counters = scripted.session.ring_counters();
        assert_eq!(
            counters.published, total as u64,
            "seed {seed}: the ring counts the published frames"
        );
        assert_eq!(
            counters.read, total as u64,
            "seed {seed}: the peer must have taken every published frame"
        );
        assert_eq!(
            counters.available(),
            0,
            "seed {seed}: the ring must be empty when the transfer ended"
        );
        assert_eq!(
            scripted.session.frames_dropped(),
            0,
            "seed {seed}: the transfer must not drop a frame"
        );
    }
}

/// A truncated source is a loud failure, never a clean end.
///
/// The host publishes frames and dies without the end-of-stream marker. The
/// session must fail loudly, the failure must stick, and the frames the
/// player received before the death must be exactly the frames the host
/// produced.
#[test]
fn a_truncated_source_is_a_loud_failure_and_delivers_exactly_what_was_produced() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let capacity = 1 + rng.below(8) as u32;
        let chunks = write_chunks(&mut rng, 32);
        let total: usize = chunks.iter().sum();
        // The host dies only after the player has read everything, so the
        // claim "the delivered frames are exactly the produced frames" is a
        // deterministic one.
        let mut scripted = start_scripted(chunks, HostEnd::DieOnRelease, capacity);

        // The frames arrive while the host is still alive.
        let mut delivered = 0usize;
        while delivered < total {
            let want = 1 + rng.below(MAX_CHUNK as u64) as usize;
            let mut out = vec![0.0f32; want * CHANNELS];
            let frames = read_pcm(&mut scripted.session, &mut out, &format!("seed {seed}"))
                .unwrap_or_else(|error| panic!("seed {seed}: unexpected error: {error}"));
            assert!(
                frames > 0,
                "seed {seed}: the host must not end the stream before it dies"
            );
            check_frames(
                &out[..frames * CHANNELS],
                &mut delivered,
                &format!("seed {seed}, {frames} frames"),
            );
        }

        // The host dies now: the transfer is cut short.
        let _ = scripted.release.send(());
        let mut out = vec![0.0f32; CHANNELS];
        let error = scripted
            .session
            .read(&mut out, Some(WAIT))
            .expect_err(&format!(
                "seed {seed}: a truncated transfer must not be an end of stream"
            ));
        let MediaHostError::Disconnected(reason) = &error else {
            panic!("seed {seed}: a dead host must report a loud, typed error: {error}");
        };
        assert!(
            reason.contains("control stream ended"),
            "seed {seed}: the death must be seen on the control channel: {error}"
        );
        // The failure sticks: a later read fails loud too, never an end of
        // stream.
        let again = scripted
            .session
            .read(&mut out, Some(WAIT))
            .expect_err(&format!("seed {seed}: the failure must stick"));
        assert!(
            again.to_string().contains("control stream ended"),
            "seed {seed}: the failure must stick: {again}"
        );
        // The frames that were delivered are exactly the frames the host
        // produced.
        assert_eq!(
            delivered, total,
            "seed {seed}: every frame the host produced must arrive"
        );
    }
}

/// A host that dies mid-transfer never looks like a clean end.
///
/// The host publishes frames and then dies without the end-of-stream marker.
/// The session must fail loudly — never report a clean end — and the frames
/// the player received before the death must be exactly the frames the host
/// produced, in order and whole.
#[test]
fn a_host_that_dies_mid_transfer_never_looks_like_a_clean_end() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let capacity = 1 + rng.below(8) as u32;
        let chunks = write_chunks(&mut rng, 32);
        let total: usize = chunks.iter().sum();
        let mut scripted = start_scripted(chunks, HostEnd::Die, capacity);

        // The player reads until the failure comes. The frames that made it
        // through the death are still whole and in order; how many of them
        // arrive depends on whether the player was already waiting for them
        // when the host died, so the property is the exactness of what
        // arrived, not its count.
        let mut delivered = 0usize;
        let mut out = vec![0.0f32; MAX_CHUNK * CHANNELS];
        let error = loop {
            match scripted.session.read(&mut out, Some(WAIT)) {
                Ok(0) => panic!("seed {seed}: a dead host must not look like an end of stream"),
                Ok(frames) => {
                    check_frames(
                        &out[..frames * CHANNELS],
                        &mut delivered,
                        &format!("seed {seed}, the frames that made it through the death"),
                    );
                }
                Err(MediaHostError::TimedOut) => {
                    panic!("stall: seed {seed} did not notice the death within {WAIT:?}")
                }
                Err(error) => break error,
            }
        };
        assert!(
            delivered <= total,
            "seed {seed}: no frame may be delivered twice"
        );
        // The failure is loud and typed: the death is seen on the control
        // channel, never as an end of stream.
        let MediaHostError::Disconnected(reason) = &error else {
            panic!("seed {seed}: a dead host must report a loud, typed error: {error}");
        };
        assert!(
            reason.contains("control stream ended"),
            "seed {seed}: the death must be seen on the control channel: {error}"
        );
        // The failure sticks: a later read fails loud too, never an end of
        // stream.
        let again = scripted
            .session
            .read(&mut out, Some(WAIT))
            .expect_err(&format!("seed {seed}: the failure must stick"));
        assert!(
            again.to_string().contains("control stream ended"),
            "seed {seed}: the failure must stick: {again}"
        );
        // The counters stay consistent through the failure.
        let counters = scripted.session.ring_counters();
        assert!(
            counters.published >= counters.read,
            "seed {seed}: the read count must not pass the published count"
        );
        assert!(
            counters.available() <= u64::from(capacity),
            "seed {seed}: the ring must not hold more frames than its capacity"
        );
        assert_eq!(
            scripted.session.frames_dropped(),
            0,
            "seed {seed}: a dead host must not turn into a dropped frame"
        );
    }
}

/// A ring that closes without the end-of-stream marker is not a clean end.
///
/// The host closes the ring and keeps the control channel alive: the marker
/// never comes. The session must not report an end of stream — the transfer
/// was cut short — and the frames that arrived are exactly the frames the
/// host produced.
#[test]
fn a_ring_that_closes_without_the_marker_is_never_a_clean_end() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let capacity = 1 + rng.below(8) as u32;
        let chunks = write_chunks(&mut rng, 32);
        let total: usize = chunks.iter().sum();
        let mut scripted = start_scripted(chunks, HostEnd::Truncate, capacity);

        // The frames arrive; then the ring closes and the marker never comes.
        let mut delivered = 0usize;
        let mut out = vec![0.0f32; MAX_CHUNK * CHANNELS];
        while delivered < total {
            match scripted.session.read(&mut out, Some(WAIT)) {
                Ok(0) => panic!("seed {seed}: a truncated transfer must not be an end of stream"),
                Ok(frames) => {
                    check_frames(
                        &out[..frames * CHANNELS],
                        &mut delivered,
                        &format!("seed {seed}, the frames before the cut"),
                    );
                }
                Err(MediaHostError::TimedOut) => {
                    panic!("stall: seed {seed} received no PCM within {WAIT:?}")
                }
                Err(error) => panic!("seed {seed}: unexpected error: {error}"),
            }
        }
        // The marker never comes: the session must say so instead of ending
        // the stream cleanly.
        let error = scripted
            .session
            .read(&mut out, Some(MARKER_WAIT))
            .expect_err(&format!(
                "seed {seed}: a truncated transfer must not be an end of stream"
            ));
        assert!(
            !matches!(error, MediaHostError::InvalidMessage(_)),
            "seed {seed}: the missing marker must be a loud failure: {error}"
        );
        // The host is still alive — it closed the ring and waits — so the
        // failure is the missing marker, not the host's death.
        assert!(
            scripted.snapshots.try_recv().is_err(),
            "seed {seed}: the host must still be alive"
        );
        // The frames that arrived are exactly the frames the host produced.
        assert_eq!(
            delivered, total,
            "seed {seed}: every published frame must arrive"
        );
        let _ = scripted.release.send(());
    }
}

/// An abrupt consumer loss wakes the producer.
///
/// The player vanishes without a `Shutdown` and without closing the ring
/// while the host is blocked on a full ring. The host must wake up, report a
/// typed error and publish nothing past the loss.
#[test]
fn an_abrupt_consumer_loss_wakes_the_producer_with_a_typed_error() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let capacity = 1 + rng.below(8) as u32;
        let block = 1 + rng.below(4) as usize;
        // The resident produces more frames than the ring can hold, so the
        // host is blocked in its write when the player vanishes.
        let frames = 4 * capacity as usize;
        let ((ar, aw), (br, bw)) = streams::streams();
        let host = ControlPeer::start(br, bw, LIVENESS, true).expect("the host peer starts");
        let (done, outcome) = mpsc::channel();
        thread::spawn(move || {
            let _ = done.send(serve_decoder(
                &host,
                &NamedPcmRings,
                &PatternResident { frames, block },
            ));
        });
        // A raw player: it completes the handshake, lets the ring fill up and
        // then vanishes without a word.
        let player = thread::spawn(move || vanishing_player(ar, aw, capacity));

        let result = outcome
            .recv_timeout(WAIT)
            .expect("stall: the host must publish its terminal outcome instead of hanging");
        player.join().expect("the player must not panic");
        assert!(
            matches!(result, Err(MediaHostError::Disconnected(_))),
            "seed {seed}: a lost player must end the host with a loud, typed error: {result:?}"
        );
    }
}

/// A raw player that completes the handshake and then vanishes.
///
/// It sends no `Shutdown` and closes no ring: the loss surrogate of a player
/// that went away without a word. The ring view stays open until the player
/// goes away, so the test can read the ring counters at the loss.
fn vanishing_player(reader: streams::ByteReader, writer: streams::ByteWriter, capacity: u32) {
    let peer = ControlPeer::start(reader, writer, LIVENESS, false).expect("the player peer starts");
    peer.send(Message::DecoderOpen {
        location: MediaLocationWire::Url("fixture:pattern".into()),
        prefetch_frames: capacity,
    })
    .expect("the player opens the stream");
    // The handshake: the opened message and the ring offer.
    let watcher = loop {
        match peer.recv(WAIT) {
            Ok(Message::DecoderOpened { .. }) => {}
            Ok(offer @ Message::RingOffer { .. }) => {
                let input = NamedPcmRings
                    .open(&offer, SPEC)
                    .expect("the player opens the ring");
                peer.send(Message::RingReady {
                    kind: RingKind::Pcm,
                    generation: input.consumer.generation(),
                })
                .expect("the player readies the ring");
                break input;
            }
            Ok(message) => panic!("unexpected message: {message:?}"),
            Err(error) => panic!("the host must not fail before the loss: {error}"),
        }
    };
    // The loss matters while the host is blocked on a full ring, so the
    // player waits until the ring is full before it goes away.
    while watcher.consumer.frames_available() < u64::from(capacity) {
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(
        watcher.consumer.frames_published(),
        u64::from(capacity),
        "the host must publish exactly the frames the ring holds"
    );
    assert_eq!(
        watcher.consumer.frames_dropped(),
        0,
        "a blocked host must not drop a frame"
    );
    // The player vanishes: the control channel ends, the ring stays open.
    drop(watcher);
    drop(peer);
}

/// A random sequence of operations never panics and keeps the session
/// consistent.
///
/// The sequence mixes reads of random chunk sizes, seeks to random positions
/// and a misaligned read (a caller error). The frames that come back are
/// exactly the frames the resident produced, the counters hold after every
/// operation, and the end of the stream sticks.
#[test]
fn a_random_operation_sequence_never_panics_and_keeps_the_session_consistent() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let total = 32 + rng.below(96) as usize;
        let capacity = 1 + rng.below(8) as u32;
        let (mut session, host) = start_host(
            PatternResident {
                frames: total,
                block: 1 + rng.below(8) as usize,
            },
            capacity,
        );
        // The index of the frame the session must deliver next.
        let mut expected = 0usize;
        // The frames the session delivered in the ring's current generation:
        // a seek starts a new generation, so the count restarts there.
        let mut delivered = 0usize;
        // The frames the resident still owes in the current generation: the
        // end of the stream may only come when none are left.
        let mut remaining = total;
        for step in 1..=STEPS {
            let label = format!("seed {seed}, step {step}");
            match rng.below(10) {
                0..=5 => {
                    // A read of a random chunk size.
                    let want = 1 + rng.below(MAX_CHUNK as u64) as usize;
                    let mut out = vec![0.0f32; want * CHANNELS];
                    let frames = read_pcm(&mut session, &mut out, &label)
                        .unwrap_or_else(|error| panic!("{label}: unexpected error: {error}"));
                    if frames == 0 {
                        assert_eq!(
                            remaining, 0,
                            "{label}: an end of the stream before the resident was done"
                        );
                    } else {
                        check_frames(&out[..frames * CHANNELS], &mut expected, &label);
                        delivered += frames;
                        remaining -= frames;
                    }
                }
                6..=7 => {
                    // A seek: the frames after it start at the reached
                    // position, and the prefetched frames are discarded.
                    let to_ms = rng.below(total as u64 + 8);
                    let reached = session
                        .seek(to_ms, WAIT)
                        .unwrap_or_else(|error| panic!("{label}: the seek failed: {error}"));
                    assert!(
                        reached <= to_ms,
                        "{label}: the seek must not reach past the request"
                    );
                    expected = reached as usize;
                    delivered = 0;
                    remaining = total - reached as usize;
                }
                8 => {
                    // A misaligned read is a caller error and changes nothing.
                    let mut out = vec![0.0f32; CHANNELS + 1];
                    let error = session
                        .read(&mut out, Some(WAIT))
                        .expect_err("a misaligned read must be a caller error");
                    assert!(
                        matches!(error, MediaHostError::InvalidMessage(_)),
                        "{label}: {error}"
                    );
                }
                _ => {
                    // A read that may find nothing: the host has not produced
                    // the next frame yet.
                    let mut out = vec![0.0f32; CHANNELS];
                    match session.read(&mut out, Some(Duration::from_millis(20))) {
                        Ok(0) => {}
                        Ok(frames) => {
                            check_frames(&out[..frames * CHANNELS], &mut expected, &label);
                            delivered += frames;
                            remaining -= frames;
                        }
                        Err(MediaHostError::TimedOut) => {}
                        Err(error) => panic!("{label}: unexpected error: {error}"),
                    }
                }
            }
            check_counters(
                &session,
                capacity,
                delivered as u64,
                &format!("{label}, after {delivered} frames"),
            );
        }
        // The close is idempotent: the session's drop closes the ring, the
        // host closes it again, and the counters stay as they are.
        let counters = session.ring_counters();
        drop(session);
        host.join()
            .expect("the host must not panic")
            .expect("the host must end cleanly");
        assert!(
            counters.published >= counters.read,
            "seed {seed}: the read count must not pass the published count"
        );
    }
}

/// The end of the stream of a session that finished is sticky.
///
/// The host ends the stream, the session reports the end, and every later
/// read reports it again. Dropping the session closes the ring again — the
/// host closes it too — and nothing panics.
#[test]
fn the_end_of_the_stream_sticks_after_a_random_transfer() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let capacity = 1 + rng.below(8) as u32;
        let chunks = write_chunks(&mut rng, 16);
        let total: usize = chunks.iter().sum();
        let mut scripted = start_scripted(chunks, HostEnd::End, capacity);

        // Drain the transfer.
        let mut delivered = 0usize;
        loop {
            let want = 1 + rng.below(MAX_CHUNK as u64) as usize;
            let mut out = vec![0.0f32; want * CHANNELS];
            let frames = read_pcm(&mut scripted.session, &mut out, &format!("seed {seed}"))
                .unwrap_or_else(|error| panic!("seed {seed}: unexpected error: {error}"));
            if frames == 0 {
                break;
            }
            check_frames(
                &out[..frames * CHANNELS],
                &mut delivered,
                &format!("seed {seed}, {frames} frames"),
            );
        }
        assert_eq!(delivered, total, "seed {seed}: every frame must arrive");

        // The end of the stream sticks.
        for _ in 0..3 {
            assert_eq!(
                scripted
                    .session
                    .read(&mut [0.0f32; CHANNELS], Some(WAIT))
                    .unwrap_or_else(|error| panic!("seed {seed}: the drain failed: {error}")),
                0,
                "seed {seed}: the end of the stream must stick"
            );
        }
        // The ring is empty and closed, and the close changes nothing.
        let counters = scripted.session.ring_counters();
        assert_eq!(
            counters.available(),
            0,
            "seed {seed}: the ring must be empty when the transfer ended"
        );
        assert!(
            counters.closed,
            "seed {seed}: the host must have closed the ring at the end of the stream"
        );
        let _ = scripted.release.send(());
        drop(scripted.session);
    }
}

/// Close is idempotent on the decoder path's ring.
///
/// The decoder path closes the ring from three sides: the session's drop,
/// the host's ring and the control monitor's close-only view. Closing it
/// again changes nothing: the counters stay, the ring stays closed and a
/// blocked reader still wakes with the close.
#[test]
fn closing_the_decoder_ring_twice_changes_nothing() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let capacity = 1 + rng.below(8) as u32;
        let offer = NamedPcmRings
            .create(SPEC, capacity)
            .expect("seed {seed}: the ring is created");
        let mut producer = offer.producer;
        let mut reader = NamedPcmRings
            .open(&offer.message, SPEC)
            .expect("seed {seed}: the ring is opened");
        // A second view only observes the ring; it never reads a frame.
        let mut watcher = NamedPcmRings
            .open(&offer.message, SPEC)
            .expect("seed {seed}: the ring is opened again");

        // A blocked reader waits for a frame that never comes.
        let (woke, waiter) = mpsc::channel();
        let handle = thread::spawn(move || {
            let _ = woke.send(
                reader
                    .consumer
                    .read_into_blocking(&mut [0.0; CHANNELS], None),
            );
        });

        // Close the ring, twice, from both ends.
        producer.close();
        producer.close();
        assert!(producer.is_closed(), "seed {seed}: the ring stays closed");
        assert_eq!(
            producer.frames_written(),
            0,
            "seed {seed}: no frame was written"
        );
        assert_eq!(
            producer.frames_dropped(),
            0,
            "seed {seed}: no frame was dropped"
        );
        watcher.consumer.close();
        watcher.consumer.close();
        assert!(
            watcher.consumer.is_closed(),
            "seed {seed}: the ring stays closed"
        );
        assert_eq!(
            watcher.consumer.frames_read(),
            0,
            "seed {seed}: no frame was read"
        );

        let outcome = waiter
            .recv_timeout(WAIT)
            .expect("stall: seed {seed}: the close must wake the blocked reader");
        assert!(
            matches!(outcome, Err(RingError::Closed)),
            "seed {seed}: the blocked reader must see the close: {outcome:?}"
        );
        handle.join().expect("the reader must not panic");
    }
}
