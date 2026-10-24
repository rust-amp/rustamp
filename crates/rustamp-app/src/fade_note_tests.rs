// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The report about a fade that does not fit the buffer it is realised in:
//! the app's half of the engine's report (S18 §2.3's constraint).
//!
//! The engine produces the report; the app decides when the user reads it.
//! The report fires on every fade-out, so the app shows it once per
//! configuration (see [`crate::fade_note`]).

use std::sync::Arc;

use super::*;
use rustamp_core::audio::{AudioSpec, Confidence, Decoder, DecoderFactory, MediaSource, ProbeHint};
use rustamp_core::meta::TrackMetadata;
use rustamp_core::playlist::MediaLocation;
use rustamp_core::state::StreamInfo;

/// A silent decoder: enough to open a queue entry without a media file.
struct SilenceDecoder {
    spec: AudioSpec,
    pos: u64,
    total: u64,
}

impl Decoder for SilenceDecoder {
    fn spec(&self) -> AudioSpec {
        self.spec
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let channels = usize::from(self.spec.channels);
        let want = out.len() / channels;
        let n = want.min(self.total.saturating_sub(self.pos) as usize);
        for sample in &mut out[..n * channels] {
            *sample = 0.0;
        }
        self.pos += n as u64;
        Ok(n)
    }

    fn seek(&mut self, to: Duration) -> Result<Duration> {
        let frames = (to.as_secs_f64() * f64::from(self.spec.sample_rate)) as u64;
        self.pos = frames.min(self.total);
        Ok(self.spec.duration_of(self.pos))
    }

    fn duration(&self) -> Option<Duration> {
        Some(self.spec.duration_of(self.total))
    }

    fn metadata(&self) -> TrackMetadata {
        TrackMetadata::default()
    }

    fn stream_info(&self) -> StreamInfo {
        StreamInfo {
            bitrate_kbps: Some(128),
            sample_rate_hz: self.spec.sample_rate,
            channels: self.spec.channels,
        }
    }
}

struct SilenceFactory;

impl DecoderFactory for SilenceFactory {
    fn name(&self) -> &str {
        "fade-note-fixture"
    }

    fn extensions(&self) -> &[&str] {
        &["fadfixture"]
    }

    fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
        if hint.extension == Some("fadfixture") {
            Confidence::CONTENT
        } else {
            Confidence::NONE
        }
    }

    fn open(&self, _source: MediaSource) -> Result<Box<dyn Decoder>> {
        Ok(Box::new(SilenceDecoder {
            spec: AudioSpec::new(44100, 2),
            pos: 0,
            total: 44_100,
        }))
    }
}

/// An app with one track in the queue and the shipped configuration: a null
/// device that grants a 1024-frame buffer (23 ms at 44.1 kHz), a ring of
/// 16384 frames (371 ms) and a 500 ms fade.
fn fixture() -> (App, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("track.fadfixture");
    std::fs::write(&path, b"").unwrap();
    let mut registry = DecoderRegistry::new();
    registry.register(Arc::new(SilenceFactory) as Arc<dyn DecoderFactory>);
    let mut app = App::with_registry(
        AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 200,
            media: Some(MediaLocation::File(path)),
            seconds: None,
        },
        registry,
    )
    .unwrap();
    app.sync_entry_ids();
    (app, dir)
}

/// The fade reports among the notes the user reads.
fn fade_reports(app: &mut App) -> Vec<String> {
    app.drain_notes()
        .into_iter()
        .filter(|note| note.contains("does not fit"))
        .collect()
}

/// The fade that does not fit the device's buffer reaches the user, with the
/// engine's own sentence and its numbers.
#[test]
fn a_fade_that_does_not_fit_the_buffer_reaches_the_user() {
    let (mut app, _dir) = fixture();

    app.submit(Command::Play).unwrap();
    app.submit(Command::StopWithFadeout).unwrap();

    let reports = fade_reports(&mut app);
    assert_eq!(reports.len(), 1, "{reports:?}");
    let report = &reports[0];
    // The engine's wording, unchanged: the requested fade, the buffer that
    // is too short and the buffer length that would hold the whole fade.
    assert!(report.contains("500 ms"), "{report}");
    assert!(report.contains("23 ms"), "{report}");
    assert!(report.contains("device's buffer"), "{report}");
}

/// The report fires on every fade-out; the user reads it once.
#[test]
fn the_second_report_about_the_same_configuration_is_suppressed() {
    let (mut app, _dir) = fixture();

    app.submit(Command::Play).unwrap();
    app.submit(Command::StopWithFadeout).unwrap();
    assert_eq!(
        fade_reports(&mut app).len(),
        1,
        "the first report reaches the user"
    );

    // The play cancels the running fade-out and ramps the gain back up: the
    // engine reports the very same conflict once more.
    app.submit(Command::Play).unwrap();
    assert!(
        fade_reports(&mut app).is_empty(),
        "the same configuration is not reported twice"
    );
}

/// A fade that does not fit the buffer is reported again when the fade
/// changes: the report is about the configuration, not about the event.
#[test]
fn a_changed_fade_arms_the_report_again() {
    let (mut app, _dir) = fixture();

    app.submit(Command::Play).unwrap();
    app.submit(Command::StopWithFadeout).unwrap();
    assert_eq!(
        fade_reports(&mut app).len(),
        1,
        "the first report reaches the user"
    );

    // The fade is a configuration setting: the engine that runs next runs
    // the configured fade, so the report is about a new configuration.
    app.submit(Command::Stop).unwrap();
    app.config.audio.fadeout_ms = 200;
    app.recreate_engine().unwrap();

    app.submit(Command::Play).unwrap();
    app.submit(Command::StopWithFadeout).unwrap();

    let reports = fade_reports(&mut app);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(
        reports[0].contains("200 ms"),
        "the new fade is reported: {reports:?}"
    );
}

/// The dedupe belongs to the fade reports alone: another note still reaches
/// the user.
#[test]
fn another_note_is_not_suppressed_by_the_fade_dedupe() {
    let (mut app, _dir) = fixture();

    app.submit(Command::Play).unwrap();
    app.submit(Command::StopWithFadeout).unwrap();
    app.submit(Command::ToggleVisPlugin).unwrap();

    let notes = app.drain_notes();
    assert!(
        notes.iter().any(|note| note.contains("does not fit")),
        "the fade report reaches the user: {notes:?}"
    );
    assert!(
        notes.iter().any(|note| note.contains("ToggleVisPlugin")),
        "the other note reaches the user too: {notes:?}"
    );
}
