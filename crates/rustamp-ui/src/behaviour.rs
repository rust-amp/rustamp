//! Headless main-window behaviour (task card D2, spec **S02**).
//!
//! [`MainWindow`] is the classic main window's state machine. It is deliberately
//! **headless**: it uses no `winit`, no `softbuffer` and no wall-clock call. Time
//! arrives as a parameter in milliseconds, so the tests are deterministic and
//! the presenter (which may read the clock) supplies it.
//!
//! The machine consumes the core contracts — [`Command`], [`Event`],
//! [`TransportState`], [`StreamInfo`] and the volume/balance units — and emits
//! its intent as [`Command`]s. It never calls the audio engine.
//!
//! Every rule here is traceable to an S02 row; the few choices S02 leaves open
//! (the display-title format, the hint's time/percentage format, the slider
//! click mapping, the marquee drag ratio and the window-button window positions)
//! are marked **provisional** in the code.
//! The built-in analyser is separately Rustamp's **own choice**, with reference
//! grade **open**, in [`crate::visualization`]; it is not an S02 measurement.

use rustamp_core::command::{Command, OpenMediaKind};
use rustamp_core::event::Event;
use rustamp_core::meta::TrackMetadata;
use rustamp_core::state::{StreamInfo, TransportState, WindowId};

use crate::layout::{BALANCE_THUMB_SIZE, MainWindowLayout, VOLUME_THUMB_SIZE};
use crate::state::{MainWindowState, MonoStereo};
use crate::visualization::{VisError, VisMode, Visualizer};
use crate::widget::{Widget, hit_test};
use rustamp_core::vis::VisFrame;

/// The program name the marquee shows when nothing is loaded.
///
/// S02 §3 records Winamp's program name as `WINAMP 2.95`; Rustamp substitutes
/// its own name. The *behaviour* (show the program name when nothing is loaded)
/// is the S02 row.
pub const PROGRAM_NAME: &str = "RUSTAMP";

/// The marquee separator between two copies of the title (S02 §3): three
/// asterisk glyphs with two blank cells on each side.
const MARQUEE_SEPARATOR: &str = "  ***  ";

/// The separator's width in character cells (S02 §3).
const MARQUEE_SEPARATOR_CELLS: usize = 7;

/// The `text.bmp` character cell width in pixels (S01 §2.2).
const MARQUEE_CELL_WIDTH: u32 = 5;

/// The marquee advances one cell every this many milliseconds (S02 §3).
const MARQUEE_STEP_MS: u64 = 200;

/// The marquee advances this many pixels per step (S02 §3).
const MARQUEE_STEP_PX: i32 = 5;

/// The pause-blink period in milliseconds (S02 §4: about 2.60 s).
const BLINK_PERIOD_MS: u64 = 2600;

/// The visible/blank half-period of the pause blink (S02 §4: about 1.30 s).
const BLINK_HALF_MS: u64 = 1300;

/// The S02 §1 transport transition for `command`, or `None` when the command
/// does not belong to the transport state machine.
///
/// `Some(from)` means "the command does not change the transport state": S02 §1
/// records that previous/next and seeking while **stopped** leave it stopped.
pub fn transport_transition(from: TransportState, command: &Command) -> Option<TransportState> {
    match command {
        Command::Play => Some(TransportState::Playing),
        Command::Pause | Command::TogglePause => match from {
            TransportState::Playing => Some(TransportState::Paused),
            TransportState::Paused => Some(TransportState::Playing),
            // S02 §1 lists no transition from stopped for pause.
            TransportState::Stopped => None,
        },
        Command::Stop | Command::StopWithFadeout => Some(TransportState::Stopped),
        // Playing an entry starts playback (RFC 0005 §5.1/§5.5; owner decision
        // O1: start and end of list play).
        Command::PlayIndex(_) | Command::PlayEntry(_) | Command::PlayLast => {
            Some(TransportState::Playing)
        }
        // Previous/next change the playlist entry but not the transport state;
        // seeking while stopped has no effect on it (S02 §1). `SkipBy` is the
        // same kind of move (RFC 0005 §5.1).
        Command::Next
        | Command::Prev
        | Command::SkipBy { .. }
        | Command::Seek { .. }
        | Command::SeekRelative { .. } => Some(from),
        _ => None,
    }
}

/// The main window's headless behaviour.
#[derive(Debug, Clone)]
pub struct MainWindow {
    layout: MainWindowLayout,
    state: MainWindowState,
    stream: Option<StreamInfo>,
    duration_ms: Option<u64>,
    position_ms: u64,
    track_title: Option<String>,
    always_on_top: bool,
    held: Option<Widget>,
    drag: Option<Widget>,
    drag_last_x: Option<i32>,
    autoscroll: bool,
    scroll_base_ms: u64,
    manual_offset: i32,
    blink_base_ms: Option<u64>,
    visualization: Visualizer,
}

impl Default for MainWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl MainWindow {
    /// A new machine with the default state (nothing loaded, stopped).
    pub fn new() -> Self {
        Self::from_state(MainWindowState::default())
    }

    /// A machine starting from a render state.
    ///
    /// The render state is the *drawing* state; the machine takes the values it
    /// can (transport, toggles, sliders, title) and recomputes the derived
    /// fields on the first [`MainWindow::refresh`].
    pub fn from_state(state: MainWindowState) -> Self {
        let track_title = if state.title.is_empty() {
            None
        } else {
            Some(state.title.clone())
        };
        let mut machine = Self {
            layout: MainWindowLayout::CLASSIC,
            state,
            stream: None,
            duration_ms: None,
            position_ms: 0,
            track_title,
            always_on_top: false,
            held: None,
            drag: None,
            drag_last_x: None,
            autoscroll: true,
            scroll_base_ms: 0,
            manual_offset: 0,
            blink_base_ms: None,
            visualization: Visualizer::default(),
        };
        machine.always_on_top = machine.state.clutter_a;
        machine
            .visualization
            .set_mode(machine.state.visualization.mode);
        machine.refresh(0);
        machine
    }

    /// The current render state (last refreshed).
    pub fn state(&self) -> &MainWindowState {
        &self.state
    }

    /// The layout the machine hit-tests against.
    pub fn layout(&self) -> &MainWindowLayout {
        &self.layout
    }

    /// Sets whether the window is focused (drives the title-bar focus variant).
    pub fn set_focused(&mut self, focused: bool) {
        self.state.title_bar_focused = focused;
    }

    /// Feed the newest **actual** engine tap frame and its unrounded source
    /// sample rate. No PCM is invented when the caller has no frame/rate.
    ///
    /// The app must drain `Engine::take_vis_frame()` in its pump, after transport
    /// and stream events. This UI-only method does not depend on the engine.
    /// Decode-ahead selection/latency and all reference S08 laws remain open.
    ///
    /// A returned [`VisError`] is an analysis report, never a playback error:
    /// [`VisError::NonFinitePcm`] is an offline validation invariant that the
    /// live path cannot reach (RFC 0014 §4.5 sanitises the samples first), so a
    /// caller only notes it and keeps playing. It must not stop, pause or reset
    /// the transport.
    pub fn push_vis_frame(
        &mut self,
        frame: &VisFrame,
        sample_rate_hz: u32,
        now_ms: u64,
    ) -> Result<(), VisError> {
        let result = self
            .visualization
            .push(frame, sample_rate_hz, now_ms, self.state.transport);
        self.state.visualization = self.visualization.state().clone();
        result
    }

