//! Deterministic null output for tests and `--null` playback.
//!
//! Nothing runs on its own: the caller drives the audio clock with
//! [`NullClock::pump`], which calls the engine's [`RenderFn`] exactly as a real
//! device would. That makes transport, gapless and seek tests reproducible
//! frame by frame.
//!
//! ```no_run
//! use rustamp_output::NullOutput;
//! let (output, clock) = NullOutput::pair();
//! // hand `output` to the engine, drive time from the test:
//! // clock.pump(1024);
//! ```
//!
//! The shared state uses a `Mutex` around the render callback. That is fine
//! here: `pump` is the audio thread of this *test* device and the lock is
//! uncontended in practice. A real backend never takes a lock on the audio
//! path; [`crate::WasapiOutput`] and [`crate::CpalOutput`] do not.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustamp_core::audio::{AudioSpec, OutputDevice, OutputFormat, RenderFn};
use rustamp_core::error::{Error, Result};

/// Frames per callback unless changed with [`NullClock::set_chunk_frames`].
pub const DEFAULT_CHUNK_FRAMES: u64 = 1024;

struct Inner {
    /// Render callback plus its scratch buffer and granted format.
    render: Option<RenderFn>,
    scratch: Vec<f32>,
    spec: Option<AudioSpec>,
}

struct Shared {
    inner: Mutex<Inner>,
    played: AtomicU64,
    paused: AtomicBool,
    running: AtomicBool,
    chunk_frames: AtomicU64,
}

/// The [`OutputDevice`] half of a null output. Pass it to the engine.
pub struct NullOutput {
    shared: Arc<Shared>,
}

/// The driving half of a null output. Lives in the test (or in the `--null`
/// mode of the CLI example) and moves the audio clock forward.
#[derive(Clone)]
pub struct NullClock {
    shared: Arc<Shared>,
}

impl NullOutput {
    /// A connected output/clock pair.
    pub fn pair() -> (NullOutput, NullClock) {
        let shared = Arc::new(Shared {
            inner: Mutex::new(Inner {
                render: None,
                scratch: Vec::new(),
                spec: None,
            }),
            played: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            running: AtomicBool::new(false),
            chunk_frames: AtomicU64::new(DEFAULT_CHUNK_FRAMES),
        });
        (
            NullOutput {
                shared: Arc::clone(&shared),
            },
            NullClock { shared },
        )
    }
}

impl OutputDevice for NullOutput {
    fn name(&self) -> &str {
        "null"
    }

    fn open(&mut self, spec: AudioSpec) -> Result<OutputFormat> {
        let chunk = self.shared.chunk_frames.load(Ordering::Relaxed).max(1);
        let mut inner = self
            .shared
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.spec = Some(spec);
        inner.scratch = vec![0.0; spec.samples_for(chunk as usize)];
        inner.render = None;
        self.shared.paused.store(false, Ordering::Release);
        self.shared.running.store(false, Ordering::Release);
        drop(inner);
        Ok(OutputFormat {
            spec,
            buffer_frames: chunk as u32,
        })
    }

    fn start(&mut self, render: RenderFn) -> Result<()> {
        let mut inner = self
            .shared
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.spec.is_none() {
            return Err(Error::Output("null output is not open".into()));
        }
        inner.render = Some(render);
        drop(inner);
        self.shared.paused.store(false, Ordering::Release);
        self.shared.running.store(true, Ordering::Release);
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        self.shared.paused.store(true, Ordering::Release);
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        self.shared.paused.store(false, Ordering::Release);
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        // Nothing is buffered: the render callback pulls from the engine's ring
        // on every `pump`, so a flush has nothing to drop.
        Ok(())
    }

    fn latency(&self) -> Duration {
        // One callback worth of frames at the granted rate.
        let inner = self
            .shared
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match inner.spec {
            Some(spec) if spec.sample_rate > 0 => spec.duration_of(
                self.shared
                    .chunk_frames
                    .load(Ordering::Relaxed)
                    .min(u64::from(u32::MAX)),
            ),
            _ => Duration::ZERO,
        }
    }

    fn played_frames(&self) -> u64 {
        self.shared.played.load(Ordering::Acquire)
    }

    fn close(&mut self) -> Result<()> {
        let mut inner = self
            .shared
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.render = None;
        inner.spec = None;
        drop(inner);
        self.shared.running.store(false, Ordering::Release);
        self.shared.paused.store(false, Ordering::Release);
        Ok(())
    }
}

impl std::fmt::Debug for NullOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NullOutput")
            .field("played", &self.played_frames())
            .finish_non_exhaustive()
    }
}

