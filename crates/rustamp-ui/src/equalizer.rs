// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless equalizer-window behaviour (specs **S07** and **S01 §3**, docking **S05**).
//!
//! [`EqualizerWindow`] is the equalizer's (`Winamp EQ`, 275 x 116) state
//! machine. Like [`crate::behaviour::MainWindow`] it is deliberately
//! **headless**: no `winit`, no `softbuffer`, no clock and no window. It owns
//! the band/preamp values (saved units `0..=63`), the On/Auto engagement flags
//! and the held/drag state, and translates the S07-settled gestures into core
//! [`Command`]s. It never opens the presets popup itself; a presets press is
//! returned as an [`EqMenuRequest`] (entries only) so the app layer can open
//! the OS window.
//!
//! Every rule cites its spec section. Choices the specs leave open are marked
//! **open/simple** in the code: the slider hit areas (S07 gives the button
//! rects but no slider hit areas), the interior tiling and body colouring (S07
//! §7.1/§7.2), the graph curve (S07 §7.3/§7.4), the band/preamp step sizes and
//! the dB mapping (S07 §9, §7.2), the balance right half (S07 §6, §12 O4) and
//! the equalizer's shade drawing (S05 §1.5).
//!
//! # The windowshade form (S01 §1, §3; S05 §1.2, §1.5)
//!
//! A shaded equalizer is a **275 x 14 title strip** (S01 §1: the windowshade
//! height is 14 px for the main window, the equalizer and the playlist
//! editor). Its band comes from `eq_ex.bmp` (S01 §3: row 0 focused, row 15
//! unfocused, both provisional), and S05 §1.2 settles for the windowshade in
//! general that "the shade is a single 14-px title-bar row: no body rows are
//! drawn".
//!
//! The On/Auto/Presets buttons and the eleven sliders belong to the body, so a
//! shaded equalizer neither draws them nor answers them: [`hit_test`] takes
//! the shade flag and only the title band answers there.
//!
//! **S07 has no shade wording at all**, so nothing in the audited specs shows
//! sliders in the equalizer's shade form. The webamp cross-check that
//! `rustamp-skin` carries names mini volume/balance cells in the same
//! `eq_ex.bmp` (rows 30-36), but no spec settles *where* they sit in the
//! strip (S05 §1.2 lists the equalizer's shade drawing as open), so they are
//! not drawn and not hit-tested here. The keyboard path is unchanged: the
//! window is still the equalizer, so `N`/`A`/`S` still act (S03 §3).

use rustamp_core::command::Command;
use rustamp_gfx::Rect;
use rustamp_proto::state::{EqState, eq_classic_from_db};

use crate::shortcuts::{Key, Modifiers};

/// The equalizer's client size at rest (S01 §1, S05 §2.1: 275 x 116; the
/// double-size form is 550 x 232 via [`crate::docking::window_size`]).
pub const EQUALIZER_WINDOW_SIZE: (u32, u32) = (275, 116);

/// Preamp slider column: the drawn thumb's top-left x (S01 §3, round 2).
pub const PREAMP_X: i32 = 22;
/// First band slider column (60 Hz); the rest are `BAND_X0 + 18 * band`
/// (S01 §3, round 2: 18 px pitch).
pub const BAND_X0: i32 = 79;
/// Band slider pitch in pixels (S01 §3).
pub const BAND_PITCH: i32 = 18;
/// How many bands the window shows (S07 §1: ten entries in `eq_data`).
pub const BAND_COUNT: u8 = 10;

/// Thumb top-left y at saved value 0, the top of travel (S01 §3.1).
pub const THUMB_TOP_Y: i32 = 38;
/// Thumb top-left y at the flat value 31 (S01 §3.1).
pub const THUMB_FLAT_Y: i32 = 63;
/// Thumb top-left y at saved value 63, the bottom of travel (S01 §3.1).
pub const THUMB_BOTTOM_Y: i32 = 89;
/// Slider thumb size, 11 x 11 (S01 §3, round 2: `eqmain.bmp` (0,164,11,11)).
pub const THUMB_SIZE: u32 = 11;
/// The flat saved value (S01 §3.1).
pub const FLAT_VALUE: u8 = 31;
/// Largest saved band/preamp value (S01 §3.1: `preamp` / `eq_data` are 0..63).
pub const MAX_VALUE: u8 = 63;

/// The slider track sprite's width in pixels.
///
/// The measured reference capture (2026-10-21) shows the track as one
/// 14-px-wide column: the sheet's groove column, the coloured body with its
/// bright core and the end caps.
pub const TRACK_WIDTH: u32 = 14;
/// The slider track sprite's height in rows.
///
/// The measured reference capture shows the track running from the top of
/// travel (y = 38) down to y = 100 inclusive.
pub const TRACK_HEIGHT: u32 = 63;
/// The track sprite's left edge relative to the thumb's top-left x.
///
/// The measured reference capture puts the track's left column one pixel left
/// of the drawn thumb (S01 §3.1: the track-slot origin is one pixel left of
/// the thumb's top-left).
pub const TRACK_LEFT_OFFSET: i32 = -1;

/// Thumb top-left y for a saved value `0..=63` (S01 §3.1).
///
/// The three measured points (0 -> 38, 31 -> 63, 63 -> 89) are linear at
/// ~0.81 px per unit; the interpolation between them is the natural reading,
/// not a separate measurement.
pub fn thumb_y_for_value(value: u8) -> i32 {
    // Measured against Winamp 2.95 for every odd value (2026-10-31).
    let value = value.min(MAX_VALUE) as i32;
    THUMB_TOP_Y + (value * 13 + 12) / 16
}

/// Saved value `0..=63` for a thumb top-left y (the inverse of
/// [`thumb_y_for_value`], clamped to the travel).
pub fn value_for_thumb_y(y: i32) -> u8 {
    let y = y.clamp(THUMB_TOP_Y, THUMB_BOTTOM_Y);
    if y == THUMB_BOTTOM_Y {
        return MAX_VALUE;
    }
    // The smallest value whose thumb sits at `y`.
    ((((y - THUMB_TOP_Y) * 16 - 12).max(0) + 12) / 13).clamp(0, 63) as u8
}

/// How far a band or preamp key moves its slider, in saved units. **Own choice:**
/// S07 §9 leaves the step open and names one saved unit as its first candidate.
pub const KEY_STEP: u8 = 2;

