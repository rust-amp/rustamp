// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The first minute of the player, end to end but **headless** (task card N5).
//!
//! Every test drives [`App`] through the same entry points the presenter
//! calls for OS input — `mouse_down`/`mouse_up` on the main window,
//! `playlist_mouse_down`/`playlist_double_click` on the playlist, and
//! `dialog_text`/`dialog_key`/`dialog_drop_file` on the dialog — with the
//! null output. No window opens and no audio device is touched.
//!
//! What it pins:
//!
//! * BETA-03: Eject and the playlist's ADD lead to the open-media file face;
//!   a typed full path (or a dropped file) reaches the queue, the playlist
//!   shows the entry, and playing it starts the transport and moves audio.
//! * BETA-04: a missing or unsupported file does not leave the transport
//!   `Playing` over silence; the error is noted and shown in the title, and
//!   Play on an empty queue does not light `Playing`.
//! * BETA-06: the main window's minimize, shade, close and menu buttons act,
//!   and the playlist's own Close hides it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use rustamp_app::app::{App, AppOptions};
use rustamp_app::cli::OutputChoice;
use rustamp_core::audio::{
    AudioSpec, Confidence, Decoder, DecoderFactory, DecoderRegistry, MediaSource, ProbeHint,
};
use rustamp_core::command::{Command, OpenMediaKind};
use rustamp_core::config::Config;
use rustamp_core::error::Result;
use rustamp_core::meta::TrackMetadata;
use rustamp_core::playlist::EntryId;
use rustamp_core::state::{TransportState, WindowId};
use rustamp_ui::{DialogKind, Key, MainWindowLayout, Widget};

/// Seconds of silence each test track lasts (long enough to stay playing).
const TRACK_FRAMES: u64 = 44_100 * 30;

/// A silent decoder: enough to open a queue entry without a media file.
struct Silence {
    pos: u64,
}

impl Decoder for Silence {
    fn spec(&self) -> AudioSpec {
        AudioSpec::new(44_100, 2)
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let n = (out.len() / 2).min(TRACK_FRAMES.saturating_sub(self.pos) as usize);
        out[..n * 2].fill(0.0);
        self.pos += n as u64;
        Ok(n)
    }

    fn seek(&mut self, to: Duration) -> Result<Duration> {
        self.pos = ((to.as_secs_f64() * 44_100.0) as u64).min(TRACK_FRAMES);
        Ok(to)
    }

    fn duration(&self) -> Option<Duration> {
        Some(Duration::from_secs(TRACK_FRAMES / 44_100))
    }

    fn metadata(&self) -> TrackMetadata {
        TrackMetadata {
            title: Some("First Minute".to_string()),
            ..TrackMetadata::default()
        }
    }
}

/// Claims `*.tst` files that exist (the registry sniffs the file first, so
/// a missing path fails before the probe).
struct SilenceFactory;

impl DecoderFactory for SilenceFactory {
    fn name(&self) -> &str {
        "first-minute"
    }

    fn extensions(&self) -> &[&str] {
        &["tst"]
    }

    fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
        if hint.extension == Some("tst") {
            Confidence::CONTENT
        } else {
            Confidence::NONE
        }
    }

    fn open(&self, _source: MediaSource) -> Result<Box<dyn Decoder>> {
        Ok(Box::new(Silence { pos: 0 }))
    }
}

fn app(file: Option<PathBuf>) -> App {
    let mut registry = DecoderRegistry::new();
    registry.register(Arc::new(SilenceFactory) as Arc<dyn DecoderFactory>);
    let options = AppOptions {
        config: Config::default(),
        skin: rustamp_app::skin::builtin(),
        output: OutputChoice::Null,
        volume: 200,
        file,
        seconds: None,
    };
    App::with_registry(options, registry).expect("the headless app builds")
}

fn file(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    let path = dir.path().join(name);
    std::fs::write(&path, b"").expect("write the test file");
    path
}

/// Presses and releases the main-window widget at its centre.
fn click(app: &mut App, widget: Widget) -> Vec<Command> {
    let rect = widget.rect(&MainWindowLayout::CLASSIC);
    let (x, y) = (
        rect.x + rect.width as i32 / 2,
        rect.y + rect.height as i32 / 2,
    );
    let mut commands = app.mouse_down(x, y).expect("press");
    commands.extend(app.mouse_up(x, y).expect("release"));
    commands
}

/// Types `path` one key press at a time, the way the presenter forwards the
/// OS text of each press.
fn type_path(app: &mut App, path: &Path) {
    for ch in path.display().to_string().chars() {
        assert!(app.dialog_text(&ch.to_string()), "{ch:?} was not typed");
    }
}

