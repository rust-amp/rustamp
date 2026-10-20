// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Hostile-input and property tests for the import of a Winamp 2.95
//! `winamp.ini` (S12).
//!
//! The mapping tests (`tests/winamp_ini.rs`, `tests/winamp_ini_prefs.rs`) and
//! the spec conformance suite (`tests/s12_conformance.rs`) show that a
//! well-formed file maps as S12 says. This suite asks the other question: what
//! happens to a file the reference would never have written - a binary renamed
//! `.ini`, a file that is only a byte order mark, UTF-16, a megabyte on one
//! line, NUL bytes, a repeated section. Only two answers are allowed: the
//! import refuses the value **visibly**, or it maps it by a documented, spec-
//! named rule. It must never panic and never write a nonsense setting silently.
//!
//! Two properties run through every case:
//!
//! * the source file is only ever read - its bytes are unchanged afterwards,
//!   even when it is read-only;
//! * importing the same file twice yields the same configuration.
//!
//! The suite is headless: it drives the import's pure seam (`plan`, the app's
//! own `import_winamp_ini`) with the null output; no window is opened.

use std::path::{Path, PathBuf};

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_app::skin::{self, SkinChoice};
use rustamp_app::winamp_ini::{self, ImportError, ImportOutcome, IniFile, plan as import_plan};
use rustamp_core::config::{Config, WindowLayout};
use rustamp_core::state::WindowId;

/// A resolver that finds no skin: the skins folder holds none.
fn no_skin(_name: &str) -> Option<PathBuf> {
    None
}

/// The player a test drives: the null output, no window.
fn app_with_config(config: Config, config_path: Option<PathBuf>) -> App {
    let skin = skin::load(&SkinChoice::Builtin);
    let volume = config.audio.volume;
    let options = AppOptions {
        config,
        skin,
        output: OutputChoice::Null,
        volume,
        media: None,
        seconds: Some(1),
    };
    let mut app = App::new(options).expect("build headless app");
    app.set_config_path(config_path);
    app
}

/// The layout of the window `id` in a configuration.
fn window(config: &Config, id: WindowId) -> &WindowLayout {
    config
        .ui
        .windows
        .iter()
        .find(|window| window.id == id)
        .unwrap_or_else(|| panic!("the configuration has a layout for {id:?}"))
}

// ---------------------------------------------------------------------------
// Text that is valid UTF-8 but not the shape S12 §2 describes
// ---------------------------------------------------------------------------

/// A UTF-8 byte order mark is not part of the file's text.
///
/// Windows editors write one in front of the first line. If the parser keeps
/// it, the *first section header* stops being a header: every key of the file
/// lands in the unnamed section and is refused as "the key is not in a
/// section", so a perfectly good file imports nothing.
#[test]
fn a_byte_order_mark_does_not_hide_the_players_section() {
    let text = "\u{feff}[Winamp]\r\nvolume=200\r\nshuffle=1\r\n";

    let file = IniFile::parse(text);
    assert!(file.problems.is_empty(), "{:?}", file.problems);
    let player = file.section("Winamp").expect("the player's own section");
    assert_eq!(player.get("volume"), Some("200"));

    let plan = import_plan(text, &Config::default(), None, &no_skin);
    assert_eq!(plan.config.audio.volume, 200);
    assert!(plan.config.playlist.shuffle);
    assert_eq!(
        plan.report.not_applied().count(),
        0,
        "{:?}",
        plan.report.lines()
    );
}

/// A file whose bytes are only a byte order mark holds no key and changes
/// nothing - the import says so instead of inventing a setting.
#[test]
fn a_file_that_is_only_a_byte_order_mark_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, "\u{feff}").unwrap();

    let plan = winamp_ini::import_file(&ini, &Config::default(), None, &no_skin)
        .expect("a byte order mark is text");
    assert_eq!(plan.config, Config::default());
    assert!(plan.report.keys.is_empty(), "{:?}", plan.report.lines());
}

