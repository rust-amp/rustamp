//! Seeded property tests for the shared-memory ring (RFC 0003 sections 6-8,
//! RFC 0007).
//!
//! The ring is the data structure every other subsystem stands on, so its
//! invariants are checked over many random operation sequences instead of a
//! handful of hand-written examples. The generator is this file's own
//! SplitMix64: no new dependency, and every case is reproducible from the
//! seed the failure message prints.
//!
//! The properties are:
//!
//! 1. **No torn, duplicated or lost frame.** Over random read and write chunk
//!    sizes the frames the consumer takes are exactly the frames the producer
//!    accepted, in order, byte for byte. Every offered frame is either
//!    accepted, dropped and counted, or still in the ring - never lost.
//! 2. **The counters stay consistent.** `read_seq <= write_seq`, the occupancy
//!    never exceeds the capacity, free space and occupancy add up to the
//!    capacity, and no counter ever moves backwards or overflows.
//! 3. **`close()` is idempotent** and a blocked peer wakes with
//!    `Err(RingError::Closed)`.
//! 4. **A generation reset invalidates the old peer** (it is stale) and leaves
//!    the new generation usable.
//! 5. **No operation panics**, not even a write larger than the capacity or a
//!    read on an empty ring.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rustamp_ipc::shm::{
    AnonymousRegion, RING_DATA_OFFSET, RingConsumer, RingError, RingKind, RingProducer, RingWake,
    VIS_SLOT_SIZE, VisSlot,
};

/// Seed of the random scenario stream.
const SEED: u64 = 0x5EED_2060_0000_0250;

/// Number of independent random scenarios per property.
const ROUNDS: usize = 400;

/// Number of operations per random scenario.
const STEPS: usize = 48;

/// Number of concurrent (two-thread) scenarios per property.
const CONCURRENT_ROUNDS: usize = 64;

/// Number of frames a scenario offers to the producer.
const TOTAL_FRAMES: u64 = 600;

/// Upper bound for a blocking call in a test.
///
/// A test must fail loudly rather than hang the suite when a wake is lost
/// (RFC 0007 section 6.1).
const WAIT: Option<Duration> = Some(Duration::from_secs(10));

/// Upper bound for the result of a blocked peer.
const RESULT_WAIT: Duration = Duration::from_secs(5);

/// How long the peer is given to reach its kernel wait before the ring is
/// closed or reset.
///
/// The peer cannot *prove* that it is asleep: the only way to see a waiter is
/// to wake it, and a wake would let it re-check the counters on its own -
/// which is exactly the thing under test. A peer that is still on its way
/// would report the new state from its first counter check, so the wait below
/// is a *false pass* at worst, never a false failure.
const SETTLE: Duration = Duration::from_millis(50);

/// A SplitMix64 generator: this file's own seeded source of randomness.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..bound`; `bound` must not be zero.
    fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "a bound of zero has no value");
        self.next_u64() % bound
    }

    /// A value in `lo..=hi`.
    fn between(&mut self, lo: u64, hi: u64) -> u64 {
        assert!(lo <= hi, "an empty range has no value");
        lo + self.below(hi - lo + 1)
    }
}

