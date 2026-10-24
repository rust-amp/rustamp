// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Configuration loading and persistence (task cards N1 and N6).
//!
//! `rustamp-core` defines the typed [`Config`] tree and its defaults. The app
//! resolves a path, reads TOML, and applies the settings the player honours
//! (the audit table below names them and the place that applies each one).
//!
//! For settings that cannot be honoured yet (e.g. output backend selection,
//! audio buffer lengths, gapless playback, and plugins), the configuration
//! loader reports clear warnings rather than silently ignoring them.
//! ReplayGain is honoured (audio.replaygain), and so is the output device
//! (audio.device). The fade-out duration (audio.fadeout_ms) and the playback
//! modes (playlist.shuffle, playlist.repeat) are honoured as well.
//!
//! On clean exit, the app writes an honest configuration to `%RUSTAMP_CONFIG%`
//! (when set) or `%APPDATA%\Rustamp\rustamp.toml`. The written file carries
//! **only** the settings the player honours, so it never advertises an
//! unsupported setting as if it were in effect.
//!
//! # The audit: what the player applies, and what a save does with it
//!
//! | Setting | Applied by the player | On save |
//! |---|---|---|
//! | `version` | the schema marker | written |
//! | `audio.volume` | `App::with_registry` → `Engine::set_volume` | written |
//! | `audio.balance` | `App::with_registry` → `Engine::set_balance` | written |
//! | `audio.fadeout_ms` | `App::with_registry` → `EngineConfig::fade_ms` | written |
//! | `audio.output_mode` | `output::create_output` (RFC 0019) | written |
//! | `audio.device` | `output::create_output` → `selection::plan`/`resolve` | written |
//! | `audio.replaygain` | `App::with_registry` → `Engine::set_replaygain_mode` | written |
//! | `ui.skin` | `crate::run` → `skin::choose` | written |
//! | `ui.double_size` | `App::with_registry` → the main window's state | written |
//! | `ui.always_on_top` | `App::with_registry` → `App::always_on_top` | written |
//! | `ui.easy_move` | `App::with_registry` → `App::easy_move` | written |
//! | `ui.snap_distance` | `window` → `rustamp_ui::docking` | written |
//! | `ui.windows` | `App::with_registry` → the window placements | written |
//! | `ui.playlist_font_size` | the playlist editor's font size | written; an unusable value is kept |
//! | `ui.playlist_title_format` | the playlist's row text | written when the user set one; a format that cannot be read is reported and the plain text stays |
//! | `eq.state` | `App::with_registry` → `Engine::set_eq` | written |
//! | `plugins.plugin_dir` | `App::plugin_dir` → `rustamp_prefs::scan` | written |
//! | `plugins.disabled` | `App::set_plugin_enabled` → `rustamp_prefs::set_enabled` | written |
//! | `playlist.shuffle` | `App::with_registry` → `Engine::set_shuffle` (S15) | written |
//! | `playlist.repeat` | `App::with_registry` → `Engine::set_repeat` (S15) | written |
//! | `audio.output` | — the backend comes from the command line | **preserved** |
//! | `audio.buffer_ms` | — the output's buffer length is the device's own | **preserved** |
//! | `audio.gapless` | — | **preserved** |
//! | `playlist.manual_advance` | — | **preserved** |
//! | `playlist.last_playlist` | — | **preserved** |
//! | `plugins.vis_plugin` | — | **preserved** |
//! | `plugins.vis_preset` | — | **preserved** |
//!
//! Two points of the table need a word, because they are easy to misread:
//!
//! * **`audio.buffer_ms` has no mapping.** The output's buffer length is the
//!   device's own: WASAPI opens the device's default period and cpal leaves
//!   the size to the host. The engine's `ring_frames` is its internal
//!   decode-to-device ring, not the output buffer. Nothing in the player
//!   asks for a buffer length, so the setting stays preserved and warned
//!   about — a mapping would have to be invented.
//! * **`audio.fadeout_ms` and the playback modes are applied.** The app hands
//!   the configured fade to the engine (`EngineConfig::fade_ms`) and the
//!   configured modes to the engine's playback order (`Engine::set_shuffle`,
//!   `Engine::set_repeat`), so the buttons and the traversal agree with the
//!   file.
//!
//! # The save rule: a setting is never destroyed by a save that did not apply it
//!
//! A setting the player does not honour (an unknown key, a setting of a newer
//! version, `playlist.manual_advance`) is **not** part of the honest
//! serialization — but it is also **not** deleted. On every save the app reads
//! the file it is about to overwrite and copies every setting it does not own
//! back into it, with the text the file used (see [`crate::config_preserve`]).
//! A user who edits `rustamp.toml` by hand keeps those lines; the loader still
//! warns that they are not applied.
//!
//! The rule has three parts:
//!
//! 1. **Owned settings** are written by the app: it reads them and acts on
//!    them, so it knows their current value. The list is [`OWNED_PATHS`].
//! 2. **Every other setting** of the previous file is preserved verbatim.
//!    It belongs to the user or to a newer version of the app, and the app
//!    does not understand it, so it does not touch it.
//! 3. A file the app cannot read is **never overwritten**: a save that cannot
//!    preserve the file's settings would destroy them, so it fails loudly
//!    instead.
//!
//! A comment that documents a setting travels with that setting. A comment
//! that stands on its own — above a setting the app owns, or at the end of
//! the file — is not kept: the app rewrites the file it owns.
//!
//! A missing config file is **not** an error (it means "use defaults").
//! A malformed config file **fails loudly** with an explicit error rather than
//! silently falling back to defaults.

use std::fmt;
use std::path::{Path, PathBuf};

use rustamp_core::config::Config;

use crate::config_preserve;
use crate::title_format::TitleFormatSetting;

/// Environment variable that overrides the config path (also the test seam).
pub const CONFIG_ENV: &str = "RUSTAMP_CONFIG";

/// The key the playlist editor's runtime font size is stored under.
///
/// The size is a view setting of the playlist editor (S03 §2 `40335` /
/// `40336`), and it is **not** part of `rustamp-core`'s configuration tree:
/// adding a field there is a contract change that needs its own RFC. The app
/// therefore keeps the value in the same file, under the `[ui]` table, next
/// to the other UI state it persists.
pub const PLAYLIST_FONT_SIZE_KEY: &str = "playlist_font_size";

