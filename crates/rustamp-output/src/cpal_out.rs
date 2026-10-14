//! Cross-platform output through `cpal` (the fallback for non-Windows hosts and
//! for Windows machines where WASAPI is unavailable).
//!
//! * Format: `f32`, taken from the device's own supported output configurations
//!   (preferring the engine's requested sample rate). If the granted rate or
//!   channel count differs from the engine's format, the engine's converter
//!   resamples/remixes.
//! * The callback gets the device's `&mut [f32]` buffer and passes it straight
//!   to the engine's [`RenderFn`]. **No allocation, no lock, no I/O there.**
//! * `played_frames` counts the frames rendered to the callback; it is never
//!   reset.
//! * `flush` is a no-op: cpal offers no way to drop what the backend already
//!   handed to the device. The engine's ring drain discards everything that has
//!   not been rendered yet, which is what a seek needs.
//! * The stream is created in `start`, not in `open`: `open` only negotiates the
//!   format, which keeps the render callback out of a lock.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rustamp_core::audio::{AudioSpec, OutputDevice, OutputFormat, RenderFn};
use rustamp_core::error::{Error, Result};

/// Callback size assumed for the reported latency when the backend does not
/// tell us (cpal exposes no buffer size).
const ASSUMED_CALLBACK_FRAMES: u32 = 1024;

struct Shared {
    played: AtomicU64,
    /// Latched by the error callback; reported by `close`.
    failed: AtomicBool,
}

/// `cpal`-backed output device.
pub struct CpalOutput {
    device: Option<cpal::Device>,
    config: Option<cpal::StreamConfig>,
    stream: Option<cpal::Stream>,
    spec: Option<AudioSpec>,
    device_name: String,
    shared: Arc<Shared>,
}

impl Default for CpalOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl CpalOutput {
    /// Not opened yet; call [`OutputDevice::open`].
    pub fn new() -> Self {
        CpalOutput {
            device: None,
            config: None,
            stream: None,
            spec: None,
            device_name: "cpal".into(),
            shared: Arc::new(Shared {
                played: AtomicU64::new(0),
                failed: AtomicBool::new(false),
            }),
        }
    }

    /// Name of the opened device, once open.
    pub fn device_name(&self) -> &str {
        &self.device_name
    }
}

/// Best `f32` output configuration for `device`: a configuration whose sample
/// rate equals `want.sample_rate` if the device has one, otherwise the
/// highest-rate candidate. Ties go to the choice with more channels.
fn pick_config(device: &cpal::Device, want: AudioSpec) -> Result<(cpal::StreamConfig, AudioSpec)> {
    let supported = device
        .supported_output_configs()
        .map_err(|e| Error::Output(format!("cpal: querying formats: {e}")))?;
    let mut best: Option<(cpal::StreamConfig, AudioSpec)> = None;
    let mut best_exact = false;
    for range in supported {
        if range.sample_format() != cpal::SampleFormat::F32 {
            continue;
        }
        let min = range.min_sample_rate();
        let max = range.max_sample_rate();
        let rate = want.sample_rate.clamp(min, max);
        let exact = rate == want.sample_rate;
        let spec = AudioSpec::new(rate, range.channels());
        let better = match &best {
            None => true,
            Some((_, current)) => {
                (exact && !best_exact) || (exact == best_exact && spec.channels > current.channels)
            }
        };
        if better {
            best = Some((
                cpal::StreamConfig {
                    channels: range.channels(),
                    sample_rate: rate,
                    buffer_size: cpal::BufferSize::Default,
                },
                spec,
            ));
            best_exact = exact;
        }
    }
    if let Some(found) = best {
        return Ok(found);
    }
    // No explicit `f32` range advertised: fall back to the device default and
    // let the stream builder fail if it is not `f32`.
    let default = device
        .default_output_config()
        .map_err(|e| Error::Output(format!("cpal: default config: {e}")))?;
    let spec = AudioSpec::new(default.sample_rate(), default.channels());
    let mut config: cpal::StreamConfig = default.into();
    config.buffer_size = cpal::BufferSize::Default;
    Ok((config, spec))
}

impl OutputDevice for CpalOutput {
    fn name(&self) -> &str {
        &self.device_name
    }

    fn open(&mut self, spec: AudioSpec) -> Result<OutputFormat> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| Error::Output("cpal: no default output device".into()))?;
        self.device_name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| "default output device".into());
        let (config, granted) = pick_config(&device, spec)?;
        self.shared.played.store(0, Ordering::Release);
        self.shared.failed.store(false, Ordering::Release);
        self.device = Some(device);
        self.config = Some(config);
        self.spec = Some(granted);
        Ok(OutputFormat {
            spec: granted,
            buffer_frames: ASSUMED_CALLBACK_FRAMES,
        })
    }

    fn start(&mut self, render: RenderFn) -> Result<()> {
        if self.stream.is_some() {
            return Err(Error::Output("cpal: already started".into()));
        }
        let (Some(device), Some(config), Some(spec)) =
            (self.device.take(), self.config.take(), self.spec)
        else {
            return Err(Error::Output("cpal: not open".into()));
        };
        let shared = Arc::clone(&self.shared);
        let mut render = render;
        let err_shared = Arc::clone(&self.shared);
        let channels = usize::from(spec.channels.max(1));
        let stream = device
            .build_output_stream(
                config,
                move |buf: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                    render(buf);
                    shared
                        .played
                        .fetch_add((buf.len() / channels) as u64, Ordering::AcqRel);
                },
                move |_e| {
                    err_shared.failed.store(true, Ordering::Release);
                },
                None,
            )
            .map_err(|e| Error::Output(format!("cpal: building the stream: {e}")))?;
        stream
            .play()
            .map_err(|e| Error::Output(format!("cpal: starting the stream: {e}")))?;
        self.stream = Some(stream);
        Ok(())
    }

    fn pause(&mut self) -> Result<()> {
        match self.stream.as_ref() {
            Some(s) => s
                .pause()
                .map_err(|e| Error::Output(format!("cpal: pause: {e}"))),
            None => Ok(()),
        }
    }

    fn resume(&mut self) -> Result<()> {
        match self.stream.as_ref() {
            Some(s) => s
                .play()
                .map_err(|e| Error::Output(format!("cpal: resume: {e}"))),
            None => Ok(()),
        }
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }

    fn latency(&self) -> Duration {
        match self.spec {
            Some(spec) if spec.sample_rate > 0 => {
                spec.duration_of(u64::from(ASSUMED_CALLBACK_FRAMES))
            }
            _ => Duration::ZERO,
        }
    }

    fn played_frames(&self) -> u64 {
        self.shared.played.load(Ordering::Acquire)
    }

    fn close(&mut self) -> Result<()> {
        self.stream = None;
        self.device = None;
        self.config = None;
        self.spec = None;
        if self.shared.failed.swap(false, Ordering::AcqRel) {
            return Err(Error::Output("cpal: the output stream failed".into()));
        }
        Ok(())
    }
}

impl std::fmt::Debug for CpalOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpalOutput")
            .field("device", &self.device_name)
            .field("spec", &self.spec)
            .finish_non_exhaustive()
    }
}
