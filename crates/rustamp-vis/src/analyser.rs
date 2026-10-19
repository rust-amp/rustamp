// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # The classic analyser modes
//!
//! Rustamp's own spectrum analyser and oscilloscope, in the modes a classic
//! player offers: **spectrum bars with falling peak markers**, a **triggered
//! oscilloscope** whose trace does not slide, and **off**.
//!
//! ## Own design, not a measurement
//!
//! Spec **S08** does not exist yet (`specs/README.md` lists it as "not
//! started"), so **every number in this module is an own choice**. Nothing
//! here claims to match a reference player's analyser; each constant says
//! what a future S08 measurement would refine. The one borrowed fact is the
//! S09 `analyzer_get_mode` bit layout, which that audited spec states.
//!
//! ## What this module does and does not do
//!
//! * The DSP is pure. [`analyse`] turns a frame into band magnitudes,
//!   [`magnitude_rows`] turns a magnitude into rows, [`decay_rows`] is the
//!   fall rule and [`find_trigger`] finds the scope's anchor. Each is a
//!   function of its arguments alone, so each is testable on its own.
//! * [`Analyser`] adds the state - attack, hold, fall - and reads no clock:
//!   the caller passes the time in milliseconds.
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
//! the transport-aware clock and draws the returned rows and trace using
//! the skin palette. Dot versus line strokes remain a renderer choice.

use std::fmt;

use rustamp_core::vis::{VIS_SAMPLES, VisFrame};

/// Own choice: the number of spectrum bands (bars) the analyser draws.
///
/// Nineteen bands of four pixels fill the 76-pixel area the main window
/// reserves. An S08 measurement would replace the count and the band table.
pub const SPECTRUM_BANDS: usize = 19;

/// Own choice: the half-open DFT-bin boundaries of the bands.
///
/// Bin 0 (DC) is excluded and every bin from 1 to 288 is used exactly once.
/// Bin `k` of a 576-point transform sits at `k * rate / 576` Hz, so at
/// 44.1 kHz the first bin is already 76.5625 Hz. The bands are therefore
/// *not* nineteen independently resolved low-Hz bands; an S08 measurement
/// would replace the table.
pub const BAND_EDGES: [usize; SPECTRUM_BANDS + 1] = [
    1, 2, 3, 4, 5, 7, 9, 12, 16, 21, 28, 37, 49, 65, 86, 114, 151, 200, 264, 289,
];

/// Own choice: the height of the spectrum display in rows, 0..=`SPECTRUM_ROWS`.
///
/// Sixteen rows fill the analyser area the main window reserves. A peak
/// marker rests on the top row of its bar, so a full-height bar and its
/// marker share that row.
pub const SPECTRUM_ROWS: u8 = 16;

/// Own choice: the display floor in dBFS.
///
/// A -60 dBFS floor gives a quiet-to-loud range that still fits into
/// sixteen rows; anything below it draws nothing.
pub const FLOOR_DB: f64 = -60.0;

/// Own choice: the number of columns of the scope trace.
///
/// The analyser area the main window reserves is 76 pixels wide, so the
/// trace uses one column per pixel.
pub const SCOPE_COLUMNS: usize = 76;

/// Own choice: the number of audio samples the scope draws behind the trigger.
///
/// 192 samples are about 4.4 ms at 44.1 kHz, which shows a few cycles of a
/// musical note. The scope may therefore only trigger within the first
/// `VIS_SAMPLES - SCOPE_SPAN_SAMPLES` samples of the window.
pub const SCOPE_SPAN_SAMPLES: usize = 192;

/// Own choice: the default fall rate of the bars, in rows per second.
///
/// The fall is linear and independent of the paint rate, so a slow and a
/// fast frame rate draw the same envelope.
pub const DEFAULT_BAR_FALL_ROWS_PER_SECOND: f32 = 24.0;

/// Own choice: the default fall rate of the peak marker, in rows per second.
///
/// The marker falls slower than the bars, so it stays readable above them.
pub const DEFAULT_PEAK_FALL_ROWS_PER_SECOND: f32 = 16.0;

/// Own choice: the default rest of the peak marker on its top, in milliseconds.
pub const DEFAULT_PEAK_HOLD_MS: u64 = 300;