/// A repeated `[Winamp]` header is one section, not two.
///
/// The module's documented rule is that a key which appears twice keeps its
/// **first** value and the repetition is reported. Two `[Winamp]` headers are
/// how a hand-edited file spells exactly that, and the rule has to hold there
/// too - otherwise the *last* section wins silently and nothing is reported.
#[test]
fn a_repeated_section_keeps_the_first_value_and_is_reported() {
    let text = "[Winamp]\nvolume=200\n[Winamp]\nvolume=10\nshuffle=1\n";
    let plan = import_plan(text, &Config::default(), None, &no_skin);

    assert_eq!(
        plan.config.audio.volume, 200,
        "the later section overwrote the earlier value"
    );
    assert!(
        plan.config.playlist.shuffle,
        "a key only the later section carries was lost"
    );
    // Two notes: the repeated header and the key the two headers share.
    assert_eq!(plan.report.problems.len(), 2, "{:?}", plan.report.problems);
    assert!(
        plan.report.problems[0].contains("Winamp")
            && plan.report.problems[0].contains("appears more than once"),
        "{:?}",
        plan.report.problems
    );
    assert!(
        plan.report.problems[1].contains("first value"),
        "{:?}",
        plan.report.problems
    );

    // The header is matched without regard to case, so `[WINAMP]` is the same
    // section as `[Winamp]`.
    let mixed = import_plan(
        "[Winamp]\nvolume=200\n[WINAMP]\nvolume=10\n",
        &Config::default(),
        None,
        &no_skin,
    );
    assert_eq!(mixed.config.audio.volume, 200);
    assert_eq!(mixed.report.keys.len(), 1, "{:?}", mixed.report.keys);
    assert_eq!(
        mixed.report.problems.len(),
        2,
        "{:?}",
        mixed.report.problems
    );
}

/// The section name and the key names are matched without regard to case
/// (S12 §4.3: the file is read through the profile API).
#[test]
fn the_section_and_the_keys_are_read_without_regard_to_case() {
    let plan = import_plan(
        "[WINAMP]\nVOLUME=200\nShuffle=1\n",
        &Config::default(),
        None,
        &no_skin,
    );
    assert_eq!(plan.config.audio.volume, 200);
    assert!(plan.config.playlist.shuffle);

    // `Volume`, `volume` and `VOLUME` are one key: the first value wins and
    // every repetition is reported.
    let repeated = import_plan(
        "[Winamp]\nVolume=200\nvolume=10\nVOLUME=20\n",
        &Config::default(),
        None,
        &no_skin,
    );
    assert_eq!(repeated.config.audio.volume, 200);
    assert_eq!(
        repeated.report.problems.len(),
        2,
        "{:?}",
        repeated.report.problems
    );
}

/// A file without a trailing newline is read to its last character.
#[test]
fn a_file_without_a_trailing_newline_is_read() {
    let plan = import_plan("[Winamp]\nvolume=200", &Config::default(), None, &no_skin);
    assert_eq!(plan.config.audio.volume, 200);
    assert!(
        plan.report.problems.is_empty(),
        "{:?}",
        plan.report.problems
    );
}

/// The whitespace around a key and around a value is not part of either.
#[test]
fn whitespace_around_a_key_or_a_value_is_dropped() {
    let plan = import_plan(
        "[Winamp]\r\n  volume = 200  \r\n\twy=29\t\r\n",
        &Config::default(),
        None,
        &no_skin,
    );
    assert_eq!(plan.config.audio.volume, 200);
    assert_eq!(window(&plan.config, WindowId::Main).y, 29);
    assert!(
        plan.report.problems.is_empty(),
        "{:?}",
        plan.report.problems
    );
}

/// Bare CR line endings are not the file S12 §2 describes ("CRLF").
///
/// The whole file is then one unreadable line. It is reported - the import
/// never guesses what the line might have meant - and nothing is applied.
#[test]
fn a_file_with_bare_cr_line_endings_is_refused_visibly() {
    let plan = import_plan(
        "[Winamp]\rvolume=200\rshuffle=1\r",
        &Config::default(),
        None,
        &no_skin,
    );
    assert_eq!(plan.config, Config::default());
    assert_eq!(plan.report.problems.len(), 1, "{:?}", plan.report.problems);
}