/// The seed of one round; every scenario is reproducible from it.
fn round_seed(round: usize) -> u64 {
    SEED.wrapping_add((round as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// A cheap mixing of a frame index, kept inside `0..=0xFFFF` so it is exact
/// in `f32`.
fn mix(index: u64) -> u64 {
    let mut z = index.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) & 0xFFFF
}

/// The payload of frame `index`: channel 0 is the index itself, channel 1 a
/// mixing of it.
///
/// Both values are exact in `f32`, so a torn frame (the channels of two
/// different frames) and a misordered frame are both detectable.
fn frame(index: u64) -> [f32; 2] {
    [index as f32, mix(index) as f32]
}

/// The frames `start..start+count` as one interleaved buffer.
fn frame_block(start: u64, count: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(count * 2);
    for index in start..start + count as u64 {
        out.extend_from_slice(&frame(index));
    }
    out
}

/// The frames `0..count` as one interleaved buffer.
fn frame_stream(count: u64) -> Vec<f32> {
    frame_block(0, count as usize)
}

/// The index a delivered frame claims, checked for internal consistency.
///
/// A torn frame does not survive this: the two channels come from different
/// frames, so they do not fit the relation `channel 1 == mix(channel 0)`.
fn frame_index(payload: &[f32], seed: u64) -> u64 {
    assert_eq!(payload.len(), 2, "seed {seed}: not a whole frame");
    let raw = payload[0];
    assert!(
        (0.0..16_777_216.0).contains(&raw) && raw.fract() == 0.0,
        "seed {seed}: a torn frame produced the channel {raw}"
    );
    let index = raw as u64;
    assert_eq!(
        payload[1],
        mix(index) as f32,
        "seed {seed}: a torn frame at index {index}"
    );
    index
}

/// A region big enough for a ring of this layout.
fn region_for(frame_size: u32, capacity: u32) -> AnonymousRegion {
    let size = RING_DATA_OFFSET + capacity as usize * frame_size as usize;
    AnonymousRegion::new(size).unwrap()
}

/// A stereo PCM ring without a wake.
fn pcm_ring(capacity: u32) -> (RingProducer<AnonymousRegion>, RingConsumer<AnonymousRegion>) {
    let frame_size = 8u32;
    let region = region_for(frame_size, capacity);
    let producer =
        RingProducer::create(region.clone(), RingKind::Pcm, frame_size, capacity).unwrap();
    let consumer = RingConsumer::attach(region, producer.expect()).unwrap();
    (producer, consumer)
}

/// A stereo PCM ring with a wake on both ends.
///
/// The wake is returned as well: the test keeps it alive, because the kernel
/// objects must outlive the wait that uses them.
fn pcm_ring_with_wake(
    capacity: u32,
) -> (
    RingProducer<AnonymousRegion>,
    RingConsumer<AnonymousRegion>,
    RingWake,
) {
    let frame_size = 8u32;
    let region = region_for(frame_size, capacity);
    let producer =
        RingProducer::create(region.clone(), RingKind::Pcm, frame_size, capacity).unwrap();
    let wake = RingWake::create(&unique_wake_name("pcm")).unwrap();
    let producer = producer.with_wake(wake.clone());
    let consumer = RingConsumer::attach(region, producer.expect())
        .unwrap()
        .with_wake(wake.clone());
    (producer, consumer, wake)
}

/// A wake name that no other test uses.
///
/// The events are named kernel objects: `CreateEventW` on an existing name
/// returns a handle to the *existing* object, so two tests that shared a name
/// would share their latches.
fn unique_wake_name(tag: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let serial = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("rustamp-ipc-props-{}-{tag}-{serial}", std::process::id())
}

/// The counter invariants that must hold after every single operation
/// (RFC 0003 section 7).
fn assert_counters(
    producer: &RingProducer<AnonymousRegion>,
    consumer: &RingConsumer<AnonymousRegion>,
    capacity: u32,
    what: &str,
) {
    let written = producer.frames_written();
    let dropped = producer.frames_dropped();
    let read = consumer.frames_read();
    let published = consumer.frames_published();

    assert!(
        read <= written,
        "{what}: read_seq {read} is above write_seq {written}"
    );
    assert_eq!(
        written, published,
        "{what}: the two ends disagree about write_seq"
    );
    let available = producer.frames_available();
    assert!(
        available <= u64::from(capacity),
        "{what}: the occupancy {available} is above the capacity {capacity}"
    );
    assert_eq!(
        available,
        consumer.frames_available(),
        "{what}: the two ends disagree about the occupancy"
    );
    assert_eq!(
        producer.free_frames() + available,
        u64::from(capacity),
        "{what}: free space and occupancy do not add up to the capacity"
    );
    assert_eq!(
        read + available,
        written,
        "{what}: the frames read and the frames in the ring do not add up to the frames written"
    );
    // The counters are frame counts that only grow; nothing may wrap.
    assert!(
        written.checked_add(dropped).is_some(),
        "{what}: the counters overflow"
    );
}

/// **No torn, duplicated or lost frame.** Over random read and write chunk
/// sizes the frames the consumer takes are exactly the frames the producer
/// accepted, in order, byte for byte.
///
/// A chunk is often larger than the ring, so the producer has to drop the tail
/// of the block - and the drop is counted, never silent. The accounting adds
/// up: every offered frame is accepted, dropped and counted, or still in the
/// ring.
#[test]
fn random_chunks_deliver_every_accepted_frame_exactly_once() {
    for round in 0..ROUNDS {
        let seed = round_seed(round);
        let mut rng = Rng::new(seed);
        run_random_stream(&mut rng, seed);
    }
}

/// One random scenario of the round-trip property.
fn run_random_stream(rng: &mut Rng, seed: u64) {
    let capacity = rng.between(1, 8) as u32;
    let (mut producer, mut consumer) = pcm_ring(capacity);

    // The indices of the frames the producer accepted, in order.
    let mut accepted: Vec<u64> = Vec::new();
    // The indices the consumer read, taken from the delivered payloads.
    let mut received: Vec<u64> = Vec::new();
    let mut next_offer = 0u64;
    let mut offered = 0u64;

    for step in 0..STEPS {
        // Write a random chunk of frames.
        let chunk = rng.between(1, 3 * u64::from(capacity)) as usize;
        if next_offer < TOTAL_FRAMES {
            let n = chunk.min((TOTAL_FRAMES - next_offer) as usize);
            let block = frame_block(next_offer, n);
            let written = producer.write_from(&block).unwrap();
            assert!(
                written <= n,
                "seed {seed}: the producer accepted more frames than it was offered"
            );
            for offset in 0..written as u64 {
                accepted.push(next_offer + offset);
            }
            next_offer += n as u64;
            offered += n as u64;
        }

        // Read a random chunk of frames.
        let want = rng.between(1, 3 * u64::from(capacity)) as usize;
        let mut out = vec![0.0f32; want * 2];
        let read = consumer.read_into(&mut out).unwrap();
        assert!(
            read <= want,
            "seed {seed}: the consumer read more frames than it asked for"
        );
        for pair in out[..read * 2].as_chunks::<2>().0 {
            received.push(frame_index(pair, seed));
        }

        assert_counters(
            &producer,
            &consumer,
            capacity,
            &format!("seed {seed} step {step}"),
        );
    }

    // Drain the ring.
    loop {
        let mut out = vec![0.0f32; 64];
        let read = consumer.read_into(&mut out).unwrap();
        if read == 0 {
            break;
        }
        for pair in out[..read * 2].as_chunks::<2>().0 {
            received.push(frame_index(pair, seed));
        }
    }

    // The accounting: nothing is lost.
    assert_eq!(
        producer.frames_written(),
        accepted.len() as u64,
        "seed {seed}: the producer's counter does not match the frames it accepted"
    );
    assert_eq!(
        offered,
        producer.frames_written() + producer.frames_dropped(),
        "seed {seed}: the offered frames are neither accepted nor dropped"
    );
    assert_eq!(
        received.len() as u64 + producer.frames_available(),
        producer.frames_written(),
        "seed {seed}: the frames read and the frames in the ring do not add up to the frames written"
    );

    // The stream: the consumer saw the accepted frames, once each, in order.
    assert!(
        received.len() <= accepted.len(),
        "seed {seed}: the consumer read more frames than the producer accepted"
    );
    assert_eq!(
        received,
        accepted[..received.len()],
        "seed {seed}: the consumer saw a frame twice, out of order, or one the producer did not accept"
    );
}

/// **Sum of read == sum of written.** A producer that never offers more than
/// the free space drops nothing, so every frame it offers arrives, in order,
/// byte for byte.
#[test]
fn a_lossless_producer_delivers_the_whole_stream() {
    for round in 0..ROUNDS {
        let seed = round_seed(round);
        let mut rng = Rng::new(seed);
        run_lossless_stream(&mut rng, seed);
    }
}

/// One random scenario of the lossless property.
fn run_lossless_stream(rng: &mut Rng, seed: u64) {
    let capacity = rng.between(1, 8) as u32;
    let (mut producer, mut consumer) = pcm_ring(capacity);

    let mut next = 0u64;
    let mut read_back: Vec<f32> = Vec::new();

    while next < TOTAL_FRAMES {
        // The producer never offers more than the ring can take, so nothing is
        // dropped.
        let free = producer.free_frames() as usize;
        if free == 0 {
            let mut out = vec![0.0f32; 2 * capacity as usize];
            let read = consumer.read_into(&mut out).unwrap();
            assert!(
                read > 0,
                "seed {seed}: a full ring did not yield to the reader"
            );
            read_back.extend_from_slice(&out[..read * 2]);
            continue;
        }
        let chunk = rng.between(1, free as u64).min(TOTAL_FRAMES - next) as usize;
        let block = frame_block(next, chunk);
        let written = producer.write_from(&block).unwrap();
        assert_eq!(
            written, chunk,
            "seed {seed}: a write that fits dropped frames"
        );
        next += chunk as u64;

        // Read a random amount back.
        let want = rng.between(1, 2 * u64::from(capacity)) as usize;
        let mut out = vec![0.0f32; want * 2];
        let read = consumer.read_into(&mut out).unwrap();
        read_back.extend_from_slice(&out[..read * 2]);
    }

    // Drain the rest.
    loop {
        let mut out = vec![0.0f32; 64];
        let read = consumer.read_into(&mut out).unwrap();
        if read == 0 {
            break;
        }
        read_back.extend_from_slice(&out[..read * 2]);
    }

    assert_eq!(
        producer.frames_dropped(),
        0,
        "seed {seed}: a lossless run dropped"
    );
    assert_eq!(
        producer.frames_written(),
        TOTAL_FRAMES,
        "seed {seed}: the producer wrote a different number of frames than it offered"
    );
    assert_eq!(
        consumer.frames_read(),
        TOTAL_FRAMES,
        "seed {seed}: the consumer read a different number of frames than the producer wrote"
    );
    assert_eq!(
        read_back,
        frame_stream(TOTAL_FRAMES),
        "seed {seed}: the delivered stream does not match the offered one"
    );
}

/// **The same property with a real peer thread.** The producer and the
/// consumer run at the same time, with random chunk sizes on both sides, and
/// every frame arrives exactly once, in order, with no drop.
///
/// The blocking calls never drop: the producer waits for space and the
/// consumer waits for data, so the accounting is exact - what the producer
/// wrote is what the consumer read.
#[test]
fn two_threads_with_random_chunks_deliver_every_frame_exactly_once() {
    const FRAMES: u64 = 2_000;
    for round in 0..CONCURRENT_ROUNDS {
        let seed = round_seed(round);
        let mut setup = Rng::new(seed);
        let capacity = setup.between(1, 8) as u32;
        let (producer, consumer, _wake) = pcm_ring_with_wake(capacity);
        let producer_seed = seed ^ 0xA5A5_A5A5_A5A5_A5A5;
        let consumer_seed = seed ^ 0x5A5A_5A5A_5A5A_5A5A;

        let (written, dropped, received) = std::thread::scope(|scope| {
            let p = scope.spawn(move || {
                let mut producer = producer;
                let mut rng = Rng::new(producer_seed);
                let mut next = 0u64;
                while next < FRAMES {
                    // The block never reaches past the end of the stream.
                    let chunk = rng.between(1, 3 * u64::from(capacity)).min(FRAMES - next) as usize;
                    let block = frame_block(next, chunk);
                    // A blocking write never drops: it writes what fits and
                    // returns the count, so the rest of the block is offered
                    // again by the next round.
                    let written = producer.write_from_blocking(&block, WAIT).unwrap();
                    assert!(written > 0, "seed {producer_seed}: the producer stalled");
                    next += written as u64;
                }
                (producer.frames_written(), producer.frames_dropped())
            });
            let c = scope.spawn(move || {
                let mut consumer = consumer;
                let mut rng = Rng::new(consumer_seed);
                let mut out = vec![0.0f32; 2 * 3 * capacity as usize];
                let mut received = 0u64;
                while received < FRAMES {
                    let want = rng.between(1, 3 * u64::from(capacity)) as usize;
                    let read = consumer
                        .read_into_blocking(&mut out[..want * 2], WAIT)
                        .unwrap();
                    assert!(read > 0, "seed {consumer_seed}: the consumer stalled");
                    for pair in out[..read * 2].as_chunks::<2>().0 {
                        let index = frame_index(pair, consumer_seed);
                        assert_eq!(
                            index, received,
                            "seed {consumer_seed}: the frame arrived out of order"
                        );
                        received += 1;
                    }
                }
                received
            });
            let (written, dropped) = p.join().unwrap();
            let received = c.join().unwrap();
            (written, dropped, received)
        });

        assert_eq!(written, FRAMES, "seed {seed}: a frame was not written");
        assert_eq!(dropped, 0, "seed {seed}: a blocking write dropped a frame");
        assert_eq!(received, FRAMES, "seed {seed}: a frame was not received");
    }
}

/// **A closed ring hands out its frames before it reports the close** (the
/// drain-then-EOS order of RFC 0007 section 6.5).
///
/// The two consumer-side waits promise different things, and the difference
/// is deliberate:
///
/// * `read_into_blocking` copies, so it delivers the frames that are still in
///   the ring first and reports the close on the next call.
/// * `wait_readable` only answers "is there something to read, or is the ring
///   closed". The close is the answer that matters there (its own
///   documentation says so), and it does not consume the frames: they stay in
///   the ring for the reader.
///
/// A caller that mixes the two therefore has to read the pending frames
/// before it treats the close as the end of the stream.
#[test]
fn a_closed_ring_hands_out_its_frames_before_the_close() {
    let (mut producer, mut consumer, _wake) = pcm_ring_with_wake(4);
    assert_eq!(producer.write_from(&[1.0, -1.0, 2.0, -2.0]).unwrap(), 2);
    producer.close();

    // The blocking read delivers the pending frames first ...
    let mut out = [0.0f32; 4];
    assert_eq!(consumer.read_into_blocking(&mut out, WAIT).unwrap(), 2);
    assert_eq!(out, [1.0, -1.0, 2.0, -2.0]);
    // ... and only then reports the close.
    assert!(matches!(
        consumer.read_into_blocking(&mut out, WAIT),
        Err(RingError::Closed)
    ));

    // The wait reports the close right away, and the frames are still there.
    let (mut producer, mut consumer, _wake) = pcm_ring_with_wake(4);
    assert_eq!(producer.write_from(&[1.0, -1.0, 2.0, -2.0]).unwrap(), 2);
    producer.close();
    assert!(matches!(
        consumer.wait_readable(WAIT),
        Err(RingError::Closed)
    ));
    assert_eq!(consumer.read_into(&mut out).unwrap(), 2);
    assert_eq!(out, [1.0, -1.0, 2.0, -2.0]);
    assert_eq!(consumer.frames_read(), 2);
}

/// **The counters only grow and stay consistent.** Every operation of a random
/// sequence leaves the counters in the state the ring's contract describes,
/// and no counter ever moves backwards.
#[test]
fn the_counters_only_grow_and_stay_consistent() {
    for round in 0..ROUNDS {
        let seed = round_seed(round);
        let mut rng = Rng::new(seed);
        let capacity = rng.between(1, 8) as u32;
        let (mut producer, mut consumer) = pcm_ring(capacity);

        let mut last_written = 0u64;
        let mut last_dropped = 0u64;
        let mut last_read = 0u64;

        for step in 0..STEPS {
            // Write a random chunk, often more than the ring can take.
            let chunk = rng.between(1, 4 * u64::from(capacity)) as usize;
            let block = frame_stream(chunk as u64);
            let written = producer.write_from(&block).unwrap();
            assert!(
                written <= chunk,
                "seed {seed}: the producer accepted too much"
            );

            // Read a random chunk.
            let want = rng.between(1, 4 * u64::from(capacity)) as usize;
            let mut out = vec![0.0f32; want * 2];
            let read = consumer.read_into(&mut out).unwrap();
            assert!(read <= want, "seed {seed}: the consumer read too much");

            assert_counters(
                &producer,
                &consumer,
                capacity,
                &format!("seed {seed} step {step}"),
            );

            assert!(
                producer.frames_written() >= last_written,
                "seed {seed}: write_seq went backwards"
            );
            assert!(
                producer.frames_dropped() >= last_dropped,
                "seed {seed}: dropped_seq went backwards"
            );
            assert!(
                consumer.frames_read() >= last_read,
                "seed {seed}: read_seq went backwards"
            );
            last_written = producer.frames_written();
            last_dropped = producer.frames_dropped();
            last_read = consumer.frames_read();
        }
    }
}

/// **`close()` is idempotent.** Closing twice changes nothing the second
/// time: the ring stays closed, the frames that are still in it are still
/// delivered, and the peer sees the same state.
#[test]
fn close_is_idempotent() {
    let (mut producer, mut consumer) = pcm_ring(4);
    assert_eq!(producer.write_from(&[1.0, -1.0]).unwrap(), 1);

    producer.close();
    producer.close();
    assert!(producer.is_closed(), "the producer does not see the close");
    assert!(consumer.is_closed(), "the peer does not see the close");

    // A close is not a reset: the published frame is still delivered.
    let mut out = [0.0f32; 2];
    assert_eq!(
        consumer.read_into(&mut out).unwrap(),
        1,
        "the close lost the published frame"
    );
    assert_eq!(out, [1.0, -1.0], "the close changed the published frame");
    assert_eq!(
        consumer.read_into(&mut out).unwrap(),
        0,
        "the closed ring handed out a second frame"
    );

    // A closed ring refuses new data.
    assert!(
        matches!(producer.write_from(&[2.0, -2.0]), Err(RingError::Closed)),
        "a closed ring accepted a write"
    );

    // The consumer's close is idempotent too, and it closes the ring for the
    // producer as well.
    let (mut producer, mut consumer) = pcm_ring(4);
    consumer.close();
    consumer.close();
    assert!(consumer.is_closed(), "the consumer does not see the close");
    assert!(producer.is_closed(), "the peer does not see the close");
    assert!(
        matches!(producer.write_from(&[1.0, -1.0]), Err(RingError::Closed)),
        "a closed ring accepted a write"
    );
}

/// **A blocked consumer wakes with `Closed`.** A consumer that waits for data
/// on an empty ring is woken by the producer's `close()` and reports the
/// close instead of waiting for its timeout (RFC 0007 section 6.5).
///
/// The wake carries no data: the peer reports the close and has read
/// nothing.
#[test]
fn a_blocked_consumer_wakes_with_closed() {
    let (mut producer, consumer, _wake) = pcm_ring_with_wake(4);
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut consumer = consumer;
        let mut out = [0.0f32; 8];
        let _ = started_tx.send(());
        let result = consumer.read_into_blocking(&mut out, WAIT);
        let _ = result_tx.send((result, consumer.frames_read()));
    });
    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("the reader never started");
    std::thread::sleep(SETTLE);

    producer.close();

    let (result, read) = result_rx
        .recv_timeout(RESULT_WAIT)
        .expect("the blocked reader was not woken by the close");
    assert!(
        matches!(result, Err(RingError::Closed)),
        "a close must end the wait with Closed, got {result:?}"
    );
    assert_eq!(read, 0, "the wake invented a frame");
    reader.join().unwrap();
}