impl NullClock {
    /// Granted format, once the engine has opened the device.
    pub fn spec(&self) -> Option<AudioSpec> {
        self.lock().spec
    }

    /// Whether the engine has started the stream.
    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Acquire)
    }

    /// Whether the engine has paused the stream.
    pub fn is_paused(&self) -> bool {
        self.shared.paused.load(Ordering::Acquire)
    }

    /// Total frames the device has consumed (the audio clock).
    pub fn frames(&self) -> u64 {
        self.shared.played.load(Ordering::Acquire)
    }

    /// Frames pulled per [`NullClock::pump`] callback. Affects the callback
    /// size the engine sees and therefore the granularity of a seek. Resizes
    /// the callback scratch buffer, so call it outside a real-time window.
    pub fn set_chunk_frames(&self, frames: u64) {
        let frames = frames.max(1);
        self.shared.chunk_frames.store(frames, Ordering::Relaxed);
        let mut inner = self.lock();
        if let Some(spec) = inner.spec {
            inner.scratch = vec![0.0; spec.samples_for(frames as usize)];
        }
    }

    /// Simulate the device pulling `frames` frames and discarding the audio.
    /// Returns the frames consumed (`0` while stopped or paused).
    ///
    /// Allocates nothing.
    pub fn pump(&self, frames: u64) -> u64 {
        self.drive(frames, None)
    }

    /// Like [`NullClock::pump`], but appends the rendered audio to `out`.
    /// Test helper; allocates through `out`.
    pub fn pump_record(&self, frames: u64, out: &mut Vec<f32>) -> u64 {
        self.drive(frames, Some(out))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.shared
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn drive(&self, frames: u64, mut out: Option<&mut Vec<f32>>) -> u64 {
        if !self.shared.running.load(Ordering::Acquire)
            || self.shared.paused.load(Ordering::Acquire)
        {
            return 0;
        }
        let chunk = self.shared.chunk_frames.load(Ordering::Relaxed).max(1);
        let mut inner = self.lock();
        let Some(spec) = inner.spec else {
            return 0;
        };
        let mut left = frames;
        let mut done = 0u64;
        while left > 0 {
            let n = left.min(chunk);
            let samples = spec.samples_for(n as usize);
            let Inner {
                render, scratch, ..
            } = &mut *inner;
            let Some(render) = render.as_mut() else {
                break;
            };
            // The clock advances before the callback: `played_frames` then
            // counts the frames the device handed to the callback, which is
            // what a real backend reports as well.
            self.shared.played.fetch_add(n, Ordering::AcqRel);
            render(&mut scratch[..samples]);
            if let Some(out) = out.as_deref_mut() {
                out.extend_from_slice(&scratch[..samples]);
            }
            left -= n;
            done += n;
        }
        done
    }
}

impl std::fmt::Debug for NullClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NullClock")
            .field("frames", &self.frames())
            .field("running", &self.is_running())
            .field("paused", &self.is_paused())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output_and_clock() -> (NullOutput, NullClock) {
        NullOutput::pair()
    }

    #[test]
    fn pump_calls_render_and_advances_the_clock() {
        let (mut out, clock) = output_and_clock();
        let spec = AudioSpec::new(8000, 2);
        assert_eq!(out.open(spec).unwrap().spec, spec);
        assert_eq!(clock.pump(100), 0, "not started yet");
        let calls = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&calls);
        out.start(Box::new(move |buf: &mut [f32]| {
            counter.fetch_add(1, Ordering::Relaxed);
            buf.fill(0.25);
        }))
        .unwrap();
        assert_eq!(clock.pump(1500), 1500);
        assert_eq!(clock.frames(), 1500);
        assert_eq!(calls.load(Ordering::Relaxed), 2, "1024 + 476");
        out.pause().unwrap();
        assert_eq!(clock.pump(100), 0);
        assert_eq!(clock.frames(), 1500);
        out.resume().unwrap();
        assert_eq!(clock.pump(10), 10);
        out.close().unwrap();
        assert_eq!(clock.pump(10), 0);
    }

    #[test]
    fn records_the_rendered_audio() {
        let (mut out, clock) = output_and_clock();
        let spec = AudioSpec::new(8000, 1);
        out.open(spec).unwrap();
        out.start(Box::new(|buf: &mut [f32]| {
            for (i, v) in buf.iter_mut().enumerate() {
                *v = i as f32;
            }
        }))
        .unwrap();
        let mut rec = Vec::new();
        clock.pump_record(4, &mut rec);
        assert_eq!(rec, vec![0.0, 1.0, 2.0, 3.0]);
    }
}