/// The key the playlist's title format is stored under.
///
/// The title format (spec **S14**) is a view setting of the playlist editor
/// as well, and it is not part of `rustamp-core`'s configuration tree either:
/// the app keeps it in the same file, under the `[ui]` table, next to the
/// other UI state it persists.
///
/// The value is a table of its own, so that the switch and the format string
/// travel together:
///
/// ```toml
/// [ui.playlist_title_format]
/// enabled = true
/// format = "[%artist% - ]%title%"
/// ```
///
/// The key is written only when the user set one: without it the rows show
/// their plain text.
pub const PLAYLIST_TITLE_FORMAT_KEY: &str = "playlist_title_format";

/// The configuration paths the app owns: the settings it reads and acts on.
///
/// The app decides the value of an owned setting on every save, so the
/// previous file's value for it is replaced, not preserved. Every other
/// setting of the file belongs to the user (or to a newer version of the
/// app) and is preserved verbatim — see the module docs for the audit and
/// the save rule.
///
/// A path is owned when the app *applies* it; the module docs' audit table
/// names the place that applies it. `eq.state` is one table, so every key
/// inside it belongs to the app.
///
/// The paths that are **not** listed here are not applied by the player:
/// `audio.output`, `audio.buffer_ms`, `audio.gapless`,
/// `playlist.manual_advance`, `playlist.last_playlist`, `plugins.vis_plugin`
/// and `plugins.vis_preset`. They are preserved on save and warned about on
/// load.
const OWNED_PATHS: &[&[&str]] = &[
    &["version"],
    &["audio", "volume"],
    &["audio", "balance"],
    &["audio", "fadeout_ms"],
    &["audio", "output_mode"],
    &["audio", "replaygain"],
    &["audio", "device"],
    &["ui", "skin"],
    &["ui", "double_size"],
    &["ui", "always_on_top"],
    &["ui", "easy_move"],
    &["ui", "snap_distance"],
    &["ui", "windows"],
    &["ui", PLAYLIST_FONT_SIZE_KEY],
    &["ui", PLAYLIST_TITLE_FORMAT_KEY],
    &["eq", "state"],
    &["playlist", "shuffle"],
    &["playlist", "repeat"],
    &["plugins", "plugin_dir"],
    &["plugins", "disabled"],
];

/// Whether the app owns the setting at `path`.
///
/// A path is owned when it *starts* with one of the [`OWNED_PATHS`]: the
/// equalizer's state is one table, so every key inside it belongs to the
/// app.
fn owns(path: &[String]) -> bool {
    OWNED_PATHS.iter().any(|owned| {
        owned.len() <= path.len()
            && owned
                .iter()
                .zip(path.iter())
                .all(|(name, part)| *name == part.as_str())
    })
}

/// Whether the app owns the setting at `path` in the file `previous`.
///
/// The playlist font size is the one owned setting whose value the app may
/// not be able to use: a value it cannot use is a setting the app did not
/// apply, so it is preserved instead of dropped. A value the app can use
/// belongs to the app, which writes its own.
///
/// The playlist's title format is the same: a table the app cannot read is a
/// setting the app did not apply, so the save keeps it. A table the app
/// **can** read belongs to the app, which writes its own — the app keeps the
/// user's format string as it is, so a format that cannot be parsed comes
/// back unchanged and can be fixed in the preferences page.
fn owns_setting(path: &[String], previous: &toml::Value) -> bool {
    if path.len() == 2 && path[0] == "ui" && path[1] == PLAYLIST_FONT_SIZE_KEY {
        return font_size_is_usable(previous);
    }
    if is_title_format_path(path) {
        return title_format_is_readable(previous);
    }
    owns(path)
}

/// Whether the path names a setting of the playlist's title format.
fn is_title_format_path(path: &[String]) -> bool {
    path.len() >= 2 && path[0] == "ui" && path[1] == PLAYLIST_TITLE_FORMAT_KEY
}

/// Whether the file's title format is one the app can read.
fn title_format_is_readable(previous: &toml::Value) -> bool {
    previous
        .get("ui")
        .and_then(toml::Value::as_table)
        .and_then(title_format_in_toml)
        .is_some()
}

/// The title format a `[ui]` table names, when it names one the app can read.
///
/// A table that is not one (a missing `enabled`, a `format` that is not a
/// string) is a setting the app cannot use: it is reported and kept.
fn title_format_in_toml(ui: &toml::Table) -> Option<TitleFormatSetting> {
    let table = ui.get(PLAYLIST_TITLE_FORMAT_KEY)?.as_table()?;
    let enabled = table.get("enabled")?.as_bool()?;
    let format = table.get("format")?.as_str()?;
    Some(TitleFormatSetting::new(enabled, format))
}

