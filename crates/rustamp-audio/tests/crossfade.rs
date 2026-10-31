//! The crossfade between two tracks, measured on the null output.
//!
//! Every test records what the device receives and compares it with a model of
//! what the rules in `rustamp_audio::crossfade` say it must be: the length of
//! the run, the level across the fade, the joins, and what each transport
//! command does in the middle of a fade.

mod common;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Harness, WAIT, finished, pattern};
use rustamp_audio::crossfade::MAX_CROSSFADE_MS;
use rustamp_audio::{EngineConfig, ReplayGainMode};
use rustamp_core::audio::{
    AudioSpec, Confidence, Decoder, DecoderFactory, DecoderRegistry, DspStage, MediaSource,
    ProbeHint,
};
use rustamp_core::error::{Error, Result};
use rustamp_core::event::Event;
use rustamp_core::state::{EqState, TransportState};
use rustamp_dsp::fade::crossfade_gains;

/// The fade most tests use, in ms and in frames at the test's rate.
const FADE_MS: u32 = 100;
const FADE: usize = 4410;
/// A track that is much longer than the fade and than the ring.
const LONG: usize = 44_100;
/// How far into a fade the transport tests act.
const INTO_FADE: usize = 1000;
/// How many frames a test pulls at once.
const CHUNK: usize = 256;
/// What the model and the recording may differ by.
const EPSILON: f32 = 2e-5;

/// A sine with a name that says everything about it:
/// `s<hz>_<rate>_<channels>_<frames>_<amplitude %>[_<flag>].sin`.
///
/// Flag `nolen`: the decoder states no length, like a stream. Flag `long`: it
/// states four times the length it has. Flag `noseek`: it refuses a seek.
struct Sine {
    spec: AudioSpec,
    hz: f64,
    amplitude: f64,
    frames: u64,
    pos: u64,
    flag: String,
}

impl Decoder for Sine {
    fn spec(&self) -> AudioSpec {
        self.spec
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let channels = usize::from(self.spec.channels);
        let n = (out.len() / channels).min((self.frames - self.pos) as usize);
        for (i, frame) in out.chunks_exact_mut(channels).take(n).enumerate() {
            let at = (self.pos + i as u64) as f64 / f64::from(self.spec.sample_rate);
            let value = self.amplitude * (2.0 * std::f64::consts::PI * self.hz * at).sin();
            frame.fill(value as f32);
        }
        self.pos += n as u64;
        Ok(n)
    }

    fn seek(&mut self, to: Duration) -> Result<Duration> {
        if self.flag == "noseek" {
            return Err(Error::Unsupported("this track does not seek"));
        }
        let frames = (to.as_secs_f64() * f64::from(self.spec.sample_rate)).round() as u64;
        self.pos = frames.min(self.frames);
        Ok(self.spec.duration_of(self.pos))
    }

    fn duration(&self) -> Option<Duration> {
        match self.flag.as_str() {
            "nolen" => None,
            "long" => Some(self.spec.duration_of(self.frames * 4)),
            _ => Some(self.spec.duration_of(self.frames)),
        }
    }
}

struct SineFactory;

impl DecoderFactory for SineFactory {
    fn name(&self) -> &str {
        "sine"
    }

    fn extensions(&self) -> &[&str] {
        &["sin"]
    }

    fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
        if hint.extension == Some("sin") {
            Confidence::CONTENT
        } else {
            Confidence::NONE
        }
    }

    fn open(&self, source: MediaSource) -> Result<Box<dyn Decoder>> {
        let MediaSource::Path(path) = source else {
            panic!("the test tracks are files");
        };
        let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
        let parts: Vec<&str> = stem.strip_prefix('s').unwrap().split('_').collect();
        Ok(Box::new(Sine {
            spec: AudioSpec::new(parts[1].parse().unwrap(), parts[2].parse().unwrap()),
            hz: parts[0].parse().unwrap(),
            amplitude: parts[4].parse::<f64>().unwrap() / 100.0,
            frames: parts[3].parse().unwrap(),
            pos: 0,
            flag: parts.get(5).unwrap_or(&"").to_string(),
        }))
    }
}

