//! Endpoint-buffer tests, not just flush-call counters. No real output, window
//! or process is opened. The orchestrator runs these in the test VM only.

mod common;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{SPEC, TestFactory, expected};
use rustamp_audio::{Engine, EngineConfig, TrackSource};
use rustamp_core::audio::{
    AudioSpec, Confidence, Decoder, DecoderFactory, DecoderRegistry, MediaSource, OutputDevice,
    OutputFormat, ProbeHint, RenderFn,
};
use rustamp_core::bus::{EventBus, EventReceiver};
use rustamp_core::error::{Error, ErrorKind, Result};
use rustamp_core::event::Event;
use rustamp_core::state::TransportState;

/// Stall escape for every wait in this harness (the M10 flakiness note's
/// rule): a bound so generous that only a real stall reaches it, with the
/// message naming the step — never a wall-clock bound a healthy-but-slow
/// VM schedule could trip. The 5 s bound this file carried before is the
/// flake class M24 proved in this guest (docs/tasks/M24-stubborn-three.md §1.3, M25, M26).
const WAIT: Duration = Duration::from_secs(30);
const RING: usize = 8192;
const TARGET: u64 = 20_000;

/// The underlying deterministic decoder is unchanged on a refused seek.
struct SeekFactory(bool, Arc<AtomicU64>);
struct SeekDecoder {
    inner: Box<dyn Decoder>,
    refuse: bool,
    dropped: Arc<AtomicU64>,
}

impl DecoderFactory for SeekFactory {
    fn name(&self) -> &str {
        "seek-test"
    }
    fn extensions(&self) -> &[&str] {
        &["tst"]
    }
    fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
        TestFactory.probe(hint)
    }
    fn open(&self, source: MediaSource) -> Result<Box<dyn Decoder>> {
        Ok(Box::new(SeekDecoder {
            inner: TestFactory.open(source)?,
            refuse: self.0,
            dropped: Arc::clone(&self.1),
        }))
    }
}

impl Decoder for SeekDecoder {
    fn spec(&self) -> AudioSpec {
        self.inner.spec()
    }
    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        self.inner.read(out)
    }
    fn duration(&self) -> Option<Duration> {
        self.inner.duration()
    }
    fn seek(&mut self, to: Duration) -> Result<Duration> {
        if self.refuse {
            Err(Error::Decode("scripted seek refusal".into()))
        } else {
            self.inner.seek(to)
        }
    }
}

impl Drop for SeekDecoder {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::Release);
    }
}

struct Endpoint {
    render: Option<RenderFn>,
    pending: VecDeque<f32>,
    paused: bool,
    rendered: u64,
    flushes: u64,
    opens: u64,
    starts: u64,
    resumes: u64,
    closes: u64,
    refuse_flush: bool,
    /// A fatal mid-run stream error the device latched (RFC 0012), like
    /// WASAPI's `ERR_GONE` or cpal's error callback. Cleared by a successful
    /// `open`, exactly as all three production backends clear theirs.
    failed: Arc<AtomicBool>,
    /// The endpoint is physically gone, so `open` fails. Models the half of
    /// RFC 0012 §4.3 item 8 where Play "will ... return a clear `Error::Output`
    /// naming the missing device".
    absent: bool,
    reset_clock_on_close: bool,
    refuse_start: bool,
    granted: AudioSpec,
}

struct BufferedOutput(Arc<Mutex<Endpoint>>, Arc<AtomicBool>);

