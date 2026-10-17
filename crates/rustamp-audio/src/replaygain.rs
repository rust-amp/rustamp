// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! ReplayGain on the decode path: read the tags, turn the gain into a linear
//! multiplier, apply it before the DSP chain.
//!
//! # Where the gain is applied, and why
//!
//! The per-track multiplier is applied on the **decode thread**, to the raw
//! decoder output **before** the first DSP stage (the equalizer), so the
//! chain is:
//!
//! ```text
//! decoder -> ReplayGain scale -> Equalizer -> [installed stages] -> Limiter
//!         -> FormatConverter -> ring -> volume/balance (render callback) -> device
//! ```
//!
//! This placement is deliberate:
//!
//! * The limiter and the converter's final `±1.0` ceiling (A14) only protect
//!   what passes through them. A boost applied after them — in the render
//!   callback next to volume/balance — could put above-full-scale samples on
//!   the device with no stage left to catch them. Volume and balance only
//!   ever attenuate (see [`crate::gain`]), which is why that invariant holds
//!   for them and would not hold for a ReplayGain boost there.
//! * Applied before the limiter, a hot correction (quiet track, positive
//!   gain) is limited at the −0.3 dBFS ceiling like any other hot signal,
//!   and whatever the resampler rebuilds above it is caught by the
//!   converter's 0 dBFS ceiling. The limiter and the converter bound from
//!   A14 therefore still hold unchanged, with no new clip protection needed.
//! * It is **not** folded into the volume/balance path: that path lives on
//!   the output thread's render callback and only attenuates. The ReplayGain
//!   multiplier is a per-track decode-side constant, read once per track
//!   from the decoder's metadata.
//!
//! # Reading the tags
//!
//! The tag keys and their `extra` encoding are owned by `rustamp-meta`
//! (`replaygain.rs`: `REPLAYGAIN_TRACK_GAIN` etc., gains as `"+6.02 dB"`,
//! peaks linear). This crate consumes that crate's constants and
//! [`rustamp_meta::replaygain::parse_gain_db`] directly, so there is a single
//! source of truth for the key spellings and the gain-string parsing. If the
//! metadata encoding ever changes, this module follows it automatically.
//!
//! # Policy (own choices, stated plainly)
//!
//! * **Track vs. album:** [`ReplayGainMode`]. `Track` uses the track gain and
//!   falls back to the album gain when the track has none; `Album` is the
//!   mirror. `Disabled` (the default) ignores every tag. The choice is a user
//!   setting because a player normalizes either per song or per album, never
//!   both at once; the setting has to live somewhere the UI can reach, which
//!   is engine-side here (see [`crate::engine`]).
//! * **Preamp:** a user offset in dB added on top of the selected gain,
//!   `0.0` dB by default, clamped to `±`[`MAX_PREAMP_DB`]. It only shifts a
//!   selected gain; it never invents one.
//! * **Peaks:** read by `rustamp-meta` but deliberately **not** used for
//!   pre-attenuation here. Peak tags are untrusted estimates, and
//!   pre-attenuating by `peak * gain` would double-correct material the
//!   limiter already shapes. A correction that overshoots is handled
//!   downstream: the limiter holds it at its ceiling, the converter clamps
//!   it at full scale.
//! * **Missing or malformed tag:** the track plays unadjusted (gain `1.0`),
//!   even with a preamp dialed in. Silence about loudness is not a license
//!   to guess.
//!
//! # Real-time safety
//!
//! [`ReplayGainControl`] is two atomics; the decode thread snapshots it once
//! per track. [`apply_gain`] allocates nothing, takes no lock and branches
//! once per block.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU8, AtomicU32, Ordering};

/// The `extra` key holding the album gain, e.g. `"-5.10 dB"`.
///
/// Re-exported from `rustamp_meta::replaygain::ALBUM_GAIN_KEY`: there is a
/// single owner of the key spelling.
pub use rustamp_meta::replaygain::ALBUM_GAIN_KEY;
/// The `extra` key holding the track gain, e.g. `"-3.50 dB"`.
///
/// Re-exported from `rustamp_meta::replaygain::TRACK_GAIN_KEY`: there is a
/// single owner of the key spelling.
pub use rustamp_meta::replaygain::TRACK_GAIN_KEY;

/// Own choice: the estimator in `rustamp-meta` never suggests more than
/// ±51 dB, and this side never applies more than that either. Real
/// corrections live far inside this; beyond it lies either digital silence
/// or a broken-hot float, and the limiter plus the converter ceiling are
/// the honest answers there, not more gain.
pub const MAX_GAIN_DB: f32 = 51.0;

