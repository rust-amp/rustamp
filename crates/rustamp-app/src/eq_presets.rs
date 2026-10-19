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

use std::path::PathBuf;

use rustamp_core::command::Command;
use rustamp_core::error::Result;
use rustamp_core::state::{EQ_BANDS, EqState};
use rustamp_platform::PopupRow;

use crate::app::App;

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

// ---------------------------------------------------------------------------
// User presets and the .eqf library file
// ---------------------------------------------------------------------------
//
// **Provenance.** The `.eqf` byte layout below (a 27-character header, a
// Ctrl-Z and `!--`, then fixed 257-byte NUL-padded names each followed by
// ten band bytes and a preamp byte, 63 = lowest gain) is the publicly
// documented library format, read from the MIT-licensed webamp project's
// `winamp-eqf` package. Only the layout facts were taken; the code is our
// own. The byte-to-dB scale is our own reading of "0..=63 across
// +-12 dB" and is not measured against the reference (S07 §12 O12). The
// submenu contents of the Presets popup are likewise our own choice.

/// The `.eqf` library header: text, Ctrl-Z, `!--`.
const EQF_HEADER: &[u8] = b"Winamp EQ library file v1.1\x1a!--";
/// The fixed width of a preset name field.
const EQF_NAME_LEN: usize = 257;
/// One preset record: the name, ten bands and the preamp.
const EQF_RECORD_LEN: usize = EQF_NAME_LEN + EQ_BANDS + 1;
/// The largest band byte (the lowest gain).
const EQF_MAX_BYTE: f32 = 63.0;
/// The file name of the user's preset library, beside the configuration.
pub const USER_PRESETS_FILE: &str = "presets.eqf";

/// A named curve the user saved, with its preamp.
#[derive(Debug, Clone, PartialEq)]
pub struct UserPreset {
    /// The name shown in the menus.
    pub name: String,
    /// Per-band gain in dB, lowest band first.
    pub bands: [f32; EQ_BANDS],
    /// Preamp gain in dB.
    pub preamp: f32,
}

/// Why a `.eqf` file could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotAnEqf;

impl std::fmt::Display for NotAnEqf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("not a Winamp EQ library file")
    }
}

impl std::error::Error for NotAnEqf {}

fn gain_to_byte(db: f32) -> u8 {
    let db = db.clamp(-EqState::MAX_GAIN_DB, EqState::MAX_GAIN_DB);
    ((EqState::MAX_GAIN_DB - db) * EQF_MAX_BYTE / (2.0 * EqState::MAX_GAIN_DB)).round() as u8
}

fn byte_to_gain(byte: u8) -> f32 {
    let byte = f32::from(byte).min(EQF_MAX_BYTE);
    let db = EqState::MAX_GAIN_DB - byte * (2.0 * EqState::MAX_GAIN_DB) / EQF_MAX_BYTE;
    // The two bytes next to the middle are 0.19 dB off; call them flat.
    if db.abs() < 0.2 { 0.0 } else { db }
}

/// Reads every preset of a `.eqf` library. A trailing partial record is
/// ignored.
pub fn parse_eqf(bytes: &[u8]) -> Result<Vec<UserPreset>, NotAnEqf> {
    let body = bytes.strip_prefix(EQF_HEADER).ok_or(NotAnEqf)?;
    Ok(body
        .as_chunks::<EQF_RECORD_LEN>()
        .0
        .iter()
        .map(|record| {
            let name_field = &record[..EQF_NAME_LEN];
            let name_end = name_field
                .iter()
                .position(|b| *b == 0)
                .unwrap_or(EQF_NAME_LEN);
            // Names are single-byte (Latin-1) text.
            let name = name_field[..name_end]
                .iter()
                .map(|b| char::from(*b))
                .collect();
            let values = &record[EQF_NAME_LEN..];
            let mut bands = [0.0; EQ_BANDS];
            for (band, byte) in bands.iter_mut().zip(values) {
                *band = byte_to_gain(*byte);
            }
            UserPreset {
                name,
                bands,
                preamp: byte_to_gain(values[EQ_BANDS]),
            }
        })
        .collect())
}