/// A key before the first section header is in no section, and a file without
/// a `[Winamp]` section holds no setting of the player's: both change nothing
/// and say why (S12 §2, §8).
#[test]
fn a_file_without_the_players_section_changes_nothing_and_says_why() {
    let headerless = import_plan("volume=200\n", &Config::default(), None, &no_skin);
    assert_eq!(headerless.config, Config::default());
    let reason = headerless.report.reason_for("volume").expect("a reason");
    assert!(reason.contains("not in a section"), "{reason}");

    let plug_in = import_plan(
        "[Some Plug-in]\nvolume=200\n",
        &Config::default(),
        None,
        &no_skin,
    );
    assert_eq!(plug_in.config, Config::default());
    let reason = plug_in.report.reason_for("volume").expect("a reason");
    assert!(reason.contains("does not interpret"), "{reason}");
}

/// S12 §2 gives the file no comment syntax, so a `;` line is not skipped
/// silently: a comment is reported as a line without `=`, and a
/// commented-out key is reported as a key the reference does not know - it is
/// never applied.
#[test]
fn a_semicolon_comment_is_reported_and_never_applied() {
    let plan = import_plan(
        "[Winamp]\n; a note\n; volume=200\nshuffle=1\n",
        &Config::default(),
        None,
        &no_skin,
    );

    assert_eq!(
        plan.config.audio.volume,
        Config::default().audio.volume,
        "a commented-out key was applied"
    );
    assert!(plan.config.playlist.shuffle);
    assert_eq!(plan.report.problems.len(), 1, "{:?}", plan.report.problems);
    let reason = plan.report.reason_for("; volume").expect("a reason");
    assert!(reason.contains("does not recognise"), "{reason}");
}

// ---------------------------------------------------------------------------
// Values the mapping cannot represent
// ---------------------------------------------------------------------------

/// A NUL byte is part of the value, not a terminator the parser may skip.
///
/// `volume=20\0` is not the number 200: the key is refused and reported.
#[test]
fn a_nul_byte_in_a_value_does_not_turn_it_into_a_number() {
    let plan = import_plan(
        "[Winamp]\nvolume=20\u{0}0\nshuffle=1\n",
        &Config::default(),
        None,
        &no_skin,
    );
    assert_eq!(plan.config.audio.volume, Config::default().audio.volume);
    let reason = plan.report.reason_for("volume").expect("a reason");
    assert!(reason.contains("not a number"), "{reason}");
    // The key behind it is still read: a hostile value does not end the file.
    assert!(plan.config.playlist.shuffle);
}

/// A value that is empty, or holds no number, is refused with a reason and
/// leaves the setting as it was.
#[test]
fn a_value_the_setting_cannot_take_is_refused_with_a_reason() {
    let mut current = Config::default();
    current.audio.volume = 77;

    let plan = import_plan(
        "[Winamp]\nvolume=\nshuffle=\nskin=\n",
        &current,
        None,
        &no_skin,
    );
    assert_eq!(plan.config.audio.volume, 77, "an empty value was applied");
    for key in ["volume", "shuffle"] {
        let reason = plan.report.reason_for(key).expect("a reason");
        assert!(reason.contains("not a number"), "{key}: {reason}");
    }
    let reason = plan.report.reason_for("skin").expect("a reason");
    assert!(reason.contains("no skin"), "{reason}");
}

/// A number the setting's own range cannot hold is a **range** problem, not a
/// spelling problem: the report must not call `3000000000` "not a number".
#[test]
fn a_number_the_setting_cannot_hold_is_reported_as_out_of_range() {
    let plan = import_plan(
        "[Winamp]\nwx=3000000000\npe_width=-1\nvolume=loud\n",
        &Config::default(),
        None,
        &no_skin,
    );

    for key in ["wx", "pe_width"] {
        let reason = plan.report.reason_for(key).expect("a reason");
        assert!(reason.contains("outside the range"), "{key}: {reason}");
    }
    // A value that holds no number at all is still a spelling problem.
    let reason = plan.report.reason_for("volume").expect("a reason");
    assert!(reason.contains("not a number"), "{reason}");
}