/// The band the letter key `letter` lowers (S03 §3: `Q` the first, `P` the last).
fn lowered_band(letter: u8) -> Option<u8> {
    b"QWERTYUIOP"
        .iter()
        .position(|key| *key == letter)
        .map(|band| band as u8)
}

/// A saved value `0..=63` as a gain in dB, clamped to ±12.
///
/// Anchors from spec S09 (audited) and S01 §3:
/// `0 = +12 dB`, `31 = 0 dB` (flat), `63 = -12 dB`.
///
/// The conversion uses a piecewise-linear law that hits all three anchors exactly:
/// - for `value <= 31`: `db = (31 - value) * 12 / 31` (so 0 -> +12 dB, 31 -> 0 dB)
/// - for `value >= 31`: `db = (31 - value) * 12 / 32` (so 31 -> 0 dB, 63 -> -12 dB)
///
/// The three anchors and the measured conversion in `rustamp-proto`
/// (`eq_classic_from_db`) are fixed; the intermediate law between the anchors
/// is an own choice (the spec leaves it open). This piecewise law ensures that
/// `eq_classic_from_db(saved_to_gain_db(v)) == v` for every `v in 0..=63`,
/// eliminating the dead zone at the bottom of the slider travel.
pub fn saved_to_gain_db(value: u8) -> f32 {
    let value = value.min(MAX_VALUE);
    if value <= FLAT_VALUE {
        (f32::from(FLAT_VALUE) - f32::from(value)) * 12.0 / f32::from(FLAT_VALUE)
    } else {
        (f32::from(FLAT_VALUE) - f32::from(value)) * 12.0 / 32.0
    }
}

// ---------------------------------------------------------------------------
// S07 §5: the volume law (main-window slider, settled by the held-states run)
// ---------------------------------------------------------------------------

/// Left edge of the volume track's thumb travel (S07 §5: 0 % at
/// `thumb_left = 107`, measured).
pub const VOLUME_TRAVEL_LEFT: i32 = 107;
/// Predicted right end of the travel (S07 §5: 100 % at `thumb_left = 158`,
/// i.e. click x = 165 — **predicted, not measured**, §12 O3).
pub const VOLUME_TRAVEL_RIGHT: i32 = 158;
/// Volume thumb width in pixels (S07 §5: 14 px, drawn centred on the click).
pub const VOLUME_THUMB_WIDTH: i32 = 14;

/// Engine volume `0..=255` for a client click x (S07 §5, settled subject to the
/// visual-reading caveat): `thumb_left = click_x - 7` clamped to the travel,
/// then `v = 5 * (thumb_left - 107)` — exactly 5 core units per pixel.
pub fn volume_core_from_click(click_x: i32) -> u8 {
    let thumb_left =
        (click_x - VOLUME_THUMB_WIDTH / 2).clamp(VOLUME_TRAVEL_LEFT, VOLUME_TRAVEL_RIGHT);
    (5 * (thumb_left - VOLUME_TRAVEL_LEFT)) as u8
}

/// Display percentage for an engine volume (S07 §5): `floor(v * 100 / 255)`.
///
/// All four non-degenerate readings (15/41/66/92 %) match this to the digit.
/// Note this is a floor, not the rounded `volume_percent` the main window
/// currently shows; the difference is reported as a finding and the main
/// window's mapping is left unchanged.
pub fn volume_display_percent(core: u8) -> u8 {
    (u32::from(core) * 100 / 255) as u8
}

// ---------------------------------------------------------------------------
// S07 §6: the balance law (partially settled; the right half is open)
// ---------------------------------------------------------------------------

/// Left edge of the balance track (S07 §6: 38 px, x 177..214).
pub const BALANCE_TRACK_LEFT: i32 = 177;
/// Balance track width in pixels (S07 §6: 38 px).
pub const BALANCE_TRACK_WIDTH: i32 = 38;
/// Balance thumb width in pixels (S07 §6: 14 px, travel 24 px).
pub const BALANCE_THUMB_WIDTH: i32 = 14;
/// Thumb-left at the left extreme (S07 §6, measured: click 178 clamps here).
pub const BALANCE_LEFT_EXTREME: i32 = 177;
/// Thumb-left at the centre (S07 §6, settled).
pub const BALANCE_CENTRE_LEFT: i32 = 189;
/// Thumb-left at the right extreme (**unmeasured**, S07 §6/§12 O4: clicks 190,
/// 196 and 202 all landed inside the default thumb and never jumped).
pub const BALANCE_RIGHT_EXTREME: i32 = 201;

/// The marquee hint for a measured balance thumb position (S07 §6, settled).
///
/// Returns `Some("CENTER")` at the centre and `Some("100% LEFT")` at the left
/// extreme, and `None` everywhere else: every intermediate value, the whole
/// right half and the sign convention for right are **unmeasured** (S07 §12
/// O4), so no hint is offered for them.
pub fn balance_hint_for_thumb(thumb_left: i32) -> Option<&'static str> {
    if thumb_left == BALANCE_CENTRE_LEFT {
        Some("CENTER")
    } else if thumb_left == BALANCE_LEFT_EXTREME {
        Some("100% LEFT")
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Every rectangle the classic equalizer window draws, in one place.
///
/// Destinations are window-relative (S01 conventions); the held rects are the
/// S07 §3.2 lone-button-down measurements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EqualizerLayout {
    /// The opaque window background: `eqmain.bmp` at (0,0) (S01 §3, verified).
    pub background: Rect,
    /// The title-bar band (S01 §3, round 2: focused/unfocused at sheet y 134/149).
    pub title_bar: Rect,
    /// The On button at rest (S01 §3: 25 px wide at (14,18)).
    pub on: Rect,
    /// The On button while held: (14,18,25,12), 300/300 px in both engagement
    /// states (S07 §3.2, settled).
    pub on_held: Rect,
    /// The Auto button: (39,18,33,12), 395/396 px changed under a lone down
    /// (S07 §3.2, settled) and 33 px wide at rest as well — the measured
    /// reference capture draws the resting button in the same footprint.
    pub auto: Rect,
    /// The Presets button: (217,18,44,12), 528/528 px change on a lone down
    /// and back on release (S07 §4.1, settled).
    pub presets: Rect,
    /// The response graph background: `eqmain.bmp` (0,294,113,19) at (86,17)
    /// (S01 §3, round 2: verified with the flat-state residual).
    pub graph: Rect,
}

