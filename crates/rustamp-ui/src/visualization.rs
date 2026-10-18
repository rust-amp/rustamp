// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Rustamp's own small main-window visualisation, **not** a measured S08 recipe.
//!
//! Reference grade: **open** for every S08 question (measurement design §3.3).
//! This module makes explicit own choices, replaceable after an audited S08:
//! 576-point periodic-Hann DFT, DC removal, independent stereo magnitudes/max
//! fold, the integer-bin bands below, -60..0 dBFS -> 0..16 rows (nearest),
//! instantaneous attack, 24 rows/s bar fall, 300 ms peak hold / 16 rows/s fall.
//! A fixed, tiny direct transform avoids adding a dependency in this lane. It
//! runs on the UI side, never in the output callback or decode thread.
//!
//! No synthetic audio, padding, interpolation or plugin-spectrum interpretation:
//! only the tap's actual 2 x 576 waveform values enter the transform. The caller
//! must supply the **source** sample rate (not the device rate or rounded kHz).
//! The frame contract contains no rate, sequence/seek generation, or playback
//! selection metadata. Its timestamp is retained, not treated as a wall clock.
//! Decode-ahead/audio-to-pixel synchronisation remains open. A missing or stale
//! frame is not silence: the scope disappears and the existing bars/peaks decay.

use rustamp_core::vis::{VIS_SAMPLES, VisFrame};
use rustamp_gfx::{Image, Rect};

use crate::state::TransportState;

/// Own-choice width: fits the unused area below the time display (Q8 open).
pub const VIS_WIDTH: usize = 76;
/// Own-choice height, allowing one row per spectrum palette entry (Q8 open).
pub const VIS_HEIGHT: u8 = 16;
/// Own-choice bar count: 19 pitches of four pixels fill 76 columns (Q2 open).
pub const SPECTRUM_BANDS: usize = 19;
/// Own-choice half-open DFT-bin boundaries. DC (bin 0) is excluded; bins 1..288
/// are each used exactly once. Frequencies scale with the actual source rate:
/// bin k is k * rate / 576 Hz. These are NOT 19 independently resolved low-Hz
/// bands; at 44.1 kHz the first bin is already 76.5625 Hz (Q1 open).
pub const BAND_EDGES: [usize; SPECTRUM_BANDS + 1] = [
    1, 2, 3, 4, 5, 7, 9, 12, 16, 21, 28, 37, 49, 65, 86, 114, 151, 200, 264, 289,
];
/// Own-choice display floor, not the tap/plugin's magnitude scaling (Q4 open).
pub const FLOOR_DB: f64 = -60.0;
/// Own-choice linear bar fall, independent of paint frequency (Q7 open).
pub const BAR_FALL_ROWS_PER_SECOND: u64 = 24;
/// Own-choice peak plateau after the last equal/higher input (Q3 open).
pub const PEAK_HOLD_MS: u64 = 300;
/// Own-choice linear peak fall (Q3 open).
pub const PEAK_FALL_ROWS_PER_SECOND: u64 = 16;
/// Own-choice freshness window: retain a supplied target for 100 ms, then
/// decay without claiming that missing PCM was digital silence (Q6/Q11 open).
pub const FRAME_FRESH_MS: u64 = 100;

const UNITS_PER_ROW: u64 = 1000;
const BAR_PITCH: i32 = 4;
const BAR_WIDTH: u32 = 3;

/// UI-local modes and switching order, all **own choices** (Q9 open).
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
    /// A supplied frame is at most [`FRAME_FRESH_MS`] of active UI time old.
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
    pub scope: [u8; VIS_WIDTH],
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
            scope: [8; VIS_WIDTH],
            input: VisInput::Missing,
            source_timestamp_us: None,
        }
    }
}

/// Result of the stateless waveform analysis (no falloff or clock).
#[derive(Debug, Clone, PartialEq)]
pub struct VisAnalysis {
    /// Largest normalised sinusoidal amplitude per band and either channel.
    pub magnitudes: [f32; SPECTRUM_BANDS],
    /// Nearest actual sample at each scope column, quantised to a display row.
    pub scope: [u8; VIS_WIDTH],
}

/// A frame cannot be analysed honestly with these inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum VisError {
    /// The source sample rate is not supplied. Never assume 44.1 kHz.
    #[error("visualisation needs the source sample rate")]
    MissingSampleRate,
    /// Reject the entire bad frame instead of inventing replacement samples.
    #[error("visualisation frame contains non-finite PCM")]
    NonFinitePcm,
}

