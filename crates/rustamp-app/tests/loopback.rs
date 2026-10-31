// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Loopback verification: the decoded samples really reach the audio device.
//! **VM-only, `#[ignore]`d.**
//!
//! `real_playback.rs` proves the device opens, the clock advances and the ring
//! drains, but not what the samples *are*. This test closes that gap with the
//! VB-Audio Virtual Cable installed in the headless test VM: its render
//! endpoint ("CABLE Input") is permanently wired to its capture endpoint
//! ("CABLE Output"). The engine plays the repo's 1.5 s, 440 Hz FLAC fixture
//! to "CABLE Input" — selected **by name**, never the system default — while
//! the test records "CABLE Output" at the same time, and the recording must
//! contain the fixture's tone.
//!
//! ## Step by step
//!
//! 1. The VM gate, before any device exists.
//! 2. Both cable endpoints are opened by name; a missing one fails loudly,
//!    naming the device (never a skip, never a vacuous pass). The opened
//!    playback device must be the cable, not the default device.
//! 3. The capture stream starts first and records a silent **pre-roll**
//!    before playback starts (see "Synchronization").
//! 4. The engine plays the fixture to its end (`Playing` → `Stopped`, ring
//!    drained), and the capture keeps recording a tail after it. The cpal
//!    endpoint-buffer flush must report its documented terminal refusal;
//!    the caller recreates the engine/output only after capture has stopped.
//! 5. The recording is analysed in 50 ms windows (see "Metric"): the pre-roll
//!    holds no tone; after the play mark the tone appears within a latency
//!    bound, lasts as long as the fixture without a dropout, does not last
//!    longer, and arrives at a plausible level on every compared channel.
//!
//! ## Synchronization (record vs. play)
//!
//! The capture stream is started and must deliver [`PRE_ROLL`] of audio
//! *before* the engine is told to play. That makes the capture provably live
//! before the first sample is rendered, so the tone's onset cannot be lost,
//! and it splits the recording at a known index (the "play mark": the frames
//! captured when `play` was called). The metric is alignment-free, so no
//! sample-exact sync is needed; the play mark only proves causality — no
//! tone before it, the tone within [`MAX_ONSET`] after it. After the engine
//! reports `Stopped` the capture records [`TAIL`] more, which covers the
//! render and capture latency, so the tone's end is inside the recording.
//!
//! ## Metric and tolerance
//!
//! Tone detection with a Goertzel filter at 440 Hz over consecutive 50 ms
//! windows, per channel. For each window it yields the 440 Hz amplitude and
//! the share of the window's power that sits at 440 Hz (`ratio`, 1.0 for a
//! pure tone). Why this metric:
//!
//! * It needs no alignment (cross-correlation would need the device latency)
//!   and no reference decode, and it is independent of the endpoint volume
//!   (an RMS envelope comparison is not, and cannot tell a tone from noise).
//! * 50 ms is exactly 22 cycles of 440 Hz at *any* sample rate, so a clean
//!   tone fills its bin with no leakage whatever rate the cable runs at.
//! * It fails on silence (amplitude below the floor), on garbage or noise
//!   (power spread across the spectrum: `ratio` ≈ 2/N ≈ 0.001), on a wrong
//!   frequency, including a sample-rate mismatch (48 kHz played as 44.1 kHz
//!   is 404 Hz, almost two 20 Hz bins away: `ratio` ≈ 0), and on dropouts or
//!   glitches (a broken window falls below the `ratio` threshold and breaks
//!   the contiguous run).
//!
//! A window counts as *tonal* when `ratio >= 0.9` and its amplitude is at
//! least [`MIN_AMPLITUDE`]. A clean digital cable gives `ratio` ≈ 1.0 (16-bit
//! quantization noise sits ~80 dB below the tone), so 0.9 passes
//! deterministically yet rejects anything that is not a dominant 440 Hz tone.
//! The assertions: at least [`MIN_RUN_SECONDS`] of contiguous tonal windows
//! (1.2 of the fixture's 1.5 s: the onset and the end each cost at most one
//! straddling window, the rest is margin), at most [`MAX_TONE_SECONDS`] of
//! tonal windows in total (the fixture plus its two straddling windows: a
//! looping or stuck buffer would play longer), no tonal window in the
//! pre-roll, and every tonal amplitude inside
//! [[`MIN_AMPLITUDE`], [`MAX_AMPLITUDE`]] (the fixture is ffmpeg's default
//! 1/8 amplitude; the floor tolerates an endpoint volume down to -20 dB, the
//! ceiling catches a gain or conversion bug).
//!
//! ## Format negotiation
//!
//! Playback goes through the production `CpalOutput` (by name). The engine
//! requests its default 44.1 kHz stereo; `CpalOutput` grants the closest
//! `f32` configuration the cable offers and the engine converts to it. The
//! test accepts any granted rate and 1–2 channels. Capture requests `f32` at
//! the playback's granted rate and accepts any rate and channel count the
//! cable offers in `f32` or `i16` (converted to `f32`), preferring `f32` and
//! the matching rate; the analysis uses the rate actually granted. It
//! compares the first `min(playback, capture)` channels, at most two.
//!
//! ## What this test cannot prove
//!
//! * The production *default* path: `default_output()` opens the default
//!   endpoint through WASAPI, which cannot select a device by name; this test
//!   uses the `cpal` fallback, which shares the engine, ring, gain and
//!   converter but not the WASAPI render loop. `real_playback.rs` covers that
//!   loop's clock and drain.
//! * Bit exactness: the endpoint volume, the Windows mixer and any resampling
//!   in the shared-mode path are allowed to change samples within tolerance.
//! * That a speaker plays it: the cable stops at a capture endpoint.
//! * Channel order and separation: the fixture carries the same tone on both
//!   channels, so swapped channels would pass.
//! * Gapless joins, seeking and other content: one track, one tone.
//!
//! The VB-Audio cable is not connected to any speaker, so this test plays
//! **no audible sound**. It opens **no window**: the engine is driven
//! in-process. Run it only inside the headless test VM:
//!
//! ```text
//! cargo test -p rustamp-app --test loopback -- --ignored
//! ```

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rustamp_audio::{Engine, EngineConfig, TrackSource};
use rustamp_core::bus::EventBus;
use rustamp_core::error::{Error, ErrorInfo, ErrorKind};
use rustamp_core::state::TransportState;
use rustamp_output::CpalOutput;