impl EqualizerLayout {
    /// The classic equalizer layout (S01 §3, S07 §3.2/§4.1).
    pub const CLASSIC: EqualizerLayout = EqualizerLayout {
        background: Rect::new(0, 0, 275, 116),
        title_bar: Rect::new(0, 0, 275, 14),
        on: Rect::new(14, 18, 25, 12),
        on_held: Rect::new(14, 18, 25, 12),
        auto: Rect::new(39, 18, 33, 12),
        presets: Rect::new(217, 18, 44, 12),
        graph: Rect::new(86, 17, 113, 19),
    };

    /// The thumb top-left x of slider `slider`.
    pub const fn slider_x(slider: EqSlider) -> i32 {
        match slider {
            EqSlider::Preamp => PREAMP_X,
            EqSlider::Band(band) => BAND_X0 + BAND_PITCH * (band as i32),
        }
    }

    /// The thumb rectangle of `slider` at saved value `value`.
    pub fn thumb_rect(slider: EqSlider, value: u8) -> Rect {
        Rect::new(
            Self::slider_x(slider),
            thumb_y_for_value(value),
            THUMB_SIZE,
            THUMB_SIZE,
        )
    }

    /// The track sprite's window rectangle of `slider`.
    ///
    /// The track is one sheet column: 14 px wide, starting one pixel left of
    /// the drawn thumb, and running from the top of travel down to y = 100.
    pub fn track_rect(slider: EqSlider) -> Rect {
        Rect::new(
            Self::slider_x(slider) + TRACK_LEFT_OFFSET,
            THUMB_TOP_Y,
            TRACK_WIDTH,
            TRACK_HEIGHT,
        )
    }

    /// The hit area of `slider`.
    ///
    /// **Simple/provisional:** S07 gives the button rects but no slider hit
    /// areas, so each slider takes its 11 px column plus a 3 px margin, over
    /// the travel plus the thumb height (y 30..100).
    pub fn slider_hit(slider: EqSlider) -> Rect {
        Rect::new(Self::slider_x(slider) - 3, 30, THUMB_SIZE + 6, 70)
    }
}

/// One of the eleven vertical sliders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EqSlider {
    /// The preamp (S01 §3: x = 22).
    Preamp,
    /// One of the ten bands, index `0..10` (S01 §3: x = 79 + 18k).
    Band(u8),
}

impl EqSlider {
    /// Every slider, preamp first, then bands 0..10.
    pub const ALL_BANDS: [EqSlider; 11] = [
        EqSlider::Preamp,
        EqSlider::Band(0),
        EqSlider::Band(1),
        EqSlider::Band(2),
        EqSlider::Band(3),
        EqSlider::Band(4),
        EqSlider::Band(5),
        EqSlider::Band(6),
        EqSlider::Band(7),
        EqSlider::Band(8),
        EqSlider::Band(9),
    ];
}

/// A hit-testable widget of the equalizer window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EqWidget {
    /// The title-bar band (the EQ window buttons are unmeasured, S05 §1.5).
    /// In the windowshade form the band is the whole window (S05 §1.2).
    TitleBar,
    /// The On button (S07 §3.2).
    On,
    /// The Auto button (S07 §3.2).
    Auto,
    /// The Presets button (S07 §4.1).
    Presets,
    /// One of the eleven sliders.
    Slider(EqSlider),
}

impl EqWidget {
    /// Every hit-testable widget of the **unshaded** window, in priority
    /// order: the buttons first (the Auto held rect reaches into the
    /// separator column), then the sliders.
    pub const ALL: &'static [EqWidget] = &[
        EqWidget::On,
        EqWidget::Auto,
        EqWidget::Presets,
        EqWidget::TitleBar,
    ];

    /// Every hit-testable widget of the **windowshade** form.
    ///
    /// S05 §1.2: the shade is a single 14-px title-bar row and no body rows
    /// are drawn. The On/Auto/Presets buttons and the eleven sliders belong
    /// to the body, so they are not visible in the shade and must not answer
    /// a click. The band itself answers: it is the window's drag target.
    pub const SHADE: &'static [EqWidget] = &[EqWidget::TitleBar];

    /// The widget's window-relative rectangle.
    pub fn rect(self, layout: &EqualizerLayout) -> Rect {
        match self {
            // The On button's held rect covers its resting one.
            EqWidget::On => layout.on_held,
            EqWidget::Auto => layout.auto,
            EqWidget::Presets => layout.presets,
            EqWidget::TitleBar => layout.title_bar,
            EqWidget::Slider(slider) => EqualizerLayout::slider_hit(slider),
        }
    }
}

fn contains(rect: &Rect, x: i32, y: i32) -> bool {
    x >= rect.x && y >= rect.y && x < rect.right() && y < rect.bottom()
}