/// Frequency of a DFT bin at the explicitly supplied source rate.
pub fn bin_frequency_hz(bin: usize, sample_rate_hz: u32) -> Result<f64, VisError> {
    if sample_rate_hz == 0 {
        return Err(VisError::MissingSampleRate);
    }
    Ok(bin as f64 * f64::from(sample_rate_hz) / VIS_SAMPLES as f64)
}

/// Analyse exactly the samples in a tap frame; leave `frame.spectrum` untouched.
///
/// Periodic Hann coherent-gain normalisation makes an integer-bin full-scale
/// sine 1.0. Nyquist uses the undoubled real coefficient. DC removal happens
/// before windowing. Per-band maxima avoid phase cancellation between channels.
pub fn analyze(frame: &VisFrame, sample_rate_hz: u32) -> Result<VisAnalysis, VisError> {
    bin_frequency_hz(0, sample_rate_hz)?;
    if frame.waveform.iter().flatten().any(|v| !v.is_finite()) {
        return Err(VisError::NonFinitePcm);
    }
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
                // Clipping affects display amplitude only, not source PCM.
                let amplitude = (re.hypot(im) * factor / gain).min(1.0) as f32;
                *magnitude = magnitude.max(amplitude);
            }
        }
    }
    let scope = std::array::from_fn(|x| {
        let n = (x * (VIS_SAMPLES - 1) + (VIS_WIDTH - 1) / 2) / (VIS_WIDTH - 1);
        let mono = (f64::from(frame.waveform[0][n]) + f64::from(frame.waveform[1][n])) * 0.5;
        ((1.0 - mono.clamp(-1.0, 1.0)) * f64::from(VIS_HEIGHT - 1) * 0.5).round() as u8
    });
    Ok(VisAnalysis { magnitudes, scope })
}

/// Own-choice -60..0 dBFS height law, clamped and rounded to the nearest row.
pub fn magnitude_height(magnitude: f32) -> u8 {
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return 0;
    }
    let db = 20.0 * f64::from(magnitude).log10();
    (((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0) * f64::from(VIS_HEIGHT)).round() as u8
}

