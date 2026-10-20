// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # The classic analyser modes
//!
//! Rustamp's own spectrum analyser and oscilloscope, in the modes a classic
//! player offers: **spectrum bars with falling peak markers**, a **triggered
//! oscilloscope** whose trace does not slide, and **off**.
//!
//! ## What the audited S08 settles
//!
//! `specs/S08-visualization.md` is audited and settles the numbers this module
//! used to choose for itself:
//!
//! * the area and its band count: **76 x 16 px with ten bands** in the normal
//!   main window and **38 x 5 px with five bands** in the shaded one (S08 §1);
//! * the level scale: a level is kept in **sixteenths of a pixel**, and the
//!   drawn height is the stored value divided by sixteen, rounded down
//!   (S08 §2.1);
//! * the fall-off: a **fixed step per update**, from a five-entry table that
//!   the `safalloff` setting selects (S08 §2.2);
//! * the peak markers: one per band or none (`sa_peaks`, S08 §3.1), and a
//!   **multiplicative factor per update** from a five-entry table that the
//!   `sa_peak_falloff` setting selects (S08 §3.2).
//!
//! ## What stays an own choice
//!
//! S08 leaves the rest open, and every own choice below says so where it is
//! made: the band frequency edges (open question 1), the update rate (open
//! question 2), the height law, the bar/gap split (open question 4) and the
//! **direction of the peak marker's travel** (open question 11).
//!
//! ## The oscilloscope
//!
//! S08 settles nothing about the scope: its **sampling** is open question 3
//! and the **`sa` value that selects it** is open question 5. Our own
//! choices, each of them pinned by a test in this module:
//!
//! * **The sampling rule** (open question 3): the trace takes **every n-th
//!   sample** of the span it covers, never an averaged bucket. Every drawn
//!   value is therefore an actual sample of the signal, which is what a
//!   scope is for; the price is that content above half the column rate
//!   folds back into the trace.
//! * **The span**: [`SCOPE_SPAN_SAMPLES`] samples, the same in every area, so
//!   the shaded area shows the same stretch of time with half the
//!   resolution - the rule S08 §1 uses for the bands.
//! * **The geometry**: one column per pixel of the area - 76 in the normal
//!   window, 38 in the shaded one - so the trace takes every second sample
//!   in the normal area and every fourth in the shaded one. The double-size
//!   window is the 1x frame scaled 2x (S01 §1), so its scope is the normal
//!   trace scaled: 76 samples over 152 px, the same span.
//! * **The trigger**: a Schmitt edge on the mean-removed mono mix. The
//!   qualifying edge closest to the middle of the usable range wins, so the
//!   drawn trace stays where it is while the window slides. A window that
//!   holds no qualifying edge is anchored at the **nominal trigger** - the
//!   middle of the usable range - and the analyser reports **no trigger**:
//!   the picture is drawn, but it slides.
//! * **The `sa` value** (open question 5): `0` selects the oscilloscope, `1`
//!   the bar display and `2` switches the analyser off; every other value
//!   falls back to the bar display. `1` is the observed value of the
//!   reference (S08 §4) and the bar display is the analyser's default, so
//!   the two agree. S12 §3.5 carries the same mapping as an *inferred*
//!   reading; S08 does not promote it, so this is our own choice.
//!
//! ## What this module does and does not do
//!
//! * The DSP is pure. [`analyse`] turns a frame into band magnitudes,
//!   [`magnitude_rows`] turns a magnitude into rows and [`find_trigger`]
//!   finds the scope's anchor. Each is a function of its arguments alone, so
//!   each is testable on its own.
//! * [`Analyser`] adds the state - the levels, the peak markers and the
//!   trigger - and reads no clock: the caller passes the time in
//!   milliseconds.
//! * The renderer is the caller's business. This module returns bar heights
//!   and peak heights in rows and the scope trace as samples; colours, pixel
//!   layout and the dot/line stroke are not decided here.
//!
//! ## The modes
//!
//! [`ClassicMode::Spectrum`] is the default and is what the player shows
//! today: bars with a separate peak marker that rests on the top of the bar
//! and then falls. [`ClassicMode::Oscilloscope`] draws the triggered trace.
//! [`ClassicMode::Off`] computes nothing.
//!
//! ## Relationship to the main window's analyser
//!
//! The main window uses this analyser through `rustamp-ui`, which supplies
//! the transport-aware clock and draws the returned rows and trace using the
//! skin palette. Dot versus line strokes remain a renderer choice.

use std::fmt;

use rustamp_core::vis::{VIS_SAMPLES, VisFrame};

/// The level scale of S08 §2.1: a level counts **sixteenths of a pixel**.
///
/// A stored value of 16 is one pixel of bar height, and the drawn height in
/// whole pixels is the stored value divided by 16, rounded down.
pub const SIXTEENTHS_PER_PIXEL: u32 = 16;

/// S08 §1: the normal main window's analyser area is 76 px wide.
pub const SPECTRUM_WIDTH: u32 = 76;

/// S08 §1: the normal main window's analyser area is 16 px tall.
pub const SPECTRUM_ROWS: u8 = 16;

/// S08 §1: the normal main window's analyser carries **ten bands**.
pub const SPECTRUM_BANDS: usize = 10;

/// S08 §1: the shaded main window's analyser area is 38 px wide.
pub const SHADED_WIDTH: u32 = 38;

/// S08 §1: the shaded main window's analyser area is 5 px tall.
pub const SHADED_ROWS: u8 = 5;

/// S08 §1: the shaded main window's analyser carries **five bands**.
pub const SHADED_BANDS: usize = 5;

/// Own choice: the half-open DFT-bin boundaries of the ten normal bands.
///
/// S08 §1 settles the band count but not the frequency edges (open question
/// 1), so this table is ours. It follows one rule: the ten bands cover the
/// bins 1..=288 (bin 0 is the DC bin) with a constant frequency ratio, so the
/// axis is logarithmic. The ratio is the tenth root of 289, the number of bins
/// the bands cover, which puts the last edge on the Nyquist bin: the edges
/// are `round(289^(k/10))` for `k = 0..=10`.
///
/// Bin `k` of the 576-point transform sits at `k * rate / 576` Hz, so at
/// 44.1 kHz the bands start at 77, 153, 230, 383, 766, 1302, 2297, 4057, 7122
/// and 12556 Hz. The first two bands hold a single bin each: one bin is
/// already 76.6 Hz at 44.1 kHz, so the low bands cannot be resolved finer.
pub const BAND_EDGES: [usize; SPECTRUM_BANDS + 1] = [1, 2, 3, 5, 10, 17, 30, 53, 93, 164, 289];

/// Own choice: the half-open DFT-bin boundaries of the five shaded bands.
///
/// S08 §1 says the shaded window keeps the band width and halves the band
/// count and the height, so the five shaded bands are the ten normal bands
/// merged pairwise: every second edge of [`BAND_EDGES`]. S08 leaves open
/// whether the shaded analyser uses the same band set (open question 7); we
/// use the same spectrum, half as many bands.
pub const SHADED_BAND_EDGES: [usize; SHADED_BANDS + 1] = [1, 3, 10, 30, 93, 289];

/// S08 §2.2: the fall-off step table, in **sixteenths of a pixel per update**.
///
/// The `safalloff` setting selects the entry and is clamped to 0..=4. The
/// step is looked up, never interpolated. In pixels the five steps are
/// 0.1875, 0.375, 0.75, 1.0 and 2.0 px per update, because a row is one
/// pixel and a pixel is sixteen stored units.
pub const FALL_STEPS: [u32; 5] = [3, 6, 12, 16, 32];

/// S08 §3.2: the peak factor table, one **exact rational** per setting.
///
/// The `sa_peak_falloff` setting selects the entry and is clamped to 0..=4.
/// The five factors are 1.05, 1.10, 1.20, 1.40 and 1.60 per update; they are
/// kept as rationals so the marker moves by an exact integer and never
/// drifts with a floating-point multiplication.
pub const PEAK_FACTORS: [(u32, u32); 5] = [(21, 20), (11, 10), (6, 5), (7, 5), (8, 5)];

/// Own choice: the number of analyser updates per second.
///
/// S08 §2.2 and §3.2 settle the fall-off **per update** and leave the rate
/// open (open question 2), so the rate is ours. Thirty updates a second
/// keeps the fall speed of the previous, time-based law: the default step of
/// twelve sixteenths is 0.75 px, and 0.75 px times 30 updates is 22.5 px per
/// second, close to the 24 px per second the old law used.
pub const UPDATES_PER_SECOND: u32 = 30;