/// Pumps (and drives the virtual clock) until `cond` holds.
fn pump_until(app: &mut App, mut cond: impl FnMut(&App) -> bool) -> bool {
    for _ in 0..3000 {
        if cond(app) {
            return true;
        }
        app.advance_null_clock(512);
        app.pump().expect("pump");
        std::thread::sleep(Duration::from_millis(1));
    }
    cond(app)
}

const FILES_PLAY: DialogKind = DialogKind::OpenMedia {
    kind: OpenMediaKind::Files,
    play: true,
};
const FILES_ADD: DialogKind = DialogKind::OpenMedia {
    kind: OpenMediaKind::Files,
    play: false,
};

// ----- BETA-03: one real path from the GUI to a chosen file -----

#[test]
fn eject_then_a_typed_path_queues_and_plays_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let song = file(&dir, "song.tst");
    let mut app = app(None);
    assert!(app.engine().queue().is_empty());

    // Eject opens the files face (S03 §5, id 40029 -> OpenMedia, RFC 0005).
    let commands = click(&mut app, Widget::Eject);
    assert_eq!(
        commands,
        vec![Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: true,
        }]
    );
    assert_eq!(app.dialog_open_kind(), Some(FILES_PLAY));

    // A full path with drive colon, separators and extension dot, plus a
    // typo fixed with Backspace.
    type_path(&mut app, &song);
    assert!(app.dialog_text("x"));
    assert!(app.dialog_key(Key::Backspace));
    assert_eq!(app.media_text(), song.display().to_string());
    assert!(app.dialog_key(Key::Enter));
    assert_eq!(app.dialog_open_kind(), None);

    // The path reached the queue and the playlist shows the entry.
    assert_eq!(app.engine().queue().len(), 1);
    assert_eq!(app.playlist().visible_ids(), vec![EntryId(0)]);

    // Playing it starts the transport and moves audio through the engine.
    assert!(
        pump_until(&mut app, |app| {
            app.engine().transport() == TransportState::Playing
                && app.state().transport == TransportState::Playing
                && app.engine().timeline_frames() > 0
        }),
        "the typed file never played: engine {:?}, ui {:?}, error {:?}",
        app.engine().transport(),
        app.state().transport,
        app.last_error()
    );
    assert!(
        pump_until(&mut app, |app| app
            .ui()
            .logical_title()
            .contains("First Minute")),
        "the marquee never named the track: {}",
        app.ui().logical_title()
    );
    assert_eq!(app.last_error(), None);
}

#[test]
fn add_then_double_click_plays_the_typed_file() {
    let dir = tempfile::tempdir().unwrap();
    let song = file(&dir, "added.tst");
    let mut app = app(None);

    // ADD (S06 §4) opens the same files face, without playing (RFC 0005 O5).
    let commands = app.playlist_mouse_down(24, 92).expect("ADD");
    assert_eq!(
        commands,
        vec![Command::OpenMedia {
            kind: OpenMediaKind::Files,
            play: false,
        }]
    );
    assert_eq!(app.dialog_open_kind(), Some(FILES_ADD));
    type_path(&mut app, &song);
    assert!(app.dialog_key(Key::Enter));
    assert_eq!(app.engine().queue().len(), 1);
    assert_eq!(app.playlist().visible_ids(), vec![EntryId(0)]);
    assert_eq!(app.engine().transport(), TransportState::Stopped);

    // Double-clicking the row plays it (S03 §5).
    let commands = app.playlist_double_click(60, 29).expect("double click");
    assert_eq!(commands, vec![Command::PlayEntry(EntryId(0))]);
    assert!(
        pump_until(&mut app, |app| app.state().transport
            == TransportState::Playing
            && app.engine().timeline_frames() > 0),
        "the added file never played: {:?}",
        app.last_error()
    );
}

#[test]
fn a_file_dropped_on_the_dialog_is_queued_and_played() {
    let dir = tempfile::tempdir().unwrap();
    let song = file(&dir, "dropped.tst");
    let mut app = app(None);
    click(&mut app, Widget::Eject);
    assert!(app.dialog_drop_file(song));
    assert!(app.dialog_key(Key::Enter));
    assert_eq!(app.engine().queue().len(), 1);
    assert!(pump_until(&mut app, |app| app.engine().transport()
        == TransportState::Playing));
}

// ----- BETA-04: no phantom playback, errors reach the user -----

#[test]
fn a_missing_file_stops_the_transport_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("not-there.tst");
    let mut app = app(Some(missing));
    app.submit(Command::Play).expect("play is submitted");

    assert!(
        pump_until(&mut app, |app| app.last_error().is_some()
            && app.engine().transport() == TransportState::Stopped),
        "a missing file left the engine {:?} with error {:?}",
        app.engine().transport(),
        app.last_error()
    );
    assert!(pump_until(&mut app, |app| app.state().transport
        == TransportState::Stopped));
    let error = app.last_error().unwrap().to_string();
    assert!(error.contains("not-there.tst"), "{error}");
    assert_eq!(app.window_title(), format!("Rustamp - {error}"));
    let notes = app.drain_notes();
    assert!(
        notes.iter().any(|n| n.starts_with("error:")),
        "the error was not noted: {notes:?}"
    );
}

