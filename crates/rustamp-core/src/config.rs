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

/// Output mode of the audio device.
///
/// The WASAPI backend runs either in **shared** mode (the endpoint mixes the
/// player's stream with the other clients of the device) or in **exclusive**
/// mode (the endpoint belongs to the player alone). The backend's own type is
/// `rustamp_output::WasapiMode`; the player maps between the two, because the
/// configuration crate may not depend on the output crate. RFC 0019.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum OutputMode {
    /// Shared mode: the endpoint mixes the player's stream with other clients.
    #[default]
    Shared,
    /// Exclusive mode: the endpoint belongs to the player alone.
    Exclusive,
}

/// Audio settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioConfig {
    /// Output backend name (`"auto"`, `"wasapi"`, `"cpal"`, `"disk"`, ...).
    pub output: String,
    /// Device identifier within the backend; `None` = system default.
    pub device: Option<String>,
    /// Output mode of the device (shared or exclusive).
    pub output_mode: OutputMode,
    /// Output buffer length in ms.
    pub buffer_ms: u32,
    /// Fade-out duration for `StopWithFadeout`, in ms.
    pub fadeout_ms: u32,
    /// Crossfade between two tracks, in ms; `0` is the gapless seam (RFC 0023).
    pub crossfade_ms: u32,
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
            output_mode: OutputMode::Shared,
            buffer_ms: 200,
            fadeout_ms: 500,
            crossfade_ms: 0,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EqConfig {
    /// Last EQ state.
    pub state: EqState,
}

