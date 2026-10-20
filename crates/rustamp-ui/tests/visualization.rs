// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless integration of the shared analyser. Run only in the test VM.

use rustamp_core::command::Command;
use rustamp_core::event::Event;
use rustamp_core::state::WindowId;
use rustamp_core::vis::{VIS_SAMPLES, VisFrame};
use rustamp_ui::visualization::{SPECTRUM_BANDS, VisError, VisInput, VisMode};
use rustamp_ui::{MainWindow, MainWindowLayout, TransportState, Widget, hit_test};
use rustamp_vis::analyser::{Analyser, ClassicMode, SPECTRUM_ROWS};

fn frame() -> VisFrame {
    VisFrame {
        timestamp_us: 12345,
        waveform: [std::array::from_fn(|n| {
            (std::f64::consts::TAU * 16.0 * n as f64 / VIS_SAMPLES as f64).sin() as f32
        }); 2],
        ..VisFrame::default()
    }
}

/// The band that holds the tone of [`frame`]: bin 16 of the transform.
const TONE_BAND: usize = 4;

#[test]
fn actual_pcm_reaches_the_main_render_state_and_absence_is_not_silence() {
    let mut window = MainWindow::new();
    assert_eq!(window.state().visualization.input, VisInput::Missing);
    window.handle_event(&Event::StateChanged(TransportState::Playing), 0);
    window.push_vis_frame(&frame(), 44100, 0).unwrap();
    let state = &window.state().visualization;
    assert_eq!(state.input, VisInput::Fresh);
    assert_eq!(state.source_timestamp_us, Some(12345));
    assert_eq!(state.bars[TONE_BAND], 16);
    window.refresh(2000);
    assert_eq!(window.state().visualization.input, VisInput::Stale);
    assert_eq!(window.state().visualization.bars, [0; SPECTRUM_BANDS]);
    assert_eq!(
        window.push_vis_frame(&frame(), 0, 2000),
        Err(VisError::MissingSampleRate)
    );
    assert_eq!(window.state().visualization.input, VisInput::Stale);
}

#[test]
fn pause_freezes_and_seek_and_track_change_clear_the_display_history() {
    let mut window = MainWindow::new();
    window.handle_event(&Event::StateChanged(TransportState::Playing), 0);
    window.push_vis_frame(&frame(), 44100, 0).unwrap();
    window.handle_command(&Command::Pause, 0);
    let frozen = window.state().visualization.clone();
    window.refresh(10000);
    window
        .push_vis_frame(&VisFrame::default(), 44100, 10000)
        .unwrap();
    assert_eq!(window.state().visualization, frozen);
    window.handle_command(&Command::Seek { to_ms: 500 }, 10000);
    assert_eq!(window.state().visualization.input, VisInput::Missing);
    window.handle_command(&Command::TogglePause, 10000);
    window.push_vis_frame(&frame(), 44100, 10000).unwrap();
    window.handle_event(
        &Event::TrackChanged {
            id: None,
            index: None,
            metadata: None,
            duration_ms: None,
        },
        10000,
    );
    assert_eq!(window.state().visualization.input, VisInput::Missing);
    assert_eq!(window.state().visualization.peaks, [0; SPECTRUM_BANDS]);
}

/// S08 §1: the shaded main window draws its own analyser, 38 x 5 px with
/// five bands.
#[test]
fn the_shaded_main_window_carries_the_five_band_analyser() {
    let mut window = MainWindow::new();
    window.handle_event(&Event::StateChanged(TransportState::Playing), 0);
    window.push_vis_frame(&frame(), 44100, 0).unwrap();
    assert_eq!(window.state().visualization.bars[TONE_BAND], 16);
    window.handle_command(&Command::ToggleWindowShade(WindowId::Main), 0);
    assert!(window.state().windowshade);
    // The area changed, so the display starts empty.
    assert_eq!(window.state().visualization.bars, [0; SPECTRUM_BANDS]);
    window.push_vis_frame(&frame(), 44100, 0).unwrap();
    let bars = window.state().visualization.bars;
    assert!(
        bars.iter().all(|height| *height <= 5),
        "a shaded bar is five rows tall"
    );
    assert_eq!(
        bars.iter().filter(|height| **height > 0).count(),
        1,
        "the shaded area has five bands"
    );
}

