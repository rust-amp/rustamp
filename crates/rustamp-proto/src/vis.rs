//! Visualization data tap contract.
//!
//! The audio engine publishes [`VisFrame`]s (a recent window of audio plus a
//! spectrum) to the visualization consumers. The exact Winamp derivation of
//! plugin-facing data - 8-bit scaling of the waveform, FFT size/window,
//! spectrum bin mapping and scaling, refresh cadence - is defined by spec
//! **S08**, and the conversion lives in `rustamp-vis`. Until then the fields
//! here carry *neutral* data: waveform as `f32` in `[-1, 1]`, spectrum as
//! linear magnitude normalised so a full-scale sine is about `1.0`.
//! Do not bake S08 assumptions into other crates.

/// Samples/bins per channel in a [`VisFrame`] (matches the classic plugin interface size).
pub const VIS_SAMPLES: usize = 576;
/// Channels in a [`VisFrame`]. Mono sources are duplicated to both.
pub const VIS_CHANNELS: usize = 2;

/// One snapshot of waveform and spectrum data.
///
/// This type is not directly `serde`-serializable (large arrays); the IPC
/// crate has a wire form.
#[derive(Debug, Clone, PartialEq)]
pub struct VisFrame {
    /// Audio-clock timestamp of the *end* of the waveform window, in microseconds since playback start.
    pub timestamp_us: u64,
    /// Most recent waveform samples, `[channel][sample]`, oldest first.
    pub waveform: [[f32; VIS_SAMPLES]; VIS_CHANNELS],
    /// Spectrum bins, `[channel][bin]`, lowest frequency first. Bin mapping: spec S08.
    pub spectrum: [[f32; VIS_SAMPLES]; VIS_CHANNELS],
}

impl Default for VisFrame {
    fn default() -> Self {
        VisFrame {
            timestamp_us: 0,
            waveform: [[0.0; VIS_SAMPLES]; VIS_CHANNELS],
            spectrum: [[0.0; VIS_SAMPLES]; VIS_CHANNELS],
        }
    }
}

// RFC 0003 sections 4 and 5: these constants are part of the byte-identical
// shared-memory ring layout on x86 and x86_64. The full layout asserts live in
// `rustamp-ipc::shm::header`; this one fails the build if the constants drift
// away from the ring slot definition.
const _: () = {
    assert!(VIS_SAMPLES == 576);
    assert!(VIS_CHANNELS == 2);
};
