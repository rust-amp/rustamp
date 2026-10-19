// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless tests for configuration loading, round-trip persistence,
//! honest serialization, unapplied field warnings, and loud failure on malformed files (task card N6).

use std::path::PathBuf;

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_app::config::{
    ConfigError, ConfigSource, ignored_fields_in_toml, load, save, serialize_honest,
};
use rustamp_app::skin::{self, SkinChoice};
use rustamp_core::command::{Command, PrefsPage};
use rustamp_core::config::{Config, ReplayGainMode, WindowLayout};
use rustamp_core::state::{EqState, WindowId};

fn test_app_with_config(config: Config, config_path: Option<PathBuf>) -> App {
    let skin = skin::load(&SkinChoice::Builtin);
    let volume = config.audio.volume;
    let options = AppOptions {
        config,
        skin,
        output: OutputChoice::Null,
        volume,
        file: None,
        seconds: Some(1),
    };
    let mut app = App::new(options).expect("build headless app");
    app.set_config_path(config_path);
    app
}

#[test]
fn config_round_trip_honours_all_supported_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("roundtrip.toml");

    let mut config = Config::default();
    config.audio.volume = 150;
    config.audio.balance = -40;
    config.ui.skin = Some(PathBuf::from("skins/amplifier.wsz"));
    config.ui.double_size = true;
    config.ui.always_on_top = true;
    config.ui.easy_move = false;
    config.ui.windows = vec![
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
    ];
    config.eq.state.enabled = false;
    config.eq.state.auto = true;
    config.eq.state.preamp = 5.0;
    config.eq.state.bands = [1.5, 2.5, 3.5, 4.5, 5.5, -1.5, -2.5, -3.5, -4.5, -5.5];

    save(&config, None, &path).expect("save config");
    assert!(path.exists(), "config file exists on disk");

    let loaded = load(Some(&path)).expect("load config");
    assert_eq!(loaded.source, ConfigSource::File(path));
    assert!(
        loaded.warnings.is_empty(),
        "honest roundtrip emits no warnings: {:?}",
        loaded.warnings
    );

    assert_eq!(loaded.config.audio.volume, 150);
    assert_eq!(loaded.config.audio.balance, -40);
    assert_eq!(
        loaded.config.ui.skin,
        Some(PathBuf::from("skins/amplifier.wsz"))
    );
    assert!(loaded.config.ui.double_size);
    assert!(loaded.config.ui.always_on_top);
    assert!(!loaded.config.ui.easy_move);
    assert_eq!(loaded.config.ui.windows, config.ui.windows);
    assert_eq!(loaded.config.eq.state, config.eq.state);
    assert_eq!(
        loaded.config, config,
        "entire loaded configuration must equal written configuration"
    );
}

#[test]
fn an_ignored_field_is_not_advertised() {
    let mut config = Config::default();
    config.audio.volume = 200;
    config.audio.balance = 0;
    config.audio.replaygain = ReplayGainMode::Album;
    config.audio.buffer_ms = 400;
    config.audio.fadeout_ms = 800;
    config.audio.gapless = false;
    config.audio.output = "cpal".to_string();
    config.audio.device = Some("Speakers".to_string());
    config.ui.snap_distance = 25;
    config.playlist.shuffle = true;
    config.playlist.repeat = true;
    config.playlist.manual_advance = true;
    config.playlist.last_playlist = Some(PathBuf::from("list.m3u"));
    config.plugins.plugin_dir = Some(PathBuf::from("plugins"));
    config.plugins.vis_plugin = Some("vis_milk2.dll".to_string());
    config.plugins.vis_preset = Some("preset.milk".to_string());
    config.plugins.disabled = vec!["bad.dll".to_string()];

    let serialized = serialize_honest(&config, None).expect("serialize honest config");

    let ignored_names = [
        "buffer_ms",
        "fadeout_ms",
        "gapless",
        "output",
        "device",
        "shuffle",
        "repeat",
        "manual_advance",
        "last_playlist",
        "vis_plugin",
        "vis_preset",
    ];

    for name in ignored_names {
        assert!(
            // Match keys, not substrings: supported `output_mode` is not `output`.
            !serialized
                .lines()
                .any(|line| line.starts_with(&format!("{name} ="))),
            "serialized honest config advertises ignored setting '{name}':\n{serialized}"
        );
    }
    assert_eq!(
        rustamp_app::config::parse(&serialized)
            .unwrap()
            .ui
            .snap_distance,
        25
    );

    // Conversely, if a user writes unapplied fields into a config file,
    // the loader explicitly identifies them with descriptive warnings.
    let toml_input = "\
[audio]
volume = 190
replaygain = \"Track\"
buffer_ms = 300
gapless = false

[ui]
snap_distance = 15

[playlist]
shuffle = true
repeat = true
";
    let val: toml::Value = toml::from_str(toml_input).unwrap();
    let warnings = ignored_fields_in_toml(&val);
    // replaygain is now honoured: no warning.
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("audio.buffer_ms") && w.contains("not applied")),
        "audio.buffer_ms warning missing: {warnings:?}"
    );
    assert!(
        warnings.iter().all(|w| !w.contains("ui.snap_distance")),
        "supported snap distance must not warn: {warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("playlist.shuffle") && w.contains("not applied")),
        "playlist.shuffle warning missing: {warnings:?}"
    );
}

