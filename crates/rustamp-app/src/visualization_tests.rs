// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless app wiring receipts, for the orchestrator's VM suite only. The
//! decoder below generates test PCM; production never substitutes these samples.

use std::sync::Arc;

use super::*;
use rustamp_core::audio::{AudioSpec, Confidence, Decoder, DecoderFactory, MediaSource, ProbeHint};
use rustamp_core::meta::TrackMetadata;
use rustamp_core::state::StreamInfo;
use rustamp_core::vis::{VIS_SAMPLES, VisFrame};
use rustamp_ui::visualization::{SPECTRUM_BANDS, VisError, VisInput, VisMode, VisState};

struct Signal {
    rate: u32,
    silent: bool,
    position: u64,
}

impl Decoder for Signal {
    fn spec(&self) -> AudioSpec {
        AudioSpec::new(self.rate, 1)
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let count = out
            .len()
            .min((u64::from(self.rate) * 120 - self.position) as usize);
        for (n, sample) in out[..count].iter_mut().enumerate() {
            *sample = if self.silent {
                0.0
            } else {
                let phase = (self.position + n as u64) % VIS_SAMPLES as u64;
                (std::f64::consts::TAU * 16.0 * phase as f64 / VIS_SAMPLES as f64).sin() as f32
                    * 0.5
            };
        }
        self.position += count as u64;
        Ok(count)
    }

    fn seek(&mut self, to: Duration) -> Result<Duration> {
        self.position =
            ((to.as_secs_f64() * f64::from(self.rate)) as u64).min(u64::from(self.rate) * 120);
        Ok(self.spec().duration_of(self.position))
    }

    fn duration(&self) -> Option<Duration> {
        Some(Duration::from_secs(120))
    }

    fn metadata(&self) -> TrackMetadata {
        TrackMetadata::default()
    }

    fn stream_info(&self) -> StreamInfo {
        // Deliberately unavailable readout rate: only source_spec() describes
        // these actual samples. Falling back to this or UI kHz must fail.
        StreamInfo {
            bitrate_kbps: None,
            sample_rate_hz: 0,
            channels: 1,
        }
    }
}

struct SignalFactory;

impl DecoderFactory for SignalFactory {
    fn name(&self) -> &str {
        "vis-wiring-fixture"
    }

    fn extensions(&self) -> &[&str] {
        &["visfixture"]
    }

    fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
        if hint.extension == Some("visfixture") {
            Confidence::CONTENT
        } else {
            Confidence::NONE
        }
    }

    fn open(&self, source: MediaSource) -> Result<Box<dyn Decoder>> {
        let silent = matches!(source, MediaSource::Path(p) if p.file_stem().unwrap() == "quiet");
        Ok(Box::new(Signal {
            rate: if silent { 48_000 } else { 32_000 },
            silent,
            position: 0,
        }))
    }
}

fn fixture() -> (App, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = DecoderRegistry::new();
    registry.register(Arc::new(SignalFactory));
    let mut app = App::with_registry(
        AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume: 0, // The tap is before the output gain, not a volume meter.
            file: None,
            seconds: None,
        },
        registry,
    )
    .unwrap();
    app.engine.set_limiter_enabled(false);
    let paths = ["tone.visfixture", "quiet.visfixture"].map(|name| {
        let path = dir.path().join(name);
        std::fs::write(&path, b"fixture").unwrap();
        TrackSource::path(path)
    });
    app.engine.set_queue(paths.into());
    app.sync_entry_ids();
    (app, dir)
}