/// The cable's render endpoint: playback target. Matched as a
/// case-insensitive substring of the device name; it must match exactly one
/// device.
const PLAYBACK_DEVICE: &str = "CABLE Input";

/// The cable's capture endpoint, wired to [`PLAYBACK_DEVICE`].
const CAPTURE_DEVICE: &str = "CABLE Output";

/// Locale-proof needle for both cable endpoints: the OS names the render
/// endpoint per UI language (English "CABLE Input", German "Lautsprecher
/// (VB-Audio Virtual Cable)"), but every name contains "cable". Both device
/// lists are searched separately (outputs vs inputs) and the exactly-one-match
/// discipline stays: none or several matches fail loudly, never a default.
const CABLE_MATCH: &str = "cable";

/// Whether a device name belongs to the virtual cable, in any OS language.
/// Pure function so the locale rule is pinnable without hardware.
fn is_cable_device(name: &str) -> bool {
    name.to_lowercase().contains(CABLE_MATCH)
}

#[test]
fn cable_matching_survives_os_locales() {
    // English render/capture names and the German render name seen in the
    // headless test VM, plus decoys that must never match.
    for name in [
        "CABLE Input",
        "CABLE Output (VB-Audio Virtual Cable)",
        "Lautsprecher (VB-Audio Virtual Cable)",
        "CABLE Output",
    ] {
        assert!(is_cable_device(name), "{name:?} must match");
    }
    for name in [
        "Realtek High Definition Audio",
        "Microphone Array (Realtek)",
        "Speakers",
        "",
    ] {
        assert!(!is_cable_device(name), "{name:?} must not match");
    }
}

/// The fixture's tone (see `crates/rustamp-decoders/tests/fixtures/FIXTURES.txt`).
const TONE_HZ: f64 = 440.0;

/// The fixture's length.
const FIXTURE_SECONDS: f64 = 1.5;

/// The fixture's peak amplitude: ffmpeg's `sine` source defaults to 1/8.
const FIXTURE_AMPLITUDE: f64 = 0.125;

/// Analysis window: 22 whole cycles of [`TONE_HZ`], at any sample rate.
const WINDOW_SECONDS: f64 = 0.05;

/// Share of a window's power that must sit at [`TONE_HZ`] for it to be tonal.
const MIN_TONE_RATIO: f64 = 0.9;

/// Lowest accepted tone amplitude: the fixture at -20 dB.
const MIN_AMPLITUDE: f64 = FIXTURE_AMPLITUDE * 0.1;

/// Highest accepted tone amplitude: the fixture at +6 dB.
const MAX_AMPLITUDE: f64 = FIXTURE_AMPLITUDE * 2.0;

/// Shortest accepted contiguous run of tonal windows.
const MIN_RUN_SECONDS: f64 = 1.2;

/// Longest accepted total of tonal windows: the fixture plus two windows.
const MAX_TONE_SECONDS: f64 = FIXTURE_SECONDS + 2.0 * WINDOW_SECONDS;

/// Audio the capture must deliver before playback starts.
const PRE_ROLL: Duration = Duration::from_millis(250);

/// Latest accepted tone onset after the play mark.
const MAX_ONSET: Duration = Duration::from_secs(1);

/// Capture kept running after the engine stopped (render + capture latency).
const TAIL: Duration = Duration::from_millis(500);

/// How long each phase may take on a loaded VM.
const DEADLINE: Duration = Duration::from_secs(30);

/// Pump interval while waiting.
const PUMP_INTERVAL: Duration = Duration::from_millis(10);

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("rustamp-decoders")
        .join("tests")
        .join("fixtures")
        .join("sine.flac")
}

/// Buffer duration requested for the capture stream.
///
/// In a virtual machine (Hyper-V headless guest), the default 10 ms WASAPI
/// endpoint buffer leaves no cushion against scheduler jitter or preemption.
/// A 100 ms buffer proved still too tight: under heavy test-suite load, VM
/// preemption spikes exceeding 100 ms triggered a capture ring overrun (xrun)
/// that dropped a packet at frame 68355, breaking the continuous tone into
/// 23- and 4-window fragments. Requesting 500 ms gives WASAPI 50 device
/// periods of capture headroom, absorbing virtualization scheduling jitter
/// without dropping frames or breaking the contiguous tone.
const CAPTURE_BUFFER: Duration = Duration::from_millis(500);

/// Bounded tolerance for transient buffer underruns/overruns (xruns) on the
/// virtual cable under VM load.
///
/// A virtual device has no physical clock and is subject to synchronization
/// glitches when endpoints open or under hypervisor scheduling load. A single
/// isolated xrun is tolerated, but persistent xruns (> 1) fail the test.
/// Silence, noise, dropouts and level anomalies are independently rejected
/// by the Goertzel tone analysis assertions.
const MAX_TOLERATED_XRUNS: usize = 1;

/// What the capture callback has recorded so far (interleaved).
#[derive(Default)]
struct Recording {
    samples: Vec<f32>,
    /// Fatal stream errors that stop capture or indicate permanent device failure.
    fatal_error: Option<String>,
    /// Count of buffer underrun/overrun occurrences reported by the stream.
    xrun_count: usize,
    /// First xrun message, for diagnostic reporting if the threshold is exceeded.
    first_xrun: Option<String>,
}

