//! Visualization tap: fills [`VisFrame`]s from the engine-format PCM.
//!
//! The tap runs on the **decode** thread, where the audio still has the engine
//! format (before the converter remaps it to the device format), and publishes
//! finished frames into a small lock-free ring that the UI drains. That keeps
//! the render callback free of the 2 x 576 sample bookkeeping.
//!
//! What it fills today (task card A1: "the raw interleaved form only"):
//!
//! * `waveform` — the most recent 576 samples per channel, oldest first, taken
//!   straight from the engine-format PCM. Mono is duplicated to both rows.
//! * `timestamp_us` — the audio-clock time of the **end** of that window.
//! * `spectrum` — left at zero. The FFT, its window, the bin mapping and the
//!   8-bit scaling are spec **S08**; guessing them here would bake an
//!   unverified derivation into the engine. See the report for the spec
//!   request.
//!
//! Rustamp's UI may derive its own display from these actual waveform samples;
//! that is not S08's unmeasured plugin conversion. `VisFrame` carries no sample
//! rate or seek generation: the caller must supply the engine's source format,
//! never infer a rate from rounded kHz or the output device. Publication happens
//! once at the end of a sufficiently large decode block (not at an exact
//! 576-frame hop). Decode can run ahead of audible output; the timestamp below
//! is not proof of audio-to-pixel synchronisation. Dropped/absent frames are not
//! digital silence. The built-in renderer must not fabricate missing samples.
//!
//! [`VisTap::push`] allocates nothing, so the tap can run inside the decode
//! loop without touching the allocator.

use rtrb::{Consumer, Producer, RingBuffer};
use rustamp_core::audio::AudioSpec;
use rustamp_core::vis::{VIS_CHANNELS, VIS_SAMPLES, VisFrame};

/// Frames between two published [`VisFrame`]s.
///
/// 576 frames is the width of one window; publishing per decode block (which is
/// at least that large) keeps the tap cheap. The final cadence is an S08
/// question.
pub const DEFAULT_PUBLISH_FRAMES: u64 = VIS_SAMPLES as u64;

/// Fills [`VisFrame`]s from interleaved engine-format frames.
pub struct VisTap {
    producer: Producer<VisFrame>,
    /// Circular history of the newest samples, `[channel][sample]`.
    history: [[f32; VIS_SAMPLES]; VIS_CHANNELS],
    /// Next write position in `history`.
    write: usize,
    /// Frames pushed since the last reset (the audio clock of the tap).
    frames: u64,
    /// Frames pushed since the last published frame.
    since_publish: u64,
    publish_every: u64,
    spec: AudioSpec,
    enabled: bool,
}

impl VisTap {
    /// Create a tap and the consumer the UI drains.
    pub fn new(capacity: usize) -> (VisTap, Consumer<VisFrame>) {
        let (producer, consumer) = RingBuffer::new(capacity.max(1));
        (
            VisTap {
                producer,
                history: [[0.0; VIS_SAMPLES]; VIS_CHANNELS],
                write: 0,
                frames: 0,
                since_publish: 0,
                publish_every: DEFAULT_PUBLISH_FRAMES,
                spec: AudioSpec::new(44100, 2),
                enabled: true,
            },
            consumer,
        )
    }

    /// Configure for a new engine format. Called at track start and on a
    /// mid-stream format change; may allocate nothing.
    pub fn prepare(&mut self, spec: AudioSpec) {
        self.spec = spec;
        self.reset();
    }

    /// Clear the history (seek, track change).
    pub fn reset(&mut self) {
        self.history = [[0.0; VIS_SAMPLES]; VIS_CHANNELS];
        self.write = 0;
        self.frames = 0;
        self.since_publish = 0;
    }

    /// Enable or disable publishing (the tap keeps counting either way).
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Frames pushed since the last reset.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Engine format in use.
    pub fn spec(&self) -> AudioSpec {
        self.spec
    }

