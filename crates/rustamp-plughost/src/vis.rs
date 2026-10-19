//! The visualization (`vis_`) plugin bridge (S09 section 5).
//!
//! A classic visualization plugin is a 32-bit DLL that exports
//! `winampVisGetHeader` and hands out one or more `winampVisModule`s
//! (S09 section 2, section 5.1). The host fills four members of the chosen
//! module — the parent window, the plugin's own module handle, the stream
//! sample rate and the channel count — calls `init`, and then draws frames by
//! calling `render` (S09 section 5.2, section 5.3).
//!
//! # What this module does
//!
//! * validate the header (version `0x101`, S09 section 5.1) and enumerate the
//!   library's modules through `get_module`, which answers null past the last
//!   one (S09 section 5.1);
//! * wire the host members into a module and run its `init` (S09 section 5.3
//!   step 2);
//! * draw one frame: refresh the data arrays, call `render`, report the
//!   plugin's own return value (S09 section 5.2, section 5.3 step 4b);
//! * keep the state of the render loop (S09 section 5.3 step 4): when a frame
//!   is due, what the data arrays hold, when the plugin asked to stop, and
//!   how long the caller waits between two frames (S09 section 5.3 step 4b,
//!   step 4c). [`VisLoop`] is that state; it is **driven by the caller** and
//!   owns no thread;
//! * stop the running visualization with `quit` (S09 section 5.3 step 5);
//! * answer the input plugin's `vis_get_mode` and `vis_set_format` host
//!   callbacks for the running visualization (S09 section 3.1 offset 0x78,
//!   section 3.8).
//!
//! # What this module does not do
//!
//! * **It does not feed the data arrays.** The host copies the spectrum and
//!   waveform rows of the frame nearest to (playback position + `latency_ms`)
//!   into the module before every `render` (S09 section 5.3 step 4b). That
//!   frame comes from the input plugin's decode thread and from the analyzer,
//!   which is the audio path. Until that path exists, every frame is the
//!   zeroed one S09 section 5.3 prescribes for a visualization that has no
//!   frame: the caller hands the loop no rows, and the loop zeroes both
//!   arrays.
//! * **It does not run the vis thread.** S09 section 5.3 runs the loop on a
//!   dedicated thread that also pumps the plugin's window messages. The host
//!   process owns that thread; this module owns the per-frame step.
//! * **It does not free the DLL.** S09 section 5.3 step 5 frees the
//!   visualization DLL after `quit` in the 5.x host; this host keeps the
//!   conservative policy of S09 section 1 line 46 and retains the mapping.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::ffi::{c_int, c_void};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};

use rustamp_plugin_abi::PluginKind;
use rustamp_plugin_abi::common::{
    VIS_DEFAULT_CHANNELS, VIS_DEFAULT_SAMPLE_RATE, VIS_MAX_ROWS, VIS_ROW_LEN, VIS_VERSION,
    read_c_string_lossy,
};
use rustamp_plugin_abi::vis::{VisHeader, VisModule};

use crate::loader::{LoadedPlugin, PluginLoadError};

/// Upper bound on the number of modules the host asks a library for.
///
/// S09 section 5.1 says the host calls `get_module(0)`, `get_module(1)`, ...
/// until the answer is null; it does not bound the list. A plugin that never
/// answers null must not hang the host, so the enumeration stops here. Own
/// policy, not S09.
pub const VIS_MAX_MODULES: usize = 64;

/// Host-visible state of one module in a visualization library's module list
/// (S09 section 5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisModuleInfo {
    /// The 0-based index the library hands this module out under.
    pub index: i32,
    /// The module's own name (S09 section 5.2, offset 0x000).
    pub description: String,
    /// How far ahead of "now" the module wants its data (S09 section 5.2,
    /// offset 0x014).
    pub latency_ms: i32,
    /// The pause the host inserts after each `render` (S09 section 5.2,
    /// offset 0x018).
    pub delay_ms: i32,
    /// Spectrum rows the module asks for, 0..=2 (S09 section 5.2, offset
    /// 0x01C).
    pub spectrum_rows: i32,
    /// Waveform rows the module asks for, 0..=2 (S09 section 5.2, offset
    /// 0x020).
    pub waveform_rows: i32,
    /// The module supplied a `config` callback (S09 section 5.2, offset 0x924).
    pub has_config: bool,
    /// The module supplied an `init` callback (S09 section 5.2, offset 0x928).
    pub has_init: bool,
    /// The module supplied a `render` callback (S09 section 5.2, offset 0x92C).
    pub has_render: bool,
    /// The module supplied a `quit` callback (S09 section 5.2, offset 0x930).
    pub has_quit: bool,
}

/// The library-level state of a loaded visualization plugin (S09 section 5.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisLibraryInfo {
    /// The header version the plugin reported (S09 section 5.1, offset 0x00).
    pub version: i32,
    /// The library's own name (S09 section 5.1, offset 0x04).
    pub description: String,
    /// The modules the library hands out, in index order.
    pub modules: Vec<VisModuleInfo>,
}

/// Host-visible state of the running visualization module (S09 section 5.2).
///
/// This is what an acceptance runner checks **before** it calls anything: the
/// members the host filled in and what the plugin's `init` returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisRunInfo {
    /// The index of the running module in the library's module list.
    pub index: i32,
    /// `parent_window` as filled by the host before `init` (S09 section 5.2,
    /// offset 0x004).
    pub parent_window: usize,
    /// `dll_instance` as filled by the host before `init` (S09 section 5.2,
    /// offset 0x008).
    pub dll_instance: usize,
    /// `sample_rate` as filled by the host before `init` (S09 section 5.2,
    /// offset 0x00C).
    pub sample_rate: i32,
    /// `channels` as filled by the host before `init` (S09 section 5.2,
    /// offset 0x010).
    pub channels: i32,
    /// The module's own latency (S09 section 5.2, offset 0x014).
    pub latency_ms: i32,
    /// The module's own delay (S09 section 5.2, offset 0x018).
    pub delay_ms: i32,
    /// `init` was actually called. False means the slot was null and skipped.
    pub init_called: bool,
    /// The raw value `init` returned.
    pub init_result: c_int,
}

/// The plugin-ready rows of one visualization frame (S09 section 5.2, offsets
/// 0x024 and 0x4A4).
///
/// The host copies the spectrum rows and then the waveform rows into the
/// module's arrays before every `render` (S09 section 5.3 step 4b). The rows
/// are bytes, exactly as the plugin reads them. How they are derived from the
/// audio is the analyzer's business — the derivation is spec S08, which does
/// not exist yet — so this type carries the bytes and nothing else: the loop
/// either copies them or zeroes both arrays completely (S09 section 5.3
/// step 4b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisRows {
    /// The spectrum rows. Only the first `spectrum_rows` of them are used
    /// (S09 section 5.2, offset 0x01C).
    pub spectrum: [[u8; VIS_ROW_LEN]; VIS_MAX_ROWS],
    /// The waveform rows. Only the first `waveform_rows` of them are used
    /// (S09 section 5.2, offset 0x020).
    pub waveform: [[u8; VIS_ROW_LEN]; VIS_MAX_ROWS],
}

/// What the player is doing, as the render loop needs to know (S09 section
/// 5.3 step 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisPlayback {
    /// A track is playing: the loop refreshes the data arrays and draws a
    /// frame.
    Playing,
    /// Playback is paused: the loop draws nothing and refreshes nothing
    /// (S09 section 5.3 step 4c).
    Paused,
    /// Playback is stopped: the loop draws one last frame with zeroed data
    /// and then nothing until playback restarts (S09 section 5.3 step 4b).
    Stopped,
}

