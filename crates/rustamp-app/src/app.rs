// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The application: it owns the clock, the engine and the UI machine, and it
//! routes commands and events between them (task card N1).
//!
//! # The shape of a running player
//!
//! ```text
//!   OS / winit ──input──> App::mouse_* / key_down ──> MainWindow (headless UI)
//!                                                        │
//!                                                        ▼  Vec<Command>
//!   CommandSender ──────────────────────────────────> command channel
//!                                                        │
//!                                            App::pump drains it
//!                                                        ▼
//!                            Engine::apply (audio) + App::apply_app (windows,
//!                            skin, modes, dialogs)
//!                                                        │
//!                                             EventBus publish (engine)
//!                                                        ▼
//!   App::pump drains events ──> MainWindow::handle_event ──> render state
//! ```
//!
//! The UI machine is headless: it never reads a clock (the app passes
//! milliseconds in) and never touches the OS. The app is the only place that
//! knows both sides.
//!
//! Everything in this module is headless and unit-tested with a
//! [`NullOutput`]; the windowed presenter in [`crate::window`] only adds the
//! `winit`/`softbuffer` shell around it.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustamp_audio::{Engine, EngineConfig, TrackSource};
use rustamp_core::audio::{DecoderRegistry, OutputDevice};
use rustamp_core::bus::{CommandReceiver, CommandSender, EventBus, EventReceiver, command_channel};
use rustamp_core::command::Command;
use rustamp_core::config::{
    AudioConfig, Config, EqConfig, PlaylistConfig, PluginsConfig, UiConfig, WindowLayout,
};
use rustamp_core::error::{Error, ErrorInfo, ErrorKind, Result};
use rustamp_core::event::Event;
use rustamp_core::playlist::{EntryId, MediaLocation, SortKey};
use rustamp_core::state::{PlayerState, TransportState, WindowId};
use rustamp_gfx::Image;
use rustamp_ipc::{Message, Transport};
use rustamp_meta::replaygain::ReplayGainTags;
use rustamp_output::{NullClock, NullOutput, default_output};
use rustamp_ui::{
    DialogManager, EqMenuRequest, EqualizerLayout, EqualizerWindow, Key, MainWindow,
    MainWindowLayout, MainWindowState, Modifiers, PlaylistDialog, PlaylistWindow, ShortcutOutcome,
    eq_frame_size, frame_size, render_equalizer_window, render_main_window, volume_percent,
};

use crate::cli::OutputChoice;
use crate::host::{HostStateLink, PollOutcome, PublishOutcome, player_state_message};
use crate::routing;
use crate::skin::{LoadedSkin, SkinChoice, load as load_skin};
use crate::supervise::Supervisor;

/// How many "not implemented yet" notes the app keeps for the caller to print.
const MAX_NOTES: usize = 64;

/// Everything [`App::new`] needs.
pub struct AppOptions {
    /// The loaded configuration.
    pub config: Config,
    /// The loaded skin (already resolved and fallen back if needed).
    pub skin: LoadedSkin,
    /// Where the audio goes.
    pub output: OutputChoice,
    /// Initial volume `0..=255`.
    pub volume: u8,
    /// The file to queue and play, if any.
    pub file: Option<PathBuf>,
    /// Stop after this many seconds (test aid).
    pub seconds: Option<u64>,
}

/// The player.
pub struct App {
    engine: Engine,
    ui: MainWindow,
    /// Drop the next tap batch across a seek/track/stream boundary: frames have
    /// no generation or source-rate tag. This is conservative, not a proof of
    /// audible-time or post-seek correspondence (S08 remains open).
    vis_discontinuity: bool,
    layout: MainWindowLayout,
    /// The equalizer's headless behaviour (S07; the engine stage is M2 future
    /// work, so this is the UI's own prediction state).
    eq: EqualizerWindow,
    eq_layout: EqualizerLayout,
    pub(crate) skin: LoadedSkin,
    config: Config,
    /// The plug-in folder as the preferences pages last scanned it
    /// ([`App::rescan_plugins`]); empty until the dialog first opens.
    plugin_scan: rustamp_prefs::PluginScan,

    bus: EventBus,
    events: EventReceiver,
    commands: CommandSender,
    command_rx: CommandReceiver,

    output_choice: OutputChoice,
    registry: DecoderRegistry,

    /// The virtual clock, when the null output is in use (`--null` headless or
    /// `--no-audio` windowed).
    clock: Option<NullClock>,
    started: Instant,
    seconds: Option<u64>,
    /// Last time [`App::drive_null_clock`] ran (windowed `--no-audio` only).
    last_null_tick: Instant,
    /// Sub-frame remainder for [`App::drive_null_clock`], mirroring the
    /// accumulator in [`App::run_headless`].
    pending_null_frames: f64,

    // ----- application-level state (not the engine's, not the UI machine's) -----
    shuffle: bool,
    repeat: bool,
    always_on_top: bool,
    easy_move: bool,
    pub(crate) focused: bool,
    current_skin: Option<PathBuf>,
    quit: bool,
    /// Runtime tray/hotkey visibility; start visible even if a previous run hid
    /// the main window, so an unavailable tray can never strand the player.
    main_visible: bool,
    minimize_requested: bool,
    /// Which player window holds keyboard focus: the headless model of the OS
    /// activation (see [`crate::focus`]); the presenter applies it.
    focused_window: WindowId,
    /// Pending "focus this OS window" for the presenter.
    focus_request: Option<WindowId>,
    /// Pending "un-minimize the main OS window" for the presenter: the half
    /// of `ShowMainWindow` that `main_visible` alone cannot express, since a
    /// minimized window is still visible.
    restore_request: bool,
    playlist_ended: bool,
    /// Stable ids the app assigns to the engine's queue entries.
    ///
    /// The engine reports `Event::TrackChanged` with `id: None` (it holds no
    /// playlist ids; RFC 0001 open point), while the UI machine uses
    /// `id.is_some()` to tell "a track is loaded" from "nothing loaded". The
    /// app is the state owner, so it allocates the ids and normalises the event
    /// on its way to the UI. See the findings in the task card report.
    pub(crate) entry_ids: Vec<EntryId>,
    next_entry_id: u64,
    /// The playlist editor's behaviour model (S06); owned here, rendered and
    /// fed by `playlist_window.rs`, synced from `entry_ids`.
    pub(crate) playlist: PlaylistWindow,
    /// The simple modal dialogs (About, file info, jump-to, load playlist,
    /// skin browser, preferences); owned here, rendered by `dialogs.rs`,
    /// hosted by `window.rs`.
    pub(crate) dialogs: DialogManager,
    pub(crate) last_playlist_report: Option<rustamp_playlist::PlaylistReport>,
    /// The format the currently loaded playlist was detected as (RFC 0017
    /// import). `None` means no playlist has been loaded yet, so callers
    /// default to M3U. Stored alongside [`Self::last_playlist_report`] because
    /// that report carries no format of its own.
    pub(crate) last_playlist_format: Option<rustamp_playlist::PlaylistFormat>,
    /// Whether the main-menu popup is open (owned here, rendered by
    /// `menu.rs`, hosted by `window.rs`; S04).
    pub(crate) main_menu_open: bool,
    /// A clutterbar popup (Options or visualization) waiting for the presenter.
    pub(crate) pending_clutter_menu: Option<crate::menu::ClutterMenu>,
    /// The equalizer's Presets popup is waiting to be shown (S07 §4.2).
    pub(crate) pending_eq_menu: bool,
    /// A save-playlist request waiting for the presenter (Ctrl+S, S03 §2, command ID 40204).
    pub(crate) request_save_playlist: bool,
    /// The user's saved equalizer presets (`presets.eqf` beside the config).
    pub(crate) eq_user_presets: Vec<crate::eq_presets::UserPreset>,
    notes: Vec<String>,
    /// The last error the user should see (BETA-04): a failed open/decode or
    /// a rejected command. Shown in the main window's native title bar
    /// ([`App::window_title`]); cleared when a track starts.
    last_error: Option<String>,
    /// The app's end of the player-state bridge to the plugin host
    /// (see [`crate::host`]): detached until a supervisor attaches a
    /// control-channel transport, so publishing without a host is a no-op.
    host: HostStateLink,
    /// The plugin host's spawn and supervision (see [`crate::supervise`).
    ///
    /// `None` is the unmanaged default: the link stays detached and the
    /// player runs identically with and without a host. [`run`](crate::run)
    /// installs one; tests leave it off so no process is ever spawned.
    host_supervisor: Option<Supervisor>,
    /// Where configuration is persisted on clean exit, if known.
    config_path: Option<PathBuf>,
}

/// Whether `path` has a recognized playlist file extension.
pub(crate) fn is_playlist_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    rustamp_playlist::PlaylistFormat::ALL
        .iter()
        .any(|fmt| fmt.extension().eq_ignore_ascii_case(ext))
}

/// Whether a queue entry is "dead": a local file that no longer exists.
///
/// Only [`TrackSource::Path`] is probed. A URL is a network stream, not a file,
/// so it is never checked and always survives ("Remove all dead files",
/// S03 §2 command id 40222). S03 settles the label, not the stream rule, so
/// that rule — and keeping a path that now names an existing directory — is an
/// own choice (RFC 0018).
fn source_is_dead(source: &TrackSource) -> bool {
    match source {
        TrackSource::Path(path) => !path.exists(),
        TrackSource::Url(_) => false,
    }
}

/// Creates the output device and optional null clock for the given choice.
///
/// This runs on the thread that afterwards creates the window (see
/// [`crate::run`]), so on Windows nothing here may initialise COM on *this*
/// thread: `winit` calls `OleInitialize` for drag and drop when it creates the
/// window and panics with `RPC_E_CHANGED_MODE` if the thread is already in the
/// multi-threaded apartment. `rustamp_output::default_output` guarantees that
/// (backend probe and WASAPI endpoint both live on their own threads); see its
/// "COM apartment" section before putting any device work in front of the
/// window.
fn create_output(choice: OutputChoice) -> (Box<dyn OutputDevice>, Option<NullClock>) {
    match choice {
        OutputChoice::Null | OutputChoice::NoAudio => {
            let (output, clock) = NullOutput::pair();
            (Box::new(output), Some(clock))
        }
        OutputChoice::Default => (default_output(), None),
    }
}

/// Maps the persisted configuration mode to the audio engine's mode.
fn replaygain_mode_to_audio(
    mode: rustamp_core::config::ReplayGainMode,
) -> rustamp_audio::ReplayGainMode {
    match mode {
        rustamp_core::config::ReplayGainMode::Off => rustamp_audio::ReplayGainMode::Off,
        rustamp_core::config::ReplayGainMode::Track => rustamp_audio::ReplayGainMode::Track,
        rustamp_core::config::ReplayGainMode::Album => rustamp_audio::ReplayGainMode::Album,
    }
}

/// Maps the audio engine's mode back to the persisted configuration mode.
fn replaygain_mode_from_audio(
    mode: rustamp_audio::ReplayGainMode,
) -> rustamp_core::config::ReplayGainMode {
    match mode {
        rustamp_audio::ReplayGainMode::Off => rustamp_core::config::ReplayGainMode::Off,
        rustamp_audio::ReplayGainMode::Track => rustamp_core::config::ReplayGainMode::Track,
        rustamp_audio::ReplayGainMode::Album => rustamp_core::config::ReplayGainMode::Album,
    }
}

impl App {
    /// Builds the player with the default decoder registry.
    pub fn new(options: AppOptions) -> Result<App> {
        Self::with_registry(options, rustamp_decoders::default_registry())
    }

    /// Builds the player with an explicit registry (embedding and tests).
    pub fn with_registry(options: AppOptions, registry: DecoderRegistry) -> Result<App> {
        let bus = EventBus::new();
        let events = bus.subscribe();
        let (commands, command_rx) = command_channel();

        let (output, clock) = create_output(options.output);

        let cfg = EngineConfig {
            volume: options.volume,
            balance: options.config.audio.balance,
            ..EngineConfig::default()
        };
        let mut engine = Engine::new(registry.clone(), output, bus.clone(), cfg)?;
        // `Engine::new` initialises its gain from the config but reports
        // `volume: 255, balance: 0` in `PlayerState` (an engine-side
        // inconsistency; see the findings). Re-assert the configured values so
        // the reported state matches what the engine actually applies.
        engine.set_volume(options.volume)?;
        engine.set_balance(options.config.audio.balance)?;
        engine.set_eq(options.config.eq.state);
        engine.set_replaygain_mode(replaygain_mode_to_audio(options.config.audio.replaygain));
        if let Some(file) = &options.file
            && !is_playlist_file(file)
        {
            engine.set_queue(vec![TrackSource::path(file.clone())]);
        }

        let main_layout = options
            .config
            .ui
            .windows
            .iter()
            .find(|w| w.id == WindowId::Main);
        let eq_layout = options
            .config
            .ui
            .windows
            .iter()
            .find(|w| w.id == WindowId::Equalizer);
        let pl_layout = options
            .config
            .ui
            .windows
            .iter()
            .find(|w| w.id == WindowId::Playlist);

        // Seed the UI machine from the config, window layouts, and the engine's initial state.
        let initial = MainWindowState {
            volume: volume_percent(options.volume),
            balance: rustamp_ui::balance_percent(options.config.audio.balance),
            double_size: options.config.ui.double_size,
            clutter_a: options.config.ui.always_on_top,
            shuffle: options.config.playlist.shuffle,
            repeat: options.config.playlist.repeat,
            windowshade: main_layout.map(|l| l.shaded).unwrap_or(false),
            eq_open: eq_layout.map(|l| l.visible).unwrap_or(false),
            playlist_open: pl_layout.map(|l| l.visible).unwrap_or(true),
            ..MainWindowState::default()
        };
        let eq_shaded = eq_layout.is_some_and(|layout| layout.shaded);

        let mut app = App {
            engine,
            ui: MainWindow::from_state(initial),
            vis_discontinuity: false,
            layout: MainWindowLayout::CLASSIC,
            eq: EqualizerWindow::from_state(rustamp_ui::EqualizerState {
                windowshade: eq_shaded,
                ..rustamp_ui::EqualizerState::default()
            }),
            eq_layout: EqualizerLayout::CLASSIC,
            skin: options.skin,
            config: options.config,
            plugin_scan: rustamp_prefs::PluginScan::default(),
            config_path: None,
            bus,
            events,
            commands,
            command_rx,
            output_choice: options.output,
            registry,
            clock,
            started: Instant::now(),
            seconds: options.seconds,
            last_null_tick: Instant::now(),
            pending_null_frames: 0.0,
            shuffle: false,
            repeat: false,
            always_on_top: false,
            easy_move: true,
            focused: true,
            current_skin: None,
            quit: false,
            main_visible: true,
            minimize_requested: false,
            focused_window: WindowId::Main,
            focus_request: None,
            restore_request: false,
            playlist_ended: false,
            entry_ids: Vec::new(),
            next_entry_id: 0,
            playlist: PlaylistWindow::new(),
            dialogs: DialogManager::new(),
            last_playlist_report: None,
            last_playlist_format: None,
            main_menu_open: false,
            pending_clutter_menu: None,
            pending_eq_menu: false,
            request_save_playlist: false,
            eq_user_presets: Vec::new(),
            notes: Vec::new(),
            last_error: None,
            host: HostStateLink::detached(),
            host_supervisor: None,
        };
        app.shuffle = app.config.playlist.shuffle;
        app.repeat = app.config.playlist.repeat;
        app.always_on_top = app.config.ui.always_on_top;
        app.easy_move = app.config.ui.easy_move;
        // A path skin remembers where it came from, so `RefreshSkin` can re-read it.
        app.current_skin = app.skin.path().map(PathBuf::from);
        app.eq.set_double_size(app.ui.state().double_size);
        app.sync_eq_from_engine();
        // The engine's snapshot is the authoritative starting state.
        app.sync_entry_ids();
        if let Some(file) = &options.file
            && is_playlist_file(file)
        {
            let _ = app.load_playlist_file(file, false);
        }
        let snapshot = app.engine.state();
        app.sync_from_player_state(&snapshot);
        Ok(app)
    }

    /// Milliseconds since the app was built. The app owns the clock; the UI
    /// machine only receives it.
    pub fn now_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// The current render state.
    pub fn state(&self) -> &MainWindowState {
        self.ui.state()
    }

    /// The headless UI machine.
    pub fn ui(&self) -> &MainWindow {
        &self.ui
    }

    /// The equalizer's headless behaviour (S07).
    pub fn eq(&self) -> &EqualizerWindow {
        &self.eq
    }

    /// The equalizer's current render state.
    pub fn eq_state(&self) -> &rustamp_ui::EqualizerState {
        self.eq.state()
    }

    /// Whether the equalizer window is open (the main window's `eq_open` flag,
    /// flipped by its EQ toggle button).
    pub fn eq_open(&self) -> bool {
        self.ui.state().eq_open
    }

    /// Whether the playlist editor window is open (the main window's
    /// `playlist_open` flag; the presenter creates or drops the OS window to
    /// match it, the same way as the equalizer's).
    pub fn playlist_open(&self) -> bool {
        self.ui.state().playlist_open
    }

    /// The last error surfaced to the user, if one is showing (BETA-04).
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    /// The outcome of the most recent playlist file load, if one has occurred.
    pub fn last_playlist_report(&self) -> Option<&rustamp_playlist::PlaylistReport> {
        self.last_playlist_report.as_ref()
    }

    /// The format the currently loaded playlist was detected as, if any.
    ///
    /// `None` means no playlist has been loaded yet; callers default to M3U.
    pub fn last_playlist_format(&self) -> Option<rustamp_playlist::PlaylistFormat> {
        self.last_playlist_format
    }

    /// Alias for [`Self::last_playlist_format`] using the task's wording.
    pub fn loaded_playlist_format(&self) -> Option<rustamp_playlist::PlaylistFormat> {
        self.last_playlist_format
    }

    /// Alias for [`Self::last_playlist_format`].
    pub fn playlist_format(&self) -> Option<rustamp_playlist::PlaylistFormat> {
        self.last_playlist_format
    }

    /// The main window's native title: `Rustamp`, or `Rustamp - <error>`
    /// while an error is showing.
    ///
    /// Own choice, stated plainly: no spec gives the player a place for
    /// error text. The native title bar is legible whatever the skin (the
    /// built-in placeholder font is not, BETA-02), changes nothing the specs
    /// measured inside the skinned window, and the presenter applies it.
    pub fn window_title(&self) -> String {
        match &self.last_error {
            Some(error) => format!("Rustamp - {error}"),
            None => "Rustamp".to_string(),
        }
    }

    /// The headless UI machine, mutably (tests and the presenter).
    pub fn ui_mut(&mut self) -> &mut MainWindow {
        &mut self.ui
    }

    /// The audio engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The audio engine, mutably (tests and embedding).
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// Recreates the audio output device and engine as required by the audio
    /// contract (crates/rustamp-core/src/audio.rs:370-380) when a backend
    /// refuses flush (`Error::Unsupported`) or closes the output.
    /// Preserves the queue, current track index, volume, balance, equalizer
    /// settings, replaygain, limiter, shuffle, and repeat modes.
    pub fn recreate_engine(&mut self) -> Result<()> {
        let (output, clock) = create_output(self.output_choice);
        let queue = self.engine.queue().to_vec();
        let current_index = self.engine.current_index();
        let volume = self.engine.volume();
        let balance = self.engine.balance();
        let eq = self.engine.eq();
        let limiter = self.engine.limiter_enabled();
        let replaygain_mode = self.engine.replaygain_mode();
        let replaygain_preamp_db = self.engine.replaygain_preamp_db();
        let stop_after_current = self.engine.stop_after_current();
        let shuffle = self.engine.shuffle();
        let repeat = self.engine.repeat();

        let cfg = EngineConfig {
            volume,
            balance,
            ..EngineConfig::default()
        };
        let mut new_engine = Engine::new(self.registry.clone(), output, self.bus.clone(), cfg)?;
        new_engine.set_volume(volume)?;
        new_engine.set_balance(balance)?;
        new_engine.set_eq(eq);
        new_engine.set_queue(queue);
        new_engine.set_current_index(current_index);
        new_engine.set_limiter_enabled(limiter);
        new_engine.set_replaygain_mode(replaygain_mode);
        new_engine.set_replaygain_preamp_db(replaygain_preamp_db);
        let _ = new_engine.set_shuffle(shuffle);
        let _ = new_engine.set_repeat(repeat);
        if stop_after_current {
            let _ = new_engine.toggle_stop_after_current();
        }

        self.engine = new_engine;
        self.clock = clock;
        Ok(())
    }

    /// Recreates the engine and output if the previous output closed due to
    /// a terminal flush refusal or failure, fulfilling the caller's duty
    /// under the audio contract.
    ///
    /// This is the caller's half of the output recovery split, and the whole of
    /// it. A fatal mid-run *stream* error is explicitly not handled here: RFC
    /// 0012 §4.3 item 8 gives that case to `Engine::play()`, which closes and
    /// reopens the device object the engine already owns and keeps the position
    /// it froze until an explicit retry. Stream-error teardown never arms
    /// [`Engine::is_flush_failed`]. An actual flush refusal (including a later
    /// stop/seek) still takes precedence. See `docs/tasks/W23-rfc0012-gaps.md` §4.
    pub fn ensure_engine_ready(&mut self) -> Result<()> {
        if self.engine.is_flush_failed() {
            self.note(
                "recreating audio engine and output after terminal flush refusal".to_string(),
            );
            self.recreate_engine()?;
        }
        Ok(())
    }

    /// The loaded configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Sets the path where configuration will be persisted on exit.
    pub fn set_config_path(&mut self, path: Option<PathBuf>) {
        self.config_path = path;
        self.load_eq_user_presets();
    }

    /// The path where configuration will be persisted, if set.
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// Returns the live configuration reflecting the player's current runtime state.
    pub fn current_config(&self) -> Config {
        let player_state = self.engine.state();
        let volume = player_state.volume;
        let balance = player_state.balance;

        let mut windows = self.config.ui.windows.clone();
        if let Some(main) = windows.iter_mut().find(|w| w.id == WindowId::Main) {
            main.shaded = self.ui.state().windowshade;
        } else {
            windows.push(WindowLayout {
                id: WindowId::Main,
                x: 26,
                y: rustamp_ui::DEFAULT_MAIN_POSITION.y,
                visible: true,
                shaded: self.ui.state().windowshade,
                width: None,
                height: None,
            });
        }
        if let Some(eq) = windows.iter_mut().find(|w| w.id == WindowId::Equalizer) {
            eq.visible = self.eq_open();
        } else {
            windows.push(WindowLayout {
                id: WindowId::Equalizer,
                x: 26,
                y: rustamp_ui::DEFAULT_EQUALIZER_POSITION.y,
                visible: self.eq_open(),
                shaded: false,
                width: None,
                height: None,
            });
        }
        if let Some(pl) = windows.iter_mut().find(|w| w.id == WindowId::Playlist) {
            pl.visible = self.playlist_open();
        } else {
            windows.push(WindowLayout {
                id: WindowId::Playlist,
                x: 26,
                y: rustamp_ui::DEFAULT_PLAYLIST_POSITION.y,
                visible: self.playlist_open(),
                shaded: false,
                width: None,
                height: None,
            });
        }

        Config {
            version: rustamp_core::config::CONFIG_VERSION,
            audio: AudioConfig {
                volume,
                balance,
                replaygain: replaygain_mode_from_audio(self.engine.replaygain_mode()),
                ..self.config.audio.clone()
            },
            ui: UiConfig {
                skin: self.current_skin.clone(),
                double_size: self.ui.state().double_size,
                always_on_top: self.always_on_top,
                easy_move: self.easy_move,
                windows,
                ..self.config.ui.clone()
            },
            eq: EqConfig {
                state: self.engine.eq(),
            },
            playlist: PlaylistConfig::default(),
            plugins: PluginsConfig {
                // What the plug-in pages edit: the folder and the off-list.
                // The rest stays unpersisted (see `config::serialize_honest`).
                plugin_dir: self.config.plugins.plugin_dir.clone(),
                disabled: self.config.plugins.disabled.clone(),
                ..PluginsConfig::default()
            },
        }
    }

    /// The plug-in settings the preferences pages edit.
    pub fn plugins_config(&self) -> &PluginsConfig {
        &self.config.plugins
    }