/// Own choice: how long an accepted frame keeps the bar floors alive.
///
/// After that the input counts as gone and the bars fall freely. The input
/// is not turned into a silence frame: the analyser simply stops holding
/// the last value up.
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
    /// How fast the bars fall after their attack, in rows per second.
    pub bar_fall_rows_per_second: f32,
    /// How fast the peak marker falls after its hold, in rows per second.
    pub peak_fall_rows_per_second: f32,
    /// How long the peak marker rests on its top, in milliseconds.
    pub peak_hold_ms: u64,
    /// The edge the oscilloscope locks on.
    pub trigger_edge: TriggerEdge,
    /// The trigger's hysteresis band, as a fraction of the frame peak.
    pub trigger_hysteresis: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            bar_fall_rows_per_second: DEFAULT_BAR_FALL_ROWS_PER_SECOND,
            peak_fall_rows_per_second: DEFAULT_PEAK_FALL_ROWS_PER_SECOND,
            peak_hold_ms: DEFAULT_PEAK_HOLD_MS,
            trigger_edge: TriggerEdge::default(),
            trigger_hysteresis: DEFAULT_TRIGGER_HYSTERESIS,
        }
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
    /// in the amplitude, not logarithmic.
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
/// The frame's `spectrum` field is ignored: it carries the engine's neutral
/// data, not a measurement.
pub fn analyse(frame: &VisFrame, sample_rate_hz: u32) -> Result<Analysis, AnalyserError> {
    check_frame(frame, sample_rate_hz)?;
    Ok(analyse_validated(frame))
}

/// The own-choice height law: -60..0 dBFS maps to 0..=`SPECTRUM_ROWS` rows.
///
/// The nearest row wins, anything above the floor is drawn, and a magnitude
/// that is not finite or not positive draws nothing.
pub fn magnitude_rows(magnitude: f32) -> u8 {
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return 0;
    }
    let db = 20.0 * f64::from(magnitude).log10();
    let rows = ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0) * f64::from(SPECTRUM_ROWS);
    rows.round() as u8
}

