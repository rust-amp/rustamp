// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless tests for the app's DSP bypass policy (W14 finding F4).
//!
//! The engine's recovery policy for a failed DSP stage is the **bypass**: the
//! failed stage is replaced by a pass-through, the failure is reported once
//! per run, and the chain runs without that stage until the caller installs a
//! chain again. These tests pin what the **app** does with that report:
//!
//! * the user gets a note that says the DSP was bypassed and why — not a
//!   generic engine error;
//! * the app remembers the bypass, so the preferences page does not claim
//!   that a DSP is running while the engine runs without it;
//! * the chain comes back when the user asks for it (switching the DSP off
//!   and on again), never by itself: there is no retry loop.
//!
//! Everything here is headless: the null output (no audio device), no window,
//! no plug-in and no helper process. The stage that fails is a plain
//! [`DspStage`] the test owns, and the file that plays is the repo's own FLAC
//! fixture.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_app::dsp::DspChain;
use rustamp_app::skin::{self, SkinChoice};
use rustamp_core::audio::{AudioSpec, DspStage};
use rustamp_core::command::Command;
use rustamp_core::config::Config;
use rustamp_core::error::{Error, ErrorInfo, ErrorKind, Result};
use rustamp_core::event::Event;
use rustamp_core::playlist::MediaLocation;
use rustamp_core::state::TransportState;
use rustamp_platform::{PrefsAction, PrefsNode};

/// A stage that fails on the first block, like a resident whose helper died.
struct FailingStage;

impl DspStage for FailingStage {
    fn name(&self) -> &str {
        "failing-dsp"
    }

    fn prepare(&mut self, _spec: AudioSpec) -> Result<()> {
        Ok(())
    }

    fn process(&mut self, _buf: &mut [f32]) -> Result<()> {
        Err(Error::Dsp("the helper died".into()))
    }

    fn reset(&mut self) {}
}

/// A stage that does nothing, like a healthy one.
struct QuietStage;

impl DspStage for QuietStage {
    fn name(&self) -> &str {
        "quiet-dsp"
    }

    fn prepare(&mut self, _spec: AudioSpec) -> Result<()> {
        Ok(())
    }

    fn process(&mut self, _buf: &mut [f32]) -> Result<()> {
        Ok(())
    }

    fn reset(&mut self) {}
}

/// The repo's own 1.5 s FLAC fixture (generated with ffmpeg, never
/// downloaded; see `crates/rustamp-decoders/tests/fixtures/FIXTURES.txt`).
fn fixture() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("rustamp-decoders")
        .join("tests")
        .join("fixtures")
        .join("sine.flac")
}

/// An app that plays the fixture.
fn app_playing_the_fixture() -> App {
    let file = fixture();
    assert!(file.exists(), "missing fixture {}", file.display());
    let options = AppOptions {
        config: Config::default(),
        skin: skin::load(&SkinChoice::Builtin),
        output: OutputChoice::Null,
        volume: 200,
        media: Some(MediaLocation::File(file)),
        seconds: None,
    };
    App::new(options).expect("build headless app")
}

/// An app that plays the fixture, with a chain of one failing stage.
fn app_with_failing_chain(built: Arc<AtomicUsize>) -> App {
    let mut app = app_playing_the_fixture();
    let counter = built.clone();
    app.install_dsp_chain(DspChain::default().with_stage(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::new(FailingStage)
    }));
    app
}

/// Pumps the app until `cond` holds, or the budget runs out.
fn pump_until(app: &mut App, mut cond: impl FnMut(&App) -> bool, rounds: u32) -> bool {
    for _ in 0..rounds {
        if cond(app) {
            return true;
        }
        app.pump().expect("pump");
        std::thread::sleep(Duration::from_millis(1));
    }
    cond(app)
}

/// The notes that name a bypass.
fn bypass_notes(app: &mut App) -> Vec<String> {
    app.drain_notes()
        .into_iter()
        .filter(|note| note.contains("bypassed"))
        .collect()
}

