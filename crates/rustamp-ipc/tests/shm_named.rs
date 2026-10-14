//! The real cross-process path: a Windows named file mapping (RFC 0003 section 3).
//!
//! This test uses two independent views of the same section and re-opens it by
//! name after the creator closed its own view, which is what the two-process
//! case does. It runs on `x86_64-pc-windows-msvc` and on
//! `i686-pc-windows-msvc`.

#![cfg(windows)]

use rustamp_ipc::shm::{
    NamedRegion, RING_DATA_OFFSET, RingConsumer, RingError, RingKind, RingProducer, ring_name,
};

const CAPACITY: u32 = 128;
const FRAME_SIZE: u32 = 8; // stereo f32

fn size() -> usize {
    RING_DATA_OFFSET + CAPACITY as usize * FRAME_SIZE as usize
}

fn unique_name(tag: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    ring_name(RingKind::Pcm, nanos as u64) + "-" + tag
}

/// Frames are numbered so the reader can check order and exactness.
fn frame(index: u32) -> [f32; 2] {
    [index as f32, -(index as f32)]
}

#[test]
fn named_mapping_round_trip_through_reopening_by_name() {
    let name = unique_name("roundtrip");
    assert!(!name.contains('\\'), "no namespace prefix");

    // The creator side.
    let creator_view = NamedRegion::create(&name, size()).unwrap();
    let mut producer =
        RingProducer::create(creator_view, RingKind::Pcm, FRAME_SIZE, CAPACITY).unwrap();
    let expect = producer.expect();
    assert_ne!(producer.generation(), 0);

    let mut samples = Vec::new();
    for index in 0..100u32 {
        samples.extend_from_slice(&frame(index));
    }
    assert_eq!(producer.write_from(&samples).unwrap(), 100);

    // A second, independent view of the same section reads the first half.
    let second_view = NamedRegion::open(&name, size()).unwrap();
    let mut first_consumer = RingConsumer::attach(second_view, expect).unwrap();
    let mut out = [0.0f32; 100];
    assert_eq!(first_consumer.read_into(&mut out).unwrap(), 50);
    assert_eq!(&out[..100], &samples[..100]);
    assert_eq!(first_consumer.frames_read(), 50);
    assert_eq!(first_consumer.frames_published(), 100);
    drop(first_consumer);

    // A keeper handle stands in for the peer that is still alive while the
    // creator closes its own view. Without it Windows would drop the section.
    let keeper = NamedRegion::open(&name, size()).unwrap();

    // Close the creator's view entirely, then re-open by name.
    drop(producer);

    let reopened = NamedRegion::open(&name, size()).unwrap();
    let mut second_consumer = RingConsumer::attach(reopened, expect).unwrap();
    assert_eq!(second_consumer.frames_read(), 50, "read_seq survived");
    let mut tail = [0.0f32; 100];
    assert_eq!(second_consumer.read_into(&mut tail).unwrap(), 50);
    assert_eq!(&tail[..100], &samples[100..]);
    assert_eq!(second_consumer.read_into(&mut tail).unwrap(), 0);
    assert_eq!(second_consumer.frames_read(), 100);
    assert_eq!(second_consumer.frames_dropped(), 0);

    drop(second_consumer);
    drop(keeper);

    // The section is gone once every handle is closed.
    assert!(matches!(
        NamedRegion::open(&name, size()),
        Err(RingError::MappingUnavailable)
    ));
}

#[test]
fn reopening_the_same_name_makes_the_old_ring_stale() {
    let name = unique_name("stale");

    let first_view = NamedRegion::create(&name, size()).unwrap();
    let first_producer =
        RingProducer::create(first_view, RingKind::Pcm, FRAME_SIZE, CAPACITY).unwrap();
    let first_expect = first_producer.expect();

    let consumer_view = NamedRegion::open(&name, size()).unwrap();
    let mut consumer = RingConsumer::attach(consumer_view, first_expect).unwrap();
    let mut out = [0.0f32; 4];
    assert_eq!(consumer.read_into(&mut out).unwrap(), 0);
    assert!(consumer.validate().is_ok());

    // A new run attaches to the same section by name and bumps the generation.
    let second_view = NamedRegion::open(&name, size()).unwrap();
    let mut second_producer =
        RingProducer::create(second_view, RingKind::Pcm, FRAME_SIZE, CAPACITY).unwrap();
    assert_eq!(second_producer.generation(), first_expect.generation + 1);
    assert_eq!(second_producer.frames_written(), 0, "counters reset");

    // The old consumer must not mistake the new run's data for its own.
    assert!(matches!(consumer.validate(), Err(RingError::Stale { .. })));
    assert_eq!(second_producer.write_from(&frame(7)).unwrap(), 1);
    // The stale consumer refuses to attach again with the old expectation.
    let third_view = NamedRegion::open(&name, size()).unwrap();
    assert!(matches!(
        RingConsumer::attach(third_view, first_expect),
        Err(RingError::Stale { .. })
    ));
    assert!(consumer.validate().is_err());
    assert_eq!(second_producer.frames_written(), 1);
}

#[test]
fn a_wrong_size_view_is_rejected_not_read() {
    let name = unique_name("size");

    let creator_view = NamedRegion::create(&name, size()).unwrap();
    let producer = RingProducer::create(creator_view, RingKind::Pcm, FRAME_SIZE, CAPACITY).unwrap();
    let expect = producer.expect();

    // A view that claims a different (smaller) size cannot satisfy the layout.
    let truncated = NamedRegion::open(&name, RING_DATA_OFFSET).unwrap();
    assert!(matches!(
        RingConsumer::attach(truncated, expect),
        Err(RingError::RegionTooSmall { .. })
    ));

    // A view of the right size but a wrong expectation is refused with a
    // specific reason instead of being read.
    let wrong = NamedRegion::open(&name, size()).unwrap();
    assert!(matches!(
        RingConsumer::attach(
            wrong,
            rustamp_ipc::shm::RingExpect::new(RingKind::Pcm, 4, CAPACITY, expect.generation)
        ),
        Err(RingError::FrameSizeMismatch { .. })
    ));
    assert_eq!(
        producer.expect().generation.cmp(&expect.generation),
        std::cmp::Ordering::Equal
    );
}
