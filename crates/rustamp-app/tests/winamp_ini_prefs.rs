// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless tests for the import of a Winamp 2.95 `winamp.ini` from the
//! preferences dialog (S12).
//!
//! The preferences page offers the import, the player asks the user for the
//! file and the report of the import is shown **on the page**: these tests
//! walk that path without a window.

use std::path::PathBuf;

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_app::skin::{self, SkinChoice};
use rustamp_core::config::Config;
use rustamp_platform::PrefsAction;

/// The player a test drives: the null output, no window.
fn app_with_config(config: Config) -> App {
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
    App::new(options).expect("build headless app")
}

/// A reference file with one key of each kind: a key the player honours, a
/// key it reads in its own way and a key it has no setting for.
const MIXED_KEYS: &str = "[Winamp]\r\n\
                          volume=200\r\n\
                          pan=-40\r\n\
                          outname=out_wave.dll\r\n";

/// The button on the Import page reaches the app: the app asks the
/// presenter for the file.
#[test]
fn the_import_button_reaches_the_app() {
    let mut app = app_with_config(Config::default());

    app.apply_prefs_action(PrefsAction::ImportWinampIni);

    assert!(
        app.take_winamp_ini_import_request(),
        "the app waits for the file the presenter picks"
    );
    assert!(
        !app.take_winamp_ini_import_request(),
        "the request is taken once"
    );
}

/// A file that is imported reaches the running player, and the page names
/// the file and what the import made of it.
#[test]
fn an_imported_file_is_applied_and_the_page_names_the_counts() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, MIXED_KEYS).unwrap();

    let mut app = app_with_config(Config::default());
    app.apply_prefs_action(PrefsAction::ImportWinampIni);
    app.take_winamp_ini_import_request();

    let outcome = app
        .finish_winamp_ini_import(Some(&ini))
        .expect("the file was chosen");
    assert!(
        matches!(
            outcome,
            rustamp_app::winamp_ini::ImportOutcome::Imported { .. }
        ),
        "{outcome:?}"
    );

    // The settings reached the running player.
    let state = app.engine().state();
    assert_eq!(state.volume, 200);
    assert_eq!(state.balance, -40);

    // The page names the file and the counts.
    let line = app
        .prefs_settings()
        .winamp_ini_report
        .expect("the page shows the report");
    assert!(line.contains("winamp.ini"), "{line}");
    assert!(line.contains("2 applied"), "{line}");
    assert!(line.contains("1 not applied"), "{line}");
    // The honest split survives: the balance is our own reading.
    assert!(line.contains("1 read in our own way"), "{line}");
}

/// A file that cannot be read is reported on the page, and the
/// configuration stays as it was.
#[test]
fn an_unreadable_file_is_reported_and_the_configuration_stays() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("winamp.ini");

    let mut config = Config::default();
    config.audio.volume = 77;
    let mut app = app_with_config(config);

    let outcome = app
        .finish_winamp_ini_import(Some(&missing))
        .expect("the import ran");
    assert!(
        matches!(
            outcome,
            rustamp_app::winamp_ini::ImportOutcome::Failed { .. }
        ),
        "{outcome:?}"
    );

    // The page says that the file was not imported, and names it.
    let line = app
        .prefs_settings()
        .winamp_ini_report
        .expect("the page shows the report");
    assert!(line.contains("not imported"), "{line}");
    assert!(line.contains("winamp.ini"), "{line}");

    // The configuration is as it was.
    assert_eq!(app.engine().state().volume, 77);
    assert_eq!(app.config().audio.volume, 77);
}

/// A cancelled dialog changes nothing: no settings, no report.
#[test]
fn a_cancelled_dialog_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, MIXED_KEYS).unwrap();

    let mut app = app_with_config(Config::default());
    app.apply_prefs_action(PrefsAction::ImportWinampIni);
    app.take_winamp_ini_import_request();

    // The user closed the file dialog.
    let outcome = app.finish_winamp_ini_import(None);
    assert!(outcome.is_none(), "a cancelled import has no outcome");

    // Nothing was imported: the settings are as they were and the page has
    // no report.
    assert_eq!(app.engine().state().volume, Config::default().audio.volume);
    assert_eq!(app.prefs_settings().winamp_ini_report, None);

    // The file is still there and was not read.
    assert!(ini.exists());
}

/// The report of the last import stays on the page until the next import
/// replaces it.
#[test]
fn the_page_keeps_the_report_of_the_last_import() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, MIXED_KEYS).unwrap();

    let mut app = app_with_config(Config::default());
    app.finish_winamp_ini_import(Some(&ini));
    let first = app
        .prefs_settings()
        .winamp_ini_report
        .expect("the page shows the report");

    // The report is still there on the next tick.
    assert_eq!(app.prefs_settings().winamp_ini_report, Some(first.clone()));

    // A cancelled import does not take it away.
    app.finish_winamp_ini_import(None);
    assert_eq!(app.prefs_settings().winamp_ini_report, Some(first));
}

/// The path the app hands to the mapper is the one the user chose.
#[test]
fn the_outcome_names_the_file_the_user_chose() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, "[Winamp]\nvolume=200\n").unwrap();

    let mut app = app_with_config(Config::default());
    let outcome = app
        .finish_winamp_ini_import(Some(&ini))
        .expect("the file was chosen");
    match outcome {
        rustamp_app::winamp_ini::ImportOutcome::Imported { path, report } => {
            assert_eq!(path, PathBuf::from(&ini));
            assert_eq!(report.applied().count(), 1);
        }
        other => panic!("the file was imported: {other:?}"),
    }
}