/// Whether the file's playlist font size is one the app can use.
fn font_size_is_usable(previous: &toml::Value) -> bool {
    previous
        .get("ui")
        .and_then(toml::Value::as_table)
        .is_some_and(|ui| playlist_font_size_in_toml(ui).is_some())
}

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
    ///
    /// A load reports it, and a save refuses to overwrite the file: the
    /// save would destroy the settings the app could not read.
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
    /// The playlist editor's runtime font size, when the file names one.
    ///
    /// `None` means the skin's size is in use (S17 §5); the value belongs to
    /// the app, not to `rustamp-core`'s configuration tree (see
    /// [`PLAYLIST_FONT_SIZE_KEY`]).
    pub playlist_font_size: Option<u32>,
    /// The playlist's title format, when the file names one.
    ///
    /// `None` means the user never set one: the rows show their plain text.
    /// The value belongs to the app, not to `rustamp-core`'s configuration
    /// tree (see [`PLAYLIST_TITLE_FORMAT_KEY`]).
    pub playlist_title_format: Option<TitleFormatSetting>,
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
                "buffer_ms" => Some("the output buffer length is the device's own"),
                "gapless" => Some("gapless mode is not configurable"),
                "replaygain" | "fadeout_ms" => None,
                "volume" | "balance" | "output_mode" | "device" => None,
                _ => Some("unrecognized audio setting"),
            };
            if let Some(r) = reason {
                warnings.push(format!("setting 'audio.{key}' is not applied: {r}"));
            }
        }
    }

    if let Some(ui) = table.get("ui").and_then(toml::Value::as_table) {
        for key in ui.keys() {
            let reason: Option<String> = match key.as_str() {
                "skin" | "double_size" | "always_on_top" | "easy_move" | "windows"
                | "snap_distance" => None,
                PLAYLIST_FONT_SIZE_KEY => {
                    if playlist_font_size_in_toml(ui).is_some() {
                        None
                    } else {
                        Some(format!(
                            "the playlist font size must be a whole number of pixels between {} and {}",
                            rustamp_ui::MIN_FONT_SIZE,
                            rustamp_ui::MAX_FONT_SIZE
                        ))
                    }
                }
                PLAYLIST_TITLE_FORMAT_KEY => {
                    if title_format_in_toml(ui).is_some() {
                        None
                    } else {
                        Some(
                            "the playlist title format must be a table with 'enabled' and 'format'"
                                .to_string(),
                        )
                    }
                }
                _ => Some("unrecognized UI setting".to_string()),
            };
            if let Some(r) = reason {
                warnings.push(format!("setting 'ui.{key}' is not applied: {r}"));
            }
        }
    }

    if let Some(playlist) = table.get("playlist").and_then(toml::Value::as_table) {
        for key in playlist.keys() {
            let reason = match key.as_str() {
                // The engine owns the traversal (S15): the app hands the
                // configured modes to it at startup.
                "shuffle" | "repeat" => None,
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
                // The plug-in pages edit and save these two.
                "plugin_dir" | "disabled" => None,
                "vis_plugin" => Some("visualization plugin loading is not supported yet"),
                "vis_preset" => Some("visualization preset is not supported yet"),
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
    let (config, warnings, _, _) = parse_text(text, path)?;
    Ok((config, warnings))
}

/// What a configuration file yields: the configuration, the warnings about
/// the settings the player does not apply, and the settings the app owns.
type ParsedFile = (Config, Vec<String>, Option<u32>, Option<TitleFormatSetting>);

/// Parses TOML into the configuration, the warnings and the app-owned
/// settings of the playlist editor.
fn parse_text(text: &str, path: Option<&Path>) -> Result<ParsedFile, ConfigError> {
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
    let ui = toml_val.get("ui").and_then(toml::Value::as_table);
    let playlist_font_size = ui.and_then(playlist_font_size_in_toml);
    let playlist_title_format = ui.and_then(title_format_in_toml);
    Ok((config, warnings, playlist_font_size, playlist_title_format))
}

/// The playlist font size a `[ui]` table names, when it names a usable one.
///
/// A value that is not a whole number of pixels in the accepted range is
/// unusable and read as "no size": the skin's size stays in use.
fn playlist_font_size_in_toml(ui: &toml::Table) -> Option<u32> {
    let raw = ui.get(PLAYLIST_FONT_SIZE_KEY)?.as_integer()?;
    let size = u32::try_from(raw).ok()?;
    (rustamp_ui::MIN_FONT_SIZE..=rustamp_ui::MAX_FONT_SIZE)
        .contains(&size)
        .then_some(size)
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
            playlist_font_size: None,
            playlist_title_format: None,
        });
    };
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let (config, warnings, playlist_font_size, playlist_title_format) =
                parse_text(&text, Some(path))?;
            let warning = warnings.first().cloned();
            Ok(ConfigLoad {
                config,
                source: ConfigSource::File(path.to_owned()),
                warnings,
                warning,
                playlist_font_size,
                playlist_title_format,
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ConfigLoad {
            config: Config::default(),
            source: ConfigSource::Defaults,
            warnings: Vec::new(),
            warning: None,
            playlist_font_size: None,
            playlist_title_format: None,
        }),
        Err(error) => Err(ConfigError::Read {
            path: path.to_owned(),
            error: error.to_string(),
        }),
    }
}

