//! Shared test support for the `rustamp-audio` integration tests.
//!
//! * [`TestFactory`] — a deterministic decoder for any `*.tst` path. The file
//!   stem is `track<seed>_<frames>`, so a test can queue several synthetic
//!   tracks and know every sample in advance.
//! * [`Harness`] — an [`Engine`] on a [`NullOutput`], with helpers to poll the
//!   engine and drive the virtual clock.
#![allow(dead_code)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use rustamp_audio::{Engine, EngineConfig, TrackSource};
use rustamp_core::audio::{AudioSpec, Confidence, Decoder, DecoderFactory, MediaSource, ProbeHint};
use rustamp_core::bus::{EventBus, EventReceiver};
use rustamp_core::error::Result;
use rustamp_core::event::Event;
use rustamp_core::meta::TrackMetadata;
use rustamp_core::state::StreamInfo;
use rustamp_output::{NullClock, NullOutput};

/// Format of every synthetic track.
pub const SPEC: AudioSpec = AudioSpec::new(44100, 2);

/// Frames per synthetic track unless the path says otherwise.
pub const TRACK_FRAMES: u64 = 3000;

/// Stall escape for every wait in this harness (the M10 flakiness note's
/// rule): a bound so generous that only a real stall reaches it, with the
/// message naming the step — never a wall-clock bound a healthy-but-slow
/// VM schedule could trip. The 5 s bounds this file carried before are
/// the flake class M24 proved in this guest (docs/tasks/M24-stubborn-three.md §1.3, M25, M26).
pub const WAIT: Duration = Duration::from_secs(30);

/// The sample value at `frame` for track `seed`: never zero, so silence in the
/// output is always detectable.
///
/// The range is `0.1..=1.1`, so the pattern reaches above full scale: the
/// converter's finishing clamps it (see [`finished`]), and the limiter's
/// ceiling (0.966) lies below the top of the range, so the default chain
/// really limits this material.
pub fn pattern(seed: u64, frame: u64) -> f32 {
    let x = frame
        .wrapping_mul(2_654_435_761)
        .wrapping_add(seed.wrapping_mul(7919))
        % 2001;
    0.1 + x as f32 / 2000.0
}

/// Frames of the track at `path`.
pub fn track_frames(path: &str) -> u64 {
    let stem = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = stem.strip_suffix(".tst").unwrap_or(stem);
    stem.rsplit_once('_')
        .and_then(|(_, f)| f.parse().ok())
        .unwrap_or(TRACK_FRAMES)
}

/// Seed of the track at `path`.
pub fn track_seed(path: &str) -> u64 {
    let stem = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = stem.strip_suffix(".tst").unwrap_or(stem);
    let head = stem.split('_').next().unwrap_or(stem);
    head.trim_start_matches("track").parse().unwrap_or(1)
}

/// A synthetic track with a known pattern, length and seek behaviour.
struct PatternDecoder {
    spec: AudioSpec,
    seed: u64,
    total: u64,
    pos: u64,
    /// ReplayGain track gain in dB (`None` = untagged), for the ReplayGain
    /// tests. Formatted into `extra` exactly the way `rustamp-meta` writes
    /// it (`"+6.02 dB"`).
    rg_track: Option<f32>,
    /// ReplayGain album gain in dB (`None` = untagged).
    rg_album: Option<f32>,
}

impl Decoder for PatternDecoder {
    fn spec(&self) -> AudioSpec {
        self.spec
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let ch = usize::from(self.spec.channels);
        let want = out.len() / ch;
        let n = want.min(self.total.saturating_sub(self.pos) as usize);
        for i in 0..n {
            let v = pattern(self.seed, self.pos + i as u64);
            for c in 0..ch {
                out[i * ch + c] = v;
            }
        }
        self.pos += n as u64;
        Ok(n)
    }