/// Own choice: the seed of the peak marker's fall distance, in sixteenths.
///
/// S08 §3.2 settles the factor but not what it multiplies (open question 11).
/// We multiply the distance the marker has already fallen below the peak it
/// fell from, so the distance starts at one pixel and grows by the factor
/// from there. A seed of zero could not grow: every factor is above one, but
/// zero times any factor is zero.
const PEAK_FALL_SEED: u32 = SIXTEENTHS_PER_PIXEL;

/// Own choice: the display floor in dBFS.
///
/// A -60 dBFS floor gives a quiet-to-loud range that still fits into the
/// area's rows; anything below it draws nothing.
pub const FLOOR_DB: f64 = -60.0;

/// Own choice: the number of columns of the scope trace.
///
/// The widest area is the normal one: 76 px, so one column per pixel
/// (S08 §1). The shaded area has 38 columns.
pub const SCOPE_COLUMNS: usize = SPECTRUM_WIDTH as usize;

/// Own choice: the number of audio samples the scope trace covers.
///
/// The span is the same in every area, so the shaded area shows the same
/// stretch of time as the normal one - with half as many columns, so each of
/// its columns covers twice the time. That is the rule S08 §1 uses for the
/// bands: the shaded window keeps the band width and halves the band count.
///
/// 152 samples are about 3.45 ms at 44.1 kHz. The span divides the normal
/// area's 76 columns and the shaded area's 38 columns exactly, so the trace
/// takes every second, resp. every fourth sample. The scope may therefore
/// only trigger within the first `VIS_SAMPLES - SCOPE_SPAN_SAMPLES` samples
/// of the window.
pub const SCOPE_SPAN_SAMPLES: usize = 152;

/// Own choice: how long an accepted frame keeps the bar floors alive.
///
/// After that the input counts as gone and the bars fall freely. The input
/// is not turned into a silence frame: the analyser simply stops holding
/// the last value up. The window is counted in analyser updates, so it is
/// `FRAME_STALE_MS` of display time at [`UPDATES_PER_SECOND`].
pub const FRAME_STALE_MS: u64 = 100;

/// Own choice: the trigger's hysteresis band, as a fraction of the frame peak.
///
/// The band makes the trigger lock on a full cycle of the signal instead of
/// on every crossing of the centre line, which is what keeps the trace
/// stationary.
pub const DEFAULT_TRIGGER_HYSTERESIS: f32 = 0.1;

/// Own choice: the largest hysteresis the trigger accepts.
///
/// A band of the full peak could never be crossed, so the value is capped
/// below it.
const MAX_TRIGGER_HYSTERESIS: f32 = 0.95;

/// The area the analyser draws in, and with it the band count (S08 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Geometry {
    /// The normal main window: 76 x 16 px with ten bands (S08 §1).
    #[default]
    Normal,
    /// The shaded main window: 38 x 5 px with five bands (S08 §1).
    Shaded,
}

impl Geometry {
    /// The number of bands the area carries (S08 §1).
    pub const fn bands(self) -> usize {
        match self {
            Self::Normal => SPECTRUM_BANDS,
            Self::Shaded => SHADED_BANDS,
        }
    }

    /// The height of the area in pixels (S08 §1).
    pub const fn rows(self) -> u8 {
        match self {
            Self::Normal => SPECTRUM_ROWS,
            Self::Shaded => SHADED_ROWS,
        }
    }

    /// The width of the area in pixels (S08 §1).
    pub const fn width(self) -> u32 {
        match self {
            Self::Normal => SPECTRUM_WIDTH,
            Self::Shaded => SHADED_WIDTH,
        }
    }

    /// The half-open DFT-bin boundaries of the area's bands (own choice).
    pub const fn edges(self) -> &'static [usize] {
        match self {
            Self::Normal => &BAND_EDGES,
            Self::Shaded => &SHADED_BAND_EDGES,
        }
    }

    /// The number of columns of the scope trace in this area (own choice).
    ///
    /// One column per pixel of the area (S08 §1): 76 in the normal window, 38
    /// in the shaded one.
    pub const fn scope_columns(self) -> usize {
        self.width() as usize
    }

    /// The number of samples one column of the scope trace covers (own
    /// choice).
    ///
    /// The span is the same in every area, so the shaded area covers twice
    /// the time per column: the same stretch of time with half the
    /// resolution.
    pub const fn scope_decimation(self) -> usize {
        SCOPE_SPAN_SAMPLES / self.scope_columns()
    }

    /// The number of samples the scope trace covers (own choice).
    ///
    /// The same in every area, so every geometry shows the same stretch of
    /// time.
    pub const fn scope_span(self) -> usize {
        self.scope_columns() * self.scope_decimation()
    }
}

/// The classic modes a caller can pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClassicMode {
    /// Bars with a falling peak marker (the default, and what the player
    /// shows today).
    #[default]
    Spectrum,
    /// The oscilloscope, locked to a stable trigger.
    Oscilloscope,
    /// Nothing is drawn and nothing is computed.
    Off,
}

impl ClassicMode {
    /// The S09 `analyzer_get_mode` bits of this mode.
    ///
    /// S09 states the layout of that return value: bit 0 (1) spectrum, bit 1
    /// (2) oscilloscope, 4 for none. Which of our modes carries which bit is
    /// our own choice.
    pub fn analyzer_bits(self) -> u32 {
        match self {
            Self::Spectrum => 1,
            Self::Oscilloscope => 2,
            Self::Off => 4,
        }
    }

    /// Own choice: the mode an `sa` value selects (S08 open question 5).
    ///
    /// `0` is the oscilloscope, `1` the bar display and `2` switches the
    /// analyser off; every other value falls back to the default mode, the
    /// bar display.
    ///
    /// S08 §4 records `1` as the observed value of the reference, and the bar
    /// display is the analyser's default, so the two agree. S12 §3.5 carries
    /// the same mapping as an *inferred* reading; S08 does not promote it, so
    /// this mapping is our own choice.
    pub fn from_sa_value(value: u8) -> Self {
        match value {
            0 => Self::Oscilloscope,
            1 => Self::Spectrum,
            2 => Self::Off,
            _ => Self::default(),
        }
    }
}

/// The edge the oscilloscope locks on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TriggerEdge {
    /// Lock on a rising edge (the default).
    #[default]
    Rising,
    /// Lock on a falling edge, for material that is inverted.
    Falling,
}

/// The analyser's own choices that a caller may change.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuning {
    /// The fall-off setting of S08 §2.2 (`safalloff`), clamped to 0..=4.
    ///
    /// The observed value of the reference is 2, so the default is 2.
    pub falloff: u8,
    /// Whether peak markers are drawn (S08 §3.1, `sa_peaks`).
    ///
    /// The observed value of the reference is 1, so the default is on.
    pub peaks: bool,
    /// The peak fall-off setting of S08 §3.2 (`sa_peak_falloff`), clamped
    /// to 0..=4.
    ///
    /// The observed value of the reference is 1, so the default is 1.
    pub peak_falloff: u8,
    /// The edge the oscilloscope locks on.
    pub trigger_edge: TriggerEdge,
    /// The trigger's hysteresis band, as a fraction of the frame peak.
    pub trigger_hysteresis: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            falloff: 2,
            peaks: true,
            peak_falloff: 1,
            trigger_edge: TriggerEdge::default(),
            trigger_hysteresis: DEFAULT_TRIGGER_HYSTERESIS,
        }
    }
}

impl Tuning {
    /// The fall-off step in sixteenths of a pixel per update (S08 §2.2).
    ///
    /// The setting is clamped to 0..=4: a value of 4 or more behaves like 4.
    pub fn fall_step(&self) -> u32 {
        FALL_STEPS[(self.falloff as usize).min(FALL_STEPS.len() - 1)]
    }

    /// The peak factor per update, as an exact rational (S08 §3.2).
    ///
    /// The setting is clamped to 0..=4: a value of 4 or more behaves like 4.
    pub fn peak_factor(&self) -> (u32, u32) {
        PEAK_FACTORS[(self.peak_falloff as usize).min(PEAK_FACTORS.len() - 1)]
    }
}

