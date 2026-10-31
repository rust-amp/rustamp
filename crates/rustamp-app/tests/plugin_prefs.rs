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
use rustamp_platform::{PrefsAction, PrefsNode};
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

fn rows(app: &App, node: PrefsNode) -> Vec<(Option<usize>, String)> {
    app.prefs_settings()
        .plugin_rows(node)
        .map(|row| (row.id, row.text.clone()))
        .collect()
}

#[test]
fn an_approved_plugin_can_be_switched_off_and_on_and_the_choice_survives_a_restart() {
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
    assert!(good_text.contains("in_good.dll") && good_text.ends_with("on"));
    let (other, other_text) = &input[1];
    assert!(other_text.contains("not approved") && other_text.ends_with("off"));
    // Other pages have nothing and say so.
    let output = rows(&app, PrefsNode::Output);
    assert_eq!(output.len(), 2, "the sentence and the folder");
    assert_eq!(output[0].0, None, "an information row has no id");
    assert_eq!(output[1].0, None, "an information row has no id");

    // The unapproved plug-in cannot be switched on: nothing changes.
    app.apply_prefs_action(PrefsAction::TogglePlugin(other.unwrap()));
    assert!(rows(&app, PrefsNode::Input)[1].1.ends_with("off"));
    assert!(app.plugins_config().disabled.is_empty());

    // The approved one switches off, and the choice is on disk at once.
    app.apply_prefs_action(PrefsAction::TogglePlugin(good.unwrap()));
    assert!(rows(&app, PrefsNode::Input)[0].1.ends_with("off"));
    let saved = load(Some(&config_path)).expect("saved at once").config;
    assert_eq!(saved.plugins.disabled, vec!["in_good.dll".to_string()]);
    assert_eq!(saved.plugins.plugin_dir, Some(plugins.clone()));

    // A restart shows it still off; switching on again clears the list.
    let mut restarted = app_with(saved, Some(config_path.clone()));
    restarted.apply_prefs_action(PrefsAction::RescanPlugins);
    assert!(rows(&restarted, PrefsNode::Input)[0].1.ends_with("off"));
    restarted.apply_prefs_action(PrefsAction::TogglePlugin(good.unwrap()));
    let saved = load(Some(&config_path)).unwrap().config;
    assert!(saved.plugins.disabled.is_empty());
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
            .any(|(id, text)| id.is_none() && text.contains("nothing is approved")),
        "{general:?}"
    );
    let entry = general.iter().find(|(id, _)| id.is_some()).unwrap();
    assert!(entry.1.contains("not approved") && entry.1.ends_with("off"));
}
