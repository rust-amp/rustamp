//! The lock-free ring between the decode thread and the output callback, plus
//! the seek/flush handshake that keeps the two sides in step.
//!
//! # Data flow
//!
//! ```text
//! decode thread: decoder -> DSP -> converter -> Producer<f32>
//! output thread: Consumer<f32> -> gain -> device
//! ```
//!
//! Both ends are wait-free; the producer polls the ring's free space with a
//! short sleep when it is full, the consumer never blocks.
//!
//! # Seek / flush handshake
//!
//! A seek invalidates everything that is already in the ring, but only the
//! consumer can drop it. Three atomics sequence the switch (all in
//! [`RingSync`]):
//!
//! 1. The decode thread bumps [`RingSync::request_epoch`] *after* it has sought
//!    the decoder. The next render callback sees the new epoch, discards the
//!    whole ring and stores [`RingSync::drain_ack`].
//! 2. The decode thread waits for that ack (servicing control messages while it
//!    waits) and only then reports the new timeline position to the engine,
//!    which re-bases its audio clock (`clock_origin`) so that the position
//!    starts exactly at the seek target.
//! 3. The engine stores [`RingSync::engine_ack`]; only then does the decode
//!    thread write the first frame of the new position, so no post-seek frame
//!    is ever dropped by the drain of step 1.
//!
//! The drain happens in the render callback and therefore also works when the
//! stream is paused and only resumes later. When the output is not running the
//! decode thread skips the waits (nothing would consume the ring anyway) and
//! the consumer drains on the first callback after the resume; that can drop
//! the first block of new audio, which is the documented price of seeking while
//! paused.
//!
//! # End-of-stream detection (F2 fix)
//!
//! The engine must finish a run only when the ring is truly empty — every frame
//! the decode thread wrote has been consumed. Using the device clock
//! (`played_frames`) for this is wrong: the clock advances for silence frames
//! written during underruns, so it can run ahead of `written` and trigger a
//! premature finish.
//!
//! Instead, the render callback maintains a consumer-side counter
//! ([`RingSync::consumed`]) that counts only the frames it actually read from
//! the ring. The engine's finish condition is then:
//!
//! ```text
//! written == end_of_stream && consumed >= written
//! ```
//!
//! On a seek-drain the counter is reset to `written` so that pre-seek frames
//! are treated as consumed and the invariant `consumed <= written` holds for
//! the new epoch.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rtrb::Consumer;
use rustamp_core::audio::RenderFn;

use crate::gain;

/// How long the decode thread waits for the render side before it gives up and
/// continues (a stalled or paused device must never wedge the decoder).
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(250);

/// Volume and balance, shared with the render callback. Kept in atomics so a
/// change is audible on the next callback without touching the ring.
#[derive(Debug, Default)]
pub struct GainState {
    volume: AtomicU8,
    balance: AtomicI8,
}

impl GainState {
    /// A gain state with the given values.
    pub fn new(volume: u8, balance: i8) -> Self {
        GainState {
            volume: AtomicU8::new(volume),
            balance: AtomicI8::new(balance),
        }
    }

    /// Set both values.
    pub fn set(&self, volume: u8, balance: i8) {
        self.volume.store(volume, Ordering::Release);
        self.balance.store(balance, Ordering::Release);
    }

    /// Current values.
    pub fn get(&self) -> (u8, i8) {
        (
            self.volume.load(Ordering::Acquire),
            self.balance.load(Ordering::Acquire),
        )
    }
}

