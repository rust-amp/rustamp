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
    Key, MainWindow, MainWindowLayout, MainWindowState, Modifiers, ShortcutOutcome, frame_size,
    render_main_window, volume_percent,
};

use crate::cli::OutputChoice;
use crate::host::{HostStateLink, PublishOutcome, player_state_message};
use crate::routing;
use crate::skin::{LoadedSkin, SkinChoice, load as load_skin};

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
    skin: LoadedSkin,
    config: Config,

    bus: EventBus,
    events: EventReceiver,
    commands: CommandSender,
    command_rx: CommandReceiver,

    /// The virtual clock, when the null output is in use.
    clock: Option<NullClock>,
    started: Instant,
    seconds: Option<u64>,

    // ----- application-level state (not the engine's, not the UI machine's) -----
    shuffle: bool,
    repeat: bool,
    always_on_top: bool,
    easy_move: bool,
    focused: bool,
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
    entry_ids: Vec<EntryId>,
    next_entry_id: u64,
    notes: Vec<String>,
    /// The app's end of the player-state bridge to the plugin host
    /// (see [`crate::host`]): detached until a supervisor attaches a
    /// control-channel transport, so publishing without a host is a no-op.
    host: HostStateLink,
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
            OutputChoice::Null => {
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
            skin: options.skin,
            config: options.config,
            bus,
            events,
            commands,
            command_rx,
            clock,
            started: Instant::now(),
            seconds: options.seconds,
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
            notes: Vec::new(),
            host: HostStateLink::detached(),
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

    /// Whether the app was started in the headless (null-output) mode.
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

    /// Takes the pending "minimize the main window" request, if any.
    pub fn take_minimize_request(&mut self) -> bool {
        std::mem::take(&mut self.minimize_requested)
    }

    /// Drains the accumulated notes (unimplemented commands, fallbacks).
    pub fn drain_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
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
        // Mirror the player state to the plugin host (see `crate::host`):
        // best-effort by contract, so the outcome is ignored — a detached or
        // dead host neither fails nor blocks the pump.
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
    /// and the stable entry ids (the engine reports `None`). The equalizer is
    /// the flat default until S07/M2 gives the engine a stage (finding F1 in
    /// [`crate::host`]).
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
    /// Process supervision is a later card; until it lands the link stays
    /// detached.
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
    // Headless playback (the `--null` mode)
    // -----------------------------------------------------------------------

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
    fn forward(&mut self, commands: &[Command]) -> Result<()> {
        for command in commands {
            self.apply_from(command, true);
        }
        self.pump()
    }

    /// Grows the app's entry-id table to the engine queue's length.
    fn sync_entry_ids(&mut self) {
        let len = self.engine.queue().len();
        while self.entry_ids.len() < len {
            let id = EntryId(self.next_entry_id);
            self.next_entry_id += 1;
            self.entry_ids.push(id);
        }
    }

    /// The stable id of queue entry `index`, allocating one if it is new.
    fn entry_id_for(&mut self, index: Option<u32>) -> Option<EntryId> {
        let index = index? as usize;
        self.sync_entry_ids();
        while self.entry_ids.len() <= index {
            let id = EntryId(self.next_entry_id);
            self.next_entry_id += 1;
            self.entry_ids.push(id);
        }
        self.entry_ids.get(index).copied()
    }

    fn note(&mut self, message: String) {
        if self.notes.len() < MAX_NOTES {
            self.notes.push(message);
        }
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
            C::CloseFocusedWindow => {
                self.quit = true;
                self.bus.publish(Event::Quit);
            }
            C::ToggleDoubleSize | C::ToggleWindowShade(WindowId::Main) => {
                // The UI machine has already applied its S02 prediction; the
                // presenter reads the new size from `frame_size`.
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
                self.note("the equalizer is not implemented yet (S07/M2)".to_string());
            }
            C::Remove(_)
            | C::Clear
            | C::MoveEntries { .. }
            | C::Select(_)
            | C::PlayEntry(_)
            | C::Sort(_)
            | C::Randomize
            | C::Reverse => {
                self.note(
                    "the playlist model is not wired to the app yet (S06/S14/S15)".to_string(),
                );
            }
            C::OpenPrefs
            | C::OpenPrefsPage(_)
            | C::OpenMedia { .. }
            | C::OpenJump { .. }
            | C::OpenFileInfo
            | C::OpenAbout
            | C::OpenSkinBrowser
            | C::OpenBookmarkEditor
            | C::AddBookmark
            | C::OpenMainMenu => {
                self.note(format!(
                    "{command:?} needs a dialog that is not implemented yet"
                ));
            }
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
}
