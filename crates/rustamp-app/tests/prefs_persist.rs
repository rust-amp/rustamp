// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless tests that what the native preferences dialog changes reaches the
//! saved configuration and comes back after a restart.

use std::path::PathBuf;

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_app::config::load;
use rustamp_app::skin::{self, SkinChoice};
use rustamp_core::config::Config;
use rustamp_platform::{Bind, PrefsAction};

fn app_with(config: Config, path: Option<PathBuf>) -> App {
    let volume = config.audio.volume;
    let options = AppOptions {
        config,
        skin: skin::load(&SkinChoice::Builtin),
        output: OutputChoice::Null,
        volume,
        media: None,
        seconds: Some(1),
    };
    let mut app = App::new(options).expect("build headless app");
    app.set_config_path(path);
    app
}

#[test]
fn window_settings_changed_in_the_dialog_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefs.toml");

    let mut app = app_with(Config::default(), Some(path.clone()));
    let before = app.prefs_settings();
    assert!(!before.always_on_top);
    assert!(before.easy_move, "easy move is on by default");
    assert!(!before.double_size);

    app.apply_prefs_action(PrefsAction::Set(Bind::AlwaysOnTop, true));
    app.apply_prefs_action(PrefsAction::Set(Bind::EasyMove, false));
    app.apply_prefs_action(PrefsAction::Set(Bind::DoubleSize, true));

    let shown = app.prefs_settings();
    assert!(shown.always_on_top && !shown.easy_move && shown.double_size);

    app.save_config().expect("save");
    let loaded = load(Some(&path)).expect("load").config;
    assert!(loaded.ui.always_on_top);
    assert!(!loaded.ui.easy_move);
    assert!(loaded.ui.double_size);

    // A new run built from the saved file shows the same values in the dialog.
    let restarted = app_with(loaded, None).prefs_settings();
    assert!(restarted.always_on_top && !restarted.easy_move && restarted.double_size);
}

#[test]
fn a_check_box_already_at_its_value_changes_nothing() {
    let mut app = app_with(Config::default(), None);
    let before = app.prefs_settings();
    app.apply_prefs_action(PrefsAction::Set(Bind::AlwaysOnTop, false));
    app.apply_prefs_action(PrefsAction::Set(Bind::EasyMove, true));
    assert_eq!(app.prefs_settings(), before);
}

#[test]
fn shuffle_and_repeat_follow_the_dialog_and_survive_a_restart() {
    // The engine owns the traversal (S15), so the modes are applied and
    // persisted: the saved file names them, and a restart comes up in them.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("modes.toml");

    let mut app = app_with(Config::default(), Some(path.clone()));
    app.apply_prefs_action(PrefsAction::Set(Bind::Shuffle, true));
    app.apply_prefs_action(PrefsAction::Set(Bind::Repeat, true));
    let shown = app.prefs_settings();
    assert!(shown.shuffle && shown.repeat);
    // The engine runs in the modes the dialog set.
    assert!(app.engine().shuffle(), "the engine shuffles");
    assert!(app.engine().repeat(), "the engine repeats");

    app.save_config().expect("save");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("shuffle = true"), "{text}");
    assert!(text.contains("repeat = true"), "{text}");

    // A new run built from the saved file comes up in the same modes.
    let loaded = load(Some(&path)).expect("load");
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert!(loaded.config.playlist.shuffle);
    assert!(loaded.config.playlist.repeat);
    let restarted = app_with(loaded.config, None);
    assert!(restarted.engine().shuffle(), "the engine shuffles again");
    assert!(restarted.engine().repeat(), "the engine repeats again");
}

#[test]
fn choosing_the_built_in_skin_row_keeps_it_selected() {
    let mut app = app_with(Config::default(), None);
    let settings = app.prefs_settings();
    let row = settings
        .skin_selected
        .expect("the skin in use appears in the list");
    app.apply_prefs_action(PrefsAction::LoadSkin(row));
    assert_eq!(app.prefs_settings().skin_selected, Some(row));
}

#[test]
fn the_replaygain_mode_follows_the_dialog_and_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("replaygain.toml");

    let mut app = app_with(Config::default(), Some(path.clone()));
    assert_eq!(
        app.prefs_settings().replaygain,
        rustamp_core::config::ReplayGainMode::Off
    );

    app.apply_prefs_action(PrefsAction::SetReplayGain(
        rustamp_core::config::ReplayGainMode::Album,
    ));
    assert_eq!(
        app.prefs_settings().replaygain,
        rustamp_core::config::ReplayGainMode::Album
    );

    app.save_config().expect("save");
    let loaded = load(Some(&path)).expect("load").config;
    assert_eq!(
        loaded.audio.replaygain,
        rustamp_core::config::ReplayGainMode::Album
    );
}

#[test]
fn the_playlist_title_format_follows_the_dialog_and_survives_a_restart() {
    // The title format (spec S14) is the app's own setting: the dialog
    // carries the switch and the format string, and the app persists both.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("title-format.toml");

    let mut app = app_with(Config::default(), Some(path.clone()));
    let before = app.prefs_settings();
    assert!(
        !before.playlist_title_format,
        "the format is off by default"
    );
    assert_eq!(
        before.playlist_title_format_text,
        rustamp_ui::title_format::DEFAULT_TEMPLATE
    );

    app.apply_prefs_action(PrefsAction::SetTitleFormat(
        "[%artist% - ]%title%".to_string(),
    ));
    app.apply_prefs_action(PrefsAction::Set(Bind::PlaylistTitleFormat, true));
    let shown = app.prefs_settings();
    assert!(shown.playlist_title_format);
    assert_eq!(shown.playlist_title_format_text, "[%artist% - ]%title%");

    // The setting is saved at once, like the other settings of the dialog.
    let loaded = load(Some(&path)).expect("load");
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert_eq!(
        loaded.app_ui.playlist_title_format,
        Some(rustamp_app::title_format::TitleFormatSetting::new(
            true,
            "[%artist% - ]%title%"
        ))
    );

    // A new run built from the saved file shows the same values in the
    // dialog, and the rows are formatted from the start.
    let mut restarted = app_with(loaded.config, None);
    let setting = loaded
        .app_ui
        .playlist_title_format
        .expect("the file names a format");
    restarted
        .set_playlist_title_format(setting.enabled, &setting.format)
        .expect("the format can be used");
    let shown = restarted.prefs_settings();
    assert!(shown.playlist_title_format);
    assert_eq!(shown.playlist_title_format_text, "[%artist% - ]%title%");
}

#[test]
fn the_output_mode_follows_the_dialog_and_survives_a_restart() {
    // Null output makes this a persistence test, not a real exclusive open.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("output_mode.toml");

    let mut app = app_with(Config::default(), Some(path.clone()));
    assert_eq!(
        app.prefs_settings().output_mode,
        rustamp_core::config::OutputMode::Shared
    );

    for mode in [
        rustamp_core::config::OutputMode::Exclusive,
        rustamp_core::config::OutputMode::Shared,
    ] {
        app.apply_prefs_action(PrefsAction::SetOutputMode(mode));
        assert_eq!(app.prefs_settings().output_mode, mode);
        // Applying the dialog action saves immediately, like other settings.
        let loaded = load(Some(&path)).expect("load");
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(loaded.config.audio.output_mode, mode);
        assert_eq!(
            app_with(loaded.config, None).prefs_settings().output_mode,
            mode
        );
        assert_eq!(app.engine().output_name(), "null", "no live device switch");
    }
}