/// **A blocked producer wakes with `Closed`.** A producer that waits for
/// space on a full ring is woken by the consumer's `close()` and reports the
/// close instead of waiting for its timeout (RFC 0007 section 6.5).
#[test]
fn a_blocked_producer_wakes_with_closed() {
    let (mut producer, mut consumer, _wake) = pcm_ring_with_wake(1);
    // Fill the ring: the writer below has to wait for space.
    assert_eq!(producer.write_from(&[1.0, -1.0]).unwrap(), 1);
    assert_eq!(producer.free_frames(), 0, "the ring is not full");

    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let writer = std::thread::spawn(move || {
        let mut producer = producer;
        let _ = started_tx.send(());
        let result = producer.write_from_blocking(&[2.0, -2.0], WAIT);
        let _ = result_tx.send(result);
    });
    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("the writer never started");
    std::thread::sleep(SETTLE);

    consumer.close();

    let result = result_rx
        .recv_timeout(RESULT_WAIT)
        .expect("the blocked writer was not woken by the close");
    assert!(
        matches!(result, Err(RingError::Closed)),
        "a close must end the wait with Closed, got {result:?}"
    );
    writer.join().unwrap();
}

/// **A generation reset invalidates the old peer and leaves the new one
/// usable.** The creator re-creates the section, so the generation moves on:
/// the old peer is stale, and the new generation is a normal ring again
/// (RFC 0003 section 8).
///
/// The paths that check the header report the stale generation: `validate()`
/// and the waiting and blocking calls. The non-blocking hot path does **not**
/// re-read the generation - RFC 0003 section 8 says so explicitly ("the
/// per-frame hot path does not re-read it, so a caller that wants the
/// guarantee calls `validate()` on its own cadence") - so this property
/// covers the paths that carry the guarantee.
#[test]
fn a_reset_invalidates_the_old_peer_and_leaves_the_new_one_usable() {
    let capacity = 8u32;
    let region = region_for(8, capacity);
    let mut producer = RingProducer::create(region.clone(), RingKind::Pcm, 8, capacity).unwrap();
    let expect = producer.expect();
    let consumer = RingConsumer::attach(region.clone(), expect).unwrap();
    assert_eq!(producer.write_from(&[1.0, -1.0]).unwrap(), 1);
    // The creator replaces the ring: the generation moves on and the counters
    // restart.
    let mut second = RingProducer::create(region.clone(), RingKind::Pcm, 8, capacity).unwrap();
    assert_eq!(
        second.generation(),
        expect.generation + 1,
        "the generation did not move on"
    );
    assert_eq!(second.frames_written(), 0, "the counters did not restart");

    // The old peer is stale.
    assert!(
        matches!(consumer.validate(), Err(RingError::Stale { .. })),
        "the old peer did not notice the reset"
    );
    assert!(
        matches!(
            RingConsumer::attach(region.clone(), expect),
            Err(RingError::Stale { .. })
        ),
        "the old expectation was accepted"
    );

    // The new generation is a normal ring again.
    let mut new_consumer = RingConsumer::attach(region.clone(), second.expect()).unwrap();
    assert_eq!(second.write_from(&[2.0, -2.0]).unwrap(), 1);
    let mut out = [0.0f32; 2];
    assert_eq!(
        new_consumer.read_into(&mut out).unwrap(),
        1,
        "the new generation delivered no frame"
    );
    assert_eq!(
        out,
        [2.0, -2.0],
        "the new generation delivered the wrong frame"
    );
    assert_eq!(new_consumer.frames_read(), 1);
    assert_eq!(second.frames_dropped(), 0);
    assert!(new_consumer.validate().is_ok());
}

