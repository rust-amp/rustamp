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
use rustamp_core::state::StreamInfo;
use rustamp_output::{NullClock, NullOutput};

/// Format of every synthetic track.
pub const SPEC: AudioSpec = AudioSpec::new(44100, 2);

/// Frames per synthetic track unless the path says otherwise.
pub const TRACK_FRAMES: u64 = 3000;

/// The sample value at `frame` for track `seed`: never zero, so silence in the
/// output is always detectable.
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

    fn skipped_packets(&self) -> u64 {
        0
    }
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
        Ok(Box::new(PatternDecoder {
            spec: SPEC,
            seed: track_seed(&name),
            total: track_frames(&name),
            pos: 0,
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

    /// Poll and pump in 256-frame steps until `cond` holds. Returns whether it
    /// held within `max_frames`.
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
            Duration::from_secs(5),
        )
    }

    /// Wait until the decode thread has produced the whole queue (its ring is
    /// then full of the tracks, in order).
    pub fn wait_buffered(&mut self) -> bool {
        self.wait_idle(|e| e.end_of_stream().is_some(), Duration::from_secs(5))
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
        let deadline = Instant::now() + Duration::from_secs(5);
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

    /// Wait (without pumping) until at least `frames` are buffered and return
    /// the position at that moment. Every rendered frame from that position on
    /// is real audio, so a caller can compare it against the source.
    pub fn wait_refilled(&mut self, frames: u64) -> Option<u64> {
        if self.wait_idle(|e| e.buffered_frames() >= frames, Duration::from_secs(5)) {
            Some(self.engine.position_frames())
        } else {
            None
        }
    }
}

/// Frames of the newest `n` frames of `seed` starting at `from`.
pub fn expected(seed: u64, from: u64, n: u64) -> Vec<f32> {
    (0..n).map(|i| pattern(seed, from + i)).collect()
}
