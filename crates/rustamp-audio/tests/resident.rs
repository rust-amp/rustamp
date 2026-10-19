//! Engine playback through the decoder resident (RFC 0006 §5).
//!
//! Headless engine tests over a fake in-process resident: byte-stream control
//! pairs drive the production framing and the real named rings, but no helper
//! binary is spawned. Run in the Hyper-V test VM, never on the host. The two
//! real-process cases (spawning the symphonia helper, killing it mid-stream)
//! are VM checks listed in `docs/tasks/M15-engine-resident.md`, not tests here.

#![cfg(windows)]

use std::io::{Cursor, Read, Write};
use std::sync::mpsc;
use std::sync::{Arc, atomic};
use std::thread;
use std::time::{Duration, Instant};

use rustamp_audio::resident::HostDecoder;
use rustamp_audio::{Engine, EngineConfig, TrackSource};
use rustamp_core::audio::{
    AudioSpec, Confidence, Decoder, DecoderFactory, DecoderRegistry, MediaSource, ProbeHint,
};
use rustamp_core::bus::{EventBus, EventReceiver};
use rustamp_core::command::Command;
use rustamp_core::error::{Error as CoreError, ErrorKind, Result as CoreResult};
use rustamp_core::event::Event;
use rustamp_core::state::TransportState;
use rustamp_mediahost::control::{ControlPeer, Liveness};
use rustamp_mediahost::decoder::{
    DecoderInfo, DecoderResident, DecoderSession, ResidentDecoder, serve_decoder,
};
use rustamp_mediahost::pcm::NamedPcmRings;
use rustamp_mediahost::{MediaHostError, Result as HostResult};
use rustamp_output::{NullClock, NullOutput};
use rustamp_proto::host::{MediaLocationWire, PcmSpecWire};

mod common;

use common::{expected, pattern};

/// Stall escape for every wait in this harness and for the resident
/// decoder's read/seek timeout (the M10 flakiness note's rule): a bound so
/// generous that only a real stall reaches it, with the message naming the
/// step — never a wall-clock bound a healthy-but-slow VM schedule could
/// trip. The 5 s bounds this file carried before are the flake class M24
/// proved in this guest (docs/tasks/M24-stubborn-three.md §1.3, M25).
const WAIT: Duration = Duration::from_secs(30);
/// Liveness policy for the in-process control peers. The framework default
/// (`Liveness::default()`: 100 ms interval, 5 s timeout) is the same
/// wall-clock trip wire, still present here after M24/M25 fixed the lib and
/// the dechost/dsphost suites: a dead resident never answers a Ping and is
/// caught by the control EOF regardless, so only the trip wire moves.
const LIVENESS: Liveness = Liveness {
    interval: Duration::from_millis(100),
    timeout: WAIT,
};

/// Fake resident serving the same synthetic pattern the in-process
/// `TestFactory` serves, so resident and in-process tracks are comparable.
/// The file stem `track<seed>_<frames>.rtst` sets the content; the stem
/// `die<seed>_<frames>.rtst` serves the same content and then fails loudly,
/// the way a resident that dies mid-track does; any other stem is refused.
struct PatternResident;

struct PatternDecoder {
    seed: u64,
    total: u64,
    pos: u64,
    /// The resident fails loudly once its frames are spent.
    dies: bool,
}

/// What the fake resident serves for one file stem.
#[derive(Clone, Copy)]
enum Script {
    /// The pattern for that many frames.
    Pattern { seed: u64, frames: u64 },
    /// The pattern for that many frames, then a loud failure.
    Die { seed: u64, frames: u64 },
}

fn parse_stem(name: &str) -> Option<Script> {
    let stem = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = stem.strip_suffix(".rtst").unwrap_or(stem);
    let (dies, head) = match stem.strip_prefix("track") {
        Some(head) => (false, head),
        None => (true, stem.strip_prefix("die")?),
    };
    let (seed, frames) = head.split_once('_')?;
    let seed: u64 = seed.parse().ok()?;
    let frames: u64 = frames.parse().ok()?;
    Some(if dies {
        Script::Die { seed, frames }
    } else {
        Script::Pattern { seed, frames }
    })
}

