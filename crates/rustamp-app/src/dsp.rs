// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The app's DSP chain: the stages the app installs in the engine, the bypass
//! the engine reports, and the manual recovery from it.
//!
//! # The policy (W14 finding F4)
//!
//! The engine's recovery policy for a failed DSP stage is the **bypass**: the
//! failed stage is replaced by a pass-through, the failure is reported once
//! per run, and the chain runs without that stage until the caller installs a
//! chain again ([`Engine::set_dsp_chain`]). The app is that caller, so the
//! app owns three things:
//!
//! * the **chain** it installed — the stages it handed to the engine;
//! * the **bypass** it remembers — so the preferences page does not claim
//!   that a DSP is running while the engine runs without it; and
//! * the **recovery** — a fresh chain, installed when the user asks for it.
//!
//! # The recovery is manual, never automatic
//!
//! A bypass means the stage failed. Building the same chain again and
//! installing it again would fail the same way, so the app never does that by
//! itself: there is no retry loop. The chain comes back when the user
//! switches the DSP off and on again (Preferences → DSP/Effect), which is the
//! one action that says "try again".
//!
//! # The chain is a recipe
//!
//! The engine takes the stages' ownership and drops the ones that fail, so the
//! app cannot keep the stages: it keeps the recipes and builds a fresh chain
//! whenever one is installed.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use rustamp_audio::resident::DspResidentConfig;
use rustamp_core::audio::{AudioSpec, DspStage};
use rustamp_core::error::{Error, ErrorInfo, Result};
use rustamp_ipc::Message;
use rustamp_mediahost::control::{ControlSender, OutputClock};
use rustamp_plugin_abi::PluginKind;
use rustamp_proto::command::Command;

/// What plays, for the plug-ins in helper processes: the player names it here,
/// and whoever talks to a helper carries the news over.
#[derive(Default)]
pub struct NowPlaying {
    generation: AtomicU64,
    track: Mutex<Option<(u32, String, String)>>,
}

impl NowPlaying {
    /// Names the current track; the same track again is no news.
    pub fn publish(&self, position: u32, title: &str, file: &str) {
        let Ok(mut track) = self.track.lock() else {
            return;
        };
        if track
            .as_ref()
            .is_some_and(|now| now.0 == position && now.1 == title && now.2 == file)
        {
            return;
        }
        *track = Some((position, title.to_owned(), file.to_owned()));
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Whether a track has been named.
    pub fn has_track(&self) -> bool {
        self.generation.load(Ordering::Acquire) > 0
    }

    /// The message for a helper that last saw generation `seen`, if there is news.
    pub(crate) fn message_since(&self, seen: &mut u64) -> Option<Message> {
        let generation = self.generation.load(Ordering::Acquire);
        if generation == *seen {
            return None;
        }
        let track = self.track.lock().ok()?;
        let (position, title, file) = track.as_ref()?;
        *seen = generation;
        Some(Message::NowPlaying {
            position: *position,
            title: title.clone(),
            file: file.clone(),
        })
    }
}

/// What the player holds of a running helper besides its blocks.
#[derive(Clone)]
pub(crate) struct HelperLink {
    #[cfg(not(test))]
    sender: ControlSender,
    #[cfg(test)]
    sender: LinkEnd,
    clock: Option<OutputClock>,
}

#[cfg(test)]
#[derive(Clone)]
enum LinkEnd {
    Helper(ControlSender),
    Recording(std::sync::mpsc::Sender<Message>),
}

impl HelperLink {
    fn new(sender: ControlSender, clock: OutputClock) -> Self {
        HelperLink {
            #[cfg(not(test))]
            sender,
            #[cfg(test)]
            sender: LinkEnd::Helper(sender),
            clock: Some(clock),
        }
    }

    /// A link that records what is sent, for the tests of what goes out.
    #[cfg(test)]
    pub(crate) fn recording(sent: std::sync::mpsc::Sender<Message>) -> Self {
        HelperLink {
            sender: LinkEnd::Recording(sent),
            clock: None,
        }
    }

