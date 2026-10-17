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

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use rustamp_audio::{Engine, EngineConfig, TrackSource};
use rustamp_core::audio::{DecoderRegistry, OutputDevice};
use rustamp_core::bus::{CommandReceiver, CommandSender, EventBus, EventReceiver, command_channel};
use rustamp_core::command::Command;
use rustamp_core::config::Config;
use rustamp_core::error::{Error, ErrorInfo, Result};
use rustamp_core::event::Event;
use rustamp_core::playlist::{EntryId, MediaLocation};
use rustamp_core::state::{PlayerState, TransportState, WindowId};
use rustamp_gfx::Image;
use rustamp_ipc::{Message, Transport};
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
    layout: MainWindowLayout,
    /// The equalizer's headless behaviour (S07; the engine stage is M2 future
    /// work, so this is the UI's own prediction state).
    eq: EqualizerWindow,
    eq_layout: EqualizerLayout,
    pub(crate) skin: LoadedSkin,
    config: Config,

    bus: EventBus,
    events: EventReceiver,
    commands: CommandSender,
    command_rx: CommandReceiver,

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
    minimize_requested: bool,
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
    /// Whether the main-menu popup is open (owned here, rendered by
    /// `menu.rs`, hosted by `window.rs`; S04).
    pub(crate) main_menu_open: bool,
    notes: Vec<String>,
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

        let (output, clock): (Box<dyn OutputDevice>, Option<NullClock>) = match options.output {
            OutputChoice::Null | OutputChoice::NoAudio => {
                let (output, clock) = NullOutput::pair();
                (Box::new(output), Some(clock))
            }
            OutputChoice::Default => (default_output(), None),
        };

        let cfg = EngineConfig {
            volume: options.volume,
            balance: options.config.audio.balance,
            ..EngineConfig::default()
        };
        let mut engine = Engine::new(registry, output, bus.clone(), cfg)?;
        // `Engine::new` initialises its gain from the config but reports
        // `volume: 255, balance: 0` in `PlayerState` (an engine-side
        // inconsistency; see the findings). Re-assert the configured values so
        // the reported state matches what the engine actually applies.
        engine.set_volume(options.volume)?;
        engine.set_balance(options.config.audio.balance)?;
        if let Some(file) = &options.file {
            engine.set_queue(vec![TrackSource::path(file.clone())]);
        }

        // Seed the UI machine from the config and the engine's initial state.
        let initial = MainWindowState {
            volume: volume_percent(options.volume),
            balance: rustamp_ui::balance_percent(options.config.audio.balance),
            double_size: options.config.ui.double_size,
            clutter_a: options.config.ui.always_on_top,
            shuffle: options.config.playlist.shuffle,
            repeat: options.config.playlist.repeat,
            ..MainWindowState::default()
        };

        let mut app = App {
            engine,
            ui: MainWindow::from_state(initial),
            layout: MainWindowLayout::CLASSIC,
            eq: EqualizerWindow::new(),
            eq_layout: EqualizerLayout::CLASSIC,
            skin: options.skin,
            config: options.config,
            bus,
            events,
            commands,
            command_rx,
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
            minimize_requested: false,
            playlist_ended: false,
            entry_ids: Vec::new(),
            next_entry_id: 0,
            playlist: PlaylistWindow::new(),
            dialogs: DialogManager::new(),
            main_menu_open: false,
            notes: Vec::new(),
            host: HostStateLink::detached(),
            host_supervisor: None,
        };
        app.shuffle = app.config.playlist.shuffle;
        app.repeat = app.config.playlist.repeat;
        app.always_on_top = app.config.ui.always_on_top;
        app.easy_move = app.config.ui.easy_move;
        // A path skin remembers where it came from, so `RefreshSkin` can re-read it.
        app.current_skin = app.skin.path().map(PathBuf::from);
        // The engine's snapshot is the authoritative starting state.
        app.sync_entry_ids();
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

    /// The headless UI machine, mutably (tests and the presenter).
    pub fn ui_mut(&mut self) -> &mut MainWindow {
        &mut self.ui
    }

    /// The audio engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// The loaded configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// A one-line description of the loaded skin.
    pub fn skin_description(&self) -> &str {
        self.skin.description()
    }

    /// Whether the app should exit.
    pub fn quit(&self) -> bool {
        self.quit
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
    /// and top-level entries (Load/Save/Delete) and fixes nothing beyond that,
    /// so this records the request through the existing dialog-stub path and
    /// opens no window: menu internals, submenu contents and load/save
    /// semantics are intentionally not implemented here (S07 §12 O12, S04).
    /// Returns the entries the real popup must carry.
    pub fn request_eq_presets_menu(&mut self, request: EqMenuRequest) -> &'static [&'static str] {
        let entries = request.entries();
        self.note(format!(
            "open presets popup {request:?} ({}): the menu is not implemented yet",
            entries.join("/")
        ));
        entries
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

    /// The single pump step: drain commands, poll the engine, drain events and
    /// refresh the UI's time-dependent parts.
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

        self.ui.refresh(self.now_ms());
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

    /// Applies one external command (see [`App::submit`]).
    pub fn apply(&mut self, command: &Command) {
        self.apply_from(command, false);
    }

    /// Applies one command to the engine, to the app when the app owns it, and
    /// to the UI machine unless the UI produced it (the UI machine already
    /// applied its own S02 prediction for its own commands).
    fn apply_from(&mut self, command: &Command, from_ui: bool) {
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
    }

    /// Applies a core [`Event`]: the app's own bookkeeping first, then the UI
    /// machine (the authoritative state).
    pub fn handle_event(&mut self, event: &Event) {
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
    /// recorded through the dialog-stub path, which opens no window.
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
    fn entry_id_for(&mut self, index: Option<u32>) -> Option<EntryId> {
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
            C::Minimize => self.minimize_requested = true,
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
            C::ToggleWindow(WindowId::Equalizer) => {
                self.note(
                    "Equalizer window toggled (the render state drives the OS window); \
                     the presets submenu contents and load/save semantics are not \
                     implemented yet (S07 §12 O12, S04)"
                        .to_string(),
                );
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
            C::MoveEntries { .. } | C::Sort(_) | C::Randomize | C::Reverse => {
                self.note(
                    "the playlist reordering is not wired to the app yet (S14/S15)".to_string(),
                );
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
            C::FocusNextWindow | C::ShowMainWindow => {
                self.note(format!("{command:?} is not implemented yet"));
            }
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

    fn add_items(&mut self, items: &[MediaLocation], at: Option<u32>, play: bool) {
        let first_new = self.engine.queue().len() as u32;
        let mut queue = self.engine.queue().to_vec();
        queue.extend(items.iter().map(|location| match location {
            MediaLocation::File(path) => TrackSource::path(path.clone()),
            MediaLocation::Url(url) => TrackSource::Url(url.clone()),
        }));
        self.engine.set_queue(queue);
        self.sync_entry_ids();
        if let Some(at) = at
            && at < first_new
        {
            self.note(format!(
                "AddItems: insert position {at} is not supported yet; the entries were appended"
            ));
        }
        if play
            && !items.is_empty()
            && let Err(error) = self.engine.play_index(first_new)
        {
            self.publish_error(&error);
        }
    }

    fn play_entry(&mut self, id: EntryId) {
        if let Some(index) = self.entry_ids.iter().position(|entry| *entry == id) {
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
        self.sync_playlist_entries();
    }

    fn clear_entries(&mut self) {
        if self.engine.transport() != TransportState::Stopped {
            let _ = self.engine.stop();
        }
        self.entry_ids.clear();
        self.engine.set_queue(Vec::new());
        self.sync_playlist_entries();
    }
}

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
        // spec fixes the title only, so the app opens the real window and
        // implements no dialog behaviour beyond it — in particular no note.
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
    fn presets_in_the_eq_window_records_the_menu_request_and_opens_nothing() {
        // S07 §4.2: the spec fixes the popup (class, size, Load/Save/Delete)
        // and nothing beyond it, so the app records the request and opens no
        // window.
        let (mut app, _dir) = app_with_file(None);
        assert!(app.eq_mouse_down(239, 24).unwrap().is_empty());
        assert!(app.eq_mouse_up(239, 24).unwrap().is_empty());
        let notes = app.drain_notes();
        assert!(
            notes.iter().any(|n| n.contains("Load/Save/Delete")),
            "{notes:?}"
        );
        assert_eq!(
            app.request_eq_presets_menu(rustamp_ui::EqMenuRequest::Presets),
            &["Load", "Save", "Delete"]
        );
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
        // RFC 0010: no confirmation semantics. Enter is consumed and changes
        // nothing; Escape closes; the focused-window close does too.
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
}