/// **A generation reset wakes the peer that waits on the old generation.** The
/// wait ends with `Stale`, not with a timeout (RFC 0007 section 5.3).
///
/// This is the property that the missing barrier between the counter store
/// and the signal broke: without it, the woken peer could re-read the *old*
/// generation, find its condition "unmet" and go back to sleep, and the
/// wakeup was lost - the peer then waited for its whole timeout.
#[test]
fn a_reset_wakes_a_blocked_peer_with_stale() {
    let capacity = 4u32;
    let region = region_for(8, capacity);
    let producer = RingProducer::create(region.clone(), RingKind::Pcm, 8, capacity).unwrap();
    let wake = RingWake::create(&unique_wake_name("reset")).unwrap();
    let expect = producer.expect();
    let consumer = RingConsumer::attach(region.clone(), expect)
        .unwrap()
        .with_wake(wake.clone());

    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut consumer = consumer;
        let mut out = [0.0f32; 8];
        let _ = started_tx.send(());
        let result = consumer.read_into_blocking(&mut out, WAIT);
        let _ = result_tx.send(result);
    });
    started_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("the reader never started");
    std::thread::sleep(SETTLE);

    // The creator replaces the ring and reuses the wake: the generation moves
    // on and the reset wakes the peer.
    let _second = RingProducer::create(region, RingKind::Pcm, 8, capacity)
        .unwrap()
        .with_wake(wake);

    let result = result_rx
        .recv_timeout(RESULT_WAIT)
        .expect("the blocked peer was not woken by the reset");
    assert!(
        matches!(result, Err(RingError::Stale { .. })),
        "a reset must end the wait with the stale generation, got {result:?}"
    );
    reader.join().unwrap();
}

