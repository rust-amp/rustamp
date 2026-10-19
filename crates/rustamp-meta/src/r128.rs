//! EBU R 128 / ITU-R BS.1770-4 integrated loudness.
//!
//! The `R128_TRACK_GAIN` / `R128_ALBUM_GAIN` tags hold the gain that
//! brings a track to the **−23 LUFS** normalization level of EBU R 128.
//! The level is measured the way ITU-R BS.1770-4 measures it:
//!
//! * the signal is **K-weighted** — a high-shelf "pre-filter" followed by
//!   the RLB high-pass, both biquadratic;
//! * every **channel** is measured on its own (there is no mono downmix),
//!   and the channels are summed with the weights of the standard: 1.0 for
//!   left, right and center, 1.41 for the surrounds, 0 for the low
//!   frequency effects channel;
//! * the loudness is measured in **400 ms blocks with 75 % overlap**;
//! * the blocks are **gated**: first the absolute gate at −70 LUFS, then
//!   the relative gate 10 LU below the mean of the block loudness values
//!   that passed the absolute gate;
//! * the integrated loudness is the loudness of the blocks that passed
//!   both gates.
//!
//! ## The coefficients
//!
//! BS.1770-4 publishes the coefficients of the two stages for 48 kHz (its
//! tables 1 and 2). For every other rate the filters are *recomputed* from
//! the same design — the analog prototype with the standard's parameters,
//! transformed with the bilinear transform and prewarped at the sample
//! rate — never resampled from the 48 kHz version. The test
//! `the_48_khz_coefficients_match_the_standard` pins the computed
//! coefficients against the published tables.
//!
//! ## What the values are used for
//!
//! [`r128_track_gain_q8`] and [`r128_album_gain_q8`] turn the measurement
//! into the Q7.8 values the tags hold. The album value pools the blocks of
//! the album's tracks and gates them *together*, the way the standard
//! measures a programme: the quiet part of one track is not gated
//! separately from the loud part of another.

use rustamp_core::error::{Error, Result};

use crate::dsp::Biquad;
use crate::replaygain::db_to_r128_q8;

/// The loudness level the `R128_*_GAIN` tags refer to: **−23 LUFS**.
///
/// This is the normalization level of EBU R 128 / ITU-R BS.1770-4 and the
/// level the Opus ecosystem's R128 tags name: the value in the tag is the
/// gain a player applies to bring the track to −23 LUFS.
pub const R128_TARGET_LUFS: f32 = -23.0;

/// The constant of the BS.1770 loudness formula.
const BS1770_OFFSET: f64 = -0.691;

/// The absolute gate of BS.1770-4: a block below it is excluded.
const ABSOLUTE_GATE_LUFS: f64 = -70.0;

/// The relative gate: this many LU below the mean of the block loudness
/// values that passed the absolute gate.
const RELATIVE_GATE_OFFSET_LU: f64 = 10.0;

/// The length of a BS.1770-4 block: 400 ms.
const BLOCK_MS: u32 = 400;

/// The distance between two BS.1770-4 blocks: 100 ms, so the blocks
/// overlap by 75 %.
const BLOCK_STEP_MS: u32 = 100;

/// The two stages of the BS.1770-4 K-weighting, for one sample rate.
#[derive(Debug, Clone, Copy, PartialEq)]
struct KWeighting {
    /// The high-shelf "pre-filter".
    shelf: Biquad,
    /// The RLB high-pass.
    high_pass: Biquad,
}