/// State shared by the decode thread, the render callback and the engine.
#[derive(Debug, Default)]
pub struct RingSync {
    /// Incremented by the decode thread when the ring content is invalidated
    /// (seek).
    epoch: AtomicU64,
    /// Epoch the render callback has finished draining.
    drain_ack: AtomicU64,
    /// Epoch the engine has re-based its clock for.
    engine_ack: AtomicU64,
    /// Whether the output device is currently pulling.
    active: AtomicBool,
    /// Absolute timeline position (device frames) the decode thread has written
    /// up to. The engine uses it to report how much audio is buffered.
    written: AtomicU64,
    /// Absolute timeline position (device frames) that the render callback has
    /// actually read out of the ring. Updated with `Relaxed` ordering on the
    /// audio thread — no allocation, no lock, no logging. Silence frames
    /// written during underruns are **not** counted here.
    ///
    /// Invariant (per epoch): `consumed <= written`.
    ///
    /// On a seek-drain the render callback resets this to the current value of
    /// `written` so that pre-seek frames are treated as consumed and the
    /// invariant holds for the new epoch.
    consumed: AtomicU64,
}

impl RingSync {
    /// New, unstarted state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode side: publish the timeline position of the last written frame.
    pub fn set_written(&self, timeline: u64) {
        self.written.store(timeline, Ordering::Release);
    }

    /// Absolute timeline position the decode thread has written up to.
    pub fn written(&self) -> u64 {
        self.written.load(Ordering::Acquire)
    }

    /// Store the running total of device frames actually read from the ring.
    /// Called from the render callback (Relaxed, no allocation, lock or log)
    /// and reset to 0 by the engine on start_run and stop.
    pub fn set_consumed(&self, frames: u64) {
        // Relaxed is sufficient: the render callback is the sole writer during
        // playback; start_run and stop reset it when the stream is not running.
        self.consumed.store(frames, Ordering::Relaxed);
    }

    /// Device frames the render callback has actually read from the ring (not
    /// counting silence written during underruns). Safe to read from any
    /// thread; the value may be stale by one callback when read off the audio
    /// thread.
    pub fn consumed(&self) -> u64 {
        self.consumed.load(Ordering::Relaxed)
    }

    /// Ask the consumer to discard the ring; returns the new epoch.
    pub fn request_epoch(&self) -> u64 {
        self.epoch.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Epoch the consumer is currently serving.
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// The engine marks the output as pulling (start/resume) or not
    /// (pause/stop).
    pub fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Release);
    }

    /// Whether the output is pulling.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// Engine side: acknowledge that the clock has been re-based for `epoch`.
    pub fn ack_engine(&self, epoch: u64) {
        self.engine_ack.store(epoch, Ordering::Release);
    }

    /// Decode side: wait until the consumer has drained `epoch`.
    ///
    /// Returns `false` when it timed out or the stream is not pulling.
    pub fn wait_for_drain(&self, epoch: u64) -> bool {
        wait_for(&self.drain_ack, epoch)
    }

    /// Decode side: whether the consumer has drained `epoch`.
    pub fn drained(&self, epoch: u64) -> bool {
        self.drain_ack.load(Ordering::Acquire) >= epoch
    }

    /// Decode side: wait until the engine has re-based its clock.
    pub fn wait_for_engine(&self, epoch: u64) -> bool {
        wait_for(&self.engine_ack, epoch)
    }
}

fn wait_for(cell: &AtomicU64, epoch: u64) -> bool {
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    while cell.load(Ordering::Acquire) < epoch {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::yield_now();
    }
    true
}

