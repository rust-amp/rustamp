// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Main-window rendering and transport adapter for the shared classic analyser.
//!
//! All signal analysis, band/height laws, envelopes and triggering live in
//! [`rustamp_vis::analyser`], which follows the audited S08 for the area and
//! the band count, the level scale, the fall-off step and the peak factor.
//! What S08 leaves open - the band frequency edges, the update rate and the
//! direction of the peak marker's travel - stays an own choice there.
//!
//! The scope's own choices live there as well (S08 open questions 3 and 5):
//! the trace takes **every n-th sample** of the span it covers, the normal
//! area draws 76 columns and the shaded area 38 - both over the same span -
//! and the double-size window shows the normal trace scaled 2x, because the
//! whole window is the 1x frame scaled 2x (S01 §1). This module only draws
//! what the analyser returns: one sample per column of the area.
//!
//! This module owns skin pixels, scope strokes, freshness and a pause-aware
//! display clock, and it draws the analyser in the area the main window
//! currently has: 76 x 16 px with ten bands, or 38 x 5 px with five bands
//! while the window is shaded (S08 §1). Analysis runs on the UI side, never
//! in the audio callback.
//!
//! No synthetic audio, padding, interpolation or plugin-spectrum interpretation:
//! only the tap's actual 2 x 576 waveform values enter the transform. The caller
//! must supply the **source** sample rate (not the device rate or rounded kHz).
//! The frame contract contains no rate, sequence/seek generation, or playback
//! selection metadata. Its timestamp is retained, not treated as a wall clock.
//! Decode-ahead/audio-to-pixel synchronisation remains open. A missing or stale
//! frame is not silence: the scope disappears and the existing bars/peaks decay.
//! A frame carrying a non-finite sample is rejected as an *invalid frame*, never
//! as a playback error: [`VisError::NonFinitePcm`] is an offline validation
//! invariant that the live path cannot reach (RFC 0014 §4.5) and must not halt
//! playback.
//!
//! The colours are the skin's own: the renderer reads the 24 slots of the
//! loaded skin's `viscolor.txt` (S17 §6, see [`slots`]) and invents no colour
//! of its own. A skin without a `viscolor.txt` falls back to the built-in
//! default list (`rustamp_skin::webamp::DEFAULT_VISCOLORS`), so the palette
//! always comes from the skin that is loaded.

use rustamp_core::vis::VisFrame;
use rustamp_gfx::{Image, Rect};
use rustamp_vis::analyser::{Analyser, AnalyserError, ClassicMode, FRAME_STALE_MS, SCOPE_COLUMNS};
pub use rustamp_vis::analyser::{Geometry, SPECTRUM_BANDS};

use crate::state::TransportState;

/// The width of a bar in pixels.
///
/// Own choice (S08 §1 leaves the bar/gap split open, question 4): the bar
/// takes the whole band share but the last pixel, so the ten bands of the
/// 76 px area and the five bands of the 38 px area are drawn alike.
const BAR_GAP: u32 = 1;

/// The `viscolor.txt` slot numbers and what they mean (S17 §6).
///
/// S17 §6 lists the slots of a skin's `viscolor.txt` and the meaning of each
/// one. The renderer draws with these slots and with no other colour, so the
/// map below is the whole contract between a skin and the analyser.
pub mod slots {
    /// The analyser background: the whole analyser area.
    pub const BACKGROUND: usize = 0;
    /// The grid dots, drawn on every second pixel.
    pub const GRID: usize = 1;
    /// The first spectrum-gradient slot: the **top** row of a bar.
    pub const SPECTRUM_TOP: usize = 2;
    /// The last spectrum-gradient slot: the **bottom** row of a bar.
    pub const SPECTRUM_BOTTOM: usize = 17;
    /// The first oscilloscope slot: the brightest of the five.
    pub const OSCILLOSCOPE_FIRST: usize = 18;
    /// The last oscilloscope slot: the dimmest of the five.
    pub const OSCILLOSCOPE_LAST: usize = 22;
    /// The peak marker that rides above a falling bar.
    pub const PEAK: usize = 23;
}

/// UI render choices and switching order, all **own choices** (Q9 open).
/// Both scope styles use the shared [`ClassicMode::Oscilloscope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisMode {
    /// Row-coloured bars with separate peak markers (the own-choice default).
    #[default]
    Spectrum,
    /// Connected, one-pixel oscilloscope, arithmetic stereo mean.
    Scope,
    /// The same samples as unconnected dots.
    ScopeDots,
    /// No analyser at all: the area keeps the skin's own art. Does not
    /// disable the engine tap or a plugin.
    Off,
}