#[test]
fn a_failed_stage_becomes_one_note_that_names_the_bypass() {
    let built = Arc::new(AtomicUsize::new(0));
    let mut app = app_with_failing_chain(built.clone());
    app.submit(Command::Play).expect("play");

    assert!(
        pump_until(&mut app, |app| app.dsp_bypass().is_some(), 4000),
        "the bypass never reached the app"
    );

    let all = app.drain_notes();
    let bypasses: Vec<&String> = all
        .iter()
        .filter(|note| note.contains("bypassed"))
        .collect();
    assert_eq!(bypasses.len(), 1, "the bypass is reported once: {all:?}");
    let note = bypasses[0];
    assert!(note.contains("failing-dsp"), "{note}");
    assert!(note.contains("the helper died"), "{note}");
    // It is not the generic engine error: the note says what was bypassed
    // and what the user can do about it.
    assert!(
        !all.iter().any(|other| other.starts_with("error:")),
        "the generic engine error is still there: {all:?}"
    );
    assert!(note.contains("switch the DSP off and on again"), "{note}");

    // The stage was built once: the app did not retry by itself.
    assert_eq!(built.load(Ordering::SeqCst), 1);
}

#[test]
fn the_remembered_state_names_the_bypassed_stage_and_its_cause() {
    let built = Arc::new(AtomicUsize::new(0));
    let mut app = app_with_failing_chain(built.clone());
    app.submit(Command::Play).expect("play");
    assert!(
        pump_until(&mut app, |app| app.dsp_bypass().is_some(), 4000),
        "the bypass never reached the app"
    );

    let bypass = app.dsp_bypass().expect("the bypass is remembered");
    assert_eq!(bypass.stage, "failing-dsp");
    assert_eq!(bypass.cause, "the helper died");
}

#[test]
fn the_preferences_page_names_the_bypass_instead_of_claiming_a_chain() {
    let built = Arc::new(AtomicUsize::new(0));
    let mut app = app_with_failing_chain(built.clone());
    app.submit(Command::Play).expect("play");
    assert!(
        pump_until(&mut app, |app| app.dsp_bypass().is_some(), 4000),
        "the bypass never reached the app"
    );

    let rows = app.prefs_settings().plugins;
    let dsp_rows: Vec<_> = rows
        .iter()
        .filter(|row| row.node == PrefsNode::DspEffect)
        .collect();
    assert!(!dsp_rows.is_empty(), "the DSP page has rows");
    assert!(
        dsp_rows
            .iter()
            .any(|row| row.text.contains("bypassed") && row.text.contains("failing-dsp")),
        "the DSP page does not name the bypass: {dsp_rows:?}"
    );
}

#[test]
fn an_explicit_re_enable_installs_a_fresh_chain() {
    let built = Arc::new(AtomicUsize::new(0));
    let mut app = app_with_failing_chain(built.clone());
    app.submit(Command::Play).expect("play");
    assert!(
        pump_until(&mut app, |app| app.dsp_bypass().is_some(), 4000),
        "the bypass never reached the app"
    );
    assert_eq!(built.load(Ordering::SeqCst), 1);

    // The manual recovery: the app installs its chain again.
    app.reenable_dsp_chain();
    assert!(
        pump_until(&mut app, |_| built.load(Ordering::SeqCst) == 2, 4000),
        "the chain was not installed again"
    );
    assert!(
        app.dsp_bypass().is_none(),
        "a fresh chain is not bypassed: {:?}",
        app.dsp_bypass()
    );

    // The fresh chain is the one the engine runs: playing again fails in it
    // and the app hears the new bypass.
    app.submit(Command::Play).expect("play again");
    assert!(
        pump_until(&mut app, |app| app.dsp_bypass().is_some(), 4000),
        "the fresh chain did not run"
    );
    let bypass = app.dsp_bypass().expect("the fresh chain failed too");
    assert_eq!(bypass.stage, "failing-dsp");
    assert_eq!(bypass.cause, "the helper died");
}

