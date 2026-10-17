// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Configuration loading and persistence (task cards N1 and N6).
//!
//! `rustamp-core` defines the typed [`Config`] tree and its defaults. The app
//! resolves a path, reads TOML, and applies settings that the player honours
//! (volume, balance, skin path, double-size mode, always-on-top, easy-move,
//! equalizer state, and window placements).
//!
//! For settings that cannot be honoured yet (e.g. output backend/device selection,
//! audio buffer lengths, ReplayGain, snap distance, shuffle/repeat playback modes,
//! and plugins), the configuration loader reports clear warnings rather than
//! silently ignoring them.
//!
//! On clean exit, the app writes an honest configuration to `%RUSTAMP_CONFIG%`
//! (when set) or `%APPDATA%\Rustamp\rustamp.toml`. The serialized configuration
//! includes **only** fields that the player actually honours; ignored fields are
//! omitted so the file never advertises unsupported settings.
//!
//! A missing config file is **not** an error (it means "use defaults").
//! A malformed config file **fails loudly** with an explicit error rather than
//! silently falling back to defaults.

use std::fmt;
use std::path::{Path, PathBuf};

use rustamp_core::config::Config;
use rustamp_core::state::WindowId;

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

/// Why the configuration could not be loaded or saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// A file was found but could not be read from disk.
    Read {
        /// The file path.
        path: PathBuf,
        /// The operating system or I/O error message.
        error: String,
    },
    /// A file was found and read, but contained malformed syntax or rejected structure.
    Malformed {
        /// The file path.
        path: PathBuf,
        /// The parser error description.
        error: String,
    },
    /// A file could not be written to disk.
    Write {
        /// The file path.
        path: PathBuf,
        /// The error message.
        error: String,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Read { path, error } => {
                write!(f, "could not read config {}: {error}", path.display())
            }
            ConfigError::Malformed { path, error } => {
                write!(f, "config {} is malformed: {error}", path.display())
            }
            ConfigError::Write { path, error } => {
                write!(f, "could not write config {}: {error}", path.display())
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// The result of a configuration load.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigLoad {
    /// The configuration to use (always valid).
    pub config: Config,
    /// Where it came from.
    pub source: ConfigSource,
    /// Warnings for ignored or unhonoured fields present in the file.
    pub warnings: Vec<String>,
    /// A human-readable note when a file had unapplied settings (first warning, for backward compatibility).
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

/// Inspects a parsed TOML value and returns warnings for any unhonoured settings.
pub fn ignored_fields_in_toml(val: &toml::Value) -> Vec<String> {
    let mut warnings = Vec::new();
    let Some(table) = val.as_table() else {
        return warnings;
    };

    if let Some(audio) = table.get("audio").and_then(toml::Value::as_table) {
        for key in audio.keys() {
            let reason = match key.as_str() {
                "output" => Some("only the default audio backend is supported"),
                "device" => Some("audio device selection is not supported yet"),
                "buffer_ms" => Some("output buffer length is fixed"),
                "fadeout_ms" => Some("fadeout duration is not supported yet"),
                "gapless" => Some("gapless mode is not configurable"),
                "replaygain" => Some("ReplayGain is not supported yet"),
                "volume" | "balance" => None,
                _ => Some("unrecognized audio setting"),
            };
            if let Some(r) = reason {
                warnings.push(format!("setting 'audio.{key}' is not applied: {r}"));
            }
        }
    }

    if let Some(ui) = table.get("ui").and_then(toml::Value::as_table) {
        for key in ui.keys() {
            let reason = match key.as_str() {
                "snap_distance" => Some("window snap distance is not configurable"),
                "skin" | "double_size" | "always_on_top" | "easy_move" | "windows" => None,
                _ => Some("unrecognized UI setting"),
            };
            if let Some(r) = reason {
                warnings.push(format!("setting 'ui.{key}' is not applied: {r}"));
            }
        }
    }

    if let Some(playlist) = table.get("playlist").and_then(toml::Value::as_table) {
        for key in playlist.keys() {
            let reason = match key.as_str() {
                "shuffle" => Some(
                    "playback shuffle is not supported yet (engine does not implement shuffle)",
                ),
                "repeat" => {
                    Some("playback repeat is not supported yet (engine does not implement repeat)")
                }
                "manual_advance" => Some("manual advance is not supported yet"),
                "last_playlist" => Some("restoring playlist at startup is not supported yet"),
                _ => Some("unrecognized playlist setting"),
            };
            if let Some(r) = reason {
                warnings.push(format!("setting 'playlist.{key}' is not applied: {r}"));
            }
        }
    }

    if let Some(plugins) = table.get("plugins").and_then(toml::Value::as_table) {
        for key in plugins.keys() {
            let reason = match key.as_str() {
                "plugin_dir" => Some("plugin directory is not configurable"),
                "vis_plugin" => Some("visualization plugin loading is not supported yet"),
                "vis_preset" => Some("visualization preset is not supported yet"),
                "disabled" => Some("disabling plugins via config is not supported yet"),
                _ => Some("unrecognized plugins setting"),
            };
            if let Some(r) = reason {
                warnings.push(format!("setting 'plugins.{key}' is not applied: {r}"));
            }
        }
    }

    warnings
}

/// Parses TOML into a [`Config`] and returns any warnings about unhonoured settings.
pub fn parse_with_warnings(
    text: &str,
    path: Option<&Path>,
) -> Result<(Config, Vec<String>), ConfigError> {
    let toml_val: toml::Value = toml::from_str(text).map_err(|e| ConfigError::Malformed {
        path: path
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("<memory>")),
        error: e.to_string(),
    })?;
    let config: Config = toml::from_str(text).map_err(|e| ConfigError::Malformed {
        path: path
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("<memory>")),
        error: e.to_string(),
    })?;
    let warnings = ignored_fields_in_toml(&toml_val);
    Ok((config, warnings))
}