impl VisMode {
    fn classic_mode(self) -> ClassicMode {
        match self {
            Self::Spectrum => ClassicMode::Spectrum,
            Self::Scope | Self::ScopeDots => ClassicMode::Oscilloscope,
            Self::Off => ClassicMode::Off,
        }
    }

    /// Cycle locally; no reference gesture or saved preference is claimed.
    pub fn next(self) -> Self {
        match self {
            Self::Spectrum => Self::Scope,
            Self::Scope => Self::ScopeDots,
            Self::ScopeDots => Self::Off,
            Self::Off => Self::Spectrum,
        }
    }
}

/// Whether the pixels have a recent actual input (not a silence substitute).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VisInput {
    /// No accepted frame since construction/reset/stop.
    #[default]
    Missing,
    /// A supplied frame is at most [`FRAME_STALE_MS`] of active UI time old.
    Fresh,
    /// Input stopped arriving. Old bars/peaks may still be falling.
    Stale,
}

/// Integer-only drawing state: exact pixels do not depend on floating paint math.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisState {
    /// The selected UI-local mode.
    pub mode: VisMode,
    /// Current bar heights, 0..=16 rows.
    ///
    /// The array holds every band of the widest area; the shaded area uses
    /// the first five of them (S08 §1).
    pub bars: [u8; SPECTRUM_BANDS],
    /// Peak heights, 0..=16 rows; zero means no marker.
    pub peaks: [u8; SPECTRUM_BANDS],
    /// Scope row per column, 0..=15; used only with actual fresh input.
    ///
    /// The array holds every column of the widest area; the shaded area
    /// fills the first 38 of them (S08 §1).
    pub scope: [u8; SCOPE_COLUMNS],
    /// Freshness rather than a fabricated quiet frame.
    pub input: VisInput,
    /// Unmodified end-of-window timestamp from the last accepted tap frame.
    pub source_timestamp_us: Option<u64>,
}

impl Default for VisState {
    fn default() -> Self {
        Self {
            mode: VisMode::Spectrum,
            bars: [0; SPECTRUM_BANDS],
            peaks: [0; SPECTRUM_BANDS],
            scope: [8; SCOPE_COLUMNS],
            input: VisInput::Missing,
            source_timestamp_us: None,
        }
    }
}

/// A frame cannot be analysed honestly with these inputs.
///
/// These are **analysis** errors, not playback errors: reporting one never
/// changes transport, playback or engine state and never substitutes samples.
/// The live caller `MainWindow::push_vis_frame` only notes them through the
/// app's bounded diagnostic route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum VisError {
    /// The source sample rate is not supplied. Never assume 44.1 kHz.
    #[error("visualisation needs the source sample rate")]
    MissingSampleRate,
    /// An **offline validation invariant**, not a live-pipeline error.
    ///
    /// An isolated test harness, a synthetic unit test or a custom offline
    /// analysis rejects a whole frame that carries a non-finite sample instead
    /// of inventing replacement samples.
    ///
    /// On the live path this arm cannot fire: `rustamp-audio` replaces every
    /// non-finite sample a stage produces with `0.0` before the frame reaches
    /// the visualiser tap (RFC 0014 §4.5, defence in depth), and a stage that
    /// detects its own non-finite output must return `Err` on the decode path
    /// instead of relying on this check.
    ///
    /// It is therefore never a pipeline-halting audio error. An offline
    /// analysis reports the frame as invalid and continues; playback state is
    /// not touched.
    #[error("visualisation frame contains non-finite PCM")]
    NonFinitePcm,
}

impl From<AnalyserError> for VisError {
    fn from(error: AnalyserError) -> Self {
        match error {
            AnalyserError::MissingSampleRate => Self::MissingSampleRate,
            AnalyserError::NonFinitePcm => Self::NonFinitePcm,
        }
    }
}

/// Transport adapter for the shared analyser. No wall-clock reads.
#[derive(Debug, Clone, Default)]
pub struct Visualizer {
    state: VisState,
    analyser: Analyser,
    active_ms: u64,
    last_ms: Option<u64>,
    last_input: Option<u64>,
    last_transport: TransportState,
    sample_rate_hz: Option<u32>,
}

impl Visualizer {
    /// The current deterministic pixel state.
    pub fn state(&self) -> &VisState {
        &self.state
    }

