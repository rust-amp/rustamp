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
//! * Device: the host's default output device, or — with
//!   [`CpalOutput::with_device_name`] — the one output device whose name
//!   contains the given text. A named device that is absent or ambiguous fails
//!   `open`, naming the wanted device and listing the devices that exist; it
//!   never falls back to the default device. A default device that is absent
//!   fails the same way.
//! * The stream's error callback latches a failure flag; `close` reports it
//!   naming the device and [`CpalOutput::has_failed`] exposes it earlier. The
//!   `OutputDevice` contract has no mid-run error hook yet, so the flag cannot
//!   become an `Event::Error` without an RFC.

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
    /// `Some`: open the output device whose name contains this text instead of
    /// the default one.
    wanted: Option<String>,
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
            wanted: None,
            shared: Arc::new(Shared {
                played: AtomicU64::new(0),
                failed: AtomicBool::new(false),
            }),
        }
    }

    /// Not opened yet; [`OutputDevice::open`] opens the one output device
    /// whose name contains `name` (case-insensitive), never the default one.
    ///
    /// `open` fails, naming the wanted device and listing the available
    /// ones, when no output device or more than one matches.
    pub fn with_device_name(name: impl Into<String>) -> Self {
        CpalOutput {
            wanted: Some(name.into()),
            ..Self::new()
        }
    }

    /// Name of the opened device, once open.
    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    /// Whether the stream's error callback has fired since `open` — the device
    /// was lost or the backend failed. `close` reports the same failure; this
    /// lets a caller notice it before closing.
    pub fn has_failed(&self) -> bool {
        self.shared.failed.load(Ordering::Acquire)
    }

    /// The latched fatal stream error, if the device or stream encountered a fatal failure.
    ///
    /// Delegates to the [`OutputDevice::stream_error`] trait method.
    pub fn stream_error(&self) -> Option<Error> {
        <Self as OutputDevice>::stream_error(self)
    }
}

/// The device's advertised name, or an empty string when the backend does not
/// offer one.
fn device_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_default()
}

/// Names of the host's output devices (best effort; a failing enumeration is
/// reported as "none", never as an error).
fn output_device_names(host: &cpal::Host) -> Vec<String> {
    let Ok(devices) = host.output_devices() else {
        return Vec::new();
    };
    devices.map(|d| device_name(&d)).collect()
}

/// Message for a named-device lookup, naming the wanted device and the devices
/// that exist.
fn device_selection_message(wanted: &str, matches: usize, names: &[String]) -> String {
    if names.is_empty() {
        format!("cpal: no output device named like \"{wanted}\"; the host lists no output device")
    } else if matches == 0 {
        format!(
            "cpal: no output device named like \"{wanted}\" (available: {})",
            names.join(" | ")
        )
    } else {
        format!(
            "cpal: {matches} output devices named like \"{wanted}\"; the name is ambiguous \
             (available: {})",
            names.join(" | ")
        )
    }
}

/// Message for a missing default output device, listing what does exist.
fn no_default_device_message(names: &[String]) -> String {
    if names.is_empty() {
        "cpal: no default output device; the host lists no output device".to_owned()
    } else {
        format!(
            "cpal: no default output device (available: {})",
            names.join(" | ")
        )
    }
}

/// The one output device of `host` whose name contains `wanted`
/// (case-insensitive).
fn find_output_device(host: &cpal::Host, wanted: &str) -> Result<cpal::Device> {
    let devices = host
        .output_devices()
        .map_err(|e| Error::Output(format!("cpal: listing output devices: {e}")))?;
    let needle = wanted.to_lowercase();
    let mut names = Vec::new();
    let mut found = Vec::new();
    for device in devices {
        let name = device_name(&device);
        if name.to_lowercase().contains(&needle) {
            found.push(device);
        }
        names.push(name);
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        n => Err(Error::Output(device_selection_message(wanted, n, &names))),
    }
}

