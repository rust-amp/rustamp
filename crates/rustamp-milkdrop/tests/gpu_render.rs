// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The real pipeline without a window: presets, audio, the engine and the
//! pixels that come back. These need a graphics adapter, so they are `#[ignore]`d;
//! they open no window and start no player. `software_*` asks for the adapter
//! the test VM has (Direct3D 12's WARP), and the others take what is there.
//!
//! ```text
//! cargo test -p rustamp-milkdrop --test gpu_render -- --ignored --test-threads=1
//! ```
//!
//! Each test leaves its last frame as a PNG in `RUSTAMP_MILKDROP_OUT`, when set.

use std::path::Path;
use std::time::{Duration, Instant};

use rustamp_core::vis::{VIS_SAMPLES, VisFrame};
use rustamp_milkdrop::{RenderedFrame, Request, Session, Settings, Status};

/// One 576-sample stretch of a tone with a 4 Hz pulse, stamped at `n * 576`.
fn tone(n: u64, loudness: f32) -> VisFrame {
    let rate = 44_100.0_f64;
    let mut frame = VisFrame {
        timestamp_us: (n + 1) * 576 * 1_000_000 / 44_100,
        ..VisFrame::default()
    };
    for index in 0..VIS_SAMPLES {
        let t = (n as f64 * 576.0 + index as f64) / rate;
        let pulse = 0.5 + 0.5 * (t * std::f64::consts::TAU * 4.0).sin();
        let value = (t * std::f64::consts::TAU * 110.0).sin() * pulse;
        frame.waveform[0][index] = value as f32 * loudness;
        frame.waveform[1][index] = (value * 0.8) as f32 * loudness;
    }
    frame
}

fn wait_running(session: &Session) -> String {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match session.status() {
            Status::Running { preset } => return preset,
            Status::Starting => {
                assert!(Instant::now() < deadline, "never started");
                std::thread::sleep(Duration::from_millis(20));
            }
            other => panic!("{other:?}; notes: {:?}", session.take_notes()),
        }
    }
}

/// Plays `seconds` of the tone in real time and returns the newest frame.
fn play(
    session: &Session,
    seconds: f64,
    n: &mut u64,
    loudness: f32,
) -> std::sync::Arc<RenderedFrame> {
    let start = Instant::now();
    while start.elapsed().as_secs_f64() < seconds {
        session.feed(&tone(*n, loudness), 44_100);
        *n += 1;
        std::thread::sleep(Duration::from_micros(576 * 1_000_000 / 44_100));
    }
    session.frame().expect("a frame after playing")
}

fn lit_pixels(frame: &RenderedFrame) -> usize {
    frame
        .rgba
        .chunks_exact(4)
        .filter(|pixel| pixel[..3].iter().any(|channel| *channel > 16))
        .count()
}

fn changed_pixels(a: &RenderedFrame, b: &RenderedFrame) -> usize {
    a.rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .filter(|(a, b)| a[..3] != b[..3])
        .count()
}

fn keep(frame: &RenderedFrame, name: &str) {
    let Some(dir) = std::env::var_os("RUSTAMP_MILKDROP_OUT") else {
        return;
    };
    let path = Path::new(&dir).join(format!("{name}.png"));
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    // The alpha a preset leaves behind says nothing: the window ignores it.
    let opaque: Vec<u8> = frame
        .rgba
        .chunks_exact(4)
        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
        .collect();
    writer.write_image_data(&opaque).unwrap();
}

fn software() -> Settings {
    Settings {
        software_only: true,
        ..Settings::default()
    }
}

/// The whole path on the software adapter: the path a machine without a GPU has.
#[test]
#[ignore = "needs a graphics adapter (the software one will do); opens no window"]
fn software_adapter_renders_the_built_in_presets_from_audio() {
    the_built_in_presets_render(software(), "software");
}

#[test]
#[ignore = "needs a graphics adapter; opens no window"]
fn any_adapter_renders_the_built_in_presets_from_audio() {
    the_built_in_presets_render(Settings::default(), "any");
}