/// Returns the widget at the window-relative point `(x, y)`, or `None` when
/// the point is outside every hit-testable region.
///
/// `shaded` selects the windowshade form (S05 §1.2): there the window is the
/// 14-px title strip, so only the band can be hit and the body controls —
/// which are not drawn — answer nothing.
pub fn hit_test(layout: &EqualizerLayout, x: i32, y: i32, shaded: bool) -> Option<EqWidget> {
    let widgets = if shaded {
        EqWidget::SHADE
    } else {
        EqWidget::ALL
    };
    for widget in widgets {
        if contains(&widget.rect(layout), x, y) {
            return Some(*widget);
        }
    }
    if shaded {
        return None;
    }
    for slider in EqSlider::ALL_BANDS {
        let widget = EqWidget::Slider(slider);
        if contains(&widget.rect(layout), x, y) {
            return Some(widget);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Everything a single equalizer frame needs.
///
/// Values are saved units `0..=63` (`preamp` / `eq_data` in S01 §3.1); the
/// engine's dB stage is M2 future work, so this is the UI's own prediction
/// state, exactly like [`crate::state::MainWindowState`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EqualizerState {
    /// Ten band values, `0..=63` (flat is 31).
    pub bands: [u8; 10],
    /// Preamp value, `0..=63` (flat is 31).
    pub preamp: u8,
    /// EQ engaged (`use_eq` in S01 §3.1; the On indicator cell link is
    /// provisional, S07 §12 item 13).
    pub enabled: bool,
    /// Preset autoloading (`autoload_eq` toggles the Auto cell, S01 §3.1).
    pub auto: bool,
    /// The button currently held down, if any (draws the held sprite, S07 §3.2).
    pub held: Option<EqWidget>,
    /// Whether the dragged thumb sprite applies (S01 §3.1).
    pub thumb_dragged: bool,
    /// Whether the window is focused (picks the focused title bar, S01 §3).
    pub focused: bool,
    /// Whether the window is drawn in windowshade form (S01 §1: the shade
    /// band is 14 px tall; S05 §1.2: a single title-bar row, no body rows).
    pub windowshade: bool,
    /// Whether the frame is drawn at 2x (S05 §2.1: 550 x 232).
    pub double_size: bool,
}

impl Default for EqualizerState {
    fn default() -> Self {
        Self {
            bands: [FLAT_VALUE; 10],
            preamp: FLAT_VALUE,
            // Provisional defaults: the fresh-launch engagement states are
            // open (S07 §12 item 13); the window renders flat either way.
            enabled: true,
            auto: false,
            held: None,
            thumb_dragged: false,
            focused: true,
            windowshade: false,
            double_size: false,
        }
    }
}

impl EqualizerState {
    /// The saved value of `slider`.
    pub fn slider_value(&self, slider: EqSlider) -> u8 {
        match slider {
            EqSlider::Preamp => self.preamp,
            EqSlider::Band(band) => self.bands.get(band as usize).copied().unwrap_or(FLAT_VALUE),
        }
    }
}

// ---------------------------------------------------------------------------
// The presets popup (S07 §4.2)
// ---------------------------------------------------------------------------

/// The popup the Presets button opens: window class `#32768`, client
/// 176 x 105 (window 182 x 111) (S07 §4.2, settled).
pub const PRESETS_POPUP_CLIENT_SIZE: (u32, u32) = (176, 105);
/// The popup's window size (S07 §4.2, settled).
pub const PRESETS_POPUP_WINDOW_SIZE: (u32, u32) = (182, 111);

/// The popup's top-level entries, top to bottom (S07 §4.2, visual reading:
/// Load, Save, Delete, each with a submenu arrow).
pub const PRESETS_MENU_ENTRIES: [&str; 3] = ["Load", "Save", "Delete"];

/// A menu the equalizer window asks the app to open.
///
/// The spec settles the popup's existence, class, size and top-level entries;
/// the submenu contents, command ids and load/save semantics are open (S07
/// §12 item 12, S04), so this carries no behaviour beyond the request — the
/// same dialog-stub shape as [`crate::playlist::PlaylistDialog`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EqMenuRequest {
    /// The Presets popup (`#32768`).
    Presets,
}

impl EqMenuRequest {
    /// The popup's top-level entries (S07 §4.2, visual reading).
    pub const fn entries(self) -> &'static [&'static str] {
        match self {
            EqMenuRequest::Presets => &PRESETS_MENU_ENTRIES,
        }
    }
}

// ---------------------------------------------------------------------------
// Behaviour
// ---------------------------------------------------------------------------

/// The equalizer window's headless behaviour.
#[derive(Debug, Clone)]
pub struct EqualizerWindow {
    layout: EqualizerLayout,
    state: EqualizerState,
    drag: Option<EqSlider>,
    /// Pointer-y minus thumb-top at grab time (S07 §8.1: 1:1 tracking with a
    /// constant offset; the 5-vs-6 value is weak/open, O9 — the grab preserves
    /// whatever offset the press had instead of asserting one).
    grab_offset: i32,
    menu_request: Option<EqMenuRequest>,
}

impl Default for EqualizerWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl EqualizerWindow {
    /// A new machine with the default (flat) state.
    pub fn new() -> Self {
        Self::from_state(EqualizerState::default())
    }

    /// A machine starting from a render state.
    pub fn from_state(state: EqualizerState) -> Self {
        Self {
            layout: EqualizerLayout::CLASSIC,
            state,
            drag: None,
            grab_offset: 5,
            menu_request: None,
        }
    }

    /// The current render state.
    pub fn state(&self) -> &EqualizerState {
        &self.state
    }

    /// The layout (always [`EqualizerLayout::CLASSIC`]).
    pub fn layout(&self) -> &EqualizerLayout {
        &self.layout
    }

    /// Takes the pending presets-menu request, if any.
    pub fn take_menu_request(&mut self) -> Option<EqMenuRequest> {
        self.menu_request.take()
    }

    /// Sets the double-size flag (S05 §2.1: the flag reaches the equalizer;
    /// the app mirrors the main window's flag here).
    pub fn set_double_size(&mut self, double_size: bool) {
        self.state.double_size = double_size;
    }

    /// Sets the focus flag (S01 §3: focused vs unfocused title bar).
    pub fn set_focused(&mut self, focused: bool) {
        self.state.focused = focused;
    }

    /// Sets the windowshade flag (S01 §1: the shade band is 14 px tall;
    /// S05 §1.2: the shade is a title strip alone).
    ///
    /// The sliders and the On/Auto/Presets buttons belong to the body, which
    /// the shade does not draw, so a drag cannot survive the switch: the held
    /// widget and the slider drag are dropped.
    pub fn set_windowshade(&mut self, shaded: bool) {
        if self.state.windowshade == shaded {
            return;
        }
        self.state.windowshade = shaded;
        self.state.held = None;
        self.state.thumb_dragged = false;
        self.drag = None;
    }

    /// Synchronizes the equalizer window state with an authoritative [`EqState`].
    ///
    /// Updates band gains, preamp gain, enabled flag and auto flag to match
    /// the engine's state, while preserving UI interaction flags (held widget,
    /// thumb dragging, focus, windowshade, double size).
    pub fn sync_from_eq_state(&mut self, eq: &EqState) {
        for (i, &gain_db) in eq.bands.iter().enumerate() {
            if i < self.state.bands.len() {
                self.state.bands[i] = eq_classic_from_db(gain_db) as u8;
            }
        }
        self.state.preamp = eq_classic_from_db(eq.preamp) as u8;
        self.state.enabled = eq.enabled;
        self.state.auto = eq.auto;
    }