    /// Sends `message`; a helper that is gone is its stage's failure to report.
    pub(crate) fn send(&self, message: Message) {
        #[cfg(not(test))]
        let _ = self.sender.send(message);
        #[cfg(test)]
        match &self.sender {
            LinkEnd::Helper(sender) => {
                let _ = sender.send(message);
            }
            LinkEnd::Recording(sent) => {
                let _ = sent.send(message);
            }
        }
    }

    /// What the helper's output plug-in reports it has played.
    pub(crate) fn played_ms(&self) -> Option<u64> {
        self.clock.as_ref().and_then(OutputClock::played_ms)
    }
}

/// Opens `config`'s chain in a helper and keeps a link to it. The same as
/// [`DspResidentConfig::open`], which keeps none.
#[cfg(windows)]
pub(crate) fn open_helper(
    config: &DspResidentConfig,
    spec: AudioSpec,
) -> Result<(Box<dyn DspStage>, HelperLink)> {
    use rustamp_audio::resident::ProcessDspChain;
    use rustamp_mediahost::control::Liveness;
    use rustamp_mediahost::dsp::DspProcess;
    use rustamp_proto::host::PcmSpecWire;

    let channels = u8::try_from(spec.channels)
        .map_err(|_| Error::Dsp(format!("{} channels cross no helper", spec.channels)))?;
    let wire = PcmSpecWire {
        sample_rate_hz: spec.sample_rate,
        channels,
    };
    let process = DspProcess::spawn_with_args(
        &config.helper,
        &config.helper_args,
        wire,
        config.stages.clone(),
        config.ring_frames,
        Liveness::default(),
    )
    .map_err(|error| Error::Dsp(error.to_string()))?;
    let link = HelperLink::new(process.control_sender(), process.output_clock());
    let chain = ProcessDspChain::new(
        process,
        config.stages.clone(),
        config.ring_frames,
        config.block_timeout,
    );
    Ok((Box::new(chain), link))
}

/// There are no helpers without Windows.
#[cfg(not(windows))]
pub(crate) fn open_helper(
    _config: &DspResidentConfig,
    _spec: AudioSpec,
) -> Result<(Box<dyn DspStage>, HelperLink)> {
    Err(Error::Unsupported("helpers need Windows"))
}

/// The app's DSP chain: the stages the app installs in the engine, in order.
///
/// The chain is empty until a stage can be built for it. A classic DSP
/// plug-in runs in a 32-bit helper process, see [`classic_dsp_chain`].
#[derive(Clone, Default)]
pub struct DspChain {
    /// How to build each stage, in order.
    recipes: Vec<Arc<dyn Fn() -> Box<dyn DspStage> + Send + Sync>>,
    /// The names of the stages of the last chain the app built, in order.
    ///
    /// They are the stages' own names ([`DspStage::name`]), so the app and
    /// the engine name a stage the same way.
    names: Vec<String>,
    /// The classic plug-in of the chain, if it has one.
    classic: Option<ClassicPlugin>,
}

/// A chain's classic DSP plug-in and how to reach its running helper.
#[derive(Clone)]
struct ClassicPlugin {
    dll: PathBuf,
    /// The helper of the newest stage, for as long as that stage lives.
    running: Arc<Mutex<Weak<HelperLink>>>,
}

impl DspChain {
    /// Adds a stage to the chain.
    ///
    /// The recipe is called once per installation: the engine drops the stage
    /// it was given when that stage fails, so a second installation needs a
    /// second stage.
    pub fn with_stage(
        mut self,
        build: impl Fn() -> Box<dyn DspStage> + Send + Sync + 'static,
    ) -> Self {
        self.recipes.push(Arc::new(build));
        self
    }