#[test]
fn a_bypass_does_not_stop_the_playback() {
    // The engine's policy: the track the failure happened in ends, and the
    // queue goes on. The app must not stop the transport on top of that.
    let mut app = app_playing_the_fixture();
    app.install_dsp_chain(DspChain::default().with_stage(|| Box::new(QuietStage)));
    app.submit(Command::Play).expect("play");
    assert_eq!(app.engine().transport(), TransportState::Playing);

    app.handle_event(&Event::Error(ErrorInfo {
        kind: ErrorKind::Dsp,
        message: "dsp error: stage 'failing-dsp' failed: the helper died; the track ends here \
                  and the stage is bypassed from now on; the block it was processing is lost"
            .to_owned(),
    }));

    assert_eq!(
        app.engine().transport(),
        TransportState::Playing,
        "the app stopped the playback the engine meant to continue"
    );
    assert!(app.dsp_bypass().is_some(), "the bypass is remembered");
}

#[test]
fn the_app_never_retries_the_bypassed_chain_by_itself() {
    let built = Arc::new(AtomicUsize::new(0));
    let mut app = app_with_failing_chain(built.clone());
    app.submit(Command::Play).expect("play");
    assert!(
        pump_until(&mut app, |app| app.dsp_bypass().is_some(), 4000),
        "the bypass never reached the app"
    );
    let notes_after_the_bypass = bypass_notes(&mut app).len();
    assert_eq!(notes_after_the_bypass, 1);

    // Let the player run on: the queue ends, the transport stops, and the
    // app keeps pumping. Nothing may be rebuilt and nothing may be reported
    // a second time: a stage that failed would fail again.
    for _ in 0..200 {
        app.pump().expect("pump");
        std::thread::sleep(Duration::from_millis(1));
    }

    assert_eq!(
        built.load(Ordering::SeqCst),
        1,
        "the app rebuilt the failed chain by itself"
    );
    assert_eq!(
        bypass_notes(&mut app).len(),
        0,
        "the app reported the same bypass again"
    );
    assert!(app.dsp_bypass().is_some(), "the bypass is still remembered");
}