impl Default for EqConfig {
    /// A fresh configuration starts with the equalizer **engaged and flat**.
    ///
    /// The fresh-launch engagement is spec-open (S07 §12 item 13: the On
    /// cell's tie to `use_eq` was never re-verified), and the UI machine's
    /// provisional default and the blessed equalizer renders both show it
    /// engaged. Flat gains are unity, so this colours nothing until a band is
    /// moved. The wire contract's own default ([`EqState::default`],
    /// disengaged) stays what a bare `PlayerState` says without a config.
    fn default() -> Self {
        Self {
            state: EqState {
                enabled: true,
                ..EqState::default()
            },
        }
    }
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
    /// Which module of the active DSP library runs (S09 section 6.3).
    pub dsp_module: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A configuration in which **every** field of **every** configuration
    /// type carries a value that differs from the default.
    ///
    /// Two properties make this the fixture of the round trip below:
    ///
    /// * The struct literals are exhaustive, so a new field cannot be added
    ///   to a configuration struct without this function failing to compile.
    /// * Every value differs from the default, so a field the serialization
    ///   drops cannot hide behind the value the loader falls back to.
    ///
    /// The window list covers every `WindowId` variant (including
    /// `Gen(n)` and `Video`) and both a sized and an unsized window.
    fn every_field_set() -> Config {
        Config {
            version: 42,
            audio: AudioConfig {
                output: "cpal".into(),
                device: Some("Speakers".into()),
                output_mode: OutputMode::Exclusive,
                buffer_ms: 350,
                fadeout_ms: 800,
                crossfade_ms: 2500,
                gapless: false,
                replaygain: ReplayGainMode::Track,
                volume: 77,
                balance: -35,
            },
            ui: UiConfig {
                skin: Some(PathBuf::from("skins/base.wsz")),
                double_size: true,
                always_on_top: true,
                easy_move: false,
                snap_distance: 25,
                windows: vec![
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
                        id: WindowId::Equalizer,
                        x: 10,
                        y: 112,
                        visible: true,
                        shaded: true,
                        width: None,
                        height: None,
                    },
                    WindowLayout {
                        id: WindowId::Playlist,
                        x: 10,
                        y: 228,
                        visible: false,
                        shaded: false,
                        width: Some(275),
                        height: Some(116),
                    },
                    WindowLayout {
                        id: WindowId::Vis,
                        x: 285,
                        y: -4,
                        visible: true,
                        shaded: false,
                        width: Some(275),
                        height: Some(200),
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
                    WindowLayout {
                        id: WindowId::Video,
                        x: 560,
                        y: -4,
                        visible: false,
                        shaded: false,
                        width: Some(352),
                        height: Some(240),
                    },
                ],
            },
            eq: EqConfig {
                state: EqState {
                    bands: [1.0, 2.0, 3.0, 4.5, 5.0, -1.0, -2.0, -3.0, -4.0, -5.0],
                    preamp: 3.5,
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
                plugin_dir: Some(PathBuf::from("custom/plugins")),
                vis_plugin: Some("vis_milk2.dll".into()),
                vis_preset: Some("preset_1.milk".into()),
                disabled: vec!["vis_x.dll".into(), "bad_dsp.dll".into()],
                dsp_module: 1,
            },
        }
    }

    /// The premise of the round trip, made checkable: every field of the
    /// fixture differs from its default.
    ///
    /// A field that stayed at its default could not be distinguished from a
    /// field the serialization drops, because the loader falls back to
    /// exactly that default. The guard makes the premise a fact instead of
    /// an assumption, and it fails loudly when a default changes under the
    /// fixture.
    #[test]
    fn every_field_of_the_configuration_differs_from_its_default() {
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
        assert_ne!(
            c.audio.crossfade_ms, d.audio.crossfade_ms,
            "audio.crossfade_ms"
        );
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

    #[test]
    fn toml_roundtrip() {
        let c = every_field_set();
        let s = toml::to_string(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();

        // Field by field, so a failure names the field that broke. The
        // whole-struct comparison below is the second, independent check.
        assert_eq!(c.version, back.version, "version");
        assert_eq!(c.audio.output, back.audio.output, "audio.output");
        assert_eq!(c.audio.device, back.audio.device, "audio.device");
        assert_eq!(
            c.audio.output_mode, back.audio.output_mode,
            "audio.output_mode"
        );
        assert_eq!(c.audio.buffer_ms, back.audio.buffer_ms, "audio.buffer_ms");
        assert_eq!(
            c.audio.fadeout_ms, back.audio.fadeout_ms,
            "audio.fadeout_ms"
        );
        assert_eq!(
            c.audio.crossfade_ms, back.audio.crossfade_ms,
            "audio.crossfade_ms"
        );
        assert_eq!(c.audio.gapless, back.audio.gapless, "audio.gapless");
        assert_eq!(
            c.audio.replaygain, back.audio.replaygain,
            "audio.replaygain"
        );
        assert_eq!(c.audio.volume, back.audio.volume, "audio.volume");
        assert_eq!(c.audio.balance, back.audio.balance, "audio.balance");

        assert_eq!(c.ui.skin, back.ui.skin, "ui.skin");
        assert_eq!(c.ui.double_size, back.ui.double_size, "ui.double_size");
        assert_eq!(
            c.ui.always_on_top, back.ui.always_on_top,
            "ui.always_on_top"
        );
        assert_eq!(c.ui.easy_move, back.ui.easy_move, "ui.easy_move");
        assert_eq!(
            c.ui.snap_distance, back.ui.snap_distance,
            "ui.snap_distance"
        );
        assert_eq!(c.ui.windows, back.ui.windows, "ui.windows");

        assert_eq!(c.eq.state.bands, back.eq.state.bands, "eq.state.bands");
        assert_eq!(c.eq.state.preamp, back.eq.state.preamp, "eq.state.preamp");
        assert_eq!(
            c.eq.state.enabled, back.eq.state.enabled,
            "eq.state.enabled"
        );
        assert_eq!(c.eq.state.auto, back.eq.state.auto, "eq.state.auto");

        assert_eq!(
            c.playlist.shuffle, back.playlist.shuffle,
            "playlist.shuffle"
        );
        assert_eq!(c.playlist.repeat, back.playlist.repeat, "playlist.repeat");
        assert_eq!(
            c.playlist.manual_advance, back.playlist.manual_advance,
            "playlist.manual_advance"
        );
        assert_eq!(
            c.playlist.last_playlist, back.playlist.last_playlist,
            "playlist.last_playlist"
        );

        assert_eq!(
            c.plugins.plugin_dir, back.plugins.plugin_dir,
            "plugins.plugin_dir"
        );
        assert_eq!(
            c.plugins.vis_plugin, back.plugins.vis_plugin,
            "plugins.vis_plugin"
        );
        assert_eq!(
            c.plugins.vis_preset, back.plugins.vis_preset,
            "plugins.vis_preset"
        );
        assert_eq!(
            c.plugins.disabled, back.plugins.disabled,
            "plugins.disabled"
        );

        assert_eq!(c, back, "the whole configuration survives the round trip");
    }

    #[test]
    fn partial_file_uses_defaults() {
        let c: Config = toml::from_str("[audio]\nvolume = 5\n").unwrap();
        assert_eq!(c.audio.volume, 5);
        assert_eq!(c.audio.buffer_ms, AudioConfig::default().buffer_ms);
        assert_eq!(c.version, CONFIG_VERSION);
    }

    #[test]
    fn the_output_mode_defaults_to_shared_and_survives_a_file() {
        // An older file without the key keeps the default: the field is
        // additive and `AudioConfig` is `#[serde(default)]`.
        let c: Config = toml::from_str("[audio]\nvolume = 5\n").unwrap();
        assert_eq!(c.audio.output_mode, OutputMode::Shared);

        // Every mode survives a write/read cycle.
        for mode in [OutputMode::Shared, OutputMode::Exclusive] {
            let mut config = Config::default();
            config.audio.output_mode = mode;
            let text = toml::to_string(&config).unwrap();
            let back: Config = toml::from_str(&text).unwrap();
            assert_eq!(back.audio.output_mode, mode);
        }
    }
}