fn until(app: &mut App, condition: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition(app) {
        assert!(
            Instant::now() < deadline,
            "vis wiring timed out: {:?}; error {:?}",
            app.state().visualization,
            app.last_error()
        );
        app.advance_null_clock(512);
        app.pump().unwrap();
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn playing() -> (App, tempfile::TempDir) {
    let (mut app, dir) = fixture();
    app.submit(Command::Play).unwrap();
    until(&mut app, |app| {
        app.state().visualization.input == VisInput::Fresh
            && app.state().visualization.bars[4] == 14
    });
    (app, dir)
}

fn assert_static_pixels(app: &mut App) {
    let actual = app.render();
    let mut state = app.state().clone();
    state.visualization = VisState {
        mode: state.visualization.mode,
        ..VisState::default()
    };
    let baseline = render_main_window(&state, app.skin.skin(), &app.layout);
    let area = app.layout.visualization;
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            assert_eq!(actual.pixel(x, y), baseline.pixel(x, y), "{x},{y}");
        }
    }
}

#[test]
fn production_pump_takes_actual_pcm_through_analysis_state_and_pixels() {
    let (mut app, _dir) = playing();
    assert_eq!(app.engine.source_spec().unwrap().sample_rate, 32_000);
    assert_eq!(app.engine.device_spec().sample_rate, 44_100);
    assert_eq!(app.engine.state().stream.unwrap().sample_rate_hz, 0);
    assert!(app.state().visualization.source_timestamp_us.unwrap() > 0);
    assert!(app.drain_notes().is_empty());
    let actual = app.render();
    let mut blank = app.state().clone();
    blank.visualization = VisState::default();
    let baseline = render_main_window(&blank, app.skin.skin(), &app.layout);
    let area = app.layout.visualization;
    assert!(
        (area.y..area.bottom()).any(|y| {
            (area.x..area.right()).any(|x| actual.pixel(x, y) != baseline.pixel(x, y))
        })
    );
}

#[test]
fn pause_freezes_the_display_discards_input_and_resume_accepts_new_pcm() {
    let (mut app, _dir) = playing();
    app.submit(Command::Pause).unwrap();
    let frozen = app.state().visualization.clone();
    let later = app.now_ms() + 10_000;
    app.pump_visualization(later);
    app.ui.refresh(later);
    assert_eq!(app.state().visualization, frozen);
    app.submit(Command::Play).unwrap();
    until(&mut app, |app| {
        app.state().visualization.input == VisInput::Fresh
            && app.state().visualization.source_timestamp_us != frozen.source_timestamp_us
    });
}

#[test]
fn stopping_playing_or_paused_cannot_retain_scope_or_stale_bars() {
    for mode in [VisMode::Spectrum, VisMode::Scope] {
        for pause_first in [false, true] {
            let (mut app, _dir) = playing();
            app.ui.set_vis_mode(mode);
            if pause_first {
                app.submit(Command::Pause).unwrap();
            }
            app.submit(Command::Stop).unwrap();
            assert_eq!(app.state().transport, TransportState::Stopped);
            assert_eq!(app.state().visualization.input, VisInput::Missing);
            assert_eq!(app.state().visualization.source_timestamp_us, None);
            assert_eq!(app.state().visualization.bars, [0; SPECTRUM_BANDS]);
            assert_eq!(app.state().visualization.peaks, [0; SPECTRUM_BANDS]);
            assert_static_pixels(&mut app);
            let later = app.now_ms() + 2000;
            app.pump_visualization(later);
            app.ui.refresh(later);
            assert_eq!(app.state().visualization.bars, [0; SPECTRUM_BANDS]);
            assert_eq!(app.state().visualization.peaks, [0; SPECTRUM_BANDS]);
            assert_static_pixels(&mut app);
        }
    }
}