/// The two clamps S12 §9 names are applied, and an open range keeps the
/// number the file wrote.
#[test]
fn the_clamps_are_the_ones_the_spec_names() {
    let high = import_plan(
        "[Winamp]\nvolume=999999\n",
        &Config::default(),
        None,
        &no_skin,
    );
    assert_eq!(high.config.audio.volume, 255);

    let low = import_plan("[Winamp]\nvolume=-1\n", &Config::default(), None, &no_skin);
    assert_eq!(low.config.audio.volume, 0);

    // A number that does not even fit an `i64` saturates and is then clamped,
    // rather than being read as no number at all.
    let huge = import_plan(
        "[Winamp]\nvolume=99999999999999999999999999\n",
        &Config::default(),
        None,
        &no_skin,
    );
    assert_eq!(huge.config.audio.volume, 255);

    // The window sizes' range is open (S12 §10, open question 3): a number a
    // `u32` can hold is kept as it was written ...
    let held = import_plan(
        "[Winamp]\npe_width=4294967295\n",
        &Config::default(),
        None,
        &no_skin,
    );
    assert_eq!(
        window(&held.config, WindowId::Playlist).width,
        Some(u32::MAX)
    );

    // ... and one that does not fit is refused, visibly: nothing is written,
    // not even a window layout to hold the value.
    let refused = import_plan(
        "[Winamp]\npe_width=4294967296\n",
        &Config::default(),
        None,
        &no_skin,
    );
    assert!(
        refused.config.ui.windows.is_empty(),
        "a refused size was kept"
    );
    let reason = refused.report.reason_for("pe_width").expect("a reason");
    assert!(reason.contains("outside the range"), "{reason}");
}

// ---------------------------------------------------------------------------
// The report's words
// ---------------------------------------------------------------------------

/// A key that was read but repeated is **not** a line that was not read.
///
/// The one-line report is what the preferences page shows; calling a repeated
/// key a "line not read" is a false statement about the file.
#[test]
fn a_repeated_key_is_not_called_a_line_that_was_not_read() {
    let plan = import_plan(
        "[Winamp]\nvolume=200\nvolume=10\n",
        &Config::default(),
        None,
        &no_skin,
    );
    assert_eq!(plan.report.problems.len(), 1);

    let line = winamp_ini::report_line(Path::new("winamp.ini"), &plan.report);
    assert!(!line.contains("not read"), "{line}");
    assert!(line.contains("1 warning"), "{line}");
    assert!(
        plan.report.summary().contains("1 warning"),
        "{}",
        plan.report.summary()
    );

    // Two unreadable lines are two warnings, in the plural.
    let two = import_plan(
        "[Winamp]\nno equals sign here\n[broken\nvolume=200\n",
        &Config::default(),
        None,
        &no_skin,
    );
    assert_eq!(two.report.problems.len(), 2, "{:?}", two.report.problems);
    let line = winamp_ini::report_line(Path::new("winamp.ini"), &two.report);
    assert!(line.contains("2 warnings"), "{line}");
}

// ---------------------------------------------------------------------------
// A megabyte on one line, and a megabyte in one value
// ---------------------------------------------------------------------------

/// A megabyte on one line is a line like any other: it is reported when it
/// cannot be read, and the keys around it are still read (S12 §4.3 cuts only
/// the value, at 2048 bytes).
#[test]
fn a_megabyte_on_one_line_is_read_without_a_panic() {
    let mut text = String::from("[Winamp]\nvolume=200\n");
    text.push_str(&"x".repeat(1024 * 1024));
    text.push('\n');
    text.push_str("shuffle=1\n");

    let plan = import_plan(&text, &Config::default(), None, &no_skin);
    assert_eq!(plan.config.audio.volume, 200);
    assert!(plan.config.playlist.shuffle);
    assert_eq!(plan.report.problems.len(), 1, "{:?}", plan.report.problems);
}