impl KWeighting {
    /// The K-weighting of BS.1770-4 for `sample_rate_hz`.
    ///
    /// The design is the one of the standard: the analog prototype with
    /// the standard's parameters, brought to the sample rate with the
    /// bilinear transform and prewarped at the filter's corner frequency.
    /// For 48 kHz the result is the coefficient table of the standard.
    fn new(sample_rate_hz: u32) -> Self {
        // Stage 1: the high-shelf "pre-filter".
        //   f0 = 1681.974450955533 Hz, G = 3.999843853973347 dB,
        //   Q  = 0.7071752369554192
        let shelf = {
            let f0 = 1681.974450955533;
            let g_db = 3.999843853973347;
            let q = 0.7071752369554192;

            let k = (core::f64::consts::PI * f0 / sample_rate_hz as f64).tan();
            let vh = 10f64.powf(g_db / 20.0);
            let vb = vh.powf(0.4996667741545416);
            let a0 = 1.0 + k / q + k * k;

            Biquad::new(
                [
                    (vh + vb * k / q + k * k) / a0,
                    2.0 * (k * k - vh) / a0,
                    (vh - vb * k / q + k * k) / a0,
                ],
                [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            )
        };

        // Stage 2: the RLB high-pass.
        //   f0 = 38.13547087602444 Hz, Q = 0.5003270373238773
        let high_pass = {
            let f0 = 38.13547087602444;
            let q = 0.5003270373238773;

            let k = (core::f64::consts::PI * f0 / sample_rate_hz as f64).tan();
            let a0 = 1.0 + k / q + k * k;

            Biquad::new(
                [1.0, -2.0, 1.0],
                [1.0, 2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
            )
        };

        Self { shelf, high_pass }
    }
}

/// The BS.1770-4 channel weights for a channel count.
///
/// The standard measures the channels of a programme with the weights
/// 1.0 for left, right and center, 1.41 for the surrounds and 0 for the
/// low frequency effects channel — the low frequency channel is not part
/// of the measurement at all.
///
/// The channel *order* is the default of WAVE_FORMAT_EXTENSIBLE: mono,
/// stereo, 3D1 (left, right, center, low frequency effects), 3D2 (left,
/// right, center, left surround, right surround) and 5D1 (left, right,
/// center, low frequency effects, left surround, right surround). A file
/// that names its layout differently is measured with this one.
fn channel_weights(channels: usize) -> Vec<f64> {
    const FRONT: f64 = 1.0;
    const SURROUND: f64 = 1.41;
    const EXCLUDED: f64 = 0.0;

    match channels {
        0 => Vec::new(),
        1 => vec![FRONT],
        2 => vec![FRONT, FRONT],
        3 => vec![FRONT, FRONT, FRONT],
        4 => vec![FRONT, FRONT, FRONT, EXCLUDED],
        5 => vec![FRONT, FRONT, FRONT, SURROUND, SURROUND],
        _ => {
            let mut weights = vec![FRONT, FRONT, FRONT, EXCLUDED, SURROUND, SURROUND];
            weights.resize(channels, FRONT);
            weights
        }
    }
}

/// The number of samples in a BS.1770-4 block (400 ms).
///
/// At least one sample: a rate so low that 400 ms is shorter than one
/// sample must not produce an empty block.
fn block_size(sample_rate_hz: u32) -> usize {
    ((sample_rate_hz as usize * BLOCK_MS as usize + 500) / 1000).max(1)
}

/// The number of samples between two BS.1770-4 blocks (100 ms).
///
/// At least one sample, so that the blocks always move forward.
fn block_step(sample_rate_hz: u32) -> usize {
    ((sample_rate_hz as usize * BLOCK_STEP_MS as usize + 500) / 1000).max(1)
}

/// The BS.1770-4 blocks of a signal.
///
/// One entry per block: the channel-weighted mean square of the K-weighted
/// signal in that block. The loudness of a block is
/// `-0.691 + 10·log10(square)`.
///
/// The blocks are what the R128 values are derived from; they are kept
/// (instead of the loudness) because the gates work on them, and because
/// the blocks of several tracks can be pooled for an album.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LoudnessBlocks {
    squares: Vec<f64>,
}

impl LoudnessBlocks {
    /// Measures the blocks of `samples`.
    ///
    /// * `samples` holds `channels`-interleaved samples, typically in
    ///   -1.0..=1.0 but any finite values are accepted.
    /// * `sample_rate_hz` is the rate the signal was recorded at; the
    ///   K-weighting is computed for it.
    ///
    /// Every channel is K-weighted on its own — there is no mono downmix —
    /// and the blocks are the 400 ms windows with 75 % overlap. A block
    /// that is cut short by the end of the signal is kept when it holds at
    /// least a quarter of a full block.
    ///
    /// # Errors
    ///
    /// * [`Error::InvalidArgument`] when `channels` or `sample_rate_hz` is
    ///   zero, when `samples` is empty, when `samples.len()` is not a whole
    ///   number of frames, or when any sample is NaN or infinite.
    pub fn measure(samples: &[f32], channels: usize, sample_rate_hz: u32) -> Result<Self> {
        if channels == 0 {
            return Err(Error::InvalidArgument(
                "the loudness measurement needs at least one channel".to_owned(),
            ));
        }
        if sample_rate_hz == 0 {
            return Err(Error::InvalidArgument(
                "the loudness measurement needs a non-zero sample rate".to_owned(),
            ));
        }
        if samples.is_empty() {
            return Err(Error::InvalidArgument(
                "the loudness measurement needs at least one sample".to_owned(),
            ));
        }
        if !samples.len().is_multiple_of(channels) {
            return Err(Error::InvalidArgument(format!(
                "the loudness measurement: {} samples are not a whole number of {}-channel frames",
                samples.len(),
                channels
            )));
        }
        if samples.iter().any(|sample| !sample.is_finite()) {
            return Err(Error::InvalidArgument(
                "the loudness measurement refuses non-finite samples".to_owned(),
            ));
        }

        let weights = channel_weights(channels);
        let weighting = KWeighting::new(sample_rate_hz);

        // The K-weighted channels, each on its own.
        let mut weighted: Vec<Vec<f64>> = Vec::with_capacity(channels);
        for channel in 0..channels {
            let samples: Vec<f64> = samples
                .iter()
                .skip(channel)
                .step_by(channels)
                .map(|&sample| f64::from(sample))
                .collect();
            let filtered = weighting.high_pass.apply(&weighting.shelf.apply(&samples));
            weighted.push(filtered);
        }

        // The blocks.
        let block = block_size(sample_rate_hz);
        let step = block_step(sample_rate_hz);
        let frames = samples.len() / channels;
        let mut squares = Vec::new();
        let mut start = 0;
        while start < frames {
            let end = (start + block).min(frames);
            if end - start >= block / 4 {
                let mut weighted_square = 0.0;
                for (channel, weight) in weights.iter().enumerate() {
                    let slice = &weighted[channel][start..end];
                    let mean_square =
                        slice.iter().map(|value| value * value).sum::<f64>() / slice.len() as f64;
                    weighted_square += weight * mean_square;
                }
                squares.push(weighted_square);
            }
            start += step;
        }

        Ok(Self { squares })
    }