/// What one step of the render loop did (S09 section 5.3 step 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisStep {
    /// Nothing was drawn and nothing is due: the playback is paused, the
    /// playback stopped and the last frame was already drawn, or no module
    /// runs.
    Idle,
    /// Nothing was drawn: the next frame is not due yet. The caller waits for
    /// the module's own delay (S09 section 5.3 step 4b).
    NotDue,
    /// A frame was drawn and the plugin asked to continue: its `render`
    /// returned 0 (S09 section 5.2, offset 0x92C).
    Rendered,
    /// A frame was drawn and the plugin asked to end the visualization: its
    /// `render` returned nonzero (S09 section 5.2, offset 0x92C). The host
    /// leaves the loop and calls [`VisLoop::stop`], which runs the module's
    /// `quit` (S09 section 5.3 step 5).
    Ended {
        /// The plugin's own return value, unchanged.
        code: c_int,
    },
}

/// What the loop would do at this time: the decision one step acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VisDecision {
    /// Nothing to do.
    Idle,
    /// The next frame is not due yet.
    NotDue,
    /// A frame is due.
    Draw,
}

/// The state of the render loop of one visualization module (S09 section 5.3
/// step 4).
///
/// S09 section 5.3 runs the loop on a **dedicated vis thread** that also
/// pumps the plugin's window messages. That thread belongs to the host
/// process, not to this crate: [`VisLoop`] is the state of one step of the
/// loop, and the caller drives it.
///
/// ```text
/// loop {
///     match vis.step(&plugin, now_ms(), frame) {
///         VisStep::Ended { .. } => { vis.stop(&mut plugin); break; }
///         VisStep::Rendered => sleep(vis.delay_ms()),
///         _ => sleep(vis.idle_sleep_ms()),
///     }
/// }
/// ```
///
/// The rules the loop applies (S09 section 5.3 step 4):
///
/// * a frame is drawn while a track is playing; the data arrays hold the
///   caller's rows, or are zeroed completely when there is no frame;
/// * a paused visualization draws nothing and refreshes nothing;
/// * when the playback stops, one last frame with zeroed data is drawn, and
///   nothing after it until playback restarts;
/// * a nonzero return from `render` ends the loop;
/// * the caller waits the module's own `delay_ms` after a frame, and the
///   smaller of one millisecond and `delay_ms` while nothing is drawn.
#[derive(Debug, Default)]
pub struct VisLoop {
    /// What the player is doing. `None` while the player has not said yet.
    playback: Option<VisPlayback>,
    /// Whether a module is running: `init` succeeded and `quit` has not run.
    running: bool,
    /// Whether the plugin asked to end the visualization.
    ended: bool,
    /// Whether the last frame with zeroed data after a stop is still to be
    /// drawn.
    final_frame: bool,
    /// The time at which the next frame is due.
    next_frame_ms: Option<u64>,
    /// The running module's own pause between two frames.
    delay_ms: u32,
}

impl VisLoop {
    /// A loop that has started no visualization.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start the visualization: the module's `init` runs (S09 section 5.3
    /// step 2).
    ///
    /// A visualization that is already running is stopped first, because only
    /// one visualization is active at a time (S09 section 5.3 step 1). A
    /// refusing `init` aborts the start, and nothing runs (S09 section 5.2,
    /// offset 0x928).
    pub fn start(&mut self, plugin: &mut LoadedPlugin, index: i32) -> Result<(), VisError> {
        plugin.vis_start(index)?;
        // The module's own cadence (S09 section 5.2, offsets 0x014 and 0x018).
        // A negative delay is nonsense: the host waits no time at all then.
        self.delay_ms = plugin
            .vis_run_info()
            .map(|info| u32::try_from(info.delay_ms.max(0)).unwrap_or(0))
            .unwrap_or(0);
        self.running = true;
        self.ended = false;
        self.final_frame = false;
        self.next_frame_ms = None;
        Ok(())
    }

    /// Stop the visualization: the module's `quit` runs (S09 section 5.3
    /// step 5).
    ///
    /// The call is idempotent: a loop that runs no module quits nothing, and
    /// a loop that already stopped does not reach the plugin twice.
    pub fn stop(&mut self, plugin: &mut LoadedPlugin) {
        if !self.running {
            return;
        }
        self.running = false;
        self.ended = false;
        self.final_frame = false;
        self.next_frame_ms = None;
        // S09 section 5.3 step 5: `quit(module)` runs when the loop is left.
        plugin.quit();
    }

    /// Tell the loop what the player is doing.
    ///
    /// The transition matters twice (S09 section 5.3 step 4b): a playback
    /// that stops draws one last frame with zeroed data, and a playback that
    /// restarts draws a frame again. A repeated state changes nothing, so a
    /// player that publishes its state often does not disturb the cadence.
    pub fn set_playback(&mut self, playback: VisPlayback) {
        if self.playback == Some(playback) {
            return;
        }
        let previous = self.playback.replace(playback);
        // A new state starts the cadence afresh: the first frame after the
        // change is due at once.
        self.next_frame_ms = None;
        // The last frame with zeroed data is drawn when the playback stops
        // after it ran (S09 section 5.3 step 4b).
        self.final_frame = matches!(
            (previous, playback),
            (
                Some(VisPlayback::Playing | VisPlayback::Paused),
                VisPlayback::Stopped
            )
        );
    }

    /// What the player is doing, as the loop last heard it. `None` while the
    /// player has not said yet.
    pub fn playback(&self) -> Option<VisPlayback> {
        self.playback
    }

    /// Whether a frame is due at this time (S09 section 5.3 step 4).
    pub fn should_render(&self, now_ms: u64) -> bool {
        matches!(self.decision(now_ms), VisDecision::Draw)
    }

    /// One step of the render loop (S09 section 5.3 step 4).
    ///
    /// The data arrays are refreshed and the plugin's `render` is called when
    /// a frame is due. The rows are the caller's frame — the one the host
    /// picked from its frame ring — or none, in which case both arrays are
    /// zeroed completely (S09 section 5.3 step 4b). A nonzero return from
    /// `render` ends the loop (S09 section 5.2, offset 0x92C); the host then
    /// calls [`stop`](Self::stop), which runs the module's `quit`.
    pub fn step(
        &mut self,
        plugin: &LoadedPlugin,
        now_ms: u64,
        rows: Option<&VisRows>,
    ) -> Result<VisStep, VisError> {
        match self.decision(now_ms) {
            VisDecision::Idle => Ok(VisStep::Idle),
            VisDecision::NotDue => Ok(VisStep::NotDue),
            VisDecision::Draw => {
                // Nothing plays: the last frame after a stop is drawn with
                // zeroed data (S09 section 5.3 step 4b).
                let rows = match self.playback {
                    Some(VisPlayback::Playing) => rows,
                    _ => None,
                };
                let code = match plugin.vis_render(rows) {
                    Ok(code) => code,
                    Err(error) => {
                        // The loop is over, but the module still runs: the
                        // host leaves the loop and calls `stop`, which quits
                        // it (S09 §5.3 step 5).
                        self.ended = true;
                        return Err(error);
                    }
                };
                // The cadence: the next frame is due one delay from now.
                self.next_frame_ms = Some(now_ms + u64::from(self.delay_ms));
                if self.playback == Some(VisPlayback::Stopped) {
                    // The last frame after a stop is drawn once (S09 section
                    // 5.3 step 4b).
                    self.final_frame = false;
                }
                if code == 0 {
                    Ok(VisStep::Rendered)
                } else {
                    // S09 section 5.2: a nonzero return ends the visualization.
                    self.ended = true;
                    Ok(VisStep::Ended { code })
                }
            }
        }
    }