    fn seek(&mut self, to: Duration) -> Result<Duration> {
        let frames = (to.as_secs_f64() * f64::from(self.spec.sample_rate)).round() as u64;
        self.pos = frames.min(self.total);
        Ok(self.spec.duration_of(self.pos))
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.spec.duration_of(self.total))
    }

    fn stream_info(&self) -> StreamInfo {
        StreamInfo {
            bitrate_kbps: Some(1411),
            sample_rate_hz: self.spec.sample_rate,
            channels: self.spec.channels,
        }
    }

    fn metadata(&self) -> TrackMetadata {
        let mut metadata = TrackMetadata::default();
        if let Some(gain) = self.rg_track {
            metadata
                .extra
                .insert("REPLAYGAIN_TRACK_GAIN".to_owned(), format!("{gain:+.2} dB"));
        }
        if let Some(gain) = self.rg_album {
            metadata
                .extra
                .insert("REPLAYGAIN_ALBUM_GAIN".to_owned(), format!("{gain:+.2} dB"));
        }
        metadata
    }

    fn skipped_packets(&self) -> u64 {
        0
    }
}

/// Frequency of the [`tone`] tracks: an even number of hertz, so the tone has
/// a crest at every multiple of 0.5 s.
pub const TONE_HZ: f64 = 1_000.0;

/// Amplitude of the [`tone`] tracks.
pub const TONE_AMPLITUDE: f64 = 0.5;

/// The sample of the continuous test tone at global frame `frame` of a
/// 44.1 kHz stream. A track `tone<start>_<frames>.tst` plays frames
/// `start..start + frames`, so `tone0_22050` followed by `tone22050_22050` is
/// one unbroken tone whose seam (0.5 s) sits on a crest, where cutting the
/// resampler's filter off would hurt most.
pub fn tone(frame: u64) -> f32 {
    let t = frame as f64 / f64::from(SPEC.sample_rate);
    (TONE_AMPLITUDE * (2.0 * std::f64::consts::PI * TONE_HZ * t).cos()) as f32
}

/// The format a `fmt<n>_<m>.tst` track switches to after `n` frames.
pub const CHANGED_SPEC: AudioSpec = AudioSpec::new(48_000, 2);

/// The sample of the seeded noise signal at `frame`: a pure function of the
/// seed and the frame index, so a test can model every one of them.
///
/// The values are hostile on purpose: the range reaches far above full
/// scale, and every 128th frame carries a value no chain may pass on (an
/// infinity, a NaN, a value far above full scale). The chain's signal
/// hygiene has something to catch here.
pub fn noise_sample(seed: u64, frame: u64) -> f32 {
    // A splitmix64-style mix of the seed and the frame index: cheap, seeded
    // and reproducible, and spread over the whole 64-bit range.
    let mut z = seed
        .wrapping_mul(0xD6E8_FEB8_6659_FD93)
        .wrapping_add(frame.wrapping_mul(0xA076_1D64_78BD_642F))
        .wrapping_add(0xE703_7ED1_A0B4_28DB);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    match frame % 128 {
        0 => f32::INFINITY,
        32 => f32::NEG_INFINITY,
        48 => f32::NAN,
        96 => 1.0e30,
        _ => {
            // A finite value in -4.0..=4.0.
            let unit = (z >> 11) as f64 / (1u64 << 53) as f64;
            let sign = if z & 1 == 0 { 1.0 } else { -1.0 };
            (sign * unit * 4.0) as f32
        }
    }
}

/// One stretch of a [`ScriptDecoder`]: a format, a length and the sample at
/// each frame.
struct Segment {
    spec: AudioSpec,
    frames: u64,
    value: Box<dyn Fn(u64) -> f32 + Send>,
}

/// A decoder that plays segments back to back, reports
/// [`rustamp_core::error::Error::FormatChanged`] between two of them and can
/// end with a decode error instead of a clean end.
struct ScriptDecoder {
    segments: Vec<Segment>,
    current: usize,
    pos: u64,
    fail_at_end: bool,
}

impl Decoder for ScriptDecoder {
    fn spec(&self) -> AudioSpec {
        self.segments[self.current].spec
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let seg = &self.segments[self.current];
        if self.pos == seg.frames {
            if self.current + 1 < self.segments.len() {
                let from = seg.spec;
                self.current += 1;
                self.pos = 0;
                return Err(rustamp_core::error::Error::FormatChanged {
                    from,
                    to: self.segments[self.current].spec,
                });
            }
            if self.fail_at_end {
                self.fail_at_end = false;
                return Err(rustamp_core::error::Error::Decode("scripted".into()));
            }
            return Ok(0);
        }
        let ch = usize::from(seg.spec.channels);
        let n = (out.len() / ch).min((seg.frames - self.pos) as usize);
        for i in 0..n {
            let v = (seg.value)(self.pos + i as u64);
            out[i * ch..(i + 1) * ch].fill(v);
        }
        self.pos += n as u64;
        Ok(n)
    }