impl OutputDevice for BufferedOutput {
    fn name(&self) -> &str {
        "buffered-flush-test"
    }
    fn open(&mut self, spec: AudioSpec) -> Result<OutputFormat> {
        assert_eq!(spec, SPEC);
        let mut ep = self.0.lock().unwrap();
        ep.opens += 1;
        if ep.absent {
            return Err(Error::Output(
                "buffered-flush-test: the endpoint is gone".into(),
            ));
        }
        ep.failed.store(false, Ordering::Release);
        Ok(OutputFormat {
            spec: ep.granted,
            buffer_frames: 256,
        })
    }
    fn start(&mut self, render: RenderFn) -> Result<()> {
        let mut ep = self.0.lock().unwrap();
        assert!(ep.render.is_none());
        if ep.refuse_start {
            return Err(Error::Output("buffered-flush-test: start refused".into()));
        }
        ep.render = Some(render);
        ep.paused = false;
        ep.starts += 1;
        Ok(())
    }
    fn pause(&mut self) -> Result<()> {
        self.0.lock().unwrap().paused = true;
        Ok(())
    }
    fn resume(&mut self) -> Result<()> {
        let mut ep = self.0.lock().unwrap();
        assert!(ep.render.is_some(), "cannot resume a closed stream");
        ep.paused = false;
        ep.resumes += 1;
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        let mut ep = self.0.lock().unwrap();
        ep.flushes += 1;
        if ep.refuse_flush {
            // Like cpal: report refusal and leave the endpoint untouched.
            return Err(Error::Unsupported("scripted endpoint flush"));
        }
        ep.pending.clear();
        Ok(())
    }
    fn played_frames(&self) -> u64 {
        self.0.lock().unwrap().rendered
    }
    fn stream_error(&self) -> Option<Error> {
        if self.1.load(Ordering::Acquire) {
            Some(Error::Output("buffered-flush-test: the stream died".into()))
        } else {
            None
        }
    }
    fn latency(&self) -> Duration {
        SPEC.duration_of(256)
    }
    fn close(&mut self) -> Result<()> {
        let mut ep = self.0.lock().unwrap();
        ep.paused = true;
        ep.render = None;
        ep.pending.clear();
        ep.closes += 1;
        if ep.reset_clock_on_close {
            ep.rendered = 0;
        }
        // Real backends release the stream but can return its latched failure.
        if ep.failed.load(Ordering::Acquire) {
            return Err(Error::Output("buffered-flush-test: old stream died".into()));
        }
        Ok(())
    }
}

struct Harness {
    engine: Engine,
    ep: Arc<Mutex<Endpoint>>,
    events: EventReceiver,
    dropped: Arc<AtomicU64>,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn new(refuse_seek: bool, refuse_flush: bool, short: bool) -> Self {
        let failed = Arc::new(AtomicBool::new(false));
        let ep = Arc::new(Mutex::new(Endpoint {
            render: None,
            pending: VecDeque::new(),
            paused: true,
            rendered: 0,
            flushes: 0,
            opens: 0,
            starts: 0,
            resumes: 0,
            closes: 0,
            refuse_flush,
            failed: Arc::clone(&failed),
            absent: false,
            reset_clock_on_close: false,
            refuse_start: false,
            granted: SPEC,
        }));
        let bus = EventBus::new();
        let events = bus.subscribe();
        let mut registry = DecoderRegistry::new();
        let dropped = Arc::new(AtomicU64::new(0));
        registry.register(Arc::new(SeekFactory(refuse_seek, Arc::clone(&dropped))));
        let mut engine = Engine::new(
            registry,
            Box::new(BufferedOutput(Arc::clone(&ep), failed)),
            bus,
            EngineConfig {
                ring_frames: RING,
                ..EngineConfig::default()
            },
        )
        .unwrap();
        // Isolate seek/flush with bit-exact samples (the pattern exceeds the
        // default limiter ceiling). No DSP history should hide a missing frame.
        engine.set_limiter_enabled(false);
        let dir = tempfile::tempdir().unwrap();
        let names = if short {
            ["track1_3000.tst", "track2_3000.tst"]
        } else {
            ["track1_44100.tst", "track2_44100.tst"]
        };
        let paths: Vec<_> = names
            .iter()
            .map(|name| {
                let path = dir.path().join(name);
                std::fs::write(&path, b"").unwrap();
                TrackSource::path(path)
            })
            .collect();
        engine.set_queue(paths);
        Self {
            engine,
            ep,
            events,
            dropped,
            _dir: dir,
        }
    }

    /// Latch a fatal mid-run stream error on the endpoint (RFC 0012).
    fn fail_stream(&self) {
        self.ep
            .lock()
            .unwrap()
            .failed
            .store(true, Ordering::Release);
    }