    /// Clear history at seek/track change, keeping the UI-local mode.
    pub fn reset(&mut self) {
        let mode = self.state.mode;
        self.analyser.reset();
        self.state = VisState::default();
        self.state.mode = mode;
        self.last_input = None;
        self.sample_rate_hz = None;
    }

    /// Switch to the other analyser area (S08 §1) and clear the display.
    ///
    /// The band count belongs to the area, so the new bands start without a
    /// history: the display is cleared and the mode is kept.
    pub fn set_geometry(&mut self, geometry: Geometry) {
        if self.analyser.geometry() == geometry {
            return;
        }
        self.analyser.set_geometry(geometry);
        self.reset();
    }

    /// Set a local render mode without changing plugin data or audio.
    pub fn set_mode(&mut self, mode: VisMode) {
        if self.analyser.mode() != mode.classic_mode() {
            self.analyser.set_mode(mode.classic_mode());
            self.reset();
        }
        self.state.mode = mode;
    }

    /// Advance using milliseconds supplied by the presenter. Pause freezes all
    /// envelopes and freshness; stop clears the screen immediately in all modes
    /// (decay is applied only while playback continues). Backwards clocks cannot
    /// advance time.
    pub fn advance(&mut self, now_ms: u64, transport: TransportState) {
        let elapsed = self.last_ms.map_or(0, |last| now_ms.saturating_sub(last));
        self.last_ms = Some(self.last_ms.unwrap_or(now_ms).max(now_ms));
        if transport != TransportState::Paused && self.last_transport != TransportState::Paused {
            self.active_ms = self.active_ms.saturating_add(elapsed);
        }
        self.last_transport = transport;
        if transport == TransportState::Paused {
            return;
        }
        if transport == TransportState::Stopped {
            self.reset();
            return;
        }
        self.analyser.advance(self.active_ms);
        let expires = self.last_input.map(|t| t.saturating_add(FRAME_STALE_MS));
        self.state.input = match expires {
            None => VisInput::Missing,
            Some(t) if self.active_ms > t => VisInput::Stale,
            Some(_) => VisInput::Fresh,
        };
        self.update_pixels();
    }

    /// Accept one actual frame, with an unrounded source rate. Paused/stopped
    /// input is discarded, not queued for resume. Invalid input returns an error;
    /// it never becomes a fake silence frame. A rejected frame is reported only
    /// ([`VisError::NonFinitePcm`] is an offline validation invariant) and never
    /// halts playback or clears the last accepted display state. Call regularly
    /// with the newest frame the engine offers; arrival time is an own-choice
    /// display clock.
    pub fn push(
        &mut self,
        frame: &VisFrame,
        sample_rate_hz: u32,
        now_ms: u64,
        transport: TransportState,
    ) -> Result<(), VisError> {
        self.advance(now_ms, transport);
        if transport != TransportState::Playing {
            return Ok(());
        }
        // Validate on a cleared clone at a rate change, so a refused frame
        // cannot discard the previous display. No second transform is needed.
        if self
            .sample_rate_hz
            .is_some_and(|rate| rate != sample_rate_hz)
        {
            let mut analyser = self.analyser.clone();
            analyser.reset();
            analyser.push(frame, sample_rate_hz, self.active_ms)?;
            self.analyser = analyser;
        } else {
            self.analyser.push(frame, sample_rate_hz, self.active_ms)?;
        }
        self.sample_rate_hz = Some(sample_rate_hz);
        self.last_input = Some(self.active_ms);
        self.state.input = VisInput::Fresh;
        self.state.source_timestamp_us = Some(frame.timestamp_us);
        self.update_pixels();
        Ok(())
    }

    fn update_pixels(&mut self) {
        self.state.bars = self.analyser.bars();
        self.state.peaks = self.analyser.peaks();
        let rows = self.analyser.geometry().rows();
        // The trace has one sample per column of the area; the shaded area
        // fills the first 38 of the 76 slots and leaves the rest alone.
        for (slot, sample) in self.state.scope.iter_mut().zip(self.analyser.scope()) {
            *slot = scope_row(*sample, rows);
        }
    }
}

// Projection of the shared trace onto skin rows; positive samples point up.
fn scope_row(sample: f32, rows: u8) -> u8 {
    ((1.0 - f64::from(sample).clamp(-1.0, 1.0)) * f64::from(rows - 1) * 0.5).round() as u8
}