/// The one input device whose name contains `wanted` (case-insensitive).
/// Panics, naming the device and listing the available ones, otherwise.
fn find_input_device(host: &cpal::Host, wanted: &str) -> cpal::Device {
    let devices = host
        .input_devices()
        .unwrap_or_else(|e| panic!("listing capture devices while looking for \"{wanted}\": {e}"));
    let needle = wanted.to_lowercase();
    let mut names = Vec::new();
    let mut found = Vec::new();
    for device in devices {
        let name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_default();
        if name.to_lowercase().contains(&needle) {
            found.push(device);
        }
        names.push(name);
    }
    match found.len() {
        1 => found.remove(0),
        0 => panic!(
            "MISSING CABLE: no capture endpoint named like \"{wanted}\" (available: {names:?}). \
             This test needs the VB-Audio Virtual Cable in the headless test VM. It fails \
             loudly by design: without the cable there is nothing to verify."
        ),
        n => panic!(
            "{n} capture endpoints are named like \"{wanted}\" (available: {names:?}); the \
             name is ambiguous, so the test cannot know which one is wired to the cable"
        ),
    }
}

/// The capture configuration: `f32` preferred over `i16`, then the rate
/// `want_rate`, then more channels. Any other sample format is refused.
fn pick_capture_config(
    device: &cpal::Device,
    want_rate: u32,
) -> (cpal::StreamConfig, cpal::SampleFormat) {
    let usable =
        |f: cpal::SampleFormat| matches!(f, cpal::SampleFormat::F32 | cpal::SampleFormat::I16);
    let mut best: Option<((bool, bool, u16), cpal::StreamConfig, cpal::SampleFormat)> = None;
    let mut offered = Vec::new();
    if let Ok(ranges) = device.supported_input_configs() {
        for range in ranges {
            offered.push(format!(
                "{:?} {}-{} Hz {} ch",
                range.sample_format(),
                range.min_sample_rate(),
                range.max_sample_rate(),
                range.channels()
            ));
            if !usable(range.sample_format()) {
                continue;
            }
            let rate = want_rate.clamp(range.min_sample_rate(), range.max_sample_rate());
            let score = (
                range.sample_format() == cpal::SampleFormat::F32,
                rate == want_rate,
                range.channels(),
            );
            if best.as_ref().is_none_or(|(s, _, _)| score > *s) {
                let buffer_frames =
                    ((f64::from(rate) * CAPTURE_BUFFER.as_secs_f64()).round() as u32).max(1);
                let config = cpal::StreamConfig {
                    channels: range.channels(),
                    sample_rate: rate,
                    buffer_size: cpal::BufferSize::Fixed(buffer_frames),
                };
                best = Some((score, config, range.sample_format()));
            }
        }
    }
    if let Some((_, config, format)) = best {
        return (config, format);
    }
    let default = device.default_input_config().unwrap_or_else(|e| {
        panic!("\"{CAPTURE_DEVICE}\" offers no capture configuration ({e}); offered: {offered:?}")
    });
    assert!(
        usable(default.sample_format()),
        "\"{CAPTURE_DEVICE}\" captures only {:?}; the test accepts f32 or i16 (offered: {offered:?})",
        default.sample_format()
    );
    let format = default.sample_format();
    let mut config: cpal::StreamConfig = default.into();
    let buffer_frames =
        ((f64::from(config.sample_rate) * CAPTURE_BUFFER.as_secs_f64()).round() as u32).max(1);
    config.buffer_size = cpal::BufferSize::Fixed(buffer_frames);
    (config, format)
}

/// Starts recording `device` into `recording`.
fn start_capture(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    format: cpal::SampleFormat,
    recording: &Arc<Mutex<Recording>>,
) -> cpal::Stream {
    let on_error = {
        let recording = Arc::clone(recording);
        move |e: cpal::Error| {
            if let Ok(mut r) = recording.lock() {
                if e.kind() == cpal::ErrorKind::Xrun {
                    r.xrun_count += 1;
                    if r.first_xrun.is_none() {
                        r.first_xrun = Some(e.to_string());
                    }
                    let count = r.xrun_count;
                    if count > MAX_TOLERATED_XRUNS {
                        r.fatal_error.get_or_insert_with(|| {
                            format!(
                                "too many buffer underrun/overruns ({count} > {MAX_TOLERATED_XRUNS}): {e}"
                            )
                        });
                    }
                } else {
                    r.fatal_error.get_or_insert_with(|| e.to_string());
                }
            }
        }
    };
    let sink = Arc::clone(recording);
    let built = match format {
        cpal::SampleFormat::F32 => device.build_input_stream(
            config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                if let Ok(mut r) = sink.lock() {
                    r.samples.extend_from_slice(data);
                }
            },
            on_error,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            config,
            move |data: &[i16], _: &cpal::InputCallbackInfo| {
                if let Ok(mut r) = sink.lock() {
                    r.samples
                        .extend(data.iter().map(|&s| f32::from(s) / 32768.0));
                }
            },
            on_error,
            None,
        ),
        other => unreachable!("pick_capture_config never picks {other:?}"),
    };
    let stream = built
        .unwrap_or_else(|e| panic!("building the capture stream on \"{CAPTURE_DEVICE}\": {e}"));
    stream
        .play()
        .unwrap_or_else(|e| panic!("starting the capture stream on \"{CAPTURE_DEVICE}\": {e}"));
    stream
}

/// Frames recorded so far.
fn recorded_frames(recording: &Mutex<Recording>, channels: usize) -> usize {
    recording.lock().map(|r| r.samples.len()).unwrap_or(0) / channels
}

/// The 440 Hz content of one analysis window.
#[derive(Debug, Clone, Copy)]
struct ToneWindow {
    /// First frame of the window in the recording.
    start: usize,
    /// Amplitude of the [`TONE_HZ`] component.
    amplitude: f64,
    /// Share of the window's power at [`TONE_HZ`]: 1.0 for a pure tone.
    ratio: f64,
}

impl ToneWindow {
    fn is_tonal(&self) -> bool {
        self.ratio >= MIN_TONE_RATIO && self.amplitude >= MIN_AMPLITUDE
    }
}