    /// Make the endpoint present or absent for the next `open`.
    fn set_absent(&self, absent: bool) {
        self.ep.lock().unwrap().absent = absent;
    }

    fn wait(&mut self, mut condition: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            self.engine.poll().unwrap();
            if condition(self) {
                return;
            }
            assert!(Instant::now() < deadline, "condition did not settle");
            std::thread::yield_now();
        }
    }

    /// Render into the endpoint without hearing anything. The device clock
    /// counts rendered frames, matching both production backends.
    fn stage(&self, frames: usize) {
        let mut ep = self.ep.lock().unwrap();
        assert!(!ep.paused);
        let mut buf = vec![0.0; ep.granted.samples_for(frames)];
        ep.render.as_mut().expect("open render callback")(&mut buf);
        ep.pending.extend(buf);
        ep.rendered += frames as u64;
    }

    /// Hear only the retained endpoint buffer, never implicitly refill it.
    fn hear(&self) -> Vec<f32> {
        let mut ep = self.ep.lock().unwrap();
        assert!(!ep.paused);
        let channels = usize::from(ep.granted.channels);
        ep.pending.drain(..).step_by(channels).collect()
    }

    fn prime(&mut self) {
        self.engine.play_index(0).unwrap();
        self.wait(|h| h.engine.buffered_frames() == RING as u64);
        self.stage(256);
        assert_eq!(self.ep.lock().unwrap().pending.len(), 512);
    }

    /// A full ring can leave the worker inside a block write. Make room for
    /// that block, but retain every callback sample in the endpoint. Stop as
    /// soon as a seek drain is due or the refusal is reported.
    fn drive_to_seek_result(&mut self) {
        let deadline = Instant::now() + WAIT;
        let flushes_before = self.ep.lock().unwrap().flushes;
        loop {
            self.engine.poll().unwrap();
            if self.engine.is_drain_pending()
                || self.engine.last_error().is_some()
                || self.ep.lock().unwrap().flushes > flushes_before
            {
                return;
            }
            assert!(Instant::now() < deadline, "seek never reached decoder");
            if self.engine.buffered_frames() >= 1024 {
                self.stage(1024);
            }
            std::thread::yield_now();
        }
    }
}

#[test]
fn a_failed_seek_preserves_endpoint_and_ring_audio_and_the_audible_timeline() {
    // Also run with a flush-refusing endpoint: an unnecessary flush must not
    // destroy the whole output lifecycle on a harmless decoder refusal.
    for refuse_flush in [false, true] {
        let mut h = Harness::new(true, refuse_flush, false);
        h.prime();
        h.engine.seek_frames(TARGET).unwrap();
        h.drive_to_seek_result();
        h.wait(|h| h.engine.last_error().is_some());
        assert!(
            h.engine
                .last_error()
                .unwrap()
                .message
                .contains("scripted seek refusal")
        );
        assert_eq!(h.engine.transport(), TransportState::Playing);
        assert!(!h.engine.is_drain_pending());
        let rendered = h.ep.lock().unwrap().rendered;
        assert_eq!(h.ep.lock().unwrap().flushes, 0);
        assert_eq!(h.ep.lock().unwrap().closes, 0);
        assert_eq!(
            h.engine.position_frames(),
            rendered,
            "no rebase to the decode write cursor"
        );
        assert_eq!(
            h.hear(),
            expected(1, 0, rendered),
            "every already-rendered frame survives"
        );
        h.wait(|h| h.engine.buffered_frames() >= 256);
        h.stage(256);
        assert_eq!(
            h.hear(),
            expected(1, rendered, 256),
            "the ring continues without a gap"
        );
    }
}