/// Build the render callback for a device with `channels` channels.
///
/// The callback pulls from `consumer`, applies volume and balance and writes
/// silence for whatever the ring could not supply. It allocates nothing, takes
/// no lock, does no I/O and logs nothing.
///
/// After every callback the `consumed` counter in `sync` is updated with the
/// number of device frames **actually read from the ring**. Silence frames
/// produced during underruns are not counted. The engine uses this to detect
/// when everything the decode thread wrote has been played, without relying on
/// the device clock.
pub fn make_render(
    mut consumer: Consumer<f32>,
    sync: Arc<RingSync>,
    gain: Arc<GainState>,
    channels: u16,
) -> RenderFn {
    let channels = channels.max(1);
    let mut served_epoch = sync.epoch();
    Box::new(move |out: &mut [f32]| {
        let epoch = sync.epoch();
        if epoch != served_epoch {
            // A seek invalidated the ring: drop everything that is in it, ack
            // the drain and stay silent for this callback.
            loop {
                let available = consumer.slots();
                if available == 0 {
                    break;
                }
                match consumer.read_chunk(available) {
                    Ok(chunk) => chunk.commit_all(),
                    Err(_) => break,
                }
            }
            served_epoch = epoch;
            // Align `consumed` with `written` so that the pre-seek frames are
            // treated as consumed and the invariant consumed <= written holds
            // for the new epoch.
            let written = sync.written();
            sync.set_consumed(written);
            sync.drain_ack.store(epoch, Ordering::Release);
            out.fill(0.0);
            return;
        }
        let mut written = 0;
        while written < out.len() {
            let available = consumer.slots();
            if available == 0 {
                break;
            }
            let want = (out.len() - written).min(available);
            let chunk = match consumer.read_chunk(want) {
                Ok(chunk) => chunk,
                Err(_) => break,
            };
            let n = chunk.len();
            {
                let (first, second) = chunk.as_slices();
                out[written..written + first.len()].copy_from_slice(first);
                let base = written + first.len();
                out[base..base + second.len()].copy_from_slice(second);
            }
            chunk.commit_all();
            written += n;
        }
        out[written..].fill(0.0);
        // Count only the frames actually read from the ring. `written` here is
        // in samples; divide by channels to get frames.
        let frames_read = (written / usize::from(channels)) as u64;
        let consumed = sync.consumed().saturating_add(frames_read);
        sync.set_consumed(consumed);
        let (volume, balance) = gain.get();
        gain::apply(&mut out[..written], channels, volume, balance);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtrb::RingBuffer;

    fn render_into(
        render: &mut RenderFn,
        frames: usize,
        channels: u16,
        sink: &mut Vec<f32>,
    ) -> Vec<f32> {
        let mut buf = vec![0.0f32; frames * usize::from(channels)];
        render(&mut buf);
        sink.extend_from_slice(&buf);
        buf
    }

    #[test]
    fn passes_audio_through_and_fills_underruns_with_silence() {
        let (mut producer, consumer) = RingBuffer::<f32>::new(16);
        let sync = Arc::new(RingSync::new());
        let gain = Arc::new(GainState::new(255, 0));
        let mut render = make_render(consumer, Arc::clone(&sync), gain, 2);
        for i in 0..6 {
            producer.push(i as f32).unwrap();
        }
        let out = render_into(&mut render, 4, 2, &mut Vec::new());
        assert_eq!(out, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 0.0, 0.0]);
    }

    #[test]
    fn applies_the_gain() {
        let (mut producer, consumer) = RingBuffer::<f32>::new(16);
        let sync = Arc::new(RingSync::new());
        let gain = Arc::new(GainState::new(255, 0));
        let mut render = make_render(consumer, Arc::clone(&sync), gain.clone(), 2);
        for _ in 0..4 {
            producer.push(1.0).unwrap();
        }
        gain.set(255, 127);
        let out = render_into(&mut render, 2, 2, &mut Vec::new());
        assert_eq!(out, vec![0.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn epoch_change_drains_the_ring() {
        let (mut producer, consumer) = RingBuffer::<f32>::new(16);
        let sync = Arc::new(RingSync::new());
        let gain = Arc::new(GainState::new(255, 0));
        let mut render = make_render(consumer, Arc::clone(&sync), gain, 1);
        for i in 0..8 {
            producer.push(i as f32).unwrap();
        }
        let epoch = sync.request_epoch();
        let out = render_into(&mut render, 4, 1, &mut Vec::new());
        assert_eq!(out, vec![0.0; 4], "silence while draining");
        assert!(sync.wait_for_drain(epoch), "the drain is acknowledged");
        // The stale frames are gone; new frames pass through.
        producer.push(42.0).unwrap();
        let out = render_into(&mut render, 1, 1, &mut Vec::new());
        assert_eq!(out, vec![42.0]);
    }

    #[test]
    fn handshake_times_out_instead_of_hanging() {
        let sync = RingSync::new();
        let epoch = sync.request_epoch();
        assert!(!sync.wait_for_drain(epoch), "no consumer, no ack");
    }

    /// The consumed counter must track only ring frames, not silence frames.
    #[test]
    fn consumed_counts_ring_frames_not_silence() {
        let (mut producer, consumer) = RingBuffer::<f32>::new(32);
        let sync = Arc::new(RingSync::new());
        let gain = Arc::new(GainState::new(255, 0));
        let channels = 2u16;
        let mut render = make_render(consumer, Arc::clone(&sync), Arc::clone(&gain), channels);

        // Write 4 frames (8 samples for stereo) into the ring.
        for i in 0..8 {
            producer.push(i as f32).unwrap();
        }
        // Request a callback for 6 frames: 4 from the ring, 2 as silence.
        render_into(&mut render, 6, channels, &mut Vec::new());

        // consumed must be 4 (the ring frames), not 6 (total callback frames).
        assert_eq!(
            sync.consumed(),
            4,
            "silence frames must not count toward consumed"
        );
    }

    /// After an epoch drain, consumed is reset to written so the invariant holds.
    #[test]
    fn consumed_resets_to_written_after_epoch_drain() {
        let (mut producer, consumer) = RingBuffer::<f32>::new(32);
        let sync = Arc::new(RingSync::new());
        let gain = Arc::new(GainState::new(255, 0));
        let mut render = make_render(consumer, Arc::clone(&sync), Arc::clone(&gain), 1);

        // Simulate: decode thread wrote 10 frames.
        sync.set_written(10);
        for i in 0..10 {
            producer.push(i as f32).unwrap();
        }
        // Consume 6 of them.
        render_into(&mut render, 6, 1, &mut Vec::new());
        assert_eq!(sync.consumed(), 6);

        // Now a seek happens: decode thread bumps epoch and updates written.
        sync.set_written(10); // still 10 before the seek position is published
        let epoch = sync.request_epoch();
        // The next render callback drains the remaining 4 frames.
        render_into(&mut render, 4, 1, &mut Vec::new());
        assert!(sync.drained(epoch));
        // consumed must now equal written (10), not 6+4=10 by coincidence but
        // because it was explicitly reset to written (which is 10 here anyway).
        assert_eq!(
            sync.consumed(),
            sync.written(),
            "consumed must equal written after epoch drain"
        );
    }

    /// After a run ends and engine resets written and consumed to 0, the next
    /// run's reads count from 0.
    #[test]
    fn consumed_resets_for_second_run() {
        let (mut producer, consumer) = RingBuffer::<f32>::new(32);
        let sync = Arc::new(RingSync::new());
        let gain = Arc::new(GainState::new(255, 0));
        let mut render = make_render(consumer, Arc::clone(&sync), Arc::clone(&gain), 1);

        // First run writes and consumes 10 frames.
        sync.set_written(10);
        for i in 0..10 {
            producer.push(i as f32).unwrap();
        }
        render_into(&mut render, 10, 1, &mut Vec::new());
        assert_eq!(sync.consumed(), 10);

        // Run ends; engine resets counters to 0 for a second run.
        sync.set_written(0);
        sync.set_consumed(0);

        // Second run writes 5 frames.
        sync.set_written(5);
        for i in 0..5 {
            producer.push(i as f32).unwrap();
        }
        render_into(&mut render, 5, 1, &mut Vec::new());
        assert_eq!(
            sync.consumed(),
            5,
            "consumed must start from 0 for the second run"
        );
    }
}
