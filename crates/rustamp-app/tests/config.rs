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
use rustamp_core::config::{Config, OutputMode, ReplayGainMode, WindowLayout};
use rustamp_core::state::{EqState, WindowId};
use rustamp_ui::Modifiers;

/// The settings the player does not apply: the loader warns about them, and
/// a save must keep them (F5: a setting is never destroyed by a save that
/// did not apply it).
///
/// `audio.buffer_ms` is the one that has no mapping at all: the output's
/// buffer length is the device's own, so the player cannot apply it.
const UNAPPLIED: &[(&str, &str, &str)] = &[
    ("audio", "output", "\"cpal\""),
    ("audio", "buffer_ms", "300"),
    ("audio", "gapless", "false"),
    ("playlist", "manual_advance", "true"),
    ("playlist", "last_playlist", "\"list.m3u\""),
    ("plugins", "vis_plugin", "\"vis.dll\""),
    ("plugins", "vis_preset", "\"preset\""),
];

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
        "gapless",
        "output",
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
    // The settings the player applies are written: the file says what the
    // player does.
    assert!(
        serialized.lines().any(|line| line == "fadeout_ms = 800"),
        "the honoured fade-out duration is written:\n{serialized}"
    );
    assert!(
        serialized.lines().any(|line| line == "shuffle = true"),
        "the honoured shuffle mode is written:\n{serialized}"
    );
    assert!(
        serialized.lines().any(|line| line == "repeat = true"),
        "the honoured repeat mode is written:\n{serialized}"
    );
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
fadeout_ms = 800

[ui]
snap_distance = 15

[playlist]
shuffle = true
repeat = true
manual_advance = true
";
    let val: toml::Value = toml::from_str(toml_input).unwrap();
    let warnings = ignored_fields_in_toml(&val);
    // replaygain, the fade-out duration and the playback modes are applied:
    // no warning.
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
        warnings.iter().all(|w| !w.contains("audio.fadeout_ms")),
        "the applied fade-out duration must not warn: {warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .all(|w| !w.contains("playlist.shuffle") && !w.contains("playlist.repeat")),
        "the applied playback modes must not warn: {warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("playlist.manual_advance") && w.contains("not applied")),
        "playlist.manual_advance warning missing: {warnings:?}"
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
fn the_configured_device_is_honestly_serialized_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("device.toml");

    let mut config = Config::default();
    config.audio.device = Some("Speakers (Realtek)".to_string());

    let serialized = serialize_honest(&config, None).expect("serialize honest config");
    let value: toml::Value = toml::from_str(&serialized).expect("valid TOML");
    assert_eq!(
        value["audio"]["device"].as_str(),
        Some("Speakers (Realtek)"),
        "the honoured device is written to the file:\n{serialized}"
    );
    assert!(
        ignored_fields_in_toml(&value).is_empty(),
        "an honoured device does not warn: {:?}",
        ignored_fields_in_toml(&value)
    );

    save(&config, None, &path).expect("save config");
    let loaded = load(Some(&path)).expect("load config");
    assert!(
        loaded.warnings.is_empty(),
        "the honoured device does not warn: {:?}",
        loaded.warnings
    );
    assert_eq!(
        loaded.config.audio.device,
        Some("Speakers (Realtek)".to_string()),
        "the device survives the round trip"
    );
    assert_eq!(loaded.config, config);
}