/// Goertzel detection of `freq` in `x` (one channel, `rate` Hz).
///
/// Returns the amplitude of the `freq` component (`2 |X(freq)| / N`) and its
/// share of the window's mean power (`(A² / 2) / mean(x²)`), which is 1.0 for
/// a pure sinusoid filling whole cycles and ≈ 2/N for white noise. An
/// all-zero window yields `(0, 0)`.
fn tone_in_window(x: &[f32], rate: f64, freq: f64) -> (f64, f64) {
    let n = x.len() as f64;
    if x.is_empty() {
        return (0.0, 0.0);
    }
    let coeff = 2.0 * (2.0 * std::f64::consts::PI * freq / rate).cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    let mut energy = 0.0f64;
    for &v in x {
        let v = f64::from(v);
        let s = v + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
        energy += v * v;
    }
    let power = (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0);
    let amplitude = 2.0 * power.sqrt() / n;
    let mean_power = energy / n;
    let ratio = if mean_power > 0.0 {
        (amplitude * amplitude / 2.0) / mean_power
    } else {
        0.0
    };
    (amplitude, ratio)
}

/// Channel `channel` of the interleaved `samples`, cut into consecutive
/// [`WINDOW_SECONDS`] windows (a trailing partial window is dropped).
fn analyse(samples: &[f32], channels: usize, channel: usize, rate: u32) -> Vec<ToneWindow> {
    let mono: Vec<f32> = samples
        .chunks_exact(channels)
        .map(|frame| frame[channel])
        .collect();
    let len = (f64::from(rate) * WINDOW_SECONDS).round() as usize;
    mono.chunks_exact(len.max(1))
        .enumerate()
        .map(|(i, window)| {
            let (amplitude, ratio) = tone_in_window(window, f64::from(rate), TONE_HZ);
            ToneWindow {
                start: i * len,
                amplitude,
                ratio,
            }
        })
        .collect()
}

