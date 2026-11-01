// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! A classic output plug-in as the player's output device.
//!
//! The plug-in runs in a helper process (see
//! `rustamp_plughost::dsp_resident`), and the blocks reach it the way they reach
//! a DSP plug-in. A device is pulled from: a thread here renders a block, hands
//! it over and waits for the helper to take it, which is where the plug-in's
//! `can_write` slows it down.
//!
//! The thread also keeps to real time, because a plug-in that writes to a file
//! takes everything at once. So the disk writer takes as long as the music does.
//!
//! The engine's clock is what the plug-in has played ([`PlayClock`]): a track has
//! not ended while the plug-in still holds its end, and a buffer that delays the
//! start delays the clock. Blocks handed over are the bound of that clock from
//! above; a plug-in without a clock, or with one that stands still, falls back on
//! them, so it cannot hold the player for longer than [`CLOCK_SLACK`].
//!
//! The thread also carries the player's news to the helper, in order with the
//! blocks: the track before the first block, because the disk writer reads the
//! title when it is opened, and a pause and resume, which reach the plug-in's own
//! `pause`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustamp_audio::resident::DspResidentConfig;
use rustamp_core::audio::{AudioSpec, DspStage, OutputDevice, OutputFormat, RenderFn};
use rustamp_core::error::{Error, Result};
use rustamp_ipc::Message;
use rustamp_plughost::dsp_resident::{MODE_ARG, output_stage};
use rustamp_proto::command::Command;

use crate::dsp::{HelperLink, NowPlaying};

/// Frames handed over at once: the classic block.
const BLOCK_FRAMES: usize = 576;
/// How far ahead of real time the thread may run: the plug-in's buffer fills
/// first, so a stutter of the helper does not reach the output at once.
const AHEAD: Duration = Duration::from_millis(250);
/// How long the end of a run waits for a plug-in's own buffer: longer than the
/// two seconds `out_wave` opens with.
const PLUGIN_DRAIN: Duration = Duration::from_millis(2_500);
/// Bounds the wait for the player to name the track before the first block.
const TRACK_WAIT: Duration = Duration::from_secs(1);
/// How far the clock may fall behind the blocks handed over: more than any
/// plug-in's buffer in the test set (2 s), and what a clock that stands still
/// costs at the end of a track.
const CLOCK_SLACK: Duration = Duration::from_secs(10);

/// The device's clock: what the plug-in reports it has played, bounded by what
/// it was given.
#[derive(Debug)]
struct PlayClock {
    sample_rate: u32,
    /// Frames handed over, since the last flush as far as they count.
    handed: u64,
    /// The clock's value and the plug-in's report at the last flush: what was
    /// flushed is not played, and never will be.
    base: u64,
    base_ms: u64,
    /// The largest answer so far: the clock never runs back.
    last: u64,
}

impl PlayClock {
    fn new(sample_rate: u32) -> Self {
        PlayClock {
            sample_rate,
            handed: 0,
            base: 0,
            base_ms: 0,
            last: 0,
        }
    }

    fn handed_over(&mut self, frames: u64) {
        self.handed += frames;
    }

    /// The buffer of the plug-in is gone: what it held does not count, played or
    /// handed.
    fn flushed(&mut self, report_ms: Option<u64>) {
        let now = self.played(report_ms);
        self.base = now;
        self.base_ms = report_ms.unwrap_or(0);
        self.handed = now;
    }

    fn played(&mut self, report_ms: Option<u64>) -> u64 {
        let played = match report_ms {
            // No report yet: a plug-in without a clock plays what it is given.
            None => self.handed,
            Some(ms) => {
                let since = ms.saturating_sub(self.base_ms) * u64::from(self.sample_rate) / 1000;
                let slack = CLOCK_SLACK.as_secs() * u64::from(self.sample_rate);
                (self.base + since)
                    .max(self.handed.saturating_sub(slack))
                    .min(self.handed)
            }
        };
        self.last = self.last.max(played);
        self.last
    }
}