#[test]
fn a_successful_seek_drops_the_endpoint_tail_before_new_audio_is_written() {
    let mut h = Harness::new(false, false, false);
    h.prime();
    assert_eq!(h.ep.lock().unwrap().pending[0], expected(1, 0, 1)[0]);
    h.engine.seek_frames(TARGET).unwrap();
    h.drive_to_seek_result();
    if h.engine.is_drain_pending() {
        h.stage(64); // Drain callback; no post-seek content is writable yet.
    }
    h.wait(|h| h.ep.lock().unwrap().flushes == 1);
    assert!(
        h.ep.lock().unwrap().pending.is_empty(),
        "pre-seek tail really discarded"
    );
    assert_eq!(h.engine.position_frames(), TARGET);
    assert_eq!(h.ep.lock().unwrap().closes, 0);
    h.wait(|h| h.engine.buffered_frames() >= 256);
    h.stage(256);
    assert_eq!(
        h.hear(),
        expected(1, TARGET, 256),
        "first post-seek frame is not flushed away"
    );
}

#[test]
fn pause_resume_retains_audio_and_stop_start_still_discards_the_previous_run() {
    let mut h = Harness::new(false, false, false);
    h.prime();
    h.engine.pause().unwrap();
    assert_eq!(h.ep.lock().unwrap().pending.len(), 512);
    assert_eq!(h.ep.lock().unwrap().flushes, 0);
    h.engine.play().unwrap();
    assert_eq!(h.hear(), expected(1, 0, 256));
    h.stage(256);
    h.engine.stop().unwrap();
    assert_eq!(h.engine.position_frames(), 0);
    assert_eq!(h.ep.lock().unwrap().flushes, 1);
    assert!(h.ep.lock().unwrap().pending.is_empty());
    assert!(h.ep.lock().unwrap().paused);
    assert_eq!(h.ep.lock().unwrap().closes, 0);
    h.engine.play_index(1).unwrap();
    assert!(h.engine.is_start_pending());
    h.stage(64);
    assert_eq!(
        h.hear(),
        vec![0.0; 64],
        "start drain never replays the previous run"
    );
    h.wait(|h| !h.engine.is_start_pending() && h.engine.buffered_frames() >= 256);
    assert_eq!(h.engine.position_frames(), 0);
    h.stage(256);
    assert_eq!(h.hear(), expected(2, 0, 256));
    assert_eq!(
        h.ep.lock().unwrap().starts,
        1,
        "reuse the flush-capable stream"
    );
    assert_eq!(h.ep.lock().unwrap().resumes, 2);
}

#[test]
fn a_refused_flush_on_seek_closes_output_and_blocks_all_restart_paths() {
    let mut h = Harness::new(false, true, false);
    h.prime();
    h.engine.seek_frames(TARGET).unwrap();
    h.drive_to_seek_result();
    if h.engine.is_drain_pending() {
        h.stage(64);
    }
    h.wait(|h| h.engine.transport() == TransportState::Stopped);
    assert_eq!(
        h.ep.lock().unwrap().flushes,
        1,
        "teardown must not retry a refused flush"
    );
    assert_eq!(h.ep.lock().unwrap().closes, 1);
    assert!(h.ep.lock().unwrap().render.is_none());
    assert!(h.ep.lock().unwrap().pending.is_empty());
    assert_eq!(h.engine.last_error().unwrap().kind, ErrorKind::Output);
    assert!(
        h.engine
            .last_error()
            .unwrap()
            .message
            .contains("recreate the engine/output")
    );
    let errors = h
        .events
        .try_iter()
        .filter(|e| matches!(e, Event::Error(_)))
        .count();
    assert_eq!(errors, 1);
    let frozen = h.engine.position_frames();
    for _ in 0..3 {
        h.engine.poll().unwrap();
        assert!(h.engine.play().is_err());
        assert!(h.engine.play_index(1).is_err());
        assert_eq!(h.engine.position_frames(), frozen);
    }
    assert!(h.events.try_iter().all(|e| !matches!(e, Event::Error(_))));
    assert_eq!(h.ep.lock().unwrap().starts, 1);
    assert_eq!(h.ep.lock().unwrap().resumes, 0);
}