/// A 44.1 kHz stereo track of `frames` frames at 50 % amplitude.
fn track(hz: u32, frames: usize) -> String {
    format!("s{hz}_44100_2_{frames}_50.sin")
}

/// The first channel of that track, as the device receives it.
fn source(hz: u32, frames: usize) -> Vec<f32> {
    sine(hz, frames, 0.5)
}

fn sine(hz: u32, frames: usize, amplitude: f64) -> Vec<f32> {
    (0..frames)
        .map(|f| {
            let at = f as f64 / 44_100.0;
            (amplitude * (2.0 * std::f64::consts::PI * f64::from(hz) * at).sin()) as f32
        })
        .collect()
}

/// What the rules say a seam is: `next` overlaps the last `fade` frames of `out`.
fn join(mut out: Vec<f32>, next: &[f32], fade: usize) -> Vec<f32> {
    let lead = out.len() - fade;
    for (i, sample) in out[lead..].iter_mut().enumerate() {
        let (outgoing, incoming) = crossfade_gains((i + 1) as f64 / (fade + 1) as f64);
        let next = next.get(i).copied().unwrap_or(0.0);
        *sample = *sample * outgoing + next * incoming;
    }
    out.extend_from_slice(next.get(fade..).unwrap_or(&[]));
    out
}

fn harness(names: &[&str], crossfade_ms: u32) -> Harness {
    let cfg = EngineConfig {
        crossfade_ms,
        ..EngineConfig::default()
    };
    let mut h = Harness::with_config(names, cfg);
    let mut registry = DecoderRegistry::new();
    registry.register(common::TestFactory::arc());
    registry.register(Arc::new(SineFactory));
    h.engine.set_registry(registry);
    // The chain leaves the signal alone, so the model is exact; the tests of
    // the limiter switch it on.
    h.engine.set_limiter_enabled(false);
    h
}

/// A device callback of `frames` frames, recorded into `out` (first channel).
///
/// Only what the ring really served is recorded: the callback fills an
/// underrun with silence, and that silence is not audio of the run. The frames
/// the ring served are counted by `consumed`, which the render callback
/// advances by exactly the frames it read.
///
/// A callback can serve fewer frames than the ring promised even when nothing
/// is wrong: the decode thread publishes its write position before it commits
/// the chunk (see `decode::Worker::write_all`, the order that keeps `consumed`
/// from running ahead of `written`), so a reader between the two steps sees a
/// backlog that is not readable yet. That window is on the order of a
/// scheduler slot; the frames are real and arrive, so the caller waits for
/// them instead of recording the silence as part of the run.
fn pump(h: &mut Harness, frames: usize, out: &mut Vec<f32>) -> usize {
    let (_, before) = h.engine.ring_counters();
    let mut stereo = Vec::new();
    h.clock.pump_record(frames as u64, &mut stereo);
    let (_, after) = h.engine.ring_counters();
    let served = ((after - before) as usize).min(frames);
    out.extend(
        stereo
            .as_chunks::<2>()
            .0
            .iter()
            .take(served)
            .map(|frame| frame[0]),
    );
    served
}