#[test]
fn a_configuration_without_a_device_does_not_advertise_one() {
    let config = Config::default();
    assert_eq!(config.audio.device, None);

    let serialized = serialize_honest(&config, None).expect("serialize honest config");
    assert!(
        !serialized.lines().any(|line| line.starts_with("device =")),
        "an unset device is not written:\n{serialized}"
    );
    let value: toml::Value = toml::from_str(&serialized).expect("valid TOML");
    assert!(value["audio"].get("device").is_none());
    assert!(ignored_fields_in_toml(&value).is_empty());
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
    app.playlist_mouse_down(254, 3, Modifiers::NONE)
        .expect("shade applies");
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

/// W14 finding F5: a setting the player does not apply must survive a save
/// that did not apply it either. The line keeps its text, byte for byte.
#[test]
fn a_setting_the_player_does_not_apply_keeps_its_line_after_a_save() {
    for (table, key, value) in UNAPPLIED {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keep.toml");
        let line = format!("{key} = {value}");
        let text = format!("[{table}]\n{line}\n");
        std::fs::write(&path, &text).unwrap();

        let loaded = load(Some(&path)).expect("the file loads");
        assert!(
            loaded
                .warnings
                .iter()
                .any(|w| w.contains(&format!("{table}.{key}")) && w.contains("not applied")),
            "{table}.{key} must be reported as not applied: {:?}",
            loaded.warnings
        );

        save(&loaded.config, loaded.playlist_font_size, &path).expect("the save succeeds");

        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(
            saved.lines().any(|written| written == line),
            "the line {line:?} must survive the save unchanged:\n{saved}"
        );

        // A second save is stable: the file the first save wrote is read
        // again, and the setting is still there.
        save(&loaded.config, loaded.playlist_font_size, &path).expect("the second save succeeds");
        let saved_again = std::fs::read_to_string(&path).unwrap();
        assert_eq!(saved_again, saved, "a second save changes nothing");

        // And the setting is still reported as not applied: the save kept
        // it, it did not apply it.
        let again = load(Some(&path)).expect("the saved file loads");
        assert!(
            again
                .warnings
                .iter()
                .any(|w| w.contains(&format!("{table}.{key}"))),
            "{table}.{key} must still be reported: {:?}",
            again.warnings
        );
    }
}

/// The settings the player does apply are written by the save, so they
/// round-trip.
#[test]
fn an_applied_setting_survives_a_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("applied.toml");
    std::fs::write(
        &path,
        "\
[audio]
replaygain = \"Track\"
device = \"Speakers (Realtek)\"
output_mode = \"Exclusive\"

[ui]
snap_distance = 25
double_size = true
",
    )
    .unwrap();

    let loaded = load(Some(&path)).expect("the file loads");
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);

    save(&loaded.config, loaded.playlist_font_size, &path).expect("the save succeeds");

    let again = load(Some(&path)).expect("the saved file loads");
    assert_eq!(again.config, loaded.config, "the applied settings survive");
    assert_eq!(again.config.audio.replaygain, ReplayGainMode::Track);
    assert_eq!(again.config.audio.output_mode, OutputMode::Exclusive);
    assert_eq!(
        again.config.audio.device,
        Some("Speakers (Realtek)".to_string())
    );
    assert_eq!(again.config.ui.snap_distance, 25);
    assert!(again.config.ui.double_size);
}

/// A setting the player does not know at all is the user's, and it survives
/// as well.
#[test]
fn an_unknown_setting_survives_a_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unknown.toml");
    std::fs::write(
        &path,
        "\
my_own_key = 42

[my_own_table]
key = \"value\"
",
    )
    .unwrap();

    let loaded = load(Some(&path)).expect("the file loads");
    save(&loaded.config, loaded.playlist_font_size, &path).expect("the save succeeds");

    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.lines().any(|line| line == "my_own_key = 42"),
        "the unknown key survives:\n{saved}"
    );
    assert!(
        saved.contains("[my_own_table]"),
        "the unknown table survives:\n{saved}"
    );
    assert!(
        saved.lines().any(|line| line == "key = \"value\""),
        "the unknown setting survives:\n{saved}"
    );
    // The saved file is still a config file the player can read.
    let again = load(Some(&path)).expect("the saved file loads");
    assert_eq!(again.config, loaded.config);
}