/// Own choice: the user preamp range. Wider than the equalizer's ±12 dB
/// (a loudness offset is not a tone control), narrow enough that a slip of
/// the control cannot launch a quiet track 40 dB past the limiter.
pub const MAX_PREAMP_DB: f32 = 24.0;

/// Which ReplayGain value normalizes a track.
///
/// Own choice of shape: exactly one source at a time, with a silent fallback
/// to the other when the preferred one is missing, and a default of
/// [`ReplayGainMode::Disabled`] so the player never changes loudness until
/// the user asks it to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplayGainMode {
    /// Ignore every ReplayGain tag; play unadjusted.
    #[default]
    Disabled,
    /// Normalize per track; fall back to the album gain when the track has none.
    Track,
    /// Normalize per album; fall back to the track gain when the album has none.
    Album,
}

impl ReplayGainMode {
    /// The discriminant stored in [`ReplayGainControl`].
    fn discriminant(self) -> u8 {
        match self {
            ReplayGainMode::Disabled => 0,
            ReplayGainMode::Track => 1,
            ReplayGainMode::Album => 2,
        }
    }

    /// The mode for a stored discriminant; unknown values mean disabled,
    /// never a guess.
    fn from_discriminant(value: u8) -> Self {
        match value {
            1 => ReplayGainMode::Track,
            2 => ReplayGainMode::Album,
            _ => ReplayGainMode::Disabled,
        }
    }
}

/// The user's ReplayGain settings, shared between the engine (writer) and
/// the decode thread (reader). Lock-free: two atomics, no allocation.
///
/// Takes effect per track: the decode thread snapshots the control when it
/// opens a track, so a change mid-track applies from the next track (or a
/// seek, which re-opens the pipeline state).
#[derive(Debug, Default)]
pub struct ReplayGainControl {
    mode: AtomicU8,
    /// The preamp in dB, stored as bits so the whole control stays atomic.
    preamp_bits: AtomicU32,
}

impl ReplayGainControl {
    /// A control block with the given mode and preamp.
    pub fn new(mode: ReplayGainMode, preamp_db: f32) -> Self {
        let control = ReplayGainControl {
            mode: AtomicU8::new(mode.discriminant()),
            preamp_bits: AtomicU32::new(0.0f32.to_bits()),
        };
        control.set_mode(mode);
        control.set_preamp_db(preamp_db);
        control
    }

    /// The requested mode.
    pub fn mode(&self) -> ReplayGainMode {
        ReplayGainMode::from_discriminant(self.mode.load(Ordering::Acquire))
    }

    /// Request a mode. Takes effect from the next track (or seek).
    pub fn set_mode(&self, mode: ReplayGainMode) {
        self.mode.store(mode.discriminant(), Ordering::Release);
    }

    /// The requested preamp in dB.
    pub fn preamp_db(&self) -> f32 {
        f32::from_bits(self.preamp_bits.load(Ordering::Acquire))
    }

    /// Request a preamp in dB. Clamped to `±`[`MAX_PREAMP_DB`]; a
    /// non-finite value keeps the previous preamp, like the engine's EQ
    /// clamp keeps the previous band gain. Takes effect from the next
    /// track (or seek).
    pub fn set_preamp_db(&self, preamp_db: f32) {
        if preamp_db.is_finite() {
            self.preamp_bits.store(
                preamp_db.clamp(-MAX_PREAMP_DB, MAX_PREAMP_DB).to_bits(),
                Ordering::Release,
            );
        }
    }

    /// The linear multiplier for a track whose `extra` tags are `extra`:
    /// the selected gain plus the preamp, converted to linear, or `1.0`
    /// when the mode is disabled or no usable tag is present.
    pub fn effective_gain(&self, extra: &BTreeMap<String, String>) -> f32 {
        effective_gain(extra, self.mode(), self.preamp_db())
    }
}

/// Parses a gain tag such as `"-3.50 dB"`, `"+6.02dB"` or bare `"6"` into
/// dB. Re-exported from `rustamp_meta::replaygain::parse_gain_db`: the trailing
/// `dB` (any ASCII case) is optional, surrounding whitespace is ignored,
/// and anything that is not a finite number is `None`.
pub use rustamp_meta::replaygain::parse_gain_db;