    /// UI-local mode selection; no plugin mode or persistence is implied.
    pub fn set_vis_mode(&mut self, mode: VisMode) {
        self.visualization.set_mode(mode);
        self.state.visualization = self.visualization.state().clone();
    }

    /// Discard analyser history after an externally initiated seek/discontinuity.
    pub fn reset_visualization(&mut self) {
        self.visualization.reset();
        self.state.visualization = self.visualization.state().clone();
    }

    /// Whether the marquee autoscrolls (S02 §3).
    pub fn autoscroll(&self) -> bool {
        self.autoscroll
    }

    /// The logical marquee text without the separator copy: the hint while a
    /// control is held, else the current entry's display title, else the
    /// program name (S02 §2.1/§3).
    pub fn logical_title(&self) -> String {
        match self.hint() {
            Some(hint) => hint,
            None => self
                .track_title
                .clone()
                .unwrap_or_else(|| PROGRAM_NAME.to_string()),
        }
    }

    /// The hint shown while a control is held, if that control has one (S02
    /// §2.1). Transport, shuffle/repeat/EQ/playlist and the time display have
    /// **no** hint (S02 §2.1, verified).
    pub fn hint(&self) -> Option<String> {
        match self.held? {
            Widget::ClutterO => Some("OPTIONS MENU".to_string()),
            Widget::ClutterA => Some("ENABLE ALWAYS-ON-TOP".to_string()),
            Widget::ClutterI => Some("FILE INFO BOX".to_string()),
            Widget::ClutterD => Some("ENABLE DOUBLESIZE MODE".to_string()),
            Widget::ClutterV => Some("VISUALIZATION MENU".to_string()),
            Widget::PositionSlider => Some(format!(
                "SEEK TO: {}/{} ({}%)",
                format_time_ms(self.position_ms),
                format_time_ms(self.duration_ms.unwrap_or(0)),
                self.state.position
            )),
            Widget::VolumeSlider => Some(format!("VOLUME: {}%", self.state.volume)),
            Widget::BalanceSlider => Some(format!("BALANCE: {}", balance_side(self.state.balance))),
            _ => None,
        }
    }

    /// Toggles the marquee autoscroll (S02 §3, the "Autoscroll songname"
    /// toggle). The toggle itself has no core `Command`; the context menu that
    /// carries it is S04.
    pub fn set_autoscroll(&mut self, on: bool, now_ms: u64) {
        if !on && self.autoscroll {
            self.manual_offset = self.auto_offset(now_ms);
        }
        self.autoscroll = on;
        if on {
            self.scroll_base_ms = now_ms;
            self.manual_offset = 0;
        }
        self.refresh(now_ms);
    }

    /// Recomputes the time-dependent parts of the render state (time display,
    /// marquee, held-button sprites). Call once per frame with the presenter's
    /// clock.
    pub fn refresh(&mut self, now_ms: u64) {
        self.update_stream();
        self.update_time(now_ms);
        self.update_marquee(now_ms);
        self.update_held();
        self.visualization.advance(now_ms, self.state.transport);
        self.state.visualization = self.visualization.state().clone();
    }

    /// Applies a core [`Event`] (the authoritative source of player state).
    pub fn handle_event(&mut self, event: &Event, now_ms: u64) {
        match event {
            Event::StateChanged(transport) => self.set_transport(*transport, now_ms),
            Event::TrackChanged {
                id,
                index,
                metadata,
                duration_ms,
            } => {
                self.reset_visualization();
                self.duration_ms = *duration_ms;
                self.position_ms = 0;
                self.track_title = if id.is_some() {
                    Some(format_display_title(
                        *index,
                        metadata.as_deref(),
                        *duration_ms,
                    ))
                } else {
                    None
                };
            }
            Event::PositionChanged {
                position_ms,
                duration_ms,
            } => {
                self.position_ms = *position_ms;
                if duration_ms.is_some() {
                    self.duration_ms = *duration_ms;
                }
            }
            Event::StreamInfoChanged(info) => self.stream = *info,
            Event::VolumeChanged { volume, balance } => {
                self.state.volume = volume_percent(*volume);
                self.state.balance = balance_percent(*balance);
            }
            Event::ModesChanged { shuffle, repeat } => {
                self.state.shuffle = *shuffle;
                self.state.repeat = *repeat;
            }
            Event::WindowVisibilityChanged { window, visible } => match window {
                WindowId::Equalizer => self.state.eq_open = *visible,
                WindowId::Playlist => self.state.playlist_open = *visible,
                _ => {}
            },
            _ => {}
        }
        self.refresh(now_ms);
    }

    /// Applies a [`Command`]'s modelled S02 effect locally.
    ///
    /// This is the UI's optimistic prediction: the core remains authoritative
    /// and confirms through [`Event`]s. It is what makes the machine "driven by
    /// `Command`s and `Event`s" (task card D2).
    pub fn handle_command(&mut self, command: &Command, now_ms: u64) {
        self.apply_command(command, now_ms);
        self.refresh(now_ms);
    }

    /// Toggles the elapsed/remaining time display (S02 §4, S03 §1 `Ctrl+T`).
    ///
    /// UI-local: there is no core command for it.
    pub fn toggle_time_remaining(&mut self, now_ms: u64) {
        self.state.time_remaining = !self.state.time_remaining;
        self.refresh(now_ms);
    }

    /// A press at window-relative `(x, y)`. Returns the commands the press
    /// produces (sliders jump on press).
    pub fn mouse_down(&mut self, x: i32, y: i32, now_ms: u64) -> Vec<Command> {
        let widget = hit_test(&self.layout, x, y);
        self.held = widget;
        self.drag = None;
        self.drag_last_x = None;
        let mut commands = Vec::new();
        match widget {
            Some(widget) if widget.is_slider() => {
                self.drag = Some(widget);
                self.set_slider(widget, x);
                commands.extend(self.slider_command(widget));
            }
            Some(Widget::Marquee) if !self.autoscroll => {
                // Scroll-on-drag is only active with autoscroll off (S02 §3).
                self.drag = Some(Widget::Marquee);
                self.drag_last_x = Some(x);
            }
            _ => {}
        }
        self.apply_all(&commands, now_ms);
        self.refresh(now_ms);
        commands
    }

    /// A pointer move. While a slider is dragged the value is clamped to the
    /// track; while the marquee is dragged the text scrolls by hand.
    pub fn mouse_move(&mut self, x: i32, y: i32, now_ms: u64) -> Vec<Command> {
        let mut commands = Vec::new();
        match self.drag {
            Some(widget) if widget.is_slider() => {
                self.set_slider(widget, x);
                commands.extend(self.slider_command(widget));
            }
            Some(Widget::Marquee) => {
                if let Some(last) = self.drag_last_x {
                    // Dragging left advances the visible text (S02 §3). The
                    // drag-distance ratio is not determined; this uses one
                    // pixel per pixel (provisional).
                    self.manual_offset = (self.manual_offset + (last - x)).max(0);
                    self.drag_last_x = Some(x);
                }
            }
            _ => {}
        }
        let _ = y;
        self.apply_all(&commands, now_ms);
        self.refresh(now_ms);
        commands
    }