/// **No operation panics.** A random sequence of hostile operations - partial
/// frames, oversized writes, empty buffers, a close in the middle - never
/// panics and never leaves the counters inconsistent.
#[test]
fn random_hostile_sequences_never_panic() {
    for round in 0..ROUNDS {
        let seed = round_seed(round);
        let mut rng = Rng::new(seed);
        run_hostile_sequence(&mut rng, seed);
    }
}

/// The operations a hostile sequence may apply.
#[derive(Debug, Clone, Copy)]
enum Hostile {
    /// A buffer that is not a whole number of stereo frames.
    MisalignedWrite,
    /// A buffer that is not a whole number of stereo frames.
    MisalignedRead,
    /// A write that is much larger than the ring.
    HugeWrite,
    /// A write of no frames at all.
    EmptyWrite,
    /// A read of no frames at all.
    EmptyRead,
    /// A read of more frames than the ring can hold.
    HugeRead,
    /// Close the ring.
    Close,
}

/// One random hostile scenario.
fn run_hostile_sequence(rng: &mut Rng, seed: u64) {
    let capacity = rng.between(1, 4) as u32;
    let (mut producer, mut consumer) = pcm_ring(capacity);
    let mut closed = false;

    for step in 0..STEPS {
        let op = match rng.below(7) {
            0 => Hostile::MisalignedWrite,
            1 => Hostile::MisalignedRead,
            2 => Hostile::HugeWrite,
            3 => Hostile::EmptyWrite,
            4 => Hostile::EmptyRead,
            5 => Hostile::HugeRead,
            _ => Hostile::Close,
        };
        let what = format!("seed {seed} step {step} ({op:?})");

        match op {
            Hostile::MisalignedWrite => {
                // An odd number of samples is not a whole number of frames.
                let odd = vec![0.0f32; rng.below(4) as usize * 2 + 1];
                let result = producer.write_from(&odd);
                if closed {
                    // The closed state is checked before the alignment, so a
                    // closed ring refuses the write either way.
                    assert!(
                        matches!(result, Err(RingError::Closed)),
                        "{what}: a closed ring accepted a write"
                    );
                } else {
                    assert!(
                        matches!(result, Err(RingError::NotFrameAligned { .. })),
                        "{what}: a partial frame was not reported"
                    );
                }
            }
            Hostile::MisalignedRead => {
                let mut odd = vec![0.0f32; rng.below(4) as usize * 2 + 1];
                assert!(
                    matches!(
                        consumer.read_into(&mut odd),
                        Err(RingError::NotFrameAligned { .. })
                    ),
                    "{what}: a partial frame was not reported"
                );
            }
            Hostile::HugeWrite => {
                let frames = rng.between(1, 8 * u64::from(capacity)) as usize;
                let block = frame_stream(frames as u64);
                let written = if closed {
                    assert!(
                        matches!(producer.write_from(&block), Err(RingError::Closed)),
                        "{what}: a closed ring accepted a write"
                    );
                    0
                } else {
                    let written = producer.write_from(&block).unwrap();
                    assert!(written <= frames, "{what}: the producer accepted too much");
                    written
                };
                assert!(
                    written as u64 <= u64::from(capacity),
                    "{what}: the producer accepted more frames than the ring holds"
                );
            }
            Hostile::EmptyWrite => {
                let result = producer.write_from(&[]);
                if closed {
                    assert!(
                        matches!(result, Err(RingError::Closed)),
                        "{what}: a closed ring accepted a write"
                    );
                } else {
                    assert_eq!(result.unwrap(), 0, "{what}: an empty write was not a no-op");
                }
            }
            Hostile::EmptyRead => {
                assert_eq!(
                    consumer.read_into(&mut []).unwrap(),
                    0,
                    "{what}: an empty read was not a no-op"
                );
            }
            Hostile::HugeRead => {
                let frames = rng.between(1, 8 * u64::from(capacity)) as usize;
                let mut out = vec![0.0f32; frames * 2];
                let read = consumer.read_into(&mut out).unwrap();
                assert!(read <= frames, "{what}: the consumer read too much");
            }
            Hostile::Close => {
                producer.close();
                producer.close();
                closed = true;
            }
        }

        assert!(
            producer.validate().is_ok(),
            "{what}: the header is not valid"
        );
        assert_counters(&producer, &consumer, capacity, &what);
    }
}

