//! ReplayGain tags and a headless loudness analysis.
//!
//! This module covers the ReplayGain half of `rustamp-meta`:
//!
//! * the tag keys the crate reads and writes ([`REPLAYGAIN_KEYS`], plus the
//!   [`REFERENCE_LOUDNESS_KEY`] the tag layer cannot carry),
//! * the in-memory form ([`ReplayGainTags`], stored in
//!   [`TrackMetadata::extra`](rustamp_core::meta::TrackMetadata::extra)),
//! * an honest write entry point ([`write_replaygain`]) that reports the keys
//!   a format could not store instead of dropping them silently,
//! * a pure analysis function ([`analyze_replaygain`]) that computes the gain
//!   and peak a ReplayGain tag would hold, without audio hardware.
//!
//! ## Loudness convention
//!
//! The analysis implements the classic ReplayGain 1.0 proposal (David
//! Robinson, "Replay Gain - a proposed standard", 2001-07-31,
//! replaygain.org; summarized on the Hydrogenaudio wiki as the "ReplayGain
//! 1.0 specification"): an equal-loudness filter (the standard Yule shelf
//! followed by a Butterworth high-pass) is applied to a mono downmix, the
//! result is split into 50 ms blocks, each block's RMS is converted to dB
//! relative to [`REFERENCE_RMS`], and the track loudness is the 95th
//! percentile of those block values. The reference is **89 dB SPL pink
//! noise**, anchored in the digital domain so that **a full-scale sine wave
//! (peak 1.0, RMS 1/sqrt(2)) measures 0.00 dB gain**. The peak is the
//! largest absolute sample value of the *unfiltered* input, linear, exactly
//! as a `REPLAYGAIN_*_PEAK` tag holds it.
//!
//! What this is not:
//!
//! * **Not EBU R128 / ITU-R BS.1770 integrated loudness.** There is no
//!   K-weighting, no channel weighting in the R128 sense, no gating and no
//!   loudness-range measurement. Every 50 ms block counts towards the
//!   percentile.
//! * **Not a substitute for the reference implementation's exact numbers.**
//!   Tools such as `mp3gain`, `metaflac --add-replay-gain`, foobar2000 or
//!   loudgain may use slightly different block sizes or filter coefficients,
//!   so their tags can differ, typically by a small amount on broadband
//!   music. Direction and scale agree on simple signals (see the tests);
//!   bit equality is not claimed.
//!
//! The `sample_rate_hz` argument is accepted and validated but does not
//! change the number — there are no filters to tune — so that a future
//! perceptual estimator can use it without changing call sites. The same
//! holds for `channels`, which today only shapes (and validates) the input.
//!
//! ## Album values
//!
//! [`album_gain_db`], [`album_gain_from_histograms`] and
//! [`album_gain_from_blocks`] compute the album-level gain of a set of
//! tracks: the negative of the 95th percentile of the pooled per-block
//! loudness values of the album, clamped to ±51 dB — the track rule applied
//! to the album as a whole. [`album_peak`] is the largest track peak of the
//! album.
//!
//! Which tracks belong to one album is a separate question: the grouping
//! and the mapping from a track to its album values live in the
//! [`album`](crate::album) module.
//!
//! ## Format support (honest answers)
//!
//! The four classic keys round-trip on every primary tag type `lofty`
//! supports, because each of them maps the keys: ID3v2 (`TXXX`,
//! MP3/AAC/AIFF/WAV), Vorbis comments (FLAC/Ogg/Opus/Speex), MP4 ilst
//! (freeform `----:com.apple.iTunes:replaygain_*`, M4A/MP4) and APEv2
//! (APE/MPC/WavPack). `R128_TRACK_GAIN` / `R128_ALBUM_GAIN` exist only in
//! the Vorbis map. `REPLAYGAIN_REFERENCE_LOUDNESS` has no `ItemKey` in
//! `lofty` 0.25.4 and unmapped keys are discarded, so the tag layer cannot
//! read or write it; it is a named constant here so callers can keep it in
//! `extra` themselves, and every write entry point reports it as skipped
//! rather than dropping it silently. RIFF INFO, AIFF text and ID3v1 carry no
//! ReplayGain keys at all, and no file format uses them as its primary
//! (write-target) tag, so they are simply absent there.
//!
//! Applying the gain to audio — the volume stage that reads these tags —
//! belongs to another lane (`rustamp-audio`) and is explicitly not done here.
//! See `docs/tasks/E4-replaygain.md` for what that side still has to do.

use std::borrow::Borrow;

use lofty::file::FileType;
use lofty::tag::{ItemKey, TagType};

use rustamp_core::error::{Error, Result};
use rustamp_core::meta::TrackMetadata;

use crate::write::write_tags_report;

/// The track gain tag, e.g. `"-3.50 dB"`.
pub const TRACK_GAIN_KEY: &str = "REPLAYGAIN_TRACK_GAIN";
/// The track peak tag, linear, e.g. `"0.987654"`.
pub const TRACK_PEAK_KEY: &str = "REPLAYGAIN_TRACK_PEAK";
/// The album gain tag, e.g. `"-5.10 dB"`.
pub const ALBUM_GAIN_KEY: &str = "REPLAYGAIN_ALBUM_GAIN";
/// The album peak tag, linear.
pub const ALBUM_PEAK_KEY: &str = "REPLAYGAIN_ALBUM_PEAK";
/// The Opus/R128 track gain tag: an integer in Q8 units (256 steps per dB).
/// Carried by the Vorbis family only.
pub const R128_TRACK_GAIN_KEY: &str = "R128_TRACK_GAIN";
/// The Opus/R128 album gain tag. Carried by the Vorbis family only.
pub const R128_ALBUM_GAIN_KEY: &str = "R128_ALBUM_GAIN";
/// The reference-loudness informational key from the original ReplayGain
/// proposal (e.g. `"89 dB SPL"`).
///
/// `lofty` 0.25.4 has no `ItemKey` for this key and discards unmapped keys,
/// so `rustamp-meta` can neither read nor write it. It is exported so callers
/// can name it consistently; any write entry point lists it as skipped.
/// See the module docs.
pub const REFERENCE_LOUDNESS_KEY: &str = "REPLAYGAIN_REFERENCE_LOUDNESS";