    /// Push interleaved engine-format frames and publish a [`VisFrame`] when a
    /// window has passed.
    ///
    /// Allocation-free; drops the frame when the consumer is too slow.
    pub fn push(&mut self, frames: &[f32]) {
        let channels = usize::from(self.spec.channels.max(1));
        let count = frames.len() / channels;
        if count == 0 {
            return;
        }
        for i in 0..count {
            let frame = &frames[i * channels..(i + 1) * channels];
            for ch in 0..VIS_CHANNELS {
                let v = if channels == 1 {
                    frame[0]
                } else {
                    frame.get(ch).copied().unwrap_or(0.0)
                };
                self.history[ch][self.write] = v;
            }
            self.write = (self.write + 1) % VIS_SAMPLES;
        }
        self.frames += count as u64;
        self.since_publish += count as u64;
        if self.since_publish < self.publish_every {
            return;
        }
        self.since_publish = 0;
        if !self.enabled {
            return;
        }
        let frame = self.build_frame();
        // Full ring: drop the frame rather than block the decode thread.
        let _ = self.producer.push(frame);
    }

    fn build_frame(&self) -> VisFrame {
        let mut out = VisFrame {
            timestamp_us: self.timestamp_us(),
            ..VisFrame::default()
        };
        for ch in 0..VIS_CHANNELS {
            // Oldest first: start at the oldest sample still in the history.
            let start = self.write;
            for (i, slot) in out.waveform[ch].iter_mut().enumerate() {
                *slot = self.history[ch][(start + i) % VIS_SAMPLES];
            }
        }
        out
    }

    /// Audio-clock time of the end of the newest window, in microseconds.
    pub fn timestamp_us(&self) -> u64 {
        let rate = u64::from(self.spec.sample_rate.max(1));
        self.frames.saturating_mul(1_000_000) / rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_the_waveform_with_the_newest_samples() {
        let spec = AudioSpec::new(8000, 1);
        let (mut tap, mut consumer) = VisTap::new(4);
        tap.prepare(spec);
        // 600 frames: enough for exactly one published window.
        let input: Vec<f32> = (0..600).map(|i| i as f32).collect();
        tap.push(&input);
        assert_eq!(tap.frames(), 600);
        let frame = consumer.pop().expect("one frame published");
        // The window holds the newest 576 samples, oldest first.
        assert_eq!(frame.waveform[0][0], 24.0);
        assert_eq!(frame.waveform[0][575], 599.0);
        assert_eq!(frame.waveform[1], frame.waveform[0], "mono is duplicated");
        assert!(frame.spectrum[0].iter().all(|v| *v == 0.0), "S08 owns this");
        assert_eq!(frame.timestamp_us, 600 * 1_000_000 / 8000);
        assert!(consumer.pop().is_err(), "one frame per window");
    }

    #[test]
    fn stereo_rows_are_separated() {
        let spec = AudioSpec::new(8000, 2);
        let (mut tap, mut consumer) = VisTap::new(2);
        tap.prepare(spec);
        let mut input = Vec::new();
        for i in 0..600 {
            input.push(i as f32);
            input.push(-(i as f32));
        }
        tap.push(&input);
        let frame = consumer.pop().unwrap();
        assert_eq!(frame.waveform[0][0], 24.0);
        assert_eq!(frame.waveform[1][0], -24.0);
    }

    #[test]
    fn wraps_around_the_history() {
        let spec = AudioSpec::new(8000, 1);
        let (mut tap, mut consumer) = VisTap::new(4);
        tap.prepare(spec);
        let first: Vec<f32> = (0..400).map(|i| i as f32).collect();
        tap.push(&first);
        let second: Vec<f32> = (400..800).map(|i| i as f32).collect();
        tap.push(&second);
        let frame = consumer.pop().unwrap();
        assert_eq!(frame.waveform[0][0], 224.0);
        assert_eq!(frame.waveform[0][575], 799.0);
    }

    #[test]
    fn full_ring_drops_instead_of_blocking() {
        let spec = AudioSpec::new(8000, 1);
        let (mut tap, mut consumer) = VisTap::new(1);
        tap.prepare(spec);
        let input = vec![1.0f32; 600];
        tap.push(&input);
        tap.push(&input);
        assert!(consumer.pop().is_ok());
        assert!(consumer.pop().is_err(), "the second frame was dropped");
    }

    #[test]
    fn disabled_tap_publishes_nothing() {
        let spec = AudioSpec::new(8000, 1);
        let (mut tap, mut consumer) = VisTap::new(2);
        tap.prepare(spec);
        tap.set_enabled(false);
        tap.push(&vec![1.0f32; 600]);
        assert!(consumer.pop().is_err());
        assert_eq!(tap.frames(), 600, "the clock still runs");
    }
}
