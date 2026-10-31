// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless tests of the preferences dialog's plug-in pages: what the scan
//! lists on which page, that only an approved plug-in can be switched on, and
//! that the choice is saved and comes back after a restart.

use std::path::{Path, PathBuf};

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_app::config::load;
use rustamp_app::skin::{self, SkinChoice};
use rustamp_core::config::Config;
use rustamp_platform::{PrefsAction, PrefsNode, RowId};
use rustamp_plughost::ApprovalStore;

/// A synthetic PE32 DLL exporting `export` (never given to an OS loader).
fn fake_dll(export: &[u8], tweak: u8) -> Vec<u8> {
    let mut bytes = vec![0; 0x600];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
    bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
    for (offset, value) in [
        (0x84, 0x014c_u16),
        (0x86, 1),
        (0x94, 224),
        (0x96, 0x2102),
        (0x98, 0x10b),
    ] {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    for (offset, value) in [
        (0xa8, 0x1100_u32),
        (0xd0, 0x2000),
        (0xd4, 0x200),
        (0xf4, 16),
        (0xf8, 0x1000),
        (0xfc, 0x100),
        (0x180, 0x400),
        (0x184, 0x1000),
        (0x188, 0x400),
        (0x18c, 0x200),
        (0x19c, 0x6000_0020),
        (0x214, 1),
        (0x218, 1),
        (0x21c, 0x1040),
        (0x220, 0x1044),
        (0x224, 0x1048),
        (0x240, 0x1100),
        (0x244, 0x1060),
    ] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[0x260..0x260 + export.len()].copy_from_slice(export);
    bytes[0x300] = tweak;
    bytes
}

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

fn config_for(plugins: &Path) -> Config {
    let mut config = Config::default();
    config.plugins.plugin_dir = Some(plugins.to_path_buf());
    config
}

fn rows(app: &App, node: PrefsNode) -> Vec<(RowId, String)> {
    app.prefs_settings()
        .plugin_rows(node)
        .map(|row| (row.id, row.text.clone()))
        .collect()
}

#[test]
fn an_approved_plugin_can_be_switched_on_and_off_and_the_choice_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let plugins = dir.path().join("Plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    let approved = fake_dll(b"winampGetInModule2\0", 1);
    std::fs::write(plugins.join("in_good.dll"), &approved).unwrap();
    std::fs::write(
        plugins.join("in_other.dll"),
        fake_dll(b"winampGetInModule2\0", 2),
    )
    .unwrap();
    let mut store = ApprovalStore::new();
    store.approve_bytes("in_good.dll", &approved).unwrap();
    let config_path = dir.path().join("rustamp.toml");
    store
        .save_to_file(&dir.path().join(rustamp_prefs::APPROVALS_FILE))
        .unwrap();

    let mut app = app_with(config_for(&plugins), Some(config_path.clone()));
    app.apply_prefs_action(PrefsAction::RescanPlugins);

    let input = rows(&app, PrefsNode::Input);
    assert_eq!(input.len(), 2, "{input:?}");
    let (good, good_text) = &input[0];
    // Approved, and still off: approving switches nothing on.
    assert!(good_text.contains("in_good.dll") && good_text.ends_with("off"));
    let (other, other_text) = &input[1];
    assert!(other_text.contains("not approved") && other_text.ends_with("off"));
    // Other pages have nothing and say so.
    let output = rows(&app, PrefsNode::Output);
    assert_eq!(output.len(), 3, "own output, the sentence and the folder");
    assert_eq!(output[0].0, RowId::OwnOutput);
    assert_eq!(output[1].0, RowId::Info, "an information row has no id");
    assert_eq!(output[2].0, RowId::Info, "an information row has no id");

    // The unapproved plug-in cannot be switched on: nothing changes.
    app.apply_prefs_action(PrefsAction::TogglePlugin(plugin(*other)));
    assert!(rows(&app, PrefsNode::Input)[1].1.ends_with("off"));
    assert!(app.plugins_config().enabled.is_empty());

    // The approved one switches on, and the choice is on disk at once.
    app.apply_prefs_action(PrefsAction::TogglePlugin(plugin(*good)));
    assert!(rows(&app, PrefsNode::Input)[0].1.ends_with("on"));
    let saved = load(Some(&config_path)).expect("saved at once").config;
    assert_eq!(saved.plugins.enabled, vec!["in_good.dll".to_string()]);
    assert_eq!(saved.plugins.plugin_dir, Some(plugins.clone()));

    // A restart shows it still on; switching off empties the list.
    let mut restarted = app_with(saved, Some(config_path.clone()));
    restarted.apply_prefs_action(PrefsAction::RescanPlugins);
    assert!(rows(&restarted, PrefsNode::Input)[0].1.ends_with("on"));
    restarted.apply_prefs_action(PrefsAction::TogglePlugin(plugin(*good)));
    let saved = load(Some(&config_path)).unwrap().config;
    assert!(saved.plugins.enabled.is_empty());
}

fn plugin(id: RowId) -> usize {
    match id {
        RowId::Plugin(id) => id,
        other => panic!("not a plug-in row: {other:?}"),
    }
}

/// A folder with `out_a.dll` and `out_b.dll`, neither approved.
fn two_outputs(dir: &Path) -> (App, PathBuf) {
    let plugins = dir.join("Plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    for (name, tweak) in [("out_a.dll", 1), ("out_b.dll", 2)] {
        std::fs::write(plugins.join(name), fake_dll(b"winampGetOutModule\0", tweak)).unwrap();
    }
    let config_path = dir.join("rustamp.toml");
    let mut app = app_with(config_for(&plugins), Some(config_path.clone()));
    app.apply_prefs_action(PrefsAction::RescanPlugins);
    (app, config_path)
}

fn approve(app: &mut App, row: usize, answer: bool) {
    app.apply_prefs_action(PrefsAction::ApprovePlugin(row));
    let pending = app.take_plugin_approval_request().expect("the warning");
    assert!(pending.warning().contains("with your rights"));
    app.finish_plugin_approval(pending, answer);
}

#[test]
fn approving_in_the_page_updates_the_rows_and_switches_nothing_on() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, config_path) = two_outputs(dir.path());
    let store = dir.path().join(rustamp_prefs::APPROVALS_FILE);

    // Saying no records nothing.
    approve(&mut app, 0, false);
    assert!(!store.exists());
    assert!(rows(&app, PrefsNode::Output)[1].1.contains("not approved"));

    // Saying yes records the image and leaves it off.
    approve(&mut app, 0, true);
    let output = rows(&app, PrefsNode::Output);
    assert!(output[1].1.contains("out_a.dll"), "{output:?}");
    assert!(output[1].1.contains("- approved - off"), "{output:?}");
    assert!(output[2].1.contains("not approved"), "{output:?}");
    assert!(
        output[0].1.ends_with("on"),
        "the own output stays: {output:?}"
    );
    assert!(app.plugins_config().enabled.is_empty());
    assert!(
        app.prefs_settings()
            .report(PrefsNode::Output)
            .contains("stays off"),
    );
    // The new schema is on disk, so a later load cannot read the approval as "on".
    let saved = load(Some(&config_path)).unwrap().config;
    assert!(saved.plugins.enabled.is_empty());

    // Approved again: the page says so and does not ask.
    app.apply_prefs_action(PrefsAction::ApprovePlugin(0));
    assert!(app.take_plugin_approval_request().is_none());
    assert!(
        app.prefs_settings()
            .report(PrefsNode::Output)
            .contains("already approved")
    );
}