    /// A press at window-relative `(x, y)`. Buttons arm their held sprite;
    /// sliders commit on the button down (S07 §5/§6 commit rule).
    ///
    /// In the windowshade form only the title band answers (S05 §1.2): the
    /// body controls are not drawn there, so they must not act either.
    pub fn mouse_down(&mut self, x: i32, y: i32) -> Vec<Command> {
        let widget = hit_test(&self.layout, x, y, self.state.windowshade);
        self.state.held = widget;
        self.drag = None;
        let mut commands = Vec::new();
        if let Some(EqWidget::Slider(slider)) = widget {
            self.drag = Some(slider);
            self.state.thumb_dragged = true;
            let thumb_top = thumb_y_for_value(self.state.slider_value(slider));
            if contains(
                &EqualizerLayout::thumb_rect(slider, self.state.slider_value(slider)),
                x,
                y,
            ) {
                // Down inside the thumb starts a drag without jumping (S07
                // §5/§6: click 108 / clicks 190-202 produced 0 px).
                self.grab_offset = y - thumb_top;
            } else {
                // Down outside the thumb jumps the thumb centre to the
                // pointer (the volume-law "drawn centred on the click" half
                // of S07 §5); the value commits immediately.
                self.grab_offset = (THUMB_SIZE as i32) / 2;
                commands.extend(self.set_slider_to_thumb(slider, y - self.grab_offset));
            }
        }
        commands
    }

    /// A pointer move. A dragged slider tracks the pointer 1:1 (S07 §8.1) and
    /// commits from the thumb, not the pointer (S07 §8.2).
    pub fn mouse_move(&mut self, x: i32, y: i32) -> Vec<Command> {
        let _ = x;
        let mut commands = Vec::new();
        if let Some(slider) = self.drag {
            commands.extend(self.set_slider_to_thumb(slider, y - self.grab_offset));
        }
        commands
    }

    /// A release at window-relative `(x, y)`. A button fires when the pointer
    /// is released inside it; a slider has already fired on press/drag.
    pub fn mouse_up(&mut self, x: i32, y: i32) -> Vec<Command> {
        let mut commands = Vec::new();
        if self.drag.is_none()
            && let Some(held) = self.state.held
            && hit_test(&self.layout, x, y, self.state.windowshade) == Some(held)
        {
            match held {
                EqWidget::On => {
                    let enabled = !self.state.enabled;
                    self.state.enabled = enabled;
                    commands.push(Command::SetEqEnabled(enabled));
                }
                EqWidget::Auto => {
                    let auto = !self.state.auto;
                    self.state.auto = auto;
                    commands.push(Command::SetEqAuto(auto));
                }
                EqWidget::Presets => {
                    self.menu_request = Some(EqMenuRequest::Presets);
                }
                _ => {}
            }
        }
        self.state.held = None;
        self.state.thumb_dragged = false;
        self.drag = None;
        commands
    }

    /// Whether a key press belongs to this window's own table (S03 §3) and not
    /// to the keys it shares with the main window.
    pub fn owns_key(key: Key, modifiers: Modifiers) -> bool {
        if modifiers != Modifiers::NONE {
            return false;
        }
        match key {
            Key::Letter(letter) => {
                let letter = letter.to_ascii_uppercase();
                letter == b'N' || letter == b'A' || letter == b'S' || lowered_band(letter).is_some()
            }
            Key::Digit(_) | Key::Grave | Key::Tab => true,
            _ => false,
        }
    }