/// The fall rule: the height drops by `rows_per_second` per second.
///
/// The fall is linear and independent of how often it is applied, so one
/// long step and many short steps reach the same height. The result never
/// goes below zero.
pub fn decay_rows(rows: f32, elapsed_ms: u64, rows_per_second: f32) -> f32 {
    let fallen = elapsed_ms as f32 / 1000.0 * rows_per_second;
    (rows - fallen).max(0.0)
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
/// The trigger is a Schmitt edge: for a rising edge the signal must first
/// fall to `-level` and then cross `level` upwards, with
/// `level = hysteresis * peak` and `peak` the largest magnitude in the
/// window. Because a whole cycle has to pass before the trigger fires
/// again, the trigger locks the *phase* of the signal instead of a single
/// crossing, and the drawn trace stays where it is.
///
/// Among the qualifying edges the one closest to [`nominal_trigger`] wins;
/// a tie goes to the earlier edge. `None` when the window holds no
/// qualifying edge, for instance because it is silent.
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

/// Build the scope trace behind a trigger point.
///
/// The trace is `SCOPE_COLUMNS` samples taken from the span behind the
/// trigger, each the nearest actual sample of that span. A trigger outside
/// the usable range is clamped into it, so the trace never reads past the
/// end of the window.
pub fn scope_trace(window: &[f32], trigger: usize) -> [f32; SCOPE_COLUMNS] {
    let usable = window.len().saturating_sub(SCOPE_SPAN_SAMPLES);
    let start = trigger.min(usable);
    std::array::from_fn(|column| {
        let offset = column * (SCOPE_SPAN_SAMPLES - 1) / (SCOPE_COLUMNS - 1);
        window.get(start + offset).copied().unwrap_or(0.0)
    })
}

/// The classic analyser: the mode, the tuning and the display state.
///
/// The analyser keeps no clock of its own. Every entry point takes the
/// current time in milliseconds, so the same input always produces the same
/// output.
#[derive(Debug, Clone)]
pub struct Analyser {
    mode: ClassicMode,
    tuning: Tuning,
    bars: [f32; SPECTRUM_BANDS],
    peaks: [f32; SPECTRUM_BANDS],
    floors: [f32; SPECTRUM_BANDS],
    peak_hold_until_ms: [u64; SPECTRUM_BANDS],
    scope: [f32; SCOPE_COLUMNS],
    trigger: Option<usize>,
    clock_ms: u64,
    last_frame_ms: Option<u64>,
}

impl Default for Analyser {
    fn default() -> Self {
        Self {
            mode: ClassicMode::default(),
            tuning: Tuning::default(),
            bars: [0.0; SPECTRUM_BANDS],
            peaks: [0.0; SPECTRUM_BANDS],
            floors: [0.0; SPECTRUM_BANDS],
            peak_hold_until_ms: [0; SPECTRUM_BANDS],
            scope: [0.0; SCOPE_COLUMNS],
            trigger: None,
            clock_ms: 0,
            last_frame_ms: None,
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

    /// The bar heights in rows, 0..=`SPECTRUM_ROWS`.
    pub fn bars(&self) -> [u8; SPECTRUM_BANDS] {
        std::array::from_fn(|band| quantise_rows(self.bars[band]))
    }

    /// The peak marker heights in rows, 0..=`SPECTRUM_ROWS`.
    pub fn peaks(&self) -> [u8; SPECTRUM_BANDS] {
        std::array::from_fn(|band| quantise_rows(self.peaks[band]))
    }

    /// The scope trace, one sample per column.
    pub fn scope(&self) -> &[f32; SCOPE_COLUMNS] {
        &self.scope
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
        self.bars = [0.0; SPECTRUM_BANDS];
        self.peaks = [0.0; SPECTRUM_BANDS];
        self.floors = [0.0; SPECTRUM_BANDS];
        self.peak_hold_until_ms = [0; SPECTRUM_BANDS];
        self.scope = [0.0; SCOPE_COLUMNS];
        self.trigger = None;
        self.last_frame_ms = None;
    }

    /// Let the envelopes fall for the time that passed since the last call.
    ///
    /// A caller that paints without a new frame calls this, so the bars and
    /// the peak marker keep falling while the input is gone. A timestamp
    /// that goes backwards is not elapsed time and changes nothing.
    pub fn advance(&mut self, now_ms: u64) {
        let start = self.clock_ms;
        let now = now_ms.max(start);
        let elapsed = now - start;
        self.clock_ms = now;
        if elapsed == 0 {
            return;
        }
        let stale_at = self.last_frame_ms.map(|frame| frame + FRAME_STALE_MS);
        let fresh_ms = stale_at.map_or(elapsed, |stale| elapsed.min(stale.saturating_sub(start)));
        let stale_ms = elapsed - fresh_ms;
        for band in 0..SPECTRUM_BANDS {
            // While the input is fresh the bar rests on the last analysed
            // value; once it is gone the bar falls freely.
            let bar = if fresh_ms > 0 {
                decay_rows(
                    self.bars[band],
                    fresh_ms,
                    self.tuning.bar_fall_rows_per_second,
                )
                .max(self.floors[band])
            } else {
                self.bars[band]
            };
            self.bars[band] = decay_rows(bar, stale_ms, self.tuning.bar_fall_rows_per_second);
            // The peak marker rests for its hold and only then falls.
            let hold_ms = self.peak_hold_until_ms[band]
                .saturating_sub(start)
                .min(elapsed);
            let fall_ms = elapsed - hold_ms;
            let peak = if fall_ms > 0 {
                decay_rows(
                    self.peaks[band],
                    fall_ms,
                    self.tuning.peak_fall_rows_per_second,
                )
            } else {
                self.peaks[band]
            };
            self.peaks[band] = peak.max(self.bars[band]);
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
                let analysis = analyse_validated(frame);
                for (band, magnitude) in analysis.magnitudes.iter().enumerate() {
                    let target = f32::from(magnitude_rows(*magnitude));
                    self.floors[band] = target;
                    // The attack is instant: a bar jumps to the new height.
                    self.bars[band] = self.bars[band].max(target);
                    if target >= self.peaks[band] {
                        self.peaks[band] = target;
                        self.peak_hold_until_ms[band] = self.clock_ms + self.tuning.peak_hold_ms;
                    }
                }
            }
            ClassicMode::Oscilloscope => {
                let signal = scope_signal(frame);
                let trigger = find_trigger(
                    &signal,
                    self.tuning.trigger_edge,
                    self.tuning.trigger_hysteresis,
                );
                self.scope =
                    scope_trace(&signal, trigger.unwrap_or_else(|| nominal_trigger(&signal)));
                self.trigger = trigger;
            }
            ClassicMode::Off => {}
        }
        self.last_frame_ms = Some(self.clock_ms);
        Ok(())
    }
}

/// Quantise a height to whole rows.
///
/// The ceiling keeps a bar that is still falling visible until it really
/// reached the row below.
fn quantise_rows(rows: f32) -> u8 {
    rows.ceil().clamp(0.0, f32::from(SPECTRUM_ROWS)) as u8
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
fn analyse_validated(frame: &VisFrame) -> Analysis {
    let window: [f64; VIS_SAMPLES] = std::array::from_fn(|n| {
        0.5 - 0.5 * (std::f64::consts::TAU * n as f64 / VIS_SAMPLES as f64).cos()
    });
    let gain: f64 = window.iter().sum();
    let mut magnitudes = [0.0f32; SPECTRUM_BANDS];
    for channel in &frame.waveform {
        let mean = channel.iter().map(|&x| f64::from(x)).sum::<f64>() / VIS_SAMPLES as f64;
        let samples: [f64; VIS_SAMPLES] =
            std::array::from_fn(|n| (f64::from(channel[n]) - mean) * window[n]);
        for (band, magnitude) in magnitudes.iter_mut().enumerate() {
            for bin in BAND_EDGES[band]..BAND_EDGES[band + 1] {
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

    /// The band that holds a bin.
    fn band_of(bin: usize) -> usize {
        (0..SPECTRUM_BANDS)
            .find(|&band| (BAND_EDGES[band]..BAND_EDGES[band + 1]).contains(&bin))
            .expect("the bin is in a band")
    }

    #[test]
    fn a_tone_scales_to_its_amplitude_and_lands_in_its_band() {
        // A period of 36 samples is bin 16 of the 576-point transform.
        let analysis = analyse(&sine(0, 36, 1.0), 44100).unwrap();
        let loudest = analysis.magnitudes.iter().copied().fold(0.0f32, f32::max);
        assert!((loudest - 1.0).abs() < 0.0001, "{loudest}");
        assert!((analysis.magnitudes[band_of(16)] - 1.0).abs() < 0.0001);
        // Half the amplitude is half the magnitude: the scaling is linear.
        let half = analyse(&sine(0, 36, 0.5), 44100).unwrap();
        assert!((half.magnitudes[band_of(16)] - 0.5).abs() < 0.0001);
        // A different bin lands in a different band.
        let other = analyse(&sine(0, 4, 0.5), 44100).unwrap();
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
                analyse(&frame, 44100).unwrap().magnitudes,
                [0.0; SPECTRUM_BANDS]
            );
            assert_eq!(scope_signal(&frame), [0.0; VIS_SAMPLES]);
        }
        let mut frame = sine(0, 36, 0.5);
        let expected = analyse(&frame, 44100).unwrap();
        frame.spectrum = [[f32::NAN; VIS_SAMPLES]; 2];
        assert_eq!(analyse(&frame, 44100).unwrap(), expected);
    }

    #[test]
    fn independent_channels_do_not_cancel_and_two_tones_survive() {
        let mono = sine(0, 36, 0.5);
        let expected = analyse(&mono, 44100).unwrap().magnitudes;
        let mut frame = mono.clone();
        frame.waveform[1].fill(0.0);
        assert_eq!(analyse(&frame, 44100).unwrap().magnitudes, expected);
        frame.waveform.swap(0, 1);
        assert_eq!(analyse(&frame, 44100).unwrap().magnitudes, expected);
        frame = mono;
        frame.waveform[1]
            .iter_mut()
            .for_each(|sample| *sample = -*sample);
        assert_eq!(analyse(&frame, 44100).unwrap().magnitudes, expected);
        assert_eq!(scope_signal(&frame), [0.0; VIS_SAMPLES]);
        frame.waveform[1] = sine(0, 4, 0.25).waveform[0];
        let two = analyse(&frame, 44100).unwrap();
        assert!((two.magnitudes[band_of(16)] - 0.5).abs() < 0.00001);
        assert!((two.magnitudes[band_of(144)] - 0.25).abs() < 0.00001);
    }

    #[test]
    fn bands_cover_the_half_spectrum_once_and_nyquist_is_not_doubled() {
        let bins: Vec<_> = BAND_EDGES
            .windows(2)
            .flat_map(|pair| pair[0]..pair[1])
            .collect();
        assert_eq!(bins, (1..=288).collect::<Vec<_>>());
        let frame = VisFrame {
            waveform: [std::array::from_fn(|n| if n % 2 == 0 { 0.5 } else { -0.5 }); 2],
            ..VisFrame::default()
        };
        assert!((analyse(&frame, 48000).unwrap().magnitudes[band_of(288)] - 0.5).abs() < 0.00001);
    }

    #[test]
    fn the_height_law_is_the_own_d_b_scale() {
        assert_eq!(magnitude_rows(1.0), SPECTRUM_ROWS);
        assert_eq!(magnitude_rows(0.5), 14);
        assert_eq!(magnitude_rows(0.25), 13);
        assert_eq!(magnitude_rows(0.01), 5);
        assert_eq!(magnitude_rows(0.001), 0);
        assert_eq!(magnitude_rows(0.0), 0);
        assert_eq!(magnitude_rows(-1.0), 0);
        assert_eq!(magnitude_rows(2.0), SPECTRUM_ROWS);
        assert_eq!(magnitude_rows(f32::NAN), 0);
    }

    #[test]
    fn the_fall_rule_is_linear_and_never_negative() {
        assert_eq!(decay_rows(16.0, 0, 24.0), 16.0);
        assert_eq!(decay_rows(16.0, 500, 24.0), 4.0);
        assert_eq!(decay_rows(16.0, 1000, 24.0), 0.0);
        assert_eq!(decay_rows(1.0, 10_000, 24.0), 0.0);
        // One long step and many short steps reach the same height.
        let mut stepped = 16.0f32;
        for _ in 0..10 {
            stepped = decay_rows(stepped, 50, 24.0);
        }
        assert!((stepped - decay_rows(16.0, 500, 24.0)).abs() < 0.0001);
    }

    #[test]
    fn the_bars_fall_at_the_configured_rate_and_stop_at_the_last_value() {
        let mut analyser = Analyser::default();
        analyser.push(&sine(0, 36, 1.0), 44100, 0).unwrap();
        let band = band_of(16);
        assert_eq!(analyser.bars()[band], SPECTRUM_ROWS);
        // While the input is fresh the bar rests on the last value.
        analyser.advance(50);
        assert_eq!(analyser.bars()[band], SPECTRUM_ROWS);
        // Once the input is gone the bar falls at the configured rate.
        analyser.advance(350);
        assert_eq!(
            analyser.bars()[band],
            10,
            "16 rows minus 24 rows/s over 250 ms"
        );
        // A slower fall rate is honoured.
        let mut slow = Analyser::default();
        slow.set_tuning(Tuning {
            bar_fall_rows_per_second: 4.0,
            ..Tuning::default()
        });
        slow.push(&sine(0, 36, 1.0), 44100, 0).unwrap();
        slow.advance(350);
        assert_eq!(slow.bars()[band], 15, "4 rows/s over 250 ms");
    }

    #[test]
    fn stale_floors_do_not_raise_the_bars_again_on_later_refreshes() {
        let mut once = Analyser::default();
        once.push(&sine(0, 36, 1.0), 44100, 0).unwrap();
        let mut many = once.clone();
        once.advance(425);
        for time in [25, 80, 100, 101, 180, 299, 300, 425] {
            many.advance(time);
        }
        assert_eq!(once.bars(), many.bars());
        assert_eq!(once.peaks(), many.peaks());
        many.advance(2000);
        many.advance(2001);
        assert_eq!(many.bars(), [0; SPECTRUM_BANDS]);
        assert_eq!(many.peaks(), [0; SPECTRUM_BANDS]);
    }

    #[test]
    fn the_peak_marker_holds_then_falls_and_stays_above_the_bar() {
        let mut analyser = Analyser::default();
        analyser.push(&sine(0, 36, 1.0), 44100, 0).unwrap();
        let band = band_of(16);
        assert_eq!(analyser.peaks()[band], SPECTRUM_ROWS);
        analyser.advance(299);
        assert_eq!(
            analyser.peaks()[band],
            SPECTRUM_ROWS,
            "the hold lasts 300 ms"
        );
        analyser.advance(300);
        assert_eq!(
            analyser.peaks()[band],
            SPECTRUM_ROWS,
            "the fall starts after the hold"
        );
        analyser.advance(425);
        assert_eq!(analyser.peaks()[band], 14, "16 rows/s after the hold");
        // The marker never drops below the bar it belongs to.
        assert!(analyser.peaks()[band] >= analyser.bars()[band]);
        // A new frame that reaches the top renews the marker and its hold.
        analyser.push(&sine(0, 36, 1.0), 44100, 500).unwrap();
        assert_eq!(analyser.peaks()[band], SPECTRUM_ROWS);
        analyser.advance(800);
        assert_eq!(
            analyser.peaks()[band],
            SPECTRUM_ROWS,
            "the hold starts again"
        );
        // The fall rate of the marker is configurable, too.
        let mut slow = Analyser::default();
        slow.set_tuning(Tuning {
            peak_fall_rows_per_second: 4.0,
            ..Tuning::default()
        });
        slow.push(&sine(0, 36, 1.0), 44100, 0).unwrap();
        slow.advance(600);
        assert_eq!(
            slow.peaks()[band],
            15,
            "4 rows/s over the 300 ms after the hold"
        );
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
            let trace = scope_trace(&signal, trigger);
            let newest = scope_trace(&signal, VIS_SAMPLES - SCOPE_SPAN_SAMPLES);
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