#[test]
fn own_choice_mode_cycle_requires_release_in_the_area_and_emits_no_command() {
    let mut window = MainWindow::new();
    let area = MainWindowLayout::CLASSIC.visualization;
    assert_eq!(
        hit_test(&MainWindowLayout::CLASSIC, area.x, area.y),
        Some(Widget::Visualization)
    );
    assert_eq!(area, rustamp_gfx::Rect::new(24, 43, 76, 16));
    for mode in [
        VisMode::Scope,
        VisMode::ScopeDots,
        VisMode::Off,
        VisMode::Spectrum,
    ] {
        assert!(window.mouse_down(area.x, area.y, 0).is_empty());
        assert!(window.mouse_up(area.x, area.y, 0).is_empty());
        assert_eq!(window.state().visualization.mode, mode);
    }
    window.mouse_down(area.x, area.y, 0);
    window.mouse_up(274, 115, 0);
    assert_eq!(window.state().visualization.mode, VisMode::Spectrum);
}

#[test]
fn main_window_uses_the_shared_trigger_and_dots_only_change_the_stroke() {
    let mut window = MainWindow::new();
    window.handle_event(&Event::StateChanged(TransportState::Playing), 0);
    window.set_vis_mode(VisMode::Scope);
    let mut shared = Analyser::default();
    shared.set_mode(ClassicMode::Oscilloscope);
    let mut reference = None;
    let mut raw_reference = None;
    for index in 0..6 {
        let frame = VisFrame {
            // A sliding, exactly periodic waveform: the previous untriggered
            // scope moved with this window, whereas the shared trigger locks it.
            waveform: [std::array::from_fn(|n| if (n + index * 5) % 36 < 18 { 0.8 } else { -0.8 });
                2],
            ..VisFrame::default()
        };
        let now = index as u64 * 20;
        shared.push(&frame, 44100, now).unwrap();
        window.push_vis_frame(&frame, 44100, now).unwrap();
        let rows = shared.scope().map(|sample| {
            ((1.0 - f64::from(sample).clamp(-1.0, 1.0)) * f64::from(SPECTRUM_ROWS - 1) * 0.5)
                .round() as u8
        });
        assert!(shared.trigger().is_some());
        assert_eq!(window.state().visualization.scope, rows);
        if let Some(reference) = reference {
            assert_eq!(rows, reference, "triggered UI pixels must not slide");
            assert_ne!(frame.waveform[0], raw_reference.unwrap());
        } else {
            reference = Some(rows);
            raw_reference = Some(frame.waveform[0]);
        }
        window.set_vis_mode(VisMode::ScopeDots);
        assert_eq!(window.state().visualization.scope, rows);
        assert_eq!(window.state().visualization.input, VisInput::Fresh);
        window.set_vis_mode(VisMode::Scope);
    }
}

#[test]
fn main_window_spectrum_tracks_shared_heights_and_envelopes() {
    let mut window = MainWindow::new();
    window.handle_event(&Event::StateChanged(TransportState::Playing), 0);
    let mut shared = Analyser::default();
    for (now, input) in [(0, frame()), (400, VisFrame::default()), (800, frame())] {
        shared.push(&input, 44100, now).unwrap();
        window.push_vis_frame(&input, 44100, now).unwrap();
        for time in [now, now + 100, now + 125, now + 300] {
            shared.advance(time);
            window.refresh(time);
            assert_eq!(window.state().visualization.bars, shared.bars());
            assert_eq!(window.state().visualization.peaks, shared.peaks());
        }
    }
}