/// Serializes a [`Config`] to TOML, including **only** fields that the player actually honours.
///
/// Unhonoured / unapplied fields (such as `audio.output`, `audio.buffer_ms`,
/// `audio.gapless`, `playlist.manual_advance`, `playlist.last_playlist`, and
/// `plugins.*`) are omitted so the config file does not advertise unsupported
/// features or mislead users.
///
/// This is the player's **own** part of the file. A save writes it together
/// with the settings the player does not own, which the previous file keeps
/// — see [`serialize_preserving`] and the module docs for the save rule.
///
/// `playlist_font_size` is the playlist editor's runtime font size: it is
/// written only when the user chose one (S03 §2 `40335` / `40336`), because
/// the skin's size is in use otherwise.
///
/// `playlist_title_format` is the playlist's title format (spec S14): it is
/// written only when the user set one, because the rows show their plain
/// text otherwise.
pub fn serialize_honest(
    config: &Config,
    playlist_font_size: Option<u32>,
    playlist_title_format: Option<&TitleFormatSetting>,
) -> Result<String, ConfigError> {
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
    // The fade-out duration: the engine runs it (`EngineConfig::fade_ms`),
    // so the value the player applies is the one worth keeping.
    audio.insert(
        "fadeout_ms".into(),
        toml::Value::Integer(i64::from(config.audio.fadeout_ms)),
    );
    // Persist the request, not a device-specific downgrade. Like the native
    // preferences label says, a change takes effect at the next startup
    // (also used when the app recreates the engine and device).
    audio.insert(
        "output_mode".into(),
        toml::Value::try_from(config.audio.output_mode).map_err(|e| ConfigError::Write {
            path: PathBuf::from("<serialize>"),
            error: e.to_string(),
        })?,
    );
    audio.insert(
        "replaygain".into(),
        toml::Value::String(
            match config.audio.replaygain {
                rustamp_core::config::ReplayGainMode::Off => "Off",
                rustamp_core::config::ReplayGainMode::Track => "Track",
                rustamp_core::config::ReplayGainMode::Album => "Album",
            }
            .to_owned(),
        ),
    );
    // The device the player opens is honoured (see `crate::output`), so the
    // name is persisted — but only when one is configured, so that the
    // default file stays small.
    if let Some(device) = &config.audio.device {
        audio.insert("device".into(), toml::Value::String(device.clone()));
    }
    // Note on deliberately unpersisted fields:
    // audio.output, audio.buffer_ms and audio.gapless are deliberately not
    // written because the audio engine does not support runtime
    // configuration of them yet. They are not deleted either: a save
    // keeps the file's own value (see `serialize_preserving`).
    // audio.replaygain, audio.device and audio.fadeout_ms are written: they
    // are honoured by the engine (task card E4/A15 wiring, the device
    // selection and the fade-out duration).
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
    ui.insert(
        "snap_distance".into(),
        toml::Value::Integer(i64::from(config.ui.snap_distance)),
    );
    // The playlist editor's runtime font size, when the user chose one: the
    // skin's size is in use otherwise, so the key stays out of the file.
    if let Some(size) = playlist_font_size {
        ui.insert(
            PLAYLIST_FONT_SIZE_KEY.into(),
            toml::Value::Integer(i64::from(size)),
        );
    }
    // The playlist's title format, when the user set one: the rows show
    // their plain text otherwise, so the key stays out of the file.
    if let Some(setting) = playlist_title_format {
        let mut title_format = toml::Table::new();
        title_format.insert("enabled".into(), toml::Value::Boolean(setting.enabled));
        title_format.insert("format".into(), toml::Value::String(setting.format.clone()));
        ui.insert(
            PLAYLIST_TITLE_FORMAT_KEY.into(),
            toml::Value::Table(title_format),
        );
    }

    if !config.ui.windows.is_empty() {
        let mut windows_array = Vec::new();
        for w in &config.ui.windows {
            // Serialize WindowLayout via serde directly. Hand-rolling child table fields
            // caused W10 finding F1 (malformed WindowId::Gen format) and N8 test failure
            // (omitted width and height). Delegating to serde guarantees all WindowLayout
            // fields serialize in the exact format the serde loader expects.
            let win_value = toml::Value::try_from(w.clone()).map_err(|e| ConfigError::Write {
                path: PathBuf::from("<serialize>"),
                error: e.to_string(),
            })?;
            windows_array.push(win_value);
        }
        ui.insert("windows".into(), toml::Value::Array(windows_array));
    }
    root.insert("ui".into(), toml::Value::Table(ui));

    // The playback modes: the engine owns the traversal (S15) and the app
    // hands the configured modes to it, so they are the player's own.
    let mut playlist = toml::Table::new();
    playlist.insert(
        "shuffle".into(),
        toml::Value::Boolean(config.playlist.shuffle),
    );
    playlist.insert(
        "repeat".into(),
        toml::Value::Boolean(config.playlist.repeat),
    );
    root.insert("playlist".into(), toml::Value::Table(playlist));

    let mut eq = toml::Table::new();
    // Serialize EqState via serde directly. Hand-rolling eq_state fields risks
    // field omission and drift. Delegating to serde ensures all bands, preamp,
    // enabled, and auto flags serialize in the format the serde loader expects.
    let eq_state = toml::Value::try_from(config.eq.state).map_err(|e| ConfigError::Write {
        path: PathBuf::from("<serialize>"),
        error: e.to_string(),
    })?;
    eq.insert("state".into(), eq_state);
    root.insert("eq".into(), toml::Value::Table(eq));

    // The plug-in pages edit the plug-in folder and the off-list, so those two
    // are persisted (and only when set, to keep a default file small).
    let mut plugins = toml::Table::new();
    if let Some(dir) = &config.plugins.plugin_dir {
        plugins.insert(
            "plugin_dir".into(),
            toml::Value::String(dir.to_string_lossy().into_owned()),
        );
    }
    if !config.plugins.disabled.is_empty() {
        plugins.insert(
            "disabled".into(),
            toml::Value::Array(
                config
                    .plugins
                    .disabled
                    .iter()
                    .cloned()
                    .map(toml::Value::String)
                    .collect(),
            ),
        );
    }
    if !plugins.is_empty() {
        root.insert("plugins".into(), toml::Value::Table(plugins));
    }

    // Note on deliberately unpersisted sections:
    // the playlist's manual_advance / last_playlist and the plugins'
    // vis_plugin / vis_preset are deliberately not written because none of
    // those features are honoured by the player yet (task card N6).
    // They are kept, not deleted: a save copies the file's own settings back
    // (see `serialize_preserving`).
    // The playlist's shuffle / repeat, audio.replaygain, audio.device and
    // audio.fadeout_ms are written (see above).

    toml::to_string_pretty(&toml::Value::Table(root)).map_err(|e| ConfigError::Write {
        path: PathBuf::from("<serialize>"),
        error: e.to_string(),
    })
}

/// Serializes a [`Config`] to TOML and keeps the settings of `previous` that
/// the player does not own.
///
/// The result is the player's own serialization ([`serialize_honest`]) plus
/// every setting of `previous` the player does not apply, copied over with
/// the text the file used. A setting the player did not apply is never
/// destroyed by a save that did not apply it either.
///
/// `previous` must be valid TOML; a file that is not is reported as
/// [`ConfigError::Malformed`] rather than silently replaced. The same is
/// reported when the merged document would not be a config file any more:
/// a setting that cannot be kept must not break the file.
pub fn serialize_preserving(
    config: &Config,
    playlist_font_size: Option<u32>,
    playlist_title_format: Option<&TitleFormatSetting>,
    previous: &str,
) -> Result<String, ConfigError> {
    merged_document(
        config,
        playlist_font_size,
        playlist_title_format,
        previous,
        Path::new("<memory>"),
    )
}

/// The merged document for `previous`, or the reason why there is none.
///
/// `path` names the file the settings came from; it is the path the errors
/// carry.
fn merged_document(
    config: &Config,
    playlist_font_size: Option<u32>,
    playlist_title_format: Option<&TitleFormatSetting>,
    previous: &str,
    path: &Path,
) -> Result<String, ConfigError> {
    let previous_value: toml::Value =
        toml::from_str(previous).map_err(|e| ConfigError::Malformed {
            path: path.to_owned(),
            error: e.to_string(),
        })?;
    let honest = serialize_honest(config, playlist_font_size, playlist_title_format)?;
    let merged = config_preserve::merge(&honest, previous, &|path| {
        owns_setting(path, &previous_value)
    });
    // The merged file must still be a config file the player can read: a
    // setting that cannot be kept must not break the file.
    toml::from_str::<Config>(&merged).map_err(|e| ConfigError::Malformed {
        path: path.to_owned(),
        error: format!("the merged file is not a config file: {e}"),
    })?;
    Ok(merged)
}

