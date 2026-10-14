//! Print stream info, metadata, duration and decoded frame count of a file.
//!
//! Usage: `cargo run -p rustamp-decoders --example decode_info -- <file>`

use std::path::PathBuf;
use std::process::ExitCode;

use rustamp_core::audio::MediaSource;
use rustamp_decoders::default_registry;

fn main() -> ExitCode {
    let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: decode_info <file>");
        return ExitCode::from(2);
    };
    let mut dec = match default_registry().open(MediaSource::Path(path)) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    let spec = dec.spec();
    println!("stream:    {:?}", dec.stream_info());
    println!("seekable:  {}", dec.is_seekable());
    println!("duration:  {:?}", dec.duration());
    println!("metadata:  {:#?}", dec.metadata());

    let ch = usize::from(spec.channels);
    let mut buf = vec![0.0f32; 4096 * ch];
    let mut frames = 0u64;
    loop {
        match dec.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => frames += n as u64,
            Err(e) => {
                eprintln!("decode error after {frames} frames: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    println!(
        "decoded:   {frames} frames ({:.3} s)",
        frames as f64 / f64::from(spec.sample_rate)
    );
    ExitCode::SUCCESS
}