    /// The blocks of several tracks, pooled into one.
    ///
    /// This is the album case: the blocks of every track of the album are
    /// measured together, so the gates see the whole album and not one
    /// track at a time.
    pub fn pooled(blocks: &[&Self]) -> Self {
        let mut squares = Vec::new();
        for block in blocks {
            squares.extend_from_slice(&block.squares);
        }
        Self { squares }
    }

    /// The integrated loudness of the blocks, in LUFS.
    ///
    /// `None` when no block passes the absolute gate: the signal is below
    /// −70 LUFS — digital silence, or a recording too quiet to measure —
    /// and the standard leaves the measurement undefined.
    pub fn integrated_loudness(&self) -> Option<f32> {
        // The loudness of every block.
        let loudness: Vec<f64> = self
            .squares
            .iter()
            .map(|square| {
                if *square > 0.0 {
                    BS1770_OFFSET + 10.0 * square.log10()
                } else {
                    f64::NEG_INFINITY
                }
            })
            .collect();

        // The absolute gate: the blocks below -70 LUFS are dropped.
        let above_absolute: Vec<f64> = loudness
            .iter()
            .copied()
            .filter(|loudness| *loudness > ABSOLUTE_GATE_LUFS)
            .collect();
        if above_absolute.is_empty() {
            return None;
        }

        // The relative gate: 10 LU below the mean of the block loudness
        // values that passed the absolute gate.
        let mean = above_absolute.iter().sum::<f64>() / above_absolute.len() as f64;
        let relative_gate = mean - RELATIVE_GATE_OFFSET_LU;

        // The blocks that pass both gates.
        let kept: Vec<f64> = self
            .squares
            .iter()
            .zip(&loudness)
            .filter(|(_, loudness)| **loudness > relative_gate)
            .map(|(square, _)| *square)
            .collect();
        if kept.is_empty() {
            return None;
        }

        // The integrated loudness: the loudness of the mean of the blocks
        // that passed the gates.
        let mean_square = kept.iter().sum::<f64>() / kept.len() as f64;
        if mean_square <= 0.0 {
            return None;
        }
        Some((BS1770_OFFSET + 10.0 * mean_square.log10()) as f32)
    }
}

/// The R128 track gain of a track, in the Q7.8 form the tags hold.
///
/// The gain is the difference between the −23 LUFS target and the
/// integrated loudness of the track. A track below the absolute gate has
/// no measurable loudness; its gain is then the neutral 0, the same
/// convention the ReplayGain values use for silence.
pub fn r128_track_gain_q8(blocks: &LoudnessBlocks) -> i32 {
    match blocks.integrated_loudness() {
        Some(lufs) => db_to_r128_q8(R128_TARGET_LUFS - lufs),
        None => 0,
    }
}

/// The R128 album gain of an album, in the Q7.8 form the tags hold.
///
/// The blocks of the album's tracks are pooled and gated *together*, the
/// way the standard measures a programme.
pub fn r128_album_gain_q8(blocks: &[&LoudnessBlocks]) -> i32 {
    match LoudnessBlocks::pooled(blocks).integrated_loudness() {
        Some(lufs) => db_to_r128_q8(R128_TARGET_LUFS - lufs),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rate the coefficient tables of the standard are given for.
    const RATE_48K: u32 = 48_000;

    /// The coefficients of the standard's table 1 (the high-shelf
    /// "pre-filter") at 48 kHz.
    const TABLE_1_B: [f64; 3] = [1.53512485958697, -2.69169618940638, 1.19839281085285];
    const TABLE_1_A: [f64; 3] = [1.0, -1.69065929318241, 0.73248077421585];

    /// The coefficients of the standard's table 2 (the RLB high-pass) at
    /// 48 kHz.
    const TABLE_2_B: [f64; 3] = [1.0, -2.0, 1.0];
    const TABLE_2_A: [f64; 3] = [1.0, -1.99004745483398, 0.99007225036621];

    /// A sine whose **RMS** is `level_dbfs` dB below full scale.
    ///
    /// The RMS is the level the standard's test signals name: a 1 kHz sine
    /// with an RMS of −23 dBFS measures −23 LUFS.
    fn sine(level_dbfs: f32, frequency: f32, seconds: f32, sample_rate_hz: u32) -> Vec<f32> {
        let amplitude = 10f32.powf(level_dbfs / 20.0) * core::f32::consts::SQRT_2;
        let frames = (sample_rate_hz as f32 * seconds) as usize;
        (0..frames)
            .map(|i| {
                amplitude
                    * (2.0 * core::f32::consts::PI * frequency * i as f32 / sample_rate_hz as f32)
                        .sin()
            })
            .collect()
    }

    /// The loudness of a mono signal, measured with the published 48 kHz
    /// coefficients of the standard.
    ///
    /// This is an independent implementation: it uses the coefficient
    /// tables of the standard instead of the computed ones, and it works
    /// on `f32`. It exists to check the production measurement against
    /// something that was not written with the same code.
    fn reference_lufs(samples: &[f32], sample_rate_hz: u32) -> Option<f32> {
        // The standard's coefficients, computed for the rate of the signal
        // from the tables of the standard (48 kHz) is not possible, so the
        // test only runs at 48 kHz.
        assert_eq!(sample_rate_hz, RATE_48K);

        let mut weighted = Vec::with_capacity(samples.len());
        let mut x1 = 0.0f32;
        let mut x2 = 0.0f32;
        let mut y1 = 0.0f32;
        let mut y2 = 0.0f32;
        for &sample in samples {
            let y =
                TABLE_1_B[0] as f32 * sample + TABLE_1_B[1] as f32 * x1 + TABLE_1_B[2] as f32 * x2
                    - TABLE_1_A[1] as f32 * y1
                    - TABLE_1_A[2] as f32 * y2;
            x2 = x1;
            x1 = sample;
            y2 = y1;
            y1 = y;
            weighted.push(y);
        }

        let mut x1 = 0.0f32;
        let mut x2 = 0.0f32;
        let mut y1 = 0.0f32;
        let mut y2 = 0.0f32;
        let filtered: Vec<f32> = weighted
            .into_iter()
            .map(|sample| {
                let y = TABLE_2_B[0] as f32 * sample
                    + TABLE_2_B[1] as f32 * x1
                    + TABLE_2_B[2] as f32 * x2
                    - TABLE_2_A[1] as f32 * y1
                    - TABLE_2_A[2] as f32 * y2;
                x2 = x1;
                x1 = sample;
                y2 = y1;
                y1 = y;
                y
            })
            .collect();

        // The blocks: 400 ms with a 100 ms step. Every block keeps its
        // mean square and its loudness.
        let block = 19_200;
        let step = 4_800;
        let mut blocks: Vec<(f32, f32)> = Vec::new();
        let mut start = 0;
        while start + block <= filtered.len() {
            let slice = &filtered[start..start + block];
            let mean_square = slice.iter().map(|value| value * value).sum::<f32>() / block as f32;
            let loudness = -0.691 + 10.0 * mean_square.log10();
            blocks.push((mean_square, loudness));
            start += step;
        }
        if blocks.is_empty() {
            return None;
        }

        // The gates: the absolute gate first, then the relative gate 10 LU
        // below the mean of the blocks that passed it.
        let above_absolute: Vec<f32> = blocks
            .iter()
            .map(|(_, loudness)| *loudness)
            .filter(|loudness| *loudness > -70.0)
            .collect();
        if above_absolute.is_empty() {
            return None;
        }
        let mean = above_absolute.iter().sum::<f32>() / above_absolute.len() as f32;
        let relative_gate = mean - 10.0;
        let kept: Vec<f32> = blocks
            .iter()
            .filter(|(_, loudness)| *loudness > relative_gate)
            .map(|(square, _)| *square)
            .collect();
        if kept.is_empty() {
            return None;
        }
        // The loudness of the mean of the kept blocks.
        let mean_square = kept.iter().sum::<f32>() / kept.len() as f32;
        Some(-0.691 + 10.0 * mean_square.log10())
    }

    #[test]
    fn the_48_khz_coefficients_match_the_standard() {
        // The standard's tables 1 and 2 give the coefficients of the two
        // K-weighting stages at 48 kHz. The production computes them from
        // the analog design; the two must agree.
        let weighting = KWeighting::new(RATE_48K);

        let shelf = weighting.shelf;
        assert!((shelf.b0 - TABLE_1_B[0]).abs() < 1e-12, "{shelf:?}");
        assert!((shelf.b1 - TABLE_1_B[1]).abs() < 1e-12, "{shelf:?}");
        assert!((shelf.b2 - TABLE_1_B[2]).abs() < 1e-12, "{shelf:?}");
        assert!((shelf.a1 - TABLE_1_A[1]).abs() < 1e-12, "{shelf:?}");
        assert!((shelf.a2 - TABLE_1_A[2]).abs() < 1e-12, "{shelf:?}");

        let high_pass = weighting.high_pass;
        assert!((high_pass.b0 - TABLE_2_B[0]).abs() < 1e-12, "{high_pass:?}");
        assert!((high_pass.b1 - TABLE_2_B[1]).abs() < 1e-12, "{high_pass:?}");
        assert!((high_pass.b2 - TABLE_2_B[2]).abs() < 1e-12, "{high_pass:?}");
        assert!((high_pass.a1 - TABLE_2_A[1]).abs() < 1e-12, "{high_pass:?}");
        assert!((high_pass.a2 - TABLE_2_A[2]).abs() < 1e-12, "{high_pass:?}");
    }

    #[test]
    fn the_measured_loudness_matches_the_standard() {
        // The published test signals of the standard: a 1 kHz sine at
        // -23 dBFS measures -23 LUFS.
        let samples = sine(-23.0, 1000.0, 10.0, RATE_48K);
        let blocks = LoudnessBlocks::measure(&samples, 1, RATE_48K).unwrap();
        let lufs = blocks.integrated_loudness().expect("a loudness");
        assert!((lufs - (-23.0)).abs() < 0.1, "{lufs} LUFS");

        // And the independent implementation agrees.
        let reference = reference_lufs(&samples, RATE_48K).expect("a loudness");
        assert!((lufs - reference).abs() < 0.1, "{lufs} vs {reference} LUFS");
    }

    #[test]
    fn a_stereo_signal_is_measured_over_its_channels() {
        // Two uncorrelated tones, one per channel, at -23 dBFS each: the
        // channels are summed, so the signal is 3 dB louder than one of
        // them.
        let left = sine(-23.0, 1000.0, 10.0, RATE_48K);
        let right = sine(-23.0, 997.0, 10.0, RATE_48K);
        let mut stereo = Vec::with_capacity(left.len() * 2);
        for (l, r) in left.iter().zip(&right) {
            stereo.push(*l);
            stereo.push(*r);
        }

        let blocks = LoudnessBlocks::measure(&stereo, 2, RATE_48K).unwrap();
        let lufs = blocks.integrated_loudness().expect("a loudness");
        assert!(
            (lufs - (-20.0)).abs() < 0.2,
            "{lufs} LUFS, expected about -20 LUFS"
        );

        // The mono downmix of the same signal is 3 dB below one channel
        // (the averaging), so it measures 6 dB below the stereo signal.
        let mono: Vec<f32> = stereo
            .chunks(2)
            .map(|frame| (frame[0] + frame[1]) / 2.0)
            .collect();
        let blocks = LoudnessBlocks::measure(&mono, 1, RATE_48K).unwrap();
        let mono_lufs = blocks.integrated_loudness().expect("a loudness");
        assert!(
            (lufs - mono_lufs - 6.0).abs() < 0.2,
            "the stereo signal is {} dB louder than its mono downmix",
            lufs - mono_lufs
        );
    }

    #[test]
    fn the_stereo_value_is_three_decibels_better_than_the_old_one() {
        // The old approach derived the R128 gain from the ReplayGain gain
        // of the mono downmix: `replaygain_gain - 19.99 dB`. For a stereo
        // signal that is 3 dB off, because the downmix loses the energy
        // the standard's channel sum keeps.
        let left = sine(-23.0, 1000.0, 10.0, RATE_48K);
        let right = sine(-23.0, 997.0, 10.0, RATE_48K);
        let mut stereo = Vec::with_capacity(left.len() * 2);
        for (l, r) in left.iter().zip(&right) {
            stereo.push(*l);
            stereo.push(*r);
        }

        // The new measurement.
        let blocks = LoudnessBlocks::measure(&stereo, 2, RATE_48K).unwrap();
        let lufs = blocks.integrated_loudness().expect("a loudness");
        let new_gain = crate::replaygain::r128_q8_to_db(r128_track_gain_q8(&blocks));
        assert!(
            (lufs - (-20.0)).abs() < 0.2,
            "the stereo signal measures {lufs} LUFS"
        );

        // The old one: the ReplayGain gain of the mono downmix, shifted by
        // the constant the old implementation used.
        let mono: Vec<f32> = stereo
            .chunks(2)
            .map(|frame| (frame[0] + frame[1]) / 2.0)
            .collect();
        let analysis = crate::replaygain::analyze_replaygain(&mono, 1, RATE_48K).unwrap();
        let old_gain = analysis.track_gain_db - 19.99;

        assert!(
            (old_gain - new_gain - 3.0).abs() < 0.2,
            "the old value is {old_gain} dB, the new one is {new_gain} dB"
        );
        // And the new value is the one the standard asks for: the signal
        // measures -20 LUFS, so it needs 3 dB of attenuation to reach the
        // -23 LUFS target.
        assert!(
            (new_gain - (-3.0)).abs() < 0.2,
            "the new R128 gain is {new_gain} dB"
        );
    }

    #[test]
    fn the_gates_drop_the_silence() {
        // A tone in the first half, silence in the second: the silent
        // blocks are below the absolute gate and do not drag the
        // measurement down.
        //
        // The block that straddles the end of the tone holds a quarter of
        // its energy, which is why the result is a tenth of a dB below the
        // tone's own loudness.
        let mut samples = sine(-23.0, 1000.0, 5.0, RATE_48K);
        samples.extend(std::iter::repeat_n(0.0f32, 5 * RATE_48K as usize));

        let blocks = LoudnessBlocks::measure(&samples, 1, RATE_48K).unwrap();
        let lufs = blocks.integrated_loudness().expect("a loudness");
        assert!((lufs - (-23.0)).abs() < 0.2, "{lufs} LUFS");
    }

    #[test]
    fn a_silent_signal_has_no_loudness() {
        let samples = vec![0.0f32; 48_000];
        let blocks = LoudnessBlocks::measure(&samples, 1, RATE_48K).unwrap();
        assert!(blocks.integrated_loudness().is_none());
        // And the gain is the neutral one.
        assert_eq!(r128_track_gain_q8(&blocks), 0);
    }

    #[test]
    fn the_r128_gain_brings_the_track_to_the_target() {
        // A track at -14 LUFS needs 9 dB of attenuation to reach -23 LUFS.
        let samples = sine(-14.0, 1000.0, 10.0, RATE_48K);
        let blocks = LoudnessBlocks::measure(&samples, 1, RATE_48K).unwrap();
        let lufs = blocks.integrated_loudness().expect("a loudness");
        let gain_db = crate::replaygain::r128_q8_to_db(r128_track_gain_q8(&blocks));
        assert!((lufs - (-14.0)).abs() < 0.1, "{lufs} LUFS");
        assert!((gain_db - (-9.0)).abs() < 0.1, "{gain_db} dB");
    }
    #[test]
    fn the_album_gates_the_blocks_of_all_tracks_together() {
        // One loud track and one that is 12 dB quieter: the album is
        // louder than the quiet track and quieter than the loud one, and
        // it is not the mean of the two.
        let loud =
            LoudnessBlocks::measure(&sine(-23.0, 1000.0, 10.0, RATE_48K), 1, RATE_48K).unwrap();
        let quiet =
            LoudnessBlocks::measure(&sine(-35.0, 1000.0, 10.0, RATE_48K), 1, RATE_48K).unwrap();

        let loud_lufs = loud.integrated_loudness().expect("a loudness");
        let quiet_lufs = quiet.integrated_loudness().expect("a loudness");
        let album_lufs = LoudnessBlocks::pooled(&[&loud, &quiet])
            .integrated_loudness()
            .expect("a loudness");

        assert!(album_lufs < loud_lufs, "{album_lufs} vs {loud_lufs}");
        assert!(album_lufs > quiet_lufs, "{album_lufs} vs {quiet_lufs}");
        let mean = (loud_lufs + quiet_lufs) / 2.0;
        assert!(
            (album_lufs - mean).abs() > 1.0,
            "the pooled album is {album_lufs} LUFS, the mean of the tracks is {mean}"
        );
    }

    #[test]
    fn the_channel_weights_follow_the_standard() {
        // The low frequency effects channel is not part of the measurement.
        assert_eq!(channel_weights(1), vec![1.0]);
        assert_eq!(channel_weights(2), vec![1.0, 1.0]);
        assert_eq!(channel_weights(3), vec![1.0, 1.0, 1.0]);
        assert_eq!(channel_weights(4), vec![1.0, 1.0, 1.0, 0.0]);
        assert_eq!(channel_weights(5), vec![1.0, 1.0, 1.0, 1.41, 1.41]);
        assert_eq!(channel_weights(6), vec![1.0, 1.0, 1.0, 0.0, 1.41, 1.41]);
    }

    #[test]
    fn an_absurd_rate_does_not_hang_the_measurement() {
        // A rate so low that a block is shorter than one sample: without a
        // minimum of one sample the blocks would never move forward and the
        // measurement would fill a vector until it runs out of memory.
        for rate in [1u32, 2, 3, 4_294_967_295] {
            let samples = vec![0.5f32; 64];
            let blocks = LoudnessBlocks::measure(&samples, 1, rate).unwrap();
            // The measurement returns; whether it finds a block at all
            // depends on whether the signal is long enough for one.
            if let Some(loudness) = blocks.integrated_loudness() {
                assert!(loudness.is_finite(), "{rate} Hz");
            }
        }
    }

    #[test]
    fn degenerate_input_is_refused() {
        let samples = vec![0.5f32; 100];
        assert!(LoudnessBlocks::measure(&samples, 0, RATE_48K).is_err());
        assert!(LoudnessBlocks::measure(&samples, 1, 0).is_err());
        assert!(LoudnessBlocks::measure(&[], 1, RATE_48K).is_err());
        assert!(LoudnessBlocks::measure(&samples[..10], 3, RATE_48K).is_err());

        let mut nan = vec![0.5f32; 100];
        nan[50] = f32::NAN;
        assert!(LoudnessBlocks::measure(&nan, 1, RATE_48K).is_err());
    }
}