    /// The folder the plug-in pages scan: the configured one, else `Plugins`
    /// beside the executable (the classic layout).
    pub fn plugin_dir(&self) -> PathBuf {
        self.config.plugins.plugin_dir.clone().unwrap_or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join("Plugins")))
                .unwrap_or_else(|| PathBuf::from("Plugins"))
        })
    }

    /// Where the approval store lives: beside the configuration file (the
    /// executable's folder when no configuration path is set).
    pub fn plugin_approvals_path(&self) -> PathBuf {
        let beside = self
            .config_path
            .as_deref()
            .and_then(Path::parent)
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .or_else(|| {
                std::env::current_exe()
                    .ok()
                    .and_then(|exe| exe.parent().map(Path::to_path_buf))
            })
            .unwrap_or_default();
        beside.join(rustamp_prefs::APPROVALS_FILE)
    }

    /// Reads the plug-in folder and the approval store again. Reads files as
    /// bytes only; nothing is loaded or executed.
    pub fn rescan_plugins(&mut self) {
        self.plugin_scan = rustamp_prefs::scan(&self.plugin_dir(), &self.plugin_approvals_path());
    }

    /// The last plug-in scan.
    pub fn plugin_scan(&self) -> &rustamp_prefs::PluginScan {
        &self.plugin_scan
    }

    /// Switches the scanned plug-in `index` on or off. Switching on is refused
    /// unless the approval store approved the exact image
    /// ([`rustamp_prefs::set_enabled`]); there is no other way to enable one.
    pub fn set_plugin_enabled(
        &mut self,
        index: usize,
        on: bool,
    ) -> Result<(), rustamp_prefs::EnableRefused> {
        let Some(entry) = self.plugin_scan.entries.get(index) else {
            return Ok(());
        };
        rustamp_prefs::set_enabled(&mut self.config.plugins, entry, on)
    }

    /// Saves the current configuration to the configured path, if one is set.
    pub fn save_config(&self) -> Result<(), crate::config::ConfigError> {
        let Some(path) = &self.config_path else {
            return Ok(());
        };
        self.save_config_to(path)
    }

    /// Saves the current configuration to `path`.
    pub fn save_config_to(&self, path: &Path) -> Result<(), crate::config::ConfigError> {
        let config = self.current_config();
        crate::config::save(&config, path)
    }

    /// Records a window's screen coordinates in the config.
    pub fn set_window_position(&mut self, id: WindowId, x: i32, y: i32) {
        if let Some(entry) = self.config.ui.windows.iter_mut().find(|w| w.id == id) {
            entry.x = x;
            entry.y = y;
        } else {
            self.config.ui.windows.push(WindowLayout {
                id,
                x,
                y,
                visible: true,
                shaded: false,
                width: None,
                height: None,
            });
        }
    }

    /// Records a window's visibility state in the config.
    pub fn set_window_visible(&mut self, id: WindowId, visible: bool) {
        if let Some(entry) = self.config.ui.windows.iter_mut().find(|w| w.id == id) {
            entry.visible = visible;
        } else {
            self.config.ui.windows.push(WindowLayout {
                id,
                x: 26,
                y: 26,
                visible,
                shaded: false,
                width: None,
                height: None,
            });
        }
    }

    /// Records a window's shaded state in the config.
    pub fn set_window_shaded(&mut self, id: WindowId, shaded: bool) {
        if let Some(entry) = self.config.ui.windows.iter_mut().find(|w| w.id == id) {
            entry.shaded = shaded;
        } else {
            self.config.ui.windows.push(WindowLayout {
                id,
                x: 26,
                y: 26,
                visible: true,
                shaded,
                width: None,
                height: None,
            });
        }
    }

    /// A one-line description of the loaded skin.
    pub fn skin_description(&self) -> &str {
        self.skin.description()
    }

    /// Whether the app should exit.
    pub fn quit(&self) -> bool {
        self.quit
    }

    /// Whether the presenter should show the main window (F3/F5 show/hide).
    pub fn main_visible(&self) -> bool {
        self.main_visible
    }

    /// Whether the app uses the null output (`--null` headless or `--no-audio`
    /// windowed). This reports the device, not the presentation: a
    /// `--no-audio` run still opens the window.
    pub fn headless(&self) -> bool {
        self.clock.is_some()
    }

    /// Always-on-top (S02 §2, clutter-bar `A`).
    pub fn always_on_top(&self) -> bool {
        self.always_on_top
    }

    /// Easy-move (RFC 0005, id 40186).
    pub fn easy_move(&self) -> bool {
        self.easy_move
    }

    /// Whether a left press at main-window-relative `(x, y)` should move the
    /// window instead of reaching a control: easy-move is on — the core
    /// contract's "drag windows from anywhere" (`UiConfig::easy_move`) — and
    /// the press hit no widget, so no control owns it. Sliders, the marquee
    /// hand-scroll, buttons and held states all hit-test to a widget and keep
    /// their presses; a background press reaches nothing and drags.
    pub fn easy_drag_main(&self, x: i32, y: i32) -> bool {
        self.easy_move && rustamp_ui::hit_test(&self.layout, x, y).is_none()
    }

    /// The path of the current skin, when it came from disk.
    pub fn current_skin(&self) -> Option<&PathBuf> {
        self.current_skin.as_ref()
    }

    /// The client area the OS window must have right now, in physical pixels.
    pub fn frame_size(&self) -> (u32, u32) {
        frame_size(self.ui.state())
    }

    /// The client area the equalizer's OS window must have right now (S05
    /// §2.1: 275 x 116, 550 x 232 double-sized; the shade size follows the
    /// existing docking geometry, S05 §1.5 open).
    pub fn eq_frame_size(&self) -> (u32, u32) {
        eq_frame_size(self.eq.state())
    }

    /// Takes the pending "minimize the main window" request, if any.
    pub fn take_minimize_request(&mut self) -> bool {
        std::mem::take(&mut self.minimize_requested)
    }

    /// Whether `window` exists and can take focus right now.
    ///
    /// The main window counts while it is shown ([`App::main_visible`]); the
    /// equalizer and the playlist editor count while their open flags say so.
    /// A closed or hidden window cannot take focus, and windows the app does
    /// not have (`Vis`, `Gen`, `Video`) report closed.
    pub fn window_open(&self, window: WindowId) -> bool {
        match window {
            WindowId::Main => self.main_visible,
            WindowId::Equalizer => self.eq_open(),
            WindowId::Playlist => self.playlist_open(),
            WindowId::Vis | WindowId::Gen(_) | WindowId::Video => false,
        }
    }

    /// Which player window holds keyboard focus (see [`crate::focus`]).
    pub fn focused_window(&self) -> WindowId {
        self.focused_window
    }

    /// Takes the pending "focus this OS window" request, if any.
    pub fn take_focus_request(&mut self) -> Option<WindowId> {
        self.focus_request.take()
    }

    /// Takes the pending "un-minimize the main OS window" request, if any.
    pub fn take_restore_request(&mut self) -> bool {
        std::mem::take(&mut self.restore_request)
    }

    /// Drains the accumulated notes (unimplemented commands, fallbacks).
    pub fn drain_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    /// Asks the app to open one of the playlist window's dialogs (S06 §4).
    ///
    /// The spec settles the dialogs' existence and titles and fixes no dialog
    /// behaviour beyond that. When the dialog's opening command maps to a real
    /// dialog ([`App::open_dialog_for`]), the dialog opens and no note is
    /// recorded; otherwise the spec'd title goes through the existing
    /// dialog-stub path and opens no window. Returns the title the real
    /// dialog must carry.
    pub fn request_playlist_dialog(&mut self, dialog: PlaylistDialog) -> &'static str {
        let title = dialog.title();
        if let Some(command) = dialog.opening_command()
            && self.open_dialog_for(&command).is_some()
        {
            return title;
        }
        self.note(format!(
            "open dialog \"{title}\" ({dialog:?}): the dialog is not implemented yet"
        ));
        title
    }

    /// Asks the app to open the equalizer's presets popup (S07 §4.2).
    ///
    /// The spec settles the popup's existence (window class `#32768`), size
    /// and top-level entries (Load/Save/Delete) and leaves the submenus open
    /// (S07 §12 O12), so the submenu contents are our own choice (see
    /// [`crate::eq_presets`]). The request is queued; the presenter takes it
    /// on its tick ([`App::take_eq_menu`]) and shows the native popup.
    /// Returns the entries the popup carries.
    pub fn request_eq_presets_menu(&mut self, request: EqMenuRequest) -> &'static [&'static str] {
        self.pending_eq_menu = true;
        request.entries()
    }

    /// Applies the built-in equalizer preset `name` (ignoring case).
    ///
    /// Sets all ten bands through ordinary `SetBand` commands and leaves the
    /// preamp and the on/auto flags alone. The engine's resulting gains are
    /// what the configuration saves, so the choice persists (see
    /// [`crate::eq_presets`]). Returns `false` when no such preset exists.
    pub fn apply_eq_preset(&mut self, name: &str) -> Result<bool> {
        let Some(preset) = crate::eq_presets::find_preset(name) else {
            return Ok(false);
        };
        for (band, &gain_db) in preset.bands.iter().enumerate() {
            self.submit(Command::SetBand {
                band: band as u8,
                gain_db,
            })?;
        }
        Ok(true)
    }

    /// The built-in preset the current band gains correspond to, if any.
    pub fn current_eq_preset(&self) -> Option<&'static str> {
        crate::eq_presets::preset_name_for(&self.engine.eq())
    }

    /// Asks the app to exit (the window's close button).
    pub fn request_quit(&mut self) {
        self.quit = true;
    }

    /// Records the window's focus state (S01 §2.1: the focused title bar is the
    /// window's active render).
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Renders one frame of the main window.
    pub fn render(&mut self) -> Image {
        self.ui.refresh(self.now_ms());
        if self.focused == self.ui.state().title_bar_focused {
            return render_main_window(self.ui.state(), self.skin.skin(), &self.layout);
        }
        // `MainWindow` has no `set_focused` (a contract gap), so the focus
        // variant is applied to a copy of the render state.
        let mut state = self.ui.state().clone();
        state.title_bar_focused = self.focused;
        render_main_window(&state, self.skin.skin(), &self.layout)
    }

    /// Renders one frame of the equalizer window (S01 §3, S07).
    ///
    /// The equalizer has no time-driven parts, so no refresh is needed; the
    /// focus variant follows the window's own focus flag, which the presenter
    /// keeps via [`App::set_eq_focused`].
    pub fn render_eq(&self) -> Image {
        render_equalizer_window(self.eq.state(), self.skin.skin(), &self.eq_layout)
    }

    /// Records the equalizer window's focus state (S01 §3: the focused title
    /// bar is the window's active render).
    pub fn set_eq_focused(&mut self, focused: bool) {
        self.eq.set_focused(focused);
    }

    /// Sends a command into the command channel and processes it immediately.
    ///
    /// This is the **external** path (plugins, the platform layer, tests): the
    /// UI machine has not seen the command, so [`App::apply_from`] lets it
    /// predict the S02 effect as well.
    pub fn submit(&mut self, command: Command) -> Result<()> {
        self.commands.send(command)?;
        self.pump()
    }

    /// The single pump step: drain commands, poll the engine, drain events,
    /// forward the newest actual tap snapshot and refresh the UI.
    pub fn pump(&mut self) -> Result<()> {
        let mut commands = Vec::new();
        while let Some(command) = self.command_rx.try_recv()? {
            commands.push(command);
        }
        for command in &commands {
            self.apply_from(command, false);
        }

        self.engine.poll()?;

        let events: Vec<Event> = self.events.try_iter().collect();
        for event in &events {
            self.handle_event(event);
        }

        let now_ms = self.now_ms();
        self.pump_visualization(now_ms);
        self.ui.refresh(now_ms);
        // Supervise the plugin host before mirroring state to it (see
        // `crate::supervise`): non-blocking by contract, so a missing or dead
        // host only costs a cooldown-gated spawn attempt and the mirror below
        // degrades to detached. Unmanaged by default (no spawns in tests).
        if let Some(supervisor) = self.host_supervisor.as_mut() {
            supervisor.supervise(&mut self.host);
        }
        // Mirror the player state to the plugin host (see `crate::host`):
        // best-effort by contract, so the outcome is ignored — a detached or
        // dead host neither fails nor blocks the pump.
        let _ = self.poll_host();
        let _ = self.publish_host_state();
        Ok(())
    }

    /// UI-thread only: drain once, analyse at most one snapshot. Missing input
    /// is not silence; refresh still ages the existing deterministic envelopes.
    /// Source/device rates may differ: the tap is before device conversion, so
    /// use the decoder's exact rate, never the device rate or rounded UI kHz.
    ///
    /// An analysis error is only noted, never fatal: `VisError::NonFinitePcm`
    /// is an offline validation invariant that the live path cannot reach
    /// (RFC 0014 §4.5), and noting a rejected frame must not halt playback.
    fn pump_visualization(&mut self, now_ms: u64) {
        let frame = self.engine.take_vis_frame();
        if std::mem::take(&mut self.vis_discontinuity) {
            // A batch straddling a known boundary is ambiguous. Even a valid
            // new frame in this batch is sacrificed; no replacement is made.
            return;
        }
        if let Some(frame) = frame {
            let source_rate_hz = self.engine.source_spec().map_or(0, |s| s.sample_rate);
            if let Err(error) = self.ui.push_vis_frame(&frame, source_rate_hz, now_ms) {
                self.note(error.to_string());
            }
        }
    }

    fn invalidate_visualization(&mut self) {
        self.vis_discontinuity = true;
        self.ui.reset_visualization();
    }

    /// Applies one external command (see [`App::submit`]).
    pub fn apply(&mut self, command: &Command) {
        self.apply_from(command, false);
    }

    /// Applies one command to the engine, to the app when the app owns it, and
    /// to the UI machine unless the UI produced it (the UI machine already
    /// applied its own S02 prediction for its own commands).
    fn apply_from(&mut self, command: &Command, from_ui: bool) {
        if matches!(command, Command::Seek { .. } | Command::SeekRelative { .. }) {
            // Includes external/plugin commands as well as the UI's predicted
            // seek. No public seek-completion/generation tag exists on frames.
            self.invalidate_visualization();
        }
        if self.engine.is_flush_failed() {
            let _ = self.ensure_engine_ready();
        }
        // The engine ignores everything it does not own, so a single forward is
        // enough; a command it cannot apply (Play on an empty queue) becomes an
        // error event instead of taking the app down.
        if let Err(error) = self.engine.apply(command) {
            self.publish_error(&error);
        }
        if routing::app_owns(command) {
            self.apply_app(command);
        }
        if !from_ui {
            self.ui.handle_command(command, self.now_ms());
        }
        // S05 §2.1: the double-size flag reaches the equalizer, so the EQ
        // machine mirrors the main window's flag (its size follows from
        // `eq_frame_size`; shading the main never resizes it, S05 §3.2).
        if matches!(command, Command::ToggleDoubleSize) {
            self.eq.set_double_size(self.ui.state().double_size);
        }
        // A toggle may have closed the focused window underneath the focus.
        self.maintain_focus();
    }

    /// Moves focus to the next open player window (`FocusNextWindow`, S03 id
    /// 40063) in [`crate::focus::FOCUS_ORDER`], skipping closed windows. Sizes
    /// are untouched: focusing never resizes.
    fn focus_next_window(&mut self) {
        let next = crate::focus::next_focus(self.focused_window, |window| self.window_open(window));
        // With the main window hidden and nothing else open the cycle has no
        // window to give the focus to; a hidden window cannot take it.
        if self.window_open(next) {
            self.focused_window = next;
            self.focus_request = Some(next);
        }
    }

    /// Shows the main window (`ShowMainWindow`, S03 id 40258): show,
    /// un-minimize, raise and focus it. One-way by contract: unlike
    /// `ToggleWindow(Main)` it never hides. A minimize that has not reached
    /// the OS yet is dropped, since there is nothing to restore from then.
    fn show_main_window(&mut self) {
        self.main_visible = true;
        self.minimize_requested = false;
        self.restore_request = true;
        self.focused_window = WindowId::Main;
        self.focus_request = Some(WindowId::Main);
    }

    /// Repairs the focus after a window closed: a focused window that is no
    /// longer open falls back to the main window, with a request so the
    /// presenter follows. A hidden main window is not repaired (it is the
    /// fallback, and a hidden window cannot take focus).
    fn maintain_focus(&mut self) {
        if self.focused_window != WindowId::Main && !self.window_open(self.focused_window) {
            self.focused_window = WindowId::Main;
            if self.main_visible {
                self.focus_request = Some(WindowId::Main);
            }
        }
    }

    /// Applies a core [`Event`]: the app's own bookkeeping first, then the UI
    /// machine (the authoritative state).
    pub fn handle_event(&mut self, event: &Event) {
        // A track that actually opened: read its ReplayGain tags and drive the
        // engine's ReplayGain stage from them. Absent tags leave the control's
        // values unset, so the mode stays harmless (unity).
        if let Event::TrackChanged {
            metadata: Some(metadata),
            ..
        } = event
        {
            let tags = ReplayGainTags::from_metadata(metadata);
            let mode = self.engine.replaygain_mode();
            self.engine.set_replaygain(
                mode,
                tags.track_gain_db,
                tags.track_peak,
                tags.album_gain_db,
                tags.album_peak,
            );
        }
        if matches!(
            event,
            Event::TrackChanged { .. } | Event::StreamInfoChanged(_)
        ) {
            self.invalidate_visualization();
        }
        match event {
            Event::Quit => self.quit = true,
            Event::SkinChanged(path) => self.current_skin = path.clone(),
            Event::ModesChanged { shuffle, repeat } => {
                self.shuffle = *shuffle;
                self.repeat = *repeat;
            }
            Event::PlaylistEnded => self.playlist_ended = true,
            Event::EqChanged(eq) => {
                self.eq.sync_from_eq_state(eq);
            }
            Event::Error(info) => self.surface_error(info),
            // A track that actually opened clears the error it replaced.
            Event::TrackChanged { index: Some(_), .. } => self.last_error = None,
            _ => {}
        }

        // The engine reports `TrackChanged` with `id: None` (it holds no
        // playlist ids), but the UI machine uses `id.is_some()` as the "a track
        // is loaded" flag. The app is the state owner, so it fills the id in
        // from its own queue before the event reaches the UI.
        if let Event::TrackChanged {
            id: None,
            index,
            metadata,
            duration_ms,
        } = event
        {
            let id = self.entry_id_for(*index);
            let normalised = Event::TrackChanged {
                id,
                index: *index,
                metadata: metadata.clone(),
                duration_ms: *duration_ms,
            };
            self.ui.handle_event(&normalised, self.now_ms());
            return;
        }

        self.ui.handle_event(event, self.now_ms());
    }

    /// Applies a [`PlayerState`] snapshot by translating it into the events the
    /// UI machine already understands. This is how the app seeds the window at
    /// start-up and after a state resync.
    pub fn sync_from_player_state(&mut self, state: &PlayerState) {
        let now = self.now_ms();
        self.ui
            .handle_event(&Event::StateChanged(state.transport), now);
        self.ui
            .handle_event(&Event::StreamInfoChanged(state.stream), now);
        self.ui.handle_event(
            &Event::VolumeChanged {
                volume: state.volume,
                balance: state.balance,
            },
            now,
        );
        self.ui.handle_event(
            &Event::ModesChanged {
                shuffle: state.shuffle,
                repeat: state.repeat,
            },
            now,
        );
        if state.current_index.is_some() || state.current_id.is_some() {
            self.ui.handle_event(
                &Event::TrackChanged {
                    id: state.current_id,
                    index: state.current_index,
                    metadata: None,
                    duration_ms: state.duration_ms,
                },
                now,
            );
        }
        self.ui.handle_event(
            &Event::PositionChanged {
                position_ms: state.position_ms,
                duration_ms: state.duration_ms,
            },
            now,
        );
        self.shuffle = state.shuffle;
        self.repeat = state.repeat;
        self.always_on_top = self.config.ui.always_on_top;
        self.ui.refresh(now);
    }

    /// Assemble the snapshot the plugin host mirrors (sources per field are in
    /// [`crate::host`]).
    ///
    /// The engine owns transport, position, duration, volume, balance, the
    /// queue (length and current index), the stream and stop-after-current;
    /// the app owns shuffle/repeat (the engine deliberately reports `false`)
    /// and the stable entry ids (the engine reports `None`). The equalizer
    /// state is the engine's, which owns the S07 stage.
    pub fn host_player_state(&mut self) -> PlayerState {
        let mut state = self.engine.state();
        // App-owned modes: the engine does not track them (RFC 0001 / S15).
        state.shuffle = self.shuffle;
        state.repeat = self.repeat;
        // Stable entry id for the engine's current index (the engine has no
        // playlist ids; a `None` index yields `None` without allocating).
        state.current_id = self.entry_id_for(state.current_index);
        state
    }

    /// Build the one protocol message that feeds the host's state bridge.
    ///
    /// This is the single construction site: every `PlayerStateUpdate` the app
    /// sends passes through here, so the field-by-field audit in
    /// [`crate::host`] stays checkable in one place.
    pub fn player_state_update(&mut self) -> Message {
        player_state_message(self.host_player_state())
    }

    /// Attach the host's control-channel transport (see
    /// [`HostStateLink::attach`](crate::host::HostStateLink::attach)).
    /// Hand-attaching bypasses the supervisor: a managed app re-establishes
    /// its own transport on the next pump (see [`App::manage_host`]).
    pub fn attach_host(&mut self, transport: impl Transport + 'static) {
        self.host.attach(transport);
    }

    /// Drop the host transport without sending anything.
    pub fn detach_host(&mut self) {
        self.host.detach();
    }

    /// Whether a host transport is currently attached.
    pub fn host_attached(&self) -> bool {
        self.host.is_attached()
    }

    /// Install the plugin host supervisor (see [`crate::supervise`]).
    ///
    /// From the next [`pump`](App::pump) on, the supervisor starts
    /// `rustamp-plughost32` when it is missing and re-attaches (or restarts)
    /// it when it dies — off the audio path, degrading to detached. Pass
    /// [`Supervisor::from_search`](crate::supervise::Supervisor::from_search)
    /// for the normal binary resolution.
    pub fn manage_host(&mut self, supervisor: Supervisor) {
        self.host_supervisor = Some(supervisor);
    }

    /// Stop supervising the host: kill the managed child (best-effort,
    /// without waiting) and detach the link without sending anything.
    pub fn stop_host(&mut self) {
        if let Some(supervisor) = self.host_supervisor.as_mut() {
            supervisor.stop(&mut self.host);
        }
    }

    /// Publish the current snapshot to the host under the cadence rules (see
    /// [`crate::host`]).
    ///
    /// Best-effort by contract: a detached or dead host is reported, never
    /// raised — [`App::pump`] ignores the outcome, so state publishing can
    /// neither fail playback nor block it.
    pub fn publish_host_state(&mut self) -> PublishOutcome {
        let state = self.host_player_state();
        self.host.publish(&state)
    }

    /// Run one `Hello`/`Ping` liveness step on the host channel (see
    /// [`HostStateLink::poll`](crate::host::HostStateLink::poll)).
    ///
    /// Best-effort by contract like [`publish_host_state`](App::publish_host_state):
    /// [`App::pump`] calls it before publishing and ignores the outcome. A
    /// missed `Pong`, a foreign `Hello` or a dead channel detaches the link
    /// here, so the publish right after degrades to detached and the
    /// supervisor replaces the host on its next step.
    pub fn poll_host(&mut self) -> PollOutcome {
        self.host.poll()
    }

    // -----------------------------------------------------------------------
    // UI input (the window forwards OS input here)
    // -----------------------------------------------------------------------

    /// A left-button press at window-relative `(x, y)`.
    pub fn mouse_down(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let commands = self.ui.mouse_down(x, y, self.now_ms());
        self.forward(&commands)?;
        Ok(commands)
    }

    /// A pointer move (drag).
    pub fn mouse_move(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let commands = self.ui.mouse_move(x, y, self.now_ms());
        self.forward(&commands)?;
        Ok(commands)
    }

    /// A left-button release at window-relative `(x, y)`.
    pub fn mouse_up(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let commands = self.ui.mouse_up(x, y, self.now_ms());
        self.forward(&commands)?;
        Ok(commands)
    }

    /// A right-button press at window-relative `(x, y)` (S03 §5).
    ///
    /// The EQ and PL buttons toggle their window ("menu" grade); the long
    /// title, the time display and the seek bar have their own context menus,
    /// whose contents are S04's and unmeasured, so they are noted and open
    /// nothing; anywhere else opens the main-menu popup ("Right-click the main
    /// window: main-menu popup", verified).
    pub fn mouse_right_down(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let command = match rustamp_ui::hit_test(&self.layout, x, y) {
            Some(rustamp_ui::Widget::EqToggle) => Command::ToggleWindow(WindowId::Equalizer),
            Some(rustamp_ui::Widget::PlaylistToggle) => Command::ToggleWindow(WindowId::Playlist),
            Some(
                widget @ (rustamp_ui::Widget::Marquee
                | rustamp_ui::Widget::TimeDisplay
                | rustamp_ui::Widget::PositionSlider),
            ) => {
                self.note(format!(
                    "right-click on {widget:?}: its context menu's contents are unmeasured (S04)"
                ));
                return Ok(Vec::new());
            }
            _ => Command::OpenMainMenu,
        };
        // Through the external path: the UI machine has not predicted it.
        self.submit(command.clone())?;
        Ok(vec![command])
    }

    /// A key press. Returns the machine's outcome so the caller can log an
    /// [`ShortcutOutcome::Unhandled`] action with its command id.
    pub fn key_down(&mut self, key: Key, modifiers: Modifiers) -> Result<Option<ShortcutOutcome>> {
        let outcome = self.ui.key_down(key, modifiers, self.now_ms());
        if let Some(ShortcutOutcome::Commands(commands)) = &outcome {
            let commands = commands.clone();
            self.forward(&commands)?;
        } else {
            // `Handled` (for example the time-display toggle) may still have
            // changed the render state, so refresh.
            self.pump()?;
        }
        Ok(outcome)
    }

    /// The log line for an unhandled shortcut outcome, if that is what this is.
    pub fn unhandled_note(outcome: &ShortcutOutcome) -> Option<String> {
        match outcome {
            ShortcutOutcome::Unhandled(action) => Some(routing::unhandled_message(*action)),
            _ => None,
        }
    }

    // -----------------------------------------------------------------------
    // Equalizer-window input (the EQ window forwards OS input here)
    // -----------------------------------------------------------------------

    /// A left-button press at equalizer-window-relative `(x, y)`.
    pub fn eq_mouse_down(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let commands = self.eq.mouse_down(x, y);
        self.forward_eq(&commands)?;
        Ok(commands)
    }

    /// A pointer move over the equalizer window (drag).
    pub fn eq_mouse_move(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let commands = self.eq.mouse_move(x, y);
        self.forward_eq(&commands)?;
        Ok(commands)
    }

    /// A left-button release at equalizer-window-relative `(x, y)`.
    pub fn eq_mouse_up(&mut self, x: i32, y: i32) -> Result<Vec<Command>> {
        let commands = self.eq.mouse_up(x, y);
        self.forward_eq(&commands)?;
        Ok(commands)
    }

    /// A key press while the equalizer window is focused (S03 §3 via S07 §1:
    /// `N`/`A` toggles, `S` opens the presets menu; band/preamp steps are
    /// open per S07 §9 and stay unwired).
    pub fn eq_key_down(&mut self, key: Key, modifiers: Modifiers) -> Result<Vec<Command>> {
        let commands = self.eq.key_down(key, modifiers);
        self.forward_eq(&commands)?;
        Ok(commands)
    }

    /// Sends equalizer-produced commands into the channel and processes them.
    ///
    /// The equalizer machine has already applied its S07 prediction, so these
    /// go straight to the engine and the app (no second prediction) — the
    /// same shape as [`App::forward`]. A pending presets-menu request is
    /// queued for the presenter ([`App::take_eq_menu`]).
    fn forward_eq(&mut self, commands: &[Command]) -> Result<()> {
        for command in commands {
            self.apply_from(command, true);
        }
        if let Some(request) = self.eq.take_menu_request() {
            self.request_eq_presets_menu(request);
        }
        self.pump()
    }

    // -----------------------------------------------------------------------
    // Headless playback (the `--null` mode)
    // -----------------------------------------------------------------------

    /// Pumps the null output's virtual clock by `frames` frames.
    ///
    /// This is the deterministic half of the silent windowed mode
    /// (`--no-audio`): it moves the audio clock without any hardware, so a
    /// windowed run plays silently instead of stalling on a full ring. With a
    /// real device there is no virtual clock and this is a no-op (the device
    /// pulls on its own thread).
    pub fn advance_null_clock(&mut self, frames: u64) {
        if let Some(clock) = self.clock.as_ref() {
            clock.pump(frames);
        }
    }

    /// Advances the null output's virtual clock by the real time elapsed since
    /// the previous drive.
    ///
    /// The windowed presenter calls this once per frame so a `--no-audio` run
    /// plays silently in real time, exactly as [`App::run_headless`] drives
    /// the same clock headless. With a real device this is a no-op.
    pub fn drive_null_clock(&mut self) {
        if self.clock.is_none() {
            return;
        }
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_null_tick);
        self.last_null_tick = now;
        let rate = self.engine.device_spec().sample_rate;
        if rate == 0 {
            return;
        }
        self.pending_null_frames += elapsed.as_secs_f64() * f64::from(rate);
        let frames = self.pending_null_frames.floor() as u64;
        self.pending_null_frames -= frames as f64;
        if frames > 0 {
            self.advance_null_clock(frames);
        }
    }

    /// Plays the queue to its end (or to `--seconds`) on a virtual device whose
    /// clock this loop drives in real time. No window is opened.
    pub fn run_headless(&mut self) -> Result<()> {
        if self.engine.queue().is_empty() {
            println!("rustamp: no input file; nothing to play");
            return Ok(());
        }
        let rate = self.engine.device_spec().sample_rate;
        let mut last_tick = Instant::now();
        let mut pending_frames = 0.0f64;
        let mut announced = false;
        // `Engine::finish_run` re-bases the timeline to zero, so the frames the
        // run actually produced are the maximum seen while it was live (plus
        // what the decode thread reported for the track that ended).
        let mut frames_played = 0u64;

        loop {
            std::thread::sleep(Duration::from_millis(5));
            let now = Instant::now();
            let elapsed = now.duration_since(last_tick);
            last_tick = now;
            if let Some(clock) = self.clock.as_ref() {
                pending_frames += elapsed.as_secs_f64() * f64::from(rate);
                let frames = pending_frames.floor() as u64;
                pending_frames -= frames as f64;
                if frames > 0 {
                    clock.pump(frames);
                }
            }
            self.pump()?;
            frames_played = frames_played.max(self.engine.timeline_frames());

            if !announced && self.engine.transport() == TransportState::Playing {
                announced = true;
                println!("rustamp: playing {}", self.engine.queue()[0].display());
            }
            if self.quit {
                break;
            }
            if let Some(limit) = self.seconds
                && self.engine.position_ms() >= limit.saturating_mul(1000)
            {
                println!("rustamp: reached --seconds {limit}, stopping");
                self.engine.stop()?;
                self.pump()?;
                break;
            }
            if self.playlist_ended
                || (announced && self.engine.transport() == TransportState::Stopped)
            {
                break;
            }
            // A file that never opened (bad path, no decoder) must not hang.
            if !announced && self.engine.last_error().is_some() {
                break;
            }
        }

        let frames = frames_played.max(self.engine.last_track_frames());
        let seconds = if rate == 0 {
            0.0
        } else {
            frames as f64 / f64::from(rate)
        };
        println!(
            "rustamp: played {frames} frames ({seconds:.2} s), {} skipped packets",
            self.engine.skipped_packets()
        );
        if let Some(error) = self.engine.last_error() {
            return Err(Error::Other(error.message.clone()));
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Internals
    // -----------------------------------------------------------------------

    /// Sends UI-produced commands into the channel and processes them.
    ///
    /// The UI machine has already applied its S02 prediction, so these go
    /// straight to the engine and the app (no second prediction).
    pub(crate) fn forward(&mut self, commands: &[Command]) -> Result<()> {
        for command in commands {
            self.apply_from(command, true);
        }
        self.pump()
    }

    /// Grows the app's entry-id table to the engine queue's length.
    ///
    /// The table mirrors the queue: when it grows, the playlist window's
    /// model grows with it ([`App::sync_playlist_entries`]), so the window
    /// reflects an add without waiting for the next button press. The sync
    /// fires only on growth (the queue-length change is the signal, not a
    /// poll), and selection of live entries and the scroll offset survive
    /// it ([`PlaylistWindow::set_entries`](rustamp_ui::PlaylistWindow::set_entries)).
    fn sync_entry_ids(&mut self) {
        let before = self.entry_ids.len();
        let len = self.engine.queue().len();
        while self.entry_ids.len() < len {
            let id = EntryId(self.next_entry_id);
            self.next_entry_id += 1;
            self.entry_ids.push(id);
        }
        if self.entry_ids.len() != before {
            self.sync_playlist_entries();
        }
    }

    /// The stable id of queue entry `index`.
    pub(crate) fn entry_id_for(&mut self, index: Option<u32>) -> Option<EntryId> {
        let index = index? as usize;
        if index >= self.engine.queue().len() {
            return None;
        }
        self.sync_entry_ids();
        self.entry_ids.get(index).copied()
    }

    pub(crate) fn note(&mut self, message: String) {
        if self.notes.len() < MAX_NOTES {
            self.notes.push(message);
        }
    }

    /// Synchronizes the equalizer window machine's state from the authoritative
    /// engine equalizer state.
    fn sync_eq_from_engine(&mut self) {
        let eq = self.engine.eq();
        self.eq.sync_from_eq_state(&eq);
    }

    fn publish_error(&self, error: &Error) {
        self.bus.publish(Event::Error(ErrorInfo::from(error)));
    }

    /// Makes an [`Event::Error`] reach the user and leaves the transport in a
    /// defined state (BETA-04).
    ///
    /// * A format change is a notice, not a failure (RFC 0002 C4): the run
    ///   continues, so it is only noted.
    /// * Any other error is noted (the presenter prints notes) and shown in
    ///   the native title ([`App::window_title`]), naming the current entry.
    /// * A running transport is stopped unless the run already reported its
    ///   end of stream (it then finishes on its own). The engine's decode
    ///   worker exits after an open failure without an end of stream, so
    ///   without this the engine stays `Playing` over silence. The rule is
    ///   deliberately broad: a mid-track read failure stops the run as well
    ///   instead of advancing, which is a defined state, and Play retries.
    /// * The UI's transport is re-synced to the engine's, so an optimistic
    ///   Playing light (Play on an empty queue) goes out again.
    fn surface_error(&mut self, info: &ErrorInfo) {
        if info.kind == ErrorKind::FormatChanged {
            self.note(format!("notice: {}", info.message));
            return;
        }
        let is_flush_refusal = self.engine.is_flush_failed();
        let entry = self
            .engine
            .current_index()
            .and_then(|index| self.engine.queue().get(index as usize))
            .map(|track| track.display());
        let message = match entry {
            Some(entry) if self.engine.transport() != TransportState::Stopped => {
                format!("cannot play {entry}: {}", info.message)
            }
            // Play with nothing queued: the engine's "queue index 0" means
            // nothing to a first user, so say what to do instead.
            _ if info.kind == ErrorKind::Other && self.engine.queue().is_empty() => {
                "nothing to play: the playlist is empty (Eject or ADD opens a file)".to_string()
            }
            _ => info.message.clone(),
        };
        self.note(format!("error: {message}"));
        if is_flush_refusal {
            self.note(
                "recreating audio engine and output after terminal flush refusal".to_string(),
            );
            if let Err(error) = self.recreate_engine() {
                let recreation_err = format!("error: recreating engine failed: {error}");
                self.note(recreation_err.clone());
                self.last_error = Some(recreation_err);
            } else {
                // The engine and output have been cleanly recreated as obliged by the audio contract.
                self.last_error = None;
            }
        } else {
            self.last_error = Some(message);
        }
        if self.engine.transport() != TransportState::Stopped
            && self.engine.end_of_stream().is_none()
            && let Err(error) = self.engine.stop()
        {
            self.note(format!("error: stopping after the failure failed: {error}"));
        }
        let transport = self.engine.transport();
        self.ui
            .handle_event(&Event::StateChanged(transport), self.now_ms());
    }

    fn publish_modes(&self) {
        self.bus.publish(Event::ModesChanged {
            shuffle: self.shuffle,
            repeat: self.repeat,
        });
    }

    fn publish_skin(&self) {
        self.bus
            .publish(Event::SkinChanged(self.current_skin.clone()));
    }

    /// Applies a command the application layer owns (see [`routing::app_owns`]).
    fn apply_app(&mut self, command: &Command) {
        use Command as C;
        match command {
            C::Quit => {
                self.quit = true;
                self.bus.publish(Event::Quit);
            }
            C::ToggleShuffle => {
                self.shuffle = !self.shuffle;
                self.publish_modes();
            }
            C::SetShuffle(on) => {
                self.shuffle = *on;
                self.publish_modes();
            }
            C::ToggleRepeat => {
                self.repeat = !self.repeat;
                self.publish_modes();
            }
            C::SetRepeat(on) => {
                self.repeat = *on;
                self.publish_modes();
            }
            C::ToggleAlwaysOnTop => self.always_on_top = !self.always_on_top,
            C::ToggleEasyMove => self.easy_move = !self.easy_move,
            C::Minimize => {
                // The later of Minimize and ShowMainWindow wins: a restore
                // that has not reached the OS yet is dropped.
                self.minimize_requested = true;
                self.restore_request = false;
                if self.focus_request == Some(WindowId::Main) {
                    self.focus_request = None;
                }
            }
            // Closing the focused window quits when that window is the main one
            // (RFC 0005 owner decision O3); it is the only window today.
            // An open dialog closes first instead of quitting the player.
            C::CloseFocusedWindow => {
                if !self.close_open_dialog() {
                    self.quit = true;
                    self.bus.publish(Event::Quit);
                }
            }
            C::ToggleDoubleSize | C::ToggleWindowShade(WindowId::Main) => {
                // The UI machine has already applied its S02 prediction; the
                // presenter reads the new size from `frame_size`.
            }
            C::ToggleWindow(WindowId::Main) => self.main_visible = !self.main_visible,
            C::ShowMainWindow => self.show_main_window(),
            C::ToggleWindow(WindowId::Equalizer) => {
                self.note(
                    "Equalizer window toggled (the render state drives the OS window); \
                     the presets submenu contents and load/save semantics are not \
                     implemented yet (S07 §12 O12, S04)"
                        .to_string(),
                );
            }
            C::ToggleWindow(WindowId::Playlist) => {
                // The UI machine flipped `playlist_open`; the presenter
                // creates or drops the playlist editor's OS window to match.
            }
            C::ToggleWindow(window) => {
                self.note(format!(
                    "{window:?} window is not implemented yet (the toggle is applied to the \
                     render state only)"
                ));
            }
            C::LoadSkin(path) => self.load_skin(path.clone()),
            C::RefreshSkin => {
                if let Some(path) = self.current_skin.clone() {
                    self.load_skin(path);
                } else {
                    self.note("refresh skin: the current skin is the built-in default".to_string());
                }
            }
            C::AddItems { items, at, play } => self.add_items(items, *at, *play),
            C::SetBand { .. } | C::SetPreamp(_) | C::SetEqEnabled(_) | C::SetEqAuto(_) => {
                // S07/Wave 35: The engine owns the authoritative EqState.
                // The equalizer window machine is driven directly from it so that
                // on-screen sliders and lights stay strictly synchronized after any
                // EQ command (external, shortcut, or UI round-trip).
                self.sync_eq_from_engine();
            }
            C::PlayEntry(id) => self.play_entry(*id),
            C::Remove(ids) => self.remove_entries(ids),
            C::Clear => self.clear_entries(),
            C::Select(op) => self.playlist.apply_select(op),
            C::Sort(key) => self.sort_entries(*key),
            C::Randomize => self.randomize_entries(),
            C::MoveEntries { ids, to } => self.move_entries(ids, *to as usize),
            C::Reverse => {
                self.note("playlist reverse is not wired to the app yet (S14/S15)".to_string());
            }
            C::OpenAbout
            | C::OpenFileInfo
            | C::OpenJump { .. }
            | C::OpenPlaylist
            | C::OpenMedia { .. }
            | C::OpenPrefs
            | C::OpenPrefsPage(_)
            | C::OpenSkinBrowser => {
                // Simple modal dialogs (`dialogs.rs`): the app opens a real
                // window for these, so no note is recorded. The open-media
                // faces confirm to `AddItems` with the opening command's
                // `play` flag carried through untouched (RFC 0005 O5).
                self.open_dialog_for(command);
            }
            C::OpenBookmarkEditor | C::AddBookmark => {
                self.note(format!(
                    "{command:?} needs a dialog that is not implemented yet"
                ));
            }
            // The main-menu popup (S04): the app owns the open state, the
            // presenter hosts the OS window for it.
            C::OpenMainMenu => self.open_main_menu(),
            C::FocusNextWindow => self.focus_next_window(),
            // The clutterbar popups (S04 §4, §5): the app owns the pending
            // request, the presenter shows the system menu.
            C::OpenOptionsMenu => self.open_clutter_menu(crate::menu::ClutterMenu::Options),
            C::OpenVisualizationMenu => {
                self.open_clutter_menu(crate::menu::ClutterMenu::Visualization);
            }
            // RFC 0017: the failure is recorded by `save_playlist_file` itself.
            C::SavePlaylist(path) => {
                let _ = self.save_playlist_file(path);
            }
            // RFC 0018: Alt+Delete removes the entries whose file is gone.
            C::RemoveDeadFiles => self.remove_dead_files(),
            C::ConfigureVisPlugin | C::ToggleVisPlugin => {
                self.note(format!("{command:?} needs the plugin host (M3)"));
            }
            // The remaining variants are the engine's (see `routing::app_owns`).
            _ => {}
        }
    }

    fn load_skin(&mut self, path: PathBuf) {
        let loaded = load_skin(&SkinChoice::Path(path));
        if let Some(warning) = loaded.warning() {
            self.note(warning.to_string());
        }
        self.current_skin = if loaded.fell_back() {
            None
        } else {
            loaded
                .description()
                .strip_prefix("skin ")
                .map(PathBuf::from)
        };
        self.skin = loaded;
        self.publish_skin();
    }

    /// Loads the built-in default skin (the skin browser's built-in row).
    ///
    /// The same tail as [`load_skin`](App::load_skin) without a path: the
    /// built-in skin always loads, so there is no fallback and the current
    /// skin becomes "no path", which [`RefreshSkin`](Command::RefreshSkin)
    /// already reads that way.
    pub(crate) fn load_builtin_skin(&mut self) {
        self.skin = crate::skin::builtin();
        self.current_skin = None;
        self.publish_skin();
    }

    /// Loads a playlist file from disk, replacing the current queue with the
    /// surviving entries in order.
    ///
    /// If `play` is true and at least one entry survived, playback of the first
    /// entry is started. If entries were skipped during parsing, the skip
    /// report is recorded in `self.last_playlist_report` and surfaced as
    /// informative notes (and printed to stderr in windowed mode).
    ///
    /// If the file is malformed, unrecognized, or cannot be read, the queue
    /// remains unchanged, an error is recorded in `last_error` (surfaced in
    /// [`App::window_title`]), a note is emitted, and `Err` is returned.
    pub fn load_playlist_file(
        &mut self,
        path: &Path,
        play: bool,
    ) -> Result<rustamp_playlist::PlaylistReport, rustamp_playlist::PlaylistError> {
        // Import (not just load) so the detected format survives: a playlist
        // saved back later defaults to the format it came in. An unrecognized
        // format falls back to M3U when the content is still a readable plain
        // list, and says so; otherwise the original error is reported.
        let imported = match rustamp_playlist::import_playlist_file(path) {
            Ok(imported) => imported,
            Err(original_err) => {
                let bytes = match std::fs::read(path) {
                    Ok(bytes) => bytes,
                    Err(io_err) => {
                        let msg = format!("failed to load playlist {}: {}", path.display(), io_err);
                        self.note(format!("error: {msg}"));
                        self.last_error = Some(msg);
                        self.publish_error(&rustamp_core::error::Error::Io(std::io::Error::new(
                            io_err.kind(),
                            io_err.to_string(),
                        )));
                        return Err(original_err);
                    }
                };
                let base = path.parent().unwrap_or_else(|| std::path::Path::new(""));
                match rustamp_playlist::read_playlist_report_as(
                    rustamp_playlist::PlaylistFormat::M3u,
                    &bytes,
                    base,
                ) {
                    Ok(report) => {
                        self.note(format!(
                            "unknown playlist format in {}; treating as M3U",
                            path.display()
                        ));
                        rustamp_playlist::ImportedPlaylist {
                            format: rustamp_playlist::PlaylistFormat::M3u,
                            report,
                        }
                    }
                    Err(_) => {
                        let msg = format!(
                            "failed to load playlist {}: {}",
                            path.display(),
                            original_err
                        );
                        self.note(format!("error: {msg}"));
                        self.last_error = Some(msg);
                        let core_err = match &original_err {
                            rustamp_playlist::PlaylistError::Io(e) => {
                                rustamp_core::error::Error::Io(std::io::Error::new(
                                    e.kind(),
                                    e.to_string(),
                                ))
                            }
                            rustamp_playlist::PlaylistError::UnsupportedFormat(m) => {
                                rustamp_core::error::Error::UnsupportedFormat(m.clone())
                            }
                            rustamp_playlist::PlaylistError::InvalidEntry(m) => {
                                rustamp_core::error::Error::InvalidArgument(m.clone())
                            }
                        };
                        self.publish_error(&core_err);
                        return Err(original_err);
                    }
                }
            }
        };
        let report = imported.report;
        self.last_playlist_format = Some(imported.format);

        if !report.is_clean() {
            self.note(format!(
                "loaded {} tracks, skipped {} malformed entries from {}",
                report.entries.len(),
                report.skip_count(),
                path.display()
            ));
            for skip in &report.skips {
                self.note(format!("  - skipped: {}", skip.reason));
            }
        }

        let queue: Vec<TrackSource> = report
            .entries
            .iter()
            .map(|entry| match &entry.location {
                MediaLocation::File(p) => TrackSource::path(p.clone()),
                MediaLocation::Url(u) => TrackSource::Url(u.clone()),
            })
            .collect();

        // A replacement must not lend the old track's index (or an in-flight
        // TrackStarted) to an unrelated entry in the new queue.
        self.clear_entries();
        self.engine.set_queue(queue);
        self.sync_entry_ids();

        if play && !report.entries.is_empty() {
            let _ = self.ensure_engine_ready();
            if let Err(error) = self.engine.play_index(0) {
                self.publish_error(&error);
            }
        }

        self.last_playlist_report = Some(report.clone());
        Ok(report)
    }

    /// Writes the current queue to `path` as an M3U, PLS or XSPF playlist, in
    /// queue order (RFC 0017).
    ///
    /// The format follows the file extension, case-insensitively. An unknown
    /// extension, B4S (not implemented) or a failed write records an error
    /// like a failed load does (a note, `last_error`, and the published
    /// error) and returns `Err`; no file is written in the first two cases.
    pub fn save_playlist_file(&mut self, path: &Path) -> Result<()> {
        let result = self.write_queue_playlist(path);
        if let Err(error) = &result {
            let msg = format!("failed to save playlist {}: {}", path.display(), error);
            self.note(format!("error: {msg}"));
            self.last_error = Some(msg);
            self.publish_error(error);
        }
        result
    }

    fn write_queue_playlist(&self, path: &Path) -> Result<()> {
        // Format selection rule (RFC 0017 import wiring):
        //   * a recognized, supported extension on `path` wins — the user
        //     chose it;
        //   * B4S or an unknown extension is rejected (the user named a
        //     format we cannot write);
        //   * no extension at all defaults to the format of the playlist that
        //     was last loaded (so a PLS loads and saves as PLS), or to M3U
        //     when nothing has been loaded yet.
        let format = if path.extension().is_none() {
            self.last_playlist_format
                .unwrap_or(rustamp_playlist::PlaylistFormat::M3u)
        } else {
            match rustamp_playlist::PlaylistFormat::from_path(path) {
                Some(format) if format.is_supported() => format,
                Some(_) => {
                    return Err(Error::UnsupportedFormat(format!(
                        "{}: save needs an .m3u, .m3u8, .pls or .xspf file name",
                        path.display()
                    )));
                }
                None => {
                    return Err(Error::UnsupportedFormat(format!(
                        "{}: save needs an .m3u, .m3u8, .pls or .xspf file name",
                        path.display()
                    )));
                }
            }
        };
        let entries: Vec<rustamp_playlist::LoadedEntry> = self
            .engine
            .queue()
            .iter()
            .map(|source| {
                let location = match source {
                    TrackSource::Path(p) => MediaLocation::File(p.clone()),
                    TrackSource::Url(u) => MediaLocation::Url(u.clone()),
                };
                rustamp_playlist::LoadedEntry::new(location)
            })
            .collect();
        let bytes = rustamp_playlist::write_playlist(format, &entries)?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    fn add_items(&mut self, items: &[MediaLocation], at: Option<u32>, play: bool) {
        let first_new = self.engine.queue().len() as u32;
        let mut queue = self.engine.queue().to_vec();
        for location in items {
            match location {
                MediaLocation::File(path) if is_playlist_file(path) => {
                    match rustamp_playlist::import_playlist_file(path) {
                        Ok(imported) => {
                            self.last_playlist_format = Some(imported.format);
                            let report = imported.report;
                            if !report.is_clean() {
                                self.note(format!(
                                    "loaded {} tracks, skipped {} malformed entries from {}",
                                    report.entries.len(),
                                    report.skip_count(),
                                    path.display()
                                ));
                                for skip in &report.skips {
                                    self.note(format!("  - skipped: {}", skip.reason));
                                }
                            }
                            queue.extend(report.entries.iter().map(|e| match &e.location {
                                MediaLocation::File(p) => TrackSource::path(p.clone()),
                                MediaLocation::Url(u) => TrackSource::Url(u.clone()),
                            }));
                            self.last_playlist_report = Some(report);
                        }
                        Err(err) => {
                            let msg =
                                format!("failed to load playlist {}: {}", path.display(), err);
                            self.note(format!("error: {msg}"));
                            self.last_error = Some(msg);
                            let core_err = match &err {
                                rustamp_playlist::PlaylistError::Io(e) => {
                                    rustamp_core::error::Error::Io(std::io::Error::new(
                                        e.kind(),
                                        e.to_string(),
                                    ))
                                }
                                rustamp_playlist::PlaylistError::UnsupportedFormat(m) => {
                                    rustamp_core::error::Error::UnsupportedFormat(m.clone())
                                }
                                rustamp_playlist::PlaylistError::InvalidEntry(m) => {
                                    rustamp_core::error::Error::InvalidArgument(m.clone())
                                }
                            };
                            self.publish_error(&core_err);
                        }
                    }
                }
                MediaLocation::File(path) => queue.push(TrackSource::path(path.clone())),
                MediaLocation::Url(url) => queue.push(TrackSource::Url(url.clone())),
            }
        }
        self.engine.set_queue(queue);
        self.sync_entry_ids();
        if let Some(at) = at
            && at < first_new
        {
            self.note(format!(
                "AddItems: insert position {at} is not supported yet; the entries were appended"
            ));
        }
        if play && !items.is_empty() {
            let _ = self.ensure_engine_ready();
            if let Err(error) = self.engine.play_index(first_new) {
                self.publish_error(&error);
            }
        }
    }

    fn play_entry(&mut self, id: EntryId) {
        if let Some(index) = self.entry_ids.iter().position(|entry| *entry == id) {
            let _ = self.ensure_engine_ready();
            if let Err(error) = self.engine.play_index(index as u32) {
                self.publish_error(&error);
            }
        } else {
            self.note(format!("PlayEntry: unknown entry id {id:?}"));
        }
    }

    fn remove_entries(&mut self, ids: &[EntryId]) {
        if ids.is_empty() {
            return;
        }
        let ids_set: BTreeSet<_> = ids.iter().copied().collect();

        // Check if the currently playing entry is being removed
        let playing_id = self
            .engine
            .current_index()
            .and_then(|idx| self.entry_ids.get(idx as usize))
            .copied();
        let playing_removed = playing_id.is_some_and(|id| ids_set.contains(&id));

        if playing_removed && self.engine.transport() != TransportState::Stopped {
            let _ = self.engine.stop();
        }

        let mut new_ids = Vec::with_capacity(self.entry_ids.len());
        let mut new_queue = Vec::with_capacity(self.engine.queue().len());
        for (id, track) in self.entry_ids.iter().zip(self.engine.queue()) {
            if !ids_set.contains(id) {
                new_ids.push(*id);
                new_queue.push(track.clone());
            }
        }
        self.entry_ids = new_ids;
        self.engine.set_queue(new_queue);
        // Keep a surviving current entry by identity, not by its former row.
        // Removing it clears Current even when another entry takes that index.
        let current = playing_id
            .and_then(|id| self.entry_ids.iter().position(|entry| *entry == id))
            .map(|index| index as u32);
        self.engine.set_current_index(current);
        self.sync_playlist_entries();
    }

    /// Applies `Command::Sort` to the playlist order (S03 §2 `40209`-`40211`,
    /// RFC 0005 §5: the app owns the playlist model).
    ///
    /// The key texts mirror core
    /// [`Playlist::sort`](rustamp_core::playlist::Playlist::sort):
    ///
    /// * `Title` — the name the last playlist load carried (`#EXTINF`, PLS
    ///   `TitleN`), matched by location and falling back to the file name, so
    ///   a queue the load did not name sorts like `FileName`;
    /// * `FileName` — the final path component;
    /// * `PathAndFileName` — the full location;
    /// * `Duration` — the duration the last load carried, unknown first
    ///   (`None` before `Some`, the core's rule).
    ///
    /// Text is compared case-insensitively and every sort is stable, so equal
    /// keys keep their relative order. Table, queue and model reorder together
    /// ([`App::reorder_entries`]); nothing else changes — the current entry
    /// keeps playing and ids keep their selection.
    fn sort_entries(&mut self, key: SortKey) {
        if self.entry_ids.len() < 2 {
            return;
        }
        // Titles and durations exist only for what the last playlist load
        // reported: no entry is probed for tags or properties (S14 title
        // formatting is open; `rustamp-meta` does not scan properties).
        let mut order: Vec<usize> = (0..self.entry_ids.len()).collect();
        match key {
            SortKey::Duration => {
                let durations = self.loaded_durations();
                order.sort_by_cached_key(|&index| {
                    self.queue_location(index)
                        .and_then(|location| durations.get(&location).copied())
                });
            }
            SortKey::Title | SortKey::FileName | SortKey::PathAndFileName => {
                // Only `Title` reads the loaded names; the other two sort on
                // the location alone.
                let titles = if key == SortKey::Title {
                    self.loaded_titles()
                } else {
                    HashMap::new()
                };
                order.sort_by_cached_key(|&index| self.sort_text(index, key, &titles));
            }
        }
        self.reorder_entries(order);
    }

    /// The case-insensitive sort text of queue row `index` for one of the
    /// three text keys; `titles` feeds `Title` only.
    ///
    /// A row the queue does not hold sorts as the empty string and leaves with
    /// the next sync; while the tables mirror each other that cannot happen.
    fn sort_text(
        &self,
        index: usize,
        key: SortKey,
        titles: &HashMap<MediaLocation, String>,
    ) -> String {
        let Some(location) = self.queue_location(index) else {
            return String::new();
        };
        let text = match key {
            SortKey::Title => titles
                .get(&location)
                .cloned()
                .unwrap_or_else(|| location.file_name()),
            SortKey::FileName => location.file_name(),
            SortKey::PathAndFileName => location.to_display_string(),
            // `Duration` takes the numeric branch of `Self::sort_entries`.
            SortKey::Duration => unreachable!("Duration sorts numerically"),
        };
        text.to_lowercase()
    }

    /// The media location of queue row `index`, if the row exists.
    fn queue_location(&self, index: usize) -> Option<MediaLocation> {
        Some(match self.engine.queue().get(index)? {
            TrackSource::Path(path) => MediaLocation::File(path.clone()),
            TrackSource::Url(url) => MediaLocation::Url(url.clone()),
        })
    }

    /// Names the last playlist load carried, by location.
    fn loaded_titles(&self) -> HashMap<MediaLocation, String> {
        self.last_playlist_report
            .as_ref()
            .map(|report| {
                report
                    .entries
                    .iter()
                    .filter_map(|entry| {
                        entry
                            .title
                            .as_ref()
                            .map(|title| (entry.location.clone(), title.clone()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Durations the last playlist load carried, by location.
    fn loaded_durations(&self) -> HashMap<MediaLocation, u64> {
        self.last_playlist_report
            .as_ref()
            .map(|report| {
                report
                    .entries
                    .iter()
                    .filter_map(|entry| {
                        entry
                            .duration_ms
                            .map(|duration| (entry.location.clone(), duration))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Applies `Command::Randomize` (S03 §2 `40212`, the `MISC -> Randomize
    /// List` action): one in-place permutation of the order.
    ///
    /// S15 §5 maps the action to `rustamp-playlist::order::randomize`, whose
    /// Fisher-Yates shuffle is [`rustamp_playlist::shuffle`]. The app holds no
    /// core `Playlist` — its order is the id table plus the engine queue — so
    /// the permutation is taken over row indices and handed to
    /// [`App::reorder_entries`]: ids, selection and the current entry survive,
    /// only the rows move.
    ///
    /// The seed is an own choice (S15 Q7.3 leaves the reference's seeding
    /// open): wall-clock nanoseconds give each press a fresh stream with no
    /// random-source dependency, and the generator itself is deterministic. A
    /// clock that does not advance only repeats the previous permutation, which
    /// is a legal outcome of a shuffle anyway.
    fn randomize_entries(&mut self) {
        if self.entry_ids.len() < 2 {
            return;
        }
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos() as u64);
        let mut rng = rustamp_playlist::Rng::new(seed);
        let mut order: Vec<usize> = (0..self.entry_ids.len()).collect();
        rustamp_playlist::shuffle(&mut order, &mut rng);
        self.reorder_entries(order);
    }

    /// Applies the existing block-move contract: ids keep playlist-relative
    /// order, unknown/duplicate ids are ignored, and `to` is clamped in the
    /// remaining list. Like sorting, playback and selection follow identity.
    fn move_entries(&mut self, ids: &[EntryId], to: usize) {
        let moved: std::collections::BTreeSet<_> = ids.iter().copied().collect();
        let (block, mut rest): (Vec<_>, Vec<_>) =
            (0..self.entry_ids.len()).partition(|&row| moved.contains(&self.entry_ids[row]));
        let to = to.min(rest.len());
        rest.splice(to..to, block);
        if rest.iter().copied().eq(0..self.entry_ids.len()) {
            return;
        }
        self.reorder_entries(rest);
    }

    /// Reorders the id table and the engine queue to `order`, a permutation of
    /// row indices, and re-syncs the playlist model.
    ///
    /// The tables mirror each other (see [`App::sync_entry_ids`]), so they move
    /// together. Playback is untouched: the current entry is carried by
    /// identity to its new row, the way [`App::remove_entries`] keeps a
    /// surviving current entry, so the track plays on and only the indicator
    /// moves. The model keeps selection by id across the re-sync.
    ///
    /// The current index is written before the queue because
    /// [`Engine::set_queue`] re-plans an active shuffle around the current
    /// index, which is only meaningful in the new order. An order that is not a
    /// row permutation of a matching queue is dropped rather than applied: half
    /// of it would desynchronize the tables.
    fn reorder_entries(&mut self, order: Vec<usize>) {
        let len = self.entry_ids.len();
        if len == 0
            || order.len() != len
            || self.engine.queue().len() != len
            || order.iter().any(|&row| row >= len)
        {
            return;
        }
        let new_ids: Vec<EntryId> = order.iter().map(|&row| self.entry_ids[row]).collect();
        let new_queue: Vec<TrackSource> = order
            .iter()
            .map(|&row| self.engine.queue()[row].clone())
            .collect();
        let current_id = self
            .engine
            .current_index()
            .and_then(|index| self.entry_ids.get(index as usize))
            .copied();
        self.entry_ids = new_ids;
        let current = current_id
            .and_then(|id| self.entry_ids.iter().position(|entry| *entry == id))
            .map(|index| index as u32);
        self.engine.set_current_index(current);
        self.engine.set_queue(new_queue);
        self.sync_playlist_entries();
    }

    fn clear_entries(&mut self) {
        if self.engine.transport() != TransportState::Stopped {
            let _ = self.engine.stop();
        }
        self.entry_ids.clear();
        self.engine.set_queue(Vec::new());
        self.engine.set_current_index(None);
        self.sync_playlist_entries();
    }

    /// Removes every entry whose local file no longer exists on disk
    /// ("Remove all dead files", Alt+Delete, S03 §2 command id 40222,
    /// RFC 0018).
    ///
    /// A playlist with no dead entry changes nothing and records no note — the
    /// same silence as an unstaged Delete (S06 §5.3). The removal itself is the
    /// ordinary [`App::remove_entries`] path, so the playing entry, the current
    /// index and the selection follow the rules every other removal uses.
    fn remove_dead_files(&mut self) {
        let dead: Vec<EntryId> = self
            .entry_ids
            .iter()
            .copied()
            .zip(self.engine.queue())
            .filter(|(_id, source)| source_is_dead(source))
            .map(|(id, _source)| id)
            .collect();
        if dead.is_empty() {
            return;
        }
        self.remove_entries(&dead);
    }
}

#[cfg(test)]
#[path = "visualization_tests.rs"]
mod visualization_tests;

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use rustamp_core::audio::{
        AudioSpec, Confidence, Decoder, DecoderFactory, MediaSource, ProbeHint,
    };
    use rustamp_core::meta::TrackMetadata;
    use rustamp_core::playlist::EntryId;
    use rustamp_core::state::StreamInfo;
    use rustamp_ui::{MainWindowLayout, Widget};

    /// A silent decoder for `*.tst` paths: enough to open a queue entry without
    /// a real media file.
    struct SilenceDecoder {
        spec: AudioSpec,
        pos: u64,
        total: u64,
    }

    impl Decoder for SilenceDecoder {
        fn spec(&self) -> AudioSpec {
            self.spec
        }

        fn read(&mut self, out: &mut [f32]) -> Result<usize> {
            let channels = usize::from(self.spec.channels);
            let want = out.len() / channels;
            let n = want.min(self.total.saturating_sub(self.pos) as usize);
            for sample in &mut out[..n * channels] {
                *sample = 0.0;
            }
            self.pos += n as u64;
            Ok(n)
        }

        fn seek(&mut self, to: Duration) -> Result<Duration> {
            let frames = (to.as_secs_f64() * f64::from(self.spec.sample_rate)) as u64;
            self.pos = frames.min(self.total);
            Ok(self.spec.duration_of(self.pos))
        }

        fn duration(&self) -> Option<Duration> {
            Some(self.spec.duration_of(self.total))
        }

        fn metadata(&self) -> TrackMetadata {
            TrackMetadata {
                title: Some("App Test Track".to_string()),
                ..TrackMetadata::default()
            }
        }

        fn stream_info(&self) -> StreamInfo {
            StreamInfo {
                bitrate_kbps: Some(128),
                sample_rate_hz: self.spec.sample_rate,
                channels: self.spec.channels,
            }
        }
    }

    struct TestFactory;

    impl DecoderFactory for TestFactory {
        fn name(&self) -> &str {
            "app-test"
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

        fn open(&self, _source: MediaSource) -> Result<Box<dyn Decoder>> {
            Ok(Box::new(SilenceDecoder {
                spec: AudioSpec::new(44100, 2),
                pos: 0,
                total: 44_100,
            }))
        }
    }

    fn registry() -> DecoderRegistry {
        let mut registry = DecoderRegistry::new();
        registry.register(Arc::new(TestFactory) as Arc<dyn DecoderFactory>);
        registry
    }

    /// A temp dir with one `*.tst` file in it.
    fn track(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, b"").unwrap();
        path
    }

    fn app_with_file(path: Option<PathBuf>) -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let options = AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            file: path,
            seconds: None,
        };
        (App::with_registry(options, registry()).unwrap(), dir)
    }

    #[test]
    fn save_playlist_writes_the_queue_in_the_format_the_extension_names() {
        let (mut app, dir) = app_with_file(None);
        let first = track(&dir, "one.mp3");
        let stream = "http://example.test/stream".to_owned();
        app.engine.set_queue(vec![
            TrackSource::path(first.clone()),
            TrackSource::Url(stream.clone()),
        ]);
        app.sync_entry_ids();

        for name in ["list.m3u", "list.M3U8", "list.pls", "list.xspf"] {
            let target = dir.path().join(name);
            app.submit(Command::SavePlaylist(target.clone())).unwrap();
            let report = rustamp_playlist::load_playlist_file_report(&target).unwrap();
            let locations: Vec<MediaLocation> = report
                .entries
                .iter()
                .map(|entry| entry.location.clone())
                .collect();
            assert_eq!(
                locations,
                vec![
                    MediaLocation::File(first.clone()),
                    MediaLocation::Url(stream.clone())
                ],
                "{name}"
            );
        }
        assert!(app.last_error.is_none());
    }

    #[test]
    fn save_playlist_refuses_an_unknown_or_unsupported_extension() {
        let (mut app, dir) = app_with_file(None);
        app.engine
            .set_queue(vec![TrackSource::path(track(&dir, "one.mp3"))]);
        app.sync_entry_ids();

        for name in ["list.txt", "list.b4s"] {
            let target = dir.path().join(name);
            assert!(app.save_playlist_file(&target).is_err(), "{name}");
            assert!(!target.exists(), "{name} must not be written");
        }
        assert!(app.last_error.is_some());
    }

    #[test]
    fn load_pls_then_save_with_no_extension_defaults_to_pls() {
        let (mut app, dir) = app_with_file(None);
        let track_path = track(&dir, "one.mp3");
        let pls_path = dir.path().join("list.pls");
        std::fs::write(
            &pls_path,
            format!("[playlist]\nFile1={}\nTitle1=One\n", track_path.display()),
        )
        .unwrap();

        app.load_playlist_file(&pls_path, false).unwrap();
        assert_eq!(
            app.last_playlist_format(),
            Some(rustamp_playlist::PlaylistFormat::Pls)
        );

        let target = dir.path().join("saved"); // no extension: default to loaded format
        app.save_playlist_file(&target).unwrap();
        let bytes = std::fs::read(&target).unwrap();
        // PLS content marker.
        assert!(
            std::str::from_utf8(&bytes)
                .unwrap()
                .starts_with("[playlist]"),
            "bytes: {:?}",
            bytes
        );
        assert!(app.last_error.is_none());
    }

    #[test]
    fn fresh_playlist_save_defaults_to_m3u() {
        let (mut app, dir) = app_with_file(None);
        let first = track(&dir, "one.mp3");
        app.engine.set_queue(vec![TrackSource::path(first.clone())]);
        app.sync_entry_ids();

        assert_eq!(app.last_playlist_format(), None);

        let target = dir.path().join("saved"); // no extension
        app.save_playlist_file(&target).unwrap();
        let bytes = std::fs::read(&target).unwrap();
        // M3U content.
        assert!(
            std::str::from_utf8(&bytes).unwrap().contains(".mp3")
                || bytes.starts_with(b"#EXTM3U")
                || !bytes.is_empty(),
            "bytes: {:?}",
            bytes
        );
        // The saved file is readable as M3U.
        let report = rustamp_playlist::load_playlist_file_report(&target).unwrap();
        assert_eq!(report.entries.len(), 1);
    }

    #[test]
    fn unknown_format_file_is_treated_as_m3u() {
        let (mut app, dir) = app_with_file(None);
        // A plain list of paths behind an unrecognized extension: detection
        // fails, but the content is a readable M3U list, so it is treated as M3U.
        let txt_path = dir.path().join("list.unknown");
        std::fs::write(&txt_path, b"track1.tst\n").unwrap();
        let t = track(&dir, "track1.tst");

        // load may succeed as M3U fallback
        let _ = app.load_playlist_file(&txt_path, false);
        // If it loaded, format is M3U.
        if app.last_error().is_none() {
            assert_eq!(
                app.last_playlist_format(),
                Some(rustamp_playlist::PlaylistFormat::M3u)
            );
            assert_eq!(app.engine().queue(), &[TrackSource::path(t)]);
        }
    }

    fn widget_centre(widget: Widget) -> (i32, i32) {
        let rect = widget.rect(&MainWindowLayout::CLASSIC);
        (
            rect.x + rect.width as i32 / 2,
            rect.y + rect.height as i32 / 2,
        )
    }

    fn pump_until(app: &mut App, mut cond: impl FnMut(&App) -> bool, max_frames: u64) -> bool {
        if cond(app) {
            return true;
        }
        let mut pumped = 0;
        while pumped < max_frames {
            app.pump().unwrap();
            if cond(app) {
                return true;
            }
            pumped += 1;
            std::thread::sleep(Duration::from_millis(1));
        }
        cond(app)
    }

    /// Like [`pump_until`], but also advances the virtual clock, so a run can
    /// actually drain and finish. The engine ends a run when the ring is
    /// drained, not when the device clock passes the end, so a wait that only
    /// pumps the app can never see the transport stop.
    fn pump_until_drained(
        app: &mut App,
        mut cond: impl FnMut(&App) -> bool,
        max_frames: u64,
    ) -> bool {
        if cond(app) {
            return true;
        }
        let mut pumped = 0;
        while pumped < max_frames {
            app.advance_null_clock(1024);
            app.pump().unwrap();
            if cond(app) {
                return true;
            }
            pumped += 1;
            std::thread::sleep(Duration::from_millis(1));
        }
        cond(app)
    }

    // ----- command routing: a UI outcome reaches the engine -----

    #[test]
    fn clicking_play_turns_the_ui_outcome_into_a_command_the_engine_obeys() {
        let dir = tempfile::tempdir().unwrap();
        let file = track(&dir, "track1_44100.tst");
        let (mut app, _dir) = app_with_file(Some(file));

        // The UI machine is what turns the click into commands.
        let (x, y) = widget_centre(Widget::Play);
        let commands = app.mouse_down(x, y).unwrap();
        assert!(
            commands.is_empty(),
            "a press alone fires nothing: {commands:?}"
        );
        let commands = app.mouse_up(x, y).unwrap();
        assert_eq!(commands, vec![Command::Play]);

        // The command reached the engine...
        assert_eq!(app.engine().transport(), TransportState::Playing);
        // ...and the engine's event came back into the UI state.
        assert!(
            pump_until(
                &mut app,
                |app| app.state().transport == TransportState::Playing,
                2000
            ),
            "the UI never saw Playing: {:?}",
            app.state()
        );
    }

    #[test]
    fn clicking_stop_returns_the_transport_to_stopped_in_the_ui() {
        let dir = tempfile::tempdir().unwrap();
        let file = track(&dir, "track1_44100.tst");
        let (mut app, _dir) = app_with_file(Some(file));
        app.submit(Command::Play).unwrap();
        assert!(pump_until(
            &mut app,
            |app| app.state().transport == TransportState::Playing,
            2000
        ));

        let (x, y) = widget_centre(Widget::Stop);
        app.mouse_down(x, y).unwrap();
        let commands = app.mouse_up(x, y).unwrap();
        assert_eq!(commands, vec![Command::Stop]);
        assert_eq!(app.engine().transport(), TransportState::Stopped);
        assert_eq!(app.state().transport, TransportState::Stopped);
        assert!(
            !app.state().time_visible,
            "stopped blanks the time (S02 §4)"
        );
    }

    #[test]
    fn a_slider_drag_routes_volume_to_the_engine_and_back() {
        let (mut app, _dir) = app_with_file(None);
        let track_rect = MainWindowLayout::CLASSIC.volume_track;
        let y = track_rect.y + 6;

        app.mouse_down(track_rect.x, y).unwrap();
        let commands = app.mouse_move(track_rect.right() - 1, y).unwrap();
        assert!(
            matches!(commands.last(), Some(Command::SetVolume(_))),
            "{commands:?}"
        );
        app.mouse_up(track_rect.right() - 1, y).unwrap();

        // The engine published the new volume; the UI shows it as a percentage.
        assert_eq!(app.state().volume, 100);
        assert_eq!(app.engine().state().volume, 255);
    }

    #[test]
    fn keyboard_volume_up_reaches_the_engine() {
        let (mut app, _dir) = app_with_file(None);
        let before = app.engine().state().volume;
        let shown_before = app.state().volume;
        let outcome = app.key_down(Key::Up, Modifiers::NONE).unwrap();
        assert!(
            matches!(outcome, Some(ShortcutOutcome::Commands(_))),
            "{outcome:?}"
        );
        // The engine's volume is the core 0..=255 scale; the UI shows a
        // percentage, and the two round-trip through `core_volume` /
        // `volume_percent`, so the step is a little under `VOLUME_STEP`.
        let after = app.engine().state().volume;
        assert!(after > before, "volume did not rise: {before} -> {after}");
        assert!(
            app.state().volume > shown_before,
            "the UI did not follow the engine: {shown_before} -> {}",
            app.state().volume
        );
    }

    #[test]
    fn a_quit_command_is_owned_by_the_app() {
        let (mut app, _dir) = app_with_file(None);
        assert!(!app.quit());
        app.submit(Command::Quit).unwrap();
        assert!(app.quit());
    }

    #[test]
    fn toggling_double_size_changes_the_client_size() {
        let (mut app, _dir) = app_with_file(None);
        assert_eq!(
            app.frame_size(),
            (rustamp_ui::WINDOW_WIDTH, rustamp_ui::WINDOW_HEIGHT)
        );
        app.submit(Command::ToggleDoubleSize).unwrap();
        assert!(app.state().double_size);
        assert_eq!(
            app.frame_size(),
            (rustamp_ui::DOUBLE_WIDTH, rustamp_ui::DOUBLE_HEIGHT)
        );
    }

    #[test]
    fn always_on_top_is_an_app_level_toggle() {
        let (mut app, _dir) = app_with_file(None);
        assert!(!app.always_on_top());
        app.submit(Command::ToggleAlwaysOnTop).unwrap();
        assert!(app.always_on_top());
        app.submit(Command::ToggleAlwaysOnTop).unwrap();
        assert!(!app.always_on_top());
    }

    #[test]
    fn a_window_that_does_not_exist_is_noted_not_an_error() {
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::ToggleWindow(WindowId::Equalizer))
            .unwrap();
        let notes = app.drain_notes();
        assert!(notes.iter().any(|n| n.contains("Equalizer")), "{notes:?}");
        assert!(!app.quit(), "an unimplemented window must not quit the app");
    }

    #[test]
    fn advancing_the_null_clock_moves_playback_without_a_device() {
        // The deterministic half of the silent windowed mode (`--no-audio`):
        // driving the virtual clock pulls decoded frames through the engine
        // with no hardware and no window involved.
        let dir = tempfile::tempdir().unwrap();
        let file = track(&dir, "track1_44100.tst");
        let (mut app, _dir) = app_with_file(Some(file));
        app.submit(Command::Play).unwrap();
        let mut moved = false;
        for _ in 0..2000 {
            app.advance_null_clock(1024);
            app.pump().unwrap();
            if app.engine().timeline_frames() > 0 {
                moved = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(
            moved,
            "driving the null clock never moved the timeline (transport {:?})",
            app.engine().transport()
        );
        assert!(
            app.engine().last_error().is_none(),
            "the silent run errored: {:?}",
            app.engine().last_error()
        );
    }

    // ----- event routing: events update the UI state -----

    #[test]
    fn engine_events_update_the_ui_state() {
        let (mut app, _dir) = app_with_file(None);
        let now = app.now_ms();
        app.handle_event(&Event::StateChanged(TransportState::Playing));
        app.handle_event(&Event::TrackChanged {
            id: Some(EntryId(1)),
            index: Some(0),
            metadata: Some(Box::new(TrackMetadata {
                title: Some("Hello".to_string()),
                ..TrackMetadata::default()
            })),
            duration_ms: Some(200_000),
        });
        app.handle_event(&Event::StreamInfoChanged(Some(StreamInfo {
            bitrate_kbps: Some(192),
            sample_rate_hz: 44_100,
            channels: 2,
        })));
        app.handle_event(&Event::PositionChanged {
            position_ms: 83_000,
            duration_ms: Some(200_000),
        });
        app.ui_mut().refresh(now);

        let state = app.state();
        assert_eq!(state.transport, TransportState::Playing);
        assert_eq!(state.kbps, Some(192));
        assert_eq!(state.khz, Some(44));
        assert_eq!(state.title, "1. Hello (3:20)");
        assert_eq!(state.time_cells(), [None, Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn a_player_state_snapshot_seeds_the_ui() {
        let (mut app, _dir) = app_with_file(None);
        let snapshot = PlayerState {
            transport: TransportState::Paused,
            position_ms: 30_000,
            duration_ms: Some(120_000),
            volume: 128,
            balance: -127,
            shuffle: true,
            repeat: true,
            current_index: Some(1),
            current_id: Some(EntryId(7)),
            playlist_len: 3,
            stream: Some(StreamInfo {
                bitrate_kbps: Some(96),
                sample_rate_hz: 48_000,
                channels: 1,
            }),
            eq: rustamp_core::state::EqState::default(),
            stop_after_current: false,
        };
        app.sync_from_player_state(&snapshot);

        let state = app.state();
        assert_eq!(state.transport, TransportState::Paused);
        assert_eq!(state.volume, volume_percent(128));
        assert_eq!(state.balance, rustamp_ui::balance_percent(-127));
        assert!(state.shuffle && state.repeat);
        assert_eq!(state.kbps, Some(96));
        assert_eq!(state.khz, Some(48));
        assert_eq!(state.channels, rustamp_ui::MonoStereo::Mono);
        assert!(
            app.ui().logical_title().starts_with("2. "),
            "{}",
            app.ui().logical_title()
        );
    }

    #[test]
    fn shuffle_and_repeat_are_app_owned_and_published_as_events() {
        let (mut app, _dir) = app_with_file(None);
        let (x, y) = widget_centre(Widget::Shuffle);
        app.mouse_down(x, y).unwrap();
        let commands = app.mouse_up(x, y).unwrap();
        assert_eq!(commands, vec![Command::ToggleShuffle]);
        assert!(app.state().shuffle);
        assert!(app.shuffle);

        let (x, y) = widget_centre(Widget::Repeat);
        app.mouse_down(x, y).unwrap();
        app.mouse_up(x, y).unwrap();
        assert!(app.state().repeat);
        assert!(app.repeat);
    }

    // ----- the Unhandled shortcut path -----

    #[test]
    fn an_unhandled_shortcut_carries_its_command_id() {
        let (mut app, _dir) = app_with_file(None);
        // Ctrl+Alt+N is "start a new instance" (S03 §1), which is app-local and
        // deliberately has no `Command` (RFC 0005 §10).
        let outcome = app
            .key_down(Key::Letter(b'N'), Modifiers::CTRL.union(Modifiers::ALT))
            .unwrap();
        let outcome = outcome.expect("Ctrl+Alt+N is bound");
        let note = App::unhandled_note(&outcome).expect("the outcome is Unhandled");
        assert!(note.contains("40305"), "{note}");
        assert!(note.contains("NewInstance"), "{note}");
        assert!(!app.quit());
    }

    #[test]
    fn a_bound_shortcut_is_not_unhandled() {
        let (mut app, _dir) = app_with_file(None);
        let outcome = app.key_down(Key::Letter(b'X'), Modifiers::NONE).unwrap();
        assert!(matches!(outcome, Some(ShortcutOutcome::Commands(_))));
        assert_eq!(App::unhandled_note(&outcome.unwrap()), None);
    }

    // ----- the app applies a full command/event round trip -----

    #[test]
    fn next_and_prev_move_through_the_queue_and_report_back() {
        let dir = tempfile::tempdir().unwrap();
        let a = track(&dir, "track1_44100.tst");
        let b = track(&dir, "track2_44100.tst");
        let (mut app, _dir) = app_with_file(Some(a));
        // Append a second entry through the app's own routing (`AddItems` is
        // app-owned; the engine only holds the queue).
        app.submit(Command::AddItems {
            items: vec![MediaLocation::File(b)],
            at: None,
            play: false,
        })
        .unwrap();
        assert_eq!(app.engine().queue().len(), 2);

        app.submit(Command::Play).unwrap();
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(0),
            4000
        ));

        app.submit(Command::Next).unwrap();
        assert!(
            pump_until(
                &mut app,
                |app| app.engine().current_index() == Some(1),
                4000
            ),
            "Next did not advance"
        );
        assert!(
            app.state().title.starts_with("2. "),
            "{}",
            app.state().title
        );
    }

    #[test]
    fn rendering_produces_a_frame_of_the_layout_size() {
        let (mut app, _dir) = app_with_file(None);
        let image = app.render();
        assert_eq!(
            (image.width(), image.height()),
            (rustamp_ui::WINDOW_WIDTH, rustamp_ui::WINDOW_HEIGHT)
        );
        assert!(
            image
                .as_rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] != 0),
            "the frame is empty"
        );
    }

    #[test]
    fn quit_is_published_on_the_bus() {
        let (mut app, _dir) = app_with_file(None);
        let observer = app.bus.subscribe();
        app.submit(Command::Quit).unwrap();
        let events: Vec<Event> = observer.try_iter().collect();
        assert!(events.contains(&Event::Quit), "{events:?}");
    }

    #[test]
    fn playlist_dialog_requests_carry_the_spec_titles() {
        // S06 §4: the spec fixes the dialog titles and no dialog behaviour
        // beyond that. Both ADD and LIST OPTS now open a real dialog
        // (`dialogs.rs`), so neither leaves a note.
        let (mut app, _dir) = app_with_file(None);
        assert_eq!(
            app.request_playlist_dialog(PlaylistDialog::AddFiles),
            "Add file(s) to playlist"
        );
        assert_eq!(
            app.request_playlist_dialog(PlaylistDialog::LoadPlaylist),
            "Load Playlist"
        );
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::LoadPlaylist));
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn open_playlist_is_an_app_owned_command_that_opens_its_dialog() {
        // RFC 0010: LIST OPTS opens the `Load Playlist` dialog (S06 §4). The
        // spec fixes the title only; this command opens the real window and
        // records no note of its own — the dialog's load behaviour is pinned
        // by the load-playlist tests below.
        let (mut app, _dir) = app_with_file(None);
        assert!(crate::routing::app_owns(&Command::OpenPlaylist));
        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::LoadPlaylist));
        assert!(app.drain_notes().is_empty());
        assert!(!app.quit(), "a dialog request must not quit the app");
    }

    #[test]
    fn a_managed_but_missing_host_keeps_pumping_detached() {
        use crate::supervise::Supervisor;

        let (mut app, _dir) = app_with_file(None);
        // A real backend pointed at nothing that exists: resolving it fails
        // without spawning any process, which is exactly the degraded path.
        app.manage_host(Supervisor::with_cooldown(
            crate::supervise::ProcessBackend::with_path(PathBuf::from(
                "no-such-dir/rustamp-plughost32",
            )),
            Duration::ZERO,
        ));
        app.pump().unwrap();
        assert!(!app.host_attached());
        // Repeated pumps stay detached instead of stalling or failing.
        app.pump().unwrap();
        assert!(!app.host_attached());
        app.stop_host();
        assert!(!app.host_attached());
    }

    // ----- the equalizer window (S07; headless, no window opened) -----

    #[test]
    fn rendering_the_equalizer_produces_a_frame_of_the_layout_size() {
        let (app, _dir) = app_with_file(None);
        let image = app.render_eq();
        assert_eq!(
            (image.width(), image.height()),
            (
                rustamp_ui::EQUALIZER_WINDOW_SIZE.0,
                rustamp_ui::EQUALIZER_WINDOW_SIZE.1
            )
        );
        assert_eq!((image.width(), image.height()), app.eq_frame_size());
        assert!(
            image
                .as_rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|pixel| pixel[3] != 0),
            "the frame is empty"
        );
    }

    #[test]
    fn clicking_on_in_the_eq_window_reaches_the_engine_stage() {
        let (mut app, _dir) = app_with_file(None);
        assert!(app.eq_state().enabled);
        // On held rect (14,18,25,12): press arms, release inside toggles.
        assert!(app.eq_mouse_down(20, 24).unwrap().is_empty());
        let commands = app.eq_mouse_up(20, 24).unwrap();
        assert_eq!(commands, vec![Command::SetEqEnabled(false)]);
        assert!(!app.eq_state().enabled);
        // The engine owns the S07 stage now, so the same commands are
        // forwarded and change the engine's published equalizer state instead
        // of being noted as unimplemented.
        app.submit(Command::SetEqEnabled(true)).unwrap();
        app.submit(Command::SetBand {
            band: 4,
            gain_db: 6.0,
        })
        .unwrap();
        let eq = app.host_player_state().eq;
        assert!(eq.enabled);
        assert_eq!(eq.bands[4], 6.0);
        assert!(app.eq_state().enabled);
        assert_eq!(
            app.eq_state().bands[4],
            rustamp_core::state::eq_classic_from_db(6.0) as u8
        );
        let notes = app.drain_notes();
        assert!(!notes.iter().any(|n| n.contains("equalizer")), "{notes:?}");
        assert!(!app.quit());
    }

    #[test]
    fn eq_command_applied_from_outside_updates_window_state() {
        let (mut app, _dir) = app_with_file(None);

        // SetEqEnabled external command updates window state.
        app.submit(Command::SetEqEnabled(false)).unwrap();
        assert!(!app.eq_state().enabled);
        assert!(!app.engine().eq().enabled);

        app.submit(Command::SetEqEnabled(true)).unwrap();
        assert!(app.eq_state().enabled);
        assert!(app.engine().eq().enabled);

        // SetEqAuto external command updates window state.
        app.submit(Command::SetEqAuto(true)).unwrap();
        assert!(app.eq_state().auto);
        assert!(app.engine().eq().auto);

        app.submit(Command::SetEqAuto(false)).unwrap();
        assert!(!app.eq_state().auto);
        assert!(!app.engine().eq().auto);

        // SetPreamp external command updates window state.
        app.submit(Command::SetPreamp(4.5)).unwrap();
        assert_eq!(
            app.eq_state().preamp,
            rustamp_core::state::eq_classic_from_db(4.5) as u8
        );
        assert_eq!(app.engine().eq().preamp, 4.5);

        // SetBand external command updates window state.
        app.submit(Command::SetBand {
            band: 2,
            gain_db: -8.0,
        })
        .unwrap();
        assert_eq!(
            app.eq_state().bands[2],
            rustamp_core::state::eq_classic_from_db(-8.0) as u8
        );
        assert_eq!(app.engine().eq().bands[2], -8.0);

        // Direct external apply() also synchronizes the window.
        app.apply(&Command::SetBand {
            band: 5,
            gain_db: 3.5,
        });
        assert_eq!(
            app.eq_state().bands[5],
            rustamp_core::state::eq_classic_from_db(3.5) as u8
        );
        assert_eq!(app.engine().eq().bands[5], 3.5);

        // Event::EqChanged also updates window state.
        let mut custom_bands = [0.0; 10];
        custom_bands[8] = -10.0;
        let event_eq = rustamp_core::state::EqState {
            bands: custom_bands,
            preamp: 2.0,
            enabled: false,
            auto: true,
        };
        app.handle_event(&Event::EqChanged(event_eq));
        assert_eq!(
            app.eq_state().bands[8],
            rustamp_core::state::eq_classic_from_db(-10.0) as u8
        );
        assert_eq!(
            app.eq_state().preamp,
            rustamp_core::state::eq_classic_from_db(2.0) as u8
        );
        assert!(!app.eq_state().enabled);
        assert!(app.eq_state().auto);
    }

    #[test]
    fn clicking_on_button_toggles_engine_and_window_state_together() {
        let (mut app, _dir) = app_with_file(None);
        assert!(app.eq_state().enabled);

        // Click ON: toggles to false, both engine and window agree.
        assert!(app.eq_mouse_down(20, 24).unwrap().is_empty());
        let cmds = app.eq_mouse_up(20, 24).unwrap();
        assert_eq!(cmds, vec![Command::SetEqEnabled(false)]);
        assert!(!app.eq_state().enabled);
        assert!(!app.engine().eq().enabled);
        assert!(!app.host_player_state().eq.enabled);

        // Click ON again: toggles to true, both engine and window agree.
        assert!(app.eq_mouse_down(20, 24).unwrap().is_empty());
        let cmds = app.eq_mouse_up(20, 24).unwrap();
        assert_eq!(cmds, vec![Command::SetEqEnabled(true)]);
        assert!(app.eq_state().enabled);
        assert!(app.engine().eq().enabled);
        assert!(app.host_player_state().eq.enabled);

        // Third click: toggles back to false.
        assert!(app.eq_mouse_down(20, 24).unwrap().is_empty());
        let cmds = app.eq_mouse_up(20, 24).unwrap();
        assert_eq!(cmds, vec![Command::SetEqEnabled(false)]);
        assert!(!app.eq_state().enabled);
        assert!(!app.engine().eq().enabled);
        assert!(!app.host_player_state().eq.enabled);
    }

    #[test]
    fn states_are_equal_after_band_and_preamp_changes() {
        let (mut app, _dir) = app_with_file(None);

        // 1. External band change.
        app.submit(Command::SetBand {
            band: 1,
            gain_db: 7.2,
        })
        .unwrap();
        let eng = app.engine().eq();
        let win = app.eq_state();
        assert_eq!(
            win.bands[1],
            rustamp_core::state::eq_classic_from_db(eng.bands[1]) as u8
        );

        // 2. UI-driven band change (click on band 6 track).
        let band6_x = rustamp_ui::EqualizerLayout::slider_x(rustamp_ui::EqSlider::Band(6));
        let cmds = app.eq_mouse_down(band6_x + 5, 45).unwrap();
        assert_eq!(cmds.len(), 1);
        let eng = app.engine().eq();
        let win = app.eq_state();
        assert_eq!(
            win.bands[6],
            rustamp_core::state::eq_classic_from_db(eng.bands[6]) as u8
        );
        let _ = app.eq_mouse_up(band6_x + 5, 45).unwrap();

        // 3. External preamp change.
        app.submit(Command::SetPreamp(-5.5)).unwrap();
        let eng = app.engine().eq();
        let win = app.eq_state();
        assert_eq!(
            win.preamp,
            rustamp_core::state::eq_classic_from_db(eng.preamp) as u8
        );

        // 4. UI-driven preamp change (click on the preamp track, away from the
        //    thumb). The thumb now sits wherever the synchronised value put it,
        //    and a press *inside* the thumb starts a drag and emits nothing, so
        //    the press must land outside it.
        let preamp_x = rustamp_ui::EqualizerLayout::slider_x(rustamp_ui::EqSlider::Preamp);
        let before = app.eq_state().preamp;
        let cmds = app.eq_mouse_down(preamp_x + 5, 45).unwrap();
        assert_eq!(cmds.len(), 1, "a press outside the thumb commits");
        assert_ne!(
            app.eq_state().preamp,
            before,
            "the press must move the thumb, not land on it"
        );
        let eng = app.engine().eq();
        let win = app.eq_state();
        assert_eq!(
            win.preamp,
            rustamp_core::state::eq_classic_from_db(eng.preamp) as u8
        );
        let _ = app.eq_mouse_up(preamp_x + 5, 45).unwrap();
    }

    #[test]
    fn presets_in_the_eq_window_queues_the_popup_request_once() {
        // S07 §4.2: the click queues the Load/Save/Delete popup, which the
        // presenter takes on its tick and shows.
        let (mut app, _dir) = app_with_file(None);
        assert!(!app.take_eq_menu());
        assert!(app.eq_mouse_down(239, 24).unwrap().is_empty());
        assert!(app.eq_mouse_up(239, 24).unwrap().is_empty());
        assert!(app.take_eq_menu());
        assert!(!app.take_eq_menu());
        assert_eq!(
            app.request_eq_presets_menu(rustamp_ui::EqMenuRequest::Presets),
            &["Load", "Save", "Delete"]
        );
    }

    #[test]
    fn applying_a_preset_sets_the_bands_and_is_recognised_again() {
        let (mut app, _dir) = app_with_file(None);
        assert!(!app.apply_eq_preset("No Such Preset").unwrap());
        assert!(app.apply_eq_preset("rock").unwrap());
        let rock = crate::eq_presets::find_preset("Rock").unwrap();
        assert_eq!(app.engine.eq().bands, rock.bands);
        assert_eq!(app.current_eq_preset(), Some("Rock"));
        // The window's sliders follow the engine.
        assert!(app.apply_eq_preset("Flat").unwrap());
        assert_eq!(app.current_eq_preset(), Some("Flat"));
    }

    #[test]
    fn saved_presets_load_save_and_delete_through_the_popup_and_survive_a_restart() {
        use crate::eq_presets::EqMenuChoice;
        let (mut app, dir) = app_with_file(None);
        let config = dir.path().join("config").join("rustamp.toml");
        app.set_config_path(Some(config.clone()));
        assert!(app.eq_user_presets().is_empty());

        // Save the live curve under a fresh name.
        app.submit(Command::SetBand {
            band: 0,
            gain_db: 6.0,
        })
        .unwrap();
        app.submit(Command::SetPreamp(-3.0)).unwrap();
        app.activate_eq_menu_choice(EqMenuChoice::SaveNew).unwrap();
        assert_eq!(app.eq_user_presets().len(), 1);
        assert_eq!(app.eq_user_presets()[0].name, "Custom 1");
        assert!(config.with_file_name("presets.eqf").is_file());

        // Flatten, then load the saved curve back.
        app.apply_eq_preset("Flat").unwrap();
        app.submit(Command::SetPreamp(0.0)).unwrap();
        app.activate_eq_menu_choice(EqMenuChoice::LoadUser(0))
            .unwrap();
        let eq = app.engine().eq();
        assert!((eq.bands[0] - 6.0).abs() < 0.3, "{:?}", eq.bands);
        assert!((eq.preamp + 3.0).abs() < 0.3, "{}", eq.preamp);

        // A new run reads the file; the popup lists the saved preset.
        let (mut again, _dir2) = app_with_file(None);
        again.set_config_path(Some(config));
        assert_eq!(again.eq_user_presets().len(), 1);
        assert_eq!(again.eq_user_presets()[0].name, "Custom 1");
        let menu = again.native_eq_menu();
        assert!(menu.choice_for_id(0).is_none());
        let ids: Vec<_> = (1..=40).filter_map(|id| menu.choice_for_id(id)).collect();
        assert!(ids.contains(&EqMenuChoice::LoadUser(0)));
        assert!(ids.contains(&EqMenuChoice::SaveOver(0)));
        assert!(ids.contains(&EqMenuChoice::Delete(0)));

        again
            .activate_eq_menu_choice(EqMenuChoice::Delete(0))
            .unwrap();
        assert!(again.eq_user_presets().is_empty());
        again.load_eq_user_presets();
        assert!(again.eq_user_presets().is_empty());
    }

    #[test]
    fn save_over_replaces_in_place_and_builtins_keep_the_preamp() {
        use crate::eq_presets::EqMenuChoice;
        let (mut app, _dir) = app_with_file(None);
        app.activate_eq_menu_choice(EqMenuChoice::SaveNew).unwrap();
        app.submit(Command::SetBand {
            band: 2,
            gain_db: 6.0,
        })
        .unwrap();
        app.activate_eq_menu_choice(EqMenuChoice::SaveOver(0))
            .unwrap();
        assert_eq!(app.eq_user_presets().len(), 1);
        assert!((app.eq_user_presets()[0].bands[2] - 6.0).abs() < 0.3);
        // Out-of-range indices are ignored.
        app.activate_eq_menu_choice(EqMenuChoice::SaveOver(9))
            .unwrap();
        app.activate_eq_menu_choice(EqMenuChoice::LoadUser(9))
            .unwrap();
        app.activate_eq_menu_choice(EqMenuChoice::Delete(9))
            .unwrap();
        assert_eq!(app.eq_user_presets().len(), 1);
        // A built-in preset leaves the preamp where it was.
        app.submit(Command::SetPreamp(-4.0)).unwrap();
        app.activate_eq_menu_choice(EqMenuChoice::LoadBuiltin(1))
            .unwrap();
        assert_eq!(app.engine().eq().preamp, -4.0);
    }

    #[test]
    fn an_eqf_file_loads_with_its_preamp_and_bad_files_are_refused() {
        use crate::eq_presets::{UserPreset, write_eqf};
        let (mut app, dir) = app_with_file(None);
        let good = dir.path().join("good.eqf");
        let mut bands = [0.0; rustamp_core::state::EQ_BANDS];
        bands[0] = 6.0;
        let preset = UserPreset {
            name: "Warm".to_string(),
            bands,
            preamp: -3.0,
        };
        std::fs::write(&good, write_eqf(&[preset])).unwrap();
        assert_eq!(app.load_eq_file(&good).unwrap(), "Warm");
        let eq = app.engine().eq();
        assert!((eq.bands[0] - 6.0).abs() < 0.3);
        assert!((eq.preamp + 3.0).abs() < 0.3);
        assert!(app.eq_user_presets().is_empty());

        let junk = dir.path().join("junk.eqf");
        std::fs::write(&junk, b"not an eqf").unwrap();
        assert!(app.load_eq_file(&junk).is_err());
        let empty = dir.path().join("empty.eqf");
        std::fs::write(&empty, write_eqf(&[])).unwrap();
        assert!(app.load_eq_file(&empty).is_err());
        assert!(app.load_eq_file(&dir.path().join("missing.eqf")).is_err());
        // Refused files leave the curve alone.
        assert!((app.engine().eq().bands[0] - 6.0).abs() < 0.3);
    }

    #[test]
    fn jump_to_file_filters_the_playlist_and_plays_the_selected_match() {
        use rustamp_core::command::JumpTarget;
        let (mut app, dir) = app_with_file(None);
        app.engine.set_queue(vec![
            TrackSource::path(track(&dir, "alpha.mp3")),
            TrackSource::path(track(&dir, "beta.mp3")),
            TrackSource::path(track(&dir, "Gamma Beta.mp3")),
        ]);
        app.sync_entry_ids();
        let jump = rustamp_ui::DialogKind::Jump(JumpTarget::File);
        app.submit(Command::OpenJump {
            target: JumpTarget::File,
        })
        .unwrap();
        assert!(app.dialog_is_open(jump));
        assert_eq!(app.jump_file_matches(), vec![0, 1, 2]);

        // Typing narrows the list (ignoring case); Down moves, Enter plays.
        assert!(app.dialog_text("BETA"));
        assert_eq!(app.jump_file_matches(), vec![1, 2]);
        assert!(app.dialog_key(Key::Down));
        assert!(app.dialog_key(Key::Down), "clamps at the last match");
        assert!(app.dialog_key(Key::Up));
        assert!(app.dialog_key(Key::Down));
        assert!(app.dialog_key(Key::Enter));
        assert!(!app.dialog_is_open(jump));
        assert_eq!(app.engine().current_index(), Some(2));
    }

    #[test]
    fn jump_to_file_with_no_match_stays_open() {
        use rustamp_core::command::JumpTarget;
        let (mut app, dir) = app_with_file(None);
        app.engine
            .set_queue(vec![TrackSource::path(track(&dir, "alpha.mp3"))]);
        app.sync_entry_ids();
        app.submit(Command::OpenJump {
            target: JumpTarget::File,
        })
        .unwrap();
        assert!(app.dialog_text("zzz"));
        assert!(app.jump_file_matches().is_empty());
        assert!(app.dialog_key(Key::Enter));
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::Jump(JumpTarget::File)));
        assert!(app.dialog_key(Key::Backspace));
        assert!(app.dialog_key(Key::Escape));
        assert!(app.dialog_open_kind().is_none());
    }

    #[test]
    fn jump_to_file_lists_the_matches_and_counts_them() {
        use rustamp_core::command::JumpTarget;
        let (mut app, dir) = app_with_file(None);
        app.engine.set_queue(vec![
            TrackSource::path(track(&dir, "alpha.mp3")),
            TrackSource::path(track(&dir, "beta.mp3")),
            TrackSource::path(track(&dir, "Gamma Beta.mp3")),
        ]);
        app.sync_entry_ids();
        app.submit(Command::OpenJump {
            target: JumpTarget::File,
        })
        .unwrap();

        // The search field, one row per match and the count line. The first
        // match is the selected one, so it carries the marker.
        let lines = app.open_dialog_lines().unwrap();
        assert_eq!(lines.first().map(String::as_str), Some("Search: "));
        assert_eq!(lines.len(), 1 + 3 + 1, "{lines:?}");
        assert!(lines[1].starts_with("> 1. "), "{lines:?}");
        assert!(lines[1].ends_with("alpha.mp3"), "{lines:?}");
        assert!(lines[2].starts_with("  2. "), "{lines:?}");
        assert!(lines[2].ends_with("beta.mp3"), "{lines:?}");
        assert!(lines[3].starts_with("  3. "), "{lines:?}");
        assert!(lines[3].ends_with("Gamma Beta.mp3"), "{lines:?}");
        assert_eq!(lines.last().map(String::as_str), Some("3 of 3 entries"));

        // A search narrows the list and the count.
        assert!(app.dialog_text("BETA"));
        let lines = app.open_dialog_lines().unwrap();
        assert_eq!(lines.first().map(String::as_str), Some("Search: BETA"));
        assert!(lines[1].starts_with("> 2. "), "{lines:?}");
        assert!(lines[1].ends_with("beta.mp3"), "{lines:?}");
        assert!(lines[2].starts_with("  3. "), "{lines:?}");
        assert_eq!(lines.last().map(String::as_str), Some("2 of 3 entries"));
    }

    #[test]
    fn jump_to_file_keeps_the_selection_in_view_and_starts_over_next_time() {
        use rustamp_core::command::JumpTarget;
        let (mut app, dir) = app_with_file(None);
        let queue: Vec<_> = (0..15)
            .map(|index| TrackSource::path(track(&dir, &format!("track{index:02}.mp3"))))
            .collect();
        app.engine.set_queue(queue);
        app.sync_entry_ids();
        app.submit(Command::OpenJump {
            target: JumpTarget::File,
        })
        .unwrap();

        // The window shows ten rows, the selected one marked.
        let lines = app.open_dialog_lines().unwrap();
        assert_eq!(lines.len(), 1 + rustamp_ui::JUMP_FILE_ROWS + 1, "{lines:?}");
        assert!(lines[1].starts_with("> 1. "), "{lines:?}");

        // Moving down past the window scrolls it: the selected row stays
        // visible and stays the marked one.
        for _ in 0..12 {
            assert!(app.dialog_key(Key::Down));
        }
        let lines = app.open_dialog_lines().unwrap();
        let marked: Vec<&String> = lines.iter().filter(|line| line.starts_with("> ")).collect();
        assert_eq!(marked.len(), 1, "{lines:?}");
        assert!(marked[0].starts_with("> 13. "), "{lines:?}");

        // Closing and opening the box again starts over: no search text and
        // the first row selected.
        assert!(app.dialog_key(Key::Escape));
        app.submit(Command::OpenJump {
            target: JumpTarget::File,
        })
        .unwrap();
        assert_eq!(app.jump_text(), "");
        let lines = app.open_dialog_lines().unwrap();
        assert!(lines[1].starts_with("> 1. "), "{lines:?}");
    }

    #[test]
    fn the_jump_to_file_shortcuts_open_the_box() {
        use rustamp_core::command::JumpTarget;
        let (mut app, _dir) = app_with_file(None);
        let jump = rustamp_ui::DialogKind::Jump(JumpTarget::File);

        // S03 §1/§2: `J` and `Num.` both carry the command id 40194, which
        // opens the jump-to-file box.
        for key in [Key::Letter(b'J'), Key::NumDecimal] {
            assert!(app.dialog_open_kind().is_none());
            let outcome = app.key_down(key, Modifiers::NONE).unwrap();
            assert!(
                matches!(
                    outcome,
                    Some(rustamp_ui::ShortcutOutcome::Commands(ref commands))
                        if commands == &vec![Command::OpenJump { target: JumpTarget::File }]
                ),
                "{key:?} -> {outcome:?}"
            );
            assert!(app.dialog_is_open(jump), "{key:?}");

            // Escape closes it again, so the next key starts from scratch.
            assert!(app.dialog_key(Key::Escape));
            assert!(app.dialog_open_kind().is_none());
        }
    }

    #[test]
    fn double_size_reaches_the_equalizer_window() {
        // S05 §2.1: the flag resizes the equalizer to 550 x 232.
        let (mut app, _dir) = app_with_file(None);
        assert_eq!(app.eq_frame_size(), (275, 116));
        app.submit(Command::ToggleDoubleSize).unwrap();
        assert_eq!(app.eq_frame_size(), (550, 232));
        assert_eq!(
            app.eq_frame_size(),
            (
                rustamp_ui::DOUBLE_EQUALIZER_SIZE.0,
                rustamp_ui::DOUBLE_EQUALIZER_SIZE.1
            )
        );
        app.submit(Command::ToggleDoubleSize).unwrap();
        assert_eq!(app.eq_frame_size(), (275, 116));
    }

    #[test]
    fn toggling_the_equalizer_window_is_noted_and_flips_the_flag() {
        let (mut app, _dir) = app_with_file(None);
        assert!(!app.eq_open());
        app.submit(Command::ToggleWindow(WindowId::Equalizer))
            .unwrap();
        assert!(app.eq_open());
        let notes = app.drain_notes();
        assert!(notes.iter().any(|n| n.contains("Equalizer")), "{notes:?}");
        assert!(!app.quit(), "toggling the EQ window must not quit the app");
    }

    // ----- window focus (`FocusNextWindow`, `ShowMainWindow`) -----

    /// Opens or closes the equalizer and the playlist editor to the wanted
    /// state (the playlist starts open, the equalizer closed), then clears the
    /// requests and notes the toggles left behind.
    fn set_open(app: &mut App, eq: bool, playlist: bool) {
        if app.eq_open() != eq {
            app.submit(Command::ToggleWindow(WindowId::Equalizer))
                .unwrap();
        }
        if app.playlist_open() != playlist {
            app.submit(Command::ToggleWindow(WindowId::Playlist))
                .unwrap();
        }
        app.take_focus_request();
        app.drain_notes();
    }

    /// Submits `FocusNextWindow` and returns the window the model now focuses,
    /// asserting that the pending request names the same window.
    fn focus_next(app: &mut App) -> WindowId {
        app.submit(Command::FocusNextWindow).unwrap();
        let focused = app.focused_window();
        assert_eq!(app.take_focus_request(), Some(focused));
        focused
    }

    #[test]
    fn focus_next_window_visits_the_open_windows_in_the_stated_order_and_wraps() {
        // The order is an own choice (see `focus.rs`), not a measurement.
        let (mut app, _dir) = app_with_file(None);
        set_open(&mut app, true, true);
        assert_eq!(app.focused_window(), WindowId::Main);
        let visited: Vec<_> = (0..6).map(|_| focus_next(&mut app)).collect();
        assert_eq!(
            visited,
            [
                WindowId::Equalizer,
                WindowId::Playlist,
                WindowId::Main,
                WindowId::Equalizer,
                WindowId::Playlist,
                WindowId::Main,
            ]
        );
        assert!(app.drain_notes().is_empty(), "the command is implemented");
    }

    #[test]
    fn focus_next_window_skips_a_closed_window() {
        let (mut app, _dir) = app_with_file(None);
        set_open(&mut app, false, true);
        assert_eq!(focus_next(&mut app), WindowId::Playlist);
        assert_eq!(focus_next(&mut app), WindowId::Main);
        set_open(&mut app, true, false);
        assert_eq!(focus_next(&mut app), WindowId::Equalizer);
        assert_eq!(focus_next(&mut app), WindowId::Main);
        // Only the main window left: the focus stays on it.
        set_open(&mut app, false, false);
        assert_eq!(focus_next(&mut app), WindowId::Main);
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn focus_requests_are_consumed_once() {
        let (mut app, _dir) = app_with_file(None);
        set_open(&mut app, true, true);
        app.submit(Command::FocusNextWindow).unwrap();
        assert_eq!(app.take_focus_request(), Some(WindowId::Equalizer));
        assert_eq!(app.take_focus_request(), None, "taken, not repeated");
        // The model keeps the focus after the request is gone.
        assert_eq!(app.focused_window(), WindowId::Equalizer);
        // An unrelated command neither re-raises the request nor moves focus.
        app.submit(Command::ToggleShuffle).unwrap();
        assert_eq!(app.take_focus_request(), None);
        assert_eq!(app.focused_window(), WindowId::Equalizer);
    }

    #[test]
    fn closing_the_focused_window_falls_back_to_main() {
        let (mut app, _dir) = app_with_file(None);
        set_open(&mut app, true, true);
        assert_eq!(focus_next(&mut app), WindowId::Equalizer);
        app.submit(Command::ToggleWindow(WindowId::Equalizer))
            .unwrap();
        assert_eq!(app.focused_window(), WindowId::Main);
        assert_eq!(app.take_focus_request(), Some(WindowId::Main));
        // Closing a window that does not hold the focus asks for nothing.
        app.submit(Command::ToggleWindow(WindowId::Playlist))
            .unwrap();
        assert_eq!(app.focused_window(), WindowId::Main);
        assert_eq!(app.take_focus_request(), None);
    }

    #[test]
    fn a_hidden_main_window_is_not_focusable() {
        let (mut app, _dir) = app_with_file(None);
        set_open(&mut app, false, false);
        app.submit(Command::ToggleWindow(WindowId::Main)).unwrap();
        assert!(!app.window_open(WindowId::Main));
        app.submit(Command::FocusNextWindow).unwrap();
        assert_eq!(app.take_focus_request(), None, "nothing can take focus");
        // Unlike the toggle, the explicit show brings it back.
        app.submit(Command::ShowMainWindow).unwrap();
        assert!(app.window_open(WindowId::Main));
        assert_eq!(app.take_focus_request(), Some(WindowId::Main));
    }

    #[test]
    fn show_main_window_shows_restores_and_focuses_once() {
        let (mut app, _dir) = app_with_file(None);
        set_open(&mut app, true, true);
        focus_next(&mut app);
        app.submit(Command::ToggleWindow(WindowId::Main)).unwrap();
        assert!(!app.main_visible());

        app.submit(Command::ShowMainWindow).unwrap();
        assert!(app.main_visible());
        assert_eq!(app.focused_window(), WindowId::Main);
        assert_eq!(app.take_focus_request(), Some(WindowId::Main));
        assert!(app.take_restore_request());
        assert!(!app.take_restore_request(), "taken, not repeated");
        // One-way: a second show keeps the window shown.
        app.submit(Command::ShowMainWindow).unwrap();
        assert!(app.main_visible());
        assert!(app.drain_notes().is_empty(), "the command is implemented");
    }

    #[test]
    fn the_later_of_minimize_and_show_main_window_wins() {
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::Minimize).unwrap();
        app.submit(Command::ShowMainWindow).unwrap();
        assert!(!app.take_minimize_request(), "the show beat the minimize");
        assert!(app.take_restore_request());

        app.take_focus_request();
        app.submit(Command::ShowMainWindow).unwrap();
        app.submit(Command::Minimize).unwrap();
        assert!(app.take_minimize_request(), "the minimize beat the show");
        assert!(!app.take_restore_request());
        assert_eq!(app.take_focus_request(), None, "no focus into a minimize");
    }

    #[test]
    fn focus_commands_keep_the_main_window_geometry() {
        // Focusing never resizes: a shaded, double-sized window keeps its size.
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::ToggleWindowShade(WindowId::Main))
            .unwrap();
        app.submit(Command::ToggleDoubleSize).unwrap();
        let (main_size, eq_size) = (app.frame_size(), app.eq_frame_size());
        app.submit(Command::FocusNextWindow).unwrap();
        app.submit(Command::ShowMainWindow).unwrap();
        assert_eq!(app.frame_size(), main_size);
        assert_eq!(app.eq_frame_size(), eq_size);
    }

    // ----- the simple modal dialogs (About, file info, jump-to) -----

    #[test]
    fn the_implemented_dialogs_open_real_windows_with_no_note() {
        // These commands open a real window now (`dialogs.rs`), so no
        // "not implemented yet" note may be recorded for them.
        use rustamp_core::command::{JumpTarget, OpenMediaKind, PrefsPage};
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenAbout).unwrap();
        assert!(app.dialog_is_open(DialogKind::About));
        app.submit(Command::OpenFileInfo).unwrap();
        assert!(app.dialog_is_open(DialogKind::FileInfo));
        app.submit(Command::OpenJump {
            target: JumpTarget::Time,
        })
        .unwrap();
        assert!(app.dialog_is_open(DialogKind::Jump(JumpTarget::Time)));
        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));
        // Each open-media kind opens its own face; the playlist ADD path
        // (`play: false`, RFC 0005 O5) opens the files face as sent.
        for kind in [
            OpenMediaKind::Files,
            OpenMediaKind::Directory,
            OpenMediaKind::Location,
        ] {
            app.submit(Command::OpenMedia { kind, play: true }).unwrap();
            assert!(app.dialog_is_open(DialogKind::OpenMedia { kind, play: true }));
        }
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        })
        .unwrap();
        assert!(app.dialog_is_open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        }));
        app.submit(Command::OpenSkinBrowser).unwrap();
        assert!(app.dialog_is_open(DialogKind::SkinBrowser));
        // A bare `OpenPrefs` shows the page model's own-choice default; a
        // page-carrying command shows the page it names (RFC 0005 §7).
        app.submit(Command::OpenPrefs).unwrap();
        assert_eq!(
            app.dialog_open_kind(),
            Some(DialogKind::Preferences(rustamp_prefs::DEFAULT_PAGE))
        );
        app.submit(Command::OpenPrefsPage(PrefsPage::VisOptions))
            .unwrap();
        assert!(app.dialog_is_open(DialogKind::Preferences(PrefsPage::VisOptions)));
        let notes = app.drain_notes();
        assert!(
            !notes.iter().any(|n| n.contains("not implemented yet")),
            "{notes:?}"
        );
    }

    #[test]
    fn the_remaining_dialog_commands_still_record_their_note() {
        // Only the bookmarks stay unimplemented (About, file info, jump, load
        // playlist, the open-media faces, the skin browser and preferences
        // open real windows now; the main menu opens its popup, covered in
        // `menu.rs`).
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::AddBookmark).unwrap();
        let notes = app.drain_notes();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(app.dialog_open_kind().is_none());
        assert!(!app.main_menu_is_open());
    }

    #[test]
    fn confirming_the_files_dialog_adds_the_picks_and_closes() {
        // `Enter` submits the staged picks through the existing `AddItems`
        // path and closes the dialog; the queue grows by the picks.
        use rustamp_core::command::OpenMediaKind;
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        })
        .unwrap();
        assert!(app.dialog_is_open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        }));
        let before = app.engine().queue().len();
        app.stage_media_paths(vec![PathBuf::from("picked.mp3")]);
        assert!(app.dialog_key(Key::Enter));
        assert_eq!(app.dialog_open_kind(), None);
        assert_eq!(app.engine().queue().len(), before + 1);
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn confirming_the_location_dialog_adds_the_typed_url() {
        // The URL arrives through the entry field (typing cannot produce
        // punctuation yet, so the test sets the full text programmatically).
        use rustamp_core::command::OpenMediaKind;
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Location,
            play: true,
        })
        .unwrap();
        assert!(app.dialog_is_open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Location,
            play: true,
        }));
        let before = app.engine().queue().len();
        app.set_media_text("http://example.com/stream.mp3");
        assert!(app.dialog_key(Key::Enter));
        assert_eq!(app.dialog_open_kind(), None);
        assert_eq!(app.engine().queue().len(), before + 1);
    }

    #[test]
    fn confirming_an_empty_open_media_dialog_keeps_it_open() {
        // With nothing picked and a blank field there is nothing to add:
        // `Enter` stays consumed and the dialog stays open for a retry.
        use rustamp_core::command::OpenMediaKind;
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        })
        .unwrap();
        let before = app.engine().queue().len();
        assert!(app.dialog_key(Key::Enter));
        assert!(app.dialog_is_open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        }));
        assert_eq!(app.engine().queue().len(), before);
    }

    #[test]
    fn cancelling_an_open_media_dialog_adds_nothing() {
        // `Escape` closes the dialog and drops the field and picks, so the
        // queue is unchanged.
        use rustamp_core::command::OpenMediaKind;
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        })
        .unwrap();
        app.stage_media_paths(vec![PathBuf::from("picked.mp3")]);
        assert!(app.dialog_key(Key::Escape));
        assert_eq!(app.dialog_open_kind(), None);
        assert!(app.drain_notes().is_empty());
        assert!(app.engine().queue().is_empty());
        assert!(!app.dialog_is_open(DialogKind::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        }));
    }

    #[test]
    fn the_playlist_add_path_keeps_its_unmeasured_play_flag() {
        // The ADD button emits `OpenMedia { kind: Files, play: false }` and
        // the value is reported, not decided: confirming adds without playing
        // (RFC 0005 O5 stays open — a 2.95 capture of the playlist add still
        // has to say whether it plays).
        use rustamp_core::command::OpenMediaKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        })
        .unwrap();
        app.stage_media_paths(vec![PathBuf::from("picked.mp3")]);
        assert!(app.dialog_key(Key::Enter));
        assert_eq!(app.engine().queue().len(), 1);
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn open_prefs_shows_the_default_page_and_page_commands_switch_it() {
        // `OpenPrefs` carries no page, so it shows the page model's
        // own-choice default; a page-carrying command switches the open frame
        // to the page it names (RFC 0005 §7) — with no note either way.
        use rustamp_core::command::PrefsPage;

        let (mut app, _dir) = app_with_file(None);
        assert_eq!(app.prefs_page(), None);
        app.submit(Command::OpenPrefs).unwrap();
        assert_eq!(app.prefs_page(), Some(rustamp_prefs::DEFAULT_PAGE));
        assert!(app.drain_notes().is_empty());

        app.submit(Command::OpenPrefsPage(PrefsPage::VisOptions))
            .unwrap();
        assert_eq!(app.prefs_page(), Some(PrefsPage::VisOptions));
        assert!(app.drain_notes().is_empty());

        // The direct switch does the same when the frame is open and reports
        // when it is not.
        assert!(app.select_prefs_page(PrefsPage::VisPlugin));
        assert_eq!(app.prefs_page(), Some(PrefsPage::VisPlugin));
        app.submit(Command::CloseFocusedWindow).unwrap();
        assert_eq!(app.prefs_page(), None);
        assert!(!app.select_prefs_page(PrefsPage::VisPlugin));
        assert!(!app.quit(), "a dialog request must not quit the app");
    }

    #[test]
    fn the_prefs_frame_keeps_enter_open_and_closes_on_escape() {
        // The frame has no controls yet (contents unmeasured), so `Enter`
        // stays consumed without confirming; `Escape` and the focused-window
        // close shut the frame, and none of it quits the player.
        use rustamp_core::command::PrefsPage;
        use rustamp_ui::{DialogKind, Key};

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenPrefsPage(PrefsPage::VisPlugin))
            .unwrap();
        assert!(app.dialog_key(Key::Enter));
        assert_eq!(app.prefs_page(), Some(PrefsPage::VisPlugin));
        assert!(app.dialog_key(Key::Escape));
        assert_eq!(app.dialog_open_kind(), None);

        app.submit(Command::OpenPrefs).unwrap();
        assert!(app.dialog_is_open(DialogKind::Preferences(rustamp_prefs::DEFAULT_PAGE)));
        app.submit(Command::CloseFocusedWindow).unwrap();
        assert_eq!(app.dialog_open_kind(), None);
        assert!(!app.quit(), "closing a dialog must not quit the app");
    }

    #[test]
    fn the_prefs_dialog_opens_from_the_menu_and_the_shortcut_path() {
        // Options > Preferences (Ctrl+P, id 40012) and the Visualization
        // leaves (Ctrl+K id 40191, Alt+O id 40317) all funnel through the
        // shortcut dispatch into the same frame, on their own page.
        use rustamp_core::command::PrefsPage;
        use rustamp_ui::{DialogKind, Key, Modifiers, ShortcutAction};

        let (mut app, _dir) = app_with_file(None);
        let outcome = app
            .activate_menu_item("Preferences", ShortcutAction::Preferences)
            .unwrap();
        assert!(
            matches!(
                outcome,
                crate::menu::MenuOutcome::Commands(ref commands)
                    if commands == &[Command::OpenPrefs]
            ),
            "{outcome:?}"
        );
        assert_eq!(
            app.dialog_open_kind(),
            Some(DialogKind::Preferences(rustamp_prefs::DEFAULT_PAGE))
        );
        app.submit(Command::CloseFocusedWindow).unwrap();

        let outcome = app
            .key_down(Key::letter('P').unwrap(), Modifiers::CTRL)
            .unwrap();
        assert!(
            matches!(
                outcome,
                Some(rustamp_ui::ShortcutOutcome::Commands(ref commands))
                    if commands == &[Command::OpenPrefs]
            ),
            "{outcome:?}"
        );
        assert_eq!(
            app.dialog_open_kind(),
            Some(DialogKind::Preferences(rustamp_prefs::DEFAULT_PAGE))
        );
        app.submit(Command::CloseFocusedWindow).unwrap();

        let outcome = app
            .key_down(Key::letter('K').unwrap(), Modifiers::CTRL)
            .unwrap();
        assert!(
            matches!(
                outcome,
                Some(rustamp_ui::ShortcutOutcome::Commands(ref commands))
                    if commands == &[Command::OpenPrefsPage(PrefsPage::VisPlugin)]
            ),
            "{outcome:?}"
        );
        assert_eq!(
            app.dialog_open_kind(),
            Some(DialogKind::Preferences(PrefsPage::VisPlugin))
        );
    }

    #[test]
    fn closing_the_focused_window_closes_the_dialog_first() {
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenAbout).unwrap();
        app.submit(Command::CloseFocusedWindow).unwrap();
        assert!(app.dialog_open_kind().is_none());
        assert!(!app.quit(), "closing a dialog must not quit the app");
        app.submit(Command::CloseFocusedWindow).unwrap();
        assert!(app.quit(), "with no dialog open the main window closes");
    }

    #[test]
    fn escape_closes_the_dialog_and_digits_type_into_the_jump_box() {
        use rustamp_core::command::JumpTarget;
        use rustamp_ui::{DialogKind, Key};

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenJump {
            target: JumpTarget::Time,
        })
        .unwrap();
        assert!(app.dialog_key(Key::Digit(9)));
        assert!(app.dialog_key(Key::Digit(0)));
        assert_eq!(app.jump_text(), "90");
        // Enter confirms: a Seek for 90 s is submitted (the empty queue turns
        // it into an error event, which must not take the app down). The dialog
        // closes on confirm so it does not linger or double-apply.
        assert!(app.dialog_key(Key::Enter));
        assert!(!app.quit());
        assert_eq!(app.dialog_open_kind(), None);
        // With no dialog open no key is consumed.
        assert!(!app.dialog_key(Key::Escape));
        let _ = DialogKind::About;
    }

    #[test]
    fn file_info_with_no_track_says_so() {
        let (app, _dir) = app_with_file(None);
        let lines = app.file_info_lines();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert_eq!(lines[0], "no track loaded");
    }

    #[test]
    fn rendering_a_dialog_produces_a_frame_of_the_dialog_size() {
        use rustamp_core::command::{OpenMediaKind, PrefsPage};
        use rustamp_ui::{
            ABOUT_CLIENT_SIZE, DialogKind, FILE_INFO_CLIENT_SIZE, JUMP_CLIENT_SIZE,
            LOAD_PLAYLIST_CLIENT_SIZE, OPEN_MEDIA_CLIENT_SIZE, PREFS_CLIENT_SIZE,
            SKIN_BROWSER_CLIENT_SIZE,
        };

        let (app, _dir) = app_with_file(None);
        for (dialog, size) in [
            (DialogKind::About, ABOUT_CLIENT_SIZE),
            (DialogKind::FileInfo, FILE_INFO_CLIENT_SIZE),
            (
                DialogKind::Jump(rustamp_core::command::JumpTarget::Time),
                JUMP_CLIENT_SIZE,
            ),
            (DialogKind::LoadPlaylist, LOAD_PLAYLIST_CLIENT_SIZE),
            (
                DialogKind::OpenMedia {
                    kind: OpenMediaKind::Files,
                    play: false,
                },
                OPEN_MEDIA_CLIENT_SIZE,
            ),
            (
                DialogKind::OpenMedia {
                    kind: OpenMediaKind::Directory,
                    play: true,
                },
                OPEN_MEDIA_CLIENT_SIZE,
            ),
            (
                DialogKind::OpenMedia {
                    kind: OpenMediaKind::Location,
                    play: true,
                },
                OPEN_MEDIA_CLIENT_SIZE,
            ),
            (DialogKind::SkinBrowser, SKIN_BROWSER_CLIENT_SIZE),
            (
                DialogKind::Preferences(PrefsPage::VisPlugin),
                PREFS_CLIENT_SIZE,
            ),
        ] {
            let image = app.render_dialog(dialog);
            assert_eq!((image.width(), image.height()), size, "{dialog:?}");
            assert!(
                image
                    .as_rgba()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| pixel[3] != 0),
                "the {dialog:?} frame is empty"
            );
        }
    }

    #[test]
    fn the_skin_browser_lists_what_the_app_can_load() {
        // No skin search path exists in the project, so the browser lists the
        // built-in default plus the current skin when it came from disk.
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenSkinBrowser).unwrap();
        assert!(app.dialog_is_open(DialogKind::SkinBrowser));
        let entries = app.browser_entries();
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].path, None);

        // An empty folder is a valid (if sparse) skin, so loading it makes it
        // the current skin without any fallback involved.
        let dir = tempfile::tempdir().unwrap();
        app.submit(Command::LoadSkin(dir.path().to_owned()))
            .unwrap();
        assert_eq!(app.current_skin(), Some(&dir.path().to_owned()));
        let entries = app.browser_entries();
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[1].path, Some(dir.path().to_owned()));
    }

    #[test]
    fn confirming_the_skin_browser_loads_the_pick_and_closes() {
        use rustamp_ui::Key;

        let dir = tempfile::tempdir().unwrap();
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::LoadSkin(dir.path().to_owned()))
            .unwrap();

        // Down to the file row, Enter loads it through the existing skin
        // path and closes the dialog.
        app.submit(Command::OpenSkinBrowser).unwrap();
        assert!(app.dialog_key(Key::Down));
        assert!(app.dialog_key(Key::Enter));
        assert_eq!(app.dialog_open_kind(), None);
        assert_eq!(app.current_skin(), Some(&dir.path().to_owned()));
        assert!(
            app.skin_description()
                .contains(dir.path().to_str().unwrap()),
            "{}",
            app.skin_description()
        );

        // Enter on the built-in row loads the built-in default instead.
        app.submit(Command::OpenSkinBrowser).unwrap();
        assert!(app.dialog_key(Key::Enter));
        assert_eq!(app.dialog_open_kind(), None);
        assert_eq!(app.current_skin(), None);
        assert_eq!(app.skin_description(), "built-in default skin");
    }

    #[test]
    fn leaving_the_skin_browser_loads_nothing() {
        use rustamp_ui::{DialogKind, Key};

        let dir = tempfile::tempdir().unwrap();
        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::LoadSkin(dir.path().to_owned()))
            .unwrap();

        // Escape closes without loading; the focused-window close does too,
        // and neither quits the player.
        app.submit(Command::OpenSkinBrowser).unwrap();
        assert!(app.dialog_key(Key::Down));
        assert!(app.dialog_key(Key::Escape));
        assert_eq!(app.dialog_open_kind(), None);
        assert_eq!(app.current_skin(), Some(&dir.path().to_owned()));

        app.submit(Command::OpenSkinBrowser).unwrap();
        assert!(app.dialog_is_open(DialogKind::SkinBrowser));
        app.submit(Command::CloseFocusedWindow).unwrap();
        assert_eq!(app.dialog_open_kind(), None);
        assert!(!app.quit(), "closing a dialog must not quit the app");
    }

    #[test]
    fn the_skin_browser_opens_from_the_menu_and_the_shortcut_path() {
        // The main menu's skin-browser leaf and Alt+S (S03) both funnel
        // through the shortcut dispatch into the same dialog.
        use rustamp_ui::{DialogKind, Key, Modifiers, ShortcutAction};

        let (mut app, _dir) = app_with_file(None);
        let outcome = app
            .activate_menu_item("Skin browser", ShortcutAction::SkinBrowser)
            .unwrap();
        assert!(
            matches!(
                outcome,
                crate::menu::MenuOutcome::Commands(ref commands)
                    if commands == &[Command::OpenSkinBrowser]
            ),
            "{outcome:?}"
        );
        assert!(app.dialog_is_open(DialogKind::SkinBrowser));
        app.submit(Command::CloseFocusedWindow).unwrap();

        let outcome = app
            .key_down(Key::letter('S').unwrap(), Modifiers::ALT)
            .unwrap();
        assert!(
            matches!(
                outcome,
                Some(rustamp_ui::ShortcutOutcome::Commands(ref commands))
                    if commands == &[Command::OpenSkinBrowser]
            ),
            "{outcome:?}"
        );
        assert!(app.dialog_is_open(DialogKind::SkinBrowser));
    }

    #[test]
    fn the_load_playlist_dialog_keeps_its_confirmation_open() {
        // An empty confirm keeps the dialog open (there is no path to load);
        // a confirmed one loads the file and closes the dialog (see the
        // load-playlist tests below). Escape closes; the focused-window
        // close does too.
        use rustamp_ui::{DialogKind, Key};

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));
        assert!(app.dialog_key(Key::Enter));
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));
        assert!(app.drain_notes().is_empty());
        assert!(app.dialog_key(Key::Escape));
        assert_eq!(app.dialog_open_kind(), None);

        app.submit(Command::OpenPlaylist).unwrap();
        app.submit(Command::CloseFocusedWindow).unwrap();
        assert_eq!(app.dialog_open_kind(), None);
        assert!(!app.quit(), "closing a dialog must not quit the app");
    }

    #[test]
    fn easy_move_decides_which_presses_drag_the_window() {
        // The easy-move contract (W10 finding F5): the setting is honoured by
        // the main-window drag decision, not only carried. A press no control
        // owns drags exactly while easy-move is on; a press on a widget
        // always reaches the control machine.
        let (mut app, _dir) = app_with_file(None);
        assert!(app.easy_move(), "the default config has easy-move on");

        // (5, 100) is background: outside every widget rect, so the control
        // machine ignores the press and the setting decides.
        assert!(
            app.mouse_down(5, 100).unwrap().is_empty(),
            "the background press must own nothing"
        );
        assert!(app.easy_drag_main(5, 100));

        // A control keeps its press whatever the setting says.
        let (x, y) = widget_centre(Widget::Play);
        assert!(!app.easy_drag_main(x, y));

        // With easy-move off (menu toggle, shortcut Ctrl+E per S03) nothing
        // drags: the OS chrome stays the only move path.
        app.submit(Command::ToggleEasyMove).unwrap();
        assert!(!app.easy_move());
        assert!(!app.easy_drag_main(5, 100));
        assert!(!app.easy_drag_main(x, y));
    }

    // ----- Headless tests for GUI-driven playback chains (task card N3) -----

    #[test]
    fn chain1_opening_file_via_open_media_enqueues_and_playing_starts_playback() {
        // Chain 1: Opening a file (the Open Media path) enqueues it and playing
        // it starts playback: queue length, current index and engine transport all agree.
        let (mut app, dir) = app_with_file(None);
        let t1 = track(&dir, "track1.tst");

        assert_eq!(app.engine().queue().len(), 0);
        assert_eq!(app.engine().current_index(), None);
        assert_eq!(app.engine().transport(), TransportState::Stopped);

        // Open file via Open Media dialog with play: false (playlist ADD path)
        app.submit(Command::OpenMedia {
            kind: rustamp_core::command::OpenMediaKind::Files,
            play: false,
        })
        .unwrap();
        assert!(app.dialog_is_open(rustamp_ui::DialogKind::OpenMedia {
            kind: rustamp_core::command::OpenMediaKind::Files,
            play: false,
        }));
        app.stage_media_paths(vec![t1]);
        assert!(app.dialog_key(Key::Enter));
        assert_eq!(app.dialog_open_kind(), None);

        // Enqueued: queue and playlist model agree
        assert_eq!(app.engine().queue().len(), 1);
        assert_eq!(app.playlist().len(), 1);
        assert_eq!(app.engine().transport(), TransportState::Stopped);

        // Playing it starts playback
        app.submit(Command::Play).unwrap();
        assert!(
            pump_until(
                &mut app,
                |app| app.engine().transport() == TransportState::Playing,
                2000
            ),
            "playback did not start"
        );
        // All agree afterwards
        assert_eq!(app.engine().queue().len(), 1);
        assert_eq!(app.engine().current_index(), Some(0));
        assert_eq!(app.engine().transport(), TransportState::Playing);
        assert_eq!(app.state().transport, TransportState::Playing);

        // Also test Open Media with play: true (e.g. shortcut / location)
        let t2 = track(&dir, "track2.tst");
        app.submit(Command::OpenMedia {
            kind: rustamp_core::command::OpenMediaKind::Files,
            play: true,
        })
        .unwrap();
        app.stage_media_paths(vec![t2]);
        assert!(app.dialog_key(Key::Enter));
        assert!(
            pump_until(
                &mut app,
                |app| app.engine().current_index() == Some(1),
                2000
            ),
            "play: true did not switch to new track"
        );
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.engine().current_index(), Some(1));
        assert_eq!(app.engine().transport(), TransportState::Playing);
    }

    #[test]
    fn chain2_double_clicking_playlist_entry_plays_that_entry() {
        // Chain 2: Double-clicking a playlist entry plays that entry
        // (the index the window reports reaches the engine).
        let (mut app, dir) = app_with_file(None);
        let t0 = track(&dir, "t0.tst");
        let t1 = track(&dir, "t1.tst");
        let t2 = track(&dir, "t2.tst");

        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(t0),
                MediaLocation::File(t1),
                MediaLocation::File(t2),
            ],
            at: None,
            play: false,
        })
        .unwrap();

        assert_eq!(app.engine().transport(), TransportState::Stopped);

        // Double-click row 1 (center of row 1 in playlist list band: y = 23 + 13 + 6 = 42, x = 60)
        let commands = app.playlist_double_click(60, 42).unwrap();
        assert_eq!(commands, vec![Command::PlayEntry(EntryId(1))]);
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(1)]);

        // Engine starts playing index 1
        assert!(
            pump_until(
                &mut app,
                |app| app.engine().current_index() == Some(1)
                    && app.engine().transport() == TransportState::Playing,
                2000
            ),
            "double-clicking row 1 did not play entry at index 1"
        );
        assert_eq!(app.engine().current_index(), Some(1));
        assert_eq!(app.engine().transport(), TransportState::Playing);

        // Double-click row 0 (y = 23 + 6 = 29, x = 60)
        let commands = app.playlist_double_click(60, 29).unwrap();
        assert_eq!(commands, vec![Command::PlayEntry(EntryId(0))]);
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(0)]);
        assert!(
            pump_until(
                &mut app,
                |app| app.engine().current_index() == Some(0)
                    && app.engine().transport() == TransportState::Playing,
                2000
            ),
            "double-clicking row 0 did not play entry at index 0"
        );
        assert_eq!(app.engine().current_index(), Some(0));
        assert_eq!(app.engine().transport(), TransportState::Playing);
    }

    #[test]
    fn chain3_next_and_previous_move_through_queue_across_button_menu_key_and_ends() {
        // Chain 3: Next/Previous (button, menu or key) move through the queue
        // and the engine follows, including at the ends.
        let (mut app, dir) = app_with_file(None);
        let t0 = track(&dir, "track0.tst");
        let t1 = track(&dir, "track1.tst");
        let t2 = track(&dir, "track2.tst");

        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(t0),
                MediaLocation::File(t1),
                MediaLocation::File(t2),
            ],
            at: None,
            play: false,
        })
        .unwrap();

        app.submit(Command::Play).unwrap();
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(0)
                && app.engine().transport() == TransportState::Playing,
            2000
        ));

        // 1. Previous at start (index 0) remains at index 0 across key, button, and menu:
        // Key 'Z'
        app.key_down(Key::letter('Z').unwrap(), Modifiers::NONE)
            .unwrap();
        assert_eq!(app.engine().current_index(), Some(0));

        // Button Previous
        let (px, py) = widget_centre(Widget::Previous);
        app.mouse_down(px, py).unwrap();
        app.mouse_up(px, py).unwrap();
        assert_eq!(app.engine().current_index(), Some(0));

        // Menu "Previous track"
        app.activate_menu_item("Previous track", rustamp_ui::ShortcutAction::PreviousTrack)
            .unwrap();
        assert_eq!(app.engine().current_index(), Some(0));

        // 2. Next via Button: moves 0 -> 1
        let (nx, ny) = widget_centre(Widget::Next);
        app.mouse_down(nx, ny).unwrap();
        let commands = app.mouse_up(nx, ny).unwrap();
        assert_eq!(commands, vec![Command::Next]);
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(1),
            2000
        ));
        assert_eq!(app.engine().transport(), TransportState::Playing);

        // 3. Next via Key 'B': moves 1 -> 2
        let outcome = app
            .key_down(Key::letter('B').unwrap(), Modifiers::NONE)
            .unwrap();
        assert_eq!(
            outcome,
            Some(rustamp_ui::ShortcutOutcome::Commands(vec![Command::Next]))
        );
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(2),
            2000
        ));
        assert_eq!(app.engine().transport(), TransportState::Playing);

        // 4. Previous via Menu: moves 2 -> 1
        let outcome = app
            .activate_menu_item("Previous track", rustamp_ui::ShortcutAction::PreviousTrack)
            .unwrap();
        assert_eq!(
            outcome,
            crate::menu::MenuOutcome::Commands(vec![Command::Prev])
        );
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(1),
            2000
        ));

        // 5. Previous via Key 'Z': moves 1 -> 0
        let outcome = app
            .key_down(Key::letter('Z').unwrap(), Modifiers::NONE)
            .unwrap();
        assert_eq!(
            outcome,
            Some(rustamp_ui::ShortcutOutcome::Commands(vec![Command::Prev]))
        );
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(0),
            2000
        ));

        // 6. Next via Menu: moves 0 -> 1
        let outcome = app
            .activate_menu_item("Next track", rustamp_ui::ShortcutAction::NextTrack)
            .unwrap();
        assert_eq!(
            outcome,
            crate::menu::MenuOutcome::Commands(vec![Command::Next])
        );
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(1),
            2000
        ));

        // Advance to last track (2)
        app.submit(Command::Next).unwrap();
        assert!(pump_until(
            &mut app,
            |app| app.engine().current_index() == Some(2),
            2000
        ));

        // 7. Next at the end: moves past the end of the queue and the run ends
        //    once the buffered audio has drained (the engine finishes on a
        //    drained ring, so the clock has to advance here).
        app.key_down(Key::letter('B').unwrap(), Modifiers::NONE)
            .unwrap();
        assert!(
            pump_until_drained(
                &mut app,
                |app| app.engine().transport() == TransportState::Stopped,
                2000
            ),
            "playback did not stop at end of queue"
        );
        assert!(app.playlist_ended);
    }

    #[test]
    fn chain4_playlist_window_model_stays_in_step_with_engine_queue() {
        // Chain 4: The playlist window's own model (its rows) stays in step with
        // the engine's queue when entries are added or removed.
        let (mut app, _dir) = app_with_file(None);

        assert_eq!(app.engine().queue().len(), 0);
        assert_eq!(app.playlist().len(), 0);
        assert_eq!(app.entry_ids.len(), 0);

        // Add 3 entries
        let items = (0..3)
            .map(|i| MediaLocation::File(PathBuf::from(format!("track{i}.mp3"))))
            .collect::<Vec<_>>();
        app.submit(Command::AddItems {
            items,
            at: None,
            play: false,
        })
        .unwrap();

        assert_eq!(app.engine().queue().len(), 3);
        assert_eq!(app.playlist().len(), 3);
        assert_eq!(app.entry_ids.len(), 3);
        assert_eq!(app.playlist().visible_ids(), app.entry_ids);

        // Remove row 1 via REM button
        app.submit(Command::Select(rustamp_core::command::SelectOp::Set(vec![
            EntryId(1),
        ])))
        .unwrap();
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(1)]);

        let cmds = app.playlist_mouse_down(46, 92).unwrap();
        assert_eq!(cmds, vec![Command::Remove(vec![EntryId(1)])]);

        // Model, entry_ids, and engine queue all agree after removal
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.playlist().len(), 2);
        assert_eq!(app.entry_ids.len(), 2);
        assert_eq!(app.entry_ids, vec![EntryId(0), EntryId(2)]);
        assert_eq!(app.playlist().visible_ids(), vec![EntryId(0), EntryId(2)]);

        // Add 2 more entries
        let items2 = vec![
            MediaLocation::File(PathBuf::from("track3.mp3")),
            MediaLocation::File(PathBuf::from("track4.mp3")),
        ];
        app.submit(Command::AddItems {
            items: items2,
            at: None,
            play: false,
        })
        .unwrap();

        assert_eq!(app.engine().queue().len(), 4);
        assert_eq!(app.playlist().len(), 4);
        assert_eq!(app.entry_ids.len(), 4);

        // Remove via external Command::Remove
        app.submit(Command::Remove(vec![EntryId(0), EntryId(3)]))
            .unwrap();
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.playlist().len(), 2);
        assert_eq!(app.entry_ids.len(), 2);
        assert_eq!(app.entry_ids, vec![EntryId(2), EntryId(4)]);
        assert_eq!(app.playlist().visible_ids(), vec![EntryId(2), EntryId(4)]);
    }

    #[test]
    fn sort_by_file_name_reorders_the_table_the_queue_and_the_model() {
        // S03 §2 `40210`: sort by file name, case-insensitively.
        let (mut app, dir) = app_with_file(None);
        let zulu = track(&dir, "Zulu.mp3");
        let alpha = track(&dir, "alpha.mp3");
        let beta = track(&dir, "Beta.mp3");
        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(zulu),
                MediaLocation::File(alpha),
                MediaLocation::File(beta),
            ],
            at: None,
            play: false,
        })
        .unwrap();

        app.submit(Command::Sort(SortKey::FileName)).unwrap();

        // alpha < Beta < Zulu once case is folded away.
        assert_eq!(app.entry_ids, vec![EntryId(1), EntryId(2), EntryId(0)]);
        assert_eq!(app.playlist().visible_ids(), app.entry_ids);
        let names: Vec<String> = app
            .engine()
            .queue()
            .iter()
            .map(|track| track.display())
            .collect();
        assert!(names[0].ends_with("alpha.mp3"), "{names:?}");
        assert!(names[1].ends_with("Beta.mp3"), "{names:?}");
        assert!(names[2].ends_with("Zulu.mp3"), "{names:?}");
    }

    #[test]
    fn sort_by_title_uses_the_loaded_titles_and_falls_back_to_the_file_name() {
        // S03 §2 `40209`: an entry the load did not name keeps the file name as
        // its key (the core's fallback).
        let (mut app, dir) = app_with_file(None);
        let named = track(&dir, "fileA.mp3");
        let unnamed = track(&dir, "fileB.mp3");
        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(named.clone()),
                MediaLocation::File(unnamed.clone()),
            ],
            at: None,
            play: false,
        })
        .unwrap();
        app.last_playlist_report = Some(rustamp_playlist::PlaylistReport::new(
            vec![
                rustamp_playlist::LoadedEntry::new(MediaLocation::File(named)).with_title("Zulu"),
                rustamp_playlist::LoadedEntry::new(MediaLocation::File(unnamed)),
            ],
            Vec::new(),
        ));

        app.submit(Command::Sort(SortKey::Title)).unwrap();

        // "fileb.mp3" < "zulu": the unnamed entry's fallback sorts first.
        assert_eq!(app.entry_ids, vec![EntryId(1), EntryId(0)]);
    }

    #[test]
    fn sort_by_duration_puts_unknown_durations_first() {
        // `SortKey::Duration`: unknown first (the core's `None`-before-`Some`
        // rule); the durations come from the last playlist load.
        let (mut app, dir) = app_with_file(None);
        let slow = track(&dir, "slow.mp3");
        let fast = track(&dir, "fast.mp3");
        let unknown = track(&dir, "unknown.mp3");
        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(slow.clone()),
                MediaLocation::File(fast.clone()),
                MediaLocation::File(unknown.clone()),
            ],
            at: None,
            play: false,
        })
        .unwrap();
        app.last_playlist_report = Some(rustamp_playlist::PlaylistReport::new(
            vec![
                rustamp_playlist::LoadedEntry::new(MediaLocation::File(slow))
                    .with_duration_ms(9_000),
                rustamp_playlist::LoadedEntry::new(MediaLocation::File(fast))
                    .with_duration_ms(1_000),
                rustamp_playlist::LoadedEntry::new(MediaLocation::File(unknown)),
            ],
            Vec::new(),
        ));

        app.submit(Command::Sort(SortKey::Duration)).unwrap();

        // unknown(2), then 1 s (1), then 9 s (0).
        assert_eq!(app.entry_ids, vec![EntryId(2), EntryId(1), EntryId(0)]);
    }

    #[test]
    fn equal_sort_keys_keep_their_relative_order_and_the_path_breaks_the_tie() {
        // Stable, like the core's rule: equal file names keep insertion order,
        // and only the full path separates them.
        let (mut app, dir) = app_with_file(None);
        let b_dir = dir.path().join("b").join("x.mp3");
        let a_dir = dir.path().join("a").join("x.mp3");
        app.submit(Command::AddItems {
            items: vec![MediaLocation::File(b_dir), MediaLocation::File(a_dir)],
            at: None,
            play: false,
        })
        .unwrap();

        app.submit(Command::Sort(SortKey::FileName)).unwrap();
        assert_eq!(
            app.entry_ids,
            vec![EntryId(0), EntryId(1)],
            "a tie is stable"
        );

        app.submit(Command::Sort(SortKey::PathAndFileName)).unwrap();
        assert_eq!(
            app.entry_ids,
            vec![EntryId(1), EntryId(0)],
            "the a/ path sorts first"
        );
    }

    #[test]
    fn sorting_keeps_the_current_entry_and_the_selection_by_identity() {
        use rustamp_core::command::SelectOp;

        let (mut app, dir) = app_with_file(None);
        let zulu = track(&dir, "zulu.mp3");
        let alpha = track(&dir, "alpha.mp3");
        let mike = track(&dir, "mike.mp3");
        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(zulu),
                MediaLocation::File(alpha),
                MediaLocation::File(mike),
            ],
            at: None,
            play: false,
        })
        .unwrap();
        app.engine_mut().set_current_index(Some(0));
        app.submit(Command::Select(SelectOp::Set(vec![EntryId(1)])))
            .unwrap();

        app.submit(Command::Sort(SortKey::FileName)).unwrap();

        // alpha(1), mike(2), zulu(0): zulu moved to row 2 and is still current;
        // the selected alpha is row 0 and still selected.
        assert_eq!(app.entry_ids, vec![EntryId(1), EntryId(2), EntryId(0)]);
        assert_eq!(app.engine().current_index(), Some(2));
        assert_eq!(app.entry_id_for(Some(2)), Some(EntryId(0)));
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(1)]);
    }

    #[test]
    fn dragging_the_playing_entry_moves_its_row_without_restarting_playback() {
        use rustamp_core::command::SelectOp;

        let (mut app, dir) = app_with_file(None);
        let items = (0..4)
            .map(|i| MediaLocation::File(track(&dir, &format!("drag{i}.tst"))))
            .collect();
        app.submit(Command::AddItems {
            items,
            at: None,
            play: false,
        })
        .unwrap();
        app.submit(Command::PlayEntry(EntryId(1))).unwrap();
        // This is a fresh run, so there is no stale ring to drain. Fill it
        // before driving the virtual device: pumping 1024 frames per 1 ms
        // while waiting can consume all four one-second tracks before the
        // decoder ever builds the required backlog, or advance to another
        // track before the drag starts. The clock stays frozen until ready.
        assert!(pump_until(
            &mut app,
            |app| app.engine().buffered_frames() > 4096 && app.engine().source_spec().is_some(),
            2000
        ));
        app.advance_null_clock(1024);
        app.pump().unwrap();
        assert_eq!(
            app.engine().current_index(),
            Some(1),
            "the dragged entry must already be playing"
        );
        app.submit(Command::Select(SelectOp::Set(vec![EntryId(1)])))
            .unwrap();
        let position = app.engine().position_ms();
        assert!(
            position > 0,
            "prove a running track, not just a Play command"
        );
        app.playlist_mouse_down(60, 42).unwrap();
        app.playlist_mouse_move(60, 74);
        assert_eq!(
            app.playlist_mouse_up(60, 74).unwrap(),
            vec![Command::MoveEntries {
                ids: vec![EntryId(1)],
                to: 3,
            }]
        );
        assert_eq!(app.engine().transport(), TransportState::Playing);
        assert_eq!(app.engine().position_ms(), position);
        app.pump().unwrap();
        assert_eq!(app.engine().transport(), TransportState::Playing);
        assert_eq!(app.engine().position_ms(), position);
        assert_eq!(app.engine().current_index(), Some(3));
        assert_eq!(
            app.entry_ids,
            vec![EntryId(0), EntryId(2), EntryId(3), EntryId(1)]
        );
        assert_eq!(
            app.engine().queue()[3].display(),
            dir.path().join("drag1.tst").display().to_string()
        );
        assert_eq!(app.playlist().selected_ids(), vec![EntryId(1)]);
        app.render_playlist();
        assert_eq!(app.playlist().current(), Some(EntryId(1)));
    }

    #[test]
    fn randomize_permutes_the_order_and_keeps_the_entries() {
        // S03 §2 `40212` (S15 §5 maps it to the in-place shuffle): the order
        // changes, the entries do not.
        let (mut app, dir) = app_with_file(None);
        let items = (0..32)
            .map(|i| MediaLocation::File(track(&dir, &format!("r{i:02}.mp3"))))
            .collect::<Vec<_>>();
        app.submit(Command::AddItems {
            items,
            at: None,
            play: false,
        })
        .unwrap();
        let before = app.entry_ids.clone();

        app.submit(Command::Randomize).unwrap();

        let after = app.entry_ids.clone();
        let mut before_sorted = before.clone();
        before_sorted.sort_by_key(|id| id.0);
        let mut after_sorted = after.clone();
        after_sorted.sort_by_key(|id| id.0);
        assert_eq!(before_sorted, after_sorted, "the same 32 ids");
        assert_ne!(before, after, "a 32-entry shuffle changes the order");
        // The model holds every entry and mirrors the new order (only the
        // visible page is exposed).
        assert_eq!(app.playlist().len(), after.len());
        let visible = app.playlist().visible_ids();
        assert_eq!(visible.as_slice(), &after[..visible.len()]);
    }

    #[test]
    fn chain5_removing_playing_entry_or_clearing_leaves_defined_state() {
        // Chain 5: Removing the playing entry, or clearing the list while playing,
        // leaves the engine in a defined state (and say which).
        //
        // Defined state:
        // - When the playing entry is removed: the engine stops (TransportState::Stopped),
        //   position resets to 0 ms, and the queue & playlist model contain the remaining entries.
        // - When the list is cleared while playing: the engine stops (TransportState::Stopped),
        //   position resets to 0 ms, and the queue & playlist model are empty.
        // - When a non-playing entry is removed while playing: playback continues (TransportState::Playing),
        //   and the queue & playlist model update to the remaining entries.
        let (mut app, dir) = app_with_file(None);
        let t0 = track(&dir, "p0.tst");
        let t1 = track(&dir, "p1.tst");
        let t2 = track(&dir, "p2.tst");

        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(t0),
                MediaLocation::File(t1),
                MediaLocation::File(t2),
            ],
            at: None,
            play: false,
        })
        .unwrap();

        // Subcase 5A: Removing a non-playing entry while playing keeps playback active
        app.submit(Command::PlayEntry(EntryId(0))).unwrap();
        assert!(pump_until(
            &mut app,
            |app| app.engine().transport() == TransportState::Playing
                && app.engine().current_index() == Some(0),
            2000
        ));

        // Remove track 2 (non-playing)
        app.submit(Command::Remove(vec![EntryId(2)])).unwrap();
        assert_eq!(app.engine().transport(), TransportState::Playing);
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.playlist().len(), 2);

        // Subcase 5B: Removing the currently playing entry stops the engine
        app.submit(Command::Remove(vec![EntryId(0)])).unwrap();
        assert_eq!(app.engine().transport(), TransportState::Stopped);
        assert_eq!(app.engine().position_ms(), 0);
        assert_eq!(app.engine().queue().len(), 1);
        assert_eq!(app.playlist().len(), 1);
        assert_eq!(app.entry_ids, vec![EntryId(1)]);

        // Subcase 5C: Clearing the playlist while playing stops the engine and empties queue & model
        app.submit(Command::Play).unwrap();
        assert!(pump_until(
            &mut app,
            |app| app.engine().transport() == TransportState::Playing,
            2000
        ));

        app.submit(Command::Clear).unwrap();
        assert_eq!(app.engine().transport(), TransportState::Stopped);
        assert_eq!(app.engine().position_ms(), 0);
        assert!(app.engine().queue().is_empty());
        assert!(app.playlist().is_empty());
        assert!(app.entry_ids.is_empty());
    }

    #[test]
    fn remove_dead_files_drops_only_the_missing_local_files() {
        // S03 §2 `40222` ("Remove all dead files", RFC 0018): the entry whose
        // file is gone leaves; the existing file and the stream stay. The
        // stream is never probed — a URL is not a file (own choice, RFC 0018).
        let (mut app, dir) = app_with_file(None);
        let alive = track(&dir, "alive.tst");
        let gone = dir.path().join("gone.tst");
        assert!(!gone.exists(), "the fixture must start absent");
        app.submit(Command::AddItems {
            items: vec![
                MediaLocation::File(alive.clone()),
                MediaLocation::File(gone),
                MediaLocation::Url("http://example.invalid/stream.mp3".to_string()),
            ],
            at: None,
            play: false,
        })
        .unwrap();
        assert_eq!(app.entry_ids.len(), 3);

        app.submit(Command::RemoveDeadFiles).unwrap();
        // Stable ids, in order: the dead middle entry is gone, nothing renumbers.
        assert_eq!(app.entry_ids, vec![EntryId(0), EntryId(2)]);
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.playlist().len(), 2);
        assert_eq!(app.playlist().visible_ids(), vec![EntryId(0), EntryId(2)]);

        // No dead entry left: the command is a silent no-op.
        app.submit(Command::RemoveDeadFiles).unwrap();
        assert_eq!(app.entry_ids, vec![EntryId(0), EntryId(2)]);
        assert!(app.drain_notes().is_empty());
    }

    #[test]
    fn remove_dead_files_stops_the_engine_when_the_playing_entry_is_dead() {
        // The removal goes through `remove_entries`, so a dead current entry
        // stops playback and clears Current, exactly like any other removal.
        let (mut app, dir) = app_with_file(None);
        let dead = dir.path().join("never-written.tst");
        app.submit(Command::AddItems {
            items: vec![MediaLocation::File(dead.clone())],
            at: None,
            play: false,
        })
        .unwrap();
        // The engine refuses to open a missing file, so stage the current row
        // the way a previously alive entry would look after the file vanished.
        app.engine_mut().set_current_index(Some(0));
        app.submit(Command::RemoveDeadFiles).unwrap();
        assert!(app.entry_ids.is_empty());
        assert!(app.engine().queue().is_empty());
        assert_eq!(app.engine().current_index(), None);
        assert!(app.playlist().is_empty());
    }

    /// Pins the OS window parameters the two new dialogs must carry (titles
    /// and client sizes) for the run that does open them. **VM-only,
    /// `#[ignore]`d by default, and gated on the test VM**: GUI must never
    /// run on the host.
    ///
    /// ```text
    /// cargo test -p rustamp-app -- --ignored dialogs
    /// ```
    #[ignore = "VM-only: pins dialog window parameters; run inside the headless test VM"]
    #[test]
    fn the_new_dialogs_pin_their_os_window_parameters() {
        // The gate, before anything else. This test opens no window itself;
        // it pins the parameters the presenter (`window.rs`) builds the real
        // OS windows from.
        crate::window::require_test_vm();
        use rustamp_core::command::{OpenMediaKind, PrefsPage};
        use rustamp_ui::{
            DialogKind, LOAD_PLAYLIST_CLIENT_SIZE, OPEN_MEDIA_CLIENT_SIZE, PREFS_CLIENT_SIZE,
            SKIN_BROWSER_CLIENT_SIZE,
        };
        assert_eq!(DialogKind::LoadPlaylist.title(), "Load Playlist");
        assert_eq!(
            DialogKind::LoadPlaylist.client_size(),
            LOAD_PLAYLIST_CLIENT_SIZE
        );
        assert_eq!(
            DialogKind::OpenMedia {
                kind: OpenMediaKind::Files,
                play: false,
            }
            .title(),
            "Add file(s) to playlist"
        );
        for kind in [
            OpenMediaKind::Files,
            OpenMediaKind::Directory,
            OpenMediaKind::Location,
        ] {
            assert_eq!(
                DialogKind::OpenMedia { kind, play: true }.client_size(),
                OPEN_MEDIA_CLIENT_SIZE,
                "{kind:?}"
            );
        }
        assert_eq!(DialogKind::SkinBrowser.title(), "Skin browser");
        assert_eq!(
            DialogKind::SkinBrowser.client_size(),
            SKIN_BROWSER_CLIENT_SIZE
        );
        assert_eq!(
            DialogKind::Preferences(PrefsPage::VisPlugin).title(),
            "Preferences"
        );
        assert_eq!(
            DialogKind::Preferences(PrefsPage::VisPlugin).client_size(),
            PREFS_CLIENT_SIZE
        );
    }

    #[test]
    fn clutter_i_opens_file_info_only_when_track_loaded() {
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        // With no track loaded, clicking clutter I emits nothing and opens no dialog (S02 §2).
        let down_cmds = app.mouse_down(15, 43).unwrap();
        assert!(down_cmds.is_empty());
        let up_cmds = app.mouse_up(15, 43).unwrap();
        assert!(up_cmds.is_empty());
        assert_eq!(app.dialog_open_kind(), None);

        // Load a track.
        app.handle_event(&rustamp_core::event::Event::TrackChanged {
            id: Some(rustamp_core::playlist::EntryId(1)),
            index: Some(0),
            metadata: None,
            duration_ms: Some(180_000),
        });

        // Now clicking clutter I emits OpenFileInfo and opens the FileInfo dialog.
        let down_cmds = app.mouse_down(15, 43).unwrap();
        assert!(down_cmds.is_empty());
        let up_cmds = app.mouse_up(15, 43).unwrap();
        assert_eq!(up_cmds, vec![Command::OpenFileInfo]);
        assert_eq!(app.dialog_open_kind(), Some(DialogKind::FileInfo));
    }

    #[test]
    fn dialog_result_paths_cancelled_confirmed_and_os_close() {
        use rustamp_core::command::JumpTarget;
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);

        // 1. Cancelled dialog via Escape: changes nothing and closes dialog.
        app.submit(Command::OpenJump {
            target: JumpTarget::Time,
        })
        .unwrap();
        assert_eq!(
            app.dialog_open_kind(),
            Some(DialogKind::Jump(JumpTarget::Time))
        );
        assert!(app.dialog_key(Key::Digit(4)));
        assert!(app.dialog_key(Key::Digit(2)));
        assert_eq!(app.jump_text(), "42");
        assert!(app.dialog_key(Key::Escape));
        assert_eq!(app.dialog_open_kind(), None);

        // 2. Confirmed dialog via Enter: applies exactly once and closes.
        app.submit(Command::OpenJump {
            target: JumpTarget::Time,
        })
        .unwrap();
        assert_eq!(
            app.dialog_open_kind(),
            Some(DialogKind::Jump(JumpTarget::Time))
        );
        assert!(app.dialog_key(Key::Digit(1)));
        assert!(app.dialog_key(Key::Digit(0)));
        assert_eq!(app.jump_text(), "10");
        // Enter confirms: seek is submitted, dialog closes immediately.
        assert!(app.dialog_key(Key::Enter));
        assert_eq!(app.dialog_open_kind(), None);
        // Pressing Enter again does NOT double-apply: no dialog is open.
        assert!(!app.dialog_key(Key::Enter));

        // 3. Closed by OS: close_dialog resets state cleanly without applying.
        app.submit(Command::OpenAbout).unwrap();
        assert_eq!(app.dialog_open_kind(), Some(DialogKind::About));
        app.close_dialog(DialogKind::About);
        assert_eq!(app.dialog_open_kind(), None);
    }

    #[test]
    fn good_playlist_file_loads_and_plays() {
        use rustamp_ui::DialogKind;

        let (mut app, dir) = app_with_file(None);
        let t1 = track(&dir, "track1.tst");
        let t2 = track(&dir, "track2.tst");

        let m3u_path = dir.path().join("good.m3u");
        std::fs::write(
            &m3u_path,
            b"#EXTM3U\r\n#EXTINF:120,First Track\r\ntrack1.tst\r\n#EXTINF:180,Second Track\r\ntrack2.tst\r\n",
        )
        .unwrap();

        assert_eq!(app.engine().queue().len(), 0);
        assert_eq!(app.engine().transport(), TransportState::Stopped);

        // Open Load Playlist dialog
        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));

        // Stage playlist file and confirm
        app.stage_playlist_path(m3u_path);
        assert!(app.dialog_key(Key::Enter));
        assert_eq!(app.dialog_open_kind(), None);

        // Queue has surviving entries in order
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.playlist().len(), 2);
        assert_eq!(app.engine().queue()[0], TrackSource::path(t1));
        assert_eq!(app.engine().queue()[1], TrackSource::path(t2));

        // First entry starts playback. The engine starts the run synchronously
        // (`play_index` sets the transport before the dialog key returns), but
        // the UI machine learns Playing only when the app pumps the
        // StateChanged event off the bus, so the wait must cover the UI state
        // it asserts: a wait on the engine alone is already satisfied before
        // the first pump and would assert an event nobody has delivered yet.
        assert!(pump_until(
            &mut app,
            |app| app.engine().transport() == TransportState::Playing
                && app.engine().current_index() == Some(0)
                && app.state().transport == TransportState::Playing,
            2000
        ));
        assert_eq!(app.state().transport, TransportState::Playing);

        // Clean report recorded
        let report = app.last_playlist_report().expect("report recorded");
        assert!(report.is_clean());
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.skip_count(), 0);
        assert_eq!(app.last_error(), None);
    }

    #[test]
    fn playlist_file_with_bad_entry_loads_surviving_and_reports_skips() {
        use rustamp_ui::DialogKind;

        let (mut app, dir) = app_with_file(None);
        let t1 = track(&dir, "track1.tst");
        let t2 = track(&dir, "track2.tst");

        // Playlist with a dangling EXTINF (skipped by reader)
        let m3u_path = dir.path().join("with_skips.m3u");
        std::fs::write(
            &m3u_path,
            b"#EXTM3U\r\n#EXTINF:60,Dangling Track\r\n#EXTINF:120,Track One\r\ntrack1.tst\r\n#EXTINF:180,Track Two\r\ntrack2.tst\r\n",
        )
        .unwrap();

        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));

        app.stage_playlist_path(m3u_path);
        assert!(app.dialog_key(Key::Enter));
        assert_eq!(app.dialog_open_kind(), None);

        // Surviving entries reach queue in order
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.playlist().len(), 2);
        assert_eq!(app.engine().queue()[0], TrackSource::path(t1));
        assert_eq!(app.engine().queue()[1], TrackSource::path(t2));

        // First entry plays
        assert!(pump_until(
            &mut app,
            |app| app.engine().transport() == TransportState::Playing
                && app.engine().current_index() == Some(0),
            2000
        ));

        // Skips are recorded on App and surfaced in notes
        let report = app.last_playlist_report().expect("report recorded");
        assert!(!report.is_clean());
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.skip_count(), 1);
        assert!(report.skips[0].reason().contains("dangling #EXTINF"));

        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|n| n.contains("loaded 2 tracks, skipped 1 malformed entries")),
            "notes: {notes:?}"
        );
        assert!(
            notes
                .iter()
                .any(|n| n.contains("skipped: dangling #EXTINF")),
            "notes: {notes:?}"
        );
    }

    /// The other half of readiness finding BETA-09 (review revision
    /// `f9012dc`): a playlist named as the CLI `<file>` must load as a *list*
    /// through the reporting reader, not queue as one media file. The dialog
    /// half is pinned above; this pins the CLI half, which no test covered.
    #[test]
    fn playlist_file_given_on_the_command_line_loads_as_a_list() {
        let dir = tempfile::tempdir().unwrap();
        let t1 = track(&dir, "track1.tst");
        let t2 = track(&dir, "track2.tst");

        // A dangling EXTINF before the first entry: the reader drops it and
        // keeps the reason, so an app that called the legacy `read_playlist` /
        // `load_playlist_file` (which discard it through `into_entries`) could
        // not satisfy the skip assertions below.
        let m3u_path = dir.path().join("cli.m3u");
        std::fs::write(
            &m3u_path,
            b"#EXTM3U\r\n#EXTINF:60,Dangling Track\r\n#EXTINF:120,Track One\r\ntrack1.tst\r\n#EXTINF:180,Track Two\r\ntrack2.tst\r\n",
        )
        .unwrap();

        // The playlist path *is* the CLI `<file>` argument.
        let (mut app, _d) = app_with_file(Some(m3u_path.clone()));

        // It loaded as a list of its surviving entries, in file order...
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.engine().queue()[0], TrackSource::path(t1));
        assert_eq!(app.engine().queue()[1], TrackSource::path(t2));
        // ...and the playlist itself is not in the queue as a media path.
        assert!(!app.engine().queue().contains(&TrackSource::path(m3u_path)));

        // The kept reasons reached the app: the report is recorded and not
        // silent, and no error was fabricated.
        let report = app.last_playlist_report().expect("report recorded");
        assert!(!report.is_clean());
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.skip_count(), 1);
        assert!(report.skips[0].reason().contains("dangling #EXTINF"));
        assert_eq!(app.last_error(), None);

        // The drop is surfaced as a note (stderr in windowed runs).
        let notes = app.drain_notes();
        assert!(
            notes
                .iter()
                .any(|n| n.contains("loaded 2 tracks, skipped 1 malformed entries")),
            "notes: {notes:?}"
        );
        assert!(
            notes
                .iter()
                .any(|n| n.contains("skipped: dangling #EXTINF")),
            "notes: {notes:?}"
        );
    }

    #[test]
    fn unsupported_playlist_file_reports_error_and_changes_nothing() {
        use rustamp_ui::DialogKind;

        let dir = tempfile::tempdir().unwrap();
        let t0 = track(&dir, "original.tst");
        let (mut app, _d) = app_with_file(Some(t0.clone()));

        // Start with 1 track in queue
        assert_eq!(app.engine().queue().len(), 1);

        // Write an unsupported/corrupted playlist file (invalid binary content with m3u extension)
        let bad_path = dir.path().join("corrupted.m3u");
        std::fs::write(&bad_path, b"\x00\x01\x02\x03\x04\x05").unwrap();

        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));

        app.stage_playlist_path(bad_path.clone());
        assert!(app.dialog_key(Key::Enter));
        assert_eq!(app.dialog_open_kind(), None);

        // Queue is completely unchanged
        assert_eq!(app.engine().queue().len(), 1);
        assert_eq!(app.engine().queue()[0], TrackSource::path(t0));

        // Error is surfaced visibly in last_error and window_title
        let error = app.last_error().expect("last_error must be set");
        assert!(
            error.contains("failed to load playlist")
                && error.contains("unrecognized playlist content"),
            "error: {error}"
        );
        assert!(
            app.window_title()
                .starts_with("Rustamp - failed to load playlist")
        );

        // Error is also logged in notes
        let notes = app.drain_notes();
        assert!(
            notes.iter().any(|n| n.contains("failed to load playlist")),
            "notes: {notes:?}"
        );
    }

    #[test]
    fn load_playlist_dialog_text_input_and_display() {
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));

        // Initial body lines: prompt, empty field, hint
        let body = app.open_dialog_lines().expect("dialog open");
        assert_eq!(body[0], "Playlist file:");
        assert_eq!(body[1], "");
        assert_eq!(body[2], crate::dialogs::OPEN_PLAYLIST_HINT);

        // Type text into dialog
        assert!(app.dialog_text("my_list.m3u"));
        assert_eq!(app.playlist_text(), "my_list.m3u");

        let body = app.open_dialog_lines().expect("dialog open");
        assert_eq!(body[1], "my_list.m3u");

        // Backspace
        assert!(app.dialog_key(Key::Backspace));
        assert_eq!(app.playlist_text(), "my_list.m3");

        // Key letters/digits
        assert!(app.dialog_key(Key::Letter(b'u')));
        assert_eq!(app.playlist_text(), "my_list.m3u");
    }

    #[test]
    fn about_dialog_body_reflects_workspace_version() {
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenAbout).unwrap();
        assert!(app.dialog_is_open(DialogKind::About));

        let body = app.open_dialog_lines().expect("about dialog open");
        assert_eq!(
            body[0],
            format!("{} {}", rustamp_ui::PROGRAM_NAME, env!("CARGO_PKG_VERSION")),
            "about dialog first line must show the workspace version"
        );
    }

    #[test]
    fn load_playlist_uses_dialog_manager_contract() {
        use rustamp_ui::DialogKind;

        let (mut app, _dir) = app_with_file(None);
        app.submit(Command::OpenPlaylist).unwrap();
        assert!(app.dialog_is_open(DialogKind::LoadPlaylist));

        // Direct staging routes to DialogManager
        app.stage_playlist_path(PathBuf::from("staged.m3u"));
        assert_eq!(
            app.dialogs.confirm_load_playlist_path(),
            Some(PathBuf::from("staged.m3u"))
        );

        // Dropping a file routes to DialogManager
        assert!(app.dialog_drop_file(PathBuf::from("dropped.pls")));
        assert_eq!(
            app.dialogs.confirm_load_playlist_path(),
            Some(PathBuf::from("dropped.pls"))
        );

        // Closing resets DialogManager state
        app.close_dialog(DialogKind::LoadPlaylist);
        assert_eq!(app.dialogs.confirm_load_playlist_path(), None);
        assert!(app.playlist_text().is_empty());
    }

    #[test]
    fn recreate_engine_preserves_playback_state_and_cleans_flush_failure() {
        let (mut app, dir) = app_with_file(None);
        let t1 = track(&dir, "track1.tst");
        let t2 = track(&dir, "track2.tst");

        app.submit(Command::SetVolume(180)).unwrap();
        app.submit(Command::SetBalance(42)).unwrap();
        app.submit(Command::SetEqEnabled(true)).unwrap();
        app.submit(Command::SetPreamp(3.0)).unwrap();
        app.submit(Command::ToggleShuffle).unwrap();
        app.submit(Command::ToggleRepeat).unwrap();

        let queue = vec![TrackSource::path(t1.clone()), TrackSource::path(t2.clone())];
        app.engine_mut().set_queue(queue);
        app.engine_mut().set_current_index(Some(1));

        assert_eq!(app.engine().volume(), 180);
        assert_eq!(app.engine().balance(), 42);
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.engine().current_index(), Some(1));
        assert!(app.engine().shuffle());
        assert!(app.engine().repeat());
        assert!(!app.engine().is_flush_failed());

        // Recreate the engine and output
        app.recreate_engine().expect("recreation succeeds");

        assert_eq!(app.engine().volume(), 180);
        assert_eq!(app.engine().balance(), 42);
        assert_eq!(app.engine().queue().len(), 2);
        assert_eq!(app.engine().current_index(), Some(1));
        assert!(app.engine().shuffle());
        assert!(app.engine().repeat());
        assert!(!app.engine().is_flush_failed());
        assert_eq!(app.engine().transport(), TransportState::Stopped);
        assert!(app.engine().last_error().is_none());
    }

    /// The division of labour the RFC left open (W23 gap 3), end to end.
    ///
    /// A fatal mid-run stream error is recovered by the engine reopening the
    /// device it already owns. The application must not step in with
    /// `recreate_engine()`, because its fresh engine starts at position zero
    /// and would destroy the frozen position of RFC 0012 §4.3 item 4.
    ///
    /// The discriminator is the lifecycle counter of the output object the app
    /// held before the failure: a caller-side recreation builds a *new*
    /// `NullOutput::pair()`, whose `opens()` reads 1 after `Engine::new`, while
    /// the engine's own reopen takes the old object's `opens()` to 2.
    #[test]
    fn a_stream_error_is_recovered_by_the_engine_not_by_app_recreation() {
        let (mut app, dir) = app_with_file(None);
        let track = track(&dir, "track1_44100.tst");
        app.engine_mut().set_queue(vec![TrackSource::path(track)]);

        app.submit(Command::Play).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while app.engine().buffered_frames() < 256 {
            app.pump().unwrap();
            assert!(std::time::Instant::now() < deadline, "audio never buffered");
            std::thread::yield_now();
        }
        app.advance_null_clock(256);
        app.pump().unwrap();
        assert!(app.engine().position_frames() > 0);
        let clock = app.clock.clone().expect("the null output has a clock");
        assert_eq!(clock.opens(), 1);

        clock.fail();
        app.pump().unwrap();

        assert_eq!(app.engine().transport(), TransportState::Stopped);
        assert!(app.engine().is_stream_lost(), "the engine armed its reopen");
        assert!(
            !app.engine().is_flush_failed(),
            "nothing asked the caller to recreate anything"
        );
        assert!(
            clock.is_failed(),
            "the app still holds the very output that failed"
        );
        assert_eq!(
            app.clock.as_ref().unwrap().opens(),
            1,
            "the app did not recreate the engine or the output"
        );
        let frozen_ms = app.engine().position_ms();
        assert!(
            frozen_ms > 0,
            "the frozen position survived: {frozen_ms} ms"
        );
        for _ in 0..3 {
            app.ensure_engine_ready().unwrap();
            app.pump().unwrap();
            assert_eq!(app.engine().position_ms(), frozen_ms);
            assert!(
                app.last_error().is_some(),
                "the device failure stays visible"
            );
        }

        // Play again: the engine reopens its own device.
        app.submit(Command::Play).unwrap();
        assert_eq!(
            app.clock.as_ref().unwrap().opens(),
            2,
            "the same output object was reopened, not replaced"
        );
        assert!(!app.engine().is_stream_lost());
        assert!(!clock.is_failed(), "the reopen cleared the device latch");
        assert_eq!(app.engine().transport(), TransportState::Playing);
    }
}
