//! The framing refuses an oversized length prefix before it allocates.
//!
//! The DoS class the MP4 probe showed in wave247: a corrupt length prefix
//! made the reader allocate what it claimed - 64 MB out of a 140-byte
//! file. The framing checks the prefix against `MAX_FRAME_LEN` first, so
//! the refusal costs nothing: the buffer of the announced size never
//! exists.
//!
//! This file deliberately contains a single test, so that no other thread
//! of the test harness allocates while the measurement runs (the same shape
//! as `no_alloc.rs`).

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};

use rustamp_ipc::{IpcError, MAX_FRAME_LEN, Message, read_frame, write_frame};

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
fn an_oversized_length_prefix_is_refused_without_allocating() {
    // The counter is live: reading a frame allocates its payload. Without
    // this the measurement below would be vacuous.
    let mut frame = Vec::new();
    write_frame(&mut frame, &Message::Ping(1)).expect("writing a frame");
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    let mut reader = Cursor::new(frame);
    let outcome = read_frame(&mut reader);
    let after = ALLOCATIONS.load(Ordering::Relaxed);
    assert!(outcome.is_ok(), "the frame must decode");
    assert!(
        after > before,
        "the counter did not move: the measurement would be vacuous"
    );

    // The refusal itself allocates nothing: the buffer of the announced
    // size never exists.
    for len in [MAX_FRAME_LEN + 1, u32::MAX] {
        let mut reader = Cursor::new(len.to_le_bytes().to_vec());
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        let outcome = read_frame(&mut reader);
        let after = ALLOCATIONS.load(Ordering::Relaxed);
        assert!(
            matches!(&outcome, Err(IpcError::FrameTooLarge(_))),
            "len {len}: got {outcome:?}"
        );
        assert_eq!(
            after - before,
            0,
            "the refusal of {len} bytes allocated {} times",
            after - before
        );
    }
}
