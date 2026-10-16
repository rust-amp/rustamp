// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-app
//!
//! The application layer: the piece that finally assembles the player (task
//! card N1). It owns the clock and the OS; the `rustamp-ui` behaviour machine
//! stays headless.
//!
//! * [`cli`] — command-line parsing.
//! * [`config`] — config file loading with defaults (RFC 0001 defines the
//!   types, not the I/O).
//! * [`skin`] — skin selection with a clear fallback to the repo's built-in
//!   default (look-alike) skin.
//! * [`routing`] — which layer owns a [`rustamp_core::command::Command`].
//! * [`app`] - [`app::App`]: engine + headless UI machine + command/event
//!   routing. Fully headless and unit-tested.
//! * [`dialogs`] - the simple modal dialogs (About, file info, jump-to) on
//!   top of [`rustamp_ui::DialogManager`]: opening, closing, jump input and
//!   the body lines. Headless; the OS window lives in [`window`].
//! * [`host`] — the app side of the player-state bridge to `rustamp-plughost32`:
//!   assembling [`rustamp_core::state::PlayerState`] snapshots and publishing
//!   them as [`rustamp_ipc::Message::PlayerStateUpdate`] over the existing
//!   control channel (RFC 0006).
//! * [`menu`] — main-menu check flags and leaf activation (wave21b) on top of
//!   the [`rustamp_ui::menu`] table. Opens no window.
//! * [`supervise`] — spawn and supervision for `rustamp-plughost32`: finding
//!   the i686 host binary, starting it with `CREATE_NO_WINDOW` over piped
//!   stdio, and re-attaching (or restarting) it when it dies — off the audio
//!   path, degrading to detached.
//! * [`window`] — the `winit`/`softbuffer` shell for the real player.
//!
//! [`run`] is the entry point the `rustamp` binary calls.
//!
//! # GUI rule: the window only runs inside the headless test VM
//!
//! [`run`] opens a window only for [`cli::OutputChoice::Default`]. With
//! [`cli::OutputChoice::Null`] it plays headless through
//! [`app::App::run_headless`] and **never** calls [`window::run`], so the
//! default `cargo test --workspace --all-features` opens no window. As a second
//! rail, [`window::run`] refuses to start when [`window::NO_GUI_ENV`] is set,
//! which the app's test harness always does. A spawned child must not create a
//! window either — **a console window counts as a window on the host** — so
//! test and tool code passes `CREATE_NO_WINDOW`. See `crates/rustamp-app/README.md`
//! for how to run the windowed player (test VM only).

#![forbid(unsafe_code)]

pub mod app;
pub mod cli;
pub mod config;
pub mod dialogs;
pub mod host;
pub mod menu;
pub mod playlist_window;
pub mod routing;
pub mod skin;
pub mod supervise;
pub mod window;

use rustamp_core::command::Command;

use crate::app::{App, AppOptions};
use crate::cli::OutputChoice;
use crate::config::ConfigSource;

/// Runs the player for a parsed command line.
///
/// With [`OutputChoice::Null`] the app uses the deterministic null output and
/// runs headless (no window); otherwise it opens the main window and plays on
/// the best device for the host.
pub fn run(args: cli::Args) -> Result<(), Box<dyn std::error::Error>> {
    // GUI rail first: with the default output a window will be opened, so a
    // mis-run must fail before the audio device or the event loop exists.
    if args.output == OutputChoice::Default {
        window::ensure_gui_allowed()?;
    }

    // ----- configuration -----
    let config_path = config::default_path();
    let loaded = config::load(config_path.as_deref());
    if let Some(warning) = &loaded.warning {
        eprintln!("rustamp: {warning}");
    }
    if let ConfigSource::File(path) = &loaded.source {
        println!("rustamp: config {}", path.display());
    }
    let config = loaded.config;

    // ----- skin -----
    let choice = skin::choose(args.skin.as_deref(), config.ui.skin.as_deref());
    let loaded_skin = skin::load(&choice);
    if let Some(warning) = loaded_skin.warning() {
        eprintln!("rustamp: {warning}");
    }
    println!("rustamp: skin {}", loaded_skin.description());

    let volume = args.volume.unwrap_or(config.audio.volume);
    let options = AppOptions {
        config,
        skin: loaded_skin,
        output: args.output,
        volume,
        file: args.file.clone(),
        seconds: args.seconds,
    };
    let mut app = App::new(options)?;
    // Start the plugin host when it is resolvable (see `crate::supervise`):
    // best-effort by contract, so a missing binary only means the player runs
    // detached — it never fails startup and never opens a window.
    app.manage_host(crate::supervise::Supervisor::from_search());

    match args.output {
        // ----- headless test mode: no window -----
        OutputChoice::Null => {
            println!("rustamp: output null (headless; no window)");
            if args.file.is_some() {
                app.submit(Command::Play)?;
            }
            app.run_headless()?;
            Ok(())
        }
        // ----- the real player: window + device -----
        OutputChoice::Default => {
            println!(
                "rustamp: output {} @ {} Hz, {} ch",
                app.engine().output_name(),
                app.engine().device_spec().sample_rate,
                app.engine().device_spec().channels
            );
            if args.file.is_some() {
                // S02 §1: a file given on the command line starts playing.
                app.submit(Command::Play)?;
            }
            window::run(app)
        }
    }
}