    /// Opens the dialog of the plug-in `dll` in the helper that processes the
    /// audio, when one is running for it. A dialog changes what the instance it
    /// belongs to does, so this is the only instance whose dialog is heard.
    pub fn configure_running(&self, dll: &Path) -> bool {
        let Some(classic) = self.classic.as_ref().filter(|classic| classic.dll == dll) else {
            return false;
        };
        let running = classic
            .running
            .lock()
            .ok()
            .and_then(|running| running.upgrade());
        let Some(link) = running else {
            return false;
        };
        link.send(Message::CommandToHost(Command::ConfigurePlugin {
            plugin_id: 0,
            kind: PluginKind::Dsp,
        }));
        true
    }

    /// Whether the chain has no stages.
    pub fn is_empty(&self) -> bool {
        self.recipes.is_empty()
    }

    /// The names of the stages of the last chain the app built, in order.
    pub fn stage_names(&self) -> Vec<&str> {
        self.names.iter().map(String::as_str).collect()
    }

    /// Builds the chain's stages, in order, and remembers their names.
    pub fn build(&mut self) -> Vec<Box<dyn DspStage>> {
        let stages: Vec<Box<dyn DspStage>> = self.recipes.iter().map(|build| build()).collect();
        self.names = stages.iter().map(|stage| stage.name().to_owned()).collect();
        stages
    }
}

/// The classic DSP plug-in as a stage: it runs in a helper process, which starts
/// when the first track is prepared, so a player that plays nothing runs none.
struct ClassicDspStage {
    config: DspResidentConfig,
    chain: Option<Box<dyn DspStage>>,
    /// Keeps the chain's [`ClassicPlugin::running`] alive for this stage.
    link: Option<Arc<HelperLink>>,
    running: Arc<Mutex<Weak<HelperLink>>>,
    now_playing: Arc<NowPlaying>,
    seen_track: u64,
}

impl DspStage for ClassicDspStage {
    fn name(&self) -> &str {
        rustamp_plughost::dsp_resident::STAGE_ID
    }

    fn prepare(&mut self, spec: AudioSpec) -> Result<()> {
        if self.chain.is_none() {
            let (chain, link) = open_helper(&self.config, spec)?;
            let link = Arc::new(link);
            if let Ok(mut running) = self.running.lock() {
                *running = Arc::downgrade(&link);
            }
            self.chain = Some(chain);
            self.link = Some(link);
        }
        self.chain.as_mut().expect("opened above").prepare(spec)
    }

    fn process(&mut self, buf: &mut [f32]) -> Result<()> {
        if let Some(link) = &self.link
            && let Some(message) = self.now_playing.message_since(&mut self.seen_track)
        {
            link.send(message);
        }
        match &mut self.chain {
            Some(chain) => chain.process(buf),
            None => Ok(()),
        }
    }

    fn reset(&mut self) {
        if let Some(chain) = &mut self.chain {
            chain.reset();
        }
    }