/// Selects the gain in dB for `mode` from a track's `extra` tags.
///
/// `Track` prefers the track gain and falls back to the album gain;
/// `Album` is the mirror. Malformed values count as absent (a tag that does
/// not parse is not a value, as in `rustamp-meta`). `Disabled` and "no
/// usable tag" both select nothing.
pub fn select_gain_db(extra: &BTreeMap<String, String>, mode: ReplayGainMode) -> Option<f32> {
    // Delegate to the metadata crate's reader, so the key spellings and the
    // gain-string parsing live in one place.
    let metadata = rustamp_core::meta::TrackMetadata {
        extra: extra.clone(),
        ..Default::default()
    };
    let tags = rustamp_meta::replaygain::ReplayGainTags::from_metadata(&metadata);
    let track = tags.track_gain_db;
    let album = tags.album_gain_db;
    match mode {
        ReplayGainMode::Disabled => None,
        ReplayGainMode::Track => track.or(album),
        ReplayGainMode::Album => album.or(track),
    }
}

/// Converts a gain in dB to a linear multiplier: `10^(gain/20)`.
///
/// A non-finite input cannot come from a parsed tag (parsing rejects it)
/// and cannot be applied honestly, so it maps to `1.0` (unadjusted). Finite
/// input is clamped to `±`[`MAX_GAIN_DB`], mirroring the estimator's clamp
/// in `rustamp-meta`.
pub fn gain_db_to_linear(gain_db: f32) -> f32 {
    if !gain_db.is_finite() {
        return 1.0;
    }
    10.0f32.powf(gain_db.clamp(-MAX_GAIN_DB, MAX_GAIN_DB) / 20.0)
}

/// The linear multiplier for a track: the selected gain plus the preamp,
/// converted to linear.
///
/// A non-finite preamp is treated as `0.0` (it cannot shift a correction).
/// When no gain is selected — disabled mode, or no usable tag — the result
/// is `1.0` whatever the preamp says: a missing tag plays unadjusted.
pub fn effective_gain(
    extra: &BTreeMap<String, String>,
    mode: ReplayGainMode,
    preamp_db: f32,
) -> f32 {
    let Some(selected) = select_gain_db(extra, mode) else {
        return 1.0;
    };
    let preamp = if preamp_db.is_finite() {
        preamp_db
    } else {
        0.0
    };
    gain_db_to_linear(selected + preamp)
}