/// Every ReplayGain-family key the tag layer can carry, in a fixed order.
pub const REPLAYGAIN_KEYS: &[&str] = &[
    TRACK_GAIN_KEY,
    TRACK_PEAK_KEY,
    ALBUM_GAIN_KEY,
    ALBUM_PEAK_KEY,
    R128_TRACK_GAIN_KEY,
    R128_ALBUM_GAIN_KEY,
];

/// Canonical extra-name to [`ItemKey`] mapping for the carried keys.
///
/// This table — not `ItemKey::from_key` on the conventional short name — is
/// what makes MP4 work: the ilst map only knows the freeform
/// `----:com.apple.iTunes:replaygain_*` form, so a short-name lookup fails
/// there, while inserting by `ItemKey` writes exactly that freeform atom.
pub(crate) const EXTRA_ITEM_KEYS: &[(&str, ItemKey)] = &[
    (TRACK_GAIN_KEY, ItemKey::ReplayGainTrackGain),
    (TRACK_PEAK_KEY, ItemKey::ReplayGainTrackPeak),
    (ALBUM_GAIN_KEY, ItemKey::ReplayGainAlbumGain),
    (ALBUM_PEAK_KEY, ItemKey::ReplayGainAlbumPeak),
    (R128_TRACK_GAIN_KEY, ItemKey::R128TrackGain),
    (R128_ALBUM_GAIN_KEY, ItemKey::R128AlbumGain),
];

/// Sort key that puts carried names in [`REPLAYGAIN_KEYS`] order.
///
/// [`TrackMetadata::extra`](rustamp_core::meta::TrackMetadata::extra) is a
/// `BTreeMap`, so iterating it yields names in *alphabetical* order, which puts
/// `R128_ALBUM_GAIN` before `R128_TRACK_GAIN`. The reports promise the fixed
/// key order instead, so both lists are sorted through this before they leave
/// the crate. Unknown names (there should be none from
/// [`ReplayGainTags::apply_to_metadata`]) sort last.
fn key_rank(name: &str) -> usize {
    REPLAYGAIN_KEYS
        .iter()
        .position(|key| key.eq_ignore_ascii_case(name))
        .unwrap_or(REPLAYGAIN_KEYS.len())
}

/// Looks up the [`ItemKey`] for a carried extra name, case-insensitively.
/// Returns `None` for anything else (including [`REFERENCE_LOUDNESS_KEY`]).
pub(crate) fn item_key_for_extra(name: &str) -> Option<ItemKey> {
    EXTRA_ITEM_KEYS
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, item)| *item)
}

/// ReplayGain values as held in
/// [`TrackMetadata::extra`](rustamp_core::meta::TrackMetadata::extra).
///
/// Gains are in dB, peaks are linear (0.0 and up). `None` means absent: this
/// type never invents a value, and [`apply_to_metadata`](Self::apply_to_metadata)
/// only sets the fields that are `Some`, never clearing, matching the
/// no-clear rule of [`crate::write_tags`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReplayGainTags {
    /// `REPLAYGAIN_TRACK_GAIN` in dB.
    pub track_gain_db: Option<f32>,
    /// `REPLAYGAIN_TRACK_PEAK`, linear.
    pub track_peak: Option<f32>,
    /// `REPLAYGAIN_ALBUM_GAIN` in dB.
    pub album_gain_db: Option<f32>,
    /// `REPLAYGAIN_ALBUM_PEAK`, linear.
    pub album_peak: Option<f32>,
    /// `R128_TRACK_GAIN` in raw Q8 units (256 steps per dB).
    pub r128_track_gain_q8: Option<i32>,
    /// `R128_ALBUM_GAIN` in raw Q8 units.
    pub r128_album_gain_q8: Option<i32>,
}

impl ReplayGainTags {
    /// Returns `true` when no field is set.
    pub fn is_empty(&self) -> bool {
        self.track_gain_db.is_none()
            && self.track_peak.is_none()
            && self.album_gain_db.is_none()
            && self.album_peak.is_none()
            && self.r128_track_gain_q8.is_none()
            && self.r128_album_gain_q8.is_none()
    }

    /// Parses the ReplayGain entries out of `metadata.extra`.
    ///
    /// Malformed values are treated as absent (a tag that does not parse is
    /// not a value), so this never fails; use
    /// [`replaygain_supported_for_extension`] plus the write reports to
    /// learn what a format can hold.
    pub fn from_metadata(metadata: &TrackMetadata) -> Self {
        let text = |key: &str| metadata.extra.get(key);
        Self {
            track_gain_db: text(TRACK_GAIN_KEY).and_then(|v| parse_gain_db(v)),
            track_peak: text(TRACK_PEAK_KEY).and_then(|v| parse_peak(v)),
            album_gain_db: text(ALBUM_GAIN_KEY).and_then(|v| parse_gain_db(v)),
            album_peak: text(ALBUM_PEAK_KEY).and_then(|v| parse_peak(v)),
            r128_track_gain_q8: text(R128_TRACK_GAIN_KEY).and_then(|v| parse_r128(v)),
            r128_album_gain_q8: text(R128_ALBUM_GAIN_KEY).and_then(|v| parse_r128(v)),
        }
    }

    /// Formats the set fields into `metadata.extra`, leaving everything else
    /// — including fields this value leaves as `None` — untouched.
    pub fn apply_to_metadata(&self, metadata: &mut TrackMetadata) {
        if let Some(gain) = self.track_gain_db {
            metadata
                .extra
                .insert(TRACK_GAIN_KEY.to_owned(), format_gain_db(gain));
        }
        if let Some(peak) = self.track_peak {
            metadata
                .extra
                .insert(TRACK_PEAK_KEY.to_owned(), format_peak(peak));
        }
        if let Some(gain) = self.album_gain_db {
            metadata
                .extra
                .insert(ALBUM_GAIN_KEY.to_owned(), format_gain_db(gain));
        }
        if let Some(peak) = self.album_peak {
            metadata
                .extra
                .insert(ALBUM_PEAK_KEY.to_owned(), format_peak(peak));
        }
        if let Some(q8) = self.r128_track_gain_q8 {
            metadata
                .extra
                .insert(R128_TRACK_GAIN_KEY.to_owned(), q8.to_string());
        }
        if let Some(q8) = self.r128_album_gain_q8 {
            metadata
                .extra
                .insert(R128_ALBUM_GAIN_KEY.to_owned(), q8.to_string());
        }
    }
}