    /// The module's own pause between two frames, in milliseconds (S09
    /// section 5.2, offset 0x018).
    ///
    /// S09 section 5.3 step 4b: the host sleeps this after every frame. The
    /// render time is **not** subtracted, so the real interval is the render
    /// time plus this pause.
    pub fn delay_ms(&self) -> u32 {
        self.delay_ms
    }

    /// The pause the loop takes when it draws nothing: the smaller of one
    /// millisecond and the module's own delay (S09 section 5.3 step 4c).
    pub fn idle_sleep_ms(&self) -> u32 {
        self.delay_ms.min(1)
    }

    /// Whether a module is running: its `init` succeeded and its `quit` has
    /// not run yet.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// What the loop would do at this time.
    fn decision(&self, now_ms: u64) -> VisDecision {
        if !self.running || self.ended {
            return VisDecision::Idle;
        }
        match self.playback {
            // S09 section 5.3 step 4c: a paused visualization draws nothing
            // and refreshes nothing.
            Some(VisPlayback::Paused) => VisDecision::Idle,
            // S09 section 5.3 step 4b: the last frame with zeroed data after
            // a stop, then nothing until playback restarts.
            Some(VisPlayback::Stopped) => {
                if self.final_frame {
                    VisDecision::Draw
                } else {
                    VisDecision::Idle
                }
            }
            // S09 section 5.3 step 4b: a frame per iteration while a track
            // plays.
            Some(VisPlayback::Playing) => {
                if self.frame_due(now_ms) {
                    VisDecision::Draw
                } else {
                    VisDecision::NotDue
                }
            }
            // The player has not said what it is doing yet.
            None => VisDecision::Idle,
        }
    }

    /// Whether the next frame is due at this time.
    ///
    /// The cadence is the module's own `delay_ms` (S09 section 5.2, offset
    /// 0x018). The caller sleeps that long after every frame, so the frame
    /// after it is due one delay later.
    fn frame_due(&self, now_ms: u64) -> bool {
        match self.next_frame_ms {
            None => true,
            Some(due) => now_ms >= due,
        }
    }
}

/// One module of a visualization library: the plugin's own struct and the
/// state the host read out of it.
#[derive(Debug)]
pub(crate) struct VisModuleEntry {
    /// The plugin's static module struct. Its address is stable while the DLL
    /// is mapped (S09 section 0).
    pub(crate) module: *mut VisModule,
    /// What the host read out of that struct.
    pub(crate) info: VisModuleInfo,
}

/// Why a visualization could not be started or driven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisError {
    /// The plugin is not a visualization plugin, so the operation is not
    /// routed for it.
    WrongKind(PluginKind),
    /// The library has no module under this index.
    UnknownModule(i32),
    /// The module's `init` refused: a nonzero return aborts the start
    /// (S09 section 5.2, offset 0x928).
    InitFailed(i32),
    /// No visualization module is running, so there is nothing to draw.
    NotRunning,
    /// The running module supplied no `render` callback (S09 section 5.2,
    /// offset 0x92C).
    MissingRender,
}

impl fmt::Display for VisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongKind(kind) => write!(
                f,
                "visualization is not routed for a {kind:?} plugin; nothing was called"
            ),
            Self::UnknownModule(index) => {
                write!(
                    f,
                    "the visualization library has no module with index {index}"
                )
            }
            Self::InitFailed(code) => write!(
                f,
                "the visualization module refused to start with code {code}; nothing was activated"
            ),
            Self::NotRunning => f.write_str("no visualization module is running"),
            Self::MissingRender => f.write_str(
                "the visualization module supplied no render callback; nothing was called",
            ),
        }
    }
}

impl std::error::Error for VisError {}

/// What happened while a visualization module was initialized.
#[derive(Debug)]
pub(crate) struct VisInit {
    /// The module the library handed out.
    pub(crate) module: *mut VisModule,
    /// Whether an `init` slot was present and called.
    pub(crate) init_called: bool,
    /// The raw value `init` returned. A nonzero value is an error
    /// ([`VisError::InitFailed`]), so a successful start always reports 0.
    pub(crate) init_result: c_int,
}

/// Validate a visualization header and enumerate the library's modules
/// (S09 section 5.1, section 5.3 step 2).
///
/// The header version must be `0x101` (S09 section 5.1): `0x102` belongs to
/// the 5.x-only main-window-taking entry point and is refused, like every
/// other 5.x-only version in this host. The modules are read by asking
/// `get_module(0)`, `get_module(1)`, ... until the answer is null.
///
/// # Safety
///
/// `header` must be null or point at a live `winampVisHeader` in the mapped
/// plugin's static data (S09 section 0). The plugin's `get_module` runs here
/// on the caller's thread.
pub(crate) unsafe fn list_vis_modules(
    header: *mut VisHeader,
) -> Result<Vec<VisModuleEntry>, PluginLoadError> {
    if header.is_null() {
        return Err(PluginLoadError::NullModule);
    }
    // SAFETY: caller guarantees a live header for this read.
    let version = unsafe { (*header).version };
    // S09 section 5.1: 0x101 in 2.x. 0x102 is 5.x-only and refused (U-7
    // decision: strict equality, the general, output and DSP policy).
    if version != VIS_VERSION {
        return Err(PluginLoadError::UnsupportedVersion {
            kind: PluginKind::Visualization,
            actual: version,
            expected: VIS_VERSION,
        });
    }
    // SAFETY: same live header.
    let get_module = unsafe { (*header).get_module };
    let Some(get_module) = get_module else {
        return Err(PluginLoadError::MissingCallback(
            "winampVisHeader.get_module",
        ));
    };
    let mut modules = Vec::new();
    for index in 0..VIS_MAX_MODULES as i32 {
        // SAFETY: cdecl getter from the mapped image (or the test double).
        let module = unsafe { get_module(index) };
        if module.is_null() {
            break;
        }
        modules.push(VisModuleEntry {
            module,
            // SAFETY: the module came from the plugin's own getter and is
            // live in the mapped image's static data.
            info: unsafe { read_vis_module_info(module, index) },
        });
    }
    Ok(modules)
}

/// Read the host-visible state out of a visualization module (S09 section 5.2).
///
/// # Safety
///
/// `module` must point at a live `winampVisModule` in plugin static data.
unsafe fn read_vis_module_info(module: *mut VisModule, index: i32) -> VisModuleInfo {
    // SAFETY: the caller guarantees a live module for these reads.
    let m = unsafe { *module };
    VisModuleInfo {
        index,
        // SAFETY: a null description pointer reads as empty and is not
        // dereferenced (S09 section 1).
        description: unsafe { read_c_string_lossy(m.description) },
        latency_ms: m.latency_ms,
        delay_ms: m.delay_ms,
        spectrum_rows: m.spectrum_rows,
        waveform_rows: m.waveform_rows,
        has_config: m.config.is_some(),
        has_init: m.init.is_some(),
        has_render: m.render.is_some(),
        has_quit: m.quit.is_some(),
    }
}