#[derive(Default)]
struct Shared {
    running: AtomicBool,
    paused: AtomicBool,
    flush: AtomicBool,
    clock: Mutex<Option<PlayClock>>,
    error: Mutex<Option<String>>,
}

/// The output device that plays through one classic output plug-in.
pub struct PluginOutput {
    name: String,
    config: DspResidentConfig,
    now_playing: Arc<NowPlaying>,
    spec: Option<AudioSpec>,
    stage: Option<Box<dyn DspStage>>,
    link: Option<HelperLink>,
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PluginOutput {
    /// The device for the plug-in `dll`, approved by the store at `approvals`.
    /// `now_playing` is where the player names the track.
    pub fn new(
        helper: PathBuf,
        dll: &Path,
        approvals: &Path,
        now_playing: Arc<NowPlaying>,
    ) -> Self {
        let name = dll.file_name().map_or_else(
            || "output plug-in".to_owned(),
            |file| file.to_string_lossy().into_owned(),
        );
        let config = DspResidentConfig::new(helper, vec![output_stage(dll, approvals)])
            .with_helper_args(vec![MODE_ARG.to_owned()])
            // The plug-in's `can_write` may hold a block for as long as its
            // buffer is full.
            .with_block_timeout(Duration::from_secs(30));
        PluginOutput {
            name,
            config,
            now_playing,
            spec: None,
            stage: None,
            link: None,
            shared: Arc::new(Shared::default()),
            thread: None,
        }
    }

    fn stop_thread(&mut self) {
        self.shared.running.store(false, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    fn report_ms(&self) -> Option<u64> {
        self.link.as_ref().and_then(HelperLink::played_ms)
    }
}

/// What the pump thread needs of the helper besides the stage.
struct PumpLink<'a> {
    now_playing: &'a NowPlaying,
    link: Option<&'a HelperLink>,
}

impl PumpLink<'_> {
    fn send(&self, message: Message) {
        if let Some(link) = self.link {
            link.send(message);
        }
    }

    fn send_track(&self, seen: &mut u64) {
        if let Some(message) = self.now_playing.message_since(seen) {
            self.send(message);
        }
    }

    fn report_ms(&self) -> Option<u64> {
        self.link.and_then(HelperLink::played_ms)
    }
}

fn pump(
    mut stage: Box<dyn DspStage>,
    mut render: RenderFn,
    spec: AudioSpec,
    shared: &Shared,
    link: &PumpLink<'_>,
) {
    let channels = usize::from(spec.channels);
    let mut block = vec![0.0f32; BLOCK_FRAMES * channels];
    let mut started = Instant::now();
    let mut paced = 0u64;
    let mut paused = false;
    let mut seen_track = 0;

    // The plug-in may read the title when it is opened, with the first block.
    let wait_from = Instant::now();
    while !link.now_playing.has_track()
        && wait_from.elapsed() < TRACK_WAIT
        && shared.running.load(Ordering::Acquire)
    {
        std::thread::sleep(Duration::from_millis(5));
    }

    while shared.running.load(Ordering::Acquire) {
        link.send_track(&mut seen_track);
        if shared.flush.swap(false, Ordering::AcqRel) {
            stage.reset();
            if let Ok(mut clock) = shared.clock.lock()
                && let Some(clock) = clock.as_mut()
            {
                clock.flushed(link.report_ms());
            }
            started = Instant::now();
            paced = 0;
        }
        let wanted_pause = shared.paused.load(Ordering::Acquire);
        if wanted_pause != paused {
            paused = wanted_pause;
            // The helper's plug-in has one module, so the id means nothing here.
            link.send(Message::CommandToHost(if paused {
                Command::PausePlugin { plugin_id: 0 }
            } else {
                Command::UnpausePlugin { plugin_id: 0 }
            }));
        }
        if paused {
            std::thread::sleep(Duration::from_millis(5));
            started = Instant::now();
            paced = 0;
            continue;
        }
        let due = Duration::from_secs_f64(paced as f64 / f64::from(spec.sample_rate));
        if due > started.elapsed() + AHEAD {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        render(&mut block);
        if let Err(error) = stage.process(&mut block) {
            if let Ok(mut slot) = shared.error.lock() {
                *slot = Some(error.to_string());
            }
            return;
        }
        paced += BLOCK_FRAMES as u64;
        if let Ok(mut clock) = shared.clock.lock()
            && let Some(clock) = clock.as_mut()
        {
            clock.handed_over(BLOCK_FRAMES as u64);
        }
    }
}

impl OutputDevice for PluginOutput {
    fn name(&self) -> &str {
        &self.name
    }

    fn open(&mut self, spec: AudioSpec) -> Result<OutputFormat> {
        let (mut stage, link) = crate::dsp::open_helper(&self.config, spec)?;
        stage.prepare(spec)?;
        // A plug-in may name the file it writes after the track it plays and
        // asks the host for the title as it *opens* (`out_disk` does): the track
        // therefore goes to the helper before the first block, not with it. The
        // pump sends it again with the first block, which the chain treats as
        // the same track and ignores.
        let mut seen = 0;
        if let Some(message) = self.now_playing.message_since(&mut seen) {
            link.send(message);
        }
        self.stage = Some(stage);
        self.link = Some(link);
        self.spec = Some(spec);
        Ok(OutputFormat {
            spec,
            buffer_frames: BLOCK_FRAMES as u32,
        })
    }

    fn start(&mut self, render: RenderFn) -> Result<()> {
        let (Some(stage), Some(spec)) = (self.stage.take(), self.spec) else {
            return Err(Error::Output(format!("{}: not open", self.name)));
        };
        if let Ok(mut clock) = self.shared.clock.lock() {
            *clock = Some(PlayClock::new(spec.sample_rate));
        }
        self.shared.running.store(true, Ordering::Release);
        let shared = Arc::clone(&self.shared);
        let now_playing = Arc::clone(&self.now_playing);
        let link = self.link.clone();
        self.thread = Some(std::thread::spawn(move || {
            let link = PumpLink {
                now_playing: &now_playing,
                link: link.as_ref(),
            };
            pump(stage, render, spec, &shared, &link);
        }));
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
        self.shared.flush.store(true, Ordering::Release);
        Ok(())
    }

    fn latency(&self) -> Duration {
        AHEAD
    }

    fn drain_time(&self) -> Duration {
        // A classic output plug-in buffers far more than the pacing window:
        // `out_wave` reports "latency 2000" when it opens. The end of a run
        // waits for that buffer, so the last second of a track is not thrown
        // away with it.
        PLUGIN_DRAIN
    }

    fn reports_played_position(&self) -> bool {
        // The plug-in's `getoutputtime` answers with the write cursor, not with
        // what has been heard: the drain time is what ends a run here.
        false
    }

    fn played_frames(&self) -> u64 {
        let report = self.report_ms();
        self.shared
            .clock
            .lock()
            .ok()
            .and_then(|mut clock| clock.as_mut().map(|clock| clock.played(report)))
            .unwrap_or(0)
    }

    fn stream_error(&self) -> Option<Error> {
        let slot = self.shared.error.lock().ok()?;
        slot.as_ref()
            .map(|message| Error::Output(format!("{}: {message}", self.name)))
    }

    fn close(&mut self) -> Result<()> {
        self.stop_thread();
        self.stage = None;
        self.link = None;
        Ok(())
    }
}

impl Drop for PluginOutput {
    fn drop(&mut self) {
        self.stop_thread();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    /// Counts what reaches the plug-in, and fails when told to.
    struct Stage {
        frames: Arc<AtomicUsize>,
        resets: Arc<AtomicUsize>,
        fail: bool,
    }

    impl DspStage for Stage {
        fn name(&self) -> &str {
            "stage"
        }
        fn prepare(&mut self, _spec: AudioSpec) -> Result<()> {
            Ok(())
        }
        fn process(&mut self, buf: &mut [f32]) -> Result<()> {
            if self.fail {
                return Err(Error::Dsp("the helper died".to_owned()));
            }
            self.frames.fetch_add(buf.len() / 2, Ordering::SeqCst);
            Ok(())
        }
        fn reset(&mut self) {
            self.resets.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn device(stage: Stage) -> PluginOutput {
        let mut device = PluginOutput::new(
            PathBuf::from("host.exe"),
            Path::new(r"C:\Plugins\out_x.dll"),
            Path::new(r"C:\a.toml"),
            Arc::default(),
        );
        device.stage = Some(Box::new(stage));
        device.spec = Some(AudioSpec::new(44_100, 2));
        device.now_playing.publish(0, "title", "file");
        device
    }

    fn wait(mut condition: impl FnMut() -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    #[test]
    fn it_is_named_after_the_plugin_and_keeps_to_real_time() {
        let frames = Arc::new(AtomicUsize::new(0));
        let mut device = device(Stage {
            frames: Arc::clone(&frames),
            resets: Arc::default(),
            fail: false,
        });
        assert_eq!(device.name(), "out_x.dll");
        let start = Instant::now();
        device.start(Box::new(|buf| buf.fill(0.0))).unwrap();
        assert!(wait(|| device.played_frames() > 0));
        std::thread::sleep(Duration::from_millis(100));
        device.close().unwrap();
        let limit = (start.elapsed() + AHEAD).as_secs_f64() * 44_100.0 + 2.0 * BLOCK_FRAMES as f64;
        let played = frames.load(Ordering::SeqCst) as f64;
        assert!(
            played <= limit,
            "{played} frames in real time, limit {limit}"
        );
    }

    #[test]
    fn a_pause_stops_the_blocks_and_a_flush_reaches_the_plugin() {
        let frames = Arc::new(AtomicUsize::new(0));
        let resets = Arc::new(AtomicUsize::new(0));
        let mut device = device(Stage {
            frames: Arc::clone(&frames),
            resets: Arc::clone(&resets),
            fail: false,
        });
        device.start(Box::new(|buf| buf.fill(0.0))).unwrap();
        assert!(wait(|| device.played_frames() > 0));
        device.flush().unwrap();
        assert!(wait(|| resets.load(Ordering::SeqCst) == 1));
        device.pause().unwrap();
        std::thread::sleep(Duration::from_millis(30));
        let before = frames.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(60));
        assert_eq!(frames.load(Ordering::SeqCst), before);
        device.resume().unwrap();
        assert!(wait(|| frames.load(Ordering::SeqCst) > before));
        device.close().unwrap();
    }

    #[test]
    fn a_dead_helper_is_a_stream_error() {
        let mut device = device(Stage {
            frames: Arc::default(),
            resets: Arc::default(),
            fail: true,
        });
        device.start(Box::new(|buf| buf.fill(0.0))).unwrap();
        assert!(wait(|| device.stream_error().is_some()));
        device.close().unwrap();
    }

    /// Runs the pump with a recording link until `script` is done with it.
    fn run_pump(
        now_playing: Arc<NowPlaying>,
        script: impl FnOnce(&Shared, &mpsc::Receiver<Message>, &Arc<AtomicUsize>),
    ) {
        let (sent, heard) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        shared.running.store(true, Ordering::Release);
        let frames = Arc::new(AtomicUsize::new(0));
        let stage = Box::new(Stage {
            frames: Arc::clone(&frames),
            resets: Arc::default(),
            fail: false,
        });
        let thread = {
            let shared = Arc::clone(&shared);
            let now_playing = Arc::clone(&now_playing);
            std::thread::spawn(move || {
                let link = HelperLink::recording(sent);
                pump(
                    stage,
                    Box::new(|buf| buf.fill(0.0)),
                    AudioSpec::new(44_100, 2),
                    &shared,
                    &PumpLink {
                        now_playing: &now_playing,
                        link: Some(&link),
                    },
                );
            })
        };
        script(&shared, &heard, &frames);
        shared.running.store(false, Ordering::Release);
        thread.join().unwrap();
    }

    #[test]
    fn the_track_goes_out_before_the_first_block_and_again_when_it_changes() {
        let now_playing = Arc::new(NowPlaying::default());
        let publisher = Arc::clone(&now_playing);
        run_pump(now_playing, |_, heard, frames| {
            // Nothing is played before the player names the track.
            std::thread::sleep(Duration::from_millis(50));
            assert_eq!(frames.load(Ordering::SeqCst), 0);
            publisher.publish(2, "first", "a.mp3");
            let first = heard.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(
                first,
                Message::NowPlaying {
                    position: 2,
                    title: "first".to_owned(),
                    file: "a.mp3".to_owned()
                }
            );
            // Named again unchanged: no news.
            publisher.publish(2, "first", "a.mp3");
            publisher.publish(3, "second", "b.mp3");
            let second = heard.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(matches!(second, Message::NowPlaying { position: 3, .. }));
        });
    }

    #[test]
    fn a_pause_and_a_resume_reach_the_plugin_once_each() {
        let now_playing = Arc::new(NowPlaying::default());
        now_playing.publish(0, "title", "file");
        run_pump(now_playing, |shared, heard, _| {
            assert!(matches!(
                heard.recv_timeout(Duration::from_secs(5)).unwrap(),
                Message::NowPlaying { .. }
            ));
            shared.paused.store(true, Ordering::Release);
            assert_eq!(
                heard.recv_timeout(Duration::from_secs(5)).unwrap(),
                Message::CommandToHost(Command::PausePlugin { plugin_id: 0 })
            );
            assert!(heard.recv_timeout(Duration::from_millis(50)).is_err());
            shared.paused.store(false, Ordering::Release);
            assert_eq!(
                heard.recv_timeout(Duration::from_secs(5)).unwrap(),
                Message::CommandToHost(Command::UnpausePlugin { plugin_id: 0 })
            );
        });
    }

    const RATE: u32 = 44_100;

    #[test]
    fn the_clock_is_what_the_plugin_played_not_what_it_was_given() {
        let mut clock = PlayClock::new(RATE);
        // Two seconds handed over, the plug-in still prebuffering.
        clock.handed_over(2 * u64::from(RATE));
        assert_eq!(clock.played(Some(0)), 0);
        // It has played half a second of them.
        assert_eq!(clock.played(Some(500)), u64::from(RATE) / 2);
        // The track's end is reached when the plug-in says so.
        assert_eq!(clock.played(Some(2000)), 2 * u64::from(RATE));
    }

    #[test]
    fn the_clock_of_a_plugin_that_reports_nothing_is_what_it_was_given() {
        let mut clock = PlayClock::new(RATE);
        clock.handed_over(1000);
        assert_eq!(clock.played(None), 1000);
    }

    #[test]
    fn a_plugin_that_takes_everything_at_once_cannot_run_ahead_of_the_blocks() {
        // The disk writer's clock can stand beyond what the pacing allowed.
        let mut clock = PlayClock::new(RATE);
        clock.handed_over(u64::from(RATE));
        assert_eq!(clock.played(Some(5000)), u64::from(RATE));
    }

    #[test]
    fn a_clock_that_stands_still_cannot_hold_the_track_for_ever() {
        let mut clock = PlayClock::new(RATE);
        let slack = CLOCK_SLACK.as_secs() * u64::from(RATE);
        clock.handed_over(slack + 500);
        assert_eq!(clock.played(Some(0)), 500);
    }

    #[test]
    fn a_flush_forgets_what_was_handed_and_the_clock_never_runs_back() {
        let mut clock = PlayClock::new(RATE);
        clock.handed_over(2 * u64::from(RATE));
        assert_eq!(clock.played(Some(1000)), u64::from(RATE));
        // A Stop or a seek: a second the plug-in held is gone.
        clock.flushed(Some(1000));
        assert_eq!(clock.played(Some(1000)), u64::from(RATE));
        clock.handed_over(u64::from(RATE));
        assert_eq!(clock.played(Some(1500)), u64::from(RATE) * 3 / 2);
        // A report from before does not take it back.
        assert_eq!(clock.played(Some(1000)), u64::from(RATE) * 3 / 2);
    }
}