impl DecoderResident for PatternResident {
    fn open(&self, location: MediaLocationWire) -> HostResult<Box<dyn ResidentDecoder>> {
        let MediaLocationWire::Url(name) = location else {
            return Err(MediaHostError::OperationFailed(
                "fake resident takes names as URLs".into(),
            ));
        };
        match parse_stem(&name) {
            Some(Script::Pattern { seed, frames }) => Ok(Box::new(PatternDecoder {
                seed,
                total: frames,
                pos: 0,
                dies: false,
            })),
            Some(Script::Die { seed, frames }) => Ok(Box::new(PatternDecoder {
                seed,
                total: frames,
                pos: 0,
                dies: true,
            })),
            None => Err(MediaHostError::OperationFailed(
                "fake resident refuses this file".into(),
            )),
        }
    }
}

impl ResidentDecoder for PatternDecoder {
    fn info(&self) -> DecoderInfo {
        DecoderInfo {
            spec: PcmSpecWire {
                sample_rate_hz: common::SPEC.sample_rate,
                channels: 2,
            },
            duration_ms: Some(self.total * 1000 / u64::from(common::SPEC.sample_rate)),
            seekable: true,
            metadata: Default::default(),
        }
    }

    fn read(&mut self, out: &mut [f32]) -> HostResult<usize> {
        let channels = 2usize;
        if self.dies && self.pos >= self.total {
            // The resident dies mid-track: the host turns this into a
            // `DecoderError`, and the engine must surface it like a local
            // decode error (RFC 0006 §5.7).
            return Err(MediaHostError::OperationFailed(
                "the fake resident died mid-track".into(),
            ));
        }
        let n = (out.len() / channels).min((self.total - self.pos) as usize);
        for i in 0..n {
            let v = pattern(self.seed, self.pos + i as u64);
            out[i * channels] = v;
            out[i * channels + 1] = v;
        }
        self.pos += n as u64;
        Ok(n)
    }

    fn seek(&mut self, to_ms: u64) -> HostResult<u64> {
        let frames = to_ms * u64::from(common::SPEC.sample_rate) / 1000;
        self.pos = frames.min(self.total);
        Ok(self.pos * 1000 / u64::from(common::SPEC.sample_rate))
    }

    fn stats(&self) -> (u64, Option<u32>) {
        (0, None)
    }
}

struct ByteReader {
    rx: mpsc::Receiver<Vec<u8>>,
    bytes: Cursor<Vec<u8>>,
}

struct ByteWriter(mpsc::Sender<Vec<u8>>);

impl Read for ByteReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        loop {
            let count = self.bytes.read(out)?;
            if count > 0 {
                return Ok(count);
            }
            match self.rx.recv() {
                Ok(bytes) => self.bytes = Cursor::new(bytes),
                Err(_) => return Ok(0),
            }
        }
    }
}

impl Write for ByteWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .send(bytes.to_vec())
            .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Factory under test: opens `*.rtst` through a fake resident without
/// spawning a process. The only test-only seam; production uses
/// [`rustamp_audio::resident::ResidentFactory`] with a real helper binary.
struct FakeResidentFactory;

impl DecoderFactory for FakeResidentFactory {
    fn name(&self) -> &str {
        "fake-resident"
    }

    fn extensions(&self) -> &[&str] {
        &["rtst"]
    }

    fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
        if hint.extension == Some("rtst") {
            Confidence::EXTENSION
        } else {
            Confidence::NONE
        }
    }

    fn open(&self, source: MediaSource) -> CoreResult<Box<dyn Decoder>> {
        let name = match &source {
            MediaSource::Path(p) => p.display().to_string(),
            MediaSource::Url(u) => u.clone(),
            MediaSource::Reader { .. } => {
                return Err(CoreError::Unsupported("fake resident takes paths only"));
            }
        };
        let (a_tx, b_rx) = mpsc::channel();
        let (b_tx, a_rx) = mpsc::channel();
        let client = ControlPeer::start(
            ByteReader {
                rx: a_rx,
                bytes: Cursor::new(Vec::new()),
            },
            ByteWriter(a_tx),
            LIVENESS,
            false,
        )
        .map_err(|e| CoreError::Decode(e.to_string()))?;
        let host = ControlPeer::start(
            ByteReader {
                rx: b_rx,
                bytes: Cursor::new(Vec::new()),
            },
            ByteWriter(b_tx),
            LIVENESS,
            true,
        )
        .map_err(|e| CoreError::Decode(e.to_string()))?;
        let (done, result) = mpsc::channel();
        thread::spawn(move || {
            let outcome = serve_decoder(&host, &NamedPcmRings, &PatternResident);
            let _ = done.send(outcome);
        });
        let session = DecoderSession::open(client, NamedPcmRings, MediaLocationWire::Url(name), 64)
            .map_err(|e| CoreError::Decode(e.to_string()))?;
        // Reap the host thread when the decoder drops: without this the test
        // process would leak one thread per opened track.
        let decoder = ReapedDecoder {
            inner: Some(
                HostDecoder::new(session, WAIT, WAIT)
                    .expect("the handshake the session validated is adoptable"),
            ),
            done: Some(result),
        };
        Ok(Box::new(decoder))
    }
}

