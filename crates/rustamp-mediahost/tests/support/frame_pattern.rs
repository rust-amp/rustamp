//! The deterministic frame pattern the resident fixtures publish.
//!
//! Frame `index` carries `[index / 8, -index / 8]`: every frame has its own
//! value, so a torn, missing or reordered frame is visible in the samples.
//! The staged VM fixture (`tests/support/resident.rs`) and the in-process fake
//! host publish the same pattern, so the in-process and the two-process kill
//! checks share one expectation instead of two near-copies of it.

/// The two interleaved samples of frame `index`.
pub fn frame_samples(index: usize) -> [f32; 2] {
    let value = index as f32 / 8.0;
    [value, -value]
}