/// Pull `frames` frames, only ever what the ring really holds: nothing in the
/// recording is the silence of an underrun.
fn pull(h: &mut Harness, frames: usize, out: &mut Vec<f32>) {
    let deadline = Instant::now() + WAIT;
    let mut left = frames;
    while left > 0 {
        assert!(
            Instant::now() < deadline,
            "stall with {left} frames to pull"
        );
        h.engine.poll().expect("poll");
        let n = left.min(CHUNK);
        if h.engine.buffered_frames() >= n as u64 {
            let served = pump(h, n, out);
            left -= served;
            if served < n {
                std::thread::sleep(Duration::from_millis(1));
            }
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Record until the run has ended by itself, and cut the recording off at the
/// end of what the decode thread wrote.
fn play_out(h: &mut Harness, out: &mut Vec<f32>) {
    let deadline = Instant::now() + WAIT;
    while h.engine.transport() == TransportState::Playing {
        assert!(Instant::now() < deadline, "the run did not end");
        h.engine.poll().expect("poll");
        if h.engine.transport() != TransportState::Playing {
            break;
        }
        // The last frames of the run are fewer than a whole chunk.
        if h.engine.buffered_frames() >= CHUNK as u64 || h.engine.end_of_stream().is_some() {
            if pump(h, CHUNK, out) < CHUNK {
                std::thread::sleep(Duration::from_millis(1));
            }
        } else {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let (written, _) = h.engine.ring_counters();
    assert!(out.len() as u64 >= written, "the recording ends early");
    out.truncate(written as usize);
}

/// The whole run from the first entry.
fn record(h: &mut Harness) -> Vec<f32> {
    h.engine.play_index(0).expect("play");
    let mut out = Vec::new();
    play_out(h, &mut out);
    out
}

#[track_caller]
fn assert_matches(actual: &[f32], model: &[f32]) {
    assert_eq!(actual.len(), model.len(), "the length of the run");
    for (i, (a, m)) in actual.iter().zip(model).enumerate() {
        assert!((a - m).abs() <= EPSILON, "frame {i}: got {a}, expected {m}");
    }
}

/// The root-mean-square level of `samples`.
fn rms(samples: &[f32]) -> f64 {
    let power: f64 = samples.iter().map(|s| f64::from(*s).powi(2)).sum();
    (power / samples.len() as f64).sqrt()
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |peak, s| peak.max(s.abs()))
}

/// The largest step from one frame to the next.
fn largest_step(samples: &[f32]) -> f32 {
    samples
        .windows(2)
        .fold(0.0, |step, pair| step.max((pair[1] - pair[0]).abs()))
}

/// The largest step a sine can take at 44.1 kHz.
fn sine_step(hz: f64, amplitude: f64) -> f32 {
    (amplitude * 2.0 * std::f64::consts::PI * hz / 44_100.0) as f32
}

// ----- the setting -----

#[test]
fn off_is_the_default_and_a_seam_is_gapless_bit_for_bit() {
    assert_eq!(EngineConfig::default().crossfade_ms, 0);
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let mut h = harness(&[&a, &b], 0);
    let out = record(&mut h);
    let mut model = source(440, LONG);
    model.extend(source(1000, LONG));
    assert_eq!(out, model, "not a single bit moves");
}

#[test]
fn the_duration_is_clamped() {
    let mut h = harness(&[], u32::MAX);
    assert_eq!(h.engine.crossfade_ms(), MAX_CROSSFADE_MS);
    h.engine.set_crossfade_ms(1500);
    assert_eq!(h.engine.crossfade_ms(), 1500);
    h.engine.set_crossfade_ms(u32::MAX);
    assert_eq!(h.engine.crossfade_ms(), MAX_CROSSFADE_MS);
}

#[test]
fn switching_it_on_during_a_track_fades_the_end_of_that_track() {
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let mut h = harness(&[&a, &b], 0);
    h.engine.play_index(0).expect("play");
    assert!(h.wait_track(0));
    h.engine.set_crossfade_ms(FADE_MS);
    let mut out = Vec::new();
    play_out(&mut h, &mut out);
    assert_matches(&out, &join(source(440, LONG), &source(1000, LONG), FADE));
}

#[test]
fn switching_it_off_during_a_track_leaves_a_plain_seam() {
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let mut h = harness(&[&a, &b], FADE_MS);
    // The limiter in the path, so that the sum limiter has to leave it too.
    h.engine.set_limiter_enabled(true);
    h.engine.play_index(0).expect("play");
    assert!(h.wait_track(0));
    h.engine.set_crossfade_ms(0);
    let mut out = Vec::new();
    play_out(&mut h, &mut out);
    let mut model = source(440, LONG);
    model.extend(source(1000, LONG));
    assert_matches(&out, &model);
}

// ----- what a fade is -----

#[test]
fn a_seam_overlaps_by_the_duration_on_the_equal_power_curve() {
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let mut h = harness(&[&a, &b], FADE_MS);
    let out = record(&mut h);
    assert_eq!(out.len(), 2 * LONG - FADE, "the sum minus the fade");
    assert_matches(&out, &join(source(440, LONG), &source(1000, LONG), FADE));
}

#[test]
fn the_level_holds_across_the_fade_and_the_joins_do_not_click() {
    let fade_ms = 500;
    let fade = 22_050;
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let mut h = harness(&[&a, &b], fade_ms);
    let out = record(&mut h);

    // Whole cycles of both tones: 0.1 s.
    let window = 4410;
    let reference = rms(&out[..window]);
    let start = LONG - fade;
    let end = LONG;
    for at in (start..end - window).step_by(window / 2) {
        let level = rms(&out[at..at + window]);
        let db = 20.0 * (level / reference).log10();
        assert!(db.abs() < 0.5, "{db:+.2} dB at frame {at}");
    }

    // Neither at the start of the fade nor at its end does the signal jump. In
    // the fade the two steps add up, each with its gain: at most sqrt(2) times
    // the larger one, plus what the curve itself adds per frame.
    let allowed = sine_step(1000.0, 0.5) * std::f32::consts::SQRT_2 * 1.01;
    let step = largest_step(&out);
    assert!(step <= allowed, "a step of {step} against {allowed}");
}

#[test]
fn the_tracks_may_differ_in_rate_and_channels() {
    // 44.1 kHz stereo into 48 kHz mono on a 44.1 kHz stereo device: one second
    // each, and the fade is counted in device frames.
    let (a, b) = (track(440, LONG), "s1000_48000_1_48000_50.sin".to_owned());
    let mut h = harness(&[&a, &b], FADE_MS);
    let out = record(&mut h);
    assert_eq!(out.len(), 2 * LONG - FADE);

    // The incoming side arrives converted: its level, and only its tone.
    let window = 4410;
    let reference = rms(&out[..window]);
    for at in (LONG - FADE..out.len() - 2 * window).step_by(window) {
        let db = 20.0 * (rms(&out[at..at + window]) / reference).log10();
        assert!(db.abs() < 0.5, "{db:+.2} dB at frame {at}");
    }
    // The resampler's look-ahead rings at the very end of a stream only.
    let settled = &out[..out.len() - 256];
    let allowed = sine_step(1000.0, 0.5) * std::f32::consts::SQRT_2 * 1.05;
    let step = largest_step(settled);
    assert!(step <= allowed, "a step of {step} against {allowed}");
}

#[test]
fn a_rate_only_the_outgoing_track_has_is_converted_as_well() {
    let (a, b) = ("s440_48000_2_48000_50.sin".to_owned(), track(1000, LONG));
    let mut h = harness(&[&a, &b], FADE_MS);
    let out = record(&mut h);
    assert_eq!(out.len(), 2 * LONG - FADE);
    // Behind the fade the incoming track is its source, bit for bit in place.
    let model = source(1000, LONG);
    assert_matches(&out[LONG..], &model[FADE..]);
}

#[test]
fn a_next_track_shorter_than_the_fade_gives_half_of_itself_and_nothing_is_lost() {
    let short = 2000;
    let (a, b, c) = (track(440, LONG), track(1000, short), track(660, LONG));
    let mut h = harness(&[&a, &b, &c], FADE_MS);
    let out = record(&mut h);
    // Half of the short track goes into its fade-in, and the next fade covers
    // all that is held of it.
    let first = short / 2;
    let model = join(source(440, LONG), &source(1000, short), first);
    let model = join(model, &source(660, LONG), short);
    assert_eq!(out.len(), LONG + short + LONG - first - short);
    assert_matches(&out, &model);
}

#[test]
fn a_track_that_ends_before_its_stated_length_leaves_the_tail_fading_over_silence() {
    // It states 4000 frames and has 1000: the fade is half of what it states.
    let (a, b) = (
        track(440, LONG),
        "s1000_44100_2_1000_50_long.sin".to_owned(),
    );
    let mut h = harness(&[&a, &b], FADE_MS);
    let out = record(&mut h);
    assert_matches(&out, &join(source(440, LONG), &source(1000, 1000), 2000));
}

#[test]
fn the_last_track_plays_to_its_end_at_unity() {
    let a = track(440, LONG);
    let mut h = harness(&[&a], FADE_MS);
    let out = record(&mut h);
    assert_eq!(out, source(440, LONG));
    let ended = h
        .drain_events()
        .iter()
        .any(|event| matches!(event, Event::PlaylistEnded));
    assert!(ended, "the playlist ended");
}

#[test]
fn a_stream_takes_no_part_on_either_side() {
    let nolen = "s1000_44100_2_20000_50_nolen.sin".to_owned();
    let a = track(440, LONG);

    let mut h = harness(&[&a, &nolen], FADE_MS);
    let out = record(&mut h);
    let mut model = source(440, LONG);
    model.extend(source(1000, 20_000));
    assert_matches(&out, &model);

    let mut h = harness(&[&nolen, &a], FADE_MS);
    let out = record(&mut h);
    let mut model = source(1000, 20_000);
    model.extend(source(440, LONG));
    assert_matches(&out, &model);
}

// ----- the traversal -----

#[test]
fn repeat_fades_the_list_into_its_own_start() {
    let a = track(440, 22_050);
    let mut h = harness(&[&a], FADE_MS);
    h.engine.set_repeat(true).expect("repeat");
    h.engine.play_index(0).expect("play");

    let source = source(440, 22_050);
    let cycle = 22_050 - FADE;
    let mut out = Vec::new();
    pull(&mut h, 2 * cycle, &mut out);
    h.engine.stop().expect("stop");

    let model = join(source.clone(), &source, FADE);
    assert_matches(&out[..2 * cycle], &model[..2 * cycle]);
}

#[test]
fn shuffle_fades_along_the_permutation() {
    let tones = [440, 660, 880, 1000];
    let names: Vec<String> = tones.iter().map(|&hz| track(hz, 22_050)).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut h = harness(&names, FADE_MS);
    h.engine.set_shuffle_seed(7);
    h.engine.set_shuffle(true).expect("shuffle");
    let plan = h.engine.shuffle_plan();
    assert_ne!(plan, vec![0, 1, 2, 3], "this seed really shuffles");

    h.engine.play().expect("play");
    let mut out = Vec::new();
    play_out(&mut h, &mut out);

    let mut model = source(tones[plan[0]], 22_050);
    for &entry in &plan[1..] {
        model = join(model, &source(tones[entry], 22_050), FADE);
    }
    assert_matches(&out, &model);
}

#[test]
fn stop_after_current_ends_the_run_with_the_track_at_unity() {
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let mut h = harness(&[&a, &b], FADE_MS);
    h.engine.toggle_stop_after_current().expect("arm");
    let out = record(&mut h);
    assert_eq!(out, source(440, LONG));
    assert_eq!(h.engine.current_index(), Some(0));
}

// ----- the display -----

#[test]
fn the_incoming_track_takes_over_where_its_first_frame_is_heard() {
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let mut h = harness(&[&a, &b], FADE_MS);
    h.engine.play_index(0).expect("play");
    let start = LONG - FADE;
    let mut out = Vec::new();

    // One frame before the fade is heard: the engine already names the
    // incoming track (the same place as a gapless seam), its position waits.
    pull(&mut h, start - 1, &mut out);
    assert!(h.wait_track(1), "the incoming track was opened");
    assert_eq!(h.engine.current_index(), Some(1));
    assert_eq!(h.engine.position_frames(), 0);
    assert_eq!(h.engine.audible_index(), Some(0));
    let changed = h
        .drain_events()
        .iter()
        .any(|event| matches!(event, Event::TrackChanged { index: Some(1), .. }));
    assert!(changed, "TrackChanged went out");

    // In the fade the position counts the incoming track's own frames ...
    pull(&mut h, 1 + INTO_FADE, &mut out);
    h.engine.poll().expect("poll");
    assert_eq!(h.engine.position_frames(), INTO_FADE as u64);
    assert_eq!(h.engine.audible_index(), Some(1));

    // ... and it does not jump at the end of the fade.
    pull(&mut h, FADE, &mut out);
    h.engine.poll().expect("poll");
    assert_eq!(h.engine.position_frames(), (INTO_FADE + FADE) as u64);
    assert_eq!(h.engine.duration_frames(), Some(LONG as u64));
}

// ----- the transport in the middle of a fade -----

/// A harness two tracks into their fade, `INTO_FADE` frames deep.
fn in_the_fade(names: &[&str]) -> (Harness, Vec<f32>) {
    let mut h = harness(names, FADE_MS);
    h.engine.play_index(0).expect("play");
    let mut out = Vec::new();
    pull(&mut h, LONG - FADE + INTO_FADE, &mut out);
    (h, out)
}

#[test]
fn a_pause_freezes_the_fade_and_the_resume_goes_on_where_it_was() {
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let (mut h, mut out) = in_the_fade(&[&a, &b]);

    h.engine.pause().expect("pause");
    assert_eq!(h.clock.pump(1024), 0, "a paused device pulls nothing");
    std::thread::sleep(Duration::from_millis(50));
    h.engine.poll().expect("poll");
    assert_eq!(h.engine.transport(), TransportState::Paused);
    let position = h.engine.position_frames();
    assert_eq!(position, INTO_FADE as u64);

    h.engine.play().expect("resume");
    play_out(&mut h, &mut out);
    assert_matches(&out, &join(source(440, LONG), &source(1000, LONG), FADE));
}

#[test]
fn a_seek_ends_the_fade_and_the_track_plays_on_at_unity() {
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let (mut h, _) = in_the_fade(&[&a, &b]);
    let target = 20_000;
    assert!(h.seek_and_settle(target as u64), "the seek lands exactly");
    assert_eq!(
        h.engine.current_index(),
        Some(1),
        "it is the incoming track"
    );

    let mut out = Vec::new();
    pull(&mut h, LONG - target - CHUNK, &mut out);
    let model = source(1000, LONG);
    assert_matches(&out, &model[target..LONG - CHUNK]);
}

#[test]
fn a_refused_seek_leaves_the_fade_running() {
    let (a, b) = (
        track(440, LONG),
        format!("s1000_44100_2_{LONG}_50_noseek.sin"),
    );
    let (mut h, mut out) = in_the_fade(&[&a, &b]);
    h.engine
        .seek_frames(20_000)
        .expect("the request itself is fine");
    play_out(&mut h, &mut out);
    assert_matches(&out, &join(source(440, LONG), &source(1000, LONG), FADE));
}

#[test]
fn a_seek_into_the_end_of_a_track_shortens_its_fade() {
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let mut h = harness(&[&a, &b], FADE_MS);
    h.engine.play_index(0).expect("play");
    let mut out = Vec::new();
    pull(&mut h, 1000, &mut out);
    let left = 2000;
    assert!(
        h.seek_and_settle((LONG - left) as u64),
        "a seek into the end"
    );
    out.clear();
    pull(&mut h, LONG - CHUNK, &mut out);
    // Only that much of the track is left, so that is all the fade has.
    let model = join(
        source(440, LONG)[LONG - left..].to_vec(),
        &source(1000, LONG),
        left,
    );
    assert_matches(&out, &model[..LONG - CHUNK]);
}

#[test]
fn a_stop_drops_the_fade_and_the_next_run_starts_at_unity() {
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let (mut h, _) = in_the_fade(&[&a, &b]);
    h.engine.stop().expect("stop");
    assert_eq!(h.engine.transport(), TransportState::Stopped);
    assert_eq!(h.engine.position_frames(), 0);

    h.engine.play_index(0).expect("play");
    assert!(h.settle_drain());
    let mut out = Vec::new();
    pull(&mut h, 4096, &mut out);
    assert_matches(&out, &source(440, 4096));
}

#[test]
fn a_skip_in_the_fade_drops_it_and_starts_the_target_at_unity() {
    let (a, b, c) = (track(440, LONG), track(1000, LONG), track(660, LONG));
    let (mut h, mut out) = in_the_fade(&[&a, &b, &c]);
    h.engine.next_track().expect("next");
    assert!(h.wait_track(2), "next goes on from the incoming track");
    play_out(&mut h, &mut out);
    // What the ring still held plays, then the target, whole and untouched: it
    // is the last track, so nothing fades it.
    let model = source(660, LONG);
    assert_matches(&out[out.len() - LONG..], &model);
}

#[test]
fn prev_in_the_fade_goes_back_to_the_outgoing_track() {
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let (mut h, _) = in_the_fade(&[&a, &b]);
    h.engine.prev_track().expect("prev");
    assert!(h.wait_track(0));
    assert_eq!(h.engine.current_index(), Some(0));
}

// ----- the chain -----

/// A stage that remembers what it was fed: how many frames, how often it was
/// prepared, and the largest step in the first channel.
#[derive(Clone, Default)]
struct Spy {
    frames: Arc<AtomicU64>,
    prepares: Arc<AtomicU64>,
    state: Arc<Mutex<(Option<f32>, f32)>>,
}

impl DspStage for Spy {
    fn name(&self) -> &str {
        "spy"
    }

    fn prepare(&mut self, _spec: AudioSpec) -> Result<()> {
        self.prepares.fetch_add(1, Ordering::SeqCst);
        self.state.lock().unwrap().0 = None;
        Ok(())
    }

    fn process(&mut self, buf: &mut [f32]) -> Result<()> {
        self.frames
            .fetch_add((buf.len() / 2) as u64, Ordering::SeqCst);
        let mut state = self.state.lock().unwrap();
        for frame in buf.as_chunks::<2>().0 {
            if let Some(last) = state.0 {
                state.1 = state.1.max((frame[0] - last).abs());
            }
            state.0 = Some(frame[0]);
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.state.lock().unwrap().0 = None;
    }

    fn latency_frames(&self) -> u32 {
        0
    }
}

#[test]
fn a_stage_sees_each_track_whole_and_never_a_seam() {
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let mut h = harness(&[&a, &b], FADE_MS);
    let spy = Spy::default();
    h.engine.set_dsp_chain(vec![Box::new(spy.clone())]);
    let out = record(&mut h);
    assert_eq!(out.len(), 2 * LONG - FADE);
    assert_eq!(spy.prepares.load(Ordering::SeqCst), 2, "one per track");
    assert_eq!(
        spy.frames.load(Ordering::SeqCst),
        2 * LONG as u64,
        "every frame of both tracks, the overlap included"
    );
    let step = spy.state.lock().unwrap().1;
    let allowed = sine_step(1000.0, 0.5) * 1.01;
    assert!(step <= allowed, "a step of {step} against {allowed}");
}

#[test]
fn replaygain_scales_each_side_by_its_own_tag() {
    let frames = 22_050;
    let a = format!("rg_-6.02_none_1_{frames}.tst");
    let b = format!("track2_{frames}.tst");
    let mut h = harness(&[&a, &b], FADE_MS);
    h.engine.set_replaygain_mode(ReplayGainMode::Track);
    let out = record(&mut h);

    let gain = 10f32.powf(-6.02 / 20.0);
    let outgoing: Vec<f32> = (0..frames as u64)
        .map(|f| finished(pattern(1, f) * gain))
        .collect();
    let incoming: Vec<f32> = (0..frames as u64)
        .map(|f| finished(pattern(2, f)))
        .collect();
    let model: Vec<f32> = join(outgoing, &incoming, FADE)
        .into_iter()
        .map(finished)
        .collect();
    assert_matches(&out, &model);
}

#[test]
fn the_equalizer_applies_on_both_sides_of_the_fade() {
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let plain = {
        let mut h = harness(&[&a, &b], FADE_MS);
        record(&mut h)
    };
    let mut h = harness(&[&a, &b], FADE_MS);
    h.engine.set_eq(EqState {
        enabled: true,
        preamp: -12.0,
        ..EqState::default()
    });
    let out = record(&mut h);
    assert_eq!(out.len(), plain.len());
    // Before the fade, in the middle of it and behind it.
    let middle = LONG - FADE / 2;
    let late = LONG + FADE;
    for at in [LONG / 2, middle - 1024, late] {
        let db = 20.0 * (rms(&out[at..at + 2048]) / rms(&plain[at..at + 2048])).log10();
        assert!((db + 12.0).abs() < 0.2, "{db:+.2} dB at frame {at}");
    }
}

#[test]
fn the_limiter_catches_what_the_two_sides_add_up_to() {
    // The same tone twice, in phase, just below the limiter's ceiling: the sum
    // of the two reaches sqrt(2) times that in the middle of the fade.
    let name = "s440_44100_2_44100_96.sin".to_owned();
    let second = "s440_44100_2_44099_96.sin".to_owned();

    let mut h = harness(&[&name, &second], FADE_MS);
    let unlimited = record(&mut h);
    assert_eq!(unlimited.len(), 44_100 + 44_099 - FADE);
    assert!(peak(&unlimited) <= 1.0, "the sum never leaves full scale");

    let mut h = harness(&[&name, &second], FADE_MS);
    h.engine.set_limiter_enabled(true);
    let ceiling = h.engine.limiter_ceiling();
    let limited = record(&mut h);
    assert_eq!(limited.len(), unlimited.len(), "the limiter costs no frame");
    let measured = peak(&limited);
    assert!(
        measured <= ceiling + 1e-3,
        "the peak is {measured} with the ceiling at {ceiling}"
    );
}

#[test]
fn with_the_limiter_on_a_quiet_fade_is_exact_to_the_frame() {
    // Two limiters in the path, each with its look-ahead: nothing of it may
    // show in the timeline or in the signal when nothing is near the ceiling.
    let (a, b) = (track(440, LONG), track(1000, LONG));
    let mut h = harness(&[&a, &b], FADE_MS);
    h.engine.set_limiter_enabled(true);
    let out = record(&mut h);
    assert_matches(&out, &join(source(440, LONG), &source(1000, LONG), FADE));

    let (a, b) = (track(440, LONG), track(1000, LONG));
    let mut h = harness(&[&a, &b], FADE_MS);
    h.engine.set_limiter_enabled(true);
    h.engine.play_index(0).expect("play");
    let mut out = Vec::new();
    pull(&mut h, LONG - FADE + INTO_FADE, &mut out);
    h.engine.poll().expect("poll");
    assert_eq!(h.engine.position_frames(), INTO_FADE as u64);
    assert_eq!(h.engine.audible_index(), Some(1));
}

#[test]
fn the_run_hands_the_installed_chain_back_after_a_fade() {
    let (a, b) = (track(440, 22_050), track(1000, 22_050));
    let mut h = harness(&[&a, &b], FADE_MS);
    let spy = Spy::default();
    h.engine.set_dsp_chain(vec![Box::new(spy.clone())]);
    record(&mut h);
    let after_the_first_run = spy.frames.load(Ordering::SeqCst);

    h.engine.play_index(0).expect("second run");
    assert!(h.settle_drain());
    let mut out = Vec::new();
    pull(&mut h, 4096, &mut out);
    assert!(spy.frames.load(Ordering::SeqCst) > after_the_first_run);
    assert_matches(&out, &source(440, 4096));
}