/// Clock-injected, allocation-free display envelope. No wall-clock reads.
#[derive(Debug, Clone, Default)]
pub struct Visualizer {
    state: VisState,
    bars: [u64; SPECTRUM_BANDS],
    peaks: [u64; SPECTRUM_BANDS],
    targets: [u64; SPECTRUM_BANDS],
    hold_until: [u64; SPECTRUM_BANDS],
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
        *self = Self::default();
        self.state.mode = mode;
    }

    /// Set a local render mode without changing plugin data or audio.
    pub fn set_mode(&mut self, mode: VisMode) {
        self.state.mode = mode;
    }

    /// Advance using milliseconds supplied by the presenter. Pause freezes all
    /// envelopes and freshness; stop clears the scope/target, then bars and peaks
    /// fall normally. Backwards clocks cannot advance time (all own choices).
    pub fn advance(&mut self, now_ms: u64, transport: TransportState) {
        let elapsed = self.last_ms.map_or(0, |last| now_ms.saturating_sub(last));
        self.last_ms = Some(self.last_ms.unwrap_or(now_ms).max(now_ms));
        let before = self.active_ms;
        if transport != TransportState::Paused && self.last_transport != TransportState::Paused {
            self.active_ms = self.active_ms.saturating_add(elapsed);
        }
        self.last_transport = transport;
        if transport == TransportState::Paused {
            return;
        }
        if transport == TransportState::Stopped {
            self.last_input = None;
            self.targets.fill(0);
            self.state.source_timestamp_us = None;
        }
        let expires = self.last_input.map(|t| t.saturating_add(FRAME_FRESH_MS));
        // Split at expiry so one large advance equals many small advances.
        let fresh_ms = expires.map_or(0, |t| t.min(self.active_ms).saturating_sub(before));
        let stale_ms = self
            .active_ms
            .saturating_sub(before)
            .saturating_sub(fresh_ms);
        for band in 0..SPECTRUM_BANDS {
            self.bars[band] = self.bars[band]
                .saturating_sub(fresh_ms.saturating_mul(BAR_FALL_ROWS_PER_SECOND))
                .max(self.targets[band]);
            self.bars[band] =
                self.bars[band].saturating_sub(stale_ms.saturating_mul(BAR_FALL_ROWS_PER_SECOND));
            let falling_ms = self
                .active_ms
                .saturating_sub(before.max(self.hold_until[band]));
            self.peaks[band] = self.peaks[band]
                .saturating_sub(falling_ms.saturating_mul(PEAK_FALL_ROWS_PER_SECOND))
                .max(self.bars[band]);
        }
        if expires.is_some_and(|t| self.active_ms > t) {
            self.targets.fill(0);
        }
        self.state.input = match expires {
            None => VisInput::Missing,
            Some(t) if self.active_ms > t => VisInput::Stale,
            Some(_) => VisInput::Fresh,
        };
        self.quantize();
    }

    /// Accept one actual frame, with an unrounded source rate. Paused/stopped
    /// input is discarded, not queued for resume. Invalid input returns an error;
    /// it never becomes a fake silence frame. Call regularly with the newest
    /// frame the engine offers; arrival time is an own-choice display clock.
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
        let analysis = analyze(frame, sample_rate_hz)?;
        if self
            .sample_rate_hz
            .is_some_and(|rate| rate != sample_rate_hz)
        {
            self.reset();
            self.advance(now_ms, transport);
        }
        self.sample_rate_hz = Some(sample_rate_hz);
        self.last_input = Some(self.active_ms);
        self.state.input = VisInput::Fresh;
        self.state.source_timestamp_us = Some(frame.timestamp_us);
        self.state.scope = analysis.scope;
        for (band, amplitude) in analysis.magnitudes.into_iter().enumerate() {
            let height = u64::from(magnitude_height(amplitude)) * UNITS_PER_ROW;
            self.targets[band] = height;
            self.bars[band] = self.bars[band].max(height);
            if height >= self.peaks[band] {
                self.peaks[band] = height;
                self.hold_until[band] = self.active_ms.saturating_add(PEAK_HOLD_MS);
            }
        }
        self.quantize();
        Ok(())
    }

    fn quantize(&mut self) {
        for band in 0..SPECTRUM_BANDS {
            // Ceiling keeps a falling sub-row visible until it really reaches 0.
            self.state.bars[band] = self.bars[band].div_ceil(UNITS_PER_ROW) as u8;
            self.state.peaks[band] = self.peaks[band].div_ceil(UNITS_PER_ROW) as u8;
        }
    }
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
    let mut image = Image::filled(VIS_WIDTH as u32, u32::from(VIS_HEIGHT), color(0));
    if state.mode != VisMode::Off {
        for y in (0..i32::from(VIS_HEIGHT)).step_by(2) {
            for x in (0..VIS_WIDTH as i32).step_by(2) {
                image.set_pixel(x, y, color(1));
            }
        }
    }
    match state.mode {
        VisMode::Spectrum => {
            for band in 0..SPECTRUM_BANDS {
                let x = band as i32 * BAR_PITCH;
                for row in 0..state.bars[band].min(VIS_HEIGHT) {
                    image.fill_rect(
                        Rect::new(x, i32::from(VIS_HEIGHT - 1 - row), BAR_WIDTH, 1),
                        color(2 + usize::from(row)),
                    );
                }
                let peak = state.peaks[band].min(VIS_HEIGHT);
                if peak > 0 {
                    image.fill_rect(
                        Rect::new(x, i32::from(VIS_HEIGHT - peak), BAR_WIDTH, 1),
                        color(23),
                    );
                }
            }
        }
        VisMode::Scope | VisMode::ScopeDots if state.input == VisInput::Fresh => {
            for (x, &row) in state.scope.iter().enumerate() {
                let row = row.min(VIS_HEIGHT - 1);
                if state.mode == VisMode::Scope && x > 0 {
                    let previous = state.scope[x - 1].min(VIS_HEIGHT - 1);
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
            area.width.min(VIS_WIDTH as u32),
            area.height.min(u32::from(VIS_HEIGHT)),
        ),
        area.x,
        area.y,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn silence_and_dc_have_no_spectrum_and_real_silence_has_a_scope() {
        for dc in [0.0, -0.5, 1.0] {
            let frame = VisFrame {
                waveform: [[dc; VIS_SAMPLES]; 2],
                ..VisFrame::default()
            };
            let analysis = analyze(&frame, 44100).unwrap();
            assert_eq!(analysis.magnitudes, [0.0; SPECTRUM_BANDS]);
        }
        assert_eq!(
            analyze(&VisFrame::default(), 44100).unwrap().scope,
            [8; VIS_WIDTH]
        );
    }

    #[test]
    fn hann_gain_and_db_law_are_numeric_not_only_bounds() {
        for (amplitude, height) in [(1.0, 16), (0.5, 14), (0.25, 13), (0.01, 5), (0.001, 0)] {
            let analysis = analyze(&tone(16, amplitude), 44100).unwrap();
            let peak = analysis.magnitudes[band_for(16)];
            assert!((peak - amplitude).abs() < 0.00001, "{peak} != {amplitude}");
            assert_eq!(magnitude_height(peak), height);
        }
        assert_eq!(magnitude_height(0.0), 0);
        assert_eq!(magnitude_height(2.0), 16);
    }

    #[test]
    fn bins_cover_the_half_spectrum_once_and_have_honest_frequency_units() {
        let bins: Vec<_> = BAND_EDGES.windows(2).flat_map(|w| w[0]..w[1]).collect();
        assert_eq!(bins, (1..=288).collect::<Vec<_>>());
        assert_eq!(bin_frequency_hz(1, 44100), Ok(76.5625));
        assert_eq!(bin_frequency_hz(288, 48000), Ok(24000.0));
        for bin in [2, 8, 16, 32, 64, 128, 250] {
            let analysis = analyze(&tone(bin, 0.5), 48000).unwrap();
            let largest = analysis.magnitudes.iter().copied().fold(0.0f32, f32::max);
            assert!((analysis.magnitudes[band_for(bin)] - largest).abs() < 0.00001);
            assert!((largest - 0.5).abs() < 0.00001);
        }
    }

    #[test]
    fn independent_channels_do_not_cancel_and_two_tones_survive() {
        let mono = tone(16, 0.5);
        let expected = analyze(&mono, 44100).unwrap().magnitudes;
        let mut frame = mono.clone();
        frame.waveform[1].fill(0.0);
        assert_eq!(analyze(&frame, 44100).unwrap().magnitudes, expected);
        frame.waveform.swap(0, 1);
        assert_eq!(analyze(&frame, 44100).unwrap().magnitudes, expected);
        frame = mono;
        frame.waveform[1].iter_mut().for_each(|x| *x = -*x);
        let anti = analyze(&frame, 44100).unwrap();
        assert_eq!(anti.magnitudes, expected);
        assert_eq!(
            anti.scope, [8; VIS_WIDTH],
            "scope explicitly averages stereo"
        );
        frame.waveform[1] = tone(128, 0.25).waveform[0];
        let two = analyze(&frame, 44100).unwrap();
        assert!((two.magnitudes[band_for(16)] - 0.5).abs() < 0.00001);
        assert!((two.magnitudes[band_for(128)] - 0.25).abs() < 0.00001);
    }

    #[test]
    fn nyquist_is_not_doubled_and_scope_uses_actual_endpoints_with_positive_up() {
        let mut frame = VisFrame {
            waveform: [std::array::from_fn(|n| if n % 2 == 0 { 0.5 } else { -0.5 }); 2],
            ..VisFrame::default()
        };
        let analysis = analyze(&frame, 48000).unwrap();
        assert!((analysis.magnitudes[18] - 0.5).abs() < 0.00001);
        frame.waveform = [[2.0; VIS_SAMPLES]; 2];
        frame.waveform[0][VIS_SAMPLES - 1] = -2.0;
        frame.waveform[1][VIS_SAMPLES - 1] = -2.0;
        let analysis = analyze(&frame, 48000).unwrap();
        assert_eq!(analysis.scope[0], 0);
        assert_eq!(analysis.scope[VIS_WIDTH - 1], 15);
    }

    #[test]
    fn missing_rate_and_bad_pcm_are_errors_and_plugin_spectrum_is_ignored() {
        let mut frame = tone(16, 0.5);
        assert_eq!(analyze(&frame, 0), Err(VisError::MissingSampleRate));
        let expected = analyze(&frame, 44100).unwrap();
        frame.spectrum = [[f32::NAN; VIS_SAMPLES]; 2];
        assert_eq!(analyze(&frame, 44100).unwrap(), expected);
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            frame.waveform[1][100] = bad;
            assert_eq!(analyze(&frame, 44100), Err(VisError::NonFinitePcm));
        }
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
        let mut vis = Visualizer::default();
        vis.set_mode(VisMode::Scope);
        vis.push(&tone(16, 1.0), 44100, 0, TransportState::Playing)
            .unwrap();
        vis.advance(0, TransportState::Stopped);
        assert_eq!(vis.state.input, VisInput::Missing);
        vis.advance(2000, TransportState::Stopped);
        assert_eq!(vis.state.bars, [0; SPECTRUM_BANDS]);
        vis.push(&tone(16, 1.0), 44100, 2000, TransportState::Paused)
            .unwrap();
        assert_eq!(vis.state.input, VisInput::Missing);
        vis.reset();
        assert_eq!(vis.state.mode, VisMode::Scope);
        assert_eq!(vis.state.source_timestamp_us, None);
        vis.push(&tone(16, 1.0), 44100, 2000, TransportState::Playing)
            .unwrap();
        vis.push(&VisFrame::default(), 48000, 2000, TransportState::Playing)
            .unwrap();
        assert_eq!(vis.state.bars, [0; SPECTRUM_BANDS]);
        assert_eq!(vis.state.peaks, [0; SPECTRUM_BANDS]);
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
