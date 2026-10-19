// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless tests for ReplayGain wiring into the app.

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_app::skin::{self, SkinChoice};
use rustamp_audio::replaygain::{ReplayGainMode, gain_db_to_linear};
use rustamp_core::config::Config;
use rustamp_core::event::Event;
use rustamp_core::meta::TrackMetadata;
use rustamp_meta::replaygain::{ALBUM_GAIN_KEY, TRACK_GAIN_KEY};

fn test_app() -> App {
    let skin = skin::load(&SkinChoice::Builtin);
    let config = Config::default();
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

fn metadata_with(track: Option<&str>, album: Option<&str>) -> TrackMetadata {
    let mut m = TrackMetadata::default();
    if let Some(g) = track {
        m.extra.insert(TRACK_GAIN_KEY.to_owned(), g.to_owned());
    }
    if let Some(g) = album {
        m.extra.insert(ALBUM_GAIN_KEY.to_owned(), g.to_owned());
    }
    m
}

#[test]
fn default_mode_is_off() {
    let app = test_app();
    assert_eq!(app.engine().replaygain_mode(), ReplayGainMode::Off);
    assert_eq!(
        app.config().audio.replaygain,
        rustamp_core::config::ReplayGainMode::Off
    );
}

#[test]
fn off_mode_passes_unity() {
    let mut app = test_app();
    let md = metadata_with(Some("-3.50 dB"), Some("-5.10 dB"));
    app.handle_event(&Event::TrackChanged {
        id: None,
        index: Some(0),
        metadata: Some(Box::new(md)),
        duration_ms: Some(1000),
    });
    assert_eq!(
        app.engine().replaygain_control().effective_gain_current(),
        1.0
    );
}

#[test]
fn track_and_album_select_the_right_tag_values() {
    // Track mode
    let mut app = test_app();
    app.engine_mut().set_replaygain_mode(ReplayGainMode::Track);
    let md = metadata_with(Some("-3.50 dB"), Some("-5.10 dB"));
    app.handle_event(&Event::TrackChanged {
        id: None,
        index: Some(0),
        metadata: Some(Box::new(md)),
        duration_ms: Some(1000),
    });
    let g = app.engine().replaygain_control().effective_gain_current();
    assert!((g - gain_db_to_linear(-3.5)).abs() < 1e-6, "{g}");

    // Album mode
    let mut app = test_app();
    app.engine_mut().set_replaygain_mode(ReplayGainMode::Album);
    let md = metadata_with(Some("-3.50 dB"), Some("-5.10 dB"));
    app.handle_event(&Event::TrackChanged {
        id: None,
        index: Some(0),
        metadata: Some(Box::new(md)),
        duration_ms: Some(1000),
    });
    let g = app.engine().replaygain_control().effective_gain_current();
    assert!((g - gain_db_to_linear(-5.1)).abs() < 1e-6, "{g}");
}

#[test]
fn absent_tags_leave_the_mode_harmless() {
    let mut app = test_app();
    app.engine_mut().set_replaygain_mode(ReplayGainMode::Track);
    let md = metadata_with(None, None);
    app.handle_event(&Event::TrackChanged {
        id: None,
        index: Some(0),
        metadata: Some(Box::new(md)),
        duration_ms: Some(1000),
    });
    assert_eq!(
        app.engine().replaygain_control().effective_gain_current(),
        1.0
    );
}