/// The manual recovery the user performs: switching the DSP off and on
/// again in the preferences.
#[test]
fn switching_the_dsp_off_and_on_again_installs_the_chain_again() {
    let dir = tempfile::tempdir().unwrap();
    let plugins = dir.path().join("Plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    let image = fake_dll(b"winampDSPGetHeader2\0", 1);
    std::fs::write(plugins.join("dsp_fake.dll"), &image).unwrap();
    let mut store = rustamp_plughost::ApprovalStore::new();
    store.approve_bytes("dsp_fake.dll", &image).unwrap();
    store
        .save_to_file(&dir.path().join(rustamp_prefs::APPROVALS_FILE))
        .unwrap();

    let mut config = Config::default();
    config.plugins.plugin_dir = Some(plugins.clone());
    config.plugins.enabled = vec!["dsp_fake.dll".to_string()];
    let options = AppOptions {
        config,
        skin: skin::load(&SkinChoice::Builtin),
        output: OutputChoice::Null,
        volume: 200,
        media: None,
        seconds: None,
    };
    let mut app = App::new(options).expect("build headless app");
    // The approval store sits beside the configuration file.
    app.set_config_path(Some(dir.path().join("rustamp.toml")));
    app.apply_prefs_action(PrefsAction::RescanPlugins);

    let built = Arc::new(AtomicUsize::new(0));
    let counter = built.clone();
    app.install_dsp_chain(DspChain::default().with_stage(move || {
        counter.fetch_add(1, Ordering::SeqCst);
        Box::new(FailingStage)
    }));
    assert_eq!(built.load(Ordering::SeqCst), 1);

    // The engine reports the bypass.
    app.handle_event(&Event::Error(ErrorInfo {
        kind: ErrorKind::Dsp,
        message: "dsp error: stage 'failing-dsp' failed: the helper died; the track ends here \
                  and the stage is bypassed from now on; the block it was processing is lost"
            .to_owned(),
    }));
    assert!(app.dsp_bypass().is_some(), "the bypass is remembered");

    // The DSP plug-in's row on the preferences page.
    let dsp_rows: Vec<_> = app
        .prefs_settings()
        .plugin_rows(PrefsNode::DspEffect)
        .map(|row| (row.id, row.text.clone()))
        .collect();
    let index = dsp_rows
        .iter()
        .find(|(_, text)| text.contains("dsp_fake.dll"))
        .and_then(|(id, _)| match id {
            rustamp_platform::RowId::Plugin(id) => Some(*id),
            _ => None,
        })
        .expect("the DSP plug-in is listed");

    // Switching the DSP off forgets the bypass: the stage is out of the
    // chain, so the page must stop naming it.
    app.apply_prefs_action(PrefsAction::TogglePlugin(index));
    assert!(
        app.dsp_bypass().is_none(),
        "switching the DSP off forgets the bypass"
    );
    assert_eq!(
        built.load(Ordering::SeqCst),
        1,
        "switching the DSP off must not build a chain"
    );

    // Switching it on again installs the chain again: that is the recovery.
    app.apply_prefs_action(PrefsAction::TogglePlugin(index));
    assert_eq!(
        built.load(Ordering::SeqCst),
        2,
        "switching the DSP on again installs the chain again"
    );
    assert!(
        app.dsp_bypass().is_none(),
        "the fresh chain is not bypassed"
    );
}

/// A switched-on DSP plug-in is in the audio path, and leaves it when it is
/// switched off. The helper starts with the first track, so nothing runs here.
#[test]
fn switching_a_dsp_plugin_puts_it_in_the_audio_path_and_out_again() {
    if rustamp_app::supervise::find_plugin_host().is_none() {
        // The i686 host is not built here: the player says so, covered by VM runs.
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let plugins = dir.path().join("Plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    let image = fake_dll(b"winampDSPGetHeader2 ", 2);
    std::fs::write(plugins.join("dsp_fake.dll"), &image).unwrap();
    let mut store = rustamp_plughost::ApprovalStore::new();
    store.approve_bytes("dsp_fake.dll", &image).unwrap();
    store
        .save_to_file(&dir.path().join(rustamp_prefs::APPROVALS_FILE))
        .unwrap();
    let mut config = Config::default();
    config.plugins.plugin_dir = Some(plugins);
    config.plugins.enabled = vec!["dsp_fake.dll".to_string()];
    let options = AppOptions {
        config,
        skin: skin::load(&SkinChoice::Builtin),
        output: OutputChoice::Null,
        volume: 200,
        media: None,
        seconds: None,
    };
    let mut app = App::new(options).expect("build headless app");
    app.set_config_path(Some(dir.path().join("rustamp.toml")));
    app.apply_prefs_action(PrefsAction::RescanPlugins);
    let row = app
        .plugin_scan()
        .entries
        .iter()
        .position(|entry| entry.kind == rustamp_plugin_abi::PluginKind::Dsp)
        .expect("the fake plug-in is listed");

    // An approved plug-in starts switched on: off, then on again.
    app.apply_prefs_action(PrefsAction::TogglePlugin(row));
    assert!(app.dsp_chain_stage_names().is_empty());
    app.apply_prefs_action(PrefsAction::TogglePlugin(row));
    assert_eq!(app.dsp_chain_stage_names(), ["classic-dsp"]);
    app.apply_prefs_action(PrefsAction::TogglePlugin(row));
    assert!(app.dsp_chain_stage_names().is_empty());
}

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