#[test]
fn stop_and_stopped_seek_report_a_refused_flush_instead_of_silent_success() {
    let mut h = Harness::new(false, true, false);
    h.prime();
    assert!(h.engine.stop().is_err());
    assert_eq!(h.engine.transport(), TransportState::Stopped);
    assert_eq!(h.engine.position_frames(), 0);
    assert_eq!(h.ep.lock().unwrap().closes, 1);
    assert!(h.engine.play().is_err());

    let mut h = Harness::new(false, true, false);
    assert!(h.engine.seek_frames(500).is_err());
    assert_eq!(h.engine.last_error().unwrap().kind, ErrorKind::Output);
    assert!(h.engine.play().is_err());
    assert_eq!(h.ep.lock().unwrap().starts, 0);
}

#[test]
fn natural_end_keeps_its_position_and_reports_a_refused_flush() {
    let mut h = Harness::new(false, true, true);
    h.engine.play_index(1).unwrap();
    h.wait(|h| h.engine.end_of_stream().is_some());
    h.stage(3000);
    assert_eq!(h.hear(), expected(2, 0, 3000));
    h.engine.poll().unwrap();
    assert_eq!(h.engine.transport(), TransportState::Stopped);
    assert_eq!(h.engine.position_frames(), 3000);
    assert_eq!(h.ep.lock().unwrap().closes, 1);
    assert_eq!(h.engine.last_error().unwrap().kind, ErrorKind::Output);
    assert!(h.engine.play().is_err());
}

#[test]
fn a_failed_initial_seek_on_a_fresh_run_starts_at_the_real_position() {
    let mut h = Harness::new(true, false, false);
    h.engine.seek_frames(TARGET).unwrap();
    h.engine.play().unwrap();
    h.wait(|h| h.engine.last_error().is_some() && h.engine.buffered_frames() >= 256);
    assert_eq!(h.engine.position_frames(), 0);
    assert_eq!(h.ep.lock().unwrap().flushes, 1, "stopped seek only");
    assert!(!h.engine.is_start_pending());
    h.stage(256);
    assert_eq!(h.hear(), expected(1, 0, 256));
}

#[test]
fn a_failed_initial_seek_undoes_the_target_without_cancelling_the_start_drain() {
    let mut h = Harness::new(true, false, false);
    h.prime();
    h.engine.stop().unwrap();
    assert!(h.engine.buffered_frames() > 0);
    h.engine.seek_frames(TARGET).unwrap();
    assert_eq!(h.engine.position_frames(), TARGET);
    h.engine.play().unwrap();
    h.wait(|h| h.engine.last_error().is_some());
    assert_eq!(
        h.engine.position_frames(),
        0,
        "refused initial target is not a real position"
    );
    assert!(
        h.engine.is_start_pending(),
        "failed seek must not cancel A8's start handshake"
    );
    assert!(h.engine.is_drain_pending());
    assert_eq!(h.ep.lock().unwrap().flushes, 2, "stop + stopped seek only");
    h.stage(64);
    assert_eq!(h.hear(), vec![0.0; 64]);
    h.wait(|h| !h.engine.is_start_pending() && h.engine.buffered_frames() >= 256);
    h.stage(256);
    assert_eq!(h.hear(), expected(1, 0, 256));
}

