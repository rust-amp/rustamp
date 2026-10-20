// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless tests for the import of a Winamp 2.95 `winamp.ini` (S12).
//!
//! The import is an explicit action: it reads the reference file, takes over
//! the settings the player honours and reports everything else. The source
//! file is never written to.

use std::io::Write;
use std::path::{Path, PathBuf};

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_app::skin::{self, SkinChoice};
use rustamp_core::config::Config;
use rustamp_core::state::WindowId;

/// The player a test drives: the null output, no window.
fn app_with_config(config: Config, config_path: Option<PathBuf>) -> App {
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

/// Writes a `.wsz` archive that holds a `main.bmp`, so the skins folder
/// takes it for a skin.
fn write_skin_archive(path: &Path) {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file(
            "main.bmp",
            zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"not a real sheet").unwrap();
    let bytes = writer.finish().unwrap().into_inner();
    std::fs::write(path, bytes).unwrap();
}

/// The reference file the tests import: the keys the mapping table names.
const KNOWN_KEYS: &str = "[Winamp]\r\n\
                          volume=200\r\n\
                          pan=-40\r\n\
                          use_eq=1\r\n\
                          autoload_eq=0\r\n\
                          preamp=63\r\n\
                          eq_data=0,63,31,31,31,31,31,31,31,31\r\n\
                          shuffle=1\r\n\
                          repeat=1\r\n\
                          pe_fontsize=14\r\n\
                          aot=1\r\n\
                          dsize=1\r\n\
                          easymove=0\r\n";

/// The known keys reach the running player: the engine, the window flags and
/// the playlist editor's font size.
#[test]
fn the_known_keys_reach_the_running_player() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, KNOWN_KEYS).unwrap();

    let mut app = app_with_config(Config::default(), Some(dir.path().join("rustamp.toml")));
    let report = app.import_winamp_ini(&ini).expect("the file is imported");

    // The engine took the audio settings and the playback modes over.
    let state = app.engine().state();
    assert_eq!(state.volume, 200);
    assert_eq!(state.balance, -40);
    assert!(state.shuffle);
    assert!(state.repeat);

    // The equalizer: the engine is the truth, the window follows it.
    assert!(state.eq.enabled);
    assert!(!state.eq.auto);
    assert_eq!(state.eq.preamp, rustamp_ui::saved_to_gain_db(63));
    assert_eq!(state.eq.bands[0], rustamp_ui::saved_to_gain_db(0));
    assert_eq!(state.eq.bands[1], rustamp_ui::saved_to_gain_db(63));
    assert_eq!(state.eq.bands[2], rustamp_ui::saved_to_gain_db(31));

    // The window flags.
    assert!(app.always_on_top());
    assert!(!app.easy_move());
    assert!(app.state().double_size);

    // The playlist editor's font size.
    assert_eq!(app.playlist_font_size(), Some(14));

    // Every key of the file was applied, and the report says so.
    assert_eq!(report.not_applied().count(), 0, "{:?}", report.lines());
    assert!(report.applied_key("volume"));
    assert!(report.applied_key("eq_data"));
}

/// The import only ever reads: a file it cannot write to is imported just
/// like any other.
#[test]
fn the_source_file_is_never_written_to() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    let text = "[Winamp]\nvolume=200\n";
    std::fs::write(&ini, text).unwrap();
    // A read-only file proves the point: an import that wrote to it would
    // fail instead of importing.
    let mut permissions = std::fs::metadata(&ini).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&ini, permissions).unwrap();

    let mut app = app_with_config(Config::default(), None);
    app.import_winamp_ini(&ini).expect("the file is imported");

    assert_eq!(std::fs::read_to_string(&ini).unwrap(), text);
    assert_eq!(app.engine().state().volume, 200);
}

/// A key the player has no setting for is reported, and the keys that can be
/// honoured still land.
#[test]
fn a_key_the_player_cannot_honour_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, "[Winamp]\nvolume=200\noutname=out_wave.dll\npilp=0\n").unwrap();

    let mut app = app_with_config(Config::default(), None);
    let report = app.import_winamp_ini(&ini).expect("the file is imported");

    assert!(report.applied_key("volume"));
    assert!(!report.applied_key("outname"));
    let reason = report.reason_for("outname").expect("a reason");
    assert!(reason.contains("command line"), "{reason}");
    assert!(!report.applied_key("pilp"));
    assert_eq!(app.engine().state().volume, 200);
}

/// A line the parser cannot read is reported and does not stop the import.
#[test]
fn a_malformed_line_does_not_stop_the_import() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(
        &ini,
        "[Winamp]\nthis line has no equals sign\nvolume=200\n[Winamp\nshuffle=1\n",
    )
    .unwrap();

    let mut app = app_with_config(Config::default(), None);
    let report = app.import_winamp_ini(&ini).expect("the file is imported");

    assert_eq!(report.problems.len(), 2, "{:?}", report.problems);
    assert_eq!(app.engine().state().volume, 200);
    assert!(app.engine().state().shuffle);
}