/// Parses a gain tag such as `"-3.50 dB"`, `"+6.02dB"` or bare `"6"` into dB.
///
/// The trailing `dB` (any ASCII case) is optional; surrounding whitespace is
/// ignored. Returns `None` for anything that is not a finite number.
pub fn parse_gain_db(text: &str) -> Option<f32> {
    let upper = text.trim().to_ascii_uppercase();
    let digits = upper.strip_suffix("DB").unwrap_or(&upper);
    let value: f32 = digits.trim().parse().ok()?;
    value.is_finite().then_some(value)
}

/// Formats a gain in dB the way ReplayGain tags hold it: `"+6.02 dB"`.
pub fn format_gain_db(gain_db: f32) -> String {
    format!("{gain_db:+.2} dB")
}

/// Parses a linear peak tag such as `"0.987654"`.
///
/// Returns `None` for anything that is not a finite, non-negative number.
/// A `dB` suffix is rejected: peaks are linear.
pub fn parse_peak(text: &str) -> Option<f32> {
    let value: f32 = text.trim().parse().ok()?;
    (value.is_finite() && value >= 0.0).then_some(value)
}

/// Formats a linear peak the way ReplayGain tags hold it: `"0.987654"`.
pub fn format_peak(peak: f32) -> String {
    format!("{peak:.6}")
}

/// Parses a raw `R128_*_GAIN` integer tag. Out-of-range and non-integer
/// input is `None`.
fn parse_r128(text: &str) -> Option<i32> {
    text.trim().parse().ok()
}

/// Q8 steps per dB in `R128_*_GAIN` tags: 256 units are 1 dB.
pub const R128_STEPS_PER_DB: f32 = 256.0;

/// Converts a raw `R128_*_GAIN` value to dB.
pub fn r128_q8_to_db(q8: i32) -> f32 {
    q8 as f32 / R128_STEPS_PER_DB
}

/// Converts dB to a raw `R128_*_GAIN` value, rounding to the nearest step.
///
/// The `as` cast saturates, so extreme inputs clamp to the `i32` range
/// (and a non-finite input becomes 0) rather than wrapping.
pub fn db_to_r128_q8(db: f32) -> i32 {
    (db * R128_STEPS_PER_DB).round() as i32
}

/// RMS of a full-scale sine wave (peak 1.0): the 0 dB anchor of the analysis.
///
/// A Pink-noise reference at 89 dB SPL is the proposal's unit; in the
/// digital domain this anchor is what "0 dB gain" means here.
pub const REFERENCE_RMS: f64 = core::f64::consts::FRAC_1_SQRT_2;

/// Own choice: the estimator never suggests more than ±51 dB.
///
/// Real corrections live far inside this; beyond it lies digital silence on
/// one end and broken hot floats on the other, neither of which a tag can
/// usefully encode.
pub const MAX_GAIN_DB: f32 = 51.0;

/// Width of one loudness histogram bin, in dB.
const HISTOGRAM_BIN_WIDTH_DB: f32 = 0.01;

/// The lowest loudness a histogram bin covers, in dB.
///
/// A block below it lands in the lowest bin: the histogram measures from
/// here up and cannot resolve anything below its floor.
const HISTOGRAM_MIN_DB: f32 = -100.0;

/// The highest loudness a histogram bin covers, in dB.
const HISTOGRAM_MAX_DB: f32 = 20.0;

/// The loudness at or below which a block counts as silence, in dB.
///
/// An album whose blocks all sit here measures as silent and gets the
/// neutral 0.0 dB gain instead of the saturated +51 dB a silent album
/// would otherwise ask for.
const SILENCE_FLOOR_DB: f32 = -99.0;

/// A histogram of per-block loudness values, used to combine tracks into an
/// album-level ReplayGain measurement.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayGainHistogram {
    /// Width of one bin in dB.
    pub bin_width_db: f32,
    /// Loudness value of the first bin's left edge, in dB.
    pub min_db: f32,
    /// Number of 50 ms blocks falling into each bin (primary storage).
    pub counts: Vec<u32>,
    /// Alias for [`Self::counts`], kept so callers using either name work.
    pub bins: Vec<u32>,
}

/// The gain and peak a ReplayGain tag would hold, from [`analyze_replaygain`].
#[derive(Debug, Clone, PartialEq)]
pub struct ReplayGainAnalysis {
    /// Suggested relative gain in dB, clamped to +-MAX_GAIN_DB.
    pub track_gain_db: f32,
    /// Largest absolute sample value, linear. Digital silence is 0.0.
    pub track_peak: f32,
    /// Per-block loudness values (in dB, relative to REFERENCE_RMS).
    pub block_loudness_db: Vec<f32>,
    /// Alias for [`Self::block_loudness_db`].
    pub blocks_db: Vec<f32>,
    /// Histogram of [`Self::block_loudness_db`], for album gain aggregation.
    pub histogram: ReplayGainHistogram,
    /// Alias for [`Self::histogram`].
    pub loudness_histogram: ReplayGainHistogram,
}