/// Applies a ReplayGain multiplier in place to interleaved `f32` samples.
///
/// Allocation-free and lock-free: a single multiply per sample. A gain of
/// exactly `1.0` — the missing-tag and 0 dB cases — is a no-op down to the
/// bit, and a non-finite gain (which [`effective_gain`] never returns) is
/// refused rather than spread.
#[inline]
pub fn apply_gain(buf: &mut [f32], gain: f32) {
    if gain == 1.0 || !gain.is_finite() {
        return;
    }
    for sample in buf.iter_mut() {
        *sample *= gain;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tagged(track: Option<&str>, album: Option<&str>) -> BTreeMap<String, String> {
        let mut extra = BTreeMap::new();
        if let Some(gain) = track {
            extra.insert(TRACK_GAIN_KEY.to_owned(), gain.to_owned());
        }
        if let Some(gain) = album {
            extra.insert(ALBUM_GAIN_KEY.to_owned(), gain.to_owned());
        }
        extra
    }

    #[test]
    fn gain_strings_parse_like_the_metadata_crate() {
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
    }

    #[test]
    fn track_is_preferred_with_album_as_fallback_and_vice_versa() {
        let both = tagged(Some("-3.50 dB"), Some("-5.10 dB"));
        assert_eq!(select_gain_db(&both, ReplayGainMode::Track), Some(-3.5));
        assert_eq!(select_gain_db(&both, ReplayGainMode::Album), Some(-5.1));

        let track_only = tagged(Some("-3.50 dB"), None);
        assert_eq!(
            select_gain_db(&track_only, ReplayGainMode::Album),
            Some(-3.5),
            "album mode falls back to the track gain"
        );

        let album_only = tagged(None, Some("-5.10 dB"));
        assert_eq!(
            select_gain_db(&album_only, ReplayGainMode::Track),
            Some(-5.1),
            "track mode falls back to the album gain"
        );

        assert_eq!(select_gain_db(&both, ReplayGainMode::Disabled), None);
        assert_eq!(
            select_gain_db(&BTreeMap::new(), ReplayGainMode::Track),
            None
        );
    }

    #[test]
    fn malformed_values_count_as_absent() {
        let broken = tagged(Some("loud"), Some("-5.10 dB"));
        assert_eq!(
            select_gain_db(&broken, ReplayGainMode::Track),
            Some(-5.1),
            "an unparsable track gain falls back to the album gain"
        );
        let both_broken = tagged(Some("loud"), Some("quiet"));
        assert_eq!(select_gain_db(&both_broken, ReplayGainMode::Track), None);
    }

    #[test]
    fn db_to_linear_matches_the_documented_factors() {
        assert_eq!(gain_db_to_linear(0.0), 1.0);
        let down = gain_db_to_linear(-6.02);
        assert!(
            (down - 0.5).abs() < 1e-3,
            "-6.02 dB halves the samples: {down}"
        );
        let up = gain_db_to_linear(6.02);
        assert!(
            (up - 2.0).abs() < 1e-3,
            "+6.02 dB doubles the samples: {up}"
        );
        assert_eq!(gain_db_to_linear(f32::NAN), 1.0);
        assert_eq!(gain_db_to_linear(f32::INFINITY), 1.0);
        // The clamp mirrors the estimator: ±51 dB, no more.
        assert_eq!(gain_db_to_linear(100.0), gain_db_to_linear(MAX_GAIN_DB));
        assert_eq!(gain_db_to_linear(-100.0), gain_db_to_linear(-MAX_GAIN_DB));
    }

    #[test]
    fn the_preamp_shifts_a_selected_gain_and_nothing_else() {
        let extra = tagged(Some("-6.02 dB"), None);
        let without = effective_gain(&extra, ReplayGainMode::Track, 0.0);
        let with = effective_gain(&extra, ReplayGainMode::Track, 6.02);
        assert!((without - 0.5).abs() < 1e-3, "{without}");
        assert!(
            (with - 1.0).abs() < 1e-3,
            "the preamp cancels the cut: {with}"
        );

        // No tag, no gain — even with a preamp dialed in.
        assert_eq!(
            effective_gain(&BTreeMap::new(), ReplayGainMode::Track, 6.0),
            1.0
        );
        assert_eq!(effective_gain(&extra, ReplayGainMode::Disabled, 6.0), 1.0);
        // A non-finite preamp cannot shift a correction.
        assert_eq!(
            effective_gain(&extra, ReplayGainMode::Track, f32::NAN),
            without
        );
    }

    #[test]
    fn unity_and_non_finite_gains_leave_the_buffer_bit_exact() {
        let mut buf = [0.5f32, -0.25, 1.1, -1.1];
        let before = buf;
        apply_gain(&mut buf, 1.0);
        assert_eq!(buf, before, "unity is a no-op");
        apply_gain(&mut buf, f32::NAN);
        assert_eq!(buf, before, "non-finite is refused");
        apply_gain(&mut buf, f32::INFINITY);
        assert_eq!(buf, before, "non-finite is refused");
    }

    #[test]
    fn a_known_gain_moves_every_sample_by_exactly_that_factor() {
        let mut buf = [0.5f32, -0.25, 1.0, -1.0];
        let gain = gain_db_to_linear(-6.02);
        apply_gain(&mut buf, gain);
        for (got, source) in buf.iter().zip([0.5f32, -0.25, 1.0, -1.0]) {
            assert_eq!(*got, source * gain);
        }
    }

    #[test]
    fn the_control_round_trips_mode_and_preamp() {
        let control = ReplayGainControl::default();
        assert_eq!(control.mode(), ReplayGainMode::Disabled);
        assert_eq!(control.preamp_db(), 0.0);

        control.set_mode(ReplayGainMode::Album);
        control.set_preamp_db(-3.0);
        assert_eq!(control.mode(), ReplayGainMode::Album);
        assert_eq!(control.preamp_db(), -3.0);

        control.set_preamp_db(100.0);
        assert_eq!(control.preamp_db(), MAX_PREAMP_DB, "clamped");
        control.set_preamp_db(-100.0);
        assert_eq!(control.preamp_db(), -MAX_PREAMP_DB, "clamped");
        control.set_preamp_db(f32::NAN);
        assert_eq!(
            control.preamp_db(),
            -MAX_PREAMP_DB,
            "non-finite keeps the previous preamp"
        );
    }

    #[test]
    fn the_control_computes_the_track_multiplier() {
        let control = ReplayGainControl::new(ReplayGainMode::Track, 0.0);
        let extra = tagged(Some("-6.02 dB"), Some("-5.10 dB"));
        let gain = control.effective_gain(&extra);
        assert!((gain - 0.5).abs() < 1e-3, "{gain}");
        assert_eq!(
            ReplayGainControl::default().effective_gain(&extra),
            1.0,
            "disabled means unadjusted"
        );
    }
}