/// A megabyte in one value is cut at the reference's staging buffer (S12
/// §4.3, §9), and the cut value is what the report names.
#[test]
fn a_megabyte_value_is_cut_at_the_reference_buffer() {
    let mut text = String::from("[Winamp]\nlast_shortdesc=");
    text.push_str(&"a".repeat(1024 * 1024));
    text.push('\n');

    let plan = import_plan(&text, &Config::default(), None, &no_skin);
    let entry = plan
        .report
        .keys
        .iter()
        .find(|entry| entry.key == "last_shortdesc")
        .expect("the key is reported");
    assert_eq!(entry.value.len(), winamp_ini::VALUE_LIMIT);
}

/// The cut falls on a character boundary: a character that would straddle it
/// is dropped whole, never split in half.
#[test]
fn the_cut_never_splits_a_character() {
    // 2047 bytes of `a` plus a three-byte character: the character would end
    // at byte 2050, so the cut at 2048 has to step back to 2047.
    let value = format!("{}{}", "a".repeat(2047), "\u{20ac}");
    assert!(value.len() > winamp_ini::VALUE_LIMIT);

    let plan = import_plan(
        &format!("[Winamp]\nlast_shortdesc={value}\n"),
        &Config::default(),
        None,
        &no_skin,
    );
    let entry = plan
        .report
        .keys
        .iter()
        .find(|entry| entry.key == "last_shortdesc")
        .expect("the key is reported");
    assert_eq!(entry.value.len(), 2047);
    assert_eq!(entry.value, "a".repeat(2047));
}

// ---------------------------------------------------------------------------
// Bytes that are not the file: the import reads, refuses, never writes
// ---------------------------------------------------------------------------

/// A binary file renamed `.ini` is not text: the import refuses it and the
/// file's bytes are untouched.
#[test]
fn a_binary_file_renamed_ini_is_refused_and_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    let bytes: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
    std::fs::write(&ini, &bytes).unwrap();

    let error = winamp_ini::import_file(&ini, &Config::default(), None, &no_skin)
        .expect_err("a binary file is not UTF-8 text");
    assert!(matches!(error, ImportError::Read { .. }), "{error:?}");

    // Through the app: reported as not imported, nothing applied, the page
    // says what happened.
    let mut app = app_with_config(Config::default(), None);
    let outcome = app
        .finish_winamp_ini_import(Some(&ini))
        .expect("the import ran");
    assert!(
        matches!(outcome, ImportOutcome::Failed { .. }),
        "{outcome:?}"
    );
    assert_eq!(app.engine().state().volume, Config::default().audio.volume);
    let page = app
        .prefs_settings()
        .winamp_ini_report
        .expect("the page shows the report");
    assert!(page.contains("not imported"), "{page}");

    assert_eq!(
        std::fs::read(&ini).unwrap(),
        bytes,
        "the file was written to"
    );
}

/// A UTF-16 file is refused rather than read as text full of NUL bytes.
#[test]
fn a_utf16_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    let text = "[Winamp]\r\nvolume=200\r\n";
    let mut bytes = vec![0xFF, 0xFE]; // the UTF-16LE byte order mark
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(&ini, &bytes).unwrap();

    let mut app = app_with_config(Config::default(), None);
    let outcome = app
        .finish_winamp_ini_import(Some(&ini))
        .expect("the import ran");
    assert!(
        matches!(outcome, ImportOutcome::Failed { .. }),
        "{outcome:?}"
    );
    assert_eq!(app.engine().state().volume, Config::default().audio.volume);
    assert_eq!(std::fs::read(&ini).unwrap(), bytes);
}

/// A UTF-16 file *without* a byte order mark is valid UTF-8 (NUL bytes are
/// characters), so it is parsed - and then no key matches, which is reported
/// key by key. It never becomes a setting.
#[test]
fn a_utf16_file_without_a_mark_never_becomes_a_setting() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    let text = "[Winamp]\r\nvolume=200\r\n";
    let bytes: Vec<u8> = text
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    std::fs::write(&ini, &bytes).unwrap();

    let plan = winamp_ini::import_file(&ini, &Config::default(), None, &no_skin)
        .expect("NUL bytes are valid UTF-8");
    assert_eq!(plan.config, Config::default());
    assert!(
        !plan.report.keys.is_empty() || !plan.report.problems.is_empty(),
        "the file was read and reported nothing"
    );
    assert_eq!(std::fs::read(&ini).unwrap(), bytes);
}