/// A frame cannot be analysed honestly with these inputs.
///
/// These are analysis errors, not playback errors: reporting one never
/// touches transport, playback or engine state, and never substitutes
/// samples for the ones that were refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyserError {
    /// The source sample rate is not supplied. Never assume 44.1 kHz.
    MissingSampleRate,
    /// The frame carries a non-finite sample.
    NonFinitePcm,
}

impl fmt::Display for AnalyserError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSampleRate => write!(f, "the analyser needs the source sample rate"),
            Self::NonFinitePcm => write!(f, "the frame carries a non-finite sample"),
        }
    }
}

impl std::error::Error for AnalyserError {}

/// The result of analysing one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    /// The largest normalised magnitude per band, over both channels.
    ///
    /// A full-scale sine on an integer bin reaches 1.0; the value is linear
    /// in the amplitude, not logarithmic. Only the bands of the geometry that
    /// was analysed are filled; the rest are zero.
    pub magnitudes: [f32; SPECTRUM_BANDS],
}

/// The frequency of DFT bin `bin` at the given source sample rate.
///
/// `None` when the rate is missing: the bands cannot be placed in the
/// frequency domain without it.
pub fn bin_frequency_hz(bin: usize, sample_rate_hz: u32) -> Option<f64> {
    if sample_rate_hz == 0 {
        return None;
    }
    Some(bin as f64 * f64::from(sample_rate_hz) / VIS_SAMPLES as f64)
}

/// Analyse the frame's waveform into band magnitudes.
///
/// The transform is a direct 576-point transform of the frame's actual
/// samples with a periodic Hann window; the channel mean is removed before
/// the window is applied. The normalisation makes an integer-bin full-scale
/// sine 1.0 and does not double the Nyquist bin. The larger magnitude of
/// the two channels wins, so an out-of-phase stereo pair does not cancel.
///
/// The bands are the ones of the given area (S08 §1): ten in the normal
/// main window, five in the shaded one.
///
/// The frame's `spectrum` field is ignored: it carries the engine's neutral
/// data, not a measurement.
pub fn analyse(
    frame: &VisFrame,
    sample_rate_hz: u32,
    geometry: Geometry,
) -> Result<Analysis, AnalyserError> {
    check_frame(frame, sample_rate_hz)?;
    Ok(analyse_validated(frame, geometry))
}

/// The height law: -60..0 dBFS maps to 0..=`rows` rows.
///
/// The nearest row wins, anything above the floor is drawn, and a magnitude
/// that is not finite or not positive draws nothing. The floor in dBFS and
/// the shape of the curve are our own choice; the *scale* of the result is
/// the area's height (S08 §1).
pub fn magnitude_rows(magnitude: f32, rows: u8) -> u8 {
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return 0;
    }
    let db = 20.0 * f64::from(magnitude).log10();
    let value = ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0) * f64::from(rows);
    value.round() as u8
}

/// The signal the oscilloscope draws: the mono mix of the two channels with
/// its mean removed.
///
/// The arithmetic mean of the channels is an own choice, so an out-of-phase
/// stereo pair cancels in the scope - the spectrum instead keeps the larger
/// of the two channels. Removing the mean centres the trigger level and
/// drops a constant offset from the trace, so the trigger behaves the same
/// for material that carries a DC offset.
pub fn scope_signal(frame: &VisFrame) -> [f32; VIS_SAMPLES] {
    let mut signal = std::array::from_fn(|n| (frame.waveform[0][n] + frame.waveform[1][n]) * 0.5);
    let mean = signal.iter().map(|&v| f64::from(v)).sum::<f64>() / VIS_SAMPLES as f64;
    for value in &mut signal {
        *value -= mean as f32;
    }
    signal
}

/// The position the oscilloscope falls back to when the window holds no
/// trigger: the middle of the range the scope may trigger in.
pub fn nominal_trigger(window: &[f32]) -> usize {
    window.len().saturating_sub(SCOPE_SPAN_SAMPLES) / 2
}

/// Find the point the oscilloscope locks on.
///
/// The trigger is a Schmitt edge on the mean-removed signal: for a **rising**
/// edge the signal must first fall to `-level` and then cross `level`
/// upwards; for a **falling** edge it must first rise to `level` and then
/// cross `-level` downwards. `level = hysteresis * peak`, and `peak` is the
/// largest magnitude in the window. The band between the two levels is what
/// makes the trigger lock on a whole cycle of the signal instead of on every
/// crossing of the centre line, and that is what keeps the drawn trace
/// stationary.
///
/// Among the qualifying edges the one closest to [`nominal_trigger`] wins;
/// a tie goes to the earlier edge. `None` when the window holds no
/// qualifying edge, for instance because it is silent: the analyser then
/// anchors the trace at the nominal trigger and reports no trigger.
pub fn find_trigger(window: &[f32], edge: TriggerEdge, hysteresis: f32) -> Option<usize> {
    let usable = window.len().checked_sub(SCOPE_SPAN_SAMPLES)?;
    let peak = window
        .iter()
        .fold(0.0f32, |largest, value| largest.max(value.abs()));
    if peak <= 0.0 {
        return None;
    }
    let level = hysteresis.clamp(0.0, MAX_TRIGGER_HYSTERESIS) * peak;
    let nominal = usable / 2;
    let mut armed = false;
    let mut best: Option<usize> = None;
    for index in 0..=usable {
        let current = window[index];
        let next = window[index + 1];
        let fires = match edge {
            TriggerEdge::Rising => {
                if current <= -level {
                    armed = true;
                }
                armed && current < level && next >= level
            }
            TriggerEdge::Falling => {
                if current >= level {
                    armed = true;
                }
                armed && current > -level && next <= -level
            }
        };
        if fires {
            best = Some(match best {
                Some(current) => {
                    if index.abs_diff(nominal) < current.abs_diff(nominal) {
                        index
                    } else {
                        current
                    }
                }
                None => index,
            });
        }
    }
    best
}

/// Build the scope trace of a window, anchored at the trigger.
///
/// The trace holds one sample per column of the area and takes **every n-th
/// sample** of the span (own choice, S08 open question 3): column `k` is the
/// sample `k * n` after the anchor, where `n` is the area's decimation.
/// Nothing is averaged and nothing is interpolated, so every drawn value is
/// an actual sample of the signal.
///
/// Only the area's columns are the trace; the rest of the returned array is
/// zero. A trigger outside the usable range is clamped into it, so the trace
/// never reads past the end of the window; a window that is too short yields
/// zeros.
pub fn scope_trace(window: &[f32], trigger: usize, geometry: Geometry) -> [f32; SCOPE_COLUMNS] {
    let columns = geometry.scope_columns();
    let decimation = geometry.scope_decimation();
    let usable = window.len().saturating_sub(SCOPE_SPAN_SAMPLES);
    let start = trigger.min(usable);
    let mut trace = [0.0; SCOPE_COLUMNS];
    for (column, slot) in trace.iter_mut().enumerate().take(columns) {
        *slot = window
            .get(start + column * decimation)
            .copied()
            .unwrap_or(0.0);
    }
    trace
}

/// The classic analyser: the mode, the tuning, the area and the display state.
///
/// The analyser keeps no clock of its own. Every entry point takes the
/// current time in milliseconds, so the same input always produces the same
/// output. The fall-off and the peak markers move in **updates** (S08 §2.2,
/// §3.2): the elapsed time is turned into a number of updates at
/// [`UPDATES_PER_SECOND`].
#[derive(Debug, Clone)]
pub struct Analyser {
    mode: ClassicMode,
    tuning: Tuning,
    geometry: Geometry,
    /// The bar heights in sixteenths of a pixel.
    bars: [u32; SPECTRUM_BANDS],
    /// The last analysed level per band, in whole pixels.
    levels: [u8; SPECTRUM_BANDS],
    /// The peak marker heights in sixteenths of a pixel.
    peaks: [u32; SPECTRUM_BANDS],
    /// The peak the marker of a band fell from, in sixteenths of a pixel.
    peak_origin: [u32; SPECTRUM_BANDS],
    /// The distance the marker has already fallen below its peak.
    peak_fall: [u32; SPECTRUM_BANDS],
    /// The updates left before the input counts as gone.
    fresh_updates: u32,
    /// The scope trace; only the area's columns belong to it.
    scope: [f32; SCOPE_COLUMNS],
    trigger: Option<usize>,
    clock_ms: u64,
    /// The updates owed, in thousandths of an update.
    pending_updates: u64,
}