/// Writes `presets` as a `.eqf` library. Names are cut to fit the field and
/// characters outside Latin-1 become `?`.
pub fn write_eqf(presets: &[UserPreset]) -> Vec<u8> {
    let mut out = Vec::with_capacity(EQF_HEADER.len() + presets.len() * EQF_RECORD_LEN);
    out.extend_from_slice(EQF_HEADER);
    for preset in presets {
        let mut name: Vec<u8> = preset
            .name
            .chars()
            .map(|ch| {
                u8::try_from(u32::from(ch))
                    .ok()
                    .filter(|b| *b != 0)
                    .unwrap_or(b'?')
            })
            .take(EQF_NAME_LEN - 1)
            .collect();
        name.resize(EQF_NAME_LEN, 0);
        out.extend_from_slice(&name);
        out.extend(preset.bands.iter().map(|db| gain_to_byte(*db)));
        out.push(gain_to_byte(preset.preamp));
    }
    out
}

/// What one row of the Presets popup does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqMenuChoice {
    /// Apply the built-in preset at this index of [`presets`].
    LoadBuiltin(usize),
    /// Apply the saved preset at this index.
    LoadUser(usize),
    /// Ask for a `.eqf` file and apply its first preset.
    LoadFromFile,
    /// Save the current curve under a fresh name.
    SaveNew,
    /// Save the current curve over the saved preset at this index.
    SaveOver(usize),
    /// Delete the saved preset at this index.
    Delete(usize),
    /// A greyed placeholder row; choosing it does nothing.
    Nothing,
}

/// The Presets popup as a row tree plus what each id means.
#[derive(Debug, Clone, PartialEq)]
pub struct EqPresetsMenu {
    /// The rows, top-level Load / Save / Delete (S07 §4.2) with submenus.
    pub rows: Vec<PopupRow>,
    choices: Vec<EqMenuChoice>,
}

impl EqPresetsMenu {
    /// What the chosen `id` means (ids are 1-based; `0` is "dismissed").
    pub fn choice_for_id(&self, id: u32) -> Option<EqMenuChoice> {
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        self.choices.get(index).copied()
    }
}

struct MenuBuilder {
    choices: Vec<EqMenuChoice>,
}

impl MenuBuilder {
    fn entry(&mut self, label: &str, choice: EqMenuChoice) -> PopupRow {
        self.choices.push(choice);
        let id = u32::try_from(self.choices.len()).unwrap_or(u32::MAX);
        PopupRow::Entry {
            id,
            label: label.to_string(),
            accelerator: None,
            checked: false,
            enabled: choice != EqMenuChoice::Nothing,
        }
    }
}

/// Builds the Presets popup. `current` marks the built-in preset the live
/// gains match, `saved` are the user's presets.
pub fn presets_menu(saved: &[UserPreset], current: Option<&str>) -> EqPresetsMenu {
    let mut b = MenuBuilder {
        choices: Vec::new(),
    };

    let mut load = Vec::new();
    for (index, preset) in presets().iter().enumerate() {
        let mut row = b.entry(preset.name, EqMenuChoice::LoadBuiltin(index));
        if let PopupRow::Entry { checked, .. } = &mut row {
            *checked = current == Some(preset.name);
        }
        load.push(row);
    }
    if !saved.is_empty() {
        load.push(PopupRow::Separator);
        for (index, preset) in saved.iter().enumerate() {
            load.push(b.entry(&preset.name, EqMenuChoice::LoadUser(index)));
        }
    }
    load.push(PopupRow::Separator);
    load.push(b.entry("Load from file...", EqMenuChoice::LoadFromFile));

    let mut save = vec![b.entry("New preset", EqMenuChoice::SaveNew)];
    if !saved.is_empty() {
        save.push(PopupRow::Separator);
        for (index, preset) in saved.iter().enumerate() {
            save.push(b.entry(&preset.name, EqMenuChoice::SaveOver(index)));
        }
    }

    let mut delete = Vec::new();
    for (index, preset) in saved.iter().enumerate() {
        delete.push(b.entry(&preset.name, EqMenuChoice::Delete(index)));
    }
    if delete.is_empty() {
        delete.push(b.entry("(no saved presets)", EqMenuChoice::Nothing));
    }

    EqPresetsMenu {
        rows: vec![
            PopupRow::Submenu {
                label: "Load".to_string(),
                rows: load,
            },
            PopupRow::Submenu {
                label: "Save".to_string(),
                rows: save,
            },
            PopupRow::Submenu {
                label: "Delete".to_string(),
                rows: delete,
            },
        ],
        choices: b.choices,
    }
}