#[test]
fn the_output_page_chooses_one_output_and_the_players_own() {
    let dir = tempfile::tempdir().unwrap();
    let (mut app, config_path) = two_outputs(dir.path());

    // Not approved: refused, and the page says why.
    app.apply_prefs_action(PrefsAction::SelectOutput(Some(0)));
    assert!(app.plugins_config().enabled.is_empty());
    assert!(
        app.prefs_settings()
            .report(PrefsNode::Output)
            .contains("approve it first")
    );

    approve(&mut app, 0, true);
    approve(&mut app, 1, true);

    app.apply_prefs_action(PrefsAction::SelectOutput(Some(0)));
    assert_eq!(app.plugins_config().enabled, ["out_a.dll"]);
    let output = rows(&app, PrefsNode::Output);
    assert!(output[0].1.ends_with("off"), "{output:?}");
    assert!(output[1].1.ends_with("on") && output[2].1.ends_with("off"));

    // One output only, and the choice is saved.
    app.apply_prefs_action(PrefsAction::SelectOutput(Some(1)));
    assert_eq!(app.plugins_config().enabled, ["out_b.dll"]);
    let saved = load(Some(&config_path)).unwrap().config;
    assert_eq!(saved.plugins.enabled, ["out_b.dll"]);

    // Back to the player's own output.
    app.apply_prefs_action(PrefsAction::SelectOutput(None));
    assert!(app.plugins_config().enabled.is_empty());
    assert!(rows(&app, PrefsNode::Output)[0].1.ends_with("on"));
    assert!(
        app.prefs_settings()
            .report(PrefsNode::Output)
            .contains("its own output")
    );
}