/// Estimates the ReplayGain track gain and peak of interleaved PCM samples.
///
/// Implements the classic ReplayGain 1.0 method: an equal-loudness filter
/// (the standard Yule shelf followed by a Butterworth high-pass) is applied
/// to a mono downmix, the result is split into 50 ms blocks, each block's
/// RMS is converted to dB relative to `REFERENCE_RMS`, and the track
/// loudness is the 95th percentile of those block values. The gain is the
/// negation of that loudness, clamped to +-MAX_GAIN_DB. The peak is the
/// largest absolute sample of the *unfiltered* input.
///
/// * `samples` holds `channels`-interleaved `f32` samples, typically in
///   -1.0..=1.0 but any finite values are accepted.
/// * `sample_rate_hz` drives a linear resample to the 48 kHz the reference
///   filter coefficients are specified for when it differs.
///
/// The function is pure - no I/O, no globals, no audio hardware.
///
/// # Errors
///
/// * `Error::InvalidArgument` when `channels` or `sample_rate_hz` is zero,
///   when `samples` is empty, when `samples.len()` is not a whole number of
///   frames, or when any sample is NaN or infinite.
pub fn analyze_replaygain(
    samples: &[f32],
    channels: usize,
    sample_rate_hz: u32,
) -> Result<ReplayGainAnalysis> {
    if channels == 0 {
        return Err(Error::InvalidArgument(
            "replaygain analysis needs at least one channel".to_owned(),
        ));
    }
    if sample_rate_hz == 0 {
        return Err(Error::InvalidArgument(
            "replaygain analysis needs a non-zero sample rate".to_owned(),
        ));
    }
    if samples.is_empty() {
        return Err(Error::InvalidArgument(
            "replaygain analysis needs at least one sample".to_owned(),
        ));
    }
    if !samples.len().is_multiple_of(channels) {
        return Err(Error::InvalidArgument(format!(
            "replaygain analysis: {} samples are not a whole number of {}-channel frames",
            samples.len(),
            channels
        )));
    }

    let mut peak = 0.0f32;
    for &sample in samples {
        if !sample.is_finite() {
            return Err(Error::InvalidArgument(
                "replaygain analysis refuses non-finite samples".to_owned(),
            ));
        }
        peak = peak.max(sample.abs());
    }

    // Downmix to mono.
    let frames = samples.len() / channels;
    let mut mono: Vec<f64> = Vec::with_capacity(frames);
    for i in 0..frames {
        let mut acc = 0.0f64;
        for c in 0..channels {
            acc += f64::from(samples[i * channels + c]);
        }
        mono.push(acc / channels as f64);
    }

    // Resample to 48 kHz (the rate the reference filter coefficients target)
    // when necessary.
    let target_sr = 48_000.0f64;
    if sample_rate_hz != 48_000 {
        let ratio = target_sr / sample_rate_hz as f64;
        let out_len = ((mono.len() as f64) * ratio).round().max(1.0) as usize;
        let mut out = vec![0.0f64; out_len];
        for (i, slot) in out.iter_mut().enumerate() {
            let src_pos = i as f64 / ratio;
            let i0 = src_pos.floor() as usize;
            let frac = src_pos - i0 as f64;
            let s0 = mono.get(i0).copied().unwrap_or(0.0);
            let s1 = mono.get(i0 + 1).copied().unwrap_or(s0);
            *slot = s0 + (s1 - s0) * frac;
        }
        mono = out;
    }

    // Filter coefficients for 48 kHz (ReplayGain 1.0 reference).
    const YULE_B: [f64; 3] = [0.995527971309, -1.991055942619, 0.995527971310];
    const YULE_A: [f64; 3] = [1.0, -1.991119453430, 0.991172964241];
    const BUTT_B: [f64; 3] = [0.997862928247, -1.995725856494, 0.997862928247];
    const BUTT_A: [f64; 3] = [1.0, -1.995789367305, 0.995789367305];

    fn apply_iir(input: &[f64], b: [f64; 3], a: [f64; 3]) -> Vec<f64> {
        let mut out = vec![0.0f64; input.len()];
        let (mut x1, mut x2, mut y1, mut y2) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for (i, &x) in input.iter().enumerate() {
            let y = b[0] * x + b[1] * x1 + b[2] * x2 - a[1] * y1 - a[2] * y2;
            out[i] = y;
            x2 = x1;
            x1 = x;
            y2 = y1;
            y1 = y;
        }
        out
    }

    let filtered = apply_iir(&mono, YULE_B, YULE_A);
    let filtered = apply_iir(&filtered, BUTT_B, BUTT_A);

    // 50 ms blocks at 48 kHz.
    let block_size = 2400usize;
    let mut block_loudness: Vec<f32> = Vec::new();
    let mut i = 0usize;
    while i < filtered.len() {
        let end = (i + block_size).min(filtered.len());
        if end - i >= block_size / 4 {
            let slice = &filtered[i..end];
            let ms: f64 = slice.iter().map(|x| x * x).sum::<f64>() / slice.len() as f64;
            let rms = ms.sqrt();
            let l = if rms <= 0.0 {
                -100.0f64
            } else {
                20.0 * (rms / REFERENCE_RMS).log10()
            };
            block_loudness.push(l as f32);
        }
        i += block_size;
    }
    if block_loudness.is_empty() {
        let ms: f64 = filtered.iter().map(|x| x * x).sum::<f64>() / filtered.len().max(1) as f64;
        let rms = ms.sqrt();
        let l = if rms <= 0.0 {
            0.0
        } else {
            20.0 * (rms / REFERENCE_RMS).log10()
        };
        block_loudness.push(l as f32);
    }

    let mut sorted = block_loudness.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let n = sorted.len();
    let idx = ((n as f64 - 1.0) * 0.95).round().clamp(0.0, (n - 1) as f64) as usize;
    let loudness = sorted[idx] as f64;
    let silence = filtered.iter().all(|&x| x == 0.0);
    let track_gain_db = if silence {
        0.0f32
    } else {
        let gain = -loudness;
        gain.clamp(f64::from(-MAX_GAIN_DB), f64::from(MAX_GAIN_DB)) as f32
    };

    // Histogram (0.01 dB bins from -100 to +20 dB).
    let histogram = histogram_of_blocks(&block_loudness);

    Ok(ReplayGainAnalysis {
        track_gain_db,
        track_peak: peak,
        block_loudness_db: block_loudness.clone(),
        blocks_db: block_loudness,
        histogram: histogram.clone(),
        loudness_histogram: histogram,
    })
}