/// A key the file does not carry leaves the setting as it was.
#[test]
fn a_missing_key_keeps_the_current_value() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, "[Winamp]\nrepeat=1\n").unwrap();

    let mut config = Config::default();
    config.audio.volume = 77;
    config.playlist.shuffle = true;

    let mut app = app_with_config(config, None);
    let report = app.import_winamp_ini(&ini).expect("the file is imported");

    let state = app.engine().state();
    assert_eq!(state.volume, 77, "the volume the file does not name stays");
    assert!(state.shuffle, "the shuffle the file does not name stays");
    assert!(state.repeat, "the key the file names is applied");
    assert!(!report.applied_key("volume"));
}

/// The reference names the skin by file name; the import finds it in the
/// skins folder beside our own configuration.
#[test]
fn the_skin_is_resolved_in_the_skins_folder_beside_the_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let skins = dir.path().join("skins");
    std::fs::create_dir_all(&skins).unwrap();
    write_skin_archive(&skins.join("base-2.91.wsz"));

    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, "[Winamp]\nskin=base-2.91.wsz\n").unwrap();

    let mut app = app_with_config(Config::default(), Some(dir.path().join("rustamp.toml")));
    let report = app.import_winamp_ini(&ini).expect("the file is imported");

    assert!(report.applied_key("skin"), "{:?}", report.lines());
    assert_eq!(app.config().ui.skin, Some(skins.join("base-2.91.wsz")));
}

/// A skin our folder does not hold is reported, not guessed.
#[test]
fn a_skin_the_folder_does_not_hold_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, "[Winamp]\nskin=base-2.91.wsz\n").unwrap();

    let mut app = app_with_config(Config::default(), Some(dir.path().join("rustamp.toml")));
    let report = app.import_winamp_ini(&ini).expect("the file is imported");

    assert!(!report.applied_key("skin"));
    let reason = report.reason_for("skin").expect("a reason");
    assert!(reason.contains("skins folder"), "{reason}");
    assert_eq!(app.config().ui.skin, None);
}

/// The band values are clamped into the slider's own scale, and the parse
/// stops at the first entry it cannot read (S12 §6).
#[test]
fn the_band_values_are_clamped_and_the_parse_stops_where_the_spec_says() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    // 64 becomes 63, -5 becomes 0, and the empty entry after the second
    // value stops the parse: the third band keeps the value it had.
    std::fs::write(&ini, "[Winamp]\neq_data=64,-5,,31\n").unwrap();

    let mut app = app_with_config(Config::default(), None);
    app.import_winamp_ini(&ini).expect("the file is imported");

    let eq = app.engine().state().eq;
    assert_eq!(eq.bands[0], rustamp_ui::saved_to_gain_db(63));
    assert_eq!(eq.bands[1], rustamp_ui::saved_to_gain_db(0));
    assert_eq!(
        eq.bands[2], 0.0,
        "the band the parse never reached keeps the value it had"
    );
}

/// The window placement lands in the configuration, where the player
/// restores it when it starts.
#[test]
fn the_window_placement_lands_in_the_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, "[Winamp]\nwx=100\nwy=200\neq_wx=10\neq_wy=20\n").unwrap();

    let mut app = app_with_config(Config::default(), None);
    let report = app.import_winamp_ini(&ini).expect("the file is imported");

    assert!(report.applied_key("wx"));
    let windows = &app.config().ui.windows;
    let main = windows
        .iter()
        .find(|window| window.id == WindowId::Main)
        .expect("the main window");
    assert_eq!((main.x, main.y), (100, 200));
    let equalizer = windows
        .iter()
        .find(|window| window.id == WindowId::Equalizer)
        .expect("the equalizer");
    assert_eq!((equalizer.x, equalizer.y), (10, 20));
}

/// `mw_open` hides or shows the main window in the running player.
#[test]
fn the_main_windows_visibility_reaches_the_running_player() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, "[Winamp]\nmw_open=0\n").unwrap();

    let mut app = app_with_config(Config::default(), None);
    app.import_winamp_ini(&ini).expect("the file is imported");

    assert!(!app.main_visible(), "the main window is hidden");
}

/// The imported settings reach our own configuration file, which is what the
/// player saves on clean exit.
#[test]
fn the_imported_settings_reach_our_configuration_file() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, "[Winamp]\nvolume=200\nshuffle=1\n").unwrap();
    let config_path = dir.path().join("rustamp.toml");

    let mut app = app_with_config(Config::default(), Some(config_path.clone()));
    app.import_winamp_ini(&ini).expect("the file is imported");
    app.save_config().expect("the configuration is saved");

    let saved = std::fs::read_to_string(&config_path).expect("the file is there");
    assert!(saved.contains("volume = 200"), "{saved}");
    assert!(saved.contains("shuffle = true"), "{saved}");
}
