//! Play a file through the engine from the command line.
//!
//! ```text
//! cargo run -p rustamp-audio --example play -- <file> [options]
//!
//!   --null              use the deterministic null output (no device needed)
//!   --output <name>     wasapi | cpal | null (default: the best for the host)
//!   --seconds <n>       stop after n seconds of playback
//!   --volume <0..255>   initial volume (default 255)
//!   --balance <-127..127>  initial balance (default 0)
//!   --vis               print the newest vis frame summary every second
//!   --help              this text
//! ```
//!
//! With `--null` the virtual device's clock is driven in real time, so the
//! example plays a file end to end on machines without an audio endpoint
//! (CI, headless containers).

use std::process::ExitCode;
use std::time::{Duration, Instant};

use rustamp_audio::{Engine, EngineConfig, TrackSource};
use rustamp_core::bus::EventBus;
use rustamp_core::error::Result;
use rustamp_core::event::Event;
use rustamp_core::state::TransportState;
use rustamp_output::{CpalOutput, NullOutput, default_output};

struct Args {
    file: String,
    output: OutputKind,
    seconds: Option<u64>,
    volume: u8,
    balance: i8,
    vis: bool,
}

enum OutputKind {
    Best,
    Null,
    Cpal,
    Wasapi,
}

fn parse_args() -> std::result::Result<Args, String> {
    let mut args = Args {
        file: String::new(),
        output: OutputKind::Best,
        seconds: None,
        volume: 255,
        balance: 0,
        vis: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--null" => args.output = OutputKind::Null,
            "--output" => {
                let v = it.next().ok_or("--output needs a value")?;
                args.output = match v.as_str() {
                    "null" => OutputKind::Null,
                    "cpal" => OutputKind::Cpal,
                    "wasapi" => OutputKind::Wasapi,
                    other => return Err(format!("unknown output: {other}")),
                };
            }
            "--seconds" => {
                let v = it.next().ok_or("--seconds needs a value")?;
                args.seconds = Some(v.parse().map_err(|_| "bad --seconds")?);
            }
            "--volume" => {
                let v = it.next().ok_or("--volume needs a value")?;
                args.volume = v.parse().map_err(|_| "bad --volume")?;
            }
            "--balance" => {
                let v = it.next().ok_or("--balance needs a value")?;
                args.balance = v.parse().map_err(|_| "bad --balance")?;
            }
            "--vis" => args.vis = true,
            "--help" | "-h" => return Err(String::new()),
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => args.file = other.to_owned(),
        }
    }
    if args.file.is_empty() {
        return Err("no input file".into());
    }
    Ok(args)
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("error: {msg}");
            }
            eprintln!(
                "usage: play <file> [--null] [--output wasapi|cpal|null] [--seconds <n>] [--volume <0..255>] [--balance <-127..127>] [--vis]"
            );
            return if msg.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &Args) -> Result<()> {
    let (output, clock): (
        Box<dyn rustamp_core::audio::OutputDevice>,
        Option<NullOutputClock>,
    ) = match args.output {
        OutputKind::Null => {
            let (out, clock) = NullOutput::pair();
            (Box::new(out), Some(NullOutputClock::new(clock)))
        }
        OutputKind::Cpal => (Box::new(CpalOutput::new()), None),
        OutputKind::Wasapi => {
            #[cfg(windows)]
            {
                (Box::new(rustamp_output::WasapiOutput::new()), None)
            }
            #[cfg(not(windows))]
            {
                return Err(rustamp_core::error::Error::Unsupported(
                    "WASAPI is Windows only",
                ));
            }
        }
        OutputKind::Best => (default_output(), None),
    };

    let cfg = EngineConfig {
        volume: args.volume,
        balance: args.balance,
        ..EngineConfig::default()
    };
    let bus = EventBus::new();
    let events = bus.subscribe();
    let mut engine = Engine::new(
        rustamp_decoders::default_registry(),
        output,
        bus.clone(),
        cfg,
    )?;
    println!(
        "output: {} @ {} Hz, {} ch",
        engine.output_name(),
        engine.device_spec().sample_rate,
        engine.device_spec().channels
    );
    engine.set_queue(vec![TrackSource::path(&args.file)]);
    engine.play_index(0)?;

    let rate = engine.device_spec().sample_rate;
    let mut clock = clock;
    let mut last_tick = Instant::now();
    let mut pending_frames = 0.0f64;
    let mut last_report = Instant::now();
    let mut last_vis = Instant::now();
    let mut vis_frames = 0u64;
    let mut started = false;
    let mut finished = false;
    let mut frames_played = 0u64;

    loop {
        // Simulate one device period. With a real device the callback runs on
        // its own thread, so the loop only has to poll.
        std::thread::sleep(Duration::from_millis(5));
        let now = Instant::now();
        if let Some(clock) = clock.as_mut() {
            let dt = now.duration_since(last_tick);
            last_tick = now;
            pending_frames += dt.as_secs_f64() * f64::from(rate);
            let frames = pending_frames.floor() as u64;
            pending_frames -= frames as f64;
            clock.pump(frames);
        }
        engine.poll()?;

        // The fade reports reach the caller, not the log: the engine queues
        // them and the caller shows them (S18 §2.3's constraint; see
        // `rustamp_audio::fade_budget`).
        while let Some(note) = engine.take_fade_note() {
            println!("note: {}", note.message());
        }

        for event in events.try_iter() {
            match &event {
                Event::StateChanged(TransportState::Playing) => {
                    started = true;
                    println!("playing");
                }
                Event::StateChanged(TransportState::Paused) => println!("paused"),
                Event::StateChanged(TransportState::Stopped) if started => finished = true,
                Event::TrackChanged {
                    index,
                    metadata,
                    duration_ms,
                    ..
                } => {
                    let title = metadata
                        .as_ref()
                        .and_then(|m| m.title.clone())
                        .unwrap_or_else(|| args.file.clone());
                    println!(
                        "track {index:?}: {title} ({:.1} s)",
                        duration_ms.unwrap_or(0) as f64 / 1000.0
                    );
                }
                Event::StreamInfoChanged(Some(info)) => println!(
                    "stream: {} Hz, {} ch, {} kbps",
                    info.sample_rate_hz,
                    info.channels,
                    info.bitrate_kbps
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "?".into())
                ),
                Event::Error(info) => eprintln!("engine error: {} ({:?})", info.message, info.kind),
                Event::PlaylistEnded => println!("end of playlist"),
                _ => {}
            }
        }

        if args.vis && last_vis.elapsed() >= Duration::from_secs(1) {
            last_vis = Instant::now();
            if let Some(frame) = engine.take_vis_frame() {
                vis_frames += 1;
                let peak = frame
                    .waveform
                    .iter()
                    .flat_map(|row| row.iter())
                    .fold(0.0f32, |acc, v| acc.max(v.abs()));
                println!(
                    "vis: t={:.3}s peak={:.3} (frame {vis_frames})",
                    frame.timestamp_us as f64 / 1e6,
                    peak
                );
            }
        }

        if last_report.elapsed() >= Duration::from_millis(500) {
            last_report = Instant::now();
            print!(
                "\r{:>7.2}s / {:>7.2}s  ",
                engine.position_ms() as f64 / 1000.0,
                engine
                    .duration_frames()
                    .map(|d| d as f64 / f64::from(rate))
                    .unwrap_or(0.0)
            );
            use std::io::Write;
            let _ = std::io::stdout().flush();
            frames_played = engine.timeline_frames();
        }

        if let Some(limit) = args.seconds
            && engine.position_ms() >= limit.saturating_mul(1000)
        {
            println!("\nreached --seconds {limit}, stopping");
            engine.stop()?;
            break;
        }
        if finished {
            break;
        }
        // Nothing playing and nothing buffered: the run is over (an error, or a
        // queue that never started).
        if !started && engine.last_error().is_some() {
            break;
        }
    }

    println!(
        "\nplayed {} frames ({} s), {} skipped packets, {} vis frames",
        engine.last_track_frames().max(frames_played),
        engine.last_track_frames().max(frames_played) as f64 / f64::from(rate),
        engine.skipped_packets(),
        vis_frames
    );
    if let Some(err) = engine.last_error() {
        return Err(rustamp_core::error::Error::Other(err.message.clone()));
    }
    engine.stop()?;
    Ok(())
}

/// Drives a [`NullOutput`]'s clock in real time.
struct NullOutputClock {
    clock: rustamp_output::NullClock,
}

impl NullOutputClock {
    fn new(clock: rustamp_output::NullClock) -> Self {
        NullOutputClock { clock }
    }

    fn pump(&mut self, frames: u64) {
        if frames > 0 {
            self.clock.pump(frames);
        }
    }
}