#[test]
fn external_and_ui_seeks_clear_history_and_drop_the_transition_batch() {
    let (mut app, _dir) = playing();
    for (command, from_ui) in [
        (Command::Seek { to_ms: 4000 }, false),
        (Command::SeekRelative { delta_ms: -1000 }, true),
    ] {
        if from_ui {
            app.ui.handle_command(&command, app.now_ms());
        }
        app.apply_from(&command, from_ui);
        assert!(app.vis_discontinuity);
        assert_eq!(app.state().visualization.input, VisInput::Missing);
        assert_eq!(app.state().visualization.peaks, [0; SPECTRUM_BANDS]);
        app.pump().unwrap();
        assert!(!app.vis_discontinuity);
        assert_eq!(app.state().visualization.input, VisInput::Missing);
        until(&mut app, |app| {
            app.state().visualization.input == VisInput::Fresh
        });
    }
    // This proves reset/drain/resumption, NOT that an untagged frame is post-seek.
}

#[test]
fn track_and_rate_change_do_not_carry_loud_envelopes_into_actual_silence() {
    let (mut app, _dir) = playing();
    app.submit(Command::PlayIndex(1)).unwrap();
    until(&mut app, |app| {
        app.engine.current_index() == Some(1)
            && app.engine.source_spec().unwrap().sample_rate == 48_000
            && app.state().visualization.input == VisInput::Fresh
    });
    assert_eq!(app.state().visualization.bars, [0; SPECTRUM_BANDS]);
    assert_eq!(app.state().visualization.peaks, [0; SPECTRUM_BANDS]);
    assert_eq!(app.state().visualization.scope, [8; 76]);
}

#[test]
fn absent_tap_frames_keep_exact_freshness_hold_fall_and_refresh_partition() {
    let (mut app, _dir) = fixture();
    let frame = VisFrame {
        timestamp_us: 12345,
        waveform: [std::array::from_fn(|n| {
            (std::f64::consts::TAU * 16.0 * n as f64 / VIS_SAMPLES as f64).sin() as f32
        }); 2],
        ..VisFrame::default()
    };
    // Isolated deterministic envelope receipt, distinct from the real-tap test.
    app.ui
        .handle_event(&Event::StateChanged(TransportState::Playing), 0);
    app.ui.push_vis_frame(&frame, 32_000, 0).unwrap();
    let mut once = app.ui.clone();
    for now in [100, 101, 300, 425] {
        app.pump_visualization(now); // No running decoder: no frame to replace it.
        app.ui.refresh(now);
        if now == 100 {
            assert_eq!(app.state().visualization.input, VisInput::Fresh);
        } else {
            assert_eq!(app.state().visualization.input, VisInput::Stale);
        }
    }
    once.refresh(425);
    assert_eq!(app.state().visualization, once.state().visualization);
    assert_eq!(app.state().visualization.source_timestamp_us, Some(12345));
    assert_eq!(app.state().visualization.bars[4], 9);
    assert_eq!(app.state().visualization.peaks[4], 13);
    app.pump_visualization(1300);
    app.ui.refresh(1300);
    assert_eq!(app.state().visualization.bars, [0; SPECTRUM_BANDS]);
    assert_eq!(app.state().visualization.peaks, [0; SPECTRUM_BANDS]);
}

#[test]
fn a_non_finite_frame_is_only_reported_and_playback_keeps_running() {
    // The live decoder sanitises non-finite samples before the tap (RFC 0014
    // §4.5), so this arm is unreachable end to end: it is pinned here by direct
    // injection. The point is that the report never halts playback and never
    // becomes a fake accepted frame.
    let (mut app, _dir) = fixture();
    app.ui
        .handle_event(&Event::StateChanged(TransportState::Playing), 0);
    let mut waveform = [[0.0f32; VIS_SAMPLES]; 2];
    waveform[0][3] = f32::NAN;
    let bad = VisFrame {
        timestamp_us: 1,
        waveform,
        ..VisFrame::default()
    };
    assert_eq!(
        app.ui.push_vis_frame(&bad, 32_000, 0),
        Err(VisError::NonFinitePcm)
    );
    assert_eq!(app.state().transport, TransportState::Playing);
    assert_eq!(app.state().visualization.input, VisInput::Missing);
    assert_eq!(app.state().visualization.source_timestamp_us, None);
}