/// The honest list and the warning list must not overlap: a setting the
/// honest serialization writes is never one the loader warns about, and a
/// setting the loader warns about is never one the honest serialization
/// writes.
#[test]
fn the_honest_list_and_the_warning_list_stay_consistent() {
    let mut config = Config::default();
    config.audio.volume = 150;
    config.audio.balance = -40;
    config.audio.replaygain = ReplayGainMode::Album;
    config.audio.device = Some("Speakers".to_string());
    config.audio.output_mode = OutputMode::Exclusive;
    config.audio.fadeout_ms = 750;
    config.ui.skin = Some(PathBuf::from("skins/x.wsz"));
    config.ui.double_size = true;
    config.ui.always_on_top = true;
    config.ui.easy_move = false;
    config.ui.snap_distance = 25;
    config.playlist.shuffle = true;
    config.playlist.repeat = true;
    config.plugins.plugin_dir = Some(PathBuf::from("plugins"));
    config.plugins.disabled = vec!["bad.dll".to_string()];

    let serialized = serialize_honest(&config, Some(18)).expect("serialize honest config");
    let value: toml::Value = toml::from_str(&serialized).expect("valid TOML");
    assert!(
        ignored_fields_in_toml(&value).is_empty(),
        "the honest list must not warn about itself: {:?}",
        ignored_fields_in_toml(&value)
    );
    for (table, key, _) in UNAPPLIED {
        assert!(
            !serialized
                .lines()
                .any(|line| line.starts_with(&format!("{key} ="))),
            "the honest list must not advertise {table}.{key}:\n{serialized}"
        );
    }
    // The newly applied settings are in the honest list.
    for line in ["fadeout_ms = 750", "shuffle = true", "repeat = true"] {
        assert!(
            serialized.lines().any(|written| written == line),
            "the honest list must write '{line}':\n{serialized}"
        );
    }
    // And the other way round: every setting the loader warns about is a
    // setting the player does not own, so a save keeps it.
    for (table, key, value) in UNAPPLIED {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("consistent.toml");
        std::fs::write(&path, format!("[{table}]\n{key} = {value}\n")).unwrap();
        let loaded = load(Some(&path)).expect("the file loads");
        assert!(
            loaded
                .warnings
                .iter()
                .any(|w| w.contains(&format!("{table}.{key}"))),
            "{table}.{key} must be in the warning list"
        );
    }
}

/// A file the player cannot read is never overwritten: the save would
/// destroy the settings the player never understood.
#[test]
fn a_malformed_file_is_not_replaced_by_a_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("malformed.toml");
    let text = "[audio\nvolume = = ";
    std::fs::write(&path, text).unwrap();

    let error = save(&Config::default(), None, &path)
        .expect_err("a malformed file must not be overwritten");
    match &error {
        ConfigError::Malformed {
            path: err_path,
            error: message,
        } => {
            assert_eq!(err_path, &path);
            assert!(!message.is_empty());
        }
        other => panic!("expected ConfigError::Malformed, got: {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        text,
        "the malformed file must stay as it was"
    );
    assert!(error.to_string().contains("is malformed"), "{error}");
}

/// A dotted key names a table. The player's own file writes that table, so
/// the setting moves into it — unchanged, but where TOML allows it.
#[test]
fn a_dotted_setting_moves_into_the_table_the_player_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dotted.toml");
    std::fs::write(&path, "audio.buffer_ms = 300\n").unwrap();

    let loaded = load(Some(&path)).expect("the file loads");
    assert!(
        loaded
            .warnings
            .iter()
            .any(|w| w.contains("audio.buffer_ms")),
        "{:?}",
        loaded.warnings
    );

    save(&loaded.config, loaded.playlist_font_size, &path).expect("the save succeeds");

    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.contains("[audio]"), "{saved}");
    assert!(
        saved.lines().any(|line| line == "buffer_ms = 300"),
        "the setting survives:\n{saved}"
    );
    assert!(
        !saved.contains("audio.buffer_ms"),
        "the dotted key moved into the table:\n{saved}"
    );
    // The saved file is still a config file the player can read.
    load(Some(&path)).expect("the saved file loads");
}

/// A playlist font size the player cannot use is a setting it did not
/// apply: the save keeps it, so the warning keeps naming it.
#[test]
fn an_unusable_playlist_font_size_survives_a_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("font-size.toml");
    std::fs::write(&path, "[ui]\nplaylist_font_size = 900\n").unwrap();

    let loaded = load(Some(&path)).expect("the file loads");
    assert_eq!(loaded.playlist_font_size, None);
    assert!(
        loaded
            .warnings
            .iter()
            .any(|w| w.contains("ui.playlist_font_size")),
        "{:?}",
        loaded.warnings
    );

    save(&loaded.config, loaded.playlist_font_size, &path).expect("the save succeeds");

    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.lines().any(|line| line == "playlist_font_size = 900"),
        "the unusable value survives the save:\n{saved}"
    );
}

