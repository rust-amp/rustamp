//! What the main window is asked to draw.
//!
//! [`MainWindowState`] is a plain data struct: it describes a *state*, never a
//! transition. The transitions, the timers and the click handling live in
//! [`crate::behaviour`]; this module only says which sprite variants a single
//! frame should use.
//!
//! The transport state is the **core** [`rustamp_core::state::TransportState`],
//! re-exported here so the UI never keeps a parallel copy (task card D2).

pub use rustamp_core::state::TransportState;

/// A transport button that can be drawn in its pressed variant.
///
/// S02 §7 verifies the pressed sprites (the S01 §2.1 rows are provisional);
/// the behaviour module decides when one applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportButton {
    /// The previous button.
    Previous,
    /// The play button.
    Play,
    /// The pause button.
    Pause,
    /// The stop button.
    Stop,
    /// The next button.
    Next,
    /// The eject button.
    Eject,
}

/// One of the four window buttons in the title bar.
///
/// S02 §7: while a button is held it is drawn from a 9 x 9 `titlebar.bmp` cell;
/// in the normal state the four buttons are part of the title-bar body sprite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowButton {
    /// The menu button (top-left).
    Menu,
    /// The minimize button.
    Minimize,
    /// The windowshade button.
    Shade,
    /// The close button (top-right).
    Close,
}

/// The channel mode shown by the mono/stereo indicator.
///
/// S02 §6 lights exactly one label while a track is playing or paused and dims
/// both when stopped; `Unknown` (no stream info) also dims both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MonoStereo {
    /// Mono (the `mono` sprite is lit).
    Mono,
    /// Stereo (the `stereo` sprite is lit).
    Stereo,
    /// No stream info: both labels are dim (S02 §6).
    #[default]
    Unknown,
}

/// Everything a single main-window frame needs.
///
/// Values are plain data. `position`, `volume` and `balance` are percentages
/// (`0..=100`); `balance` is 50 when centred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainWindowState {
    /// Stopped, playing or paused (drives the play/pause/stop indicator).
    pub transport: TransportState,
    /// The transport button currently held down, if any.
    pub pressed: Option<TransportButton>,
    /// The window button currently held down, if any (S02 §7).
    pub pressed_window_button: Option<WindowButton>,
    /// Held slider or toggle, selecting its skin's depressed sprite.
    pub pressed_widget: Option<crate::widget::Widget>,
    /// Whether the position-bar thumb is held and drawn pressed (S02 §7).
    pub position_thumb_pressed: bool,
    /// Whether the window is focused (picks the focused title bar).
    pub title_bar_focused: bool,
    /// Whether the window is drawn in windowshade form.
    pub windowshade: bool,
    /// Whether the frame is drawn at 2x.
    pub double_size: bool,
    /// Clutter-bar `O` flag (options menu; S01 §2.1 provisional).
    pub clutter_o: bool,
    /// Clutter-bar `A` flag (always-on-top).
    pub clutter_a: bool,
    /// Clutter-bar `I` flag.
    pub clutter_i: bool,
    /// Clutter-bar `D` flag (double-size).
    pub clutter_d: bool,
    /// Clutter-bar `V` flag.
    pub clutter_v: bool,
    /// Shuffle engaged.
    pub shuffle: bool,
    /// Repeat engaged.
    pub repeat: bool,
    /// The equalizer window is open.
    pub eq_open: bool,
    /// The playlist window is open.
    pub playlist_open: bool,
    /// The time to display, in seconds.
    pub time_seconds: u32,
    /// Whether the time display is drawn at all (S02 §4: stopped and the
    /// blink-off phase are blank).
    pub time_visible: bool,
    /// Whether the time is remaining (`true`) or elapsed (`false`).
    pub time_remaining: bool,
    /// Whether a track is loaded.
    ///
    /// **Measured** (2026-10-21, base-2.91 client capture of Winamp 2.95):
    /// while nothing is loaded, the reference draws **no position bar at
    /// all** - the skin's own `main.bmp` shows through the whole track band
    /// (client 16,72 - 263,81). A track that is loaded but cannot be seeked
    /// (a stream) is a different case and keeps the disabled look.
    pub track_loaded: bool,
    /// Whether the current track can be seeked.
    ///
    /// The engine owns the fact (`Engine::is_seekable`): a stream has no
    /// position to seek to, and the engine refuses such a seek. The app hands
    /// the fact to the behaviour machine (see
    /// [`crate::behaviour::MainWindow::set_seekable`]), and a track that
    /// cannot be seeked draws the position slider in its disabled look and
    /// offers no seek at all.
    pub seekable: bool,
    /// Whether the track's length is unknown, so the remaining time cannot be
    /// shown.
    ///
    /// The time display then shows the placeholder - the skin's own dash and
    /// no digits - instead of a `00:00` that would read as "the track is
    /// over". The elapsed time still counts: it is known.
    pub duration_unknown: bool,
    /// The marquee text to draw. When the title scrolls this already includes
    /// the separator and a second copy (S02 §3).
    pub title: String,
    /// Pixels the marquee text is shifted to the left (S02 §3).
    pub marquee_offset: i32,
    /// The bitrate shown next to `kbps`, if known.
    pub kbps: Option<u32>,
    /// The sample rate shown next to `kHz`, if known.
    pub khz: Option<u32>,
    /// The channel mode.
    pub channels: MonoStereo,
    /// Playback position, `0..=100`.
    pub position: u8,
    /// The position thumb's offset in pixels while it follows the track. Finer
    /// than [`position`](Self::position), which is what a press sets.
    pub position_px: Option<u32>,
    /// Volume, `0..=100`.
    pub volume: u8,
    /// The volume in the engine's own `0..=255`, which the percentage cannot
    /// carry without drifting: the keys and the thumb work on this one.
    pub volume_core: u8,
    /// Balance, `0..=100` (50 is centred).
    pub balance: u8,
    /// Rustamp's own deterministic analyser/scope drawing state. The area and
    /// the band count follow the audited S08; see [`crate::visualization`].
    pub visualization: crate::visualization::VisState,
}

