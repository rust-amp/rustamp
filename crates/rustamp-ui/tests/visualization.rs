// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Headless integration of the UI-local analyser. Run only in the test VM.

use rustamp_core::command::Command;
use rustamp_core::event::Event;
use rustamp_core::vis::{VIS_SAMPLES, VisFrame};
use rustamp_ui::visualization::{VisError, VisInput, VisMode};
use rustamp_ui::{MainWindow, MainWindowLayout, TransportState, Widget, hit_test};

fn frame() -> VisFrame {
    VisFrame {
        timestamp_us: 12345,
        waveform: [std::array::from_fn(|n| {
            (std::f64::consts::TAU * 16.0 * n as f64 / VIS_SAMPLES as f64).sin() as f32
        }); 2],
        ..VisFrame::default()
    }
}

#[test]
fn actual_pcm_reaches_the_main_render_state_and_absence_is_not_silence() {
    let mut window = MainWindow::new();
    assert_eq!(window.state().visualization.input, VisInput::Missing);
    window.handle_event(&Event::StateChanged(TransportState::Playing), 0);
    window.push_vis_frame(&frame(), 44100, 0).unwrap();
    let state = &window.state().visualization;
    assert_eq!(state.input, VisInput::Fresh);
    assert_eq!(state.source_timestamp_us, Some(12345));
    assert_eq!(state.bars[8], 16);
    window.refresh(2000);
    assert_eq!(window.state().visualization.input, VisInput::Stale);
    assert_eq!(window.state().visualization.bars, [0; 19]);
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
    assert_eq!(window.state().visualization.peaks, [0; 19]);
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