    /// A key press. `N` toggles EQ enabled, `A` toggles autoloading, `S` opens
    /// the presets menu (S03 §3 via S07 §1; plain keys only — modified forms
    /// are unmeasured). `1`…`0` raise a band and `Q`…`P` lower it, `` ` `` raises
    /// the preamp and `Tab` lowers it (S03 §3). The step is [`KEY_STEP`].
    pub fn key_down(&mut self, key: Key, modifiers: Modifiers) -> Vec<Command> {
        if modifiers != Modifiers::NONE {
            return Vec::new();
        }
        if let Key::Letter(letter) = key
            && let Some(band) = lowered_band(letter.to_ascii_uppercase())
        {
            return self.step_slider(EqSlider::Band(band), false);
        }
        match key {
            // S03 §3: `1` is the first band and `0` the last.
            Key::Digit(digit) => {
                let band = if digit == 0 { 9 } else { digit - 1 };
                self.step_slider(EqSlider::Band(band), true)
            }
            // The direction of these two is unresolved (S03 §3): they follow
            // the bands' keys, raising with the key beside `1`.
            Key::Grave => self.step_slider(EqSlider::Preamp, true),
            Key::Tab => self.step_slider(EqSlider::Preamp, false),
            Key::Letter(b'N' | b'n') => {
                let enabled = !self.state.enabled;
                self.state.enabled = enabled;
                vec![Command::SetEqEnabled(enabled)]
            }
            Key::Letter(b'A' | b'a') => {
                let auto = !self.state.auto;
                self.state.auto = auto;
                vec![Command::SetEqAuto(auto)]
            }
            Key::Letter(b'S' | b's') => {
                self.menu_request = Some(EqMenuRequest::Presets);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Moves `slider` by [`KEY_STEP`] and commits the value. A smaller saved
    /// value is a higher gain.
    fn step_slider(&mut self, slider: EqSlider, raise: bool) -> Vec<Command> {
        let step = |value: u8| {
            if raise {
                value.saturating_sub(KEY_STEP)
            } else {
                value.saturating_add(KEY_STEP).min(MAX_VALUE)
            }
        };
        match slider {
            EqSlider::Preamp => {
                self.state.preamp = step(self.state.preamp);
                vec![Command::SetPreamp(saved_to_gain_db(self.state.preamp))]
            }
            EqSlider::Band(band) => match self.state.bands.get_mut(band as usize) {
                Some(value) => {
                    *value = step(*value);
                    vec![Command::SetBand {
                        band,
                        gain_db: saved_to_gain_db(*value),
                    }]
                }
                None => Vec::new(),
            },
        }
    }

    /// Moves `slider`'s thumb-top to `thumb_top` (clamped to the travel) and
    /// commits the value, returning the engine command.
    fn set_slider_to_thumb(&mut self, slider: EqSlider, thumb_top: i32) -> Option<Command> {
        let value = value_for_thumb_y(thumb_top);
        match slider {
            EqSlider::Preamp => {
                self.state.preamp = value;
                Some(Command::SetPreamp(saved_to_gain_db(value)))
            }
            EqSlider::Band(band) if (band as usize) < self.state.bands.len() => {
                self.state.bands[band as usize] = value;
                Some(Command::SetBand {
                    band,
                    gain_db: saved_to_gain_db(value),
                })
            }
            EqSlider::Band(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_rects_match_s07_3_2() {
        let l = EqualizerLayout::CLASSIC;
        assert_eq!(l.on_held, Rect::new(14, 18, 25, 12));
        // The Auto button is 33 px wide at x = 39 at rest as well: the
        // measured reference capture draws the resting button in the same
        // footprint as the held one.
        assert_eq!(l.auto, Rect::new(39, 18, 33, 12));
        // The resting rects: On is 25 px wide (not 26).
        assert_eq!(l.on, Rect::new(14, 18, 25, 12));
        assert_eq!(l.presets, Rect::new(217, 18, 44, 12));
        assert_eq!(l.graph, Rect::new(86, 17, 113, 19));
    }

    #[test]
    fn slider_columns_match_s01_3() {
        assert_eq!(EqualizerLayout::slider_x(EqSlider::Preamp), 22);
        for band in 0..10u8 {
            assert_eq!(
                EqualizerLayout::slider_x(EqSlider::Band(band)),
                79 + 18 * i32::from(band)
            );
        }
    }

    #[test]
    fn thumb_y_mapping_hits_the_three_measured_points() {
        assert_eq!(thumb_y_for_value(0), 38);
        assert_eq!(thumb_y_for_value(31), 63);
        assert_eq!(thumb_y_for_value(63), 89);
        // Monotonic across the whole scale.
        let mut last = thumb_y_for_value(0);
        for value in 1..=63u8 {
            let y = thumb_y_for_value(value);
            assert!(y >= last, "value {value}: {y} < {last}");
            last = y;
        }
    }

    #[test]
    fn thumb_y_round_trips_through_the_inverse() {
        for value in 0..=MAX_VALUE {
            let y = thumb_y_for_value(value);
            assert_eq!(
                thumb_y_for_value(value_for_thumb_y(y)),
                y,
                "value {value} at y {y}"
            );
        }
        assert_eq!(value_for_thumb_y(0), 0);
        assert_eq!(value_for_thumb_y(1000), 63);
    }

    #[test]
    fn saved_to_gain_db_anchors_and_round_trip() {
        use rustamp_proto::state::eq_classic_from_db;

        // Three measured anchors from spec S09 (audited) and S01 §3:
        // 0 -> +12 dB, 31 -> 0 dB (flat), 63 -> -12 dB.
        assert_eq!(saved_to_gain_db(0), 12.0);
        assert_eq!(saved_to_gain_db(31), 0.0);
        assert_eq!(saved_to_gain_db(63), -12.0);

        // Anchors mapped back through eq_classic_from_db.
        assert_eq!(eq_classic_from_db(saved_to_gain_db(0)), 0);
        assert_eq!(eq_classic_from_db(saved_to_gain_db(31)), 31);
        assert_eq!(eq_classic_from_db(saved_to_gain_db(63)), 63);

        // The intermediate law between the anchors is an own choice (the spec leaves
        // it open), while the three anchors and the measured conversion are fixed.
        // The round trip eq_classic_from_db(saved_to_gain_db(v)) == v must hold for
        // every single slider value in 0..=63 without dead zones.
        for value in 0..=63u8 {
            let db = saved_to_gain_db(value);
            let classic = eq_classic_from_db(db);
            assert_eq!(
                classic,
                i32::from(value),
                "round trip failed for value {value}: db = {db}, classic = {classic}"
            );
        }
    }

    #[test]
    fn hit_testing_finds_the_buttons_and_sliders() {
        let l = EqualizerLayout::CLASSIC;
        assert_eq!(hit_test(&l, 14, 18, false), Some(EqWidget::On));
        assert_eq!(hit_test(&l, 38, 29, false), Some(EqWidget::On));
        // The corrected Auto column: x = 39 belongs to Auto, not the gap.
        assert_eq!(hit_test(&l, 39, 24, false), Some(EqWidget::Auto));
        assert_eq!(hit_test(&l, 55, 24, false), Some(EqWidget::Auto));
        assert_eq!(hit_test(&l, 217, 18, false), Some(EqWidget::Presets));
        assert_eq!(hit_test(&l, 260, 29, false), Some(EqWidget::Presets));
        assert_eq!(
            hit_test(&l, 22, 63, false),
            Some(EqWidget::Slider(EqSlider::Preamp))
        );
        assert_eq!(
            hit_test(&l, 79, 63, false),
            Some(EqWidget::Slider(EqSlider::Band(0)))
        );
        assert_eq!(
            hit_test(&l, 241, 63, false),
            Some(EqWidget::Slider(EqSlider::Band(9)))
        );
        assert_eq!(hit_test(&l, 137, 7, false), Some(EqWidget::TitleBar));
        assert_eq!(hit_test(&l, 274, 115, false), None);
        assert_eq!(hit_test(&l, -1, 0, false), None);
    }

    /// S05 §1.2: the shade is a single 14-px title-bar row. Only the band
    /// answers there; the body controls are not drawn, so they must not.
    #[test]
    fn the_shaded_equalizer_answers_only_its_title_band() {
        let l = EqualizerLayout::CLASSIC;
        // The band is the whole shaded window.
        assert_eq!(hit_test(&l, 0, 0, true), Some(EqWidget::TitleBar));
        assert_eq!(hit_test(&l, 137, 7, true), Some(EqWidget::TitleBar));
        assert_eq!(hit_test(&l, 274, 13, true), Some(EqWidget::TitleBar));
        // The On/Auto/Presets buttons and every slider are body controls.
        assert_eq!(hit_test(&l, 20, 24, true), None);
        assert_eq!(hit_test(&l, 55, 24, true), None);
        assert_eq!(hit_test(&l, 239, 24, true), None);
        for slider in EqSlider::ALL_BANDS {
            let rect = EqualizerLayout::slider_hit(slider);
            assert_eq!(
                hit_test(&l, rect.x, rect.y, true),
                None,
                "{slider:?} must not answer in the shade"
            );
            assert_eq!(
                hit_test(&l, rect.x, rect.y, false),
                Some(EqWidget::Slider(slider)),
                "{slider:?} must answer in the unshaded window"
            );
        }
        // Below the strip nothing hits, in either form.
        assert_eq!(hit_test(&l, 0, 14, true), None);
    }

    #[test]
    fn on_and_auto_toggle_on_release_inside() {
        let mut window = EqualizerWindow::new();
        assert!(window.state().enabled);
        assert!(window.mouse_down(20, 24).is_empty());
        assert_eq!(window.state().held, Some(EqWidget::On));
        let commands = window.mouse_up(20, 24);
        assert_eq!(commands, vec![Command::SetEqEnabled(false)]);
        assert!(!window.state().enabled);

        assert!(window.mouse_down(55, 24).is_empty());
        let commands = window.mouse_up(55, 24);
        assert_eq!(commands, vec![Command::SetEqAuto(true)]);
        assert!(window.state().auto);
    }

    #[test]
    fn releasing_outside_cancels_the_press() {
        let mut window = EqualizerWindow::new();
        assert!(window.mouse_down(20, 24).is_empty());
        assert!(window.mouse_up(200, 100).is_empty());
        assert!(window.state().enabled);
        assert_eq!(window.state().held, None);
    }

    #[test]
    fn presets_opens_the_menu_request_and_no_command() {
        let mut window = EqualizerWindow::new();
        assert!(window.mouse_down(239, 24).is_empty());
        assert!(window.mouse_up(239, 24).is_empty());
        assert_eq!(window.take_menu_request(), Some(EqMenuRequest::Presets));
        assert_eq!(window.take_menu_request(), None);
    }

    #[test]
    fn slider_commits_on_the_button_down() {
        // Pressing below the flat thumb jumps and commits immediately (the
        // S07 §5/§6 commit rule); the release adds nothing.
        let mut window = EqualizerWindow::new();
        let commands = window.mouse_down(22, 95);
        assert_eq!(commands.len(), 1);
        assert!(matches!(commands[0], Command::SetPreamp(_)));
        assert!(window.mouse_up(22, 95).is_empty());
        assert!(window.state().preamp > FLAT_VALUE);
    }

    #[test]
    fn pressing_inside_the_thumb_starts_a_drag_without_jumping() {
        // S07 §5/§6: a down inside the thumb produced 0 px (no jump).
        let mut window = EqualizerWindow::new();
        let before = window.state().preamp;
        let commands = window.mouse_down(22, 65);
        assert!(commands.is_empty());
        assert_eq!(window.state().preamp, before);
        // ...and the drag then tracks the pointer 1:1 (S07 §8.1).
        let commands = window.mouse_move(22, 75);
        assert_eq!(commands.len(), 1);
        let _ = window.mouse_up(22, 75);
    }

    #[test]
    fn the_band_keys_move_the_band_they_name() {
        let mut eq = EqualizerWindow::new();
        let flat = eq.state().bands[0];
        let raised = eq.key_down(Key::Digit(1), Modifiers::NONE);
        assert!(
            matches!(raised.as_slice(), [Command::SetBand { band: 0, gain_db }] if *gain_db > 0.0)
        );
        assert_eq!(eq.state().bands[0], flat - KEY_STEP);
        eq.key_down(Key::Letter(b'Q'), Modifiers::NONE);
        assert_eq!(eq.state().bands[0], flat, "the matching keys cancel out");

        let lowered = eq.key_down(Key::Letter(b'P'), Modifiers::NONE);
        assert!(
            matches!(lowered.as_slice(), [Command::SetBand { band: 9, gain_db }] if *gain_db < 0.0)
        );
        let raised = eq.key_down(Key::Digit(0), Modifiers::NONE);
        assert!(matches!(
            raised.as_slice(),
            [Command::SetBand { band: 9, .. }]
        ));
    }

    #[test]
    fn the_preamp_keys_move_the_preamp_and_stop_at_the_ends() {
        let mut eq = EqualizerWindow::new();
        let flat = eq.state().preamp;
        eq.key_down(Key::Grave, Modifiers::NONE);
        assert_eq!(eq.state().preamp, flat - KEY_STEP);
        for _ in 0..64 {
            eq.key_down(Key::Tab, Modifiers::NONE);
        }
        assert_eq!(eq.state().preamp, MAX_VALUE);
    }

    #[test]
    fn the_window_owns_its_table_and_nothing_else() {
        assert!(EqualizerWindow::owns_key(
            Key::Letter(b'Q'),
            Modifiers::NONE
        ));
        assert!(EqualizerWindow::owns_key(Key::Digit(5), Modifiers::NONE));
        assert!(!EqualizerWindow::owns_key(
            Key::Letter(b'X'),
            Modifiers::NONE
        ));
        assert!(!EqualizerWindow::owns_key(
            Key::Letter(b'Q'),
            Modifiers::CTRL
        ));
    }

    #[test]
    fn band_drag_emits_set_band_for_the_right_band() {
        let mut window = EqualizerWindow::new();
        let commands = window.mouse_down(133, 95);
        assert_eq!(commands.len(), 1);
        match commands[0] {
            Command::SetBand { band, .. } => assert_eq!(band, 3),
            ref other => panic!("expected SetBand, got {other:?}"),
        }
    }

    #[test]
    fn volume_law_is_five_units_per_pixel_committed_on_down() {
        // S07 §5 table: clicks 122/135/148/161 -> thumbs 115/128/141/154 ->
        // 40/105/170/235 core -> 15/41/66/92 %.
        for (click, core, percent) in [
            (122, 40, 15),
            (135, 105, 41),
            (148, 170, 66),
            (161, 235, 92),
        ] {
            assert_eq!(volume_core_from_click(click), core, "click {click}");
            assert_eq!(volume_display_percent(core), percent, "core {core}");
        }
        // The measured 0 % end clamps; the 100 % end is predicted at 158.
        assert_eq!(volume_core_from_click(100), 0);
        assert_eq!(volume_core_from_click(107), 0);
        assert_eq!(volume_core_from_click(165), 255);
        assert_eq!(volume_display_percent(0), 0);
        assert_eq!(volume_display_percent(255), 100);
    }

    #[test]
    fn balance_centre_and_left_are_settled_the_rest_is_open() {
        assert_eq!(balance_hint_for_thumb(189), Some("CENTER"));
        assert_eq!(balance_hint_for_thumb(177), Some("100% LEFT"));
        // The right half, every intermediate value and the sign convention
        // for right are unmeasured (S07 §6, §12 O4).
        assert_eq!(balance_hint_for_thumb(201), None);
        assert_eq!(balance_hint_for_thumb(183), None);
        assert_eq!(balance_hint_for_thumb(195), None);
    }

    #[test]
    fn presets_popup_facts_match_s07_4_2() {
        assert_eq!(PRESETS_POPUP_CLIENT_SIZE, (176, 105));
        assert_eq!(PRESETS_POPUP_WINDOW_SIZE, (182, 111));
        assert_eq!(EqMenuRequest::Presets.entries(), &PRESETS_MENU_ENTRIES);
        assert_eq!(PRESETS_MENU_ENTRIES, ["Load", "Save", "Delete"]);
    }

    #[test]
    fn plain_n_a_s_keys_drive_the_settled_shortcuts() {
        let mut window = EqualizerWindow::new();
        let commands = window.key_down(Key::Letter(b'N'), Modifiers::NONE);
        assert_eq!(commands, vec![Command::SetEqEnabled(false)]);
        let commands = window.key_down(Key::Letter(b'A'), Modifiers::NONE);
        assert_eq!(commands, vec![Command::SetEqAuto(true)]);
        assert!(
            window
                .key_down(Key::Letter(b'S'), Modifiers::NONE)
                .is_empty()
        );
        assert_eq!(window.take_menu_request(), Some(EqMenuRequest::Presets));
        // Modified forms are unmeasured and do nothing.
        assert!(
            window
                .key_down(Key::Letter(b'N'), Modifiers::CTRL)
                .is_empty()
        );
        assert!(!window.state().enabled);
    }

    #[test]
    fn sync_from_eq_state_updates_values_and_preserves_ui_flags() {
        let mut window = EqualizerWindow::new();
        window.set_focused(false);
        window.set_double_size(true);

        let mut bands = [0.0; 10];
        bands[0] = 12.0;
        bands[1] = -12.0;
        bands[4] = 6.0;
        let eq = EqState {
            bands,
            preamp: -6.0,
            enabled: false,
            auto: true,
        };

        window.sync_from_eq_state(&eq);

        assert_eq!(window.state().bands[0], 0); // 12 dB -> 0
        assert_eq!(window.state().bands[1], 63); // -12 dB -> 63
        assert_eq!(window.state().bands[4], eq_classic_from_db(6.0) as u8);
        assert_eq!(window.state().preamp, eq_classic_from_db(-6.0) as u8);
        assert!(!window.state().enabled);
        assert!(window.state().auto);
        // Preserves UI-only flags:
        assert!(!window.state().focused);
        assert!(window.state().double_size);
    }

    /// S05 §1.2: the shade is a title strip, so the body controls are not
    /// drawn there and must not answer a click.
    #[test]
    fn the_shaded_equalizer_answers_neither_the_buttons_nor_the_sliders() {
        let mut window = EqualizerWindow::new();
        window.set_windowshade(true);
        assert!(window.state().windowshade);

        // The On button (14,18,25,12).
        assert!(window.mouse_down(20, 24).is_empty());
        assert_eq!(window.state().held, None);
        assert!(window.mouse_up(20, 24).is_empty());
        assert!(window.state().enabled, "the On button must not toggle");

        // The Auto button (40,18,32,12).
        assert!(window.mouse_down(55, 24).is_empty());
        assert!(window.mouse_up(55, 24).is_empty());
        assert!(!window.state().auto, "the Auto button must not toggle");

        // The Presets button (217,18,44,12).
        assert!(window.mouse_down(239, 24).is_empty());
        assert!(window.mouse_up(239, 24).is_empty());
        assert_eq!(window.take_menu_request(), None);

        // The preamp and a band: a press commits nothing and starts no drag.
        let preamp = window.state().preamp;
        assert!(window.mouse_down(22, 95).is_empty());
        assert_eq!(window.state().preamp, preamp);
        assert_eq!(window.state().held, None);
        assert!(window.mouse_move(22, 40).is_empty());
        assert_eq!(window.state().preamp, preamp);
        assert!(window.mouse_up(22, 40).is_empty());
        assert_eq!(window.state().preamp, preamp);

        let band = window.state().bands[0];
        assert!(window.mouse_down(79, 95).is_empty());
        assert_eq!(window.state().bands[0], band);
        assert!(window.mouse_up(79, 95).is_empty());
        assert_eq!(window.state().bands[0], band);
    }

    /// The unshaded path is untouched: the same clicks still act.
    #[test]
    fn the_unshaded_equalizer_still_answers_the_body_controls() {
        let mut window = EqualizerWindow::new();
        assert!(!window.state().windowshade);

        assert!(window.mouse_down(20, 24).is_empty());
        assert_eq!(window.state().held, Some(EqWidget::On));
        assert_eq!(window.mouse_up(20, 24), vec![Command::SetEqEnabled(false)]);

        assert!(window.mouse_down(239, 24).is_empty());
        assert!(window.mouse_up(239, 24).is_empty());
        assert_eq!(window.take_menu_request(), Some(EqMenuRequest::Presets));

        let commands = window.mouse_down(22, 95);
        assert_eq!(commands.len(), 1);
        assert!(matches!(commands[0], Command::SetPreamp(_)));
        assert!(window.state().preamp > FLAT_VALUE);
    }

    /// Shading drops a running slider drag: the sliders are gone with the
    /// body (S05 §1.2).
    #[test]
    fn shading_cancels_a_running_slider_drag() {
        let mut window = EqualizerWindow::new();
        assert!(window.mouse_down(22, 65).is_empty());
        assert_eq!(
            window.state().held,
            Some(EqWidget::Slider(EqSlider::Preamp))
        );

        window.set_windowshade(true);

        assert_eq!(window.state().held, None);
        assert!(!window.state().thumb_dragged);
        let preamp = window.state().preamp;
        // The drag does not continue behind the shade.
        assert!(window.mouse_move(22, 40).is_empty());
        assert_eq!(window.state().preamp, preamp);
    }
}