/// Best `f32` output configuration for `device`: a configuration whose sample
/// rate equals `want.sample_rate` if the device has one, otherwise the
/// highest-rate candidate. Ties go to the choice with more channels.
///
/// `name` names the device in every error, so an unusable format is a loud
/// failure of `open` instead of a late failure when the stream is built.
fn pick_config(
    device: &cpal::Device,
    want: AudioSpec,
    name: &str,
) -> Result<(cpal::StreamConfig, AudioSpec)> {
    let supported = device
        .supported_output_configs()
        .map_err(|e| Error::Output(format!("cpal: querying the formats of \"{name}\": {e}")))?;
    let mut best: Option<(cpal::StreamConfig, AudioSpec)> = None;
    let mut best_exact = false;
    let mut advertised = Vec::new();
    for range in supported {
        let format = range.sample_format();
        advertised.push(format!(
            "{format:?} {}ch {}..={}Hz",
            range.channels(),
            range.min_sample_rate(),
            range.max_sample_rate()
        ));
        if format != cpal::SampleFormat::F32 {
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
    // No explicit `f32` range advertised: fall back to the device default, but
    // only when the default really is `f32`; otherwise the request is
    // impossible and must fail here, not when the stream is built.
    let default = device
        .default_output_config()
        .map_err(|e| Error::Output(format!("cpal: default config of \"{name}\": {e}")))?;
    if default.sample_format() != cpal::SampleFormat::F32 {
        return Err(Error::Output(format!(
            "cpal: \"{name}\" grants no f32 output format (default: {:?}; advertised: {})",
            default.sample_format(),
            if advertised.is_empty() {
                "none".to_owned()
            } else {
                advertised.join(" | ")
            }
        )));
    }
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
        if spec.sample_rate == 0 {
            return Err(Error::InvalidArgument("cpal: sample rate 0".into()));
        }
        if spec.channels == 0 {
            return Err(Error::InvalidArgument("cpal: channel count 0".into()));
        }
        if self.stream.is_some() {
            // A stream keeps running with its own device and config; reopening
            // underneath it would leave two configurations in one object.
            return Err(Error::Output(
                "cpal: already started; close the device before reopening".into(),
            ));
        }
        let host = cpal::default_host();
        let device = match &self.wanted {
            Some(wanted) => find_output_device(&host, wanted)?,
            None => host.default_output_device().ok_or_else(|| {
                Error::Output(no_default_device_message(&output_device_names(&host)))
            })?,
        };
        self.device_name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| "default output device".into());
        let (config, granted) = pick_config(&device, spec, &self.device_name)?;
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
        let stream = match device.build_output_stream(
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
        ) {
            Ok(stream) => stream,
            Err(e) => {
                // Give the device and its config back: a failed start must not
                // wedge the object between "open" and "not open".
                self.device = Some(device);
                self.config = Some(config);
                return Err(Error::Output(format!(
                    "cpal: building the stream on \"{}\": {e}",
                    self.device_name
                )));
            }
        };
        if let Err(e) = stream.play() {
            self.device = Some(device);
            self.config = Some(config);
            return Err(Error::Output(format!(
                "cpal: starting the stream on \"{}\": {e}",
                self.device_name
            )));
        }
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

    fn stream_error(&self) -> Option<Error> {
        if self.shared.failed.load(Ordering::Acquire) {
            Some(Error::Output(format!(
                "{}: audio stream encountered a fatal error or disconnected",
                self.device_name
            )))
        } else {
            None
        }
    }

    fn close(&mut self) -> Result<()> {
        self.stream = None;
        self.device = None;
        self.config = None;
        self.spec = None;
        if self.shared.failed.swap(false, Ordering::AcqRel) {
            return Err(Error::Output(format!(
                "cpal: the output stream on \"{}\" failed",
                self.device_name
            )));
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

#[cfg(test)]
mod tests {
    use super::*;

    // These tests exercise only pure helpers: they open no device and start no
    // audio, so they are safe on any machine.

    #[test]
    fn a_named_device_that_matches_nothing_lists_the_available_devices() {
        let names = vec!["Speakers (Realtek)".to_owned(), "Headphones".to_owned()];
        let message = device_selection_message("CABLE Input", 0, &names);
        assert!(message.contains("CABLE Input"), "{message}");
        assert!(message.contains("Realtek"), "{message}");
        assert!(message.contains("Headphones"), "{message}");
    }

    #[test]
    fn an_ambiguous_named_device_says_how_many_and_which() {
        let names = vec!["CABLE Input".to_owned(), "CABLE Output".to_owned()];
        let message = device_selection_message("cable", 2, &names);
        assert!(message.contains('2'), "{message}");
        assert!(message.contains("ambiguous"), "{message}");
        assert!(message.contains("CABLE Input"), "{message}");
    }

    #[test]
    fn a_named_device_lookup_with_no_devices_says_so() {
        let message = device_selection_message("anything", 0, &[]);
        assert!(message.contains("anything"), "{message}");
        assert!(message.contains("no output device"), "{message}");
    }

    #[test]
    fn a_missing_default_device_lists_the_alternatives() {
        let names = vec!["Speakers".to_owned()];
        let message = no_default_device_message(&names);
        assert!(message.contains("no default output device"), "{message}");
        assert!(message.contains("Speakers"), "{message}");
        let none = no_default_device_message(&[]);
        assert!(none.contains("lists no output device"), "{none}");
    }

    #[test]
    fn open_rejects_an_impossible_format_before_touching_a_device() {
        let mut out = CpalOutput::new();
        assert!(matches!(
            out.open(AudioSpec::new(0, 2)),
            Err(Error::InvalidArgument(_))
        ));
        assert!(matches!(
            out.open(AudioSpec::new(44100, 0)),
            Err(Error::InvalidArgument(_))
        ));
        assert!(!out.has_failed());
        assert!(out.stream_error().is_none());
    }

    #[test]
    fn cpal_stream_error_reflects_error_callback() {
        let out = CpalOutput::new();
        assert!(OutputDevice::stream_error(&out).is_none());
        assert!(!out.has_failed());
        out.shared.failed.store(true, Ordering::Release);
        assert!(out.has_failed());
        let err = OutputDevice::stream_error(&out).expect("stream error");
        let msg = err.to_string();
        assert!(msg.contains("cpal"), "{msg}");
        assert!(msg.contains("fatal error or disconnected"), "{msg}");
    }
}