impl Default for Analyser {
    fn default() -> Self {
        Self {
            mode: ClassicMode::default(),
            tuning: Tuning::default(),
            geometry: Geometry::default(),
            bars: [0; SPECTRUM_BANDS],
            levels: [0; SPECTRUM_BANDS],
            peaks: [0; SPECTRUM_BANDS],
            peak_origin: [0; SPECTRUM_BANDS],
            peak_fall: [0; SPECTRUM_BANDS],
            fresh_updates: 0,
            scope: [0.0; SCOPE_COLUMNS],
            trigger: None,
            clock_ms: 0,
            pending_updates: 0,
        }
    }
}

impl Analyser {
    /// The selected mode.
    pub fn mode(&self) -> ClassicMode {
        self.mode
    }

    /// Select a mode and clear the display.
    ///
    /// The mode decides which data the analyser computes; the caller draws
    /// it. A mode switch starts the new mode from an empty display, so no
    /// stale value of the other mode can show up.
    pub fn set_mode(&mut self, mode: ClassicMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        self.reset();
    }

    /// The current tuning.
    pub fn tuning(&self) -> &Tuning {
        &self.tuning
    }

    /// Replace the tuning. The change applies to the frames that follow.
    pub fn set_tuning(&mut self, tuning: Tuning) {
        self.tuning = tuning;
    }

    /// The area the analyser draws in (S08 §1).
    pub fn geometry(&self) -> Geometry {
        self.geometry
    }

    /// Switch to another area and clear the display.
    ///
    /// The band count belongs to the area (S08 §1), so the new bands have no
    /// history: the display starts empty.
    pub fn set_geometry(&mut self, geometry: Geometry) {
        if self.geometry == geometry {
            return;
        }
        self.geometry = geometry;
        self.reset();
    }

    /// The bar heights in rows, 0..=the area's rows.
    pub fn bars(&self) -> [u8; SPECTRUM_BANDS] {
        std::array::from_fn(|band| self.row(self.bars[band]))
    }

    /// The peak marker heights in rows, 0..=the area's rows.
    pub fn peaks(&self) -> [u8; SPECTRUM_BANDS] {
        std::array::from_fn(|band| self.row(self.peaks[band]))
    }

    /// The scope trace, one sample per column of the area.
    ///
    /// The slice holds the area's columns: 76 in the normal window, 38 in
    /// the shaded one (S08 §1).
    pub fn scope(&self) -> &[f32] {
        &self.scope[..self.geometry.scope_columns()]
    }

    /// The point the last scope trace is locked on, if the window held a
    /// trigger.
    pub fn trigger(&self) -> Option<usize> {
        self.trigger
    }

    /// Clear the display without changing the mode or the tuning.
    ///
    /// The clock keeps running, so a timestamp that goes backwards cannot
    /// resurrect an old envelope.
    pub fn reset(&mut self) {
        self.bars = [0; SPECTRUM_BANDS];
        self.levels = [0; SPECTRUM_BANDS];
        self.peaks = [0; SPECTRUM_BANDS];
        self.peak_origin = [0; SPECTRUM_BANDS];
        self.peak_fall = [0; SPECTRUM_BANDS];
        self.fresh_updates = 0;
        self.scope = [0.0; SCOPE_COLUMNS];
        self.trigger = None;
    }

    /// Let the display move on for the time that passed since the last call.
    ///
    /// The fall-off is a fixed step **per update** (S08 §2.2), so the elapsed
    /// time is turned into a number of updates. The count is exact and does
    /// not depend on how the elapsed time is split into calls: the updates
    /// that are owed are kept in thousandths, so a long run neither loses nor
    /// gains an update.
    pub fn advance(&mut self, now_ms: u64) {
        let start = self.clock_ms;
        let now = now_ms.max(start);
        self.clock_ms = now;
        let elapsed = now - start;
        if elapsed == 0 {
            return;
        }
        self.pending_updates += elapsed * u64::from(UPDATES_PER_SECOND);
        let updates = (self.pending_updates / 1000) as u32;
        self.pending_updates %= 1000;
        for _ in 0..updates {
            self.step();
        }
    }

    /// Accept one frame of actual audio.
    ///
    /// The frame is refused as a whole when it carries a non-finite sample
    /// or when the source rate is missing; a refused frame changes nothing,
    /// not even the envelopes. The time is the caller's own clock in
    /// milliseconds; it is only used for the envelopes, never for the audio
    /// itself.
    pub fn push(
        &mut self,
        frame: &VisFrame,
        sample_rate_hz: u32,
        now_ms: u64,
    ) -> Result<(), AnalyserError> {
        check_frame(frame, sample_rate_hz)?;
        self.advance(now_ms);
        match self.mode {
            ClassicMode::Spectrum => {
                let analysis = analyse_validated(frame, self.geometry);
                for band in 0..self.geometry.bands() {
                    let level = magnitude_rows(analysis.magnitudes[band], self.geometry.rows());
                    self.levels[band] = level;
                    let level16 = u32::from(level) * SIXTEENTHS_PER_PIXEL;
                    if level16 >= self.bars[band] {
                        // A rising bar jumps to the new level at once
                        // (S08 §2.1), and the marker rides on it.
                        self.bars[band] = level16;
                        self.peaks[band] = level16;
                        self.peak_origin[band] = level16;
                        self.peak_fall[band] = 0;
                    }
                }
                self.fresh_updates = fresh_updates();
            }
            ClassicMode::Oscilloscope => {
                let signal = scope_signal(frame);
                let trigger = find_trigger(
                    &signal,
                    self.tuning.trigger_edge,
                    self.tuning.trigger_hysteresis,
                );
                self.scope = scope_trace(
                    &signal,
                    trigger.unwrap_or_else(|| nominal_trigger(&signal)),
                    self.geometry,
                );
                self.trigger = trigger;
            }
            ClassicMode::Off => {}
        }
        Ok(())
    }

    /// One analyser update: the bars fall by the configured step and the peak
    /// markers descend.
    ///
    /// A rising bar is instantaneous (S08 §2.1): a bar that is at or below
    /// the current level jumps to it. A bar that is above the level falls by
    /// the fixed step of the table (S08 §2.2) and rests on the level while
    /// the input is fresh; once the input is gone it falls freely to the
    /// bottom.
    ///
    /// The marker descends towards the bar. The distance it has already
    /// fallen below the peak it fell from grows by the factor of the table
    /// (S08 §3.2), so the marker's step grows with every update (S08 §3.3).
    /// S08 leaves the direction of that travel open (open question 11); ours
    /// is the falling one, because that is the only direction in which a
    /// peak marker still means "the peak the bar reached".
    fn step(&mut self) {
        let step = self.tuning.fall_step();
        let (num, den) = self.tuning.peak_factor();
        let fresh = self.fresh_updates > 0;
        if fresh {
            self.fresh_updates -= 1;
        }
        for band in 0..self.geometry.bands() {
            // The level is only live while the input is fresh. Once it is
            // gone the bar has nothing to rest on and falls freely.
            let level = if fresh {
                u32::from(self.levels[band]) * SIXTEENTHS_PER_PIXEL
            } else {
                0
            };
            if level >= self.bars[band] {
                self.bars[band] = level;
            } else {
                // The bar falls by the fixed step and rests on the level while
                // the input is fresh. Once the input is gone the level is
                // zero, so the bar falls freely to the bottom.
                self.bars[band] = self.bars[band].saturating_sub(step).max(level);
            }
            if self.tuning.peaks {
                let marker = self.peak_origin[band].saturating_sub(self.peak_fall[band]);
                if self.bars[band] >= marker {
                    // The bar reached the marker: it is re-seated on the top
                    // of the bar and starts falling again from there.
                    self.peaks[band] = self.bars[band];
                    self.peak_origin[band] = self.bars[band];
                    self.peak_fall[band] = 0;
                } else {
                    // The marker descends. Its distance below the peak grows
                    // by the factor, so its step grows with every update.
                    self.peak_fall[band] = peak_fall_step(self.peak_fall[band], (num, den));
                    let marker = self.peak_origin[band].saturating_sub(self.peak_fall[band]);
                    // The marker never draws below the bar it belongs to.
                    self.peaks[band] = marker.max(self.bars[band]);
                }
            } else {
                self.peaks[band] = 0;
                self.peak_origin[band] = 0;
                self.peak_fall[band] = 0;
            }
        }
    }