fn the_built_in_presets_render(settings: Settings, name: &str) {
    let session = Session::start(Settings {
        preset_seconds: 0.0,
        ..settings
    });
    let mut preset = wait_running(&session);
    println!("{:?}", session.take_notes());
    let mut n = 0;
    let mut seen = Vec::new();
    for _ in 0..3 {
        let first = play(&session, 1.5, &mut n, 0.8);
        let second = play(&session, 0.5, &mut n, 0.8);
        assert!(
            second.generation > first.generation,
            "{preset}: the frames stopped"
        );
        assert!(
            lit_pixels(&second) > 200,
            "{preset} drew nothing: {} lit pixels",
            lit_pixels(&second)
        );
        assert!(
            changed_pixels(&first, &second) > 100,
            "{preset} does not move"
        );
        keep(&second, &format!("{name}-{preset}"));
        seen.push(preset.clone());
        session.request(Request::Next);
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let Status::Running { preset: now } = session.status() else {
                panic!("{:?}", session.status());
            };
            if now != preset {
                preset = now;
                break;
            }
            assert!(Instant::now() < deadline, "next never took effect");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    println!("{name}: rendered {seen:?}; {:?}", session.take_notes());
    assert_eq!(seen.len(), 3);
    assert_eq!(
        seen.iter().collect::<std::collections::HashSet<_>>().len(),
        3,
        "next walked through all of them: {seen:?}"
    );
}

/// The audio drives the picture: a preset whose shape follows the bass differs
/// between loud and silent, and the analysis is what the engine reads.
#[test]
#[ignore = "needs a graphics adapter (the software one will do); opens no window"]
fn the_picture_follows_the_audio() {
    let session = Session::start(Settings {
        preset_seconds: 0.0,
        ..software()
    });
    wait_running(&session);
    let mut n = 0;
    let loud = play(&session, 2.0, &mut n, 0.9);
    let lit_loud = lit_pixels(&loud);
    let silent = play(&session, 6.0, &mut n, 0.0);
    let lit_silent = lit_pixels(&silent);
    keep(&loud, "audio-loud");
    keep(&silent, "audio-silent");
    println!("lit with audio: {lit_loud}; lit in silence: {lit_silent}");
    assert_ne!(
        lit_loud, lit_silent,
        "the picture is the same with and without audio"
    );
}

/// A timed change happens by itself.
#[test]
#[ignore = "needs a graphics adapter (the software one will do); opens no window"]
fn a_timed_change_moves_to_another_preset() {
    let session = Session::start(Settings {
        preset_seconds: 2.0,
        random_order: false,
        ..software()
    });
    let first = wait_running(&session);
    let mut n = 0;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        play(&session, 0.2, &mut n, 0.5);
        match session.status() {
            Status::Running { preset } if preset != first => {
                println!("{first} -> {preset}");
                return;
            }
            Status::Running { .. } => {}
            other => panic!("{other:?}"),
        }
        assert!(Instant::now() < deadline, "no timed change in 60 s");
    }
}

/// Resizing keeps rendering, at the new size.
#[test]
#[ignore = "needs a graphics adapter (the software one will do); opens no window"]
fn a_resize_reaches_the_frames() {
    let session = Session::start(software());
    wait_running(&session);
    let mut n = 0;
    assert_eq!(play(&session, 0.5, &mut n, 0.5).width, 640);
    session.resize(321, 203);
    let frame = play(&session, 1.0, &mut n, 0.5);
    assert_eq!((frame.width, frame.height), (321, 203));
    assert_eq!(frame.rgba.len(), 321 * 203 * 4);
    session.resize(1920, 1080);
    let frame = play(&session, 1.0, &mut n, 0.5);
    assert_eq!(
        (frame.width, frame.height),
        (640, 360),
        "the software adapter renders small and the window scales"
    );
}

/// Presets that will not load leave the list; a folder of only broken ones gets
/// the message. A folder with a good one among them plays that.
#[test]
#[ignore = "needs a graphics adapter (the software one will do); opens no window"]
fn broken_presets_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("a-broken.milk"),
        "[preset00]\nper_frame_1=x = = (;\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("b-good.milk"),
        "[preset00]\nfDecay=0.9\nnWaveMode=0\nfWaveAlpha=1\nwave_r=1\n",
    )
    .unwrap();
    let session = Session::start(Settings {
        preset_folder: Some(dir.path().to_path_buf()),
        preset_seconds: 0.0,
        ..software()
    });
    assert_eq!(wait_running(&session), "b-good");
    println!("{:?}", session.take_notes());
}