impl Default for MainWindowState {
    fn default() -> Self {
        Self {
            transport: TransportState::Stopped,
            pressed: None,
            pressed_window_button: None,
            pressed_widget: None,
            position_thumb_pressed: false,
            title_bar_focused: true,
            windowshade: false,
            double_size: false,
            clutter_o: false,
            clutter_a: false,
            clutter_i: false,
            clutter_d: false,
            clutter_v: false,
            shuffle: false,
            repeat: false,
            eq_open: false,
            playlist_open: false,
            time_seconds: 0,
            time_visible: true,
            time_remaining: false,
            track_loaded: false,
            seekable: true,
            duration_unknown: false,
            title: String::new(),
            marquee_offset: 0,
            kbps: None,
            khz: None,
            channels: MonoStereo::Unknown,
            position: 0,
            position_px: None,
            volume: 50,
            volume_core: 128,
            balance: 50,
            visualization: crate::visualization::VisState::default(),
        }
    }
}

impl MainWindowState {
    /// The four time-display cells, left to right: tens of minutes, minutes,
    /// tens of seconds, seconds.
    ///
    /// `None` is the "no digit" cell (S01 §2.3, cell 10). **Provisional rule:**
    /// S01 does not say when the leading cell is blank and S02 §4 does not
    /// settle it; this renders the tens of minutes as the "no digit" cell while
    /// the track is under ten minutes. The boundaries are exercised in the
    /// behaviour tests.
    ///
    /// A track whose length is unknown has **no remaining time**: the cells are
    /// all blank then, and the renderer draws the placeholder (the skin's own
    /// dash) beside them. A remaining `00:00` would claim that nothing is
    /// left of the track, which is a statement about a length nobody knows.
    /// The elapsed display is untouched: the elapsed time is known and counts.
    pub fn time_cells(&self) -> [Option<u8>; 4] {
        if self.shows_time_placeholder() {
            return [None; 4];
        }
        let total = self.time_seconds;
        let minutes = (total / 60).min(99);
        let seconds = total % 60;
        [
            Some((minutes / 10) as u8),
            Some((minutes % 10) as u8),
            Some((seconds / 10) as u8),
            Some((seconds % 10) as u8),
        ]
    }

    /// Whether the time display shows the placeholder for an unknown
    /// remaining time.
    ///
    /// A track whose length is unknown has no remaining time to show. The
    /// display then carries the placeholder alone - the skin's own dash - and
    /// no cells at all: four blank cells would still read as a time.
    pub fn shows_time_placeholder(&self) -> bool {
        self.duration_unknown && self.time_remaining
    }

    /// Whether the decoder-active indicator is drawn.
    ///
    /// **Provisional:** S01 §2.1/§2.4 identifies the two decoder sprites but not
    /// the condition that lights the active one; this ties it to playback, which
    /// S02 does not settle.
    pub fn decoder_active(&self) -> bool {
        self.transport == TransportState::Playing
    }

    /// Whether the given transport button is drawn pressed.
    pub fn is_pressed(&self, button: TransportButton) -> bool {
        self.pressed == Some(button)
    }

    /// Whether the given window button is drawn pressed (S02 §7).
    pub fn is_window_button_pressed(&self, button: WindowButton) -> bool {
        self.pressed_window_button == Some(button)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_cells_render_minutes_and_seconds() {
        let state = MainWindowState {
            time_seconds: 12 * 60 + 34,
            ..MainWindowState::default()
        };
        assert_eq!(state.time_cells(), [Some(1), Some(2), Some(3), Some(4)]);

        // Under ten minutes the leading cell is the "no digit" cell.
        let state = MainWindowState {
            time_seconds: 83,
            ..MainWindowState::default()
        };
        assert_eq!(state.time_cells(), [Some(0), Some(1), Some(2), Some(3)]);

        let state = MainWindowState::default();
        assert_eq!(state.time_cells(), [Some(0), Some(0), Some(0), Some(0)]);
    }

    #[test]
    fn decoder_is_active_only_while_playing() {
        let mut state = MainWindowState::default();
        assert!(!state.decoder_active());
        state.transport = TransportState::Playing;
        assert!(state.decoder_active());
        state.transport = TransportState::Paused;
        assert!(!state.decoder_active());
    }

    #[test]
    fn pressed_matches_only_the_held_button() {
        let mut state = MainWindowState::default();
        assert!(!state.is_pressed(TransportButton::Play));
        state.pressed = Some(TransportButton::Play);
        assert!(state.is_pressed(TransportButton::Play));
        assert!(!state.is_pressed(TransportButton::Stop));
    }

    #[test]
    fn window_button_pressed_matches_only_the_held_button() {
        let mut state = MainWindowState::default();
        assert!(!state.is_window_button_pressed(WindowButton::Close));
        state.pressed_window_button = Some(WindowButton::Close);
        assert!(state.is_window_button_pressed(WindowButton::Close));
        assert!(!state.is_window_button_pressed(WindowButton::Shade));
    }
}