    /// A release at window-relative `(x, y)`. A button fires when the pointer
    /// is released inside it; a slider has already fired on press/drag.
    pub fn mouse_up(&mut self, x: i32, y: i32, now_ms: u64) -> Vec<Command> {
        let mut commands = Vec::new();
        if self.drag.is_none()
            && let Some(held) = self.held
            && hit_test(&self.layout, x, y) == Some(held)
        {
            commands.extend(self.release_command(held));
        }
        self.held = None;
        self.drag = None;
        self.drag_last_x = None;
        self.apply_all(&commands, now_ms);
        self.refresh(now_ms);
        commands
    }

    // ----- internals -----

    fn apply_all(&mut self, commands: &[Command], now_ms: u64) {
        for command in commands {
            self.apply_command(command, now_ms);
        }
    }

    fn set_transport(&mut self, transport: TransportState, now_ms: u64) {
        if transport == TransportState::Paused && self.state.transport != TransportState::Paused {
            self.blink_base_ms = Some(now_ms);
        }
        if transport != TransportState::Paused {
            self.blink_base_ms = None;
        }
        self.state.transport = transport;
    }

    fn apply_command(&mut self, command: &Command, now_ms: u64) {
        if let Some(transport) = transport_transition(self.state.transport, command) {
            self.set_transport(transport, now_ms);
        }
        match command {
            Command::Seek { to_ms } => {
                self.reset_visualization();
                if self.state.transport != TransportState::Stopped {
                    self.position_ms = clamp_position(*to_ms, self.duration_ms);
                }
            }
            Command::SeekRelative { delta_ms } => {
                self.reset_visualization();
                if self.state.transport != TransportState::Stopped {
                    let target = (self.position_ms as i64 + delta_ms).max(0) as u64;
                    self.position_ms = clamp_position(target, self.duration_ms);
                }
            }
            Command::SetVolume(volume) => self.state.volume = volume_percent(*volume),
            Command::SetBalance(balance) => self.state.balance = balance_percent(*balance),
            Command::ToggleShuffle => self.state.shuffle = !self.state.shuffle,
            Command::SetShuffle(on) => self.state.shuffle = *on,
            Command::ToggleRepeat => self.state.repeat = !self.state.repeat,
            Command::SetRepeat(on) => self.state.repeat = *on,
            Command::ToggleDoubleSize => self.state.double_size = !self.state.double_size,
            Command::ToggleAlwaysOnTop => self.always_on_top = !self.always_on_top,
            Command::ToggleWindow(WindowId::Equalizer) => self.state.eq_open = !self.state.eq_open,
            Command::ToggleWindow(WindowId::Playlist) => {
                self.state.playlist_open = !self.state.playlist_open;
            }
            Command::ToggleWindowShade(WindowId::Main) => {
                self.state.windowshade = !self.state.windowshade;
            }
            _ => {}
        }
    }

    /// The command a button produces when it is released, if any.
    ///
    /// Eject opens files (S03 §5 "Click the eject button: open file(s)",
    /// id 40029, which RFC 0005 maps to `OpenMedia { kind: Files, play: true }`);
    /// its Shift+click (open directory) needs modifier-aware pointer input,
    /// which this machine does not receive, and stays a gap. The four window
    /// buttons act per S03 §5 (main menu / minimize / windowshade / close):
    /// the shade button's effect is measured (S05 §1.3), close quits because
    /// this is the main window (RFC 0005 O3). The clutter-bar O and V emit
    /// `OpenOptionsMenu` / `OpenVisualizationMenu` (RFC 0016, S02 §2); the
    /// app does not render those popups yet.
    fn release_command(&mut self, widget: Widget) -> Option<Command> {
        match widget {
            Widget::Previous => Some(Command::Prev),
            Widget::Play => Some(Command::Play),
            Widget::Pause => Some(Command::TogglePause),
            Widget::Stop => Some(Command::Stop),
            Widget::Next => Some(Command::Next),
            Widget::Eject => Some(Command::OpenMedia {
                kind: OpenMediaKind::Files,
                play: true,
            }),
            Widget::WindowMinimize => Some(Command::Minimize),
            Widget::WindowShade => Some(Command::ToggleWindowShade(WindowId::Main)),
            Widget::WindowClose => Some(Command::Quit),
            Widget::Shuffle => Some(Command::ToggleShuffle),
            Widget::Repeat => Some(Command::ToggleRepeat),
            Widget::EqToggle => Some(Command::ToggleWindow(WindowId::Equalizer)),
            Widget::PlaylistToggle => Some(Command::ToggleWindow(WindowId::Playlist)),
            Widget::ClutterA => Some(Command::ToggleAlwaysOnTop),
            Widget::ClutterD => Some(Command::ToggleDoubleSize),
            Widget::ClutterO => Some(Command::OpenOptionsMenu),
            Widget::ClutterV => Some(Command::OpenVisualizationMenu),
            Widget::ClutterI => {
                if self.track_title.is_some() {
                    Some(Command::OpenFileInfo)
                } else {
                    None
                }
            }
            // The window-menu button opens the main-menu popup (S04 §1 row 1:
            // posted left click at client (6,3)); the popup itself is the
            // app presenter's window, opened from this command.
            Widget::WindowMenu => Some(Command::OpenMainMenu),
            Widget::TimeDisplay => {
                // Clicking the time display toggles elapsed/remaining (S02 §4).
                // There is no core command or persisted field for it, so it is
                // UI-local (reported gap).
                self.state.time_remaining = !self.state.time_remaining;
                None
            }
            Widget::Visualization => {
                // Own choice only: Q9's switching gesture/styles are unmeasured.
                self.set_vis_mode(self.state.visualization.mode.next());
                None
            }
            _ => None,
        }
    }

    fn set_slider(&mut self, widget: Widget, x: i32) {
        let percent = self.slider_percent(widget, x);
        match widget {
            Widget::PositionSlider => self.state.position = percent,
            Widget::VolumeSlider => self.state.volume = percent,
            Widget::BalanceSlider => self.state.balance = percent,
            _ => {}
        }
    }

    /// The click/drag-to-percentage mapping for a slider.
    ///
    /// **Provisional:** S02 does not give the mapping. The cursor sets the
    /// thumb's centre, using each track's S01 geometry.
    fn slider_percent(&self, widget: Widget, x: i32) -> u8 {
        let (track, thumb_width, inset) = match widget {
            Widget::PositionSlider => (
                self.layout.position_track,
                self.layout.position_thumb.width,
                0,
            ),
            Widget::VolumeSlider => (self.layout.volume_track, VOLUME_THUMB_SIZE.0, 1),
            Widget::BalanceSlider => (self.layout.balance_track, BALANCE_THUMB_SIZE.0, 0),
            _ => return 0,
        };
        let usable = track
            .width
            .saturating_sub(thumb_width)
            .saturating_sub(inset) as i32;
        if usable <= 0 {
            return 0;
        }
        let local = x - track.x - inset as i32 - thumb_width as i32 / 2;
        let percent = (local * 100 + usable / 2).div_euclid(usable);
        percent.clamp(0, 100) as u8
    }

    fn slider_command(&self, widget: Widget) -> Option<Command> {
        match widget {
            Widget::PositionSlider => self.duration_ms.map(|duration| Command::Seek {
                to_ms: (u128::from(duration) * u128::from(self.state.position) / 100) as u64,
            }),
            Widget::VolumeSlider => Some(Command::SetVolume(core_volume(self.state.volume))),
            Widget::BalanceSlider => Some(Command::SetBalance(core_balance(self.state.balance))),
            _ => None,
        }
    }