/// Saves `config` to `path`, keeping the settings of the file that are not
/// the player's own.
///
/// Creates any missing parent directories. Returns a [`ConfigError::Write`] if writing fails.
///
/// A file that is not valid TOML is **not** overwritten: the save would
/// destroy the settings the app could not read, so it fails loudly with
/// [`ConfigError::Malformed`] and the file stays as it was.
pub fn save(
    config: &Config,
    playlist_font_size: Option<u32>,
    playlist_title_format: Option<&TitleFormatSetting>,
    path: &Path,
) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| ConfigError::Write {
            path: path.to_owned(),
            error: e.to_string(),
        })?;
    }
    let toml_str = match std::fs::read_to_string(path) {
        Ok(previous) => {
            // A file the app cannot read is never overwritten: the save
            // would destroy settings the app never understood.
            toml::from_str::<toml::Value>(&previous).map_err(|e| ConfigError::Malformed {
                path: path.to_owned(),
                error: e.to_string(),
            })?;
            merged_document(
                config,
                playlist_font_size,
                playlist_title_format,
                &previous,
                path,
            )?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            serialize_honest(config, playlist_font_size, playlist_title_format)?
        }
        Err(error) => {
            return Err(ConfigError::Read {
                path: path.to_owned(),
                error: error.to_string(),
            });
        }
    };
    std::fs::write(path, toml_str).map_err(|e| ConfigError::Write {
        path: path.to_owned(),
        error: e.to_string(),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::config::{
        AudioConfig, EqConfig, OutputMode, PlaylistConfig, PluginsConfig, ReplayGainMode, UiConfig,
        WindowLayout,
    };
    use rustamp_core::state::{EqState, WindowId};

    /// A configuration in which **every** field of **every** configuration
    /// type carries a value that differs from the default.
    ///
    /// Two properties make this the fixture of the round trips below:
    ///
    /// * The struct literals are exhaustive, so a new field cannot be added
    ///   to a configuration struct without this function failing to compile.
    /// * Every value differs from the default, so a field the writer drops
    ///   cannot hide behind the value the loader falls back to.
    ///
    /// The window list covers every `WindowId` variant (including `Gen(n)`
    /// and `Video`) and both a sized and an unsized window.
    ///
    /// The settings the player does **not** honour are set as well
    /// (`audio.output`, `audio.buffer_ms`, `audio.gapless`,
    /// `playlist.manual_advance`, `playlist.last_playlist`,
    /// `plugins.vis_plugin`, `plugins.vis_preset`): the writer must not
    /// advertise them, and a fixture that left them at their default could
    /// not tell an advertised setting from an omitted one.
    fn every_field_set() -> Config {
        Config {
            version: 7,
            audio: AudioConfig {
                output: "cpal".into(),
                device: Some("Speakers (Realtek)".into()),
                output_mode: OutputMode::Exclusive,
                buffer_ms: 350,
                fadeout_ms: 800,
                gapless: false,
                replaygain: ReplayGainMode::Album,
                volume: 150,
                balance: -40,
            },
            ui: UiConfig {
                skin: Some(PathBuf::from("skins/amplifier.wsz")),
                double_size: true,
                always_on_top: true,
                easy_move: false,
                snap_distance: 25,
                windows: vec![
                    WindowLayout {
                        id: WindowId::Main,
                        x: 50,
                        y: 75,
                        visible: true,
                        shaded: true,
                        width: None,
                        height: None,
                    },
                    WindowLayout {
                        id: WindowId::Equalizer,
                        x: 50,
                        y: 191,
                        visible: true,
                        shaded: false,
                        width: None,
                        height: None,
                    },
                    WindowLayout {
                        id: WindowId::Playlist,
                        x: 50,
                        y: 307,
                        visible: false,
                        shaded: false,
                        width: Some(275),
                        height: Some(116),
                    },
                    WindowLayout {
                        id: WindowId::Vis,
                        x: 330,
                        y: 75,
                        visible: true,
                        shaded: false,
                        width: Some(275),
                        height: Some(200),
                    },
                    WindowLayout {
                        id: WindowId::Gen(42),
                        x: 330,
                        y: 285,
                        visible: true,
                        shaded: false,
                        width: Some(300),
                        height: Some(250),
                    },
                    WindowLayout {
                        id: WindowId::Video,
                        x: 610,
                        y: 75,
                        visible: false,
                        shaded: false,
                        width: Some(352),
                        height: Some(240),
                    },
                ],
            },
            eq: EqConfig {
                state: EqState {
                    bands: [1.5, 2.5, 3.5, 4.5, 5.5, -1.5, -2.5, -3.5, -4.5, -5.5],
                    preamp: 5.0,
                    enabled: false,
                    auto: true,
                },
            },
            playlist: PlaylistConfig {
                shuffle: true,
                repeat: true,
                manual_advance: true,
                last_playlist: Some(PathBuf::from("playlists/favs.m3u")),
            },
            plugins: PluginsConfig {
                plugin_dir: Some(PathBuf::from("plugins")),
                vis_plugin: Some("vis_milk2.dll".into()),
                vis_preset: Some("preset_1.milk".into()),
                disabled: vec!["bad.dll".into()],
            },
        }
    }

    /// The premise of the round trips, made checkable: every field of the
    /// fixture differs from its default.
    ///
    /// A field that stayed at its default could not be distinguished from a
    /// field the writer drops, because the loader falls back to exactly that
    /// default. The guard makes the premise a fact instead of an assumption,
    /// and it fails loudly when a default changes under the fixture.
    #[test]
    fn every_field_of_the_fixture_differs_from_its_default() {
        let c = every_field_set();
        let d = Config::default();

        assert_ne!(c.version, d.version, "version");
        assert_ne!(c.audio.output, d.audio.output, "audio.output");
        assert_ne!(c.audio.device, d.audio.device, "audio.device");
        assert_ne!(
            c.audio.output_mode, d.audio.output_mode,
            "audio.output_mode"
        );
        assert_ne!(c.audio.buffer_ms, d.audio.buffer_ms, "audio.buffer_ms");
        assert_ne!(c.audio.fadeout_ms, d.audio.fadeout_ms, "audio.fadeout_ms");
        assert_ne!(c.audio.gapless, d.audio.gapless, "audio.gapless");
        assert_ne!(c.audio.replaygain, d.audio.replaygain, "audio.replaygain");
        assert_ne!(c.audio.volume, d.audio.volume, "audio.volume");
        assert_ne!(c.audio.balance, d.audio.balance, "audio.balance");

        assert_ne!(c.ui.skin, d.ui.skin, "ui.skin");
        assert_ne!(c.ui.double_size, d.ui.double_size, "ui.double_size");
        assert_ne!(c.ui.always_on_top, d.ui.always_on_top, "ui.always_on_top");
        assert_ne!(c.ui.easy_move, d.ui.easy_move, "ui.easy_move");
        assert_ne!(c.ui.snap_distance, d.ui.snap_distance, "ui.snap_distance");
        assert_ne!(c.ui.windows, d.ui.windows, "ui.windows");

        assert_ne!(c.eq.state.bands, d.eq.state.bands, "eq.state.bands");
        assert_ne!(c.eq.state.preamp, d.eq.state.preamp, "eq.state.preamp");
        assert_ne!(c.eq.state.enabled, d.eq.state.enabled, "eq.state.enabled");
        assert_ne!(c.eq.state.auto, d.eq.state.auto, "eq.state.auto");

        assert_ne!(c.playlist.shuffle, d.playlist.shuffle, "playlist.shuffle");
        assert_ne!(c.playlist.repeat, d.playlist.repeat, "playlist.repeat");
        assert_ne!(
            c.playlist.manual_advance, d.playlist.manual_advance,
            "playlist.manual_advance"
        );
        assert_ne!(
            c.playlist.last_playlist, d.playlist.last_playlist,
            "playlist.last_playlist"
        );

        assert_ne!(
            c.plugins.plugin_dir, d.plugins.plugin_dir,
            "plugins.plugin_dir"
        );
        assert_ne!(
            c.plugins.vis_plugin, d.plugins.vis_plugin,
            "plugins.vis_plugin"
        );
        assert_ne!(
            c.plugins.vis_preset, d.plugins.vis_preset,
            "plugins.vis_preset"
        );
        assert_ne!(c.plugins.disabled, d.plugins.disabled, "plugins.disabled");
    }

    /// The value at `path` in a TOML document, when the document has one.
    fn value_at<'a>(document: &'a toml::Value, path: &[&str]) -> Option<&'a toml::Value> {
        let mut current = document;
        for name in path {
            current = current.get(name)?;
        }
        Some(current)
    }

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
    fn a_generic_window_id_survives_the_written_file() {
        // W10 finding F1: the writer hand-rolled the id as a string, which is
        // not the form the loader reads back for a generic window, so a clean
        // exit wrote a file that the next launch refused. The round-trip test
        // above only used unit variants, which is why it could not see it.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("generic.toml");
        let mut config = Config::default();
        config.ui.windows = vec![
            WindowLayout {
                id: WindowId::Gen(3),
                x: 1,
                y: 2,
                visible: true,
                shaded: false,
                width: Some(275),
                height: Some(232),
            },
            WindowLayout {
                id: WindowId::Main,
                x: 0,
                y: 0,
                visible: true,
                shaded: false,
                width: None,
                height: None,
            },
        ];

        save(&config, None, None, &path).expect("save succeeds");
        let loaded = load(Some(&path)).expect("a written file must load again");
        assert_eq!(loaded.config.ui.windows, config.ui.windows);
    }

    /// The honest serialization writes exactly the settings the player
    /// honours (the audit table in this module names them). The round trip
    /// therefore carries a **non-default value for every one of them**: a
    /// setting that stays at its default cannot be told apart from a setting
    /// the writer dropped, because the loader falls back to exactly that
    /// default.
    ///
    /// The comparison is field by field, so a failure names the setting that
    /// broke, and the whole configuration is compared on top of it.
    #[test]
    fn config_round_trip_honours_all_supported_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saved.toml");

        let config = every_field_set();

        save(&config, Some(18), None, &path).expect("save succeeds");
        assert!(path.exists(), "config file was written");

        let loaded = load(Some(&path)).expect("load succeeds");
        assert_eq!(loaded.source, ConfigSource::File(path));
        assert!(
            loaded.warnings.is_empty(),
            "honest save emits no warnings: {:?}",
            loaded.warnings
        );
        assert_eq!(
            loaded.playlist_font_size,
            Some(18),
            "the playlist font size survives the round trip"
        );

        // Field by field, so a failure names the setting that broke.
        assert_eq!(loaded.config.version, config.version, "version");
        assert_eq!(
            loaded.config.audio.volume, config.audio.volume,
            "audio.volume"
        );
        assert_eq!(
            loaded.config.audio.balance, config.audio.balance,
            "audio.balance"
        );
        assert_eq!(
            loaded.config.audio.fadeout_ms, config.audio.fadeout_ms,
            "audio.fadeout_ms"
        );
        assert_eq!(
            loaded.config.audio.output_mode, config.audio.output_mode,
            "audio.output_mode"
        );
        assert_eq!(
            loaded.config.audio.replaygain, config.audio.replaygain,
            "audio.replaygain"
        );
        assert_eq!(
            loaded.config.audio.device, config.audio.device,
            "audio.device"
        );
        assert_eq!(loaded.config.ui.skin, config.ui.skin, "ui.skin");
        assert_eq!(
            loaded.config.ui.double_size, config.ui.double_size,
            "ui.double_size"
        );
        assert_eq!(
            loaded.config.ui.always_on_top, config.ui.always_on_top,
            "ui.always_on_top"
        );
        assert_eq!(
            loaded.config.ui.easy_move, config.ui.easy_move,
            "ui.easy_move"
        );
        assert_eq!(
            loaded.config.ui.snap_distance, config.ui.snap_distance,
            "ui.snap_distance"
        );
        assert_eq!(loaded.config.eq.state, config.eq.state, "eq.state");
        assert_eq!(
            loaded.config.playlist.shuffle, config.playlist.shuffle,
            "playlist.shuffle"
        );
        assert_eq!(
            loaded.config.playlist.repeat, config.playlist.repeat,
            "playlist.repeat"
        );
        assert_eq!(
            loaded.config.plugins.plugin_dir, config.plugins.plugin_dir,
            "plugins.plugin_dir"
        );
        assert_eq!(
            loaded.config.plugins.disabled, config.plugins.disabled,
            "plugins.disabled"
        );

        // Every window keeps every field, including its size.
        assert_eq!(
            loaded.config.ui.windows.len(),
            config.ui.windows.len(),
            "ui.windows: every window survives"
        );
        for (written, read_back) in config
            .ui
            .windows
            .iter()
            .zip(loaded.config.ui.windows.iter())
        {
            assert_eq!(read_back.id, written.id, "ui.windows: the id survives");
            assert_eq!(read_back.x, written.x, "ui.windows: x survives");
            assert_eq!(read_back.y, written.y, "ui.windows: y survives");
            assert_eq!(
                read_back.visible, written.visible,
                "ui.windows: visible survives"
            );
            assert_eq!(
                read_back.shaded, written.shaded,
                "ui.windows: shaded survives"
            );
            assert_eq!(read_back.width, written.width, "ui.windows: width survives");
            assert_eq!(
                read_back.height, written.height,
                "ui.windows: height survives"
            );
        }

        // The whole configuration, with the settings the player does not
        // apply back at their defaults: they are not written, so the loader
        // falls back to the default for every one of them.
        let mut expected = config.clone();
        expected.audio.output = Config::default().audio.output;
        expected.audio.buffer_ms = Config::default().audio.buffer_ms;
        expected.audio.gapless = Config::default().audio.gapless;
        expected.playlist.manual_advance = Config::default().playlist.manual_advance;
        expected.playlist.last_playlist = Config::default().playlist.last_playlist;
        expected.plugins.vis_plugin = Config::default().plugins.vis_plugin;
        expected.plugins.vis_preset = Config::default().plugins.vis_preset;
        assert_eq!(
            loaded.config, expected,
            "the whole configuration survives the round trip, except the settings the player does not apply"
        );
    }

    /// The honest serialization must never write an owned setting as its
    /// default: the file says what the player does, and the player's state
    /// differs from the default in every setting the app owns.
    ///
    /// The check is driven by [`OWNED_PATHS`] — the list of the settings the
    /// app owns — so it covers the whole owned set: a setting that is added
    /// to that list and to the writer, but left at its default in the
    /// fixture, fails here instead of passing unnoticed.
    ///
    /// A writer that writes the default for one setting (the N9/F5 class of
    /// bug) fails here as well: the written document and the default's
    /// document then say the same thing at that path.
    #[test]
    fn the_honest_serialization_never_writes_an_owned_setting_as_its_default() {
        // The app's own settings are part of the fixture: the playlist font
        // size and the playlist's title format are the two settings the app
        // owns beside the configuration tree.
        let title_format = TitleFormatSetting::new(true, "[%artist% - ]%title%");
        let written = toml::from_str::<toml::Value>(
            &serialize_honest(&every_field_set(), Some(18), Some(&title_format))
                .expect("serialize the fixture"),
        )
        .expect("the writer's output is valid TOML");
        let default = toml::from_str::<toml::Value>(
            &serialize_honest(&Config::default(), None, None).expect("serialize the default"),
        )
        .expect("the default's document is valid TOML");

        for owned in OWNED_PATHS {
            let path: Vec<&str> = owned.to_vec();
            assert_ne!(
                value_at(&written, &path),
                value_at(&default, &path),
                "the owned setting {} is written as its default",
                path.join(".")
            );
        }
    }

    /// The writer's output is a config file the loader reads back, without
    /// the file system in between: the in-memory path of the honest
    /// serialization, which the file round trip cannot show on its own — the
    /// save merges the writer's output with the file that was there.
    ///
    /// The settings the player does not apply are not advertised: the
    /// document does not mention them at all.
    #[test]
    fn the_honest_serialization_round_trips_in_memory() {
        let config = every_field_set();
        let text = serialize_honest(&config, Some(18), None).expect("serialize");

        let back = parse(&text).expect("the writer's output is a config file");
        assert_eq!(back.version, config.version, "version");
        assert_eq!(back.audio.volume, config.audio.volume, "audio.volume");
        assert_eq!(back.audio.balance, config.audio.balance, "audio.balance");
        assert_eq!(
            back.audio.fadeout_ms, config.audio.fadeout_ms,
            "audio.fadeout_ms"
        );
        assert_eq!(
            back.audio.output_mode, config.audio.output_mode,
            "audio.output_mode"
        );
        assert_eq!(
            back.audio.replaygain, config.audio.replaygain,
            "audio.replaygain"
        );
        assert_eq!(back.audio.device, config.audio.device, "audio.device");
        assert_eq!(back.ui.skin, config.ui.skin, "ui.skin");
        assert_eq!(back.ui.double_size, config.ui.double_size, "ui.double_size");
        assert_eq!(
            back.ui.always_on_top, config.ui.always_on_top,
            "ui.always_on_top"
        );
        assert_eq!(back.ui.easy_move, config.ui.easy_move, "ui.easy_move");
        assert_eq!(
            back.ui.snap_distance, config.ui.snap_distance,
            "ui.snap_distance"
        );
        assert_eq!(back.ui.windows, config.ui.windows, "ui.windows");
        assert_eq!(back.eq.state, config.eq.state, "eq.state");
        assert_eq!(
            back.playlist.shuffle, config.playlist.shuffle,
            "playlist.shuffle"
        );
        assert_eq!(
            back.playlist.repeat, config.playlist.repeat,
            "playlist.repeat"
        );
        assert_eq!(
            back.plugins.plugin_dir, config.plugins.plugin_dir,
            "plugins.plugin_dir"
        );
        assert_eq!(
            back.plugins.disabled, config.plugins.disabled,
            "plugins.disabled"
        );

        let document = toml::from_str::<toml::Value>(&text).expect("valid TOML");
        for (table, key) in [
            ("audio", "output"),
            ("audio", "buffer_ms"),
            ("audio", "gapless"),
            ("playlist", "manual_advance"),
            ("playlist", "last_playlist"),
            ("plugins", "vis_plugin"),
            ("plugins", "vis_preset"),
        ] {
            assert!(
                value_at(&document, &[table, key]).is_none(),
                "the setting {table}.{key} is not applied and must not be advertised:\n{text}"
            );
        }
    }

    /// The settings the player does not apply are never written, and they
    /// come back at their documented defaults: the loader falls back to the
    /// default for every one of them.
    ///
    /// The fixture sets them to non-default values, so the file would say
    /// something different if the writer advertised them. The settings the
    /// player *does* apply are the subject of the round trip above.
    #[test]
    fn unpersisted_fields_deliberately_fall_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unpersisted.toml");

        let config = every_field_set();

        save(&config, None, None, &path).expect("save succeeds");

        let text = std::fs::read_to_string(&path).expect("the file is there");
        for (table, key) in [
            ("audio", "output"),
            ("audio", "buffer_ms"),
            ("audio", "gapless"),
            ("playlist", "manual_advance"),
            ("playlist", "last_playlist"),
            ("plugins", "vis_plugin"),
            ("plugins", "vis_preset"),
        ] {
            assert!(
                !text
                    .lines()
                    .any(|line| line.starts_with(&format!("{key} ="))),
                "the setting {table}.{key} is not applied and must not be written:\n{text}"
            );
        }

        let loaded = load(Some(&path)).expect("load succeeds");
        assert_eq!(loaded.config.audio.output, Config::default().audio.output);
        assert_eq!(
            loaded.config.audio.buffer_ms,
            Config::default().audio.buffer_ms
        );
        assert_eq!(loaded.config.audio.gapless, Config::default().audio.gapless);
        assert!(!loaded.config.playlist.manual_advance);
        assert_eq!(loaded.config.playlist.last_playlist, None);
        assert_eq!(loaded.config.plugins.vis_plugin, None);
        assert_eq!(loaded.config.plugins.vis_preset, None);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    /// The playlist editor's runtime font size is the app's own setting
    /// (S03 §2 `40335` / `40336`): it is written only when the user chose
    /// one, and an unusable value is reported and ignored.
    #[test]
    fn the_playlist_font_size_round_trips_and_an_unusable_value_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("playlist-font.toml");

        save(&Config::default(), Some(18), None, &path).expect("save succeeds");
        let loaded = load(Some(&path)).expect("load succeeds");
        assert_eq!(loaded.playlist_font_size, Some(18));
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);

        // Without a chosen size the key stays out of the file: the skin's
        // size is in use then.
        save(&Config::default(), None, None, &path).expect("save succeeds");
        let text = std::fs::read_to_string(&path).expect("the file is there");
        assert!(!text.contains(PLAYLIST_FONT_SIZE_KEY), "{text}");
        assert_eq!(
            load(Some(&path)).expect("load succeeds").playlist_font_size,
            None
        );

        // A value the playlist cannot use is reported and ignored.
        for value in ["900", "-4", "\"big\""] {
            std::fs::write(&path, format!("[ui]\n{PLAYLIST_FONT_SIZE_KEY} = {value}\n"))
                .expect("the file is written");
            let loaded = load(Some(&path)).expect("load succeeds");
            assert_eq!(loaded.playlist_font_size, None, "{value}");
            assert!(
                loaded
                    .warnings
                    .iter()
                    .any(|w| w.contains("ui.playlist_font_size") && w.contains("not applied")),
                "{value}: {:?}",
                loaded.warnings
            );
        }
    }

    /// The playlist's title format is the app's own setting (spec S14): it
    /// is written only when the user set one, and a table the app cannot
    /// read is reported and kept.
    #[test]
    fn the_playlist_title_format_round_trips_and_a_table_it_cannot_read_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("playlist-title-format.toml");
        let setting = TitleFormatSetting::new(true, "[%artist% - ]%title%");

        save(&Config::default(), None, Some(&setting), &path).expect("save succeeds");
        let loaded = load(Some(&path)).expect("load succeeds");
        assert_eq!(loaded.playlist_title_format, Some(setting));
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);

        // Without a setting the key stays out of the file: the rows show
        // their plain text then.
        save(&Config::default(), None, None, &path).expect("save succeeds");
        let text = std::fs::read_to_string(&path).expect("the file is there");
        assert!(!text.contains(PLAYLIST_TITLE_FORMAT_KEY), "{text}");
        assert_eq!(
            load(Some(&path))
                .expect("load succeeds")
                .playlist_title_format,
            None
        );

        // A table the app cannot read is reported, and the save keeps it: a
        // setting the app did not apply is never destroyed.
        let broken = "[ui.playlist_title_format]\nenabled = true\nformat = 42\n";
        std::fs::write(&path, broken).expect("the file is written");
        let loaded = load(Some(&path)).expect("load succeeds");
        assert_eq!(loaded.playlist_title_format, None);
        assert!(
            loaded
                .warnings
                .iter()
                .any(|w| w.contains("ui.playlist_title_format") && w.contains("not applied")),
            "{:?}",
            loaded.warnings
        );
        save(&Config::default(), None, None, &path).expect("save succeeds");
        let saved = std::fs::read_to_string(&path).expect("the file is there");
        assert!(
            saved.lines().any(|line| line == "format = 42"),
            "the table the app cannot read survives:\n{saved}"
        );
    }

    /// A format string the user typed comes back as it is, even when it
    /// cannot be used: the app keeps the text, so the user can fix it.
    #[test]
    fn a_format_string_that_cannot_be_used_survives_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("broken-format.toml");
        let setting = TitleFormatSetting::new(true, "%title");

        save(&Config::default(), None, Some(&setting), &path).expect("save succeeds");
        let loaded = load(Some(&path)).expect("load succeeds");
        assert_eq!(loaded.playlist_title_format, Some(setting));
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    }

    #[test]
    fn ignored_fields_are_not_advertised() {
        let mut config = Config::default();
        config.audio.volume = 200;
        config.audio.replaygain = ReplayGainMode::Album;
        config.audio.buffer_ms = 400;
        config.audio.fadeout_ms = 900;
        config.playlist.shuffle = true;
        config.playlist.repeat = true;
        config.plugins.disabled = vec!["bad_plugin.dll".into()];

        let serialized = serialize_honest(&config, None, None).expect("serialize honest");

        // The serialized text must NOT contain any unhonoured field names.
        let ignored_keywords = [
            "buffer_ms",
            "gapless",
            "manual_advance",
            "last_playlist",
            "vis_plugin",
            "vis_preset",
        ];
        for keyword in ignored_keywords {
            assert!(
                !serialized.contains(keyword),
                "honest serialization advertised ignored field '{keyword}':\n{serialized}"
            );
        }

        // The honoured settings are written: the file says what the player
        // does.
        assert!(
            serialized.contains("fadeout_ms = 900"),
            "the honoured fade-out duration is written:\n{serialized}"
        );
        assert!(
            serialized.contains("shuffle = true"),
            "the honoured shuffle mode is written:\n{serialized}"
        );
        assert!(
            serialized.contains("repeat = true"),
            "the honoured repeat mode is written:\n{serialized}"
        );

        // When a file *does* contain ignored fields, loading it reports them in warnings.
        let toml_with_ignored = "\
[audio]
volume = 180
replaygain = \"Track\"
device = \"Speakers (Realtek)\"
buffer_ms = 350
fadeout_ms = 900

[playlist]
shuffle = true
repeat = true
manual_advance = true
";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ignored.toml");
        std::fs::write(&path, toml_with_ignored).unwrap();

        let loaded = load(Some(&path)).expect("load succeeds");
        assert_eq!(loaded.config.audio.volume, 180);
        // replaygain is now honoured, so it no longer warns.
        assert_eq!(loaded.config.audio.replaygain, ReplayGainMode::Track);
        // The device is honoured too: it is loaded, and it does not warn.
        assert_eq!(
            loaded.config.audio.device,
            Some("Speakers (Realtek)".to_string())
        );
        assert!(
            loaded.warnings.iter().all(|w| !w.contains("audio.device")),
            "the honoured device must not warn: {:?}",
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
        // The fade-out duration and the playback modes are honoured too, so
        // they do not warn.
        assert_eq!(loaded.config.audio.fadeout_ms, 900);
        assert!(loaded.config.playlist.shuffle);
        assert!(loaded.config.playlist.repeat);
        assert!(
            loaded
                .warnings
                .iter()
                .all(|w| !w.contains("audio.fadeout_ms")),
            "the honoured fade-out duration must not warn: {:?}",
            loaded.warnings
        );
        assert!(
            loaded
                .warnings
                .iter()
                .all(|w| !w.contains("playlist.shuffle") && !w.contains("playlist.repeat")),
            "the honoured playback modes must not warn: {:?}",
            loaded.warnings
        );
        assert!(
            loaded
                .warnings
                .iter()
                .any(|w| w.contains("playlist.manual_advance") && w.contains("not applied")),
            "expected warning for playlist.manual_advance in {:?}",
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