#[test]
fn a_schema_1_file_keeps_what_ran_and_a_save_writes_the_explicit_list() {
    let dir = tempfile::tempdir().unwrap();
    let plugins = dir.path().join("Plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    let mut store = ApprovalStore::new();
    for (name, export, tweak) in [
        ("in_on.dll", &b"winampGetInModule2\0"[..], 1),
        ("in_off.dll", &b"winampGetInModule2\0"[..], 2),
        ("out_b.dll", &b"winampGetOutModule\0"[..], 3),
        ("out_a.dll", &b"winampGetOutModule\0"[..], 4),
    ] {
        let image = fake_dll(export, tweak);
        std::fs::write(plugins.join(name), &image).unwrap();
        store.approve_bytes(name, &image).unwrap();
    }
    store
        .save_to_file(&dir.path().join(rustamp_prefs::APPROVALS_FILE))
        .unwrap();
    let config_path = dir.path().join("rustamp.toml");
    std::fs::write(
        &config_path,
        format!(
            "version = 1\n[plugins]\nplugin_dir = '{}'\ndisabled = [\"in_off.dll\"]\n",
            plugins.display()
        ),
    )
    .unwrap();

    let loaded = load(Some(&config_path)).unwrap();
    assert_eq!(loaded.config.version, rustamp_core::config::CONFIG_VERSION);
    // What ran still runs: the first output by name, and everything not off.
    assert_eq!(loaded.config.plugins.enabled, ["in_on.dll", "out_a.dll"]);
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);

    let app = app_with(loaded.config, Some(config_path.clone()));
    app.save_config().unwrap();
    let text = std::fs::read_to_string(&config_path).unwrap();
    assert!(!text.contains("disabled"), "{text}");
    let again = load(Some(&config_path)).unwrap().config;
    assert_eq!(again.version, rustamp_core::config::CONFIG_VERSION);
    assert_eq!(again.plugins.enabled, ["in_on.dll", "out_a.dll"]);
}

#[test]
fn a_broken_approval_list_approves_nothing_and_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let plugins = dir.path().join("Plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    std::fs::write(
        plugins.join("gen_one.dll"),
        fake_dll(b"winampGetGeneralPurposePlugin\0", 1),
    )
    .unwrap();
    std::fs::write(
        dir.path().join(rustamp_prefs::APPROVALS_FILE),
        "not a store",
    )
    .unwrap();

    let mut app = app_with(config_for(&plugins), Some(dir.path().join("rustamp.toml")));
    app.apply_prefs_action(PrefsAction::RescanPlugins);
    let general = rows(&app, PrefsNode::GeneralPurpose);
    assert!(
        general
            .iter()
            .any(|(id, text)| *id == RowId::Info && text.contains("nothing is approved")),
        "{general:?}"
    );
    let entry = general
        .iter()
        .find(|(id, _)| matches!(id, RowId::Plugin(_)))
        .unwrap();
    assert!(entry.1.contains("not approved") && entry.1.ends_with("off"));
    // An approval against a broken list is refused and leaves the list alone.
    let id = plugin(entry.0);
    app.apply_prefs_action(PrefsAction::ApprovePlugin(id));
    let pending = app.take_plugin_approval_request().expect("the warning");
    app.finish_plugin_approval(pending, true);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(rustamp_prefs::APPROVALS_FILE)).unwrap(),
        "not a store"
    );
    assert!(
        app.prefs_settings()
            .report(PrefsNode::GeneralPurpose)
            .contains("was not approved")
    );
}