    fn update_stream(&mut self) {
        match self.stream {
            Some(info) => {
                self.state.kbps = info.bitrate_kbps;
                self.state.khz = Some(sample_rate_khz(info.sample_rate_hz));
                self.state.channels = if info.channels == 1 {
                    MonoStereo::Mono
                } else {
                    MonoStereo::Stereo
                };
            }
            None => {
                self.state.kbps = None;
                self.state.khz = None;
                self.state.channels = MonoStereo::Unknown;
            }
        }
        // S02 §5: stopped shows a blank bitrate/sample-rate readout.
        if self.state.transport == TransportState::Stopped {
            self.state.kbps = None;
            self.state.khz = None;
        }
    }

    fn update_time(&mut self, now_ms: u64) {
        if self.state.transport == TransportState::Stopped {
            // S02 §4: stopped (and nothing loaded) shows a blank display.
            self.state.time_visible = false;
            self.state.time_seconds = 0;
            return;
        }
        self.state.time_seconds = if self.state.time_remaining {
            (self
                .duration_ms
                .unwrap_or(0)
                .saturating_sub(self.position_ms)
                / 1000) as u32
        } else {
            (self.position_ms / 1000) as u32
        };
        self.state.time_visible = if self.state.transport == TransportState::Paused {
            let base = *self.blink_base_ms.get_or_insert(now_ms);
            (now_ms.saturating_sub(base) % BLINK_PERIOD_MS) < BLINK_HALF_MS
        } else {
            true
        };
    }

    fn update_held(&mut self) {
        self.state.pressed = self.held.and_then(Widget::transport_button);
        self.state.pressed_window_button = self.held.and_then(Widget::window_button);
        self.state.pressed_widget = self.held;
        self.state.position_thumb_pressed = self.held == Some(Widget::PositionSlider);
        // Momentary letters light while held; the latched A/D letters stay lit
        // while their option is on (S02 §2).
        self.state.clutter_o = self.held == Some(Widget::ClutterO);
        self.state.clutter_i = self.held == Some(Widget::ClutterI);
        self.state.clutter_v = self.held == Some(Widget::ClutterV);
        self.state.clutter_a = self.always_on_top || self.held == Some(Widget::ClutterA);
        self.state.clutter_d = self.state.double_size || self.held == Some(Widget::ClutterD);
    }

    fn update_marquee(&mut self, now_ms: u64) {
        let hint = self.hint();
        let is_hint = hint.is_some();
        let text = hint.unwrap_or_else(|| {
            self.track_title
                .clone()
                .unwrap_or_else(|| PROGRAM_NAME.to_string())
        });
        let cells = text.chars().count();
        let fits = cells as u32 * MARQUEE_CELL_WIDTH <= self.layout.marquee.width;
        if is_hint || fits {
            self.state.title = text;
            self.state.marquee_offset = 0;
            return;
        }
        let period_px = (cells + MARQUEE_SEPARATOR_CELLS) as i32 * MARQUEE_CELL_WIDTH as i32;
        let offset = if self.autoscroll {
            self.auto_offset_for(cells, now_ms)
        } else {
            self.manual_offset % period_px
        };
        self.state.title = format!("{text}{MARQUEE_SEPARATOR}{text}");
        self.state.marquee_offset = offset;
    }

    /// The autoscroll offset in pixels for a title of `cells` characters.
    fn auto_offset_for(&self, cells: usize, now_ms: u64) -> i32 {
        let period_px = (cells + MARQUEE_SEPARATOR_CELLS) as i32 * MARQUEE_CELL_WIDTH as i32;
        let elapsed = now_ms.saturating_sub(self.scroll_base_ms);
        let steps = (elapsed / MARQUEE_STEP_MS) as i32;
        (steps * MARQUEE_STEP_PX) % period_px
    }

    fn auto_offset(&self, now_ms: u64) -> i32 {
        let text = self.logical_title();
        let cells = text.chars().count();
        if cells as u32 * MARQUEE_CELL_WIDTH <= self.layout.marquee.width {
            return 0;
        }
        self.auto_offset_for(cells, now_ms)
    }
}

/// Clamps a seek target to `0..=duration` when the duration is known.
fn clamp_position(to_ms: u64, duration_ms: Option<u64>) -> u64 {
    match duration_ms {
        Some(duration) => to_ms.min(duration),
        None => to_ms,
    }
}

/// The `kbps`/`kHz` sample-rate readout: `44100` -> `44` (S02 §5).
fn sample_rate_khz(hz: u32) -> u32 {
    (hz + 500) / 1000
}

/// A core volume (`0..=255`) as the display percentage (`0..=100`).
pub fn volume_percent(volume: u8) -> u8 {
    ((u32::from(volume) * 100 + 127) / 255).min(100) as u8
}

/// A display percentage (`0..=100`) as a core volume (`0..=255`).
pub fn core_volume(percent: u8) -> u8 {
    ((u32::from(percent.min(100)) * 255 + 50) / 100).min(255) as u8
}

/// A core balance (`-127..=127`) as the display percentage (50 is centred).
pub fn balance_percent(balance: i8) -> u8 {
    let delta = i32::from(balance);
    let rounded = (delta * 50 + if delta >= 0 { 63 } else { -63 }) / 127;
    (50 + rounded).clamp(0, 100) as u8
}

/// A display percentage (50 is centred) as a core balance (`-127..=127`).
pub fn core_balance(percent: u8) -> i8 {
    let delta = i32::from(percent.min(100)) - 50;
    let rounded = (delta * 127 + if delta >= 0 { 25 } else { -25 }) / 50;
    rounded.clamp(-127, 127) as i8
}

/// The balance hint's `<CENTER or side>` (S02 §2.1).
///
/// **Provisional:** S02 records only the shape; this renders `CENTER`, `LEFT`
/// or `RIGHT`.
fn balance_side(percent: u8) -> &'static str {
    match percent {
        50 => "CENTER",
        p if p < 50 => "LEFT",
        _ => "RIGHT",
    }
}

/// `m:ss` for a millisecond value.
fn format_time_ms(ms: u64) -> String {
    let total = ms / 1000;
    format!("{}:{:02}", total / 60, total % 60)
}