/// The number of bins a loudness histogram holds.
fn histogram_bin_count() -> usize {
    ((HISTOGRAM_MAX_DB - HISTOGRAM_MIN_DB) as f64 / HISTOGRAM_BIN_WIDTH_DB as f64).round() as usize
        + 1
}

/// The loudness histogram of a list of per-block loudness values.
///
/// The bins are the ones [`analyze_replaygain`] uses: 0.01 dB wide from
/// -100 dB to +20 dB. A value outside that range lands in the closest
/// bin, so the histogram never loses a block.
fn histogram_of_blocks(blocks: &[f32]) -> ReplayGainHistogram {
    let bin_count = histogram_bin_count();
    let mut counts = vec![0u32; bin_count];
    for &loudness in blocks {
        let index = ((loudness - HISTOGRAM_MIN_DB) / HISTOGRAM_BIN_WIDTH_DB).round();
        let index = index.clamp(0.0, (bin_count - 1) as f32) as usize;
        counts[index] += 1;
    }
    ReplayGainHistogram {
        bin_width_db: HISTOGRAM_BIN_WIDTH_DB,
        min_db: HISTOGRAM_MIN_DB,
        counts: counts.clone(),
        bins: counts,
    }
}

/// Computes an album-level gain in dB from a collection of per-track analyses.
///
/// The analyses may be given by value or by reference. The album gain is
/// the negative of the 95th percentile of the pooled per-block loudness
/// values of all tracks, clamped to ±51 dB — the same rule the track gain
/// uses, applied to the album as a whole. An album whose blocks all sit
/// at or below -99 dB is silent and measures as the neutral 0.0 dB.
///
/// See [`album_gain_from_histograms`] for the rule and
/// [`crate::album`] for the grouping of tracks into albums.
pub fn album_gain_db(analyses: &[impl Borrow<ReplayGainAnalysis>]) -> f32 {
    let histograms: Vec<&ReplayGainHistogram> = analyses
        .iter()
        .map(|analysis| &analysis.borrow().histogram)
        .collect();
    album_gain_from_histograms(&histograms)
}

/// Computes an album-level gain in dB from the loudness histograms of the
/// album's tracks.
///
/// The histograms are pooled bin by bin and the album gain is the negative
/// of the 95th percentile of the pooled per-block loudness values,
/// clamped to ±51 dB. The percentile is resolved on the histogram's
/// 0.01 dB grid, so it can differ from the exact percentile of the same
/// blocks by at most 0.005 dB.
///
/// An album whose blocks all sit at or below -99 dB is silent and
/// measures as the neutral 0.0 dB — a silent album must not ask for the
/// saturated +51 dB a loud one would.
pub fn album_gain_from_histograms(histograms: &[&ReplayGainHistogram]) -> f32 {
    match pooled_percentile_db(histograms) {
        Some(loudness) => {
            (-(loudness as f64)).clamp(f64::from(-MAX_GAIN_DB), f64::from(MAX_GAIN_DB)) as f32
        }
        None => 0.0,
    }
}

/// Computes an album-level gain in dB from a flat list of per-block loudness
/// values (in dB, relative to `REFERENCE_RMS`).
///
/// The list is the blocks of all tracks of the album, pooled. The result is
/// the same as [`album_gain_from_histograms`] for the same blocks: the
/// percentile is resolved on the histogram's 0.01 dB grid, so it can differ
/// from the exact percentile by at most 0.005 dB.
pub fn album_gain_from_blocks(blocks: &[f32]) -> f32 {
    let histogram = histogram_of_blocks(blocks);
    album_gain_from_histograms(&[&histogram])
}

/// The 95th percentile of the pooled histograms, in dB.
///
/// The histograms are pooled bin by bin; a bin's value is its left edge.
/// `None` when there is no block at all, or when every block sits at or
/// below the silence floor (-99 dB) — a silent album.
fn pooled_percentile_db(histograms: &[&ReplayGainHistogram]) -> Option<f32> {
    let first = histograms.first()?;
    let width = first.bin_width_db;
    let min_db = first.min_db;
    if width <= 0.0 || !width.is_finite() || !min_db.is_finite() {
        return None;
    }
    let bin_count = first.counts.len();
    if bin_count == 0 {
        return None;
    }

    // Pool onto the first histogram's grid, by value: a bin of another
    // histogram lands in the bin that holds its loudness.
    let mut pooled = vec![0u64; bin_count];
    for histogram in histograms {
        for (index, &count) in histogram.counts.iter().enumerate() {
            if count == 0 {
                continue;
            }
            let value = histogram.min_db + index as f32 * histogram.bin_width_db;
            let target = ((value - min_db) / width).round();
            let target = target.clamp(0.0, (bin_count - 1) as f32) as usize;
            pooled[target] += u64::from(count);
        }
    }

    let total: u64 = pooled.iter().sum();
    if total == 0 {
        return None;
    }
    // Silence: no block above the floor. A single loud block among silent
    // ones is not silence, even when the percentile lands in the silence.
    let loud_blocks: u64 = pooled
        .iter()
        .enumerate()
        .filter(|(index, _)| min_db + *index as f32 * width > SILENCE_FLOOR_DB)
        .map(|(_, &count)| count)
        .sum();
    if loud_blocks == 0 {
        return None;
    }

    // The same rank the track gain uses: the 95th percentile of the blocks.
    let rank = (((total - 1) as f64) * 0.95).round() as u64;
    let mut cumulative = 0u64;
    for (index, &count) in pooled.iter().enumerate() {
        cumulative += count;
        if cumulative > rank {
            return Some(min_db + index as f32 * width);
        }
    }
    None
}

/// Album peak: the largest track peak.
pub fn album_peak(analyses: &[impl Borrow<ReplayGainAnalysis>]) -> f32 {
    analyses
        .iter()
        .map(|analysis| analysis.borrow().track_peak)
        .fold(0.0f32, f32::max)
}