/// A setting that cannot be kept must not break the file: when the merged
/// document would not be a config file any more, the save says so and
/// leaves the file alone.
#[test]
fn a_merge_that_would_break_the_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("collide.toml");
    // A root key that collides with the table the player writes.
    let text = "audio = \"not a table\"\n";
    std::fs::write(&path, text).unwrap();

    let error = save(&Config::default(), None, &path).expect_err("the merge must be refused");
    match &error {
        ConfigError::Malformed {
            path: err_path,
            error: message,
        } => {
            assert_eq!(err_path, &path);
            assert!(message.contains("not a config file"), "{message}");
        }
        other => panic!("expected ConfigError::Malformed, got: {other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        text,
        "the file stays as it was"
    );
}

/// A comment that documents a setting the player does not apply travels
/// with it: the user's notes survive the save as well.
#[test]
fn a_comment_above_a_preserved_setting_survives_a_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("comment.toml");
    std::fs::write(
        &path,
        "[playlist]\n# I want manual advance, even if it does not work yet.\nmanual_advance = true\n",
    )
    .unwrap();

    let loaded = load(Some(&path)).expect("the file loads");
    save(&loaded.config, loaded.playlist_font_size, &path).expect("the save succeeds");

    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.contains("# I want manual advance, even if it does not work yet."),
        "the note survives with its setting:\n{saved}"
    );
    assert!(
        saved.lines().any(|line| line == "manual_advance = true"),
        "the setting survives:\n{saved}"
    );
}

/// The clean exit of the app is a save, so it keeps the settings the player
/// does not apply.
#[test]
fn the_clean_exit_keeps_the_settings_the_player_does_not_apply() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clean-exit.toml");
    std::fs::write(
        &path,
        "\
[audio]
volume = 100

[playlist]
manual_advance = true
",
    )
    .unwrap();

    let loaded = load(Some(&path)).expect("the file loads");
    let mut app = test_app_with_config(loaded.config, Some(path.clone()));
    app.submit(Command::SetVolume(188)).unwrap();
    app.save_config().expect("the clean exit saves");

    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.lines().any(|line| line == "manual_advance = true"),
        "the setting the player does not apply survives the clean exit:\n{saved}"
    );
    let again = load(Some(&path)).expect("the saved file loads");
    assert_eq!(
        again.config.audio.volume, 188,
        "the applied setting is written"
    );
}

/// A setting that has no mapping at all stays preserved, and it still warns:
/// the output's buffer length is the device's own, so the player cannot
/// apply it.
#[test]
fn a_setting_without_a_mapping_stays_preserved_and_still_warns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("buffer.toml");
    std::fs::write(&path, "[audio]\nbuffer_ms = 300\n").unwrap();

    let loaded = load(Some(&path)).expect("the file loads");
    assert_eq!(loaded.config.audio.buffer_ms, 300);
    assert!(
        loaded
            .warnings
            .iter()
            .any(|w| w.contains("audio.buffer_ms") && w.contains("not applied")),
        "the setting without a mapping is reported: {:?}",
        loaded.warnings
    );

    save(&loaded.config, loaded.playlist_font_size, &path).expect("the save succeeds");

    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(
        saved.lines().any(|line| line == "buffer_ms = 300"),
        "the setting without a mapping survives the save:\n{saved}"
    );
    let again = load(Some(&path)).expect("the saved file loads");
    assert!(
        again
            .warnings
            .iter()
            .any(|w| w.contains("audio.buffer_ms") && w.contains("not applied")),
        "the setting without a mapping still warns: {:?}",
        again.warnings
    );
}

/// The settings the player applies now are written by the save, so they
/// round-trip: the fade-out duration and the playback modes.
#[test]
fn the_newly_applied_settings_survive_a_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("newly-applied.toml");
    std::fs::write(
        &path,
        "\
[audio]
fadeout_ms = 800

[playlist]
shuffle = true
repeat = true
",
    )
    .unwrap();

    let loaded = load(Some(&path)).expect("the file loads");
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert_eq!(loaded.config.audio.fadeout_ms, 800);
    assert!(loaded.config.playlist.shuffle);
    assert!(loaded.config.playlist.repeat);

    save(&loaded.config, loaded.playlist_font_size, &path).expect("the save succeeds");

    let again = load(Some(&path)).expect("the saved file loads");
    assert_eq!(again.config, loaded.config, "the applied settings survive");
    assert_eq!(again.config.audio.fadeout_ms, 800);
    assert!(again.config.playlist.shuffle);
    assert!(again.config.playlist.repeat);
}