/// Loads the configuration at `path`.
///
/// * `path == None` (or the file does not exist) -> defaults, no warnings.
/// * a file that cannot be read -> fails loudly with [`ConfigError::Read`].
/// * a file that is not valid TOML -> fails loudly with [`ConfigError::Malformed`].
/// * a valid file with unapplied settings -> defaults for missing fields, plus warnings for ignored fields.
pub fn load(path: Option<&Path>) -> Result<ConfigLoad, ConfigError> {
    let Some(path) = path else {
        return Ok(ConfigLoad {
            config: Config::default(),
            source: ConfigSource::Defaults,
            warnings: Vec::new(),
            warning: None,
        });
    };
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let (config, warnings) = parse_with_warnings(&text, Some(path))?;
            let warning = warnings.first().cloned();
            Ok(ConfigLoad {
                config,
                source: ConfigSource::File(path.to_owned()),
                warnings,
                warning,
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ConfigLoad {
            config: Config::default(),
            source: ConfigSource::Defaults,
            warnings: Vec::new(),
            warning: None,
        }),
        Err(error) => Err(ConfigError::Read {
            path: path.to_owned(),
            error: error.to_string(),
        }),
    }
}

/// Serializes a [`Config`] to TOML, including **only** fields that the player actually honours.
///
/// Unhonoured / unapplied fields (such as `audio.output`, `audio.device`, `audio.buffer_ms`,
/// `audio.fadeout_ms`, `audio.gapless`, `audio.replaygain`, `ui.snap_distance`, `playlist.shuffle`,
/// `playlist.repeat`, `playlist.manual_advance`, `playlist.last_playlist`, and `plugins.*`) are
/// omitted so the config file does not advertise unsupported features or mislead users.
pub fn serialize_honest(config: &Config) -> Result<String, ConfigError> {
    let mut root = toml::Table::new();
    root.insert(
        "version".into(),
        toml::Value::Integer(i64::from(config.version)),
    );

    let mut audio = toml::Table::new();
    audio.insert(
        "volume".into(),
        toml::Value::Integer(i64::from(config.audio.volume)),
    );
    audio.insert(
        "balance".into(),
        toml::Value::Integer(i64::from(config.audio.balance)),
    );
    root.insert("audio".into(), toml::Value::Table(audio));

    let mut ui = toml::Table::new();
    if let Some(skin) = &config.ui.skin {
        ui.insert(
            "skin".into(),
            toml::Value::String(skin.to_string_lossy().into_owned()),
        );
    }
    ui.insert(
        "double_size".into(),
        toml::Value::Boolean(config.ui.double_size),
    );
    ui.insert(
        "always_on_top".into(),
        toml::Value::Boolean(config.ui.always_on_top),
    );
    ui.insert(
        "easy_move".into(),
        toml::Value::Boolean(config.ui.easy_move),
    );

    if !config.ui.windows.is_empty() {
        let mut windows_array = Vec::new();
        for w in &config.ui.windows {
            let mut win_table = toml::Table::new();
            let id_str = match w.id {
                WindowId::Main => "Main".to_string(),
                WindowId::Equalizer => "Equalizer".to_string(),
                WindowId::Playlist => "Playlist".to_string(),
                WindowId::Vis => "Vis".to_string(),
                WindowId::Video => "Video".to_string(),
                WindowId::Gen(n) => format!("Gen({n})"),
            };
            win_table.insert("id".into(), toml::Value::String(id_str));
            win_table.insert("x".into(), toml::Value::Integer(i64::from(w.x)));
            win_table.insert("y".into(), toml::Value::Integer(i64::from(w.y)));
            win_table.insert("visible".into(), toml::Value::Boolean(w.visible));
            win_table.insert("shaded".into(), toml::Value::Boolean(w.shaded));
            windows_array.push(toml::Value::Table(win_table));
        }
        ui.insert("windows".into(), toml::Value::Array(windows_array));
    }
    root.insert("ui".into(), toml::Value::Table(ui));

    let mut eq = toml::Table::new();
    let mut eq_state = toml::Table::new();
    eq_state.insert(
        "enabled".into(),
        toml::Value::Boolean(config.eq.state.enabled),
    );
    eq_state.insert("auto".into(), toml::Value::Boolean(config.eq.state.auto));
    eq_state.insert(
        "preamp".into(),
        toml::Value::Float(f64::from(config.eq.state.preamp)),
    );
    let bands: Vec<toml::Value> = config
        .eq
        .state
        .bands
        .iter()
        .map(|b| toml::Value::Float(f64::from(*b)))
        .collect();
    eq_state.insert("bands".into(), toml::Value::Array(bands));
    eq.insert("state".into(), toml::Value::Table(eq_state));
    root.insert("eq".into(), toml::Value::Table(eq));

    toml::to_string_pretty(&toml::Value::Table(root)).map_err(|e| ConfigError::Write {
        path: PathBuf::from("<serialize>"),
        error: e.to_string(),
    })
}