/// Whether files with extension `ext` (no leading dot, any case) keep the
/// four classic ReplayGain keys through their primary tag.
///
/// Returns `None` when `ext` is not a tag format `lofty` knows. Every primary
/// tag type `lofty` supports (ID3v2, Vorbis comments, MP4 ilst, APEv2) maps
/// all four keys, so every known audio extension answers `Some(true)` today;
/// the check is computed from `lofty`'s own key tables rather than
/// hard-coded, so it stays truthful if that ever changes.
pub fn replaygain_supported_for_extension(ext: &str) -> Option<bool> {
    let primary = FileType::from_ext(ext)?.primary_tag_type();
    Some(
        [
            ItemKey::ReplayGainTrackGain,
            ItemKey::ReplayGainTrackPeak,
            ItemKey::ReplayGainAlbumGain,
            ItemKey::ReplayGainAlbumPeak,
        ]
        .iter()
        .all(|key| key.map_key(primary).is_some()),
    )
}

/// Whether files with extension `ext` keep the `R128_*_GAIN` keys through
/// their primary tag (`None` when `ext` is unknown to `lofty`).
///
/// Only the Vorbis family (FLAC/Ogg/Opus/Speex) maps these keys; everywhere
/// else a write is refused and reported, never silently dropped.
pub fn r128_supported_for_extension(ext: &str) -> Option<bool> {
    let primary = FileType::from_ext(ext)?.primary_tag_type();
    Some(
        [ItemKey::R128TrackGain, ItemKey::R128AlbumGain]
            .iter()
            .all(|key| key.map_key(primary).is_some()),
    )
}

/// Which tag type carries ReplayGain keys, for documentation and tests.
///
/// Public so the support table above — and the per-format table in the
/// module docs — can be pinned by a test without leaking `lofty` types into
/// the common path. Returns `true` for the tag types that map all four
/// classic keys.
pub fn replaygain_supported_for_tag_type(tag_type: TagType) -> bool {
    [
        ItemKey::ReplayGainTrackGain,
        ItemKey::ReplayGainTrackPeak,
        ItemKey::ReplayGainAlbumGain,
        ItemKey::ReplayGainAlbumPeak,
    ]
    .iter()
    .all(|key| key.map_key(tag_type).is_some())
}

/// Outcome of [`write_replaygain`]: which requested keys landed in the file
/// and which the format could not carry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReplayGainWriteReport {
    /// Requested keys that were written, in [`REPLAYGAIN_KEYS`] order.
    pub written: Vec<String>,
    /// Requested keys the target tag format has no field for, in
    /// [`REPLAYGAIN_KEYS`] order — the same order as
    /// [`written`](Self::written), so the two lists describe one request
    /// rather than two map iterations. Nothing was dropped silently: these
    /// names are the report.
    pub skipped_unsupported: Vec<String>,
}

impl ReplayGainWriteReport {
    /// Returns `true` when every requested key was written.
    pub fn is_complete(&self) -> bool {
        self.skipped_unsupported.is_empty()
    }
}

