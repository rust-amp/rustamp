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
//!
//! # Single instance (task card F2)
//!
//! A windowed launch (the default output or `--no-audio`) first claims the
//! single-instance lock ([`rustamp_platform::single_instance`]). The first
//! instance holds it and listens for forwarded command lines for its whole
//! life; a second launch forwards its command line to the running player and
//! exits without opening a second window. `--null` is the headless test mode
//! and never claims the lock, so a test can run beside a normal player.
//!
//! # Global hotkeys (task card F3)
//!
//! The first (windowed) instance also starts the global-hotkey session
//! ([`rustamp_platform::HotkeySession`]), which registers the default
//! `Ctrl+Alt` chords on its own thread for the whole run. A refused chord is a
//! stderr warning, never a startup failure. `--null` starts no session.
//! Fired actions queue in the platform crate; the `window.rs` follow-up drains
//! them through the normal command path (see the F3 card).

#![forbid(unsafe_code)]

use std::process::ExitCode;

use rustamp_app::cli::{self, OutputChoice, USAGE};
use rustamp_platform::HotkeySession;
use rustamp_platform::single_instance::{self, Claim, ForwardRequest, HandOff};

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

    // `--null` is the headless test mode: it never participates in the
    // single-instance protocol (no window, no config-derived instance) and
    // registers no hotkeys. Every other output mode is a real player and
    // takes the lock.
    if args.output == OutputChoice::Null {
        return run_player(args, None, None);
    }

    // The key follows the resolved config path, so two normal launches share
    // one instance while tests with their own `RUSTAMP_CONFIG` never collide.
    let key = single_instance::instance_key(rustamp_app::config::default_path().as_deref());
    match single_instance::claim(&key) {
        Claim::First(instance) => {
            // The session thread registers the default global hotkeys for the
            // whole run; a taken chord is a warning, never a startup failure.
            let session = HotkeySession::start();
            for warning in session.warnings() {
                eprintln!("rustamp: {warning}");
            }
            run_player(args, Some(instance), Some(session))
        }
        Claim::AlreadyRunning => forward(&key, &args),
    }
}

/// Runs the player, keeping `instance` (when present) and the hotkey `session`
/// alive for the whole run.
fn run_player(
    args: cli::Args,
    _instance: Option<single_instance::Instance>,
    _hotkeys: Option<HotkeySession>,
) -> ExitCode {
    match rustamp_app::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rustamp: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Hands the second launch's command line to the running instance and exits.
fn forward(key: &str, args: &cli::Args) -> ExitCode {
    let request = forward_request(args);
    // The running instance's output mode, skin and `--seconds` were fixed at
    // its start-up; report each one we cannot apply rather than drop it.
    for refusal in &request.refusals {
        eprintln!("rustamp: {refusal}");
    }
    match single_instance::forward(key, &request) {
        HandOff::Delivered => {
            println!("rustamp: forwarded to the running instance");
            ExitCode::SUCCESS
        }
        HandOff::Unreachable => {
            // The lock is held but nobody answered: the holder is wedged (a
            // start-up race would have been covered by the retry window).
            // Report rather than open a second window.
            eprintln!(
                "rustamp: another instance holds the lock but did not accept the command \
                 line; it appears unresponsive"
            );
            ExitCode::FAILURE
        }
    }
}

/// The part of a second launch's command line the running instance can apply.
///
/// A file and an initial volume still make sense; the output mode, the skin and
/// the `--seconds` test aid were chosen when the first instance started, so
/// they become refusals.
fn forward_request(args: &cli::Args) -> ForwardRequest {
    let mut refusals = Vec::new();
    if args.output == OutputChoice::NoAudio {
        refusals.push(
            "--no-audio is ignored: the running instance fixed its output at start-up".to_owned(),
        );
    }
    if args.skin.is_some() {
        refusals
            .push("--skin is ignored: the running instance chose its skin at start-up".to_owned());
    }
    if args.seconds.is_some() {
        refusals.push(
            "--seconds is ignored: it is a start-up test aid on the running instance".to_owned(),
        );
    }
    ForwardRequest {
        file: args.file.clone(),
        volume: args.volume,
        refusals,
    }
}