/// RFC 0012 §4.3 against the one backend class that cannot flush.
///
/// This is where the two halves of the output recovery used to collide: the
/// stream-error teardown flushed, this endpoint refuses the flush, the refusal
/// is terminal and hands the run to caller-side engine/output recreation
/// (`crates/rustamp-core/src/audio.rs` `OutputDevice::flush`, W22) - which
/// builds a fresh engine at position zero and throws away the position RFC 0012
/// §4.3 item 4 just froze. The teardown of a dead device therefore does not
/// flush it. The fake endpoint drops its retained buffer during the later
/// close/open; this does not promise to retract audio already in real hardware.
#[test]
fn a_stream_error_on_a_flush_refusing_endpoint_keeps_its_position_and_reopens() {
    let mut h = Harness::new(false, true, false);
    h.prime();
    h.engine.poll().unwrap();
    assert_eq!(h.ep.lock().unwrap().pending.len(), 512);
    assert_eq!(h.dropped.load(Ordering::Acquire), 0, "the decoder is live");
    let _ = h.events.try_iter().count();

    h.fail_stream();
    h.engine.poll().unwrap();

    assert_eq!(h.engine.transport(), TransportState::Stopped);
    assert_eq!(
        h.dropped.load(Ordering::Acquire),
        1,
        "teardown joined decoding"
    );
    assert_eq!(
        h.ep.lock().unwrap().flushes,
        0,
        "a dead endpoint is never asked for a flush it would refuse"
    );
    assert!(
        !h.engine.is_flush_failed(),
        "the terminal caller-side latch stays clear"
    );
    assert!(h.engine.is_stream_lost(), "the engine owns this recovery");
    assert_eq!(
        h.ep.lock().unwrap().closes,
        0,
        "RFC 0012 §4.3 pauses the device; the close belongs to the reopen"
    );
    assert!(h.ep.lock().unwrap().paused);
    assert_eq!(
        h.engine.position_frames(),
        256,
        "the position freezes at the 256 frames the endpoint really rendered"
    );
    assert_eq!(h.engine.last_error().unwrap().kind, ErrorKind::Output);
    assert!(
        h.engine
            .last_error()
            .unwrap()
            .message
            .contains("the stream died")
    );
    assert!(
        !h.engine
            .last_error()
            .unwrap()
            .message
            .contains("recreate the engine/output"),
        "a stream error must not ask the caller to recreate anything"
    );
    let events: Vec<_> = h.events.try_iter().collect();
    assert!(
        matches!(events.as_slice(), [
        Event::StateChanged(TransportState::Stopped),
        Event::PositionChanged { position_ms: 5, .. },
        Event::Error(info),
    ] if info.kind == ErrorKind::Output),
        "ordered failure events: {events:?}"
    );

    // The freeze is persistent, not a single reading.
    for _ in 0..3 {
        h.engine.poll().unwrap();
        assert_eq!(h.engine.position_frames(), 256);
        assert!(h.events.try_iter().all(|e| !matches!(e, Event::Error(_))));
    }
    assert_eq!(
        h.ep.lock().unwrap().pending.len(),
        512,
        "nothing discarded the retained endpoint buffer yet"
    );

    // RFC 0012 §4.3 item 8: the explicit Play reopens this very device.
    h.engine.play().unwrap();
    assert!(!h.engine.is_stream_lost());
    assert!(!h.engine.is_flush_failed());
    assert_eq!(h.engine.transport(), TransportState::Playing);
    assert_eq!(h.ep.lock().unwrap().closes, 1);
    assert_eq!(h.ep.lock().unwrap().opens, 2);
    assert_eq!(h.ep.lock().unwrap().starts, 2, "a fresh render callback");
    assert_eq!(h.ep.lock().unwrap().flushes, 0, "still never flushed");
    assert!(
        h.ep.lock().unwrap().pending.is_empty(),
        "the close of the reopen dropped what the refused flush could not"
    );

    // The reopened run is a real run, bit for bit, even though this backend
    // keeps counting `played_frames` across the reopen.
    h.wait(|h| h.engine.buffered_frames() >= 256);
    assert_eq!(h.engine.position_frames(), 0);
    h.stage(256);
    assert_eq!(
        h.hear(),
        expected(1, 0, 256),
        "the reopened stream carries the track from its start"
    );
}

