//! The render path must not allocate.
//!
//! A counting global allocator wraps the system one and only counts while a
//! test enables it, so the rest of the binary (and the test harness itself) is
//! unaffected. There is exactly one test in this file on purpose: a global
//! allocator counts allocations from *every* thread, so a second test running in
//! parallel would make the result meaningless.
//!
//! What is measured:
//!
//! 1. A soak of several seconds of virtual audio through the engine's
//!    [`RenderFn`] (the callback the device calls), after the decode thread has
//!    gone idle. The decode thread is not touched: it allocates by design.
//! 2. The same through a crossfade.
//! 3. [`rustamp_audio::VisTap::push`], the other hot loop of the engine.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use common::{Harness, SPEC};
use rustamp_audio::VisTap;
use rustamp_core::audio::AudioSpec;

static ENABLED: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

// SAFETY: every method forwards to `System` unchanged; the counters are plain
// atomics and never affect the returned pointer.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: `layout` comes from the caller of `alloc`; forwarding it to
        // the system allocator is what this wrapper is for.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` and `layout` come from a previous `alloc` of this
        // allocator, which forwards to `System`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: `ptr` and `layout` come from a previous allocation, and
        // `new_size` is the requested size; `System` handles both.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

/// Run `body` with allocation counting on and return how many allocations
/// happened. Never runs two measurements at once.
fn count_allocations(body: impl FnOnce()) -> usize {
    let before = ALLOCATIONS.load(Ordering::Relaxed);
    ENABLED.store(true, Ordering::SeqCst);
    body();
    ENABLED.store(false, Ordering::SeqCst);
    ALLOCATIONS.load(Ordering::Relaxed) - before
}

#[test]
fn render_path_and_vis_tap_allocate_nothing() {
    // --- engine render path -------------------------------------------------
    let mut h = Harness::new(&["track1_3000.tst"]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    // Let the decode thread prime the ring; its own allocations must not land
    // inside the measurement window.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while h.engine.buffered_frames() == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        h.engine.buffered_frames() > 0,
        "stall: the ring was never primed within 30s"
    );
    // Drain the queue so the decode thread has nothing left to do.
    let rate = u64::from(SPEC.sample_rate);
    let mut pumped = 0u64;
    while h.engine.buffered_frames() > 0 && pumped < rate * 4 {
        h.clock.pump(1024);
        pumped += 1024;
    }
    h.engine.poll().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));

    // A few seconds of virtual audio through the render callback: the ring
    // runs dry halfway and the callback must write silence, still without
    // allocating.
    let soak = count_allocations(|| {
        let mut frames = 0u64;
        while frames < rate * 3 {
            h.clock.pump(1024);
            frames += 1024;
        }
    });
    assert_eq!(soak, 0, "the render path allocated {soak} times");

    // --- a crossfade -----------------------------------------------------------
    // The mix is the decode thread's: once the decode thread is idle, the
    // callback plays an overlap like anything else that is in the ring.
    let cfg = rustamp_audio::EngineConfig {
        crossfade_ms: 20,
        ..rustamp_audio::EngineConfig::default()
    };
    let mut fading = Harness::with_config(&["track1_3000.tst", "track2_3000.tst"], cfg);
    fading.engine.play_index(0).unwrap();
    assert!(fading.wait_buffered(), "stall: the queue was never written");
    std::thread::sleep(std::time::Duration::from_millis(50));
    let through_the_fade = count_allocations(|| {
        let mut frames = 0u64;
        while frames < 8192 {
            fading.clock.pump(1024);
            frames += 1024;
        }
    });
    assert_eq!(
        through_the_fade, 0,
        "the render path allocated {through_the_fade} times through a crossfade"
    );

    // --- vis tap ------------------------------------------------------------
    let (mut tap, _consumer) = VisTap::new(4);
    tap.prepare(AudioSpec::new(44100, 2));
    let block = vec![0.5f32; 1024 * 2];
    let tap_allocations = count_allocations(|| {
        for _ in 0..200 {
            tap.push(&block);
        }
    });
    assert_eq!(
        tap_allocations, 0,
        "the vis tap allocated {tap_allocations} times"
    );
}
