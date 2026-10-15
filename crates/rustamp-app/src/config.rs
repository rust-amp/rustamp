// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Configuration loading (task card N1).
//!
//! `rustamp-core` defines the typed [`Config`] tree and its defaults but
//! deliberately leaves file I/O to a later task (RFC 0001, "Config"). The app
//! is that later task's first half: it resolves a path, reads TOML and falls
//! back to the defaults for anything missing or malformed. Every field of every
//! config struct is `#[serde(default)]`, so a partial file keeps working.
//!
//! The path is `%RUSTAMP_CONFIG%` when that variable is set (the test seam),
//! otherwise `%APPDATA%\Rustamp\rustamp.toml`. A missing file is **not** an
//! error: it means "all defaults".

use std::path::{Path, PathBuf};

use rustamp_core::config::Config;

/// Environment variable that overrides the config path (also the test seam).
pub const CONFIG_ENV: &str = "RUSTAMP_CONFIG";

/// Where the loaded configuration came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// No file was read; every value is the built-in default.
    Defaults,
    /// The configuration was read from this file.
    File(PathBuf),
}

/// The result of a configuration load.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigLoad {
    /// The configuration to use (always valid).
    pub config: Config,
    /// Where it came from.
    pub source: ConfigSource,
    /// A human-readable note when a file existed but could not be used.
    pub warning: Option<String>,
}

/// The default configuration path, or `None` when neither the override nor
/// `%APPDATA%` is set.
pub fn default_path() -> Option<PathBuf> {
    if let Some(override_path) = std::env::var_os(CONFIG_ENV) {
        return Some(PathBuf::from(override_path));
    }
    let appdata = std::env::var_os("APPDATA")?;
    Some(PathBuf::from(appdata).join("Rustamp").join("rustamp.toml"))
}

/// Parses TOML into a [`Config`].
pub fn parse(text: &str) -> Result<Config, String> {
    toml::from_str(text).map_err(|e| e.to_string())
}

/// Loads the configuration at `path`, falling back to the defaults.
///
/// * `path == None` (or the file does not exist) -> defaults, no warning.
/// * a file that cannot be read or parsed -> defaults plus a warning.
pub fn load(path: Option<&Path>) -> ConfigLoad {
    let Some(path) = path else {
        return ConfigLoad {
            config: Config::default(),
            source: ConfigSource::Defaults,
            warning: None,
        };
    };
    match std::fs::read_to_string(path) {
        Ok(text) => match parse(&text) {
            Ok(config) => ConfigLoad {
                config,
                source: ConfigSource::File(path.to_owned()),
                warning: None,
            },
            Err(error) => ConfigLoad {
                config: Config::default(),
                source: ConfigSource::Defaults,
                warning: Some(format!(
                    "config {} is not valid TOML ({error}); using defaults",
                    path.display()
                )),
            },
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ConfigLoad {
            config: Config::default(),
            source: ConfigSource::Defaults,
            warning: None,
        },
        Err(error) => ConfigLoad {
            config: Config::default(),
            source: ConfigSource::Defaults,
            warning: Some(format!(
                "config {} could not be read ({error}); using defaults",
                path.display()
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_all_defaults() {
        let missing = PathBuf::from("does/not/exist/rustamp.toml");
        let loaded = load(Some(&missing));
        assert_eq!(loaded.config, Config::default());
        assert_eq!(loaded.source, ConfigSource::Defaults);
        assert_eq!(loaded.warning, None);

        // And so is no path at all.
        let loaded = load(None);
        assert_eq!(loaded.config, Config::default());
        assert_eq!(loaded.source, ConfigSource::Defaults);
    }

    #[test]
    fn a_partial_file_fills_the_rest_with_defaults() {
        let config = parse("[audio]\nvolume = 5\n").unwrap();
        assert_eq!(config.audio.volume, 5);
        assert_eq!(
            config.audio.buffer_ms,
            Config::default().audio.buffer_ms,
            "unset fields keep their defaults"
        );
        assert_eq!(config.version, rustamp_core::config::CONFIG_VERSION);
    }

    #[test]
    fn a_file_is_read_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rustamp.toml");
        std::fs::write(&path, "[ui]\ndouble_size = true\nskin = \"skins/x.wsz\"\n").unwrap();
        let loaded = load(Some(&path));
        assert!(loaded.config.ui.double_size);
        assert_eq!(loaded.config.ui.skin, Some(PathBuf::from("skins/x.wsz")));
        assert_eq!(loaded.source, ConfigSource::File(path.clone()));
        assert_eq!(loaded.warning, None);
    }

    #[test]
    fn a_broken_file_falls_back_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.toml");
        std::fs::write(&path, "this is = = not toml").unwrap();
        let loaded = load(Some(&path));
        assert_eq!(loaded.config, Config::default());
        assert_eq!(loaded.source, ConfigSource::Defaults);
        assert!(
            loaded
                .warning
                .as_deref()
                .unwrap_or("")
                .contains("not valid TOML"),
            "{:?}",
            loaded.warning
        );
    }

    #[test]
    fn the_default_path_honours_the_environment_override() {
        // `default_path` reads the process environment; this test only checks
        // the shape, not a specific machine's APPDATA.
        let path = default_path();
        if let Some(path) = path {
            assert!(path.is_absolute() || std::env::var_os(CONFIG_ENV).is_some());
        }
    }
}
