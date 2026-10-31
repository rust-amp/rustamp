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
//! takes everything at once, and the engine's clock is this device's clock. So
//! the disk writer takes as long as the music does. Own choice: it is what the
//! position display and the end of a track need.
//!
//! Limits: a pause stops the blocks and leaves what the plug-in already holds to
//! play out, because the channel carries blocks and a reset only.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustamp_audio::resident::DspResidentConfig;
use rustamp_core::audio::{AudioSpec, DspStage, OutputDevice, OutputFormat, RenderFn};
use rustamp_core::error::{Error, Result};
use rustamp_plughost::dsp_resident::{MODE_ARG, output_stage};

/// Frames handed over at once: the classic block.
const BLOCK_FRAMES: usize = 576;
/// How far ahead of real time the thread may run: the plug-in's buffer fills
/// first, so a stutter of the helper does not reach the output at once.
const AHEAD: Duration = Duration::from_millis(250);

#[derive(Default)]
struct Shared {
    running: AtomicBool,
    paused: AtomicBool,
    flush: AtomicBool,
    played: AtomicU64,
    error: Mutex<Option<String>>,
}

/// The output device that plays through one classic output plug-in.
pub struct PluginOutput {
    name: String,
    config: DspResidentConfig,
    spec: Option<AudioSpec>,
    stage: Option<Box<dyn DspStage>>,
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl PluginOutput {
    /// The device for the plug-in `dll`, approved by the store at `approvals`.
    pub fn new(helper: PathBuf, dll: &Path, approvals: &Path) -> Self {
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
            spec: None,
            stage: None,
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
}

fn pump(mut stage: Box<dyn DspStage>, mut render: RenderFn, spec: AudioSpec, shared: &Shared) {
    let channels = usize::from(spec.channels);
    let mut block = vec![0.0f32; BLOCK_FRAMES * channels];
    let mut started = Instant::now();
    let mut paced = 0u64;
    while shared.running.load(Ordering::Acquire) {
        if shared.flush.swap(false, Ordering::AcqRel) {
            stage.reset();
            started = Instant::now();
            paced = 0;
        }
        if shared.paused.load(Ordering::Acquire) {
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
        shared
            .played
            .fetch_add(BLOCK_FRAMES as u64, Ordering::AcqRel);
    }
}

impl OutputDevice for PluginOutput {
    fn name(&self) -> &str {
        &self.name
    }

    fn open(&mut self, spec: AudioSpec) -> Result<OutputFormat> {
        let mut stage = self.config.open(spec)?;
        stage.prepare(spec)?;
        self.stage = Some(stage);
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
        self.shared.running.store(true, Ordering::Release);
        let shared = Arc::clone(&self.shared);
        self.thread = Some(std::thread::spawn(move || {
            pump(stage, render, spec, &shared);
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

    fn played_frames(&self) -> u64 {
        self.shared.played.load(Ordering::Acquire)
    }

    fn stream_error(&self) -> Option<Error> {
        let slot = self.shared.error.lock().ok()?;
        slot.as_ref()
            .map(|message| Error::Output(format!("{}: {message}", self.name)))
    }

    fn close(&mut self) -> Result<()> {
        self.stop_thread();
        self.stage = None;
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
        );
        device.stage = Some(Box::new(stage));
        device.spec = Some(AudioSpec::new(44_100, 2));
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
}