    fn latency_frames(&self) -> u32 {
        self.chain
            .as_ref()
            .map_or(0, |chain| chain.latency_frames())
    }
}

/// The chain that runs module `module` of the classic DSP plug-in `dll` through
/// `helper`, the 32-bit plug-in host. `now_playing` is where the player names the
/// track.
pub fn classic_dsp_chain(
    helper: PathBuf,
    dll: &Path,
    approvals: &Path,
    module: u32,
    now_playing: Arc<NowPlaying>,
) -> DspChain {
    let stage = rustamp_plughost::dsp_resident::stage(dll, approvals, module);
    let running: Arc<Mutex<Weak<HelperLink>>> = Arc::default();
    let recipe_running = Arc::clone(&running);
    let mut chain = DspChain::default().with_stage(move || {
        let config = DspResidentConfig::new(helper.clone(), vec![stage.clone()])
            .with_helper_args(vec![rustamp_plughost::dsp_resident::MODE_ARG.to_owned()]);
        Box::new(ClassicDspStage {
            config,
            chain: None,
            link: None,
            running: Arc::clone(&recipe_running),
            now_playing: Arc::clone(&now_playing),
            seen_track: 0,
        })
    });
    chain.classic = Some(ClassicPlugin {
        dll: dll.to_owned(),
        running,
    });
    chain
}

/// Bounds the question what a library holds: a plug-in's `init` runs for it.
#[cfg(windows)]
const ASK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The names of the modules of the DSP library `dll`, in the order of their
/// indices, asked of the helper: the 64-bit player cannot map it. The seam for
/// the preferences' module list.
#[cfg(windows)]
pub fn dsp_module_names(
    helper: &Path,
    dll: &Path,
    approvals: &Path,
) -> std::result::Result<Vec<String>, String> {
    use rustamp_plughost::dsp_resident::modules_args;
    let output = crate::plugin_input::ask_helper(helper, &modules_args(dll, approvals), ASK_WAIT)?;
    Ok(output.lines().map(str::to_owned).collect())
}

/// Shows the dialog of module `module` of `dll` in a process of its own, for a
/// plug-in that is not running: the plug-in reads what it saves when it next
/// starts. Does not wait for the user.
#[cfg(windows)]
pub fn configure_in_own_process(
    helper: &Path,
    dll: &Path,
    approvals: &Path,
    module: u32,
) -> std::result::Result<(), String> {
    use std::os::windows::process::CommandExt;
    let mut child = std::process::Command::new(helper)
        .args(rustamp_plughost::dsp_resident::configure_args(
            dll, approvals, module,
        ))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn()
        .map_err(|error| format!("{}: {error}", helper.display()))?;
    // Reaps the child when the user is done with the dialog.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// A DSP stage the engine bypassed after it failed.
///
/// The engine reports the bypass once per run, in the message of an
/// [`Event::Error`](rustamp_core::event::Event::Error) with the kind
/// [`Dsp`](rustamp_core::error::ErrorKind::Dsp). The app remembers what the
/// report said, so the preferences page can say that the chain runs without
/// that stage instead of claiming that a DSP is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BypassedDsp {
    /// The stage's name, as the engine reported it.
    pub stage: String,
    /// Why the stage failed, as the engine reported it.
    pub cause: String,
}

impl BypassedDsp {
    /// Reads the bypass out of the engine's failure report.
    ///
    /// The engine's report has a fixed shape (see `rustamp-audio`'s
    /// `report_stage_failure`): `dsp error: stage '<name>' failed: <cause>;
    /// the track ends here and the stage is bypassed from now on; <what is
    /// lost>`. The app reads the stage and the cause out of it and shows the
    /// rest of the report to nobody: the loss of the held-back frames is the
    /// engine's accounting, and the app's note says what the user can act on.
    ///
    /// A report that does not have that shape is still a bypass — the engine
    /// only reports a `Dsp` error when a stage failed — so the whole message
    /// becomes the cause then.
    pub fn from_error(info: &ErrorInfo) -> Self {
        let message = &info.message;
        BypassedDsp {
            stage: between(message, "stage '", '\'').unwrap_or_else(|| message.clone()),
            cause: between(message, "failed: ", ';').unwrap_or_else(|| message.clone()),
        }
    }
}

/// The text between `start` and the next `end` after it.
fn between(text: &str, start: &str, end: char) -> Option<String> {
    let rest = text.split_once(start)?.1;
    let tail = rest.find(end)?;
    Some(rest[..tail].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::audio::AudioSpec;
    use rustamp_core::error::{ErrorKind, Result};

    /// The report the engine writes for a failed stage (see
    /// `rustamp-audio`'s `report_stage_failure`).
    fn report(stage: &str, cause: &str) -> ErrorInfo {
        ErrorInfo {
            kind: ErrorKind::Dsp,
            message: format!(
                "dsp error: stage '{stage}' failed: {cause}; the track ends here and the stage \
                 is bypassed from now on; the block it was processing is lost"
            ),
        }
    }

    #[test]
    fn the_stage_and_the_cause_are_read_out_of_the_report() {
        let bypass = BypassedDsp::from_error(&report("resident-dsp", "the helper died"));
        assert_eq!(bypass.stage, "resident-dsp");
        assert_eq!(bypass.cause, "the helper died");
    }

    #[test]
    fn a_report_without_that_shape_still_names_the_bypass() {
        let bypass = BypassedDsp::from_error(&ErrorInfo {
            kind: ErrorKind::Dsp,
            message: "dsp error: something else went wrong".to_owned(),
        });
        assert_eq!(bypass.stage, "dsp error: something else went wrong");
        assert_eq!(bypass.cause, "dsp error: something else went wrong");
    }

    #[test]
    fn a_track_is_news_once_per_helper() {
        let now = NowPlaying::default();
        let mut seen = 0;
        assert!(!now.has_track());
        assert_eq!(now.message_since(&mut seen), None);
        now.publish(1, "title", "file");
        assert!(now.has_track());
        assert!(matches!(
            now.message_since(&mut seen),
            Some(Message::NowPlaying { position: 1, .. })
        ));
        assert_eq!(now.message_since(&mut seen), None);
        now.publish(1, "title", "file");
        assert_eq!(now.message_since(&mut seen), None);
        // A helper that starts later hears the current track.
        let mut later = 0;
        assert!(now.message_since(&mut later).is_some());
    }

    #[test]
    fn a_chain_without_a_running_helper_has_no_dialog_to_open() {
        let chain = classic_dsp_chain(
            PathBuf::from("host.exe"),
            Path::new(r"C:\dsp_x.dll"),
            Path::new(r"C:\dsp_x.toml"),
            0,
            Arc::default(),
        );
        assert!(!chain.configure_running(Path::new(r"C:\dsp_x.dll")));
        assert!(!chain.configure_running(Path::new(r"C:\dsp_y.dll")));
        assert!(!DspChain::default().configure_running(Path::new(r"C:\dsp_x.dll")));
    }

    #[test]
    fn the_dialog_opens_in_the_newest_live_helper_of_that_plugin() {
        let (sent, heard) = std::sync::mpsc::channel();
        let chain = classic_dsp_chain(
            PathBuf::from("host.exe"),
            Path::new(r"C:\dsp_x.dll"),
            Path::new(r"C:\dsp_x.toml"),
            0,
            Arc::default(),
        );
        let link = Arc::new(HelperLink::recording(sent));
        let classic = chain.classic.as_ref().unwrap();
        *classic.running.lock().unwrap() = Arc::downgrade(&link);
        assert!(chain.configure_running(Path::new(r"C:\dsp_x.dll")));
        assert!(matches!(
            heard.try_recv(),
            Ok(Message::CommandToHost(Command::ConfigurePlugin { .. }))
        ));
        assert!(!chain.configure_running(Path::new(r"C:\dsp_y.dll")));
        // The stage is gone: nothing to reach.
        drop(link);
        assert!(!chain.configure_running(Path::new(r"C:\dsp_x.dll")));
    }

    #[test]
    fn a_chain_builds_its_stages_in_order() {
        let mut chain = DspChain::default()
            .with_stage(|| Box::new(NullStage("first")))
            .with_stage(|| Box::new(NullStage("second")));
        assert_eq!(chain.stage_names(), Vec::<&str>::new());
        let stages = chain.build();
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].name(), "first");
        assert_eq!(stages[1].name(), "second");
        // The chain knows the names of the stages it built: they are the
        // stages' own, so the app and the engine name a stage the same way.
        assert_eq!(chain.stage_names(), vec!["first", "second"]);
    }

    #[test]
    fn a_chain_builds_fresh_stages_every_time() {
        // The engine drops a stage that failed, so a second installation
        // needs a second stage: the recipe builds one per call.
        let mut chain = DspChain::default().with_stage(|| Box::new(NullStage("stage")));
        assert_eq!(chain.build().len(), 1);
        assert_eq!(chain.build().len(), 1);
    }

    /// A stage that does nothing, for the chain tests.
    struct NullStage(&'static str);

    impl DspStage for NullStage {
        fn name(&self) -> &str {
            self.0
        }

        fn prepare(&mut self, _spec: AudioSpec) -> Result<()> {
            Ok(())
        }

        fn process(&mut self, _buf: &mut [f32]) -> Result<()> {
            Ok(())
        }

        fn reset(&mut self) {}
    }
}