    /// The drawn height of a stored level: the stored value divided by
    /// sixteen, rounded down (S08 §2.1).
    fn row(&self, level: u32) -> u8 {
        let row = level / SIXTEENTHS_PER_PIXEL;
        row.min(u32::from(self.geometry.rows())) as u8
    }
}

/// The number of updates an accepted frame keeps the bar floors alive.
fn fresh_updates() -> u32 {
    (FRAME_STALE_MS * u64::from(UPDATES_PER_SECOND) / 1000) as u32
}

/// The distance the peak marker has fallen after one more update (S08 §3.2).
///
/// The distance below the peak grows by the factor of the table, so the
/// marker's step grows with every update. The first step after the marker was
/// re-seated is the seed, because a distance of zero could not grow: every
/// factor is above one, but zero times any factor is zero.
fn peak_fall_step(fall: u32, (num, den): (u32, u32)) -> u32 {
    if fall == 0 {
        PEAK_FALL_SEED
    } else {
        (fall * num + den / 2) / den
    }
}

/// Refuse a frame that cannot be analysed.
fn check_frame(frame: &VisFrame, sample_rate_hz: u32) -> Result<(), AnalyserError> {
    if sample_rate_hz == 0 {
        return Err(AnalyserError::MissingSampleRate);
    }
    if frame
        .waveform
        .iter()
        .flatten()
        .any(|value| !value.is_finite())
    {
        return Err(AnalyserError::NonFinitePcm);
    }
    Ok(())
}