/// Fill the host members of a visualization module and run its `init`
/// (S09 section 5.2, section 5.3 step 2).
///
/// The host writes the parent window, the plugin's own module handle, the
/// stream sample rate and the channel count before `init` runs (S09 section
/// 5.3 step 2). The rate and the channel count are the defaults S09 section
/// 5.2 gives (44100 Hz and 2 channels); the input plugin replaces them with
/// the values it announces through `vis_set_format` (S09 section 3.8).
///
/// # Safety
///
/// `module` must point at a live `winampVisModule` in the mapped plugin's
/// static data; the plugin's `init` runs here on the caller's thread, which
/// must be the thread that owns the host window.
pub(crate) unsafe fn init_vis_module(
    module: *mut VisModule,
    host_hwnd: usize,
    hinstance: *mut c_void,
) -> Result<VisInit, VisError> {
    if module.is_null() {
        return Err(VisError::NotRunning);
    }
    // SAFETY: live module in plugin static data.
    unsafe {
        (*module).parent_window = host_hwnd as *mut c_void;
        (*module).dll_instance = hinstance;
        (*module).sample_rate = VIS_DEFAULT_SAMPLE_RATE;
        (*module).channels = VIS_DEFAULT_CHANNELS;
    }
    let mut init_called = false;
    let mut init_result = 0;
    // SAFETY: cdecl, one argument (S09 section 5.2, offset 0x928).
    if let Some(init) = unsafe { (*module).init } {
        init_called = true;
        init_result = unsafe { init(module) };
        if init_result != 0 {
            // S09 section 5.2: a nonzero return aborts the start. The caller
            // must not activate the module.
            return Err(VisError::InitFailed(init_result));
        }
    }
    Ok(VisInit {
        module,
        init_called,
        init_result,
    })
}

/// The visualization module that is running, if any.
///
/// S09 section 5.3: one visualization is active at a time, and the host's
/// `vis_get_mode` and `vis_set_format` callbacks answer for the running one
/// (S09 section 3.1 offset 0x78, section 3.8). The pointer lives in an
/// atomic because the input plugin's decode thread reads it (S09 section 3.6)
/// while the host's own thread starts and stops the visualization.
static ACTIVE_VIS_MODULE: AtomicUsize = AtomicUsize::new(0);

/// The visualization module that is running, or null.
pub(crate) fn active_vis_module() -> *mut VisModule {
    ACTIVE_VIS_MODULE.load(Ordering::SeqCst) as *mut VisModule
}

/// Make `module` the running visualization. A null pointer clears the slot.
pub(crate) fn set_active_vis_module(module: *mut VisModule) {
    ACTIVE_VIS_MODULE.store(module as usize, Ordering::SeqCst);
}

/// Clear the running visualization, but only if the slot still holds
/// `module`: a replaced or quitting plugin must not deactivate its successor.
pub(crate) fn clear_active_vis_module(module: *mut VisModule) {
    let _ =
        ACTIVE_VIS_MODULE.compare_exchange(module as usize, 0, Ordering::SeqCst, Ordering::SeqCst);
}

/// Stop the visualization that is running, if any (S09 section 5.3 step 1 and
/// step 5).
///
/// The running module's `quit` is called and the slot is cleared. The DLL
/// stays mapped: the 5.x host frees the visualization DLL after `quit`
/// (S09 section 5.3 step 5), this host keeps the conservative policy of
/// S09 section 1 line 46.
pub(crate) fn stop_active_vis_module() {
    let module = active_vis_module();
    if module.is_null() {
        return;
    }
    // SAFETY: the slot holds a live module in plugin static data.
    if let Some(quit) = unsafe { (*module).quit } {
        // SAFETY: cdecl, one argument (S09 section 5.2, offset 0x930).
        unsafe { quit(module) };
    }
    clear_active_vis_module(module);
}

/// Zero the module's data arrays (S09 section 5.3 step 4b).
///
/// The host copies the frame's spectrum and waveform rows into these arrays
/// before every `render`. With no frame available — the case until the audio
/// path feeds the host — both arrays are zeroed completely, which is what
/// S09 section 5.3 prescribes for a visualization that has no frame.
///
/// # Safety
///
/// `module` must point at a live `winampVisModule` in plugin static data.
pub(crate) unsafe fn zero_vis_data(module: *mut VisModule) {
    // SAFETY: the caller guarantees a live module; both arrays are inline.
    unsafe {
        (*module).spectrum_data = [[0; VIS_ROW_LEN]; VIS_MAX_ROWS];
        (*module).waveform_data = [[0; VIS_ROW_LEN]; VIS_MAX_ROWS];
    }
}

/// Copy the frame's rows into the module's data arrays (S09 section 5.3
/// step 4b).
///
/// The host copies `spectrum_rows` rows and then `waveform_rows` rows
/// (S09 section 5.3 step 4b). A module that asks for fewer rows than the
/// arrays hold gets exactly the rows it asked for; a module that asks for
/// more than the two the ABI has is served the two (S09 section 5.2).
///
/// # Safety
///
/// `module` must point at a live `winampVisModule` in plugin static data.
unsafe fn copy_vis_rows(module: *mut VisModule, rows: &VisRows) {
    // SAFETY: the caller guarantees a live module; both counts are members.
    let (spectrum_rows, waveform_rows) =
        unsafe { ((*module).spectrum_rows, (*module).waveform_rows) };
    let spectrum_rows = spectrum_rows.clamp(0, VIS_MAX_ROWS as i32) as usize;
    let waveform_rows = waveform_rows.clamp(0, VIS_MAX_ROWS as i32) as usize;
    // SAFETY: same live module. Both counts are clamped to the length of the
    // inline arrays, so no index is out of bounds.
    unsafe {
        for row in 0..spectrum_rows {
            (*module).spectrum_data[row] = rows.spectrum[row];
        }
        for row in 0..waveform_rows {
            (*module).waveform_data[row] = rows.waveform[row];
        }
    }
}

/// Draw one frame (S09 section 5.3 step 4b).
///
/// The data arrays are refreshed first: the frame's rows, or zeroed
/// completely when there is no frame — the case until the audio path feeds
/// the host. Then the plugin's `render` is called. Its return value is the
/// plugin's own: 0 continues the visualization, a nonzero value ends it and
/// the host then calls `quit` (S09 section 5.2, offset 0x92C).
///
/// # Safety
///
/// `module` must point at a live `winampVisModule` in plugin static data; the
/// plugin's `render` runs here on the caller's thread.
pub(crate) unsafe fn render_vis_frame(
    module: *mut VisModule,
    rows: Option<&VisRows>,
) -> Result<c_int, VisError> {
    if module.is_null() {
        return Err(VisError::NotRunning);
    }
    // SAFETY: live module in plugin static data.
    let Some(render) = (unsafe { (*module).render }) else {
        return Err(VisError::MissingRender);
    };
    // SAFETY: same live module.
    unsafe { zero_vis_data(module) };
    if let Some(rows) = rows {
        // SAFETY: same live module.
        unsafe { copy_vis_rows(module, rows) };
    }
    // SAFETY: cdecl, one argument (S09 section 5.2, offset 0x92C).
    Ok(unsafe { render(module) })
}

