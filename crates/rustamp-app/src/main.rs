// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! `rustamp` - the application binary (task card N1).
//!
//! Parses the command line and hands it to [`rustamp_app::run`], which loads
//! the config and the skin, builds the audio engine, opens the main window and
//! routes commands and events between the headless UI machine and the engine.
//!
//! ```text
//! rustamp [<file|url>] [--null] [--no-audio] [--volume <0..=255>] [--skin <path>] [--seconds <n>]
//!        [/CLASS=<name>] [/CONFIG=<path>] [/BOOKMARK <path>] [/ADD <path>]
//!        [/INSTALL] [/UNREG] [/NOREG] [/ONLYINSTALL]
//! ```

// A release build is a **GUI** program: double-clicking `rustamp.exe` must not
// open a console window (owner report, 2026-10-21 - the release zip started with
// one). Debug builds keep the console, because that is where a developer wants
// the notes. `--console` reattaches one in a release build (see
// [`ensure_console`]), which is also how the release binary can be watched.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//!
//! With `--null` the app uses the deterministic null output and runs
//! **headless** (no window), so the binary can be exercised in a headless test;
//! with `--no-audio` it uses the same null output but still opens the main
//! window (silent windowed playback, no audio device needed); with the default
//! output it opens the main window and plays on the best device for the host.
//!
//! An argument that names an `http://` or `https://` URL is streamed (S19 —
//! `https` is our own extension: the spec records only the plain stream);
//! any other scheme is refused by name.
//!
//! # The registration step (S13 §1)
//!
//! Before the player starts, the command line decides what happens to the
//! shell registration (the verbs a drop from Explorer offers, S13 §3):
//! `/INSTALL` writes it, `/UNREG` removes it, `/NOREG` skips the step and
//! `/ONLYINSTALL` writes it and ends the run right there. A failed write is a
//! warning, never a reason to stop the music — except for `/ONLYINSTALL`,
//! where the registration is the whole job.
//!
//! # Single instance (task card F2, S13 §2)
//!
//! A windowed launch (the default output or `--no-audio`) first claims the
//! single-instance lock ([`rustamp_platform::single_instance`]). The instance
//! is the **window class** — `Winamp v1.x` by default, `/CLASS=<name>` when
//! the command line says otherwise (S13 §2). The first instance holds the lock
//! and listens for forwarded command lines for its whole life; a second launch
//! forwards its command line to the running player and exits without opening a
//! second window. `--null` is the headless test mode and never claims the
//! lock, so a test can run beside a normal player.
//!
//! # Global hotkeys (task card F3)
//!
//! The first (windowed) instance also starts the global-hotkey session
//! ([`rustamp_platform::HotkeySession`]), which registers the default
//! `Ctrl+Alt` chords on its own thread for the whole run. A refused chord is a
//! stderr warning, never a startup failure. `--null` starts no session.
//! Fired actions queue in the platform crate; `Windowed::about_to_wait` drains
//! them through the normal command path (see the F3/F5 cards).

#![forbid(unsafe_code)]

use std::process::ExitCode;

use rustamp_app::cli::{self, OutputChoice, USAGE};
use rustamp_platform::HotkeySession;
use rustamp_platform::shell_registration::{self, RegistrationError, RegistrationStep};
use rustamp_platform::single_instance::{self, Claim, HandOff};

fn main() -> ExitCode {
    // Attach the console (or open one) BEFORE anything is printed: in a release
    // build this process is a GUI program, so `rustamp: ...` notes would go
    // nowhere without it. Scanning the raw arguments keeps this ahead of the
    // parse, which may itself print.
    if std::env::args()
        .skip(1)
        .any(|argument| argument == "--console")
    {
        rustamp_platform::console::ensure_console();
    }
    // `--approve-plugin <file>...` is an operator command, not a player run: it
    // records the exact images in the approval store (see `plugin_approval`) and
    // ends. It is recognised on the raw arguments, before the player's own parse.
    {
        let raw: Vec<String> = std::env::args().skip(1).collect();
        if raw.first().map(String::as_str) == Some("--approve-plugin") {
            // `/CONFIG=<path>` picks the configuration (and so the store) as it
            // does for the player; the rest are the plug-in files.
            let (configs, files): (Vec<&String>, Vec<&String>) = raw[1..]
                .iter()
                .partition(|argument| argument.to_ascii_uppercase().starts_with("/CONFIG="));
            let config = configs
                .last()
                .map(|argument| std::path::PathBuf::from(&argument["/CONFIG=".len()..]))
                .or_else(rustamp_app::config::default_path);
            let files: Vec<String> = files.into_iter().cloned().collect();
            let (text, code) = rustamp_app::plugin_approval::run(config.as_deref(), &files);
            if code == 0 {
                println!("{text}");
            } else {
                eprintln!("{text}");
            }
            return ExitCode::from(code);
        }
    }
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

    // S13 §1: the registration step comes before the player starts, and
    // `/ONLYINSTALL` ends the run right after it.
    match shell_registration::registration_step(args.registration) {
        RegistrationStep::Skip => {}
        RegistrationStep::Install => {
            if let Err(error) = install_registration() {
                eprintln!("rustamp: {error}");
            }
        }
        RegistrationStep::Uninstall => {
            if let Err(error) = shell_registration::uninstall() {
                eprintln!("rustamp: {error}");
            }
        }
        RegistrationStep::InstallAndExit => {
            return match install_registration() {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("rustamp: {error}");
                    ExitCode::FAILURE
                }
            };
        }
    }

    // `--null` is the headless test mode: it never participates in the
    // single-instance protocol (no window, no config-derived instance) and
    // registers no hotkeys. Every other output mode is a real player and
    // takes the lock.
    if args.output == OutputChoice::Null {
        return run_player(args, None, None);
    }

    // The instance is the window class (S13 §2) and the configuration: the
    // class is what `/CLASS=` overrides, the configuration keeps two normal
    // launches in one instance while tests with their own `RUSTAMP_CONFIG`
    // never collide.
    let config_path = args.resolved_config_path();
    let key = single_instance::instance_key(args.instance_class(), config_path.as_deref());
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

/// Writes the shell registration (S13 §1, `/INSTALL`).
fn install_registration() -> Result<(), RegistrationError> {
    let exe =
        std::env::current_exe().map_err(|error| RegistrationError::Path(error.to_string()))?;
    shell_registration::install(&exe)
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
    let request = cli::forward_request(args);
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
