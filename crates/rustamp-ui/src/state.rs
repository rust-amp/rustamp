//! What the main window is asked to draw.
//!
//! [`MainWindowState`] is a plain data struct: it describes a *state*, never a
//! transition. There is no click handling, no timer and no marquee timing here
//! — those belong to spec S02 and later cards. This module only says which
//! sprite variants a single frame should use.

/// The transport state the play/pause/stop indicator reflects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportState {
    /// Nothing is playing: the stop indicator is drawn.
    #[default]
    Stopped,
    /// Playing: the play indicator is drawn.
    Playing,
    /// Paused: the pause indicator is drawn.
    Paused,
}

/// A transport button that can be drawn in its pressed variant.
///
/// The pressed states are **provisional** in S01 §2.1 (they were never
/// captured); this card renders them but does not decide when they apply.
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

/// The channel mode shown by the mono/stereo indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MonoStereo {
    /// Mono (the `mono` sprite is lit).
    Mono,
    /// Stereo (the `stereo` sprite is lit).
    #[default]
    Stereo,
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
    /// Whether the time is remaining (`true`) or elapsed (`false`).
    pub time_remaining: bool,
    /// The marquee title.
    pub title: String,
    /// The bitrate shown next to `kbps`, if known.
    pub kbps: Option<u32>,
    /// The sample rate shown next to `kHz`, if known.
    pub khz: Option<u32>,
    /// The channel mode.
    pub channels: MonoStereo,
    /// Playback position, `0..=100`.
    pub position: u8,
    /// Volume, `0..=100`.
    pub volume: u8,
    /// Balance, `0..=100` (50 is centred).
    pub balance: u8,
}

impl Default for MainWindowState {
    fn default() -> Self {
        Self {
            transport: TransportState::Stopped,
            pressed: None,
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
            time_remaining: false,
            title: String::new(),
            kbps: None,
            khz: None,
            channels: MonoStereo::Stereo,
            position: 0,
            volume: 50,
            balance: 50,
        }
    }
}

impl MainWindowState {
    /// The four time-display cells, left to right: tens of minutes, minutes,
    /// tens of seconds, seconds.
    ///
    /// `None` is the "no digit" cell (S01 §2.3, cell 10). **Provisional rule:**
    /// S01 does not say when the leading cell is blank; this renders the tens of
    /// minutes as the "no digit" cell while the track is under ten minutes. That
    /// choice is an S02 question and is not a verified S01 fact.
    pub fn time_cells(&self) -> [Option<u8>; 4] {
        let total = self.time_seconds;
        let minutes = (total / 60).min(99);
        let seconds = total % 60;
        let tens_minutes = if minutes < 10 {
            None
        } else {
            Some((minutes / 10) as u8)
        };
        [
            tens_minutes,
            Some((minutes % 10) as u8),
            Some((seconds / 10) as u8),
            Some((seconds % 10) as u8),
        ]
    }

    /// Whether the decoder-active indicator is drawn.
    ///
    /// **Provisional:** S01 §2.1/§2.4 identifies the two decoder sprites but not
    /// the condition that lights the active one; this ties it to playback, which
    /// is an S02 question.
    pub fn decoder_active(&self) -> bool {
        self.transport == TransportState::Playing
    }

    /// Whether the given transport button is drawn pressed.
    pub fn is_pressed(&self, button: TransportButton) -> bool {
        self.pressed == Some(button)
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
        assert_eq!(state.time_cells(), [None, Some(1), Some(2), Some(3)]);

        let state = MainWindowState::default();
        assert_eq!(state.time_cells(), [None, Some(0), Some(0), Some(0)]);
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
}