#[test]
fn an_unsupported_file_typed_into_the_dialog_stops_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let odd = file(&dir, "notes.xyz");
    let mut app = app(None);
    click(&mut app, Widget::Eject);
    type_path(&mut app, &odd);
    assert!(app.dialog_key(Key::Enter));
    assert!(
        pump_until(&mut app, |app| app.last_error().is_some()
            && app.engine().transport() == TransportState::Stopped
            && app.state().transport == TransportState::Stopped),
        "an unsupported file left the engine {:?}",
        app.engine().transport()
    );
    assert!(app.window_title().starts_with("Rustamp - "));

    // A good file afterwards plays and clears the error.
    let song = file(&dir, "good.tst");
    click(&mut app, Widget::Eject);
    type_path(&mut app, &song);
    assert!(app.dialog_key(Key::Enter));
    assert!(pump_until(&mut app, |app| app.engine().transport()
        == TransportState::Playing
        && app.last_error().is_none()));
    assert_eq!(app.window_title(), "Rustamp");
}

#[test]
fn play_on_an_empty_queue_does_not_light_playing() {
    let mut app = app(None);
    let commands = click(&mut app, Widget::Play);
    assert_eq!(commands, vec![Command::Play]);
    assert_eq!(app.engine().transport(), TransportState::Stopped);
    assert!(pump_until(&mut app, |app| app.state().transport
        == TransportState::Stopped));
    let error = app.last_error().expect("the rejected Play is shown");
    assert!(error.contains("playlist is empty"), "{error}");
}

// ----- BETA-06: the controls that look real act -----

#[test]
fn the_main_window_buttons_act() {
    let mut app = app(None);

    // Menu (S04 §1).
    assert_eq!(
        click(&mut app, Widget::WindowMenu),
        vec![Command::OpenMainMenu]
    );
    assert!(app.main_menu_is_open());
    app.close_main_menu();

    // Minimize (S03 §5).
    assert_eq!(
        click(&mut app, Widget::WindowMinimize),
        vec![Command::Minimize]
    );
    assert!(app.take_minimize_request());

    // Shade (S05 §1.3: 275 x 14, and the same button leaves it).
    assert_eq!(
        click(&mut app, Widget::WindowShade),
        vec![Command::ToggleWindowShade(WindowId::Main)]
    );
    assert_eq!(app.frame_size(), (275, 14));
    click(&mut app, Widget::WindowShade);
    assert_eq!(app.frame_size(), (275, 116));

    // Close quits (main window, RFC 0005 O3).
    assert!(!app.quit());
    assert_eq!(click(&mut app, Widget::WindowClose), vec![Command::Quit]);
    assert!(app.quit());
}

#[test]
fn a_right_click_opens_the_main_menu_or_toggles_eq_and_pl() {
    // S03 §5: right-click the main window -> main menu (verified); on the EQ
    // or PL button -> toggle that window; the title's context menu is S04's
    // and unmeasured, so it opens nothing.
    let centre = |widget: Widget| {
        let rect = widget.rect(&MainWindowLayout::CLASSIC);
        (
            rect.x + rect.width as i32 / 2,
            rect.y + rect.height as i32 / 2,
        )
    };
    let mut app = app(None);
    let (x, y) = centre(Widget::Play);
    assert_eq!(
        app.mouse_right_down(x, y).unwrap(),
        vec![Command::OpenMainMenu]
    );
    assert!(app.main_menu_is_open());
    app.close_main_menu();

    let (x, y) = centre(Widget::PlaylistToggle);
    app.mouse_right_down(x, y).unwrap();
    assert!(!app.playlist_open());
    let (x, y) = centre(Widget::EqToggle);
    app.mouse_right_down(x, y).unwrap();
    assert!(app.eq_open());

    let (x, y) = centre(Widget::Marquee);
    assert!(app.mouse_right_down(x, y).unwrap().is_empty());
    assert!(!app.main_menu_is_open());
}

#[test]
fn the_playlist_close_hides_it_and_the_pl_button_brings_it_back() {
    let mut app = app(None);
    assert!(app.playlist_open());
    assert_eq!(
        app.playlist_mouse_down(264, 3).expect("close"),
        vec![Command::ToggleWindow(WindowId::Playlist)]
    );
    assert!(!app.playlist_open());
    assert!(!app.quit());
    click(&mut app, Widget::PlaylistToggle);
    assert!(app.playlist_open());
}