/// Answer the input plugin's `vis_get_mode` for the running visualization
/// (S09 section 3.1, offset 0x78).
///
/// The answer is nonzero while a visualization is running, and the row counts
/// are the ones the running module asked for. With no visualization running
/// both counts are zero and the answer is 0.
pub(crate) fn vis_get_mode(spectrum_rows: *mut c_int, waveform_rows: *mut c_int) -> c_int {
    let module = active_vis_module();
    if module.is_null() {
        write_row_counts(spectrum_rows, waveform_rows, 0, 0);
        return 0;
    }
    // SAFETY: the slot holds a live module in plugin static data.
    let (spectrum, waveform) = unsafe { ((*module).spectrum_rows, (*module).waveform_rows) };
    write_row_counts(spectrum_rows, waveform_rows, spectrum, waveform);
    1
}

/// Write the row counts into the caller's out-parameters, if they are there.
fn write_row_counts(
    spectrum_rows: *mut c_int,
    waveform_rows: *mut c_int,
    spectrum: c_int,
    waveform: c_int,
) {
    if !spectrum_rows.is_null() {
        // SAFETY: the caller guarantees a writable int pointer.
        unsafe { *spectrum_rows = spectrum };
    }
    if !waveform_rows.is_null() {
        // SAFETY: the caller guarantees a writable int pointer.
        unsafe { *waveform_rows = waveform };
    }
}

/// Copy the announced stream format into the running visualization module
/// (S09 section 3.8).
///
/// `vis_set_format` is called by the input plugin when a stream starts
/// (S09 section 3.5 step 3); the host copies the sample rate and the channel
/// count into the running module's own members. With no visualization running
/// there is no module to copy them into, and nothing happens.
pub(crate) fn vis_set_format(sample_rate: c_int, channels: c_int) {
    let module = active_vis_module();
    if module.is_null() {
        return;
    }
    // SAFETY: the slot holds a live module in plugin static data.
    unsafe {
        (*module).sample_rate = sample_rate;
        (*module).channels = channels;
    }
}