#[test]
fn both_output_modes_are_honestly_serialized_without_ignored_field_warnings() {
    use rustamp_core::config::OutputMode;

    for mode in [OutputMode::Shared, OutputMode::Exclusive] {
        let mut config = Config::default();
        config.audio.output_mode = mode;
        let serialized = serialize_honest(&config, None).expect("serialize");
        let value: toml::Value = toml::from_str(&serialized).expect("valid TOML");
        assert!(value["audio"].get("output_mode").is_some());
        assert!(ignored_fields_in_toml(&value).is_empty());
        assert_eq!(
            rustamp_app::config::parse(&serialized)
                .unwrap()
                .audio
                .output_mode,
            mode
        );
    }
}

#[test]
fn a_malformed_file_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("malformed.toml");
    std::fs::write(&path, "[audio\nvolume = invalid = = ").unwrap();

    let result = load(Some(&path));
    assert!(result.is_err(), "malformed config must fail loudly");

    let err = result.unwrap_err();
    match &err {
        ConfigError::Malformed {
            path: err_path,
            error,
        } => {
            assert_eq!(err_path, &path);
            assert!(!error.is_empty());
        }
        other => panic!("expected ConfigError::Malformed, got: {other:?}"),
    }

    let report = err.to_string();
    assert!(
        report.contains("malformed.toml"),
        "error report must include path: {report}"
    );
    assert!(
        report.contains("is malformed"),
        "error report must state 'is malformed': {report}"
    );
}

#[test]
fn app_saves_config_on_clean_exit_headless() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("saved_exit.toml");

    let mut config = Config::default();
    config.audio.volume = 100;
    config.audio.balance = 0;

    let mut app = test_app_with_config(config, Some(config_path.clone()));

    // Submit volume and balance changes.
    app.submit(Command::SetVolume(188)).unwrap();
    app.submit(Command::SetBalance(-25)).unwrap();
    app.submit(Command::ToggleAlwaysOnTop).unwrap();
    app.submit(Command::ToggleDoubleSize).unwrap();

    // Clean exit in headless mode calls save_config.
    app.save_config().expect("save config on exit");
    assert!(config_path.exists(), "config file was written on exit");

    // Re-read file and assert changes took effect.
    let loaded = load(Some(&config_path)).expect("load saved config");
    assert_eq!(loaded.config.audio.volume, 188);
    assert_eq!(loaded.config.audio.balance, -25);
    assert!(loaded.config.ui.always_on_top);
    assert!(loaded.config.ui.double_size);
}

#[test]
fn app_restores_eq_state_and_window_layouts_from_config() {
    let mut config = Config::default();
    config.audio.volume = 210;
    config.eq.state = EqState {
        enabled: true,
        auto: true,
        preamp: 7.5,
        bands: [2.0, 3.0, 4.0, 5.0, 6.0, -1.0, -2.0, -3.0, -4.0, -5.0],
    };
    config.ui.windows = vec![
        WindowLayout {
            id: WindowId::Main,
            x: 88,
            y: 99,
            visible: true,
            shaded: true,
            width: None,
            height: None,
        },
        WindowLayout {
            id: WindowId::Equalizer,
            x: 88,
            y: 215,
            visible: true,
            shaded: false,
            width: None,
            height: None,
        },
        WindowLayout {
            id: WindowId::Playlist,
            x: 88,
            y: 331,
            visible: false,
            shaded: false,
            width: None,
            height: None,
        },
    ];

    let app = test_app_with_config(config, None);

    // Assert engine received the EQ state:
    let engine_eq = app.engine().eq();
    assert!(engine_eq.enabled);
    assert!(engine_eq.auto);
    assert!((engine_eq.preamp - 7.5).abs() < 1e-4);
    assert!((engine_eq.bands[0] - 2.0).abs() < 1e-4);
    assert!((engine_eq.bands[4] - 6.0).abs() < 1e-4);

    // Assert the UI equalizer window state is synced:
    let ui_eq = app.eq_state();
    assert!(ui_eq.enabled);
    assert!(ui_eq.auto);
    // The window holds classic units `0..=63`, and the law is anchored at
    // `0 = +12 dB`, `31 = 0 dB`, `63 = -12 dB` (RFC 0011, S09), so a *boosted*
    // preamp sits *below* the flat centre: +7.5 dB is 11, not above 31.
    assert_eq!(
        ui_eq.preamp,
        rustamp_core::state::eq_classic_from_db(7.5) as u8
    );
    assert!(
        ui_eq.preamp < 31,
        "a boosted preamp sits above the flat centre"
    );

    // Assert window layout flags:
    assert!(app.state().windowshade, "main window is shaded from config");
    assert!(app.eq_open(), "equalizer window is visible from config");
    assert!(
        !app.playlist_open(),
        "playlist window is hidden from config"
    );
}