/// The other half of RFC 0012 §4.3 item 8: while the device is still missing,
/// Play "will ... return a clear `Error::Output` naming the missing device".
/// That must stay retryable - a missing device is not the terminal flush
/// refusal, so the engine keeps the reopen armed instead of demanding
/// caller-side recreation it does not need.
#[test]
fn play_retries_the_reopen_while_the_device_is_still_missing() {
    let mut h = Harness::new(false, true, false);
    h.prime();
    h.fail_stream();
    h.engine.poll().unwrap();
    assert_eq!(h.engine.position_frames(), 256);
    h.set_absent(true);
    h.ep.lock().unwrap().reset_clock_on_close = true;
    let _ = h.events.try_iter().count();

    for attempt in 1..=3u64 {
        let error = h
            .engine
            .play()
            .expect_err("a missing device cannot be reopened");
        assert!(
            error.to_string().contains("buffered-flush-test"),
            "attempt {attempt} must name the device: {error}"
        );
        assert_eq!(h.engine.transport(), TransportState::Stopped);
        assert!(
            h.engine.is_stream_lost(),
            "attempt {attempt} leaves the reopen armed for the next Play"
        );
        assert!(
            !h.engine.is_flush_failed(),
            "attempt {attempt} must not turn a missing device into a flush refusal"
        );
        assert_eq!(
            h.ep.lock().unwrap().opens,
            1 + attempt,
            "attempt {attempt} really tried to open the device"
        );
        assert_eq!(h.ep.lock().unwrap().starts, 1, "nothing started");
        assert_eq!(
            h.engine.position_frames(),
            256,
            "attempt {attempt} keeps the frozen position"
        );
    }

    h.set_absent(false);
    h.engine.play().expect("the device came back");
    assert!(!h.engine.is_stream_lost());
    assert_eq!(h.engine.transport(), TransportState::Playing);
    assert_eq!(h.ep.lock().unwrap().opens, 5);
    assert_eq!(h.ep.lock().unwrap().starts, 2);
    assert_eq!(h.ep.lock().unwrap().flushes, 0);
}

#[test]
fn a_refused_start_after_reopen_stays_retryable_and_keeps_the_failure_position() {
    let mut h = Harness::new(false, true, false);
    h.prime();
    h.fail_stream();
    h.engine.poll().unwrap();
    h.ep.lock().unwrap().refuse_start = true;

    for _ in 0..2 {
        let error = h.engine.play().expect_err("the fresh stream cannot start");
        assert!(error.to_string().contains("start refused"));
        assert!(h.engine.is_stream_lost());
        assert!(!h.engine.is_flush_failed());
        assert_eq!(h.engine.transport(), TransportState::Stopped);
        assert_eq!(h.engine.position_frames(), 256);
        assert_eq!(h.ep.lock().unwrap().starts, 1);
    }

    h.ep.lock().unwrap().refuse_start = false;
    h.engine.play().unwrap();
    assert_eq!(h.ep.lock().unwrap().opens, 4);
    assert_eq!(h.ep.lock().unwrap().starts, 2);
    assert!(!h.engine.is_stream_lost());
    assert!(h.engine.last_error().is_none());
    h.wait(|h| h.engine.buffered_frames() >= 256);
    h.stage(256);
    assert_eq!(h.hear(), expected(1, 0, 256));
}

#[test]
fn a_reopen_adopts_the_new_granted_rate_and_channel_count() {
    let mut h = Harness::new(false, true, false);
    h.prime();
    h.fail_stream();
    h.engine.poll().unwrap();
    let changed = AudioSpec::new(48_000, 1);
    h.ep.lock().unwrap().granted = changed;
    h.engine.play().unwrap();

    assert_eq!(h.engine.device_spec(), changed);
    h.wait(|h| h.engine.buffered_frames() == RING as u64);
    assert_eq!(h.engine.duration_frames(), Some(48_000));
    assert_eq!(h.engine.position_frames(), 0);
    h.stage(256);
    let heard = h.hear();
    assert_eq!(heard.len(), 256);
    assert!(heard.iter().all(|sample| sample.is_finite()));
    assert!(heard.iter().any(|sample| sample.abs() > 0.01));
    assert_eq!(h.engine.position_frames(), 256);
    assert_eq!(h.engine.position_ms(), 5);
}

#[test]
fn an_explicit_stop_after_stream_loss_still_fails_closed_if_flush_is_refused() {
    let mut h = Harness::new(false, true, false);
    h.prime();
    h.fail_stream();
    h.engine.poll().unwrap();
    assert_eq!(h.ep.lock().unwrap().flushes, 0);

    h.engine
        .stop()
        .expect_err("a real flush refusal is terminal");
    assert_eq!(h.ep.lock().unwrap().flushes, 1);
    assert!(h.engine.is_flush_failed());
    let opens = h.ep.lock().unwrap().opens;
    assert!(h.engine.play().is_err());
    assert_eq!(
        h.ep.lock().unwrap().opens,
        opens,
        "only the caller may recover"
    );
}
