// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Where the presets come from: the configured folder, or the ones the player
//! brings along when none is configured.
//!
//! A configured folder is taken at its word: when it holds no preset, that is
//! reported, and the built-in presets do not quietly stand in for it.

use std::path::{Path, PathBuf};

use thiserror::Error;
use walkdir::WalkDir;

/// How deep the preset folder is searched. Preset packs nest a few folders; the
/// limit is here for a folder that was picked by mistake.
const MAX_DEPTH: usize = 16;

const BUILTIN_PREFIX: &str = "builtin:";

/// The presets the player brings along: Rustamp's own work, under its licence.
const BUILTIN: [(&str, &str); 3] = [
    ("Ember Tunnel", include_str!("../presets/ember-tunnel.milk")),
    ("Oxide Rings", include_str!("../presets/oxide-rings.milk")),
    ("Slow Current", include_str!("../presets/slow-current.milk")),
];

/// Why there is nothing to show.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PresetError {
    /// The configured folder cannot be read.
    #[error("the preset folder {} cannot be read: {reason}", folder.display())]
    Unreadable {
        /// The configured folder.
        folder: PathBuf,
        /// What the file system said.
        reason: String,
    },
    /// The configured folder holds no `.milk` or `.milk2` file.
    #[error("the preset folder {} holds no MilkDrop presets (.milk, .milk2)", folder.display())]
    Empty {
        /// The configured folder.
        folder: PathBuf,
    },
}

/// The presets to choose from, named by opaque strings the playlist can hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetSet {
    items: Vec<String>,
}

impl PresetSet {
    /// The presets of `folder` (searched recursively, sorted by path), or the
    /// built-in ones when no folder is configured.
    pub fn discover(folder: Option<&Path>) -> Result<Self, PresetError> {
        let Some(folder) = folder else {
            return Ok(Self {
                items: BUILTIN
                    .iter()
                    .map(|(name, _)| format!("{BUILTIN_PREFIX}{name}"))
                    .collect(),
            });
        };
        if let Err(error) = std::fs::read_dir(folder) {
            return Err(PresetError::Unreadable {
                folder: folder.to_path_buf(),
                reason: error.to_string(),
            });
        }
        let mut items: Vec<String> = WalkDir::new(folder)
            .max_depth(MAX_DEPTH)
            .into_iter()
            // A subfolder that cannot be read costs its presets, not the rest.
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file() && is_preset(entry.path()))
            .filter_map(|entry| entry.path().to_str().map(str::to_owned))
            .collect();
        items.sort();
        if items.is_empty() {
            return Err(PresetError::Empty {
                folder: folder.to_path_buf(),
            });
        }
        Ok(Self { items })
    }

    /// The presets, in order.
    pub fn items(&self) -> &[String] {
        &self.items
    }

    /// The bytes of the preset `item` (one of [`PresetSet::items`]).
    pub fn read(item: &str) -> std::io::Result<Vec<u8>> {
        match item.strip_prefix(BUILTIN_PREFIX) {
            Some(name) => BUILTIN
                .iter()
                .find(|(builtin, _)| *builtin == name)
                .map(|(_, text)| text.as_bytes().to_vec())
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound)),
            None => std::fs::read(item),
        }
    }

    /// What the title bar calls the preset `item`.
    pub fn display_name(item: &str) -> &str {
        match item.strip_prefix(BUILTIN_PREFIX) {
            Some(name) => name,
            None => Path::new(item)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(item),
        }
    }
}

fn is_preset(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("milk") || extension.eq_ignore_ascii_case("milk2")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_folder_the_built_in_presets_are_there_and_readable() {
        let set = PresetSet::discover(None).unwrap();
        assert_eq!(set.items().len(), BUILTIN.len());
        for item in set.items() {
            assert!(item.starts_with(BUILTIN_PREFIX));
            let bytes = PresetSet::read(item).unwrap();
            assert!(String::from_utf8(bytes).unwrap().starts_with("[preset00]"));
        }
        assert_eq!(
            PresetSet::display_name(&set.items()[0]),
            BUILTIN[0].0,
            "the title bar shows the name, not the marker"
        );
    }

    #[test]
    fn a_built_in_name_that_does_not_exist_is_an_error() {
        assert!(PresetSet::read("builtin:nothing").is_err());
    }

    #[test]
    fn the_folder_is_searched_recursively_sorted_and_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("pack").join("deep");
        std::fs::create_dir_all(&nested).unwrap();
        for file in [
            dir.path().join("b.milk"),
            dir.path().join("a.MILK"),
            nested.join("c.milk2"),
            dir.path().join("notes.txt"),
            dir.path().join("milk"),
        ] {
            std::fs::write(file, "[preset00]\n").unwrap();
        }
        let set = PresetSet::discover(Some(dir.path())).unwrap();
        let names: Vec<_> = set
            .items()
            .iter()
            .map(|item| PresetSet::display_name(item))
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
        assert_eq!(
            PresetSet::read(&set.items()[0]).unwrap(),
            b"[preset00]\n",
            "a preset of the folder is read from the folder"
        );
    }

    #[test]
    fn an_empty_folder_is_reported_and_built_ins_do_not_stand_in() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("readme.txt"), "x").unwrap();
        let error = PresetSet::discover(Some(dir.path())).unwrap_err();
        assert_eq!(
            error,
            PresetError::Empty {
                folder: dir.path().to_path_buf()
            }
        );
        assert!(error.to_string().contains("holds no MilkDrop presets"));
    }

    #[test]
    fn a_folder_that_does_not_exist_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone");
        let error = PresetSet::discover(Some(&missing)).unwrap_err();
        assert!(matches!(error, PresetError::Unreadable { .. }), "{error}");
        assert!(error.to_string().contains("gone"));
    }
}