/// The slot of the spectrum gradient row `row`, counted from the bar's bottom.
///
/// S17 §6: slots 2..17 are the 16-colour gradient, **slot 2 at the top of the
/// bar and slot 17 at the bottom**. The base skin's own file names slot 2 "top
/// of spec", and our Rust theme names slot 2 "top spectrum peak" and slot 17
/// "bottom baseline ember", so the ramp runs from the top row down.
///
/// A bar that is shorter than the gradient - the shaded area is five rows
/// tall (S08 §1) - shows the whole ramp in its own rows.
fn spectrum_slot(row: u8, rows: u8) -> usize {
    let span = u32::from(rows - 1).max(1);
    let ramp = (slots::SPECTRUM_BOTTOM - slots::SPECTRUM_TOP) as u32;
    let offset = u32::from(row) * ramp / span;
    slots::SPECTRUM_BOTTOM - offset as usize
}

/// The slot of the oscilloscope row `row` in an area of `rows` rows.
///
/// S17 §6: slots 18..22 are the five oscilloscope colours, slot 18 the
/// brightest and the later ones progressively dimmer. Which sample wears which
/// colour is not determined by the spec, so it stays an own choice: the trace
/// steps one slot away from the centre line per row, and the outermost rows
/// share the dimmest colour. The quiet trace therefore keeps the skin's
/// brightest oscilloscope colour.
fn oscilloscope_slot(row: u8, rows: u8) -> usize {
    let row = i32::from(row);
    // The centre line lies between the two middle rows.
    let low = i32::from((rows - 1) / 2);
    let high = i32::from(rows / 2);
    let distance = (row - low).abs().min((row - high).abs()) as usize;
    let steps = slots::OSCILLOSCOPE_LAST - slots::OSCILLOSCOPE_FIRST;
    slots::OSCILLOSCOPE_FIRST + distance.min(steps)
}

/// Whether the analyser has something to show in its area.
///
/// Measured 2026-10-21 (base-2.91 client capture of Winamp 2.95): while
/// nothing plays, the reference draws **no** analyser - the skin's own
/// `main.bmp` shows through the whole area. The area is therefore painted
/// only when the display has content, and the skin's own art is what an empty
/// analyser shows (S08 open question 9).
fn has_content(state: &VisState, geometry: Geometry) -> bool {
    let bands = geometry.bands().min(SPECTRUM_BANDS);
    match state.mode {
        VisMode::Spectrum => {
            state.bars[..bands].iter().any(|bar| *bar > 0)
                || state.peaks[..bands].iter().any(|peak| *peak > 0)
        }
        VisMode::Scope | VisMode::ScopeDots => state.input == VisInput::Fresh,
        VisMode::Off => false,
    }
}