struct ReapedDecoder {
    inner: Option<HostDecoder<NamedPcmRings>>,
    done: Option<mpsc::Receiver<HostResult<()>>>,
}

impl Drop for ReapedDecoder {
    fn drop(&mut self) {
        // Drop the session first (it sends Shutdown to the host), then reap
        // the host thread so no test leaks one thread per opened track.
        drop(self.inner.take());
        if let Some(done) = self.done.take() {
            let _ = done.recv_timeout(WAIT);
        }
    }
}

impl Decoder for ReapedDecoder {
    fn spec(&self) -> AudioSpec {
        self.inner.as_ref().expect("decoder in use").spec()
    }

    fn read(&mut self, out: &mut [f32]) -> CoreResult<usize> {
        self.inner.as_mut().expect("decoder in use").read(out)
    }

    fn seek(&mut self, to: Duration) -> CoreResult<Duration> {
        self.inner.as_mut().expect("decoder in use").seek(to)
    }

    fn duration(&self) -> Option<Duration> {
        self.inner.as_ref().expect("decoder in use").duration()
    }

    fn metadata(&self) -> rustamp_core::meta::TrackMetadata {
        self.inner.as_ref().expect("decoder in use").metadata()
    }

    fn stream_info(&self) -> rustamp_core::state::StreamInfo {
        self.inner.as_ref().expect("decoder in use").stream_info()
    }

    fn is_seekable(&self) -> bool {
        self.inner.as_ref().expect("decoder in use").is_seekable()
    }

    fn skipped_packets(&self) -> u64 {
        self.inner
            .as_ref()
            .expect("decoder in use")
            .skipped_packets()
    }
}

struct Harness {
    engine: Engine,
    clock: NullClock,
    events: EventReceiver,
    _dir: tempfile::TempDir,
}

fn track_path(dir: &tempfile::TempDir, name: &str) -> String {
    let path = dir.path().join(name);
    std::fs::write(&path, b"").expect("placeholder track");
    path.display().to_string()
}

impl Harness {
    fn new(names: &[&str]) -> Harness {
        let dir = tempfile::tempdir().expect("temp dir");
        let (output, clock) = NullOutput::pair();
        let bus = EventBus::new();
        let events = bus.subscribe();
        let mut registry = DecoderRegistry::new();
        registry.register(common::TestFactory::arc());
        registry.register(Arc::new(FakeResidentFactory));
        // Bit-exact comparisons: the default-on limiter would clamp the
        // synthetic pattern's overs.
        let mut engine = Engine::new(registry, Box::new(output), bus, EngineConfig::default())
            .expect("engine construction");
        engine.set_limiter_enabled(false);
        engine.set_queue(
            names
                .iter()
                .map(|name| {
                    let path = track_path(&dir, name);
                    TrackSource::path(path)
                })
                .collect(),
        );
        Harness {
            engine,
            clock,
            events,
            _dir: dir,
        }
    }