/// The current entry's display title (S02 §3.2): `<position>. <name> (<length>)`.
///
/// **Provisional:** S02 gives the shape and says the exact formatting is S14;
/// this is the shape with the bitmap font's capitals.
fn format_display_title(
    index: Option<u32>,
    metadata: Option<&TrackMetadata>,
    duration_ms: Option<u64>,
) -> String {
    let position = index.map(|i| i + 1).unwrap_or(0);
    let name = metadata
        .and_then(TrackMetadata::display_title)
        .unwrap_or_default();
    match duration_ms {
        Some(ms) => format!("{position}. {name} ({})", format_time_ms(ms)),
        None => format!("{position}. {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{TransportButton, WindowButton};
    use rustamp_core::playlist::EntryId;

    fn layout() -> MainWindowLayout {
        MainWindowLayout::CLASSIC
    }

    fn centre(widget: Widget) -> (i32, i32) {
        let r = widget.rect(&layout());
        (r.x + r.width as i32 / 2, r.y + r.height as i32 / 2)
    }

    /// Press and release a widget at its centre, returning the commands.
    fn click(machine: &mut MainWindow, widget: Widget, now_ms: u64) -> Vec<Command> {
        let (x, y) = centre(widget);
        let mut commands = machine.mouse_down(x, y, now_ms);
        commands.extend(machine.mouse_up(x, y, now_ms));
        commands
    }

    fn playing(machine: &mut MainWindow, now_ms: u64) {
        machine.handle_event(&Event::StateChanged(TransportState::Playing), now_ms);
    }

    fn load_track(machine: &mut MainWindow, title: &str, duration_ms: u64, now_ms: u64) {
        machine.handle_event(
            &Event::TrackChanged {
                id: Some(EntryId(1)),
                index: Some(0),
                metadata: Some(Box::new(TrackMetadata {
                    title: Some(title.to_string()),
                    ..TrackMetadata::default()
                })),
                duration_ms: Some(duration_ms),
            },
            now_ms,
        );
    }

    // ----- transport state machine (S02 §1) -----

    #[test]
    fn transport_transitions_follow_s02() {
        use TransportState::{Paused, Playing, Stopped};
        let cases = [
            (Stopped, Command::Play, Some(Playing)),
            (Paused, Command::Play, Some(Playing)),
            (Playing, Command::Pause, Some(Paused)),
            (Paused, Command::Pause, Some(Playing)),
            (Playing, Command::TogglePause, Some(Paused)),
            (Stopped, Command::Stop, Some(Stopped)),
            (Playing, Command::Stop, Some(Stopped)),
            (Paused, Command::Stop, Some(Stopped)),
            // States that must not change.
            (Stopped, Command::Next, Some(Stopped)),
            (Stopped, Command::Prev, Some(Stopped)),
            (Playing, Command::Next, Some(Playing)),
            (Paused, Command::Prev, Some(Paused)),
            (Stopped, Command::Seek { to_ms: 1000 }, Some(Stopped)),
            // Unrelated commands are not part of the transport machine.
            (Playing, Command::SetVolume(10), None),
        ];
        for (from, command, expected) in cases {
            assert_eq!(
                transport_transition(from, &command),
                expected,
                "{from:?} + {command:?}"
            );
        }
    }

    #[test]
    fn machine_transport_is_driven_by_commands_and_events() {
        let mut machine = MainWindow::new();
        assert_eq!(machine.state().transport, TransportState::Stopped);

        machine.handle_command(&Command::Play, 0);
        assert_eq!(machine.state().transport, TransportState::Playing);

        machine.handle_command(&Command::Pause, 0);
        assert_eq!(machine.state().transport, TransportState::Paused);

        machine.handle_command(&Command::Pause, 0);
        assert_eq!(machine.state().transport, TransportState::Playing);

        // Events are authoritative.
        machine.handle_event(&Event::StateChanged(TransportState::Stopped), 0);
        assert_eq!(machine.state().transport, TransportState::Stopped);

        // Previous/next while stopped leave it stopped (S02 §1).
        machine.handle_command(&Command::Next, 0);
        machine.handle_command(&Command::Prev, 0);
        machine.handle_command(&Command::Seek { to_ms: 5000 }, 0);
        assert_eq!(machine.state().transport, TransportState::Stopped);

        // Pause from stopped is not a transition in S02 §1.
        machine.handle_command(&Command::Pause, 0);
        assert_eq!(machine.state().transport, TransportState::Stopped);
    }

    // ----- mouse dispatch -----

    #[test]
    fn every_interactive_widget_emits_its_command() {
        // A long enough duration so the position slider can seek.
        let cases: [(Widget, &[Command]); 15] = [
            (Widget::Previous, &[Command::Prev]),
            (Widget::Play, &[Command::Play]),
            (Widget::Pause, &[Command::TogglePause]),
            (Widget::Stop, &[Command::Stop]),
            (Widget::Next, &[Command::Next]),
            (Widget::Shuffle, &[Command::ToggleShuffle]),
            (Widget::Repeat, &[Command::ToggleRepeat]),
            (
                Widget::EqToggle,
                &[Command::ToggleWindow(WindowId::Equalizer)],
            ),
            (
                Widget::PlaylistToggle,
                &[Command::ToggleWindow(WindowId::Playlist)],
            ),
            (Widget::ClutterA, &[Command::ToggleAlwaysOnTop]),
            (Widget::ClutterD, &[Command::ToggleDoubleSize]),
            // S03 §5: eject opens file(s) (id 40029, RFC 0005).
            (
                Widget::Eject,
                &[Command::OpenMedia {
                    kind: OpenMediaKind::Files,
                    play: true,
                }],
            ),
            // S03 §5 window buttons; shade measured in S05 §1.3, close of
            // the main window quits (RFC 0005 O3).
            (Widget::WindowMinimize, &[Command::Minimize]),
            (
                Widget::WindowShade,
                &[Command::ToggleWindowShade(WindowId::Main)],
            ),
            (Widget::WindowClose, &[Command::Quit]),
        ];
        for (widget, expected) in cases {
            let mut machine = MainWindow::new();
            playing(&mut machine, 0);
            load_track(&mut machine, "T1", 200_000, 0);
            let commands = click(&mut machine, widget, 0);
            assert_eq!(commands, expected, "{widget:?}");
        }
    }

    #[test]
    fn widgets_without_a_core_command_emit_nothing() {
        for widget in [
            Widget::TitleBar,
            Widget::TimeDisplay,
            Widget::ClutterI,
            Widget::Marquee,
            Widget::Mono,
            Widget::Stereo,
        ] {
            let mut machine = MainWindow::new();
            playing(&mut machine, 0);
            assert_eq!(click(&mut machine, widget, 0), [], "{widget:?}");
        }
    }

    #[test]
    fn clutter_o_and_v_open_their_popups_on_release_inside() {
        // S02 §2: O opens the Options popup, V the visualization menu
        // (RFC 0016).
        for (widget, expected) in [
            (Widget::ClutterO, Command::OpenOptionsMenu),
            (Widget::ClutterV, Command::OpenVisualizationMenu),
        ] {
            let mut machine = MainWindow::new();
            playing(&mut machine, 0);
            assert_eq!(click(&mut machine, widget, 0), [expected], "{widget:?}");
        }
    }

    #[test]
    fn clutter_o_and_v_emit_nothing_when_released_outside() {
        for widget in [Widget::ClutterO, Widget::ClutterV] {
            let mut machine = MainWindow::new();
            let (x, y) = centre(widget);
            machine.mouse_down(x, y, 0);
            assert_eq!(machine.mouse_up(200, 100, 0), [], "{widget:?}");
            assert!(!machine.state().clutter_o && !machine.state().clutter_v);
        }
    }

    #[test]
    fn the_window_menu_button_opens_the_main_menu() {
        // S04 §1 row 1: the main menu opens from the window-menu button
        // (client (6,3)); the click emits `OpenMainMenu`, which the app turns
        // into the popup window.
        let mut machine = MainWindow::new();
        let (x, y) = centre(Widget::WindowMenu);
        machine.mouse_down(x, y, 0);
        assert_eq!(machine.mouse_up(x, y, 0), [Command::OpenMainMenu]);
    }

    #[test]
    fn slider_press_emits_the_seek_and_volume_commands() {
        let mut machine = MainWindow::new();
        playing(&mut machine, 0);
        load_track(&mut machine, "T1", 200_000, 0);

        // Press at the left edge of the position track: seek to 0.
        let (x, y) = centre(Widget::PositionSlider);
        let commands = machine.mouse_down(x, y, 0);
        assert_eq!(commands.len(), 1);
        assert!(matches!(commands[0], Command::Seek { .. }), "{commands:?}");
        machine.mouse_up(x, y, 0);

        let (x, y) = centre(Widget::VolumeSlider);
        let commands = machine.mouse_down(x, y, 0);
        assert!(matches!(commands[0], Command::SetVolume(_)), "{commands:?}");
        machine.mouse_up(x, y, 0);

        let (x, y) = centre(Widget::BalanceSlider);
        let commands = machine.mouse_down(x, y, 0);
        assert!(
            matches!(commands[0], Command::SetBalance(_)),
            "{commands:?}"
        );
        machine.mouse_up(x, y, 0);
    }

    #[test]
    fn a_drag_that_leaves_the_slider_is_clamped() {
        let mut machine = MainWindow::new();
        playing(&mut machine, 0);
        load_track(&mut machine, "T1", 200_000, 0);

        let (x, y) = centre(Widget::VolumeSlider);
        machine.mouse_down(x, y, 0);
        let commands = machine.mouse_move(-10_000, y, 0);
        assert_eq!(commands, vec![Command::SetVolume(0)]);
        let commands = machine.mouse_move(10_000, y, 0);
        assert_eq!(commands, vec![Command::SetVolume(255)]);
        machine.mouse_up(x, y, 0);
        assert_eq!(machine.state().volume, 100);

        // The position slider clamps to the track, not to nothing.
        let (x, y) = centre(Widget::PositionSlider);
        machine.mouse_down(x, y, 0);
        let commands = machine.mouse_move(-10_000, y, 0);
        assert_eq!(commands, vec![Command::Seek { to_ms: 0 }]);
        let commands = machine.mouse_move(10_000, y, 0);
        assert_eq!(commands, vec![Command::Seek { to_ms: 200_000 }]);
        machine.mouse_up(x, y, 0);
    }

    #[test]
    fn a_press_outside_any_widget_is_inert() {
        let mut machine = MainWindow::new();
        let commands = machine.mouse_down(5, 100, 0);
        assert_eq!(commands, []);
        assert_eq!(machine.hint(), None);
        assert_eq!(machine.state().pressed, None);
        assert_eq!(machine.mouse_up(5, 100, 0), []);
    }

    #[test]
    fn releasing_outside_the_button_does_not_fire_it() {
        let mut machine = MainWindow::new();
        let (x, y) = centre(Widget::Play);
        machine.mouse_down(x, y, 0);
        assert_eq!(machine.state().pressed, Some(TransportButton::Play));
        let commands = machine.mouse_up(x, y - 40, 0);
        assert_eq!(commands, []);
        assert_eq!(machine.state().pressed, None);
    }

    // ----- clutterbar (S02 §2) -----

    #[test]
    fn the_five_clutterbar_actions_do_their_thing() {
        // O: momentary, opens the Options popup (RFC 0016).
        let mut machine = MainWindow::new();
        let (x, y) = centre(Widget::ClutterO);
        machine.mouse_down(x, y, 0);
        assert!(machine.state().clutter_o);
        assert_eq!(machine.hint().as_deref(), Some("OPTIONS MENU"));
        assert_eq!(machine.mouse_up(x, y, 0), [Command::OpenOptionsMenu]);
        assert!(!machine.state().clutter_o);

        // A: latched always-on-top.
        let mut machine = MainWindow::new();
        let (x, y) = centre(Widget::ClutterA);
        machine.mouse_down(x, y, 0);
        assert!(machine.state().clutter_a);
        assert_eq!(machine.hint().as_deref(), Some("ENABLE ALWAYS-ON-TOP"));
        assert_eq!(machine.mouse_up(x, y, 0), [Command::ToggleAlwaysOnTop]);
        assert!(machine.state().clutter_a);

        // I: momentary file-info (S02 §2: opens file info box if a track is loaded).
        let mut machine = MainWindow::new();
        let (x, y) = centre(Widget::ClutterI);
        machine.mouse_down(x, y, 0);
        assert!(machine.state().clutter_i);
        assert_eq!(machine.hint().as_deref(), Some("FILE INFO BOX"));
        assert_eq!(machine.mouse_up(x, y, 0), []);
        assert!(!machine.state().clutter_i);

        load_track(&mut machine, "T1", 200_000, 0);
        machine.mouse_down(x, y, 0);
        assert_eq!(machine.mouse_up(x, y, 0), [Command::OpenFileInfo]);
        assert!(!machine.state().clutter_i);

        // D: latched double-size.
        let mut machine = MainWindow::new();
        let (x, y) = centre(Widget::ClutterD);
        machine.mouse_down(x, y, 0);
        assert!(machine.state().clutter_d);
        assert_eq!(machine.hint().as_deref(), Some("ENABLE DOUBLESIZE MODE"));
        assert_eq!(machine.mouse_up(x, y, 0), [Command::ToggleDoubleSize]);
        assert!(machine.state().double_size);
        assert!(machine.state().clutter_d);

        // V: momentary, opens the visualization menu (RFC 0016).
        let mut machine = MainWindow::new();
        let (x, y) = centre(Widget::ClutterV);
        machine.mouse_down(x, y, 0);
        assert!(machine.state().clutter_v);
        assert_eq!(machine.hint().as_deref(), Some("VISUALIZATION MENU"));
        assert_eq!(machine.mouse_up(x, y, 0), [Command::OpenVisualizationMenu]);
        assert!(!machine.state().clutter_v);
    }

    // ----- hints (S02 §2.1) -----

    #[test]
    fn hints_are_the_exact_s02_strings_on_the_exact_widgets() {
        // The clutter letters do not change a value, so their hints are exact
        // constants. The sliders jump to the pressed position, so the expected
        // string is the one that press produces.
        let expected = [
            (Widget::ClutterO, "OPTIONS MENU"),
            (Widget::ClutterA, "ENABLE ALWAYS-ON-TOP"),
            (Widget::ClutterI, "FILE INFO BOX"),
            (Widget::ClutterD, "ENABLE DOUBLESIZE MODE"),
            (Widget::ClutterV, "VISUALIZATION MENU"),
            (Widget::PositionSlider, "SEEK TO: 1:40/3:20 (50%)"),
            (Widget::VolumeSlider, "VOLUME: 49%"),
            (Widget::BalanceSlider, "BALANCE: CENTER"),
        ];
        for (widget, hint) in expected {
            let mut machine = MainWindow::new();
            playing(&mut machine, 0);
            load_track(&mut machine, "T1", 200_000, 0);
            let (x, y) = centre(widget);
            machine.mouse_down(x, y, 0);
            assert_eq!(machine.hint().as_deref(), Some(hint), "{widget:?}");
        }
    }

    #[test]
    fn transport_and_toggle_controls_have_no_hint() {
        for widget in [
            Widget::Previous,
            Widget::Play,
            Widget::Pause,
            Widget::Stop,
            Widget::Next,
            Widget::Eject,
            Widget::Shuffle,
            Widget::Repeat,
            Widget::EqToggle,
            Widget::PlaylistToggle,
            Widget::TimeDisplay,
            Widget::WindowMenu,
            Widget::WindowShade,
        ] {
            let mut machine = MainWindow::new();
            let (x, y) = centre(widget);
            machine.mouse_down(x, y, 0);
            assert_eq!(machine.hint(), None, "{widget:?}");
        }
    }

    #[test]
    fn the_balance_hint_names_the_side() {
        let mut machine = MainWindow::new();
        let track = layout().balance_track;
        let y = track.y + 6;
        machine.mouse_down(track.x, y, 0);
        assert_eq!(machine.hint().as_deref(), Some("BALANCE: LEFT"));
        machine.mouse_up(track.x, y, 0);
        machine.mouse_down(track.right() - 1, y, 0);
        assert_eq!(machine.hint().as_deref(), Some("BALANCE: RIGHT"));
    }

    // ----- time display (S02 §4) -----

    #[test]
    fn time_display_counts_up_while_playing_and_toggles_to_remaining() {
        let mut machine = MainWindow::new();
        playing(&mut machine, 0);
        load_track(&mut machine, "T1", 200_000, 0);
        machine.handle_event(
            &Event::PositionChanged {
                position_ms: 83_000,
                duration_ms: Some(200_000),
            },
            0,
        );
        assert!(machine.state().time_visible);
        assert_eq!(
            machine.state().time_cells(),
            [None, Some(1), Some(2), Some(3)]
        );

        // Clicking the display toggles to remaining (S02 §4).
        click(&mut machine, Widget::TimeDisplay, 0);
        assert!(machine.state().time_remaining);
        // 200 - 83 = 117 s -> 1:57.
        assert_eq!(
            machine.state().time_cells(),
            [None, Some(1), Some(5), Some(7)]
        );
        click(&mut machine, Widget::TimeDisplay, 0);
        assert!(!machine.state().time_remaining);
    }

    #[test]
    fn the_leading_no_digit_rule_switches_at_ten_minutes() {
        let mut machine = MainWindow::new();
        playing(&mut machine, 0);
        load_track(&mut machine, "T1", 20 * 60 * 1000, 0);

        machine.handle_event(
            &Event::PositionChanged {
                position_ms: 9 * 60 * 1000 + 59_000,
                duration_ms: Some(20 * 60 * 1000),
            },
            0,
        );
        assert_eq!(
            machine.state().time_cells(),
            [None, Some(9), Some(5), Some(9)]
        );

        machine.handle_event(
            &Event::PositionChanged {
                position_ms: 10 * 60 * 1000,
                duration_ms: Some(20 * 60 * 1000),
            },
            0,
        );
        assert_eq!(
            machine.state().time_cells(),
            [Some(1), Some(0), Some(0), Some(0)]
        );
    }

    #[test]
    fn pause_blinks_with_injected_time() {
        let mut machine = MainWindow::new();
        playing(&mut machine, 0);
        load_track(&mut machine, "T1", 200_000, 0);
        machine.handle_event(
            &Event::PositionChanged {
                position_ms: 30_000,
                duration_ms: Some(200_000),
            },
            0,
        );
        // Entering paused starts the blink at t = 1000.
        machine.handle_event(&Event::StateChanged(TransportState::Paused), 1000);

        machine.refresh(1000);
        assert!(machine.state().time_visible);
        machine.refresh(1000 + BLINK_HALF_MS - 1);
        assert!(machine.state().time_visible);
        machine.refresh(1000 + BLINK_HALF_MS);
        assert!(!machine.state().time_visible);
        machine.refresh(1000 + BLINK_PERIOD_MS);
        assert!(machine.state().time_visible);
        // The frozen time does not change while paused.
        assert_eq!(machine.state().time_seconds, 30);
    }

    #[test]
    fn stopped_and_nothing_loaded_show_a_blank_time() {
        let mut machine = MainWindow::new();
        assert!(!machine.state().time_visible);

        playing(&mut machine, 0);
        load_track(&mut machine, "T1", 200_000, 0);
        machine.refresh(0);
        assert!(machine.state().time_visible);

        machine.handle_event(&Event::StateChanged(TransportState::Stopped), 0);
        assert!(!machine.state().time_visible);
    }

    // ----- marquee (S02 §3) -----

    #[test]
    fn nothing_loaded_shows_the_program_name_without_scrolling() {
        let mut machine = MainWindow::new();
        machine.refresh(0);
        assert_eq!(machine.logical_title(), PROGRAM_NAME);
        assert_eq!(machine.state().title, PROGRAM_NAME);
        assert_eq!(machine.state().marquee_offset, 0);
    }

    #[test]
    fn a_long_title_scrolls_five_pixels_every_200_ms() {
        let mut machine = MainWindow::new();
        playing(&mut machine, 0);
        load_track(
            &mut machine,
            "This Is A Very Long Track Title Indeed",
            200_000,
            0,
        );
        machine.set_autoscroll(true, 0);

        machine.refresh(0);
        assert_eq!(machine.state().marquee_offset, 0);
        assert!(
            machine.state().title.contains(MARQUEE_SEPARATOR),
            "the scrolling marquee inserts the separator: {}",
            machine.state().title
        );

        machine.refresh(200);
        assert_eq!(machine.state().marquee_offset, 5);
        machine.refresh(400);
        assert_eq!(machine.state().marquee_offset, 10);
        machine.refresh(1000);
        assert_eq!(machine.state().marquee_offset, 25);
    }

    #[test]
    fn a_short_title_does_not_scroll_or_add_a_separator() {
        let mut machine = MainWindow::new();
        playing(&mut machine, 0);
        load_track(&mut machine, "T1", 200_000, 0);
        machine.refresh(0);
        assert_eq!(machine.state().title, "1. T1 (3:20)");
        assert!(!machine.state().title.contains(MARQUEE_SEPARATOR));
        machine.refresh(10_000);
        assert_eq!(machine.state().marquee_offset, 0);
    }

    #[test]
    fn the_autoscroll_toggle_stops_and_resumes_the_marquee() {
        let mut machine = MainWindow::new();
        playing(&mut machine, 0);
        load_track(
            &mut machine,
            "This Is A Very Long Track Title Indeed",
            200_000,
            0,
        );
        machine.refresh(0);
        machine.refresh(200);
        let moving = machine.state().marquee_offset;
        assert_eq!(moving, 5);

        machine.set_autoscroll(false, 200);
        let frozen = machine.state().marquee_offset;
        machine.refresh(200 + 2000);
        assert_eq!(machine.state().marquee_offset, frozen);

        machine.set_autoscroll(true, 3000);
        machine.refresh(3000 + 200);
        assert_eq!(machine.state().marquee_offset, 5);
    }

    #[test]
    fn dragging_the_marquee_with_autoscroll_off_scrolls_by_hand() {
        let mut machine = MainWindow::new();
        playing(&mut machine, 0);
        load_track(
            &mut machine,
            "This Is A Very Long Track Title Indeed",
            200_000,
            0,
        );
        machine.set_autoscroll(false, 0);
        let (x, y) = centre(Widget::Marquee);

        machine.mouse_down(x, y, 0);
        machine.mouse_move(x - 20, y, 0);
        let advanced = machine.state().marquee_offset;
        assert!(advanced > 0, "dragging left advances the text");
        machine.mouse_move(x, y, 0);
        assert!(
            machine.state().marquee_offset < advanced,
            "dragging right goes back"
        );
        machine.mouse_up(x, y, 0);
    }

    #[test]
    fn the_marquee_shows_the_hint_while_a_control_is_held() {
        let mut machine = MainWindow::new();
        playing(&mut machine, 0);
        load_track(&mut machine, "T1", 200_000, 0);
        let (x, y) = centre(Widget::ClutterO);
        machine.mouse_down(x, y, 0);
        assert_eq!(machine.state().title, "OPTIONS MENU");
        assert_eq!(machine.state().marquee_offset, 0);
        machine.mouse_up(x, y, 0);
        assert_eq!(machine.logical_title(), "1. T1 (3:20)");
    }

    // ----- stream info (S02 §5/§6) -----

    #[test]
    fn kbps_khz_and_channels_come_from_stream_info() {
        let mut machine = MainWindow::new();
        playing(&mut machine, 0);
        machine.handle_event(
            &Event::StreamInfoChanged(Some(StreamInfo {
                bitrate_kbps: Some(192),
                sample_rate_hz: 44_100,
                channels: 2,
            })),
            0,
        );
        assert_eq!(machine.state().kbps, Some(192));
        assert_eq!(machine.state().khz, Some(44));
        assert_eq!(machine.state().channels, MonoStereo::Stereo);

        machine.handle_event(
            &Event::StreamInfoChanged(Some(StreamInfo {
                bitrate_kbps: Some(128),
                sample_rate_hz: 48_000,
                channels: 1,
            })),
            0,
        );
        assert_eq!(machine.state().kbps, Some(128));
        assert_eq!(machine.state().khz, Some(48));
        assert_eq!(machine.state().channels, MonoStereo::Mono);
    }

    #[test]
    fn the_readout_is_absent_when_the_stream_is_unknown_or_stopped() {
        let mut machine = MainWindow::new();
        playing(&mut machine, 0);
        machine.handle_event(
            &Event::StreamInfoChanged(Some(StreamInfo {
                bitrate_kbps: None,
                sample_rate_hz: 44_100,
                channels: 2,
            })),
            0,
        );
        assert_eq!(machine.state().kbps, None);
        assert_eq!(machine.state().khz, Some(44));

        machine.handle_event(&Event::StreamInfoChanged(None), 0);
        assert_eq!(machine.state().kbps, None);
        assert_eq!(machine.state().khz, None);
        assert_eq!(machine.state().channels, MonoStereo::Unknown);

        machine.handle_event(
            &Event::StreamInfoChanged(Some(StreamInfo {
                bitrate_kbps: Some(192),
                sample_rate_hz: 44_100,
                channels: 2,
            })),
            0,
        );
        machine.handle_event(&Event::StateChanged(TransportState::Stopped), 0);
        assert_eq!(machine.state().kbps, None);
        assert_eq!(machine.state().khz, None);
    }

    // ----- core events -----

    #[test]
    fn core_events_update_the_render_state() {
        let mut machine = MainWindow::new();
        machine.handle_event(
            &Event::ModesChanged {
                shuffle: true,
                repeat: true,
            },
            0,
        );
        assert!(machine.state().shuffle && machine.state().repeat);

        machine.handle_event(
            &Event::VolumeChanged {
                volume: 255,
                balance: -127,
            },
            0,
        );
        assert_eq!(machine.state().volume, 100);
        assert_eq!(machine.state().balance, 0);

        machine.handle_event(
            &Event::WindowVisibilityChanged {
                window: WindowId::Equalizer,
                visible: true,
            },
            0,
        );
        assert!(machine.state().eq_open);

        machine.handle_event(
            &Event::TrackChanged {
                id: None,
                index: None,
                metadata: None,
                duration_ms: None,
            },
            0,
        );
        assert_eq!(machine.logical_title(), PROGRAM_NAME);
    }

    #[test]
    fn volume_and_balance_conversions_round_trip() {
        for percent in 0..=100u8 {
            assert_eq!(volume_percent(core_volume(percent)), percent);
        }
        assert_eq!(core_volume(0), 0);
        assert_eq!(core_volume(100), 255);
        assert_eq!(core_balance(50), 0);
        assert_eq!(core_balance(0), -127);
        assert_eq!(core_balance(100), 127);
        assert_eq!(balance_percent(0), 50);
    }

    #[test]
    fn window_button_pressed_state_tracks_the_held_button() {
        let mut machine = MainWindow::new();
        for (widget, button) in [
            (Widget::WindowMenu, WindowButton::Menu),
            (Widget::WindowMinimize, WindowButton::Minimize),
            (Widget::WindowShade, WindowButton::Shade),
            (Widget::WindowClose, WindowButton::Close),
        ] {
            let (x, y) = centre(widget);
            machine.mouse_down(x, y, 0);
            assert_eq!(
                machine.state().pressed_window_button,
                Some(button),
                "{widget:?}"
            );
            machine.mouse_up(x, y, 0);
            assert_eq!(machine.state().pressed_window_button, None);
        }
    }

    #[test]
    fn the_position_thumb_is_pressed_while_held() {
        let mut machine = MainWindow::new();
        playing(&mut machine, 0);
        load_track(&mut machine, "T1", 200_000, 0);
        let (x, y) = centre(Widget::PositionSlider);
        machine.mouse_down(x, y, 0);
        assert!(machine.state().position_thumb_pressed);
        machine.mouse_up(x, y, 0);
        assert!(!machine.state().position_thumb_pressed);
    }

    // ----- RFC 0005 predictions -----

    #[test]
    fn playing_an_entry_predicts_the_playing_state() {
        use TransportState::{Paused, Playing, Stopped};
        // Owner decision O1: start and end of list play.
        for from in [Stopped, Playing, Paused] {
            for command in [
                Command::PlayIndex(0),
                Command::PlayIndex(7),
                Command::PlayEntry(EntryId(1)),
                Command::PlayLast,
            ] {
                assert_eq!(
                    transport_transition(from, &command),
                    Some(Playing),
                    "{from:?} + {command:?}"
                );
            }
        }
    }

    #[test]
    fn a_skip_by_moves_the_entry_without_changing_the_transport() {
        use TransportState::{Paused, Playing, Stopped};
        // S02 §1: changing the entry while stopped does not start playback.
        for from in [Stopped, Playing, Paused] {
            for delta in [10, -10, 0] {
                assert_eq!(
                    transport_transition(from, &Command::SkipBy { delta }),
                    Some(from),
                    "{from:?} + {delta}"
                );
            }
        }
    }

    #[test]
    fn the_rfc_0005_app_commands_are_not_part_of_the_transport_machine() {
        for command in [
            Command::ToggleStopAfterCurrent,
            Command::OpenFileInfo,
            Command::OpenMedia {
                kind: rustamp_core::command::OpenMediaKind::Files,
                play: true,
            },
            Command::RefreshSkin,
            Command::Minimize,
        ] {
            assert_eq!(
                transport_transition(TransportState::Playing, &command),
                None,
                "{command:?}"
            );
        }
    }

    #[test]
    fn the_machine_follows_the_playing_predictions() {
        let mut machine = MainWindow::new();
        machine.handle_command(&Command::PlayIndex(0), 0);
        assert_eq!(machine.state().transport, TransportState::Playing);

        machine.handle_event(&Event::StateChanged(TransportState::Stopped), 0);
        machine.handle_command(&Command::PlayLast, 0);
        assert_eq!(machine.state().transport, TransportState::Playing);

        machine.handle_event(&Event::StateChanged(TransportState::Stopped), 0);
        machine.handle_command(&Command::PlayEntry(EntryId(1)), 0);
        assert_eq!(machine.state().transport, TransportState::Playing);

        // A skip leaves the transport alone (S02 §1).
        machine.handle_command(&Command::SkipBy { delta: 10 }, 0);
        assert_eq!(machine.state().transport, TransportState::Playing);
        machine.handle_event(&Event::StateChanged(TransportState::Stopped), 0);
        machine.handle_command(&Command::SkipBy { delta: 10 }, 0);
        assert_eq!(machine.state().transport, TransportState::Stopped);
    }
}