/// The transform of an already checked frame.
fn analyse_validated(frame: &VisFrame, geometry: Geometry) -> Analysis {
    let window: [f64; VIS_SAMPLES] = std::array::from_fn(|n| {
        0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / VIS_SAMPLES as f64).cos()
    });
    let gain: f64 = window.iter().sum();
    let edges = geometry.edges();
    let mut magnitudes = [0.0f32; SPECTRUM_BANDS];
    for channel in &frame.waveform {
        let mean = channel.iter().map(|&x| f64::from(x)).sum::<f64>() / VIS_SAMPLES as f64;
        let samples: [f64; VIS_SAMPLES] =
            std::array::from_fn(|n| (f64::from(channel[n]) - mean) * window[n]);
        for (band, magnitude) in magnitudes.iter_mut().enumerate().take(geometry.bands()) {
            for bin in edges[band]..edges[band + 1] {
                let angle = std::f64::consts::TAU * bin as f64 / VIS_SAMPLES as f64;
                let (sin, cos) = angle.sin_cos();
                let (mut re, mut im) = (0.0, 0.0);
                let (mut phase_re, mut phase_im) = (1.0, 0.0);
                for &sample in &samples {
                    re += sample * phase_re;
                    im -= sample * phase_im;
                    (phase_re, phase_im) = (
                        phase_re * cos - phase_im * sin,
                        phase_im * cos + phase_re * sin,
                    );
                }
                let factor = if bin == VIS_SAMPLES / 2 { 1.0 } else { 2.0 };
                // Clipping affects the display amplitude only, not the audio.
                let amplitude = (re.hypot(im) * factor / gain).min(1.0) as f32;
                *magnitude = magnitude.max(amplitude);
            }
        }
    }
    Analysis { magnitudes }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame whose two channels carry the same sine.
    ///
    /// The sine comes from a table of one period, so a window that slides
    /// over it holds the same values again and a test can compare two
    /// windows directly.
    fn sine(start: usize, period: usize, amplitude: f32) -> VisFrame {
        let table: Vec<f32> = (0..period)
            .map(|n| {
                let phase = n as f32 * std::f32::consts::TAU / period as f32;
                phase.sin() * amplitude
            })
            .collect();
        let wave = std::array::from_fn(|n| table[(start + n) % period]);
        VisFrame {
            waveform: [wave, wave],
            ..VisFrame::default()
        }
    }

    /// The largest difference between two traces.
    fn trace_difference(left: &[f32; SCOPE_COLUMNS], right: &[f32; SCOPE_COLUMNS]) -> f32 {
        left.iter()
            .zip(right.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max)
    }

    /// The band of the normal geometry that holds a bin.
    fn band_of(bin: usize) -> usize {
        (0..SPECTRUM_BANDS)
            .find(|&band| (BAND_EDGES[band]..BAND_EDGES[band + 1]).contains(&bin))
            .expect("the bin is in a band")
    }

    /// A silent frame: every level is zero.
    fn silence() -> VisFrame {
        VisFrame::default()
    }

    /// A deterministic noise signal in -1..1.
    ///
    /// The generator is a fixed bit mixer, so the test is reproducible
    /// without a dependency and without a random seed.
    fn noise(seed: u32) -> Vec<f32> {
        (0..VIS_SAMPLES)
            .map(|n| {
                let mut x = seed
                    .wrapping_mul(2_246_822_519)
                    .wrapping_add(n as u32)
                    .wrapping_add(1);
                x ^= x >> 13;
                x = x.wrapping_mul(0x5bd1_e995);
                x ^= x >> 15;
                (x as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }

    #[test]
    fn a_tone_scales_to_its_amplitude_and_lands_in_its_band() {
        // A period of 36 samples is bin 16 of the 576-point transform.
        let analysis = analyse(&sine(0, 36, 1.0), 44100, Geometry::Normal).unwrap();
        let loudest = analysis.magnitudes.iter().copied().fold(0.0f32, f32::max);
        assert!((loudest - 1.0).abs() < 0.0001, "{loudest}");
        assert!((analysis.magnitudes[band_of(16)] - 1.0).abs() < 0.0001);
        // Half the amplitude is half the magnitude: the scaling is linear.
        let half = analyse(&sine(0, 36, 0.5), 44100, Geometry::Normal).unwrap();
        assert!((half.magnitudes[band_of(16)] - 0.5).abs() < 0.0001);
        // A different bin lands in a different band.
        let other = analyse(&sine(0, 4, 0.5), 44100, Geometry::Normal).unwrap();
        assert!((other.magnitudes[band_of(144)] - 0.5).abs() < 0.0001);
        // The bands carry the frequency of their bins.
        assert_eq!(bin_frequency_hz(16, 44100), Some(1225.0));
        assert_eq!(bin_frequency_hz(288, 48000), Some(24000.0));
        assert_eq!(bin_frequency_hz(16, 0), None);
    }

    #[test]
    fn silence_and_dc_have_no_spectrum_and_plugin_spectrum_is_ignored() {
        for dc in [0.0, -0.5, 1.0] {
            let frame = VisFrame {
                waveform: [[dc; VIS_SAMPLES]; 2],
                ..VisFrame::default()
            };
            assert_eq!(
                analyse(&frame, 44100, Geometry::Normal).unwrap().magnitudes,
                [0.0; SPECTRUM_BANDS]
            );
            assert_eq!(scope_signal(&frame), [0.0; VIS_SAMPLES]);
        }
        let mut frame = sine(0, 36, 0.5);
        let expected = analyse(&frame, 44100, Geometry::Normal).unwrap();
        frame.spectrum = [[f32::NAN; VIS_SAMPLES]; 2];
        assert_eq!(analyse(&frame, 44100, Geometry::Normal).unwrap(), expected);
    }

    #[test]
    fn independent_channels_do_not_cancel_and_two_tones_survive() {
        let mono = sine(0, 36, 0.5);
        let expected = analyse(&mono, 44100, Geometry::Normal).unwrap().magnitudes;
        let mut frame = mono.clone();
        frame.waveform[1].fill(0.0);
        assert_eq!(
            analyse(&frame, 44100, Geometry::Normal).unwrap().magnitudes,
            expected
        );
        frame.waveform.swap(0, 1);
        assert_eq!(
            analyse(&frame, 44100, Geometry::Normal).unwrap().magnitudes,
            expected
        );
        frame = mono;
        frame.waveform[1]
            .iter_mut()
            .for_each(|sample| *sample = -*sample);
        assert_eq!(
            analyse(&frame, 44100, Geometry::Normal).unwrap().magnitudes,
            expected
        );
        assert_eq!(scope_signal(&frame), [0.0; VIS_SAMPLES]);
        frame.waveform[1] = sine(0, 4, 0.25).waveform[0];
        let two = analyse(&frame, 44100, Geometry::Normal).unwrap();
        assert!((two.magnitudes[band_of(16)] - 0.5).abs() < 0.00001);
        assert!((two.magnitudes[band_of(144)] - 0.25).abs() < 0.00001);
    }

    #[test]
    fn bands_cover_the_half_spectrum_once_and_nyquist_is_not_doubled() {
        for geometry in [Geometry::Normal, Geometry::Shaded] {
            let bins: Vec<_> = geometry
                .edges()
                .windows(2)
                .flat_map(|pair| pair[0]..pair[1])
                .collect();
            assert_eq!(bins, (1..=288).collect::<Vec<_>>());
        }
        let frame = VisFrame {
            waveform: [std::array::from_fn(|n| if n % 2 == 0 { 0.5 } else { -0.5 }); 2],
            ..VisFrame::default()
        };
        assert!(
            (analyse(&frame, 48000, Geometry::Normal).unwrap().magnitudes[band_of(288)] - 0.5)
                .abs()
                < 0.00001
        );
    }

    #[test]
    fn the_height_law_is_the_own_d_b_scale() {
        assert_eq!(magnitude_rows(1.0, SPECTRUM_ROWS), SPECTRUM_ROWS);
        assert_eq!(magnitude_rows(0.5, SPECTRUM_ROWS), 14);
        assert_eq!(magnitude_rows(0.25, SPECTRUM_ROWS), 13);
        assert_eq!(magnitude_rows(0.01, SPECTRUM_ROWS), 5);
        assert_eq!(magnitude_rows(0.001, SPECTRUM_ROWS), 0);
        assert_eq!(magnitude_rows(0.0, SPECTRUM_ROWS), 0);
        assert_eq!(magnitude_rows(-1.0, SPECTRUM_ROWS), 0);
        assert_eq!(magnitude_rows(2.0, SPECTRUM_ROWS), SPECTRUM_ROWS);
        assert_eq!(magnitude_rows(f32::NAN, SPECTRUM_ROWS), 0);
        // The shaded area is five rows tall (S08 §1): the same law, less room.
        assert_eq!(magnitude_rows(1.0, SHADED_ROWS), SHADED_ROWS);
        assert_eq!(magnitude_rows(0.5, SHADED_ROWS), 4);
        assert_eq!(magnitude_rows(0.0, SHADED_ROWS), 0);
    }

    /// S08 §1: ten bands in the normal area, five in the shaded one.
    #[test]
    fn the_band_count_follows_the_area() {
        assert_eq!(Geometry::Normal.bands(), 10);
        assert_eq!(Geometry::Shaded.bands(), 5);
        assert_eq!(Geometry::Normal.rows(), 16);
        assert_eq!(Geometry::Shaded.rows(), 5);
        assert_eq!(Geometry::Normal.width(), 76);
        assert_eq!(Geometry::Shaded.width(), 38);
        assert_eq!(BAND_EDGES.len(), SPECTRUM_BANDS + 1);
        assert_eq!(SHADED_BAND_EDGES.len(), SHADED_BANDS + 1);

        // The normal area analyses ten bands, the shaded one five.
        let frame = sine(0, 36, 1.0);
        let normal = analyse(&frame, 44100, Geometry::Normal).unwrap();
        let shaded = analyse(&frame, 44100, Geometry::Shaded).unwrap();
        assert!((normal.magnitudes[band_of(16)] - 1.0).abs() < 0.0001);
        assert!(
            (shaded.magnitudes[SHADED_BAND_EDGES.partition_point(|&edge| edge <= 16) - 1] - 1.0)
                .abs()
                < 0.0001
        );
        assert!(
            shaded.magnitudes[SHADED_BANDS..]
                .iter()
                .all(|magnitude| *magnitude == 0.0),
            "the shaded area has no bands beyond its five"
        );

        // The analyser follows the area: five bands in the shaded window.
        let mut analyser = Analyser::default();
        analyser.push(&frame, 44100, 0).unwrap();
        assert!(analyser.bars()[band_of(16)] > 0);
        analyser.set_geometry(Geometry::Shaded);
        assert_eq!(analyser.geometry(), Geometry::Shaded);
        assert_eq!(analyser.bars(), [0; SPECTRUM_BANDS]);
        analyser.push(&frame, 44100, 0).unwrap();
        let lit = analyser.bars().iter().filter(|height| **height > 0).count();
        assert!(
            (1..=SHADED_BANDS).contains(&lit),
            "the shaded area has five bands"
        );
        assert!(
            analyser.bars()[SHADED_BANDS..]
                .iter()
                .all(|height| *height == 0),
            "the shaded area has no bands beyond its five"
        );
    }

    /// S08 §2.2: the fall-off is the table's step, per update.
    #[test]
    fn the_fall_off_step_is_the_table_entry_per_setting() {
        for (setting, step) in FALL_STEPS.iter().enumerate() {
            let mut analyser = Analyser::default();
            analyser.set_tuning(Tuning {
                falloff: setting as u8,
                ..Tuning::default()
            });
            analyser.push(&sine(0, 36, 1.0), 44100, 0).unwrap();
            let band = band_of(16);
            let full = SPECTRUM_ROWS as u32 * SIXTEENTHS_PER_PIXEL;
            assert_eq!(analyser.bars[band], full);
            // The level drops: the next update falls by exactly the step.
            analyser.push(&silence(), 44100, 0).unwrap();
            analyser.step();
            assert_eq!(
                analyser.bars[band],
                full - step,
                "setting {setting} falls by {step} sixteenths"
            );
        }
        // The step is counted in sixteenths of a pixel: a row is one pixel, so
        // the five steps are 0.1875, 0.375, 0.75, 1.0 and 2.0 rows.
        for (step, rows) in FALL_STEPS.iter().zip([0.1875f32, 0.375, 0.75, 1.0, 2.0]) {
            assert_eq!(*step as f32 / SIXTEENTHS_PER_PIXEL as f32, rows);
        }
        // The setting is clamped to 0..=4 (S08 §2.2).
        let mut clamped = Analyser::default();
        clamped.set_tuning(Tuning {
            falloff: 9,
            ..Tuning::default()
        });
        assert_eq!(clamped.tuning().fall_step(), FALL_STEPS[4]);
        assert_eq!(clamped.tuning().fall_step(), 32);
    }

    /// S08 §2.1: a rising bar jumps to the new level at once.
    #[test]
    fn a_rising_bar_jumps_at_once() {
        let mut analyser = Analyser::default();
        analyser.push(&silence(), 44100, 0).unwrap();
        let band = band_of(16);
        assert_eq!(analyser.bars[band], 0);
        // One frame with a tone: the bar is at the level at once, not after
        // a fall and a rise.
        analyser.push(&sine(0, 36, 1.0), 44100, 0).unwrap();
        assert_eq!(
            analyser.bars[band],
            SPECTRUM_ROWS as u32 * SIXTEENTHS_PER_PIXEL
        );
        assert_eq!(analyser.bars()[band], SPECTRUM_ROWS);
        // A level below the bar does not jump: the bar falls by the step at
        // the next update.
        analyser.push(&silence(), 44100, 0).unwrap();
        assert_eq!(
            analyser.bars[band],
            SPECTRUM_ROWS as u32 * SIXTEENTHS_PER_PIXEL
        );
        analyser.step();
        assert_eq!(
            analyser.bars[band],
            SPECTRUM_ROWS as u32 * SIXTEENTHS_PER_PIXEL - FALL_STEPS[2]
        );
    }

    /// S08 §3.1 and §3.2: the markers are switched off and on, and their
    /// fall distance grows by the factor of the table.
    #[test]
    fn the_peak_markers_follow_the_factor_table() {
        for (setting, (num, den)) in PEAK_FACTORS.iter().enumerate() {
            // The distance below the peak grows by the factor, step by step.
            assert_eq!(peak_fall_step(0, (*num, *den)), PEAK_FALL_SEED);
            let mut fall = PEAK_FALL_SEED;
            for _ in 0..8 {
                let next = peak_fall_step(fall, (*num, *den));
                assert_eq!(next, (fall * num + den / 2) / den, "setting {setting}");
                assert!(next > fall, "the distance grows with every update");
                fall = next;
            }
        }
        // The analyser moves its marker with the same rule.
        let mut analyser = Analyser::default();
        analyser.push(&sine(0, 36, 1.0), 44100, 0).unwrap();
        let band = band_of(16);
        // The marker rests on the top of the bar.
        assert_eq!(analyser.peaks[band], analyser.bars[band]);
        assert_eq!(analyser.peak_fall[band], 0);
        // The bar falls; the marker leaves it and its distance grows.
        analyser.push(&silence(), 44100, 0).unwrap();
        analyser.step();
        assert_eq!(analyser.peak_fall[band], PEAK_FALL_SEED);
        let mut previous = analyser.peak_fall[band];
        for _ in 0..4 {
            analyser.step();
            let current = analyser.peak_fall[band];
            assert_eq!(
                current,
                peak_fall_step(previous, PEAK_FACTORS[1]),
                "the distance grows by the factor"
            );
            previous = current;
        }
        // The marker never falls below the bar it belongs to.
        assert!(analyser.peaks[band] >= analyser.bars[band]);
        // The setting is clamped to 0..=4 (S08 §3.2).
        let mut clamped = Analyser::default();
        clamped.set_tuning(Tuning {
            peak_falloff: 9,
            ..Tuning::default()
        });
        assert_eq!(clamped.tuning().peak_factor(), PEAK_FACTORS[4]);
        assert_eq!(clamped.tuning().peak_factor(), (8, 5));
    }

    /// S08 §3.1: `sa_peaks` = 0 draws no marker at all.
    #[test]
    fn the_peak_markers_can_be_switched_off() {
        let mut analyser = Analyser::default();
        analyser.push(&sine(0, 36, 1.0), 44100, 0).unwrap();
        let band = band_of(16);
        assert!(analyser.peaks[band] > 0);
        analyser.set_tuning(Tuning {
            peaks: false,
            ..Tuning::default()
        });
        analyser.push(&silence(), 44100, 0).unwrap();
        analyser.step();
        assert_eq!(analyser.peaks[band], 0);
        assert!(analyser.bars[band] > 0, "the bars keep going");
    }

    /// A long run must not overflow, drift or hang.
    #[test]
    fn a_long_run_stays_bounded() {
        let mut analyser = Analyser::default();
        let mut time = 0;
        for step in 0..20_000u64 {
            time += 20;
            if step % 3 == 0 {
                analyser
                    .push(&sine((step % 36) as usize, 36, 1.0), 44100, time)
                    .unwrap();
            } else {
                analyser.advance(time);
            }
            assert!(
                analyser
                    .bars()
                    .iter()
                    .all(|height| *height <= SPECTRUM_ROWS)
            );
            assert!(
                analyser
                    .peaks()
                    .iter()
                    .all(|height| *height <= SPECTRUM_ROWS)
            );
            assert!(
                analyser
                    .bars()
                    .iter()
                    .zip(analyser.peaks().iter())
                    .all(|(bar, peak)| peak >= bar),
                "the marker never falls below its bar"
            );
        }
        // After the input is gone the display empties itself.
        analyser.advance(time + 10_000);
        assert_eq!(analyser.bars(), [0; SPECTRUM_BANDS]);
        assert_eq!(analyser.peaks(), [0; SPECTRUM_BANDS]);
    }

    /// The whole law in one receipt: a full-scale tone, then twelve updates
    /// without a new frame, leave the bar at nine rows and the marker at
    /// thirteen.
    #[test]
    fn the_frozen_display_has_the_expected_rows() {
        let mut analyser = Analyser::default();
        analyser.push(&sine(0, 36, 1.0), 44100, 0).unwrap();
        let band = band_of(16);
        assert_eq!(analyser.bars()[band], SPECTRUM_ROWS);
        assert_eq!(analyser.peaks()[band], SPECTRUM_ROWS);
        // 425 ms are twelve updates at thirty a second. The first three
        // still hold the input, the other nine let the bar fall.
        analyser.advance(100);
        analyser.advance(101);
        analyser.advance(300);
        analyser.advance(425);
        assert_eq!(analyser.bars()[band], 9, "256 - 9 x 12 sixteenths");
        assert_eq!(analyser.peaks()[band], 13);
        assert!(analyser.peaks()[band] >= analyser.bars()[band]);
    }

    #[test]
    fn the_trigger_keeps_the_trace_stationary_while_the_window_slides() {
        let mut triggered: Option<[f32; SCOPE_COLUMNS]> = None;
        let mut untriggered: Option<[f32; SCOPE_COLUMNS]> = None;
        let mut trigger_points: Vec<usize> = Vec::new();
        for frame in 0..6 {
            // The window slides over a continuous tone, so an untriggered
            // scope would slide with it.
            let vis = sine(frame * 5, 36, 1.0);
            let signal = scope_signal(&vis);
            let trigger = find_trigger(&signal, TriggerEdge::Rising, DEFAULT_TRIGGER_HYSTERESIS)
                .expect("a tone has a rising edge");
            trigger_points.push(trigger);
            let trace = scope_trace(&signal, trigger, Geometry::Normal);
            let newest = scope_trace(&signal, VIS_SAMPLES - SCOPE_SPAN_SAMPLES, Geometry::Normal);
            match triggered {
                None => {
                    triggered = Some(trace);
                    untriggered = Some(newest);
                }
                Some(reference) => {
                    // The trace is the same, up to the rounding of the
                    // centred signal.
                    assert!(
                        trace_difference(&reference, &trace) < 0.0001,
                        "the triggered trace must not slide"
                    );
                    assert!(
                        trace_difference(&untriggered.unwrap(), &newest) > 0.1,
                        "the untriggered trace slides"
                    );
                }
            }
        }
        // The trigger point itself moves with the window; only the drawn
        // trace stays where it is.
        assert!(
            trigger_points.windows(2).any(|pair| pair[0] != pair[1]),
            "{trigger_points:?}"
        );
    }

    #[test]
    fn the_trigger_ignores_silence_and_picks_the_edge_near_the_middle() {
        let silent = scope_signal(&VisFrame::default());
        assert_eq!(find_trigger(&silent, TriggerEdge::Rising, 0.1), None);
        // A period of 100 samples crosses the centre line five times in the
        // window; the edge closest to the middle of the usable range wins.
        let signal = scope_signal(&sine(0, 100, 1.0));
        let trigger = find_trigger(&signal, TriggerEdge::Rising, 0.1).unwrap();
        assert!((200..=210).contains(&trigger), "got {trigger}");
        // The falling edge of the same signal is a different point.
        let falling = find_trigger(&signal, TriggerEdge::Falling, 0.1).unwrap();
        assert!(falling != trigger);
    }

    /// Own choice (S08 open question 5): which `sa` value selects which mode.
    #[test]
    fn the_sa_value_selects_the_mode() {
        assert_eq!(ClassicMode::from_sa_value(0), ClassicMode::Oscilloscope);
        assert_eq!(ClassicMode::from_sa_value(1), ClassicMode::Spectrum);
        assert_eq!(ClassicMode::from_sa_value(2), ClassicMode::Off);
        // An unknown value falls back to the default mode, the bar display.
        assert_eq!(ClassicMode::from_sa_value(3), ClassicMode::Spectrum);
        assert_eq!(ClassicMode::from_sa_value(255), ClassicMode::Spectrum);
        // `1` is the observed value of the reference (S08 §4) and the bar
        // display is the analyser's default, so the two agree.
        assert_eq!(ClassicMode::default(), ClassicMode::Spectrum);
        assert_eq!(ClassicMode::from_sa_value(1), ClassicMode::default());
    }

    /// Own choice (S08 open question 3): the trace takes every n-th sample.
    #[test]
    fn the_scope_takes_every_nth_sample_of_the_span() {
        // A sawtooth of eight steps: every sample of the ramp carries a
        // different value, so the rule is visible in the trace.
        let ramp: Vec<f32> = (0..VIS_SAMPLES).map(|n| (n % 8) as f32 / 8.0).collect();
        let trace = scope_trace(&ramp, 100, Geometry::Normal);
        // Column k is the sample 2k after the anchor: an actual sample, not
        // the mean of a bucket and not an interpolated value.
        for column in 0..Geometry::Normal.scope_columns() {
            assert_eq!(trace[column], ramp[100 + column * 2], "column {column}");
        }
        // The mean of the bucket around the second column would be 0.625;
        // the decimation gives the sample 0.75. The rule is not an average.
        assert_eq!(trace[1], 0.75);
        assert_ne!(trace[1], 0.625);
        // The shaded area takes every fourth sample and covers the same span.
        let shaded = scope_trace(&ramp, 100, Geometry::Shaded);
        for column in 0..Geometry::Shaded.scope_columns() {
            assert_eq!(shaded[column], ramp[100 + column * 4], "column {column}");
        }
        // The rest of the array is not part of the trace.
        assert!(
            shaded[Geometry::Shaded.scope_columns()..]
                .iter()
                .all(|value| *value == 0.0)
        );
    }

    /// The trigger locks a periodic signal; noise has no phase to lock.
    #[test]
    fn the_trigger_locks_a_periodic_signal_and_not_noise() {
        // A periodic signal: the trace does not move while the window slides
        // over it, because the trigger holds the phase.
        let mut reference: Option<[f32; SCOPE_COLUMNS]> = None;
        for slide in 0..6 {
            let signal: Vec<f32> = (0..VIS_SAMPLES)
                .map(|n| {
                    let phase = std::f32::consts::TAU * ((n + slide * 5) % 36) as f32 / 36.0;
                    phase.sin()
                })
                .collect();
            let trigger = find_trigger(&signal, TriggerEdge::Rising, DEFAULT_TRIGGER_HYSTERESIS)
                .expect("a tone has a rising edge");
            let trace = scope_trace(&signal, trigger, Geometry::Normal);
            match reference {
                None => reference = Some(trace),
                Some(expected) => assert!(
                    trace_difference(&expected, &trace) < 0.0001,
                    "the triggered trace of a periodic signal does not move"
                ),
            }
        }
        // Noise crosses the band everywhere, so the trigger fires - but every
        // window draws another picture, because there is no phase to hold.
        let mut reference: Option<[f32; SCOPE_COLUMNS]> = None;
        for seed in 0..4 {
            let signal = noise(seed);
            let trigger = find_trigger(&signal, TriggerEdge::Rising, DEFAULT_TRIGGER_HYSTERESIS)
                .expect("noise crosses the band");
            let trace = scope_trace(&signal, trigger, Geometry::Normal);
            match reference {
                None => reference = Some(trace),
                Some(expected) => assert!(
                    trace_difference(&expected, &trace) > 0.1,
                    "the trace of noise moves"
                ),
            }
        }
    }

    /// S08 §1: the shaded area has its own scope geometry.
    #[test]
    fn the_shaded_area_has_its_own_scope_geometry() {
        assert_eq!(Geometry::Normal.scope_columns(), 76);
        assert_eq!(Geometry::Shaded.scope_columns(), 38);
        // The normal area takes every second sample, the shaded one every
        // fourth: the same span, half the resolution.
        assert_eq!(Geometry::Normal.scope_decimation(), 2);
        assert_eq!(Geometry::Shaded.scope_decimation(), 4);
        assert_eq!(Geometry::Normal.scope_span(), SCOPE_SPAN_SAMPLES);
        assert_eq!(Geometry::Shaded.scope_span(), SCOPE_SPAN_SAMPLES);
        assert_eq!(Geometry::Normal.scope_span(), Geometry::Shaded.scope_span());
        // The shaded area is five rows tall (S08 §1).
        assert_eq!(Geometry::Shaded.rows(), 5);
        // The analyser follows the area: the shaded trace has 38 columns.
        let mut analyser = Analyser::default();
        analyser.set_mode(ClassicMode::Oscilloscope);
        analyser.set_geometry(Geometry::Shaded);
        analyser.push(&sine(0, 36, 1.0), 44100, 0).unwrap();
        assert_eq!(analyser.scope().len(), 38);
        assert!(analyser.scope().iter().any(|value| *value != 0.0));
        analyser.set_geometry(Geometry::Normal);
        analyser.push(&sine(0, 36, 1.0), 44100, 10).unwrap();
        assert_eq!(analyser.scope().len(), 76);
    }

    /// An empty or a one-sample block must not panic.
    #[test]
    fn an_empty_or_a_single_sample_block_does_not_panic() {
        for block in [vec![], vec![0.5], vec![-0.5]] {
            assert_eq!(
                find_trigger(&block, TriggerEdge::Rising, DEFAULT_TRIGGER_HYSTERESIS),
                None
            );
            assert_eq!(
                find_trigger(&block, TriggerEdge::Falling, DEFAULT_TRIGGER_HYSTERESIS),
                None
            );
            assert_eq!(nominal_trigger(&block), 0);
        }
        // The empty block yields an empty trace.
        let trace = scope_trace(&[], 0, Geometry::Normal);
        assert!(trace.iter().all(|value| *value == 0.0));
        // A single sample is drawn in the first column and nowhere else.
        let trace = scope_trace(&[0.5], 0, Geometry::Normal);
        assert_eq!(trace[0], 0.5);
        assert!(trace[1..].iter().all(|value| *value == 0.0));
        // A trigger beyond the window is clamped into it.
        let trace = scope_trace(&[0.5], 10_000, Geometry::Shaded);
        assert_eq!(trace[0], 0.5);
        assert!(trace[1..].iter().all(|value| *value == 0.0));
    }

    /// Own choice: a window without a trigger is anchored at the nominal one.
    #[test]
    fn a_window_without_a_trigger_is_anchored_at_the_nominal_one() {
        // The nominal trigger is the middle of the range the scope may
        // trigger in.
        let window: Vec<f32> = (0..VIS_SAMPLES).map(|n| n as f32).collect();
        assert_eq!(
            nominal_trigger(&window),
            (VIS_SAMPLES - SCOPE_SPAN_SAMPLES) / 2
        );
        // Silence holds no edge: the analyser reports no trigger and draws
        // the trace at the nominal anchor anyway.
        let mut analyser = Analyser::default();
        analyser.set_mode(ClassicMode::Oscilloscope);
        analyser.push(&silence(), 44100, 0).unwrap();
        assert_eq!(analyser.trigger(), None);
        assert_eq!(analyser.scope().len(), 76);
        assert!(analyser.scope().iter().all(|value| *value == 0.0));
        // A frame whose usable range never leaves the flat part holds no
        // edge either: the trace is anchored in the middle of the range.
        let mut frame = VisFrame::default();
        frame.waveform[0][VIS_SAMPLES - 1] = 1.0;
        frame.waveform[1][VIS_SAMPLES - 1] = 1.0;
        analyser.push(&frame, 44100, 10).unwrap();
        assert_eq!(analyser.trigger(), None);
        let trace = analyser.scope();
        assert!(
            trace
                .iter()
                .all(|value| (*value - trace[0]).abs() < f32::EPSILON),
            "the trace is anchored in the flat part of the window"
        );
        assert!(trace[0] < 0.0);
    }

    #[test]
    fn the_mode_selects_what_the_analyser_computes() {
        assert_eq!(Analyser::default().mode(), ClassicMode::Spectrum);
        assert_eq!(ClassicMode::Spectrum.analyzer_bits(), 1);
        assert_eq!(ClassicMode::Oscilloscope.analyzer_bits(), 2);
        assert_eq!(ClassicMode::Off.analyzer_bits(), 4);

        let mut analyser = Analyser::default();
        analyser.push(&sine(0, 36, 1.0), 44100, 0).unwrap();
        assert_eq!(analyser.bars()[band_of(16)], SPECTRUM_ROWS);
        assert!(analyser.scope().iter().all(|value| *value == 0.0));

        analyser.set_mode(ClassicMode::Oscilloscope);
        assert_eq!(
            analyser.bars()[band_of(16)],
            0,
            "the mode switch clears the display"
        );
        analyser.push(&sine(0, 36, 1.0), 44100, 10).unwrap();
        assert!(analyser.scope().iter().any(|value| *value != 0.0));
        assert!(analyser.trigger().is_some());

        analyser.set_mode(ClassicMode::Off);
        analyser.push(&sine(0, 36, 1.0), 44100, 20).unwrap();
        assert!(analyser.scope().iter().all(|value| *value == 0.0));
        assert_eq!(analyser.trigger(), None);
    }

    #[test]
    fn a_refused_frame_changes_nothing() {
        let mut analyser = Analyser::default();
        analyser.push(&sine(0, 36, 1.0), 44100, 0).unwrap();
        let before = analyser.bars();
        let mut broken = sine(0, 36, 1.0);
        broken.waveform[1][7] = f32::NAN;
        assert_eq!(
            analyser.push(&broken, 44100, 10),
            Err(AnalyserError::NonFinitePcm)
        );
        assert_eq!(
            analyser.push(&sine(0, 36, 1.0), 0, 10),
            Err(AnalyserError::MissingSampleRate)
        );
        assert_eq!(analyser.bars(), before, "a refused frame changes no state");
        // The analyser keeps working afterwards.
        analyser.push(&sine(0, 36, 1.0), 44100, 20).unwrap();
        assert_eq!(analyser.bars(), before);
    }
}