/// A read-only file is imported like any other, and its bytes stay as they
/// were: the import only ever reads.
#[test]
fn a_read_only_file_is_read_and_left_byte_for_byte_as_it_was() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    let bytes = b"[Winamp]\r\nvolume=200\r\nshuffle=1\r\n".to_vec();
    std::fs::write(&ini, &bytes).unwrap();
    let mut permissions = std::fs::metadata(&ini).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&ini, permissions).unwrap();

    let mut app = app_with_config(Config::default(), None);
    app.import_winamp_ini(&ini).expect("the file is imported");
    assert_eq!(app.engine().state().volume, 200);
    assert!(app.engine().state().shuffle);
    assert_eq!(std::fs::read(&ini).unwrap(), bytes);
}

/// A file the player can take no setting from changes nothing in the running
/// player either - the import does not reset what it did not find.
#[test]
fn a_file_with_no_usable_setting_leaves_the_running_player_alone() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, "[Winamp]\ncwd=C:\\Temp\noutname=out_wave.dll\n").unwrap();

    let mut config = Config::default();
    config.audio.volume = 77;
    config.playlist.shuffle = true;
    let mut app = app_with_config(config, None);
    let before = app.engine().state();

    let report = app.import_winamp_ini(&ini).expect("the file is imported");

    assert_eq!(report.applied().count(), 0, "{:?}", report.lines());
    assert_eq!(app.engine().state(), before);
}

// ---------------------------------------------------------------------------
// Idempotence
// ---------------------------------------------------------------------------

/// The file the idempotence cases import: every kind of key the mapping
/// honours - a scalar, a partial band list, two windows and two flags.
const EVERY_KIND: &str = "[Winamp]\r\n\
                          volume=200\r\n\
                          pan=-40\r\n\
                          eq_data=10,20,30\r\n\
                          wx=100\r\n\
                          wy=200\r\n\
                          pe_wx=10\r\n\
                          pe_wy=20\r\n\
                          pe_width=275\r\n\
                          pe_height=116\r\n\
                          mw_open=1\r\n\
                          eq_open=0\r\n\
                          pe_open=1\r\n\
                          windowshade=0\r\n\
                          eq_ws=0\r\n";

/// Mapping the same text onto its own result changes nothing: no accumulation,
/// no doubled window entries, no band that drifts.
#[test]
fn mapping_the_same_text_twice_yields_the_same_configuration() {
    let once = import_plan(EVERY_KIND, &Config::default(), None, &no_skin);
    let twice = import_plan(EVERY_KIND, &once.config, once.playlist_font_size, &no_skin);

    assert_eq!(twice.config, once.config);
    assert_eq!(
        twice.config.ui.windows.len(),
        once.config.ui.windows.len(),
        "a window entry was doubled"
    );
}

/// Importing the same file twice in the app yields the same configuration and
/// the same running state.
#[test]
fn importing_the_same_file_twice_yields_the_same_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let ini = dir.path().join("winamp.ini");
    std::fs::write(&ini, EVERY_KIND).unwrap();
    let config_path = dir.path().join("rustamp.toml");

    let mut app = app_with_config(Config::default(), Some(config_path.clone()));
    app.import_winamp_ini(&ini).expect("the first import");
    let first = app.config().clone();
    let first_state = app.engine().state();

    app.import_winamp_ini(&ini).expect("the second import");

    assert_eq!(
        app.config(),
        &first,
        "the second import changed the configuration"
    );
    assert_eq!(
        app.config().ui.windows.len(),
        first.ui.windows.len(),
        "the second import doubled a window entry"
    );
    assert_eq!(app.engine().state(), first_state);

    // And what the player saves is the same file, saved twice.
    app.save_config().expect("the configuration is saved");
    let saved = std::fs::read_to_string(&config_path).expect("the file is there");
    app.save_config().expect("the configuration is saved again");
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("the file is there"),
        saved
    );
}
