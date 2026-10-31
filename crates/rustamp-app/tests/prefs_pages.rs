// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless tests of the preferences pages' own settings (S16): the
//! visualization mode, the snap distance, and the skins and plug-in folders.
//!
//! Every test here runs without a window. The pages themselves are laid out by
//! `rustamp-platform` and pinned there; these tests drive the actions the
//! dialog emits and check what the player does with them, including what a save
//! writes and what a restart reads back.

use std::path::{Path, PathBuf};

use rustamp_app::app::{App, AppOptions, FolderRequest};
use rustamp_app::cli::OutputChoice;
use rustamp_app::config::{self, load};
use rustamp_app::skin::{self, SkinChoice};
use rustamp_core::config::Config;
use rustamp_platform::{PrefsAction, PrefsNode, VIS_MODE_CHOICES};
use rustamp_ui::visualization::VisMode;

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

/// The configuration file's text, for the assertions about what a save wrote.
fn file_text(path: &Path) -> String {
    std::fs::read_to_string(path).expect("the configuration file is there")
}

// ---------------------------------------------------------------------------
// The visualization mode (Options > Visualization)
// ---------------------------------------------------------------------------

#[test]
fn the_visualization_page_sets_the_mode_and_the_choice_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustamp.toml");
    let mut app = app_with(Config::default(), Some(path.clone()));

    // The default is the first row: the spectrum.
    assert_eq!(app.vis_mode(), VisMode::Spectrum);
    assert_eq!(app.prefs_settings().vis_mode_row, 0);

    // The row the list sends is the row the player maps to its own mode.
    app.apply_prefs_action(PrefsAction::SetVisMode(1));
    assert_eq!(app.vis_mode(), VisMode::Scope);
    assert_eq!(app.prefs_settings().vis_mode_row, 1);

    let text = file_text(&path);
    assert!(text.contains("vis_mode = \"scope\""), "{text}");

    // A restart reads the mode back out of the file.
    let loaded = load(Some(&path)).expect("the file loads");
    assert_eq!(loaded.app_ui.vis_mode, VisMode::Scope);
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    let mut restarted = app_with(loaded.config, Some(path.clone()));
    restarted.set_vis_mode(loaded.app_ui.vis_mode);
    assert_eq!(restarted.vis_mode(), VisMode::Scope);

    // Every mode goes round: the file names it and the load reads it.
    for (row, mode) in [
        VisMode::Spectrum,
        VisMode::Scope,
        VisMode::ScopeDots,
        VisMode::Off,
    ]
    .into_iter()
    .enumerate()
    {
        app.apply_prefs_action(PrefsAction::SetVisMode(row));
        let loaded = load(Some(&path)).expect("the file loads");
        assert_eq!(loaded.app_ui.vis_mode, mode, "row {row}");
    }
}

#[test]
fn an_unknown_visualization_mode_is_reported_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustamp.toml");
    std::fs::write(&path, "[ui]\nvis_mode = \"spiral\"\n").unwrap();

    let loaded = load(Some(&path)).expect("the file loads");
    // The player does not know the name: the default stays, and the user is
    // told why.
    assert_eq!(loaded.app_ui.vis_mode, VisMode::Spectrum);
    assert!(
        loaded
            .warnings
            .iter()
            .any(|w| w.contains("ui.vis_mode") && w.contains("not applied")),
        "{:?}",
        loaded.warnings
    );

    // A save must not destroy a setting it did not apply: the line stays.
    let app = app_with(loaded.config, Some(path.clone()));
    app.save_config().expect("the save succeeds");
    let text = file_text(&path);
    assert!(text.contains("vis_mode = \"spiral\""), "{text}");
}

#[test]
fn every_visualization_row_maps_to_and_from_one_mode() {
    // The dialog names the row, the player names the mode: the two orders are
    // pinned together here, because the platform crate cannot name the mode
    // itself.
    let modes = [
        VisMode::Spectrum,
        VisMode::Scope,
        VisMode::ScopeDots,
        VisMode::Off,
    ];
    assert_eq!(modes.len(), VIS_MODE_CHOICES.len(), "one row per mode");
    for (row, mode) in modes.into_iter().enumerate() {
        assert_eq!(rustamp_app::dialogs::vis_mode_row(mode), row, "{mode:?}");
        assert_eq!(rustamp_app::dialogs::vis_mode_at(row), Some(mode), "{row}");
    }
    assert_eq!(
        rustamp_app::dialogs::vis_mode_at(VIS_MODE_CHOICES.len()),
        None
    );
}

// ---------------------------------------------------------------------------
// The snap distance (Options)
// ---------------------------------------------------------------------------

