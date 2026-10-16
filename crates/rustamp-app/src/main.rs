// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! `rustamp` — the application binary (task card N1).
//!
//! Parses the command line and hands it to [`rustamp_app::run`], which loads
//! the config and the skin, builds the audio engine, opens the main window and
//! routes commands and events between the headless UI machine and the engine.
//!
//! ```text
//! rustamp [<file>] [--null] [--no-audio] [--volume <0..=255>] [--skin <path>] [--seconds <n>]
//! ```
//!
//! With `--null` the app uses the deterministic null output and runs
//! **headless** (no window), so the binary can be exercised in a headless test;
//! with `--no-audio` it uses the same null output but still opens the main
//! window (silent windowed playback, no audio device needed); with the default
//! output it opens the main window and plays on the best device for the host.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use rustamp_app::cli::{self, USAGE};

fn main() -> ExitCode {
    let args = match cli::parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("rustamp: {error}");
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    if args.help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match rustamp_app::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rustamp: {error}");
            ExitCode::FAILURE
        }
    }
}
