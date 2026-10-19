// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Main-window rendering and transport adapter for the shared classic analyser.
//!
//! All signal analysis, band/height laws, envelopes and triggering live in
//! [`rustamp_vis::analyser`]. These remain own choices, not measured S08 facts.
//! This module owns skin pixels, scope strokes, freshness and a pause-aware
//! display clock. Analysis runs on the UI side, never in the audio callback.
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

use rustamp_core::vis::VisFrame;
use rustamp_gfx::{Image, Rect};
pub use rustamp_vis::analyser::SPECTRUM_BANDS;
use rustamp_vis::analyser::{
    Analyser, AnalyserError, ClassicMode, FRAME_STALE_MS, SCOPE_COLUMNS, SPECTRUM_ROWS,
};

use crate::state::TransportState;

const BAR_PITCH: i32 = 4;
const BAR_WIDTH: u32 = 3;

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
    /// Background only; does not disable the engine tap or a plugin.
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
    pub bars: [u8; SPECTRUM_BANDS],
    /// Peak heights, 0..=16 rows; zero means no marker.
    pub peaks: [u8; SPECTRUM_BANDS],
    /// Scope row per column, 0..=15; used only with actual fresh input.
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
        self.state.scope = self.analyser.scope().map(scope_row);
    }
}

// Projection of the shared trace onto skin rows; positive samples point up.
fn scope_row(sample: f32) -> u8 {
    ((1.0 - f64::from(sample).clamp(-1.0, 1.0)) * f64::from(SPECTRUM_ROWS - 1) * 0.5).round() as u8
}

/// Paint only within `area`. Slot use/orientation/grid/strokes are own choices
/// (Q2/Q4/Q8/Q9 open), not promoted from S17's provisional slot descriptions.
/// Slot 0 background; slot 1 dots at even x/y; slots 2..17 bottom->top ramp;
/// slot 23 peaks; slot 18 scope. No colour constants bypass the skin palette.
pub fn draw_visualization(
    canvas: &mut Image,
    area: Rect,
    state: &VisState,
    colors: &[[u8; 3]; 24],
) {
    let color = |slot: usize| {
        let [r, g, b] = colors[slot];
        [r, g, b, 255]
    };
    // Fixed-size scratch makes even a changed layout safe: clip, never spill.
    let mut image = Image::filled(SCOPE_COLUMNS as u32, u32::from(SPECTRUM_ROWS), color(0));
    if state.mode != VisMode::Off {
        for y in (0..i32::from(SPECTRUM_ROWS)).step_by(2) {
            for x in (0..SCOPE_COLUMNS as i32).step_by(2) {
                image.set_pixel(x, y, color(1));
            }
        }
    }
    match state.mode {
        VisMode::Spectrum => {
            for band in 0..SPECTRUM_BANDS {
                let x = band as i32 * BAR_PITCH;
                for row in 0..state.bars[band].min(SPECTRUM_ROWS) {
                    image.fill_rect(
                        Rect::new(x, i32::from(SPECTRUM_ROWS - 1 - row), BAR_WIDTH, 1),
                        color(2 + usize::from(row)),
                    );
                }
                let peak = state.peaks[band].min(SPECTRUM_ROWS);
                if peak > 0 {
                    image.fill_rect(
                        Rect::new(x, i32::from(SPECTRUM_ROWS - peak), BAR_WIDTH, 1),
                        color(23),
                    );
                }
            }
        }
        VisMode::Scope | VisMode::ScopeDots if state.input == VisInput::Fresh => {
            for (x, &row) in state.scope.iter().enumerate() {
                let row = row.min(SPECTRUM_ROWS - 1);
                if state.mode == VisMode::Scope && x > 0 {
                    let previous = state.scope[x - 1].min(SPECTRUM_ROWS - 1);
                    // A one-column vertical bridge connects adjacent samples.
                    image.fill_rect(
                        Rect::new(
                            x as i32,
                            i32::from(row.min(previous)),
                            1,
                            u32::from(row.abs_diff(previous)) + 1,
                        ),
                        color(18),
                    );
                } else {
                    image.set_pixel(x as i32, i32::from(row), color(18));
                }
            }
        }
        _ => {}
    }
    canvas.blit_rect(
        &image,
        Rect::new(
            0,
            0,
            area.width.min(SCOPE_COLUMNS as u32),
            area.height.min(u32::from(SPECTRUM_ROWS)),
        ),
        area.x,
        area.y,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::vis::VIS_SAMPLES;
    use rustamp_vis::analyser::BAND_EDGES;

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

    #[test]
    fn peak_hold_bar_fall_and_staleness_have_exact_time_boundaries() {
        let mut vis = Visualizer::default();
        vis.push(&tone(16, 1.0), 44100, 0, TransportState::Playing)
            .unwrap();
        let band = band_for(16);
        vis.push(&VisFrame::default(), 44100, 0, TransportState::Playing)
            .unwrap();
        vis.advance(125, TransportState::Playing);
        assert_eq!((vis.state.bars[band], vis.state.peaks[band]), (13, 16));
        assert_eq!(vis.state.input, VisInput::Stale);
        vis.advance(300, TransportState::Playing);
        assert_eq!(vis.state.peaks[band], 16, "hold through 300 ms");
        vis.advance(425, TransportState::Playing);
        assert_eq!(vis.state.peaks[band], 14, "16 rows/s after hold");
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
        state.bars[18] = 1;
        draw_visualization(&mut image, area, &state, &palette);
        assert_eq!(image.pixel(3, 4), Some([23, 0, 0, 255]));
        assert_eq!(image.pixel(5, 5), Some([16, 0, 0, 255]));
        assert_eq!(image.pixel(3, 19), Some([2, 0, 0, 255]));
        assert_eq!(image.pixel(6, 19), Some([0, 0, 0, 255]), "one-column gap");
        assert_eq!(
            image.pixel(75, 19),
            Some([2, 0, 0, 255]),
            "last bar starts at 72"
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
        draw_visualization(&mut image, Rect::new(98, 29, 1, 1), &state, &palette);
        assert_eq!(image.pixel(98, 29), Some([23, 0, 0, 255]));
        assert_eq!(image.pixel(99, 29), Some(sentinel));
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
        draw_visualization(&mut image, area, &state, &palette);
        assert!(
            !image
                .as_rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[0] == 18)
        );
        state.input = VisInput::Fresh;
        state.scope[0] = 0;
        state.scope[1] = 15;
        draw_visualization(&mut image, area, &state, &palette);
        assert_eq!(image.pixel(1, 7), Some([18, 0, 0, 255]));
        state.mode = VisMode::ScopeDots;
        draw_visualization(&mut image, area, &state, &palette);
        assert_eq!(image.pixel(1, 7), Some([0, 0, 0, 255]));
        assert_eq!(image.pixel(1, 15), Some([18, 0, 0, 255]));
        state.mode = VisMode::Off;
        draw_visualization(&mut image, area, &state, &palette);
        assert!(
            image
                .as_rgba()
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| *p == [0, 0, 0, 255])
        );
    }
}