/// One line per window, for failure messages.
fn describe(windows: &[ToneWindow]) -> String {
    windows
        .iter()
        .map(|w| {
            format!(
                "{}{}: a={:.4} r={:.3}",
                if w.is_tonal() { "*" } else { " " },
                w.start,
                w.amplitude,
                w.ratio
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Plays the fixture into the cable and finds its tone in the recording.
///
/// Honest by construction: with no cable both lookups fail; with silence,
/// noise, a wrong frequency or rate, a dropout, or a tone that was already
/// there before playback, an assertion below fails.
#[test]
#[ignore = "VM-only: needs the VB-Audio Virtual Cable; run inside the headless test VM"]
fn the_cable_carries_the_fixture_tone() {
    // The gate, before any device exists.
    rustamp_app::window::require_test_vm();

    let fixture = fixture();
    assert!(
        fixture.exists(),
        "missing fixture {} (generated with ffmpeg; see crates/rustamp-decoders/tests/fixtures/FIXTURES.txt)",
        fixture.display()
    );

    // Playback endpoint, by cable name and never the default device: the OS
    // names it per UI language, so match the locale-proof needle instead.
    let output = Box::new(CpalOutput::with_device_name(CABLE_MATCH));
    let mut engine = match Engine::new(
        rustamp_decoders::default_registry(),
        output,
        EventBus::new(),
        EngineConfig::default(),
    ) {
        Ok(engine) => engine,
        Err(error) => panic!(
            "MISSING CABLE: the playback endpoint \"{PLAYBACK_DEVICE}\" could not be opened \
             ({error}). This test needs the VB-Audio Virtual Cable in the headless test VM. \
             It fails loudly by design: without the cable there is nothing to verify."
        ),
    };
    let name = engine.output_name().to_owned();
    assert!(
        is_cable_device(&name),
        "the engine opened \"{name}\", not the cable (matched by \"{CABLE_MATCH}\")"
    );
    let granted = engine.device_spec();
    assert!(
        granted.sample_rate > 0 && (1..=2).contains(&granted.channels),
        "\"{name}\" granted an unusable format: {} Hz, {} ch",
        granted.sample_rate,
        granted.channels
    );

    // Capture endpoint, by cable name, started first.
    let host = cpal::default_host();
    let capture = find_input_device(&host, CABLE_MATCH);
    let (config, format) = pick_capture_config(&capture, granted.sample_rate);
    let rate = config.sample_rate;
    let channels = usize::from(config.channels);
    assert!(
        rate > 0 && channels > 0,
        "\"{CAPTURE_DEVICE}\" offers an unusable format: {rate} Hz, {channels} ch"
    );
    let recording = Arc::new(Mutex::new(Recording {
        samples: Vec::with_capacity(rate as usize * channels * 10),
        ..Default::default()
    }));
    let stream = start_capture(&capture, config, format, &recording);

    // Pre-roll: the capture is live before the first sample is rendered.
    let pre_roll_frames = (f64::from(rate) * PRE_ROLL.as_secs_f64()) as usize;
    let start = Instant::now();
    while recorded_frames(&recording, channels) < pre_roll_frames {
        assert!(
            start.elapsed() < DEADLINE,
            "\"{CAPTURE_DEVICE}\" delivered {} of {pre_roll_frames} pre-roll frames within \
             {DEADLINE:?}: the capture stream is not running",
            recorded_frames(&recording, channels)
        );
        std::thread::sleep(PUMP_INTERVAL);
    }
    let play_mark = recorded_frames(&recording, channels);

    // Play the fixture to its end.
    engine.set_queue(vec![TrackSource::path(fixture.clone())]);
    engine
        .play_index(0)
        .expect("starting playback on the cable");
    assert_eq!(engine.transport(), TransportState::Playing);
    let start = Instant::now();
    while engine.transport() != TransportState::Stopped {
        engine
            .poll()
            .expect("polling the engine while the cable plays");
        assert!(
            start.elapsed() < DEADLINE,
            "\"{name}\" never finished the 1.5 s fixture within {DEADLINE:?} (transport {:?}, \
             played {} frames, last error {:?})",
            engine.transport(),
            engine.played_frames(),
            engine.last_error()
        );
        std::thread::sleep(PUMP_INTERVAL);
    }
    engine.poll().expect("draining the final events");
    assert!(engine.played_frames() > 0, "\"{name}\" consumed no frames");
    assert_eq!(
        engine.buffered_frames(),
        0,
        "the ring still holds frames: they were decoded but never rendered"
    );

    // Tail: the tone's end crosses the render and capture latency.
    // Wait until the required frames (pre-roll + fixture + tail) have actually
    // arrived, without using a blind thread sleep as an ordering mechanism.
    let fixture_frames = (f64::from(rate) * FIXTURE_SECONDS) as usize;
    let tail_frames = (f64::from(rate) * TAIL.as_secs_f64()) as usize;
    let target_frames = play_mark + fixture_frames + tail_frames;
    let start = Instant::now();
    while recorded_frames(&recording, channels) < target_frames {
        assert!(
            start.elapsed() < DEADLINE,
            "\"{CAPTURE_DEVICE}\" delivered {} of {target_frames} frames within {DEADLINE:?}: \
             the capture stream stalled during tail",
            recorded_frames(&recording, channels)
        );
        std::thread::sleep(PUMP_INTERVAL);
    }

    // Stop and drop the capture stream now that all audio has been captured.
    // Dropping the capture stream before recreating the engine avoids driver
    // and COM contention on the virtual cable while capture is in flight.
    drop(stream);

    // This test deliberately uses CpalOutput, whose flush always refuses.
    // Require that exact refusal, not an absent error, a different unsupported
    // operation, a decoder/stream failure, or a failure to close the output.
    let expected_flush_message = format!(
        "{name}: output flush failed: unsupported operation: cpal endpoint-buffer flush; \
         close and recreate the output stream instead; output closed; \
         recreate the engine/output before playback"
    );
    let expected_flush_error = ErrorInfo {
        kind: ErrorKind::Output,
        message: format!("output error: {expected_flush_message}"),
    };
    assert_eq!(
        engine.last_error(),
        Some(&expected_flush_error),
        "natural completion must report precisely the cpal flush refusal"
    );
    assert!(
        engine.is_flush_failed(),
        "a refused flush must latch the closed output's terminal failure"
    );
    assert_eq!(engine.transport(), TransportState::Stopped);

    // The audio contract (OutputDevice::flush in rustamp-core/src/audio.rs) requires
    // the caller (the application or test) to recreate the engine and output
    // before playback can resume. Exercise that documented path honestly:
    let output = Box::new(CpalOutput::with_device_name(CABLE_MATCH));
    let recreated_engine = match Engine::new(
        rustamp_decoders::default_registry(),
        output,
        EventBus::new(),
        EngineConfig::default(),
    ) {
        Ok(engine) => engine,
        Err(error) => panic!(
            "MISSING CABLE: the playback endpoint \"{PLAYBACK_DEVICE}\" could not be reopened \
             ({error})"
        ),
    };
    assert!(
        recreated_engine.last_error().is_none(),
        "the freshly recreated engine has an error: {:?}",
        recreated_engine.last_error()
    );
    assert!(
        !recreated_engine.is_flush_failed(),
        "caller recreation must provide a fresh output without the terminal latch"
    );
    assert!(
        is_cable_device(recreated_engine.output_name()),
        "caller recreation opened {:?}, not the cable",
        recreated_engine.output_name()
    );
    assert_eq!(recreated_engine.transport(), TransportState::Stopped);

    // Verify capture health: fatal stream errors fail the test, and xruns
    // are bounded by MAX_TOLERATED_XRUNS.
    let recording = std::mem::take(&mut *recording.lock().expect("the capture lock"));
    if let Some(error) = &recording.fatal_error {
        panic!("the capture stream on \"{CAPTURE_DEVICE}\" failed: {error}");
    }
    if recording.xrun_count > MAX_TOLERATED_XRUNS {
        panic!(
            "the capture stream on \"{CAPTURE_DEVICE}\" reported {} buffer underrun/overruns \
             (exceeded tolerance limit of {MAX_TOLERATED_XRUNS}): {:?}",
            recording.xrun_count, recording.first_xrun
        );
    }
    let total_frames = recording.samples.len() / channels;
    assert!(
        total_frames >= target_frames,
        "recorded {total_frames} frames, fewer than the expected target ({target_frames}): \
         the capture stalled"
    );

    // The comparison, per channel.
    let window_frames = (f64::from(rate) * WINDOW_SECONDS).round() as usize;
    let max_onset_frames = (f64::from(rate) * MAX_ONSET.as_secs_f64()) as usize;
    let min_run = (MIN_RUN_SECONDS / WINDOW_SECONDS).round() as usize;
    let max_tonal = (MAX_TONE_SECONDS / WINDOW_SECONDS).round() as usize;
    let compared = channels.min(usize::from(granted.channels)).min(2);
    for channel in 0..compared {
        let windows = analyse(&recording.samples, channels, channel, rate);
        let context = || {
            format!(
                "channel {channel} of \"{CAPTURE_DEVICE}\" @ {rate} Hz/{channels} ch \
                 ({format:?}), playback \"{name}\" @ {} Hz/{} ch, play mark at frame \
                 {play_mark}, windows of {window_frames} frames (* = tonal):\n{}",
                granted.sample_rate,
                granted.channels,
                describe(&windows)
            )
        };

        // Nothing at 440 Hz before playback started.
        let (before, after): (Vec<ToneWindow>, Vec<ToneWindow>) = windows
            .iter()
            .partition(|w| w.start + window_frames <= play_mark);
        assert!(
            before.len() >= 4,
            "only {} pre-roll windows: the pre-roll cannot prove the cable was quiet; {}",
            before.len(),
            context()
        );
        assert!(
            !before.iter().any(ToneWindow::is_tonal),
            "the cable carried a 440 Hz tone before playback started, so the recording \
             cannot be attributed to this playback; {}",
            context()
        );

        // The tone starts within the latency bound after the play mark.
        let first = after.iter().find(|w| w.is_tonal()).unwrap_or_else(|| {
            panic!(
                "no 440 Hz tone in the recording: the decoded samples did not reach the \
                 cable (silence, noise or a wrong frequency/rate); {}",
                context()
            )
        });
        assert!(
            first.start <= play_mark + max_onset_frames,
            "the tone started {} frames after the play mark, later than {MAX_ONSET:?}; {}",
            first.start - play_mark.min(first.start),
            context()
        );

        // It lasts as long as the fixture, without a dropout, and no longer.
        let longest_run = after
            .iter()
            .fold((0usize, 0usize), |(run, best), w| {
                let run = if w.is_tonal() { run + 1 } else { 0 };
                (run, best.max(run))
            })
            .1;
        assert!(
            longest_run >= min_run,
            "the longest uninterrupted tone is {longest_run} windows, fewer than {min_run} \
             ({MIN_RUN_SECONDS} s of the {FIXTURE_SECONDS} s fixture): a dropout or a \
             truncated playback; {}",
            context()
        );
        let tonal = after.iter().filter(|w| w.is_tonal()).count();
        assert!(
            tonal <= max_tonal,
            "{tonal} tonal windows, more than {max_tonal} ({MAX_TONE_SECONDS} s): the tone \
             lasted longer than the fixture; {}",
            context()
        );

        // At a plausible level.
        for w in after.iter().filter(|w| w.is_tonal()) {
            assert!(
                w.amplitude <= MAX_AMPLITUDE,
                "tone amplitude {:.4} at frame {} exceeds {MAX_AMPLITUDE} (fixture \
                 {FIXTURE_AMPLITUDE}): a gain or conversion bug; {}",
                w.amplitude,
                w.start,
                context()
            );
        }
    }

    // The old engine is still terminal even after caller-side recreation.
    // Stop must return the same refusal, not silently succeed or replace it
    // with an unrelated error. No further playback uses this closed output.
    let error = engine
        .stop()
        .expect_err("stopping the old engine must retain its terminal flush refusal");
    match error {
        Error::Output(message) => assert_eq!(
            message, expected_flush_message,
            "stop must return the latched cpal flush refusal unchanged"
        ),
        other => panic!("stop returned {other:?}, not the documented output flush refusal"),
    }
    assert!(engine.is_flush_failed(), "stop must not clear the refusal");
    assert_eq!(engine.last_error(), Some(&expected_flush_error));
    assert_eq!(engine.transport(), TransportState::Stopped);
}

// ----- the equalizer, measured on the cable -----

/// The tone's amplitude in the generated file: leaves room for a 12 dB boost.
const EQ_TONE_AMPLITUDE: f64 = 0.05;

/// How long each generated tone plays.
const EQ_TONE_SECONDS: f64 = 1.0;

/// Analysis window for the equalizer: whole cycles of every band's centre.
const EQ_WINDOW_SECONDS: f64 = 0.1;

/// Writes `seconds` of a sine at `hz` as a 16-bit stereo WAV file.
fn write_tone(path: &std::path::Path, hz: f64, seconds: f64) {
    const RATE: u32 = 44_100;
    let frames = (f64::from(RATE) * seconds) as u32;
    let data_len = frames * 4;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for frame in 0..frames {
        let phase = 2.0 * std::f64::consts::PI * hz * f64::from(frame) / f64::from(RATE);
        let sample = (phase.sin() * EQ_TONE_AMPLITUDE * 32767.0) as i16;
        bytes.extend_from_slice(&sample.to_le_bytes());
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes).expect("writing the generated tone");
}

/// Plays `track` into the cable with `eq` applied and returns the median
/// amplitude of the tone at `hz` in the recording, on the first channel.
///
/// The median of the windows that hold the tone: the onset and the end straddle
/// a window, and they must not move the level.
fn measured_level(track: &std::path::Path, hz: f64, eq: &[rustamp_core::command::Command]) -> f64 {
    measured_level_through(track, hz, eq, rustamp_app::dsp::DspChain::default())
}

/// [`measured_level`] with `chain` in the audio path.
fn measured_level_through(
    track: &std::path::Path,
    hz: f64,
    eq: &[rustamp_core::command::Command],
    chain: rustamp_app::dsp::DspChain,
) -> f64 {
    let (mono, rate) = recorded(
        track,
        eq,
        chain,
        rustamp_decoders::default_registry(),
        EQ_TONE_SECONDS,
    );
    level_of(&mono, rate, hz)
}

/// Plays `track` into the cable and returns the first channel of what came out,
/// from the moment it started, and its rate.
fn recorded(
    track: &std::path::Path,
    eq: &[rustamp_core::command::Command],
    mut chain: rustamp_app::dsp::DspChain,
    registry: rustamp_core::audio::DecoderRegistry,
    seconds: f64,
) -> (Vec<f32>, u32) {
    // A natural end closes a cpal output (see the test above), so every
    // measurement has its own engine.
    let output = Box::new(CpalOutput::with_device_name(CABLE_MATCH));
    let mut engine = Engine::new(registry, output, EventBus::new(), EngineConfig::default())
        .expect("the playback endpoint of the cable");
    assert!(is_cable_device(engine.output_name()));
    let granted = engine.device_spec();
    for command in eq {
        engine.apply(command).expect("applying the equalizer");
    }
    engine.set_dsp_chain(chain.build());

    let host = cpal::default_host();
    let capture = find_input_device(&host, CABLE_MATCH);
    let (config, format) = pick_capture_config(&capture, granted.sample_rate);
    let rate = config.sample_rate;
    let channels = usize::from(config.channels);
    let recording = Arc::new(Mutex::new(Recording::default()));
    let stream = start_capture(&capture, config, format, &recording);
    let pre_roll = (f64::from(rate) * PRE_ROLL.as_secs_f64()) as usize;
    let start = Instant::now();
    while recorded_frames(&recording, channels) < pre_roll {
        assert!(start.elapsed() < DEADLINE, "the capture is not running");
        std::thread::sleep(PUMP_INTERVAL);
    }
    let mark = recorded_frames(&recording, channels);

    engine.set_queue(vec![TrackSource::path(track.to_path_buf())]);
    engine
        .play_index(0)
        .expect("starting playback on the cable");
    let start = Instant::now();
    while engine.transport() != TransportState::Stopped {
        engine.poll().expect("polling the engine");
        assert!(start.elapsed() < DEADLINE, "the tone never finished");
        std::thread::sleep(PUMP_INTERVAL);
    }
    let wanted = mark + (f64::from(rate) * (seconds + TAIL.as_secs_f64())) as usize;
    let start = Instant::now();
    while recorded_frames(&recording, channels) < wanted {
        assert!(start.elapsed() < DEADLINE, "the capture stalled");
        std::thread::sleep(PUMP_INTERVAL);
    }
    drop(stream);

    let recording = std::mem::take(&mut *recording.lock().expect("the capture lock"));
    assert_eq!(recording.fatal_error, None, "the capture failed");
    let mono: Vec<f32> = recording
        .samples
        .chunks_exact(channels)
        .skip(mark)
        .map(|frame| frame[0])
        .collect();
    assert_eq!(
        engine
            .last_error()
            .filter(|error| error.kind != ErrorKind::Output),
        None,
        "the track failed"
    );
    (mono, rate)
}

/// The median amplitude of the tone at `hz` in the windows that hold it.
fn level_of(mono: &[f32], rate: u32, hz: f64) -> f64 {
    let window = (f64::from(rate) * EQ_WINDOW_SECONDS).round() as usize;
    let mut levels: Vec<f64> = mono
        .chunks_exact(window)
        .map(|chunk| tone_in_window(chunk, f64::from(rate), hz))
        .filter(|(amplitude, ratio)| *ratio >= MIN_TONE_RATIO && *amplitude > 0.001)
        .map(|(amplitude, _)| amplitude)
        .collect();
    assert!(
        levels.len() >= 5,
        "only {} windows hold the {hz} Hz tone",
        levels.len()
    );
    levels.sort_by(f64::total_cmp);
    levels[levels.len() / 2]
}

fn decibels(level: f64, reference: f64) -> f64 {
    20.0 * (level / reference).log10()
}

/// The equalizer changes what reaches the device: every band lifts its own
/// centre by what it says, leaves a far tone alone, and the switch and the
/// preamp do what they say. The tolerance covers the shared-mode path, and the
/// comparison is a ratio, so the endpoint volume does not enter it.
#[test]
#[ignore = "VM-only: needs the VB-Audio Virtual Cable; run inside the headless test VM"]
fn the_cable_carries_what_the_equalizer_says() {
    use rustamp_core::command::Command;
    use rustamp_dsp::eq::BAND_CENTERS_HZ;

    rustamp_app::window::require_test_vm();
    const TOLERANCE_DB: f64 = 1.0;
    let dir = tempfile::tempdir().unwrap();
    let on = Command::SetEqEnabled(true);
    let mut results = Vec::new();

    for (band, &hz) in BAND_CENTERS_HZ.iter().enumerate() {
        let track = dir.path().join(format!("band{band}.wav"));
        write_tone(&track, hz, EQ_TONE_SECONDS);
        let flat = measured_level(&track, hz, std::slice::from_ref(&on));
        for gain_db in [12.0f32, -12.0] {
            let set = Command::SetBand {
                band: band as u8,
                gain_db,
            };
            let level = measured_level(&track, hz, &[on.clone(), set]);
            results.push((band, hz, gain_db, decibels(level, flat)));
        }
    }
    let report = results
        .iter()
        .map(|(band, hz, want, got)| format!("band {band} ({hz} Hz) {want:+} dB: {got:+.2} dB"))
        .collect::<Vec<_>>()
        .join("\n");
    println!("{report}");
    for (_, _, want, got) in &results {
        assert!(
            (f64::from(*want) - got).abs() <= TOLERANCE_DB,
            "a band does not give what it says:\n{report}"
        );
    }

    // A band far from the tone leaves it alone, and so does a switched-off
    // equalizer, however its bands stand.
    let hz = 1000.0;
    let track = dir.path().join("far.wav");
    write_tone(&track, hz, EQ_TONE_SECONDS);
    let flat = measured_level(&track, hz, std::slice::from_ref(&on));
    let boost = |band: u8| Command::SetBand {
        band,
        gain_db: 12.0,
    };
    let far = measured_level(&track, hz, &[on.clone(), boost(0)]);
    assert!(
        decibels(far, flat).abs() <= TOLERANCE_DB / 2.0,
        "60 Hz moved a 1 kHz tone by {:+.2} dB",
        decibels(far, flat)
    );
    let off = measured_level(&track, hz, &[boost(4), Command::SetEqEnabled(false)]);
    assert!(
        decibels(off, flat).abs() <= TOLERANCE_DB / 2.0,
        "a switched-off equalizer moved the tone by {:+.2} dB",
        decibels(off, flat)
    );

    // The preamp moves everything.
    let preamp = measured_level(&track, hz, &[on.clone(), Command::SetPreamp(-6.0)]);
    assert!(
        (decibels(preamp, flat) + 6.0).abs() <= TOLERANCE_DB,
        "a -6 dB preamp gave {:+.2} dB",
        decibels(preamp, flat)
    );
}

// ----- a classic DSP plug-in, measured on the cable -----

/// The approval store for exactly `dll`, in `dir`.
fn approved(dir: &std::path::Path, dll: &std::path::Path) -> PathBuf {
    let bytes = std::fs::read(dll).expect("reading the plug-in");
    let name = dll.file_name().unwrap().to_string_lossy().into_owned();
    let mut store = rustamp_plughost::approval::ApprovalStore::new();
    store.approve_bytes(&name, &bytes).expect("the approval");
    let path = dir.join(format!("{name}.toml"));
    std::fs::write(&path, store.to_toml_string()).unwrap();
    path
}

fn beside_the_test(file: &str) -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.ancestors()
        .skip(1)
        .take(3)
        .map(|dir| dir.join(file))
        .find(|path| path.is_file())
        .unwrap_or_else(|| {
            panic!(
                "REFUSING LOUDLY: {file} is not staged beside {}",
                exe.display()
            )
        })
}

/// A classic DSP plug-in in the player's audio path changes what reaches the
/// device. Our own fixture doubles every sample, so the answer is exact: +6 dB.
/// The third-party plug-ins' results are their own, so those must only differ
/// from the plain tone in a way the recording shows.
#[test]
#[ignore = "VM-only: runs real DSP plug-ins and needs the VB-Audio Virtual Cable"]
fn the_cable_carries_what_a_dsp_plugin_does() {
    use rustamp_app::dsp::classic_dsp_chain;

    rustamp_app::window::require_test_vm();
    let dir = tempfile::tempdir().unwrap();
    let hz = 1000.0;
    let track = dir.path().join("tone.wav");
    write_tone(&track, hz, EQ_TONE_SECONDS);
    let helper = rustamp_app::supervise::find_plugin_host().expect("the staged plug-in host");
    let plain = measured_level(&track, hz, &[]);

    let fixture = beside_the_test("dsp_rustamp_fixture.dll");
    let chain = classic_dsp_chain(helper.clone(), &fixture, &approved(dir.path(), &fixture));
    let doubled = decibels(measured_level_through(&track, hz, &[], chain), plain);
    println!("dsp_rustamp_fixture: {doubled:+.2} dB");
    assert!(
        (doubled - 6.02).abs() <= 1.0,
        "the fixture doubles every sample, the cable shows {doubled:+.2} dB"
    );

    let plugins = std::env::var_os("RUSTAMP_TESTPLUGINS")
        .map_or_else(|| PathBuf::from(r"C:\rustamp\testplugins"), PathBuf::from);
    let optilab = plugins.join("dsp").join("dsp_optilab_core.dll");
    let chain = classic_dsp_chain(helper, &optilab, &approved(dir.path(), &optilab));
    let changed = decibels(measured_level_through(&track, hz, &[], chain), plain);
    println!("dsp_optilab_core: {changed:+.2} dB");
    assert!(
        changed.abs() >= 0.05,
        "the Optilab plug-in left the level at {changed:+.3} dB"
    );
}

// ----- a classic input plug-in, measured on the cable -----

/// A one-pattern ProTracker module: a looping square wave on the first channel.
fn tiny_module() -> Vec<u8> {
    const SAMPLE_BYTES: usize = 2048;
    let mut bytes = vec![0u8; 20];
    bytes[..4].copy_from_slice(b"tone");
    for sample in 0..31 {
        let mut header = [0u8; 30];
        if sample == 0 {
            header[22..24].copy_from_slice(&((SAMPLE_BYTES / 2) as u16).to_be_bytes());
            header[25] = 64;
            header[28..30].copy_from_slice(&((SAMPLE_BYTES / 2) as u16).to_be_bytes());
        }
        bytes.extend_from_slice(&header);
    }
    bytes.push(1);
    bytes.push(127);
    bytes.extend_from_slice(&[0u8; 128]);
    bytes.extend_from_slice(b"M.K.");
    let mut pattern = vec![0u8; 64 * 4 * 4];
    // Row 0, channel 0: sample 1 at period 428.
    pattern[..4].copy_from_slice(&[0x01, 0xAC, 0x10, 0x00]);
    bytes.extend_from_slice(&pattern);
    bytes.extend((0..SAMPLE_BYTES).map(|index| if index % 64 < 32 { 80u8 } else { 176u8 }));
    bytes
}

/// A classic input plug-in plays through the player: what it decodes reaches the
/// device at the level the built-in decoder gives the same file, and a format
/// the player has no decoder for plays at all.
#[test]
#[ignore = "VM-only: runs real input plug-ins and needs the VB-Audio Virtual Cable"]
fn the_cable_carries_what_an_input_plugin_decodes() {
    use rustamp_app::dsp::DspChain;
    use rustamp_app::plugin_input::factory;
    use rustamp_core::audio::DecoderRegistry;

    rustamp_app::window::require_test_vm();
    let dir = tempfile::tempdir().unwrap();
    let helper = rustamp_app::supervise::find_plugin_host().expect("the staged plug-in host");
    let plugins = std::env::var_os("RUSTAMP_TESTPLUGINS")
        .map_or_else(|| PathBuf::from(r"C:\rustamp\testplugins"), PathBuf::from);
    let only = |dll: &std::path::Path| {
        let mut registry = DecoderRegistry::new();
        registry.register(
            factory(helper.clone(), dll, &approved(dir.path(), dll))
                .expect("the plug-in's factory"),
        );
        registry
    };

    // in_wave and in_flac, against the built-in decoders.
    let hz = 1000.0;
    let wave = dir.path().join("tone.wav");
    write_tone(&wave, hz, EQ_TONE_SECONDS);
    let flac = fixture();
    for (dll, track, hz, seconds) in [
        ("in_wave.dll", wave.as_path(), hz, EQ_TONE_SECONDS),
        ("in_flac.dll", flac.as_path(), TONE_HZ, FIXTURE_SECONDS),
    ] {
        let dll = plugins.join("w566").join(dll);
        let builtin = recorded(
            track,
            &[],
            DspChain::default(),
            rustamp_decoders::default_registry(),
            seconds,
        );
        let through = recorded(track, &[], DspChain::default(), only(&dll), seconds);
        let difference = decibels(
            level_of(&through.0, through.1, hz),
            level_of(&builtin.0, builtin.1, hz),
        );
        println!(
            "{}: {difference:+.2} dB against the built-in decoder",
            dll.display()
        );
        assert!(
            difference.abs() <= 1.0,
            "{} played at {difference:+.2} dB",
            dll.display()
        );
    }

    // in_openmpt plays a format the player has no decoder for.
    let module = dir.path().join("tone.mod");
    std::fs::write(&module, tiny_module()).unwrap();
    let dll = plugins
        .join("libopenmpt")
        .join("Winamp2")
        .join("in_openmpt.dll");
    let (mono, _) = recorded(&module, &[], DspChain::default(), only(&dll), 1.0);
    let peak = mono
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    println!("in_openmpt: peak {peak:.4}");
    assert!(peak > 0.01, "in_openmpt played silence (peak {peak})");
}
