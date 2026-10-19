//! Built-in equalizer presets (the "Load" submenu's content).
//!
//! **Provenance: own design.** The values below are our own curves, shaped
//! after the plain-language descriptions in the task brief (for example "Rock:
//! bass up, mids down, treble up"). They are not copied from Winamp, from any
//! SDK or from another player's preset file, so nothing here needs a
//! `THIRD_PARTY.md` entry. The band order is the equalizer's own (lowest to
//! highest centre, `rustamp_core::state::EQ_BANDS` bands), the unit is dB, and
//! every gain sits well inside `±EqState::MAX_GAIN_DB`.
//!
//! **Persistence.** A preset is applied as ordinary `SetBand` commands, and the
//! engine's resulting band gains are what the configuration already saves
//! (`eq.state`). No separate "selected preset" field is stored, which would be
//! a `rustamp-core` contract change; instead [`preset_name_for`] recovers the
//! name from the saved gains, so the selection survives a restart for free.

use rustamp_core::state::{EQ_BANDS, EqState};

/// One named equalizer curve.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EqPreset {
    /// The name shown in the Load submenu.
    pub name: &'static str,
    /// Per-band gain in dB, lowest band first.
    pub bands: [f32; EQ_BANDS],
}

/// The built-in presets, in menu order. `Flat` comes first.
pub const BUILTIN_PRESETS: [EqPreset; 9] = [
    EqPreset {
        name: "Flat",
        bands: [0.0; EQ_BANDS],
    },
    EqPreset {
        name: "Rock",
        bands: [4.0, 3.0, 1.5, -1.0, -2.0, -1.0, 1.5, 3.0, 3.0, 3.0],
    },
    EqPreset {
        name: "Pop",
        bands: [2.0, 3.0, 2.0, 1.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0],
    },
    EqPreset {
        name: "Jazz",
        bands: [0.0, 0.0, 1.0, 2.0, 2.0, 2.0, 1.0, 1.0, 1.0, 1.0],
    },
    EqPreset {
        name: "Classical",
        bands: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0],
    },
    EqPreset {
        name: "Vocal",
        bands: [-2.0, -2.0, -1.0, 1.0, 3.0, 3.0, 2.0, 1.0, 1.0, 1.0],
    },
    EqPreset {
        name: "Dance",
        bands: [4.0, 4.0, 2.0, -1.0, -2.0, -2.0, 0.0, 3.0, 3.0, 3.0],
    },
    EqPreset {
        name: "Full Bass",
        bands: [6.0, 6.0, 6.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    },
    EqPreset {
        name: "Full Treble",
        bands: [0.0, 0.0, 0.0, 0.0, 0.0, 3.0, 6.0, 6.0, 6.0, 6.0],
    },
];

/// Two gains closer than this are the same preset value (dB). The classic
/// slider scale quantises to roughly 0.4 dB steps, so a preset that went
/// through the slider round trip must still match.
const MATCH_TOLERANCE_DB: f32 = 0.5;

/// The built-in presets in menu order.
pub fn presets() -> &'static [EqPreset] {
    &BUILTIN_PRESETS
}

/// Finds a preset by name, ignoring ASCII case.
pub fn find_preset(name: &str) -> Option<&'static EqPreset> {
    BUILTIN_PRESETS
        .iter()
        .find(|preset| preset.name.eq_ignore_ascii_case(name))
}

/// The name of the preset whose gains match `state`'s bands, if any.
///
/// This is how the selection is persisted: the saved gains identify the preset.
pub fn preset_name_for(state: &EqState) -> Option<&'static str> {
    BUILTIN_PRESETS
        .iter()
        .find(|preset| {
            preset
                .bands
                .iter()
                .zip(state.bands.iter())
                .all(|(want, have)| (want - have).abs() < MATCH_TOLERANCE_DB)
        })
        .map(|preset| preset.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_gains_are_in_range() {
        for (i, preset) in BUILTIN_PRESETS.iter().enumerate() {
            assert!(
                BUILTIN_PRESETS[i + 1..]
                    .iter()
                    .all(|other| other.name != preset.name),
                "duplicate preset {}",
                preset.name
            );
            assert!(
                preset.bands.iter().all(|g| g.abs() <= EqState::MAX_GAIN_DB),
                "{} exceeds the gain range",
                preset.name
            );
        }
    }

    #[test]
    fn flat_is_first_and_all_zero() {
        assert_eq!(presets()[0].name, "Flat");
        assert!(presets()[0].bands.iter().all(|g| *g == 0.0));
    }

    #[test]
    fn lookup_ignores_case() {
        assert_eq!(find_preset("rock").map(|p| p.name), Some("Rock"));
        assert!(find_preset("nope").is_none());
    }

    #[test]
    fn every_preset_is_recognised_from_its_own_gains() {
        for preset in &BUILTIN_PRESETS {
            let state = EqState {
                bands: preset.bands,
                ..EqState::default()
            };
            assert_eq!(preset_name_for(&state), Some(preset.name));
        }
    }

    #[test]
    fn unmatched_gains_have_no_name() {
        let mut state = EqState::default();
        state.bands[0] = 11.0;
        assert_eq!(preset_name_for(&state), None);
    }
}