/// A name for a new saved preset that no built-in or saved preset uses.
pub fn fresh_name(saved: &[UserPreset]) -> String {
    (1..)
        .map(|n| format!("Custom {n}"))
        .find(|name| {
            find_preset(name).is_none() && !saved.iter().any(|p| p.name.eq_ignore_ascii_case(name))
        })
        .expect("an unbounded range has a free name")
}

impl App {
    /// The user's saved presets file, beside the configuration.
    fn user_presets_path(&self) -> Option<PathBuf> {
        Some(self.config_path()?.parent()?.join(USER_PRESETS_FILE))
    }

    /// Reads the saved presets from disk (called when the config path is
    /// set). A missing file is an empty library; an unreadable one is noted.
    pub(crate) fn load_eq_user_presets(&mut self) {
        self.eq_user_presets.clear();
        let Some(path) = self.user_presets_path() else {
            return;
        };
        match std::fs::read(&path) {
            Ok(bytes) => match parse_eqf(&bytes) {
                Ok(presets) => self.eq_user_presets = presets,
                Err(error) => self.note(format!("{}: {error}", path.display())),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => self.note(format!("{}: {error}", path.display())),
        }
    }

    fn save_eq_user_presets(&mut self) {
        let Some(path) = self.user_presets_path() else {
            return; // No config path: the presets live for this run only.
        };
        let result = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&path, write_eqf(&self.eq_user_presets)));
        if let Err(error) = result {
            self.note(format!("could not save {}: {error}", path.display()));
        }
    }

    /// The user's saved equalizer presets.
    pub fn eq_user_presets(&self) -> &[UserPreset] {
        &self.eq_user_presets
    }

    /// Takes the pending Presets popup request, if any.
    pub fn take_eq_menu(&mut self) -> bool {
        std::mem::take(&mut self.pending_eq_menu)
    }

    /// The Presets popup for the live state.
    pub fn native_eq_menu(&self) -> EqPresetsMenu {
        presets_menu(&self.eq_user_presets, self.current_eq_preset())
    }

    /// Runs what one chosen Presets popup row means.
    pub fn activate_eq_menu_choice(&mut self, choice: EqMenuChoice) -> Result<()> {
        match choice {
            EqMenuChoice::Nothing => {}
            EqMenuChoice::LoadBuiltin(index) => {
                if let Some(preset) = presets().get(index) {
                    self.apply_eq_preset(preset.name)?;
                }
            }
            EqMenuChoice::LoadUser(index) => {
                if let Some(preset) = self.eq_user_presets.get(index).cloned() {
                    self.apply_user_preset(&preset)?;
                }
            }
            // The presenter owns the file dialog and calls `load_eq_file`.
            EqMenuChoice::LoadFromFile => {}
            EqMenuChoice::SaveNew => {
                let name = fresh_name(&self.eq_user_presets);
                self.save_eq_preset(name, None);
            }
            EqMenuChoice::SaveOver(index) => {
                if let Some(name) = self.eq_user_presets.get(index).map(|p| p.name.clone()) {
                    self.save_eq_preset(name, Some(index));
                }
            }
            EqMenuChoice::Delete(index) => {
                if index < self.eq_user_presets.len() {
                    self.eq_user_presets.remove(index);
                    self.save_eq_user_presets();
                }
            }
        }
        Ok(())
    }

    /// Applies a saved preset: its ten bands and its preamp. (Built-in
    /// presets leave the preamp alone; S07 §12 O12 leaves this open, so a
    /// saved preset restoring the preamp it was saved with is our choice.)
    fn apply_user_preset(&mut self, preset: &UserPreset) -> Result<()> {
        for (band, &gain_db) in preset.bands.iter().enumerate() {
            self.submit(Command::SetBand {
                band: band as u8,
                gain_db,
            })?;
        }
        self.submit(Command::SetPreamp(preset.preamp))
    }

    /// Loads the first preset of the `.eqf` file at `path` and applies it.
    /// Returns the preset's name. The library of saved presets is untouched.
    pub fn load_eq_file(&mut self, path: &std::path::Path) -> Result<String, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let presets = parse_eqf(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let preset = presets
            .into_iter()
            .next()
            .ok_or_else(|| format!("{}: the file holds no presets", path.display()))?;
        self.apply_user_preset(&preset)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(preset.name)
    }

    /// Stores the engine's current curve as `name`, replacing the preset at
    /// `over` or appending.
    fn save_eq_preset(&mut self, name: String, over: Option<usize>) {
        let eq = self.engine().eq();
        let preset = UserPreset {
            name,
            bands: eq.bands,
            preamp: eq.preamp,
        };
        match over.and_then(|index| self.eq_user_presets.get_mut(index)) {
            Some(slot) => *slot = preset,
            None => self.eq_user_presets.push(preset),
        }
        self.save_eq_user_presets();
    }
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
    fn eqf_round_trips_within_the_byte_resolution() {
        let saved = vec![
            UserPreset {
                name: "Warm".to_string(),
                bands: [6.0, 4.0, 2.0, 0.0, -2.0, -4.0, -6.0, -12.0, 12.0, 0.0],
                preamp: -3.0,
            },
            UserPreset {
                name: "Caf\u{e9}".to_string(),
                bands: [0.0; EQ_BANDS],
                preamp: 0.0,
            },
        ];
        let bytes = write_eqf(&saved);
        assert!(bytes.starts_with(b"Winamp EQ library file v1.1\x1a!--"));
        assert_eq!(bytes.len(), 31 + 2 * 268);
        let back = parse_eqf(&bytes).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[1], saved[1]);
        assert_eq!(back[0].name, "Warm");
        for (want, have) in saved[0].bands.iter().zip(back[0].bands) {
            assert!((want - have).abs() <= 0.2, "{want} vs {have}");
        }
        assert!((back[0].preamp + 3.0).abs() <= 0.2);
        // The extremes stay the extremes.
        assert_eq!(back[0].bands[7], -12.0);
        assert_eq!(back[0].bands[8], 12.0);
    }

    #[test]
    fn eqf_rejects_other_files_and_ignores_a_trailing_partial_record() {
        assert_eq!(parse_eqf(b"hello"), Err(NotAnEqf));
        assert_eq!(parse_eqf(b""), Err(NotAnEqf));
        let mut bytes = write_eqf(&[]);
        assert_eq!(parse_eqf(&bytes), Ok(Vec::new()));
        bytes.extend_from_slice(&[1; 100]);
        assert_eq!(parse_eqf(&bytes), Ok(Vec::new()));
    }

    #[test]
    fn the_popup_has_load_save_delete_with_the_builtins_under_load() {
        let saved = vec![UserPreset {
            name: "Mine".to_string(),
            bands: [1.0; EQ_BANDS],
            preamp: 0.0,
        }];
        let menu = presets_menu(&saved, Some("Rock"));
        let labels: Vec<_> = menu
            .rows
            .iter()
            .map(|row| match row {
                PopupRow::Submenu { label, .. } => label.as_str(),
                _ => "?",
            })
            .collect();
        assert_eq!(labels, ["Load", "Save", "Delete"]);
        let PopupRow::Submenu { rows: load, .. } = &menu.rows[0] else {
            unreachable!()
        };
        // Nine built-ins, a separator, one saved preset, a separator and
        // "Load from file..."; Rock is ticked.
        assert_eq!(load.len(), 13);
        assert!(matches!(
            load.last(),
            Some(PopupRow::Entry { label, .. }) if label == "Load from file..."
        ));
        assert!(load.iter().any(|row| matches!(
            row,
            PopupRow::Entry { label, checked: true, .. } if label == "Rock"
        )));
        assert_eq!(menu.choice_for_id(1), Some(EqMenuChoice::LoadBuiltin(0)));
        // With nothing saved, Delete offers one greyed placeholder.
        let empty = presets_menu(&[], None);
        let PopupRow::Submenu { rows: delete, .. } = &empty.rows[2] else {
            unreachable!()
        };
        assert!(matches!(
            delete.as_slice(),
            [PopupRow::Entry { enabled: false, .. }]
        ));
    }

    #[test]
    fn fresh_names_skip_the_taken_ones() {
        let taken = |name: &str| UserPreset {
            name: name.to_string(),
            bands: [0.0; EQ_BANDS],
            preamp: 0.0,
        };
        assert_eq!(fresh_name(&[]), "Custom 1");
        assert_eq!(
            fresh_name(&[taken("custom 1"), taken("Custom 2")]),
            "Custom 3"
        );
    }

    #[test]
    fn unmatched_gains_have_no_name() {
        let mut state = EqState::default();
        state.bands[0] = 11.0;
        assert_eq!(preset_name_for(&state), None);
    }
}