/// Serializes the tests that touch the process-wide running-visualization
/// slot, exactly as the DSP bridge's slot lock does.
///
/// The slot is process-global, so every test that starts, stops or inspects
/// the running visualization takes this lock — including the ones in other
/// modules that drive a visualization through [`crate::loader::LoadedPlugin`].
#[cfg(test)]
pub(crate) static VIS_SLOT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loader::{LoadedPlugin, PluginLoadError};
    use std::cell::{Cell, RefCell};

    thread_local! {
        /// The modules the synthetic getter hands out, in index order.
        static VIS_MODULES: RefCell<Vec<*mut VisModule>> = const { RefCell::new(Vec::new()) };
        /// How often the synthetic getter was called.
        static VIS_GETTER_CALLS: Cell<i32> = const { Cell::new(0) };
    }

    /// A getter that answers the modules of [`VIS_MODULES`] and null past the
    /// last one (S09 §5.1).
    unsafe extern "C" fn test_get_module(index: c_int) -> *mut VisModule {
        VIS_GETTER_CALLS.with(|calls| calls.set(calls.get() + 1));
        VIS_MODULES.with(|modules| {
            let modules = modules.borrow();
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            modules.get(index).copied().unwrap_or(core::ptr::null_mut())
        })
    }

    /// A getter that never answers null: the enumeration must still stop.
    unsafe extern "C" fn test_get_module_forever(_index: c_int) -> *mut VisModule {
        VIS_GETTER_CALLS.with(|calls| calls.set(calls.get() + 1));
        VIS_MODULES.with(|modules| {
            let mut modules = modules.borrow_mut();
            if modules.is_empty() {
                // A stable, non-null module the getter always answers with.
                let module: &'static mut VisModule = Box::leak(Box::new(test_module(c"endless")));
                modules.push(module as *mut VisModule);
            }
            modules[0]
        })
    }

    /// A synthetic module with the fields a test needs.
    fn test_module(description: &'static core::ffi::CStr) -> VisModule {
        VisModule {
            description: description.as_ptr(),
            latency_ms: 14,
            delay_ms: 14,
            spectrum_rows: 2,
            waveform_rows: 2,
            init: Some(test_init),
            render: Some(test_render),
            quit: Some(test_quit),
            ..VisModule::default()
        }
    }

    thread_local! {
        /// What the synthetic callbacks recorded, in call order.
        static VIS_LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn vis_note(entry: impl Into<String>) {
        VIS_LOG.with(|log| log.borrow_mut().push(entry.into()));
    }

    fn vis_log() -> Vec<String> {
        VIS_LOG.with(|log| log.borrow().clone())
    }

    fn vis_log_clear() {
        VIS_LOG.with(|log| log.borrow_mut().clear());
        VIS_MODULES.with(|modules| modules.borrow_mut().clear());
        VIS_GETTER_CALLS.with(|calls| calls.set(0));
    }

    /// `init` reads the members the host must have filled before it runs.
    unsafe extern "C" fn test_init(module: *mut VisModule) -> c_int {
        if module.is_null() {
            vis_note("init with no module");
            return -1;
        }
        // SAFETY: the test owns the module and keeps it alive across the call.
        let (parent, instance, rate, channels) = unsafe {
            (
                (*module).parent_window as usize,
                (*module).dll_instance as usize,
                (*module).sample_rate,
                (*module).channels,
            )
        };
        vis_note(format!(
            "init parent={parent:#x} instance={instance:#x} rate={rate} channels={channels}"
        ));
        0
    }

    /// `init` of the module that refuses to start.
    unsafe extern "C" fn test_init_refusing(_module: *mut VisModule) -> c_int {
        vis_note("init refused");
        1
    }

    /// `render` reads the first bytes of both data arrays.
    unsafe extern "C" fn test_render(module: *mut VisModule) -> c_int {
        if module.is_null() {
            return -1;
        }
        // SAFETY: the test owns the module and keeps it alive across the call.
        let (spectrum, waveform) =
            unsafe { ((*module).spectrum_data[0][0], (*module).waveform_data[0][0]) };
        vis_note(format!("render spectrum={spectrum} waveform={waveform}"));
        0
    }

    /// `render` of the module that ends the visualization.
    unsafe extern "C" fn test_render_ending(_module: *mut VisModule) -> c_int {
        vis_note("render ending");
        5
    }

    /// `render` reads the first byte of every row of both data arrays.
    unsafe extern "C" fn test_render_rows(module: *mut VisModule) -> c_int {
        if module.is_null() {
            return -1;
        }
        // SAFETY: the test owns the module and keeps it alive across the call.
        let (s0, s1, w0, w1) = unsafe {
            (
                (*module).spectrum_data[0][0],
                (*module).spectrum_data[1][0],
                (*module).waveform_data[0][0],
                (*module).waveform_data[1][0],
            )
        };
        vis_note(format!("render rows s0={s0} s1={s1} w0={w0} w1={w1}"));
        0
    }

    /// `quit` records the teardown.
    unsafe extern "C" fn test_quit(_module: *mut VisModule) {
        vis_note("quit");
    }

    /// `config` records the call. A visualization config opens a dialog, so
    /// the host calls it on the UI thread (S09 §5.3 step 7).
    unsafe extern "C" fn test_config(_module: *mut VisModule) {
        vis_note("config");
    }

    /// A header whose getter hands out `modules`.
    fn test_header(modules: Vec<*mut VisModule>) -> VisHeader {
        VIS_MODULES.with(|slot| *slot.borrow_mut() = modules);
        VisHeader {
            version: VIS_VERSION,
            description: c"synthetic vis".as_ptr(),
            get_module: Some(test_get_module),
        }
    }

    #[test]
    fn the_module_list_is_read_until_the_getter_answers_null() {
        vis_log_clear();
        let mut first = test_module(c"first");
        let mut second = test_module(c"second");
        let mut header = test_header(vec![&mut first, &mut second]);
        // SAFETY: live stack header and modules; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        assert_eq!(modules.len(), 2, "the list stops at the first null answer");
        assert_eq!(modules[0].info.index, 0);
        assert_eq!(modules[0].info.description, "first");
        assert_eq!(modules[1].info.index, 1);
        assert_eq!(modules[1].info.description, "second");
        // S09 §5.2: the host reads the plugin's own row counts and cadence.
        assert_eq!(modules[0].info.latency_ms, 14);
        assert_eq!(modules[0].info.delay_ms, 14);
        assert_eq!(modules[0].info.spectrum_rows, 2);
        assert_eq!(modules[0].info.waveform_rows, 2);
        assert!(modules[0].info.has_init);
        assert!(modules[0].info.has_render);
        assert!(modules[0].info.has_quit);
        assert!(!modules[0].info.has_config);
        assert_eq!(
            VIS_GETTER_CALLS.with(|calls| calls.get()),
            3,
            "the getter is asked for the module after the last one, to see the null"
        );
    }

    #[test]
    fn a_wrong_header_version_is_refused_before_any_module_is_read() {
        vis_log_clear();
        let mut module = test_module(c"first");
        let mut header = test_header(vec![&mut module]);
        // 0x102 is the 5.x-only main-window-taking entry point (S09 §5.1).
        header.version = rustamp_plugin_abi::common::VIS_VERSION_5X;
        // SAFETY: live stack header; no DLL is involved.
        let error = unsafe { list_vis_modules(&mut header) }.expect_err("0x102 must be refused");
        assert!(
            matches!(
                error,
                PluginLoadError::UnsupportedVersion {
                    kind: PluginKind::Visualization,
                    actual: 0x102,
                    expected: VIS_VERSION,
                }
            ),
            "{error:?}"
        );
        assert_eq!(
            VIS_GETTER_CALLS.with(|calls| calls.get()),
            0,
            "no module may be read before the version check"
        );
    }

    #[test]
    fn a_null_header_and_a_null_getter_are_refused() {
        vis_log_clear();
        // SAFETY: a null header is exactly the input under test.
        let error = unsafe { list_vis_modules(core::ptr::null_mut()) }
            .expect_err("a null header must be refused");
        assert!(matches!(error, PluginLoadError::NullModule), "{error:?}");

        let mut header = VisHeader {
            version: VIS_VERSION,
            description: c"no getter".as_ptr(),
            get_module: None,
        };
        // SAFETY: live stack header; no DLL is involved.
        let error =
            unsafe { list_vis_modules(&mut header) }.expect_err("a null getter must be refused");
        assert!(
            matches!(error, PluginLoadError::MissingCallback(_)),
            "{error:?}"
        );
    }

    #[test]
    fn the_enumeration_stops_at_the_own_bound() {
        vis_log_clear();
        let mut header = VisHeader {
            version: VIS_VERSION,
            description: c"endless".as_ptr(),
            get_module: Some(test_get_module_forever),
        };
        // The synthetic getter answers with a module for every index, so the
        // list never ends by itself. The bound is what keeps such a plugin
        // from hanging the host (own policy, documented in VIS_MAX_MODULES).
        // SAFETY: live stack header; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        assert_eq!(modules.len(), VIS_MAX_MODULES);
        assert_eq!(
            VIS_GETTER_CALLS.with(|calls| calls.get()),
            VIS_MAX_MODULES as i32,
            "the getter must not be asked past the bound"
        );
        assert_eq!(VIS_MAX_MODULES, 64);
    }

    #[test]
    fn init_fills_the_host_members_before_it_runs() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        // SAFETY: live stack module; no DLL is involved.
        let init = unsafe { init_vis_module(&mut module, 0xBEEF, 0xCAFE as *mut c_void) }
            .expect("a module with an init slot must start");
        assert!(init.init_called);
        assert_eq!(init.init_result, 0);
        // S09 §5.3 step 2: the host members are filled before `init` runs.
        assert_eq!(
            vis_log(),
            vec!["init parent=0xbeef instance=0xcafe rate=44100 channels=2"]
        );
    }

    #[test]
    fn a_refusing_init_is_not_activated() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"refusing");
        module.init = Some(test_init_refusing);
        // SAFETY: live stack module; no DLL is involved.
        let error = unsafe { init_vis_module(&mut module, 0x1, 0x2 as *mut c_void) }
            .expect_err("a refusing init must be loud");
        assert_eq!(error, VisError::InitFailed(1));
        assert_eq!(vis_log(), vec!["init refused"]);
    }

    #[test]
    fn a_render_frame_zeroes_the_arrays_and_reports_the_plugins_own_code() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        // SAFETY: the test owns the module and keeps it alive across the call.
        unsafe { zero_vis_data(&mut module) };
        module.spectrum_data[0][0] = 200;
        module.waveform_data[1][575] = 200;
        // SAFETY: live stack module; no DLL is involved.
        let code =
            unsafe { render_vis_frame(&mut module, None) }.expect("a module with render must draw");
        assert_eq!(code, 0, "0 means the visualization continues (S09 §5.2)");
        assert_eq!(
            vis_log(),
            vec!["render spectrum=0 waveform=0"],
            "the arrays are zeroed completely before the frame is drawn"
        );

        let mut ending = test_module(c"ending");
        ending.render = Some(test_render_ending);
        // SAFETY: live stack module; no DLL is involved.
        let code =
            unsafe { render_vis_frame(&mut ending, None) }.expect("a module with render must draw");
        assert_eq!(
            code, 5,
            "the plugin's own return value is reported unchanged"
        );
    }

    #[test]
    fn a_module_without_render_is_refused_not_called() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"silent");
        module.render = None;
        // SAFETY: live stack module; no DLL is involved.
        let error =
            unsafe { render_vis_frame(&mut module, None) }.expect_err("a null render must fail");
        assert_eq!(error, VisError::MissingRender);
        assert!(vis_log().is_empty());
    }

    #[test]
    fn the_running_module_answers_the_host_callbacks() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        // Nothing is running yet.
        let mut spectrum = 7;
        let mut waveform = 7;
        assert_eq!(vis_get_mode(&mut spectrum, &mut waveform), 0);
        assert_eq!(
            (spectrum, waveform),
            (0, 0),
            "no rows without a running module"
        );

        // S09 §3.8: the announced format is copied into the running module.
        set_active_vis_module(&mut module);
        vis_set_format(48_000, 1);
        assert_eq!(module.sample_rate, 48_000);
        assert_eq!(module.channels, 1);

        // S09 §3.1 offset 0x78: the running module's row counts are reported.
        assert_eq!(vis_get_mode(&mut spectrum, &mut waveform), 1);
        assert_eq!((spectrum, waveform), (2, 2));
        // Null out-parameters are allowed.
        assert_eq!(
            vis_get_mode(core::ptr::null_mut(), core::ptr::null_mut()),
            1
        );

        stop_active_vis_module();
        assert_eq!(vis_log(), vec!["quit"], "stopping runs the module's quit");
        assert!(active_vis_module().is_null());
        assert_eq!(vis_get_mode(&mut spectrum, &mut waveform), 0);
    }

    #[test]
    fn only_one_visualization_runs_at_a_time() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut first = test_module(c"first");
        let mut second = test_module(c"second");
        let mut header = test_header(vec![&mut first, &mut second]);
        // SAFETY: live stack header and modules; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::vis_for_test("vis_synthetic", &mut header, modules, 0x99);

        plugin.vis_start(0).expect("the first module must start");
        assert!(plugin.is_initialized());
        assert_eq!(vis_log().len(), 1, "only the first init ran so far");

        // S09 §5.3 step 1: the running visualization is stopped first.
        plugin.vis_start(1).expect("the second module must start");
        assert_eq!(
            vis_log(),
            vec![
                "init parent=0x99 instance=0x0 rate=44100 channels=2",
                "quit",
                "init parent=0x99 instance=0x0 rate=44100 channels=2",
            ]
        );
        assert!(plugin.is_initialized());
    }

    #[test]
    fn the_plugin_lifecycle_runs_init_render_and_quit_once() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        let mut header = test_header(vec![&mut module]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::vis_for_test("vis_synthetic", &mut header, modules, 0x99);

        // Nothing runs before the user picks a module.
        assert!(!plugin.is_initialized());
        assert_eq!(plugin.vis_render(None), Err(VisError::NotRunning));
        assert!(vis_log().is_empty());

        plugin.vis_start(0).expect("the module must start");
        assert!(plugin.is_initialized());
        let run = plugin
            .vis_run_info()
            .expect("a running visualization reports its state");
        assert_eq!(run.index, 0);
        assert_eq!(run.parent_window, 0x99, "the host filled the parent window");
        assert_eq!(run.sample_rate, 44_100);
        assert_eq!(run.channels, 2);
        assert_eq!(run.latency_ms, 14);
        assert_eq!(run.delay_ms, 14);
        assert!(run.init_called);
        assert_eq!(run.init_result, 0);
        assert_eq!(plugin.vis_render(None).expect("a frame must be drawn"), 0);
        assert_eq!(plugin.vis_render(None).expect("a frame must be drawn"), 0);
        assert_eq!(
            vis_log(),
            vec![
                "init parent=0x99 instance=0x0 rate=44100 channels=2",
                "render spectrum=0 waveform=0",
                "render spectrum=0 waveform=0",
            ]
        );

        plugin.quit();
        assert!(!plugin.is_initialized());
        assert!(
            plugin.vis_run_info().is_none(),
            "no module runs after the quit"
        );
        assert_eq!(plugin.vis_render(None), Err(VisError::NotRunning));
        assert_eq!(
            vis_log().last(),
            Some(&"quit".to_string()),
            "quit must reach the plugin"
        );
        // A second quit must not call the plugin again.
        let before = vis_log().len();
        plugin.quit();
        assert_eq!(vis_log().len(), before);
    }

    #[test]
    fn the_config_callback_is_routed_to_the_running_module() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        module.config = Some(test_config);
        let mut header = test_header(vec![&mut module]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::vis_for_test("vis_synthetic", &mut header, modules, 0x99);

        // Nothing runs before the user picks a module, so there is no live
        // module to configure (S09 §5.3 step 2).
        assert_eq!(
            plugin.config(),
            Err(crate::loader::PluginConfigError::NotInitialized)
        );
        assert!(
            vis_log().is_empty(),
            "nothing may be called before the start"
        );

        plugin.vis_start(0).expect("the module must start");
        assert_eq!(plugin.config(), Ok(()));
        assert_eq!(
            vis_log(),
            vec![
                "init parent=0x99 instance=0x0 rate=44100 channels=2",
                "config"
            ],
            "the config callback must run exactly once, on the running module"
        );

        // A module without a `config` slot is refused, never called.
        vis_log_clear();
        let mut bare = test_module(c"bare");
        let mut header = test_header(vec![&mut bare]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::vis_for_test("vis_bare", &mut header, modules, 0x99);
        plugin.vis_start(0).expect("the module must start");
        assert_eq!(
            plugin.config(),
            Err(crate::loader::PluginConfigError::MissingCallback)
        );
        assert!(
            !vis_log().iter().any(|entry| entry == "config"),
            "a null config slot must never be called"
        );

        // A module that is not running any more is not live: its dialog must
        // not be opened (S09 §5.3 step 1).
        vis_log_clear();
        let mut ended = test_module(c"ended");
        ended.config = Some(test_config);
        let mut header = test_header(vec![&mut ended]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::vis_for_test("vis_ended", &mut header, modules, 0x99);
        plugin.vis_start(0).expect("the module must start");
        plugin.quit();
        assert_eq!(
            plugin.config(),
            Err(crate::loader::PluginConfigError::NotInitialized)
        );
        assert!(
            !vis_log().iter().any(|entry| entry == "config"),
            "a quit module must not be configured"
        );
    }

    #[test]
    fn an_unknown_module_index_and_a_wrong_kind_are_refused() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        let mut header = test_header(vec![&mut module]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        let mut plugin = LoadedPlugin::vis_for_test("vis_synthetic", &mut header, modules, 0x99);
        assert_eq!(plugin.vis_start(7), Err(VisError::UnknownModule(7)));
        assert!(vis_log().is_empty(), "no module may be started");

        // A plugin of another kind has no visualization bridge.
        let mut other = LoadedPlugin::unsupported_for_test(PluginKind::Output);
        assert_eq!(
            other.vis_start(0),
            Err(VisError::WrongKind(PluginKind::Output))
        );
        assert_eq!(
            other.vis_render(None),
            Err(VisError::WrongKind(PluginKind::Output))
        );
        assert!(other.vis_info().is_none());
    }

    /// A frame whose rows carry a known byte, so a test can tell a copied row
    /// from a zeroed one.
    fn test_rows() -> VisRows {
        VisRows {
            spectrum: [[0xA0; VIS_ROW_LEN]; VIS_MAX_ROWS],
            waveform: [[0xB0; VIS_ROW_LEN]; VIS_MAX_ROWS],
        }
    }

    /// A synthetic library with one module, wrapped as a loaded plugin.
    ///
    /// The module lives on the caller's stack and stays alive as long as the
    /// returned plugin is used.
    fn test_plugin(module: &mut VisModule) -> LoadedPlugin {
        let mut header = test_header(vec![module as *mut VisModule]);
        // SAFETY: live stack header and module; no DLL is involved.
        let modules = unsafe { list_vis_modules(&mut header) }.expect("a valid header must list");
        LoadedPlugin::vis_for_test("vis_synthetic", &mut header, modules, 0x99)
    }

    #[test]
    fn a_step_without_a_frame_zeroes_both_arrays() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        module.render = Some(test_render_rows);
        let mut plugin = test_plugin(&mut module);

        let mut vis = VisLoop::new();
        vis.start(&mut plugin, 0).expect("the module must start");
        vis.set_playback(VisPlayback::Playing);

        // No frame is available yet: the audio path does not exist. The
        // arrays are zeroed completely (S09 §5.3 step 4b).
        assert_eq!(
            vis.step(&plugin, 0, None).expect("a frame must be drawn"),
            VisStep::Rendered
        );
        assert_eq!(
            vis_log().last(),
            Some(&"render rows s0=0 s1=0 w0=0 w1=0".to_string()),
            "both arrays must be zeroed before the frame is drawn"
        );
    }

    #[test]
    fn a_step_copies_the_rows_the_module_asked_for() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        module.render = Some(test_render_rows);
        // The module asks for one spectrum row and two waveform rows
        // (S09 §5.2, offsets 0x01C and 0x020).
        module.spectrum_rows = 1;
        let mut plugin = test_plugin(&mut module);

        let mut vis = VisLoop::new();
        vis.start(&mut plugin, 0).expect("the module must start");
        vis.set_playback(VisPlayback::Playing);

        let rows = test_rows();
        assert_eq!(
            vis.step(&plugin, 0, Some(&rows))
                .expect("a frame must be drawn"),
            VisStep::Rendered
        );
        assert_eq!(
            vis_log().last(),
            Some(&"render rows s0=160 s1=0 w0=176 w1=176".to_string()),
            "the rows the module asked for are copied, the others stay zeroed"
        );
    }

    #[test]
    fn a_nonzero_render_return_ends_the_loop() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        module.render = Some(test_render_ending);
        let mut plugin = test_plugin(&mut module);

        let mut vis = VisLoop::new();
        vis.start(&mut plugin, 0).expect("the module must start");
        vis.set_playback(VisPlayback::Playing);

        // S09 §5.2: a nonzero return ends the visualization.
        assert_eq!(
            vis.step(&plugin, 0, None).expect("a frame must be drawn"),
            VisStep::Ended { code: 5 }
        );
        assert!(!vis.should_render(0), "the loop is over");
        assert_eq!(
            vis.step(&plugin, 1_000, None)
                .expect("no frame may be drawn"),
            VisStep::Idle
        );
        assert_eq!(
            vis_log(),
            vec![
                "init parent=0x99 instance=0x0 rate=44100 channels=2",
                "render ending",
            ],
            "the plugin must be called once, not once per step"
        );

        // Leaving the loop runs the module's `quit` (S09 §5.3 step 5).
        vis.stop(&mut plugin);
        assert_eq!(vis_log().last(), Some(&"quit".to_string()));
    }

    #[test]
    fn a_module_without_render_is_refused_and_still_quit() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        module.render = None;
        let mut plugin = test_plugin(&mut module);

        let mut vis = VisLoop::new();
        vis.start(&mut plugin, 0).expect("the module must start");
        vis.set_playback(VisPlayback::Playing);

        // A null render slot is never called (S09 §5.2, offset 0x92C).
        assert_eq!(
            vis.step(&plugin, 0, None),
            Err(VisError::MissingRender),
            "the step must report the refusal"
        );
        assert!(
            !vis_log().iter().any(|entry| entry.starts_with("render")),
            "the plugin must not be called through a null slot: {:?}",
            vis_log()
        );

        // The module still runs, so leaving the loop quits it (S09 §5.3
        // step 5).
        vis.stop(&mut plugin);
        assert_eq!(vis_log().last(), Some(&"quit".to_string()));
    }

    #[test]
    fn a_paused_visualization_draws_nothing() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        module.render = Some(test_render_rows);
        let mut plugin = test_plugin(&mut module);

        let mut vis = VisLoop::new();
        vis.start(&mut plugin, 0).expect("the module must start");
        vis.set_playback(VisPlayback::Playing);
        assert_eq!(
            vis.step(&plugin, 0, None).expect("a frame must be drawn"),
            VisStep::Rendered
        );

        // S09 §5.3 step 4c: a paused visualization makes no render call and
        // no data refresh.
        vis.set_playback(VisPlayback::Paused);
        assert!(!vis.should_render(1_000));
        assert_eq!(
            vis.step(&plugin, 1_000, None)
                .expect("no frame may be drawn"),
            VisStep::Idle
        );
        assert_eq!(
            vis_log().len(),
            2,
            "the paused step must not reach the plugin: {:?}",
            vis_log()
        );

        // The pause while nothing is drawn is the smaller of one millisecond
        // and the module's own delay (S09 §5.3 step 4c).
        assert_eq!(vis.idle_sleep_ms(), 1);
    }

    #[test]
    fn the_playback_stopping_draws_one_last_zeroed_frame() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        module.render = Some(test_render_rows);
        let mut plugin = test_plugin(&mut module);

        let mut vis = VisLoop::new();
        vis.start(&mut plugin, 0).expect("the module must start");
        vis.set_playback(VisPlayback::Playing);
        assert_eq!(
            vis.step(&plugin, 0, None).expect("a frame must be drawn"),
            VisStep::Rendered
        );

        // S09 §5.3 step 4b: the playback stops, one last frame with zeroed
        // data is drawn, and nothing after it until playback restarts.
        vis.set_playback(VisPlayback::Stopped);
        assert_eq!(
            vis.step(&plugin, 1_000, None)
                .expect("a frame must be drawn"),
            VisStep::Rendered
        );
        assert_eq!(
            vis_log().last(),
            Some(&"render rows s0=0 s1=0 w0=0 w1=0".to_string()),
            "the last frame after a stop carries zeroed data"
        );
        assert_eq!(
            vis.step(&plugin, 1_100, None)
                .expect("no frame may be drawn"),
            VisStep::Idle
        );
        assert_eq!(vis_log().len(), 3, "the last frame is drawn once");

        // A restart draws a frame again.
        vis.set_playback(VisPlayback::Playing);
        assert_eq!(
            vis.step(&plugin, 1_200, None)
                .expect("a frame must be drawn"),
            VisStep::Rendered
        );
        assert_eq!(vis_log().len(), 4);
    }

    #[test]
    fn the_cadence_follows_the_modules_own_delay() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        module.delay_ms = 14;
        let mut plugin = test_plugin(&mut module);

        let mut vis = VisLoop::new();
        vis.start(&mut plugin, 0).expect("the module must start");
        vis.set_playback(VisPlayback::Playing);

        // S09 §5.2, offset 0x018: the module's own delay is the pause
        // between two frames.
        assert_eq!(vis.delay_ms(), 14);
        assert_eq!(
            vis.step(&plugin, 100, None).expect("a frame must be drawn"),
            VisStep::Rendered
        );
        // The next frame is due one delay later: the render time is not
        // subtracted, so the real interval is the render time plus the delay
        // (S09 §5.3 step 4b).
        assert!(!vis.should_render(113));
        assert!(vis.should_render(114));
        assert_eq!(
            vis.step(&plugin, 113, None).expect("no frame may be drawn"),
            VisStep::NotDue
        );
        assert_eq!(
            vis.step(&plugin, 114, None).expect("a frame must be drawn"),
            VisStep::Rendered
        );
    }

    #[test]
    fn the_stop_calls_the_plugins_quit_exactly_once() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        let mut plugin = test_plugin(&mut module);

        let mut vis = VisLoop::new();
        vis.start(&mut plugin, 0).expect("the module must start");
        assert!(vis.is_running());

        // S09 §5.3 step 5: leaving the loop runs the module's `quit`.
        vis.stop(&mut plugin);
        assert!(!vis.is_running());
        assert_eq!(
            vis_log(),
            vec![
                "init parent=0x99 instance=0x0 rate=44100 channels=2",
                "quit",
            ],
            "the order is init, then quit"
        );

        // A second stop must not reach the plugin again.
        vis.stop(&mut plugin);
        assert_eq!(vis_log().len(), 2);
    }

    #[test]
    fn a_loop_that_never_started_quits_nothing() {
        let _guard = VIS_SLOT_LOCK.lock().unwrap();
        vis_log_clear();
        let mut module = test_module(c"first");
        let mut plugin = test_plugin(&mut module);

        let mut vis = VisLoop::new();
        vis.stop(&mut plugin);
        assert!(
            vis_log().is_empty(),
            "nothing was started, so nothing is quit"
        );
        assert_eq!(
            vis.step(&plugin, 0, None).expect("no frame may be drawn"),
            VisStep::Idle
        );
        assert!(vis_log().is_empty());
    }
}