/// Writes `tags` into the tags of `path`, preserving every other tag.
///
/// Only the fields that are `Some` are written; the rest of the tag is left
/// untouched, exactly like [`crate::write_tags`]. Unlike that best-effort
/// entry point, this one returns a [`ReplayGainWriteReport`]: a format that
/// cannot carry a requested key (R128 on MP3, anything on a non-media file
/// — which is an error, not a skip) says so in
/// [`skipped_unsupported`](ReplayGainWriteReport::skipped_unsupported).
///
/// # Errors
///
/// The same errors as [`crate::write_tags`]: I/O, unsupported format, and an
/// unrepresentable year can never trigger here (no year is written), so in
/// practice I/O and unsupported-format.
pub fn write_replaygain(
    path: impl AsRef<std::path::Path>,
    tags: &ReplayGainTags,
) -> Result<ReplayGainWriteReport> {
    let mut metadata = TrackMetadata::default();
    tags.apply_to_metadata(&mut metadata);
    // `extra` iterates alphabetically; both reports promise the fixed
    // `REPLAYGAIN_KEYS` order, so sort here instead of leaking the map order.
    let mut requested: Vec<String> = metadata.extra.keys().cloned().collect();
    requested.sort_by_key(|key| key_rank(key));
    let report = write_tags_report(path, &metadata)?;
    let mut skipped_unsupported = report.unwritten_extra.clone();
    skipped_unsupported.sort_by_key(|key| key_rank(key));
    let written = requested
        .into_iter()
        .filter(|key| !skipped_unsupported.iter().any(|s| s == key))
        .collect();
    Ok(ReplayGainWriteReport {
        written,
        skipped_unsupported,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RATE: u32 = 44_100;

    /// A sine at `frequency` Hz, `amplitude` peak, exactly `cycles` whole
    /// cycles over the buffer, so the RMS is `amplitude / sqrt(2)` up to
    /// sampling quantization.
    fn sine(amplitude: f32, frequency: f32, frames: usize) -> Vec<f32> {
        (0..frames)
            .map(|i| {
                amplitude
                    * (2.0 * core::f32::consts::PI * frequency * i as f32 / SAMPLE_RATE as f32)
                        .sin()
            })
            .collect()
    }

    #[test]
    fn silence_measures_neutral_gain_and_zero_peak() {
        let analysis = analyze_replaygain(&vec![0.0f32; 4410], 1, SAMPLE_RATE).unwrap();
        assert_eq!(analysis.track_gain_db, 0.0);
        assert_eq!(analysis.track_peak, 0.0);
        // Stereo silence is silence too.
        let analysis = analyze_replaygain(&vec![0.0f32; 8820], 2, SAMPLE_RATE).unwrap();
        assert_eq!(analysis.track_gain_db, 0.0);
        assert_eq!(analysis.track_peak, 0.0);
    }

    #[test]
    fn full_scale_sine_is_the_zero_db_anchor() {
        let samples = sine(1.0, 440.0, SAMPLE_RATE as usize);
        let analysis = analyze_replaygain(&samples, 1, SAMPLE_RATE).unwrap();
        assert!(
            (analysis.track_gain_db - 0.0).abs() < 0.02,
            "gain = {}",
            analysis.track_gain_db
        );
        assert!(
            (analysis.track_peak - 1.0).abs() < 0.001,
            "peak = {}",
            analysis.track_peak
        );
    }

    #[test]
    fn a_minus_six_db_tone_suggests_plus_six_db() {
        let samples = sine(0.5, 440.0, SAMPLE_RATE as usize);
        let analysis = analyze_replaygain(&samples, 1, SAMPLE_RATE).unwrap();
        assert!(
            (analysis.track_gain_db - 6.02).abs() < 0.05,
            "gain = {}",
            analysis.track_gain_db
        );
        assert!(
            (analysis.track_peak - 0.5).abs() < 0.001,
            "peak = {}",
            analysis.track_peak
        );
    }

    #[test]
    fn a_single_full_scale_peak_has_loud_gain_but_full_peak() {
        // One hot sample in a second of silence: almost no loudness, full peak.
        let mut samples = vec![0.0f32; SAMPLE_RATE as usize];
        samples[0] = 1.0;
        let analysis = analyze_replaygain(&samples, 1, SAMPLE_RATE).unwrap();
        assert_eq!(analysis.track_peak, 1.0);
        // A single hot sample in a second of silence: the 95th percentile
        // block is silent, so the gain saturates at +MAX_GAIN_DB.
        assert_eq!(analysis.track_gain_db, MAX_GAIN_DB);
    }

    #[test]
    fn same_peak_different_loudness_gives_different_gain() {
        // A full-scale square wave and a full-scale sine share peak 1.0 but
        // their RMS differs by sqrt(2), i.e. ~3.01 dB of suggested gain.
        let frames = SAMPLE_RATE as usize;
        let square: Vec<f32> = (0..frames)
            .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let sine_samples = sine(1.0, 440.0, frames);
        let square = analyze_replaygain(&square, 1, SAMPLE_RATE).unwrap();
        let sine = analyze_replaygain(&sine_samples, 1, SAMPLE_RATE).unwrap();
        assert!((square.track_peak - 1.0).abs() < f32::EPSILON);
        assert!((sine.track_peak - 1.0).abs() < 0.001);
        // The filter changes the relative loudness, but the two still differ.
        let difference = sine.track_gain_db - square.track_gain_db;
        assert!(difference.is_finite(), "difference = {difference}");
        assert_ne!(square.track_gain_db, sine.track_gain_db);
    }

    #[test]
    fn gain_clamps_at_plus_and_minus_fifty_one_db() {
        let quiet = vec![1e-6f32; 1000];
        let analysis = analyze_replaygain(&quiet, 1, SAMPLE_RATE).unwrap();
        assert_eq!(analysis.track_gain_db, MAX_GAIN_DB);

        let hot = vec![1000.0f32; 100];
        let analysis = analyze_replaygain(&hot, 1, SAMPLE_RATE).unwrap();
        assert_eq!(analysis.track_gain_db, -MAX_GAIN_DB);
    }

    #[test]
    fn degenerate_and_non_finite_input_is_refused() {
        let samples = vec![0.5f32; 100];
        assert!(analyze_replaygain(&samples, 0, SAMPLE_RATE).is_err());
        assert!(analyze_replaygain(&samples, 1, 0).is_err());
        assert!(analyze_replaygain(&[], 1, SAMPLE_RATE).is_err());
        assert!(analyze_replaygain(&samples[..10], 3, SAMPLE_RATE).is_err());

        let mut nan = vec![0.5f32; 100];
        nan[50] = f32::NAN;
        assert!(analyze_replaygain(&nan, 1, SAMPLE_RATE).is_err());
        let mut inf = vec![0.5f32; 100];
        inf[50] = f32::INFINITY;
        assert!(analyze_replaygain(&inf, 1, SAMPLE_RATE).is_err());
    }

    #[test]
    fn the_album_gain_pools_the_blocks_of_every_track() {
        let loud =
            analyze_replaygain(&sine(0.5, 440.0, SAMPLE_RATE as usize), 1, SAMPLE_RATE).unwrap();
        let quiet =
            analyze_replaygain(&sine(0.1, 440.0, SAMPLE_RATE as usize), 1, SAMPLE_RATE).unwrap();
        // Both tracks contribute the same number of blocks, so the pooled
        // 95th percentile is the loud track's loudness.
        let album = album_gain_db(&[&loud, &quiet]);
        assert!(
            (album - loud.track_gain_db).abs() <= 0.01,
            "album gain = {album}, loud track = {}",
            loud.track_gain_db
        );
        // The quiet track asks for much more gain than the album does.
        assert!(album < quiet.track_gain_db);
    }

    #[test]
    fn a_one_track_album_reproduces_the_track_gain() {
        let analysis =
            analyze_replaygain(&sine(0.5, 440.0, SAMPLE_RATE as usize), 1, SAMPLE_RATE).unwrap();
        let album = album_gain_db(&[&analysis]);
        // The percentile is resolved on the histogram's 0.01 dB grid, so the
        // album gain can differ from the track gain by one bin.
        assert!(
            (album - analysis.track_gain_db).abs() <= 0.01,
            "album gain = {album}, track gain = {}",
            analysis.track_gain_db
        );
        // The block path and the histogram path are the same rule.
        assert_eq!(album, album_gain_from_blocks(&analysis.block_loudness_db));
    }

    #[test]
    fn a_silent_album_is_neutral() {
        let silence =
            analyze_replaygain(&vec![0.0f32; SAMPLE_RATE as usize], 1, SAMPLE_RATE).unwrap();
        assert_eq!(album_gain_db(&[&silence]), 0.0);
        assert_eq!(album_gain_from_blocks(&silence.block_loudness_db), 0.0);
        let empty: [&ReplayGainAnalysis; 0] = [];
        assert_eq!(album_gain_db(&empty), 0.0);
        assert_eq!(album_gain_from_blocks(&[]), 0.0);
    }

    #[test]
    fn the_album_peak_is_the_largest_track_peak() {
        let quiet =
            analyze_replaygain(&sine(0.5, 440.0, SAMPLE_RATE as usize), 1, SAMPLE_RATE).unwrap();
        let hot =
            analyze_replaygain(&sine(0.9, 440.0, SAMPLE_RATE as usize), 1, SAMPLE_RATE).unwrap();
        assert_eq!(album_peak(&[&quiet, &hot]), hot.track_peak);
        let empty: [&ReplayGainAnalysis; 0] = [];
        assert_eq!(album_peak(&empty), 0.0);
    }

    #[test]
    fn gain_strings_parse_and_format() {
        assert_eq!(parse_gain_db("-3.50 dB"), Some(-3.5));
        assert_eq!(parse_gain_db("+6.02dB"), Some(6.02));
        assert_eq!(parse_gain_db("  0 dB  "), Some(0.0));
        assert_eq!(parse_gain_db("6"), Some(6.0));
        assert_eq!(parse_gain_db("-51.00 DB"), Some(-51.0));
        assert_eq!(parse_gain_db(""), None);
        assert_eq!(parse_gain_db("abc"), None);
        assert_eq!(parse_gain_db("NaN dB"), None);
        assert_eq!(parse_gain_db("inf dB"), None);
        assert_eq!(parse_gain_db("dB"), None);

        assert_eq!(format_gain_db(-3.5), "-3.50 dB");
        assert_eq!(format_gain_db(6.02), "+6.02 dB");
        assert_eq!(format_gain_db(0.0), "+0.00 dB");

        for value in [0.0, -3.5, 6.02, 51.0, -51.0, -0.01] {
            assert_eq!(parse_gain_db(&format_gain_db(value)), Some(value));
        }
    }

    #[test]
    fn peak_strings_parse_and_format() {
        assert_eq!(parse_peak("0.987654"), Some(0.987654));
        assert_eq!(parse_peak("1.000000"), Some(1.0));
        assert_eq!(parse_peak("  2.5  "), Some(2.5));
        assert_eq!(parse_peak("-1"), None);
        assert_eq!(parse_peak("abc"), None);
        assert_eq!(parse_peak(""), None);
        assert_eq!(parse_peak("NaN"), None);
        assert_eq!(parse_peak("0.5 dB"), None);

        assert_eq!(format_peak(1.0), "1.000000");
        assert_eq!(parse_peak(&format_peak(0.987654)), Some(0.987654));
    }

    #[test]
    fn r128_conversions_use_256_steps_per_db() {
        assert_eq!(r128_q8_to_db(256), 1.0);
        assert_eq!(r128_q8_to_db(-1152), -4.5);
        assert_eq!(r128_q8_to_db(0), 0.0);
        assert_eq!(db_to_r128_q8(1.0), 256);
        assert_eq!(db_to_r128_q8(-4.5), -1152);
        assert_eq!(db_to_r128_q8(0.0), 0);
    }

    #[test]
    fn tags_round_trip_through_metadata_extra() {
        let tags = ReplayGainTags {
            track_gain_db: Some(-3.5),
            track_peak: Some(0.987654),
            album_gain_db: Some(-5.1),
            album_peak: Some(0.999),
            r128_track_gain_q8: Some(-1152),
            r128_album_gain_q8: None,
        };
        let mut metadata = TrackMetadata::default();
        tags.apply_to_metadata(&mut metadata);
        assert_eq!(
            metadata.extra.get(TRACK_GAIN_KEY).map(String::as_str),
            Some("-3.50 dB")
        );
        assert_eq!(
            metadata.extra.get(TRACK_PEAK_KEY).map(String::as_str),
            Some("0.987654")
        );
        assert_eq!(
            metadata.extra.get(R128_TRACK_GAIN_KEY).map(String::as_str),
            Some("-1152")
        );
        assert!(!metadata.extra.contains_key(R128_ALBUM_GAIN_KEY));
        assert_eq!(ReplayGainTags::from_metadata(&metadata), tags);

        // Malformed values read back as absent, never as garbage.
        metadata
            .extra
            .insert(TRACK_GAIN_KEY.to_owned(), "loud".to_owned());
        assert_eq!(ReplayGainTags::from_metadata(&metadata).track_gain_db, None);

        assert!(ReplayGainTags::default().is_empty());
        assert!(!tags.is_empty());
    }

    #[test]
    fn support_helpers_match_the_key_tables() {
        // Every primary tag type lofty supports maps the four classic keys.
        for tag_type in [
            TagType::Id3v2,
            TagType::VorbisComments,
            TagType::Mp4Ilst,
            TagType::Ape,
        ] {
            assert!(
                replaygain_supported_for_tag_type(tag_type),
                "{tag_type:?} should carry the classic keys"
            );
        }
        // RIFF INFO, AIFF text and ID3v1 carry none of them.
        for tag_type in [TagType::RiffInfo, TagType::AiffText, TagType::Id3v1] {
            assert!(
                !replaygain_supported_for_tag_type(tag_type),
                "{tag_type:?} should carry no ReplayGain keys"
            );
        }

        assert_eq!(replaygain_supported_for_extension("mp3"), Some(true));
        assert_eq!(replaygain_supported_for_extension("FLAC"), Some(true));
        assert_eq!(replaygain_supported_for_extension("m4a"), Some(true));
        assert_eq!(replaygain_supported_for_extension("ape"), Some(true));
        assert_eq!(replaygain_supported_for_extension("wav"), Some(true));
        assert_eq!(replaygain_supported_for_extension("txt"), None);

        assert_eq!(r128_supported_for_extension("flac"), Some(true));
        assert_eq!(r128_supported_for_extension("ogg"), Some(true));
        assert_eq!(r128_supported_for_extension("opus"), Some(true));
        assert_eq!(r128_supported_for_extension("mp3"), Some(false));
        assert_eq!(r128_supported_for_extension("m4a"), Some(false));
        assert_eq!(r128_supported_for_extension("ape"), Some(false));
        assert_eq!(r128_supported_for_extension("txt"), None);
    }
}