/// **The vis ring numbers its slots by the offered index.** The slot's `seq`
/// is the number of frames ever offered, so the gaps in the sequence are
/// exactly the dropped frames (RFC 0003 section 7).
#[test]
fn a_vis_ring_numbers_its_slots_by_the_offered_index() {
    for round in 0..ROUNDS {
        let seed = round_seed(round);
        let mut rng = Rng::new(seed);
        let capacity = rng.between(1, 6) as u32;
        let region = region_for(VIS_SLOT_SIZE as u32, capacity);
        let mut producer = RingProducer::create(
            region.clone(),
            RingKind::Vis,
            VIS_SLOT_SIZE as u32,
            capacity,
        )
        .unwrap();
        let mut consumer = RingConsumer::attach(region, producer.expect()).unwrap();

        // The offered index of every accepted slot.
        let mut expected: Vec<u64> = Vec::new();
        let mut accepted = 0u64;
        let mut dropped = 0u64;
        let mut read = 0usize;

        for _ in 0..STEPS {
            // Write a random number of slots.
            for _ in 0..rng.between(1, 3) {
                let slot = VisSlot::zeroed();
                if producer.write_slot(&slot).unwrap() {
                    expected.push(accepted + dropped);
                    accepted += 1;
                } else {
                    dropped += 1;
                }
            }

            // Read a random number of slots.
            let mut out = VisSlot::zeroed();
            for _ in 0..rng.between(1, 3) {
                if !consumer.read_slot_into(&mut out).unwrap() {
                    break;
                }
                assert!(
                    read < expected.len(),
                    "seed {seed}: a slot arrived that was never offered"
                );
                assert_eq!(
                    out.seq, expected[read],
                    "seed {seed}: the slot carries the wrong offered index"
                );
                read += 1;
            }
        }

        assert_eq!(producer.frames_written(), accepted, "seed {seed}");
        assert_eq!(producer.frames_dropped(), dropped, "seed {seed}");
        assert_eq!(consumer.frames_read(), read as u64, "seed {seed}");
        assert_eq!(
            accepted + dropped,
            producer.frames_written() + producer.frames_dropped(),
            "seed {seed}: the offered slots are neither accepted nor dropped"
        );
    }
}