    fn drain_events(&mut self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    /// Poll and pump until `cond` holds or `timeout` elapses: the wait
    /// measures the run's own condition with a wall-clock stall escape (the
    /// M10 rule), not a pump count — a pump only advances the run while the
    /// ring holds audio, so a pump budget can expire on underruns without
    /// the run having made progress (the proxy M24 replaced in the lib).
    fn wait_pumping(&mut self, mut cond: impl FnMut(&Engine) -> bool, timeout: Duration) -> bool {
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
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn wait_track(&mut self, index: u32) -> bool {
        let deadline = Instant::now() + WAIT;
        loop {
            self.engine.poll().expect("poll");
            if self.engine.current_index() == Some(index) && self.engine.duration_frames().is_some()
            {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn wait_buffered(&mut self) -> bool {
        let deadline = Instant::now() + WAIT;
        loop {
            self.engine.poll().expect("poll");
            if self.engine.end_of_stream().is_some() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn settle_seek(&mut self, target: u64) -> bool {
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
            thread::sleep(Duration::from_millis(1));
        }
    }
}

static NEXT_ID: atomic::AtomicU64 = atomic::AtomicU64::new(1);

#[test]
fn resident_track_plays_bit_exact_to_sticky_eos() {
    let mut h = Harness::new(&["track7_3000.rtst"]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0), "the resident track must open");
    // The handshake replays the fake resident's format and length.
    assert_eq!(h.engine.source_spec(), Some(common::SPEC));
    // Wire truth (M15): the resident reports whole milliseconds, so a
    // 3000-frame track at 44.1 kHz is 3000*1000/44100 = 68 ms on the wire
    // and 0.068*44100 = 2998.8 -> 2999 device frames back. The content is
    // still 3000 frames; only the duration quantizes.
    assert_eq!(h.engine.duration_frames(), Some(2999));
    assert!(h.wait_buffered(), "the resident track must buffer");

    let mut rec = Vec::new();
    let mut pumped = 0u64;
    while pumped < 3000 {
        let step = 500.min(3000 - pumped);
        h.clock.pump_record(step, &mut rec);
        pumped += step;
        h.engine.poll().unwrap();
    }
    let left: Vec<f32> = rec.iter().step_by(2).copied().collect();
    assert_eq!(left.len(), 3000);
    assert_eq!(&left[..], &expected(7, 0, 3000)[..]);

    // Sticky end of stream: the run finishes only once the ring is drained,
    // and the timeline freezes exactly at the content end.
    assert!(
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
        "the run must finish"
    );
    // With duration quantized to 2999 frames (M15), position_frames() clamps to duration.
    assert_eq!(h.engine.position_frames(), 2999);
    assert_eq!(h.engine.buffered_frames(), 0);
    let events = h.drain_events();
    assert!(events.contains(&Event::PlaylistEnded), "{events:?}");
}

#[test]
fn seek_in_resident_track_plays_the_target() {
    let mut h = Harness::new(&["track7_44100.rtst"]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    // Wire truth (M15): seeks quantize to whole milliseconds, so the target
    // must be ms-exact for a bit-exact assertion. 450 ms * 44.1 = 19845
    // frames round-trips exactly; 20000 does not.
    let target = 19_845u64;
    h.engine.seek_frames(target).unwrap();
    assert!(h.settle_seek(target), "seek did not land on {target}");

    // Wait for fresh post-seek audio without pumping silence in between, then
    // compare against the pattern at the settled position.
    let deadline = Instant::now() + WAIT;
    let p0 = loop {
        h.engine.poll().unwrap();
        if h.engine.buffered_frames() >= 4096 {
            break h.engine.position_frames();
        }
        if Instant::now() >= deadline {
            panic!("the ring must refill after the seek");
        }
        thread::sleep(Duration::from_millis(1));
    };
    let mut rec = Vec::new();
    h.clock.pump_record(1024, &mut rec);
    h.engine.poll().unwrap();
    assert_eq!(h.engine.position_frames(), p0 + 1024);
    let left: Vec<f32> = rec.iter().step_by(2).copied().collect();
    assert_eq!(&left[..], &expected(7, p0, 1024)[..]);
}

#[test]
fn resident_to_inprocess_source_change_is_gapless() {
    // The queue mixes factories: the first track decodes out-of-process, the
    // second in-process. The seam must be exact and the clock continuous —
    // the ring drain and the timeline do not care which factory opened the
    // decoder, because both present the same Decoder trait.
    let mut h = Harness::new(&["track7_3000.rtst", "track2_3000.tst"]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    assert!(h.wait_buffered(), "both tracks must buffer");

    let total = 6000u64;
    assert_eq!(h.engine.buffered_frames(), total);
    let mut rec = Vec::new();
    let mut pumped = 0u64;
    while pumped < total {
        let step = 500.min(total - pumped);
        h.clock.pump_record(step, &mut rec);
        pumped += step;
        if pumped < total {
            h.engine.poll().unwrap();
        }
    }
    let left: Vec<f32> = rec.iter().step_by(2).copied().collect();
    assert_eq!(left.len() as u64, total);
    assert_eq!(&left[..3000], &expected(7, 0, 3000)[..]);
    assert_eq!(&left[3000..], &expected(2, 0, 3000)[..]);
    assert!(!left.contains(&0.0), "the seam must not contain silence");
    assert_eq!(left[2999], pattern(7, 2999));
    assert_eq!(left[3000], pattern(2, 0));

    assert!(
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
        "the mixed run must finish"
    );
    assert_eq!(h.engine.position_frames(), 3000);
    assert_eq!(h.engine.current_index(), Some(1));
    let events = h.drain_events();
    assert!(events.contains(&Event::PlaylistEnded), "{events:?}");
}

#[test]
fn resident_refusal_is_a_loud_open_error() {
    // No engine run: the factory must fail the open loudly without spawning
    // anything, exactly like an undecodable file in-process.
    let factory = FakeResidentFactory;
    let outcome = factory.open(MediaSource::Path(
        format!(
            "track-refuse-{}.rtst",
            NEXT_ID.fetch_add(1, atomic::Ordering::Relaxed)
        )
        .into(),
    ));
    // The stem does not parse as track<seed>_<frames>, so the fake resident
    // refuses it with OperationFailed -> Decode, never EOS.
    match outcome {
        Err(CoreError::Decode(_)) => {}
        Err(error) => panic!("refusal must be Decode, got {error}"),
        Ok(_) => panic!("refusal must be loud, never a decoder"),
    }
}

#[test]
fn a_resident_that_dies_mid_track_fails_the_track_loudly() {
    // RFC 0006 §5.7: the host reports the failure and the engine surfaces it
    // as an event exactly like a local decode error — never as the end of the
    // track, never as silence, and never as a hang.
    let mut h = Harness::new(&["die7_3000.rtst"]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0), "the resident track must open");

    // The failure arrives as an error event.
    let deadline = Instant::now() + WAIT;
    let error = loop {
        h.engine.poll().unwrap();
        let error = h.drain_events().into_iter().find_map(|event| match event {
            Event::Error(info) => Some(info),
            _ => None,
        });
        if let Some(info) = error {
            break info;
        }
        assert!(
            Instant::now() < deadline,
            "the resident's death must be reported"
        );
        thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(error.kind, ErrorKind::Decode, "{}", error.message);
    assert!(
        error.message.contains("resident"),
        "the report must name the resident: {}",
        error.message
    );

    // And the track ends: the run does not hang and the transport parks.
    assert!(
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
        "the failed track must end the run"
    );
}

#[test]
fn seek_past_the_end_clamps_through_the_wire() {
    let mut h = Harness::new(&["track7_3000.rtst"]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    // The engine clamps to the 3000-frame duration; the millisecond wire
    // quantizes the round trip, so the resident lands within one millisecond
    // (~44 frames at 44.1 kHz) of it — coarse tolerance, by wire design.
    h.engine.seek_frames(1_000_000).unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        h.engine.poll().unwrap();
        let pos = h.engine.position_frames();
        if (2950..=3000).contains(&pos) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "seek past the end never settled near 3000 (at {pos})"
        );
        h.clock.pump(64);
        thread::sleep(Duration::from_millis(1));
    }
    // The clamped seek terminates: pumping the rest finishes the run with an
    // empty ring, exactly like an in-process seek past the end.
    assert!(
        h.wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT),
        "the clamped run must finish"
    );
    assert_eq!(h.engine.buffered_frames(), 0);
}

#[test]
fn resident_commands_reach_the_engine_unchanged() {
    // Smoke test: pause/resume/seek commands behave the same on a resident
    // track as on an in-process one (the transport path is untouched).
    let mut h = Harness::new(&["track7_44100.rtst"]);
    h.engine.play_index(0).unwrap();
    assert!(h.wait_track(0));
    h.engine.apply(&Command::Pause).unwrap();
    assert_eq!(h.engine.transport(), TransportState::Paused);
    h.engine.apply(&Command::Play).unwrap();
    assert_eq!(h.engine.transport(), TransportState::Playing);
    // Wire truth (M15): seeks quantize to whole milliseconds, so the smoke
    // target must be ms-exact. 20 ms * 44.1 = 882 frames round-trips exactly;
    // 1000 does not.
    h.engine.seek_frames(882).unwrap();
    assert!(h.settle_seek(882));
    let _ = h.drain_events();
    assert_eq!(
        h.engine.state().current_index,
        Some(0),
        "transport is unchanged by the decode source"
    );
}
