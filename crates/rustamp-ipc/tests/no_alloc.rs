//! The ring read/write path must not allocate (task card G1, RFC 0003 section 7).
//!
//! A counting global allocator wraps the system allocator and the test asserts
//! that a long run of `write_from`/`read_into` (and the vis equivalents) moves
//! the counter not at all. This file deliberately contains a single test so that
//! no other thread of the test harness allocates while the measurement runs.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use rustamp_ipc::shm::{
    AnonymousRegion, RING_DATA_OFFSET, RingConsumer, RingKind, RingProducer, VIS_SLOT_SIZE, VisSlot,
};

/// Counts every allocation request that goes through the global allocator.
struct CountingAllocator;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards to `System` with the same arguments, so the
// allocation contract is exactly the system allocator's. The counter is a
// separate atomic and does not change the returned pointers.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded unchanged from this method's contract.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded unchanged from this method's contract.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded unchanged from this method's contract.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded unchanged from this method's contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn ring_read_write_path_allocates_nothing() {
    const CHANNELS: u32 = 2;
    const FRAME_SIZE: u32 = CHANNELS * 4;
    const CAPACITY: u32 = 1024;
    const BLOCK_FRAMES: usize = 256;
    const ITERATIONS: usize = 100_000;

    // Everything that may allocate happens before the measurement: the region,
    // the ring, the caller buffers and the vis slots.
    let region =
        AnonymousRegion::new(RING_DATA_OFFSET + CAPACITY as usize * FRAME_SIZE as usize).unwrap();
    let mut producer =
        RingProducer::create(region.clone(), RingKind::Pcm, FRAME_SIZE, CAPACITY).unwrap();
    let mut consumer = RingConsumer::attach(region, producer.expect()).unwrap();
    let block = vec![0.5f32; BLOCK_FRAMES * CHANNELS as usize];
    let mut out = vec![0.0f32; BLOCK_FRAMES * CHANNELS as usize];

    let vis_region = AnonymousRegion::new(RING_DATA_OFFSET + 8 * VIS_SLOT_SIZE).unwrap();
    let mut vis_producer =
        RingProducer::create(vis_region.clone(), RingKind::Vis, VIS_SLOT_SIZE as u32, 8).unwrap();
    let mut vis_consumer = RingConsumer::attach(vis_region, vis_producer.expect()).unwrap();
    let vis_slot = VisSlot::zeroed();
    let mut vis_out = VisSlot::zeroed();

    // Warm up: fault in the mapping pages and exercise both paths once.
    for _ in 0..64 {
        producer.write_from(&block).unwrap();
        consumer.read_into(&mut out).unwrap();
        vis_producer.write_slot(&vis_slot).unwrap();
        vis_consumer.read_slot_into(&mut vis_out).unwrap();
    }

    let before = ALLOCATIONS.load(Ordering::Relaxed);
    let before_read = consumer.frames_read();
    let mut written = 0u64;
    let mut read = 0u64;
    for _ in 0..ITERATIONS {
        written += producer.write_from(&block).unwrap() as u64;
        read += consumer.read_into(&mut out).unwrap() as u64;
        vis_producer.write_slot(&vis_slot).unwrap();
        vis_consumer.read_slot_into(&mut vis_out).unwrap();
    }
    let after = ALLOCATIONS.load(Ordering::Relaxed);

    assert_eq!(
        after - before,
        0,
        "the ring path allocated {} times",
        after - before
    );
    assert!(written > 0, "the producer never wrote");
    assert!(read > 0, "the consumer never read");
    // Sanity: the block really did move data through the ring.
    assert_eq!(consumer.frames_read() - before_read, read);
}