    fn seek(&mut self, to: Duration) -> Result<Duration> {
        let spec = self.spec();
        let frames = (to.as_secs_f64() * f64::from(spec.sample_rate)).round() as u64;
        self.pos = frames.min(self.segments[self.current].frames);
        Ok(spec.duration_of(self.pos))
    }

    fn duration(&self) -> Option<Duration> {
        Some(
            self.segments
                .iter()
                .map(|s| s.spec.duration_of(s.frames))
                .sum(),
        )
    }

    fn stream_info(&self) -> StreamInfo {
        let spec = self.spec();
        StreamInfo {
            bitrate_kbps: None,
            sample_rate_hz: spec.sample_rate,
            channels: spec.channels,
        }
    }

    fn skipped_packets(&self) -> u64 {
        0
    }
}

/// The two numbers of a `<prefix><a>_<b>` stem.
fn stem_numbers(name: &str, prefix: &str) -> Option<(u64, u64)> {
    let stem = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = stem.strip_suffix(".tst").unwrap_or(stem);
    let (a, b) = stem.strip_prefix(prefix)?.split_once('_')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// The scripted decoders: `tone<start>_<frames>` (a stretch of [`tone`]),
/// `fmt<n>_<m>` (`n` frames of track 1's pattern at [`SPEC`], a format change
/// to [`CHANGED_SPEC`], then `m` frames of track 2's pattern),
/// `err<n>_<seed>` (`n` frames of [`tone`], then a decode error) and
/// `noise<seed>_<frames>` (the seeded noise signal of [`noise_sample`]).
fn scripted(name: &str) -> Option<ScriptDecoder> {
    let script = |segments, fail_at_end| ScriptDecoder {
        segments,
        current: 0,
        pos: 0,
        fail_at_end,
    };
    if let Some((start, frames)) = stem_numbers(name, "tone") {
        let value = Box::new(move |f| tone(start + f));
        return Some(script(
            vec![Segment {
                spec: SPEC,
                frames,
                value,
            }],
            false,
        ));
    }
    if let Some((seed, frames)) = stem_numbers(name, "noise") {
        let value = Box::new(move |f| noise_sample(seed, f));
        return Some(script(
            vec![Segment {
                spec: SPEC,
                frames,
                value,
            }],
            false,
        ));
    }
    if let Some((n, m)) = stem_numbers(name, "fmt") {
        return Some(script(
            vec![
                Segment {
                    spec: SPEC,
                    frames: n,
                    value: Box::new(|f| pattern(1, f)),
                },
                Segment {
                    spec: CHANGED_SPEC,
                    frames: m,
                    value: Box::new(|f| pattern(2, f)),
                },
            ],
            false,
        ));
    }
    if let Some((n, _)) = stem_numbers(name, "err") {
        return Some(script(
            vec![Segment {
                spec: SPEC,
                frames: n,
                value: Box::new(tone),
            }],
            true,
        ));
    }
    None
}

/// A ReplayGain-tagged synthetic track: `rg_<track>_<album>_<seed>_<frames>`,
/// where `<track>`/`<album>` are gains in dB or `none` for untagged, e.g.
/// `rg_-6.02_none_1_44100.tst` (track gain −6.02 dB, seed 1, one second).
/// The samples are [`pattern`] for `seed`, exactly like an untagged track.
fn parse_rg_name(name: &str) -> Option<(Option<f32>, Option<f32>, u64, u64)> {
    let stem = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = stem.strip_suffix(".tst").unwrap_or(stem);
    let rest = stem.strip_prefix("rg_")?;
    let mut parts = rest.split('_');
    let track = parts.next()?;
    let album = parts.next()?;
    let seed: u64 = parts.next()?.parse().ok()?;
    let frames: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    fn gain(text: &str) -> Option<Option<f32>> {
        if text == "none" {
            Some(None)
        } else {
            text.parse().ok().map(Some)
        }
    }
    Some((gain(track)?, gain(album)?, seed, frames))
}

/// Factory for `*.tst` paths.
pub struct TestFactory;

impl TestFactory {
    /// Ready for [`rustamp_core::audio::DecoderRegistry::register`].
    pub fn arc() -> Arc<dyn DecoderFactory> {
        Arc::new(TestFactory)
    }
}

impl DecoderFactory for TestFactory {
    fn name(&self) -> &str {
        "test"
    }

    fn extensions(&self) -> &[&str] {
        &["tst"]
    }

    fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
        if hint.extension == Some("tst") {
            Confidence::CONTENT
        } else {
            Confidence::NONE
        }
    }

    fn open(&self, source: MediaSource) -> Result<Box<dyn Decoder>> {
        let name = match &source {
            MediaSource::Path(p) => p.display().to_string(),
            MediaSource::Url(u) => u.clone(),
            MediaSource::Reader { extension, .. } => extension.clone().unwrap_or_default(),
        };
        if let Some(decoder) = scripted(&name) {
            return Ok(Box::new(decoder));
        }
        if let Some((rg_track, rg_album, seed, total)) = parse_rg_name(&name) {
            return Ok(Box::new(PatternDecoder {
                spec: SPEC,
                seed,
                total,
                pos: 0,
                rg_track,
                rg_album,
            }));
        }
        Ok(Box::new(PatternDecoder {
            spec: SPEC,
            seed: track_seed(&name),
            total: track_frames(&name),
            pos: 0,
            rg_track: None,
            rg_album: None,
        }))
    }
}