/// The playlist editor's windowshade is saved with the other windows
/// (S01 §1: the shade band is 14 px tall), so a shaded editor comes back
/// shaded - and the live state is written back out.
#[test]
fn the_playlist_editors_windowshade_round_trips_through_the_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("playlist-shade.toml");

    let mut app = test_app_with_config(Config::default(), Some(path.clone()));
    assert!(!app.playlist_shaded(), "a fresh editor is not shaded");
    assert_eq!(app.playlist_frame_size(), (275, 116));

    // The title-bar shade button collapses the editor ...
    app.playlist_mouse_down(254, 3).expect("shade applies");
    assert!(app.playlist_shaded());
    assert_eq!(app.playlist_frame_size(), (275, 14));

    // ... and the state survives the save.
    app.save_config().expect("save config");
    let loaded = load(Some(&path)).expect("load config");
    let playlist = loaded
        .config
        .ui
        .windows
        .iter()
        .find(|w| w.id == WindowId::Playlist)
        .expect("the editor's layout is saved");
    assert!(playlist.shaded, "the shaded editor is saved shaded");

    // A new player built from that file comes up shaded.
    let restarted = test_app_with_config(loaded.config, None);
    assert!(
        restarted.playlist_shaded(),
        "a saved shaded editor comes back shaded"
    );
    assert_eq!(restarted.playlist_frame_size(), (275, 14));
}

/// The equalizer's windowshade is saved with the other windows (S01 §1: the
/// shade band is 14 px tall), so a shaded equalizer comes back shaded - and
/// the live state is written back out.
#[test]
fn the_equalizers_windowshade_round_trips_through_the_config() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eq-shade.toml");

    let mut app = test_app_with_config(Config::default(), Some(path.clone()));
    assert!(
        !app.eq_state().windowshade,
        "a fresh equalizer is not shaded"
    );
    assert_eq!(app.eq_frame_size(), (275, 116));

    // The windowshade command collapses the equalizer ...
    app.submit(Command::ToggleWindowShade(WindowId::Equalizer))
        .unwrap();
    assert!(app.eq_state().windowshade);
    assert_eq!(app.eq_frame_size(), (275, 14));

    // ... and the state survives the save.
    app.save_config().expect("save config");
    let loaded = load(Some(&path)).expect("load config");
    let equalizer = loaded
        .config
        .ui
        .windows
        .iter()
        .find(|w| w.id == WindowId::Equalizer)
        .expect("the equalizer's layout is saved");
    assert!(equalizer.shaded, "the shaded equalizer is saved shaded");

    // A new player built from that file comes up shaded.
    let restarted = test_app_with_config(loaded.config, None);
    assert!(
        restarted.eq_state().windowshade,
        "a saved shaded equalizer comes back shaded"
    );
    assert_eq!(restarted.eq_frame_size(), (275, 14));
}

#[test]
fn preferences_dialog_open_page_declares_settings_not_applied() {
    let config = Config::default();
    let mut app = test_app_with_config(config, None);

    app.submit(Command::OpenPrefs).unwrap();
    let lines = app.open_dialog_lines().expect("preferences dialog open");

    assert!(
        lines.iter().any(|line| line.contains("not applied yet")),
        "preferences dialog must declare settings are not applied yet, got: {lines:?}"
    );

    app.submit(Command::OpenPrefsPage(PrefsPage::VisOptions))
        .unwrap();
    let lines = app.open_dialog_lines().expect("vis options open");
    assert!(
        lines.iter().any(|line| line.contains("not applied yet")),
        "preferences options page must declare settings are not applied yet, got: {lines:?}"
    );
}
