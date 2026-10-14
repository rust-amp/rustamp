//! Typed configuration root.
//!
//! Only data definitions and defaults live here; TOML file I/O,
//! `winamp.ini` import and plugin-facing ini emulation come later. Every
//! struct uses `#[serde(default)]` so older config files keep loading when
//! fields are added.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::state::{EqState, WindowId};

/// Current config schema version.
pub const CONFIG_VERSION: u32 = 1;

/// Root configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Schema version for migrations.
    pub version: u32,
    /// Audio engine and output settings.
    pub audio: AudioConfig,
    /// Window and skin settings.
    pub ui: UiConfig,
    /// Equalizer settings.
    pub eq: EqConfig,
    /// Playlist behavior.
    pub playlist: PlaylistConfig,
    /// Plugin settings.
    pub plugins: PluginsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: CONFIG_VERSION,
            audio: AudioConfig::default(),
            ui: UiConfig::default(),
            eq: EqConfig::default(),
            playlist: PlaylistConfig::default(),
            plugins: PluginsConfig::default(),
        }
    }
}

/// ReplayGain mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ReplayGainMode {
    /// Disabled.
    #[default]
    Off,
    /// Use track gain.
    Track,
    /// Use album gain.
    Album,
}

/// Audio settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Output backend name (`"auto"`, `"wasapi"`, `"cpal"`, `"disk"`, ...).
    pub output: String,
    /// Device identifier within the backend; `None` = system default.
    pub device: Option<String>,
    /// Output buffer length in ms.
    pub buffer_ms: u32,
    /// Fade-out duration for `StopWithFadeout`, in ms.
    pub fadeout_ms: u32,
    /// Gapless playback.
    pub gapless: bool,
    /// ReplayGain mode.
    pub replaygain: ReplayGainMode,
    /// Last volume `0..=255`.
    pub volume: u8,
    /// Last balance `-127..=127`.
    pub balance: i8,
}

impl Default for AudioConfig {
    fn default() -> Self {
        AudioConfig {
            output: "auto".into(),
            device: None,
            buffer_ms: 200,
            fadeout_ms: 500,
            gapless: true,
            replaygain: ReplayGainMode::Off,
            volume: 200,
            balance: 0,
        }
    }
}

/// Saved placement of one window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowLayout {
    /// Which window.
    pub id: WindowId,
    /// Screen X of the top-left corner.
    pub x: i32,
    /// Screen Y of the top-left corner.
    pub y: i32,
    /// Visible.
    pub visible: bool,
    /// Windowshade mode.
    pub shaded: bool,
    /// Width in unscaled pixels for resizable windows.
    pub width: Option<u32>,
    /// Height in unscaled pixels for resizable windows.
    pub height: Option<u32>,
}

/// UI settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// Skin path; `None` = built-in default skin.
    pub skin: Option<PathBuf>,
    /// Double-size mode.
    pub double_size: bool,
    /// Always on top.
    pub always_on_top: bool,
    /// Drag windows from anywhere ("easy move").
    pub easy_move: bool,
    /// Window snapping distance in pixels (0 = off).
    pub snap_distance: u32,
    /// Saved window layouts.
    pub windows: Vec<WindowLayout>,
}

impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            skin: None,
            double_size: false,
            always_on_top: false,
            easy_move: true,
            snap_distance: 10,
            windows: Vec::new(),
        }
    }
}

/// Equalizer settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EqConfig {
    /// Last EQ state.
    pub state: EqState,
}

/// Playlist settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaylistConfig {
    /// Shuffle.
    pub shuffle: bool,
    /// Repeat.
    pub repeat: bool,
    /// Do not advance automatically at track end.
    pub manual_advance: bool,
    /// Playlist file restored at startup.
    pub last_playlist: Option<PathBuf>,
}

/// Plugin settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginsConfig {
    /// Directory scanned for classic plugins.
    pub plugin_dir: Option<PathBuf>,
    /// Selected visualization plugin (file name).
    pub vis_plugin: Option<String>,
    /// Selected visualization preset.
    pub vis_preset: Option<String>,
    /// Plugin file names that must not be loaded.
    pub disabled: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_roundtrip() {
        let mut c = Config::default();
        c.audio.volume = 77;
        c.audio.device = Some("Speakers".into());
        c.ui.skin = Some(PathBuf::from("skins/base.wsz"));
        c.eq.state.bands[3] = 4.5;
        c.eq.state.enabled = true;
        c.ui.windows = vec![
            WindowLayout {
                id: WindowId::Main,
                x: 10,
                y: -4,
                visible: true,
                shaded: false,
                width: None,
                height: None,
            },
            WindowLayout {
                id: WindowId::Gen(3),
                x: 0,
                y: 0,
                visible: false,
                shaded: true,
                width: Some(275),
                height: Some(232),
            },
        ];
        c.plugins.disabled = vec!["vis_x.dll".into()];
        let s = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(c, back);
    }

    #[test]
    fn partial_file_uses_defaults() {
        let c: Config = toml::from_str("[audio]\nvolume = 5\n").unwrap();
        assert_eq!(c.audio.volume, 5);
        assert_eq!(c.audio.buffer_ms, AudioConfig::default().buffer_ms);
        assert_eq!(c.version, CONFIG_VERSION);
    }
}