/// An engine on a virtual device, with an event subscription.
pub struct Harness {
    pub engine: Engine,
    pub clock: NullClock,
    pub events: EventReceiver,
    pub bus: EventBus,
    /// Directory holding the (empty) `*.tst` files the registry sniffs.
    _dir: tempfile::TempDir,
    /// Full paths of the queued tracks, in order.
    pub paths: Vec<String>,
}

impl Harness {
    /// New engine with the given queue. The names are created as empty files in
    /// a temporary directory, because `DecoderRegistry::open` sniffs the source
    /// before it probes; the test factory itself ignores the content.
    pub fn new(paths: &[&str]) -> Harness {
        Harness::with_config(paths, EngineConfig::default())
    }

    /// New engine with an explicit configuration.
    pub fn with_config(names: &[&str], cfg: EngineConfig) -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut paths = Vec::new();
        for name in names {
            let path = dir.path().join(name);
            std::fs::write(&path, b"").expect("writing the placeholder track");
            paths.push(path.display().to_string());
        }
        let (output, clock) = NullOutput::pair();
        let bus = EventBus::new();
        let events = bus.subscribe();
        let mut registry = rustamp_core::audio::DecoderRegistry::new();
        registry.register(TestFactory::arc());
        let mut engine =
            Engine::new(registry, Box::new(output), bus.clone(), cfg).expect("engine construction");
        engine.set_queue(paths.iter().map(TrackSource::path).collect());
        Harness {
            engine,
            clock,
            events,
            bus,
            _dir: dir,
            paths,
        }
    }

    /// Every event published since the last call, in order.
    pub fn drain_events(&mut self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    /// Poll and pump until `cond` holds or `timeout` elapses: the wait
    /// measures the run's own condition with a wall-clock stall escape (the
    /// M10 rule), not a pump count — a pump only advances the run while the
    /// ring holds audio, so a pump budget can expire on underruns without
    /// the run having made progress (the proxy M24 replaced in the lib).
    pub fn wait_pumping(
        &mut self,
        mut cond: impl FnMut(&Engine) -> bool,
        timeout: Duration,
    ) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            self.engine.poll().expect("poll");
            if cond(&self.engine) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            self.clock.pump(256);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Poll and pump in 256-frame steps until `cond` holds. Returns whether it
    /// held within `max_frames`. Deprecated in favor of `wait_pumping`.
    pub fn wait_until(&mut self, mut cond: impl FnMut(&Engine) -> bool, max_frames: u64) -> bool {
        if cond(&self.engine) {
            return true;
        }
        let mut pumped = 0;
        while pumped < max_frames {
            self.engine.poll().expect("poll");
            if cond(&self.engine) {
                return true;
            }
            self.clock.pump(256);
            pumped += 256;
            std::thread::sleep(Duration::from_millis(1));
        }
        cond(&self.engine)
    }

    /// Poll (and sleep, but never pump) until `cond` holds.
    pub fn wait_idle(&mut self, mut cond: impl FnMut(&Engine) -> bool, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            self.engine.poll().expect("poll");
            if cond(&self.engine) {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Wait until the decode thread has opened the track at `index` and its
    /// metadata has arrived.
    pub fn wait_track(&mut self, index: u32) -> bool {
        self.wait_idle(
            |e| e.current_index() == Some(index) && e.duration_frames().is_some(),
            WAIT,
        )
    }

    /// Wait until the decode thread has produced the whole queue (its ring is
    /// then full of the tracks, in order).
    pub fn wait_buffered(&mut self) -> bool {
        self.wait_idle(|e| e.end_of_stream().is_some(), WAIT)
    }

    /// Seek to `target` frames and drive the pipeline until the seek is fully
    /// in effect: the ring has been drained, the clock re-based to exactly
    /// `target`, and the decode thread has written fresh audio after it.
    ///
    /// The drain happens in the render callback, so this has to pump. The
    /// position is checked after every poll, so it is observed at the target
    /// even though the pumping continues afterwards.
    pub fn seek_and_settle(&mut self, target: u64) -> bool {
        self.engine.seek_frames(target).expect("seek");
        self.settle_seek(target)
    }

    /// Drive the pipeline until a seek that has already been requested is in
    /// effect: the ring drained and the clock re-based, so the position is
    /// exactly `target`.
    ///
    /// The drain happens in the render callback, so this has to pump.
    pub fn settle_seek(&mut self, target: u64) -> bool {
        let deadline = Instant::now() + WAIT;
        loop {
            self.engine.poll().expect("poll");
            if self.engine.position_frames() == target {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            self.clock.pump(64);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Drive the start of a run that found a previous run's audio in the ring
    /// until it is complete: the render callback has dropped that audio and
    /// the engine has re-based its clock, so the decode thread is free to
    /// write the run's first frame. A no-op when nothing is pending.
    ///
    /// The drain happens in the render callback, so this has to pump. Every
    /// pump happens before the re-base (the condition is checked right after
    /// each poll), and until the re-base the decode thread writes nothing, so
    /// the pumped frames are silence that the re-base leaves out of the
    /// position.
    pub fn settle_drain(&mut self) -> bool {
        let deadline = Instant::now() + WAIT;
        loop {
            self.engine.poll().expect("poll");
            if !self.engine.is_drain_pending() && !self.engine.is_start_pending() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            self.clock.pump(64);
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Wait (without pumping) until at least `frames` are buffered and return
    /// the position at that moment. Every rendered frame from that position on
    /// is real audio, so a caller can compare it against the source.
    pub fn wait_refilled(&mut self, frames: u64) -> Option<u64> {
        if self.wait_idle(|e| e.buffered_frames() >= frames, WAIT) {
            Some(self.engine.position_frames())
        } else {
            None
        }
    }
}

/// The samples the device receives for the `n` frames of `seed` starting at
/// `from`, when the chain leaves them alone: the source pattern, put through
/// the converter's finishing (see [`finished`]).
pub fn expected(seed: u64, from: u64, n: u64) -> Vec<f32> {
    source(seed, from, n).into_iter().map(finished).collect()
}

/// The source pattern as the decoder produces it, before the chain.
pub fn source(seed: u64, from: u64, n: u64) -> Vec<f32> {
    (0..n).map(|i| pattern(seed, from + i)).collect()
}

/// The converter's finishing, the last level decision of the chain (see
/// `rustamp_audio::convert`): a non-finite sample becomes silence, a
/// subnormal becomes zero, and a sample above full scale is clamped to
/// `±1.0`.
///
/// The tests model it, because the device receives the *finished* samples:
/// the source pattern reaches above full scale (`0.1..=1.1`), and the
/// converter's ceiling holds every sample above `1.0` at exactly `1.0`. A
/// test that compares the rendered audio with the source must therefore
/// expect the finished source — and a test that scales the source itself
/// (a stage gain, a ReplayGain factor) must scale the *raw* source and
/// finish the product, because the chain runs before the converter.
pub fn finished(x: f32) -> f32 {
    if !x.is_finite() || x.is_subnormal() {
        0.0
    } else {
        x.clamp(-1.0, 1.0)
    }
}
