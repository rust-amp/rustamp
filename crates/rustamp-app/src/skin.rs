// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Skin selection and loading (task card N1).
//!
//! The app resolves the skin in this order: `--skin <path>`, then the config's
//! `ui.skin`, then the built-in default skin. The built-in skin is the
//! repo-owned look-alike default skin (`rustamp_skin::BuiltinDefaultSkin`,
//! task card J1).
//!
//! A path that cannot be loaded is **not** fatal: the app falls back to the
//! built-in skin and carries the reason as a warning, so a bad `--skin` never
//! stops the player.

use std::path::{Path, PathBuf};

use rustamp_skin::{BuiltinDefaultSkin, BuiltinLookalikeSkin, Skin};

/// Which skin to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkinChoice {
    /// The repo's built-in default (Rust-themed) skin.
    Builtin,
    /// The classic look-alike skin.
    Lookalike,
    /// A folder, `.wsz` or `.zip`.
    Path(PathBuf),
}

/// Resolves the skin choice: `--skin` beats the config, the config beats the
/// built-in default.
pub fn choose(cli: Option<&Path>, config: Option<&Path>) -> SkinChoice {
    match (cli, config) {
        (Some(path), _) => resolve_choice(path),
        (None, Some(path)) => resolve_choice(path),
        (None, None) => SkinChoice::Builtin,
    }
}

fn resolve_choice(path: &Path) -> SkinChoice {
    if let Some(s) = path.to_str()
        && (s.eq_ignore_ascii_case("lookalike") || s.eq_ignore_ascii_case("builtin:lookalike"))
    {
        SkinChoice::Lookalike
    } else {
        SkinChoice::Path(path.to_owned())
    }
}

/// A loaded skin plus what the user should be told about it.
pub struct LoadedSkin {
    skin: Skin,
    description: String,
    path: Option<PathBuf>,
    fell_back: bool,
    warning: Option<String>,
}

impl LoadedSkin {
    /// The skin itself.
    pub fn skin(&self) -> &Skin {
        &self.skin
    }

    /// A one-line description of where the skin came from.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The path the skin was loaded from, if it came from disk (not the
    /// built-in default and not a fallback).
    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    /// Whether the requested skin failed and the built-in default was used.
    pub fn fell_back(&self) -> bool {
        self.fell_back
    }

    /// The reason for the fallback, if any.
    pub fn warning(&self) -> Option<&str> {
        self.warning.as_deref()
    }
}

/// Loads the built-in default (Rust-themed) skin.
///
/// Pre-populates all 32 classic entries plus `nums_ex.bmp` from the embedded
/// Rust theme.
pub fn builtin() -> LoadedSkin {
    let mut source = BuiltinDefaultSkin::source();
    let skin = Skin::from_source(&mut source, &BuiltinDefaultSkin)
        .expect("the built-in default skin always loads");
    LoadedSkin {
        skin,
        description: "built-in default skin".to_string(),
        path: None,
        fell_back: false,
        warning: None,
    }
}

/// Loads the classic look-alike skin.
pub fn lookalike() -> LoadedSkin {
    let mut source = BuiltinLookalikeSkin::source();
    let skin = Skin::from_source(&mut source, &BuiltinLookalikeSkin)
        .expect("the lookalike skin always loads");
    LoadedSkin {
        skin,
        description: "built-in lookalike skin".to_string(),
        path: None,
        fell_back: false,
        warning: None,
    }
}

/// Loads the chosen skin, falling back to the built-in default on any error.
pub fn load(choice: &SkinChoice) -> LoadedSkin {
    match choice {
        SkinChoice::Builtin => builtin(),
        SkinChoice::Lookalike => lookalike(),
        SkinChoice::Path(path) => match Skin::load(path) {
            Ok(skin) => LoadedSkin {
                skin,
                description: format!("skin {}", path.display()),
                path: Some(path.clone()),
                fell_back: false,
                warning: None,
            },
            Err(error) => {
                let mut loaded = builtin();
                loaded.fell_back = true;
                loaded.warning = Some(format!(
                    "could not load skin {} ({error}); using the built-in default skin",
                    path.display()
                ));
                loaded
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cli_beats_the_config_and_the_config_beats_the_default() {
        let cli = PathBuf::from("cli.wsz");
        let config = PathBuf::from("config.wsz");
        assert_eq!(
            choose(Some(&cli), Some(&config)),
            SkinChoice::Path(cli.clone())
        );
        assert_eq!(
            choose(None, Some(&config)),
            SkinChoice::Path(config.clone())
        );
        assert_eq!(choose(None, None), SkinChoice::Builtin);
    }

    #[test]
    fn the_builtin_skin_loads_and_renders_the_main_window() {
        let loaded = builtin();
        assert!(!loaded.fell_back());
        assert_eq!(loaded.warning(), None);
        assert_eq!(loaded.description(), "built-in default skin");
        assert!(
            loaded.skin().has_main_window_sprites(),
            "the built-in skin must carry the main-window sheets"
        );
    }

    #[test]
    fn a_bad_path_falls_back_to_the_builtin_skin() {
        let loaded = load(&SkinChoice::Path(PathBuf::from("no/such/skin.wsz")));
        assert!(loaded.fell_back());
        assert!(
            loaded
                .warning()
                .unwrap_or("")
                .contains("built-in default skin"),
            "{:?}",
            loaded.warning()
        );
        assert!(loaded.skin().has_main_window_sprites());
    }

    #[test]
    fn a_folder_skin_loads_from_disk() {
        // An empty folder is a valid (if sparse) skin: `rustamp-skin` fills
        // every missing sheet from the built-in fallback (S17), so this must
        // not fall back wholesale.
        let dir = tempfile::tempdir().unwrap();
        let loaded = load(&SkinChoice::Path(dir.path().to_owned()));
        assert!(!loaded.fell_back(), "{:?}", loaded.warning());
        assert!(loaded.skin().has_main_window_sprites());
        assert!(
            loaded.description().contains(dir.path().to_str().unwrap()),
            "{}",
            loaded.description()
        );
    }

    #[test]
    fn selecting_lookalike_routes_to_lookalike_skin() {
        let choice = choose(Some(Path::new("lookalike")), None);
        assert_eq!(choice, SkinChoice::Lookalike);
        let loaded = load(&choice);
        assert!(!loaded.fell_back());
        assert_eq!(loaded.description(), "built-in lookalike skin");
        assert!(loaded.skin().has_main_window_sprites());
    }
}