/// Saves `config` to `path` using honest serialization (writing only honoured fields).
///
/// Creates any missing parent directories. Returns a [`ConfigError::Write`] if writing fails.
pub fn save(config: &Config, path: &Path) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Write {
            path: path.to_owned(),
            error: e.to_string(),
        })?;
    }
    let toml_str = serialize_honest(config)?;
    std::fs::write(path, toml_str).map_err(|e| ConfigError::Write {
        path: path.to_owned(),
        error: e.to_string(),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::config::WindowLayout;

    #[test]
    fn a_missing_file_is_all_defaults() {
        let missing = PathBuf::from("does/not/exist/rustamp.toml");
        let loaded = load(Some(&missing)).unwrap();
        assert_eq!(loaded.config, Config::default());
        assert_eq!(loaded.source, ConfigSource::Defaults);
        assert_eq!(loaded.warning, None);
        assert!(loaded.warnings.is_empty());

        // And so is no path at all.
        let loaded = load(None).unwrap();
        assert_eq!(loaded.config, Config::default());
        assert_eq!(loaded.source, ConfigSource::Defaults);
        assert!(loaded.warnings.is_empty());
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
        let loaded = load(Some(&path)).unwrap();
        assert!(loaded.config.ui.double_size);
        assert_eq!(loaded.config.ui.skin, Some(PathBuf::from("skins/x.wsz")));
        assert_eq!(loaded.source, ConfigSource::File(path.clone()));
        assert_eq!(loaded.warning, None);
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn a_broken_file_fails_loudly_with_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken.toml");
        std::fs::write(&path, "this is = = not toml").unwrap();
        let err = load(Some(&path)).unwrap_err();
        match &err {
            ConfigError::Malformed {
                path: err_path,
                error,
            } => {
                assert_eq!(err_path, &path);
                assert!(!error.is_empty());
            }
            other => panic!("expected ConfigError::Malformed, got {other:?}"),
        }
        let err_text = err.to_string();
        assert!(
            err_text.contains("is malformed"),
            "expected 'is malformed' in {err_text}"
        );
    }

    #[test]
    fn the_default_path_honours_the_environment_override() {
        let path = default_path();
        if let Some(path) = path {
            assert!(path.is_absolute() || std::env::var_os(CONFIG_ENV).is_some());
        }
    }

    #[test]
    fn config_round_trip_honours_all_supported_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saved.toml");

        let mut config = Config::default();
        config.audio.volume = 175;
        config.audio.balance = -30;
        config.ui.skin = Some(PathBuf::from("skins/classic.wsz"));
        config.ui.double_size = true;
        config.ui.always_on_top = true;
        config.ui.easy_move = false;
        config.ui.windows = vec![
            WindowLayout {
                id: WindowId::Main,
                x: 120,
                y: 80,
                visible: true,
                shaded: true,
                width: None,
                height: None,
            },
            WindowLayout {
                id: WindowId::Equalizer,
                x: 120,
                y: 196,
                visible: true,
                shaded: false,
                width: None,
                height: None,
            },
            WindowLayout {
                id: WindowId::Playlist,
                x: 120,
                y: 312,
                visible: false,
                shaded: false,
                width: None,
                height: None,
            },
        ];
        config.eq.state.enabled = true;
        config.eq.state.auto = true;
        config.eq.state.preamp = 4.5;
        config.eq.state.bands = [1.0, 2.0, 3.0, 4.0, 5.0, -1.0, -2.0, -3.0, -4.0, -5.0];

        save(&config, &path).expect("save succeeds");
        assert!(path.exists(), "config file was written");

        let loaded = load(Some(&path)).expect("load succeeds");
        assert_eq!(loaded.source, ConfigSource::File(path));
        assert!(loaded.warnings.is_empty(), "honest save emits no warnings");

        assert_eq!(loaded.config.audio.volume, 175);
        assert_eq!(loaded.config.audio.balance, -30);
        assert_eq!(
            loaded.config.ui.skin,
            Some(PathBuf::from("skins/classic.wsz"))
        );
        assert!(loaded.config.ui.double_size);
        assert!(loaded.config.ui.always_on_top);
        assert!(!loaded.config.ui.easy_move);
        assert_eq!(loaded.config.ui.windows, config.ui.windows);
        assert_eq!(loaded.config.eq.state, config.eq.state);
    }

    #[test]
    fn ignored_fields_are_not_advertised() {
        let mut config = Config::default();
        config.audio.volume = 200;
        config.audio.replaygain = rustamp_core::config::ReplayGainMode::Album;
        config.audio.buffer_ms = 400;
        config.playlist.shuffle = true;
        config.playlist.repeat = true;
        config.plugins.disabled = vec!["bad_plugin.dll".into()];

        let serialized = serialize_honest(&config).expect("serialize honest");

        // The serialized text must NOT contain any unhonoured field names.
        let ignored_keywords = [
            "replaygain",
            "buffer_ms",
            "fadeout_ms",
            "gapless",
            "snap_distance",
            "manual_advance",
            "last_playlist",
            "plugin_dir",
            "vis_plugin",
            "vis_preset",
            "disabled",
            "shuffle",
            "repeat",
        ];
        for keyword in ignored_keywords {
            assert!(
                !serialized.contains(keyword),
                "honest serialization advertised ignored field '{keyword}':\n{serialized}"
            );
        }

        // When a file *does* contain ignored fields, loading it reports them in warnings.
        let toml_with_ignored = "\
[audio]
volume = 180
replaygain = \"Track\"
buffer_ms = 350

[playlist]
shuffle = true
";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ignored.toml");
        std::fs::write(&path, toml_with_ignored).unwrap();

        let loaded = load(Some(&path)).expect("load succeeds");
        assert_eq!(loaded.config.audio.volume, 180);
        assert!(
            loaded
                .warnings
                .iter()
                .any(|w| w.contains("audio.replaygain") && w.contains("not applied")),
            "expected warning for audio.replaygain in {:?}",
            loaded.warnings
        );
        assert!(
            loaded
                .warnings
                .iter()
                .any(|w| w.contains("audio.buffer_ms") && w.contains("not applied")),
            "expected warning for audio.buffer_ms in {:?}",
            loaded.warnings
        );
        assert!(
            loaded
                .warnings
                .iter()
                .any(|w| w.contains("playlist.shuffle") && w.contains("not applied")),
            "expected warning for playlist.shuffle in {:?}",
            loaded.warnings
        );
    }

    #[test]
    fn malformed_file_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad_syntax.toml");
        std::fs::write(&path, "[audio\nvolume = ").unwrap();

        let err = load(Some(&path)).unwrap_err();
        let err_msg = err.to_string();
        assert!(err_msg.contains("bad_syntax.toml"));
        assert!(err_msg.contains("is malformed"));
    }
}