#[test]
fn the_snap_distance_field_changes_the_setting_and_saves_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustamp.toml");
    let mut app = app_with(Config::default(), Some(path.clone()));
    assert_eq!(app.config().ui.snap_distance, 10, "the default");
    assert_eq!(app.prefs_settings().snap_distance_text, "10");

    app.apply_prefs_action(PrefsAction::SetSnapDistance(25));
    assert_eq!(app.config().ui.snap_distance, 25);
    assert_eq!(app.prefs_settings().snap_distance_text, "25");
    let text = file_text(&path);
    assert!(text.contains("snap_distance = 25"), "{text}");

    // 0 is the "no snapping" value and is a setting like any other.
    app.apply_prefs_action(PrefsAction::SetSnapDistance(0));
    assert_eq!(app.config().ui.snap_distance, 0);
    assert!(file_text(&path).contains("snap_distance = 0"));

    let loaded = load(Some(&path)).expect("the file loads");
    assert_eq!(loaded.config.ui.snap_distance, 0);
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
}

// ---------------------------------------------------------------------------
// The skins folder (Skins)
// ---------------------------------------------------------------------------

/// Writes a skin folder (`<dir>/<name>/main.bmp`), which the scan lists.
fn write_skin_folder(dir: &Path, name: &str) -> PathBuf {
    let folder = dir.join(name);
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("main.bmp"), b"not a real sheet").unwrap();
    folder
}

#[test]
fn the_skins_page_changes_the_folder_the_list_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustamp.toml");
    let skins = dir.path().join("my-skins");
    write_skin_folder(&skins, "One");
    let mut app = app_with(Config::default(), Some(path.clone()));

    // The default folder is the one beside the configuration.
    assert_eq!(app.skins_dir(), Some(dir.path().join("skins")));
    assert!(app.prefs_settings().skins_dir_text.contains("skins"));

    // The button only asks: the dialog is the presenter's business.
    app.apply_prefs_action(PrefsAction::ChooseSkinFolder);
    assert_eq!(app.take_folder_request(), Some(FolderRequest::Skins));
    // Asking twice is one answer: the request was taken.
    assert_eq!(app.take_folder_request(), None);

    app.set_skins_folder(skins.clone());
    assert_eq!(app.skins_dir(), Some(skins.clone()));
    let settings = app.prefs_settings();
    assert!(
        settings.skins_dir_text.contains("my-skins"),
        "{}",
        settings.skins_dir_text
    );
    // The list shows what the chosen folder holds.
    assert!(
        settings.skins.iter().any(|row| row.ends_with("One")),
        "{:?}",
        settings.skins
    );
    assert_eq!(settings.report(PrefsNode::Skins), settings.skins_dir_text);

    // The choice is written and comes back after a restart.
    let text = file_text(&path);
    assert!(text.contains("skins_dir"), "{text}");
    let loaded = load(Some(&path)).expect("the file loads");
    assert_eq!(loaded.app_ui.skins_dir.as_deref(), Some(skins.as_path()));
    let mut restarted = app_with(loaded.config, Some(path.clone()));
    restarted.set_skins_dir_from_load(loaded.app_ui.skins_dir);
    assert_eq!(restarted.skins_dir(), Some(skins));
}

// ---------------------------------------------------------------------------
// The plug-in folder (every plug-in page)
// ---------------------------------------------------------------------------

#[test]
fn the_plug_in_pages_change_the_folder_the_scan_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rustamp.toml");
    let plugins = dir.path().join("my-plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    let mut app = app_with(Config::default(), Some(path.clone()));

    app.apply_prefs_action(PrefsAction::ChoosePluginFolder);
    assert_eq!(app.take_folder_request(), Some(FolderRequest::Plugins));

    app.set_plugin_folder(plugins.clone());
    // The folder is read again at once, and the pages say what it holds
    // (nothing, here) and which folder it is.
    let scan = app.plugin_scan();
    assert_eq!(scan.dir, plugins);
    assert!(scan.entries.is_empty(), "{:?}", scan.entries);
    assert!(
        app.prefs_settings()
            .report(PrefsNode::GeneralPurpose)
            .contains("my-plugins"),
        "{:?}",
        app.prefs_settings().report(PrefsNode::GeneralPurpose)
    );

    let text = file_text(&path);
    assert!(text.contains("plugin_dir"), "{text}");
    let loaded = load(Some(&path)).expect("the file loads");
    assert_eq!(
        loaded.config.plugins.plugin_dir.as_deref(),
        Some(plugins.as_path())
    );
}

// ---------------------------------------------------------------------------
// The writer's contract
// ---------------------------------------------------------------------------

#[test]
fn the_config_writer_names_the_visualization_mode_and_the_skins_folder() {
    // The writer's own contract, without a file system in between: a mode and
    // a folder the app owns are written, and the loader reads them back.
    let ui = config::AppUi {
        vis_mode: VisMode::ScopeDots,
        skins_dir: Some(PathBuf::from("C:/players/my-skins")),
        ..config::AppUi::default()
    };
    let text = config::serialize_honest(&Config::default(), &ui).expect("serialize");
    assert!(text.contains("vis_mode = \"scope_dots\""), "{text}");
    assert!(
        text.contains("skins_dir = \"C:/players/my-skins\""),
        "{text}"
    );

    let parsed: toml::Value = toml::from_str(&text).expect("valid TOML");
    assert!(
        config::ignored_fields_in_toml(&parsed).is_empty(),
        "{:?}",
        config::ignored_fields_in_toml(&parsed)
    );
}