/// Paint only within `area`. Every colour comes from the skin's palette
/// (`viscolor.txt`, S17 §6), so a skin change repaints the analyser in the new
/// colours without any other wiring.
///
/// The geometry is the area the main window currently has (S08 §1): the
/// normal window draws 76 x 16 px with ten bands, the shaded window 38 x 5 px
/// with five.
///
/// An area with nothing to show is not painted at all: the skin's own art
/// stays visible there (measured 2026-10-21).
///
/// | Slot | Drawn as |
/// |---|---|
/// | 0 | the analyser background |
/// | 1 | the grid dots, on every second pixel |
/// | 2..17 | the bar rows, slot 2 at the top of a bar and slot 17 at the bottom |
/// | 18..22 | the oscilloscope trace, one step per row away from the centre line |
/// | 23 | the peak marker above a falling bar |
///
/// The grid pitch, the stroke styles and the row the oscilloscope colour
/// steps from are own choices (Q2/Q4/Q9 open).
pub fn draw_visualization(
    canvas: &mut Image,
    area: Rect,
    geometry: Geometry,
    state: &VisState,
    colors: &[[u8; 3]; 24],
) {
    if !has_content(state, geometry) {
        return;
    }
    let color = |slot: usize| {
        let [r, g, b] = colors[slot];
        [r, g, b, 255]
    };
    let rows = geometry.rows();
    let bands = geometry.bands();
    let width = geometry.width();
    // The bar takes the band's share without the last pixel: the gap is at
    // least one pixel wide (own choice, S08 question 4).
    let bar_width = (width / bands as u32).saturating_sub(BAR_GAP);
    // Fixed-size scratch makes even a changed layout safe: clip, never spill.
    let mut image = Image::filled(width, u32::from(rows), color(slots::BACKGROUND));
    for y in (0..i32::from(rows)).step_by(2) {
        for x in (0..width as i32).step_by(2) {
            image.set_pixel(x, y, color(slots::GRID));
        }
    }
    match state.mode {
        VisMode::Spectrum => {
            for band in 0..bands {
                let x = (band as u32 * width / bands as u32) as i32;
                for row in 0..state.bars[band].min(rows) {
                    image.fill_rect(
                        Rect::new(x, i32::from(rows - 1 - row), bar_width, 1),
                        color(spectrum_slot(row, rows)),
                    );
                }
                let peak = state.peaks[band].min(rows);
                if peak > 0 {
                    image.fill_rect(
                        Rect::new(x, i32::from(rows - peak), bar_width, 1),
                        color(slots::PEAK),
                    );
                }
            }
        }
        VisMode::Scope | VisMode::ScopeDots => {
            // The trace has one sample per column of the area (own choice:
            // S08 leaves the oscilloscope's sampling open, question 3). The
            // analyser already took every n-th sample of the span, so the
            // column of the area and the column of the trace are the same.
            // The area is painted only with fresh input, so the trace is there.
            for x in 0..width as i32 {
                let column = x as usize;
                let row = state.scope[column].min(rows - 1);
                if state.mode == VisMode::Scope && x > 0 {
                    let previous = state.scope[column - 1].min(rows - 1);
                    // A one-column vertical bridge connects adjacent samples.
                    // Every row of the bridge wears the colour of its row, so
                    // the trace keeps the skin's brightness ramp.
                    for y in row.min(previous)..=row.max(previous) {
                        image.set_pixel(x, i32::from(y), color(oscilloscope_slot(y, rows)));
                    }
                } else {
                    image.set_pixel(x, i32::from(row), color(oscilloscope_slot(row, rows)));
                }
            }
        }
        VisMode::Off => {}
    }
    canvas.blit_rect(
        &image,
        Rect::new(
            0,
            0,
            area.width.min(width),
            area.height.min(u32::from(rows)),
        ),
        area.x,
        area.y,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::vis::VIS_SAMPLES;
    use rustamp_vis::analyser::{BAND_EDGES, SHADED_BANDS, SHADED_ROWS, SPECTRUM_ROWS};

    fn tone(bin: usize, amplitude: f32) -> VisFrame {
        let wave = std::array::from_fn(|n| {
            (std::f64::consts::TAU * bin as f64 * n as f64 / VIS_SAMPLES as f64).sin() as f32
                * amplitude
        });
        VisFrame {
            waveform: [wave, wave],
            ..VisFrame::default()
        }
    }

    fn band_for(bin: usize) -> usize {
        (0..SPECTRUM_BANDS)
            .find(|&b| (BAND_EDGES[b]..BAND_EDGES[b + 1]).contains(&bin))
            .unwrap()
    }

    #[test]
    fn non_finite_pcm_is_an_offline_invariant_and_never_halts_playback() {
        // The zero-sanitizer in `rustamp-audio` keeps this arm off the live path
        // (RFC 0014 §4.5), so it is an offline validation invariant: it reports
        // an invalid frame and touches nothing else. There is no playback state
        // in this module to halt, and the last accepted display state survives.
        let mut vis = Visualizer::default();
        vis.set_mode(VisMode::Scope);
        vis.push(&tone(16, 1.0), 44100, 0, TransportState::Playing)
            .unwrap();
        let accepted = vis.state().clone();
        let mut bad = tone(16, 1.0);
        bad.waveform[0][7] = f32::NAN;
        // Same instant, so no elapsed display time confuses the comparison.
        assert_eq!(
            vis.push(&bad, 44100, 0, TransportState::Playing),
            Err(VisError::NonFinitePcm)
        );
        assert_eq!(vis.state(), &accepted);
        // A rejected frame does not wedge the analyser: the next finite frame is
        // still accepted and the selected mode is unchanged.
        vis.push(&tone(16, 0.25), 44100, 200, TransportState::Playing)
            .unwrap();
        assert_eq!(vis.state().input, VisInput::Fresh);
        assert_eq!(vis.state().mode, VisMode::Scope);
    }

    /// S08 §2.1 and §2.2: a rising bar jumps at once, a falling one steps
    /// down, and the display empties itself once the input is gone.
    #[test]
    fn the_bar_falls_by_its_step_and_the_display_empties_when_the_input_goes() {
        let mut vis = Visualizer::default();
        vis.push(&tone(16, 1.0), 44100, 0, TransportState::Playing)
            .unwrap();
        let band = band_for(16);
        assert_eq!(
            vis.state.bars[band], SPECTRUM_ROWS,
            "the bar jumps to the level"
        );
        assert_eq!(
            vis.state.peaks[band], SPECTRUM_ROWS,
            "the marker rides on it"
        );
        // The level falls: the bar steps down and never overtakes the marker.
        vis.push(&VisFrame::default(), 44100, 0, TransportState::Playing)
            .unwrap();
        vis.advance(125, TransportState::Playing);
        assert!(vis.state.bars[band] < SPECTRUM_ROWS, "the bar fell");
        assert!(vis.state.peaks[band] >= vis.state.bars[band]);
        assert_eq!(vis.state.input, VisInput::Stale);
        // Once the input is gone the display empties itself.
        vis.advance(1300, TransportState::Playing);
        assert_eq!((vis.state.bars[band], vis.state.peaks[band]), (0, 0));
        // Missing is different from the explicitly supplied silent frame.
        let mut fresh = Visualizer::default();
        assert_eq!(fresh.state.input, VisInput::Missing);
        fresh
            .push(&VisFrame::default(), 44100, 0, TransportState::Playing)
            .unwrap();
        fresh.advance(100, TransportState::Playing);
        assert_eq!(fresh.state.input, VisInput::Fresh);
        fresh.advance(101, TransportState::Playing);
        assert_eq!(fresh.state.input, VisInput::Stale);
    }

    /// S08 §1: the shaded window draws its own analyser, 38 x 5 px with five
    /// bands.
    #[test]
    fn the_shaded_area_carries_five_bands_in_five_rows() {
        let mut vis = Visualizer::default();
        vis.push(&tone(16, 1.0), 44100, 0, TransportState::Playing)
            .unwrap();
        assert!(
            vis.state.bars.iter().any(|height| *height > 0),
            "the tone shows up in the normal area"
        );
        vis.set_geometry(Geometry::Shaded);
        // The area changed, so the display starts empty.
        assert_eq!(vis.state.bars, [0; SPECTRUM_BANDS]);
        vis.push(&tone(16, 1.0), 44100, 0, TransportState::Playing)
            .unwrap();
        assert!(
            vis.state.bars.iter().all(|height| *height <= SHADED_ROWS),
            "a shaded bar is at most five rows tall"
        );
        assert!(
            vis.state.bars[SHADED_BANDS..]
                .iter()
                .all(|height| *height == 0),
            "the shaded area has no bands beyond its five"
        );
    }

    #[test]
    fn envelopes_are_independent_of_refresh_partition_and_pause_duration() {
        let mut once = Visualizer::default();
        once.push(&tone(16, 1.0), 44100, 0, TransportState::Playing)
            .unwrap();
        let mut many = once.clone();
        once.advance(425, TransportState::Playing);
        for time in [25, 80, 100, 101, 180, 299, 300, 425] {
            many.advance(time, TransportState::Playing);
        }
        assert_eq!(once.state(), many.state());
        let frozen = many.state.clone();
        many.advance(425, TransportState::Paused);
        many.advance(10000, TransportState::Paused);
        many.advance(10000, TransportState::Playing);
        assert_eq!(many.state, frozen);
        many.advance(10125, TransportState::Playing);
        once.advance(550, TransportState::Playing);
        assert_eq!(once.state(), many.state());
        let unchanged = once.state.clone();
        once.advance(1, TransportState::Playing);
        assert_eq!(once.state, unchanged, "backwards clock is not elapsed time");
    }

    #[test]
    fn stop_and_reset_do_not_replay_scope_or_retain_old_rate_envelopes() {
        for mode in [VisMode::Spectrum, VisMode::Scope] {
            let mut vis = Visualizer::default();
            vis.set_mode(mode);
            vis.push(&tone(16, 1.0), 44100, 0, TransportState::Playing)
                .unwrap();
            vis.advance(0, TransportState::Stopped);
            assert_eq!(vis.state.input, VisInput::Missing);
            assert_eq!(vis.state.bars, [0; SPECTRUM_BANDS]);
            assert_eq!(vis.state.peaks, [0; SPECTRUM_BANDS]);
            vis.advance(2000, TransportState::Stopped);
            assert_eq!(vis.state.bars, [0; SPECTRUM_BANDS]);
            assert_eq!(vis.state.peaks, [0; SPECTRUM_BANDS]);
            vis.push(&tone(16, 1.0), 44100, 2000, TransportState::Paused)
                .unwrap();
            assert_eq!(vis.state.input, VisInput::Missing);
            vis.reset();
            assert_eq!(vis.state.mode, mode);
            assert_eq!(vis.state.source_timestamp_us, None);
            vis.push(&tone(16, 1.0), 44100, 2000, TransportState::Playing)
                .unwrap();
            vis.push(&VisFrame::default(), 48000, 2000, TransportState::Playing)
                .unwrap();
            assert_eq!(vis.state.bars, [0; SPECTRUM_BANDS]);
            assert_eq!(vis.state.peaks, [0; SPECTRUM_BANDS]);
        }
    }

    #[test]
    fn palette_pitch_rows_peaks_and_clip_are_exact() {
        let palette = std::array::from_fn(|slot| [slot as u8, 0, 0]);
        let sentinel = [250, 251, 252, 255];
        let mut image = Image::filled(100, 30, sentinel);
        let area = Rect::new(3, 4, 76, 16);
        let mut state = VisState::default();
        state.bars[0] = 16;
        state.peaks[0] = 16;
        state.bars[9] = 1;
        draw_visualization(&mut image, area, Geometry::Normal, &state, &palette);
        assert_eq!(image.pixel(3, 4), Some([23, 0, 0, 255]), "peak marker");
        assert_eq!(image.pixel(3, 5), Some([3, 0, 0, 255]), "bar row 14");
        assert_eq!(image.pixel(3, 19), Some([17, 0, 0, 255]), "bar row 0");
        assert_eq!(image.pixel(9, 19), Some([0, 0, 0, 255]), "one-column gap");
        assert_eq!(
            image.pixel(75, 19),
            Some([17, 0, 0, 255]),
            "the last band starts at 68"
        );
        for y in 0..30 {
            for x in 0..100 {
                if x < area.x || x >= area.right() || y < area.y || y >= area.bottom() {
                    assert_eq!(image.pixel(x, y), Some(sentinel));
                }
            }
        }
        // Even externally supplied out-of-range heights/rows cannot escape.
        state.bars.fill(255);
        state.peaks.fill(255);
        draw_visualization(
            &mut image,
            Rect::new(98, 29, 1, 1),
            Geometry::Normal,
            &state,
            &palette,
        );
        assert_eq!(image.pixel(98, 29), Some([23, 0, 0, 255]));
        assert_eq!(image.pixel(99, 29), Some(sentinel));
    }

    /// The shaded area draws five bars in five rows, and a bar of the shorter
    /// area shows the whole colour ramp (S08 §1, S17 §6).
    #[test]
    fn the_shaded_area_draws_five_bars_in_five_rows() {
        let palette = std::array::from_fn(|slot| [slot as u8, 0, 0]);
        let area = Rect::new(0, 0, 38, 5);
        let mut image = Image::new(38, 5);
        let mut state = VisState::default();
        for band in 0..5 {
            state.bars[band] = 5;
        }
        draw_visualization(&mut image, area, Geometry::Shaded, &state, &palette);
        // The first band starts at the left edge and shows the whole ramp.
        for row in 0..SHADED_ROWS {
            let y = i32::from(SHADED_ROWS - 1 - row);
            let expected = slots::SPECTRUM_BOTTOM as u8 - row * 15 / (SHADED_ROWS - 1);
            assert_eq!(
                image.pixel(0, y),
                Some([expected, 0, 0, 255]),
                "row {row} of the shaded bar"
            );
        }
        // The peak marker sits on the top row of its band.
        state.peaks[0] = 5;
        draw_visualization(&mut image, area, Geometry::Shaded, &state, &palette);
        assert_eq!(image.pixel(0, 0), Some([23, 0, 0, 255]));
        // The fifth band starts at 30 and the sixth has no place: the last
        // pixel of the area shows the grid, not a bar.
        assert_eq!(image.pixel(30, 4), Some([17, 0, 0, 255]));
        assert_eq!(
            image.pixel(36, 4),
            Some([slots::GRID as u8, 0, 0, 255]),
            "the shaded bars stop inside the area"
        );
    }

    /// S17 §6: slots 2..17 are the spectrum gradient, slot 2 at the **top** of
    /// a bar and slot 17 at the bottom.
    #[test]
    fn the_spectrum_gradient_runs_from_slot_2_at_the_top_to_slot_17_at_the_bottom() {
        let palette = std::array::from_fn(|slot| [slot as u8, 0, 0]);
        let area = Rect::new(0, 0, 76, 16);
        let mut image = Image::new(76, 16);
        let mut state = VisState::default();
        state.bars[0] = 16;
        draw_visualization(&mut image, area, Geometry::Normal, &state, &palette);
        for row in 0..SPECTRUM_ROWS {
            let y = i32::from(SPECTRUM_ROWS - 1 - row);
            assert_eq!(
                image.pixel(0, y),
                Some([slots::SPECTRUM_BOTTOM as u8 - row, 0, 0, 255]),
                "row {row} of the bar"
            );
        }
    }

    /// S17 §6: slots 18..22 are the five oscilloscope colours, slot 18 the
    /// brightest. The trace steps one slot per row away from the centre line.
    #[test]
    fn the_oscilloscope_wears_the_five_brightness_slots() {
        let palette = std::array::from_fn(|slot| [slot as u8, 0, 0]);
        let area = Rect::new(0, 0, 76, 16);
        let mut image = Image::new(76, 16);
        let mut state = VisState {
            mode: VisMode::ScopeDots,
            input: VisInput::Fresh,
            ..VisState::default()
        };
        // One sample per row, from the top row to the bottom row.
        for (column, row) in state.scope.iter_mut().enumerate().take(16) {
            *row = column as u8;
        }
        draw_visualization(&mut image, area, Geometry::Normal, &state, &palette);
        for (column, row) in state.scope.iter().enumerate().take(16) {
            let distance = (*row as i32 - 7).abs().min((*row as i32 - 8).abs());
            let expected = slots::OSCILLOSCOPE_FIRST as u8 + distance.min(4) as u8;
            assert_eq!(
                image.pixel(column as i32, i32::from(*row)),
                Some([expected, 0, 0, 255]),
                "row {row}"
            );
        }
    }

    /// S08 §1: the shaded area draws its own scope - 38 columns, one per
    /// pixel of the 38 x 5 area.
    #[test]
    fn the_shaded_area_draws_its_own_scope_columns() {
        let palette = std::array::from_fn(|slot| [slot as u8, 0, 0]);
        let area = Rect::new(0, 0, 38, 5);
        let mut image = Image::new(38, 5);
        let mut state = VisState {
            mode: VisMode::ScopeDots,
            input: VisInput::Fresh,
            ..VisState::default()
        };
        // A staircase: every column of the area carries its own row.
        for (column, row) in state.scope.iter_mut().enumerate().take(38) {
            *row = (column % 5) as u8;
        }
        draw_visualization(&mut image, area, Geometry::Shaded, &state, &palette);
        for column in 0..38usize {
            let row = (column % 5) as u8;
            // The centre line of five rows is row 2.
            let distance = (row as i32 - 2).unsigned_abs();
            let expected = slots::OSCILLOSCOPE_FIRST as u8 + distance.min(4) as u8;
            assert_eq!(
                image.pixel(column as i32, i32::from(row)),
                Some([expected, 0, 0, 255]),
                "column {column}"
            );
        }
    }

    #[test]
    fn missing_scope_draws_no_fake_line_and_styles_have_different_strokes() {
        let palette = std::array::from_fn(|slot| [slot as u8, 0, 0]);
        let area = Rect::new(0, 0, 76, 16);
        let mut image = Image::new(76, 16);
        let mut state = VisState {
            mode: VisMode::Scope,
            ..VisState::default()
        };
        draw_visualization(&mut image, area, Geometry::Normal, &state, &palette);
        assert!(
            !image.as_rgba().as_chunks::<4>().0.iter().any(|p| {
                (slots::OSCILLOSCOPE_FIRST as u8..=slots::OSCILLOSCOPE_LAST as u8).contains(&p[0])
            }),
            "no oscilloscope colour without fresh input"
        );
        state.input = VisInput::Fresh;
        state.scope[0] = 0;
        state.scope[1] = 15;
        draw_visualization(&mut image, area, Geometry::Normal, &state, &palette);
        assert_eq!(image.pixel(1, 7), Some([18, 0, 0, 255]));
        state.mode = VisMode::ScopeDots;
        draw_visualization(&mut image, area, Geometry::Normal, &state, &palette);
        assert_eq!(image.pixel(1, 7), Some([0, 0, 0, 255]));
        assert_eq!(image.pixel(1, 15), Some([22, 0, 0, 255]));
        state.mode = VisMode::Off;
        // The "off" mode paints nothing at all: the area keeps the skin's own
        // art (measured 2026-10-21), so a fresh canvas stays untouched.
        let mut fresh = Image::filled(76, 16, [1, 2, 3, 255]);
        draw_visualization(&mut fresh, area, Geometry::Normal, &state, &palette);
        assert!(
            fresh
                .as_rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [1, 2, 3, 255]),
            "the off mode paints nothing"
        );
    }
}
