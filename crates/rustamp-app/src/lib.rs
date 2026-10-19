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
//! * [`cursor`] — the skin's cursors: which one a window shows and the
//!   cache of the operating system's cursors. Headless; the `SetCursor`
//!   half lives in [`rustamp_platform::cursor`].
//! * [`skin`] — skin selection with a clear fallback to the repo's built-in
//!   default (Rust-themed) skin.
//! * [`routing`] — which layer owns a [`rustamp_core::command::Command`].
//! * [`region`] — the skin's window shape: the `region.txt` section a
//!   window uses and the rectangles its mask becomes. Pure; the operating
//!   system's window region lives in [`rustamp_platform::window_region`].
//! * [`app`] - [`app::App`]: engine + headless UI machine + command/event
//!   routing. Fully headless and unit-tested.
//! * [`eq_autoload`] — the equalizer's automatic per-track preset selection
//!   (the AUTO button): the rule table, the decision and the track-start
//!   wiring (S07 §12 item 13).
//! * [`dialogs`] - the simple modal dialogs (About, file info, jump-to) on
//!   top of [`rustamp_ui::DialogManager`]: opening, closing, jump input and
//!   the body lines. Headless; the OS window lives in [`window`].
//! * [`directory`] - what a chosen directory contributes to the playlist:
//!   the media files of the tree below it, and the command that appends
//!   them. Pure; the folder dialog itself lives in [`window`].
//! * [`double_click`] - the double-click rule of S03 §5: the second press
//!   must follow the first within 500 ms and 4 px. The playlist list and the
//!   three title bars share it.
//! * [`focus`] — the window-focus order for `FocusNextWindow` (own choice over
//!   [`rustamp_core::state::WindowId`], not a measurement). Pure; the model
//!   lives in [`app::App`], the OS half in [`window`].
//! * [`host`] — the app side of the player-state bridge to `rustamp-plughost32`:
//!   assembling [`rustamp_core::state::PlayerState`] snapshots and publishing
//!   them as [`rustamp_ipc::Message::PlayerStateUpdate`] over the existing
//!   control channel (RFC 0006), and reading the host's plug-in answers back.
//! * [`plugin_inventory`] — what the host told us about its plug-ins: the
//!   host-assigned id, the name and the kind of every loaded plug-in. The
//!   ids belong to one host process (RFC 0020 Q6).
//! * [`menu`] — main-menu check flags, leaf activation (wave21b) and the
//!   popup's open state and click routing on top of [`rustamp_ui::menu`].
//!   Headless; the OS window lives in [`window`].
//! * [`library_window`] — the media library: the app's index, the view, the
//!   glue between them, the input map and the background scan. Headless; the
//!   OS window lives in [`window`].
//! * [`modern_shell`] — the window path for a window that is **not** skinned
//!   (the media library): the shared shell's attributes with the native
//!   frame and the resizability put back.
//! * [`supervise`] — spawn and supervision for `rustamp-plughost32`: finding
//!   the i686 host binary, starting it with `CREATE_NO_WINDOW` over piped
//!   stdio (with the operator's approval store, which the host needs before
//!   it loads anything), and re-attaching (or restarting) it when it dies —
//!   off the audio path, degrading to detached.
//! * [`window`] — the `winit`/`softbuffer` shell for the real player.
//!
//! [`run`] is the entry point the `rustamp` binary calls.
//!
//! # GUI rule: the window only runs inside the headless test VM
//!
//! [`run`] opens a window for [`cli::OutputChoice::Default`] and for
//! [`cli::OutputChoice::NoAudio`] (the silent windowed mode, `--no-audio`).
//! With [`cli::OutputChoice::Null`] it plays headless through
//! [`app::App::run_headless`] and **never** calls [`window::run`], so the
//! default `cargo test --workspace --all-features` opens no window. As a second
//! rail, [`window::run`] refuses to start when [`window::NO_GUI_ENV`] is set,
//! which the app's test harness always does. A spawned child must not create a
//! window either — **a console window counts as a window on the host** — so
//! test and tool code passes `CREATE_NO_WINDOW`. See `crates/rustamp-app/README.md`
//! for how to run the windowed player (test VM only).

// Only the Win32 external-control subclass needs FFI. All other app code
// remains unsafe-free; the exception is scoped to that private module.
#![deny(unsafe_code)]

pub mod app;
pub mod cli;
pub mod config;
pub mod cursor;
pub mod dialogs;
pub mod directory;
pub mod double_click;
pub mod eq_autoload;
pub mod eq_presets;
#[cfg(any(windows, test))]
mod external_control;
pub mod focus;
pub mod host;
pub mod library_window;
pub mod menu;
pub mod modern_shell;
mod output;
pub mod playlist_window;
pub mod plugin_inventory;
pub mod region;
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
/// runs headless (no window); with [`OutputChoice::NoAudio`] it uses the same
/// null output but still opens the main window (silent windowed playback, no
/// audio device needed); otherwise it opens the main window and plays on the
/// best device for the host.
pub fn run(args: cli::Args) -> Result<(), Box<dyn std::error::Error>> {
    // GUI rail first: a windowed run (the default output or `--no-audio`)
    // opens a window, so a mis-run must fail before the audio device or the
    // event loop exists. `--null` stays headless and needs no rail.
    if args.output != OutputChoice::Null {
        window::ensure_gui_allowed()?;
    }

    // ----- configuration -----
    let config_path = config::default_path();
    // The error is returned, not printed here: the caller (`main`) prints it
    // once, so a config failure is not reported twice.
    let loaded = config::load(config_path.as_deref())?;
    for warning in &loaded.warnings {
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
    // The device is built here, on the thread that calls `window::run` below.
    // On Windows that order is only safe because `rustamp-output` keeps every
    // COM initialisation on its own threads: `winit` needs this thread free to
    // enter a single-threaded apartment when it creates the window. See
    // `output::create_output`.
    let mut app = App::new(options)?;
    app.set_config_path(config_path);
    for warning in &loaded.warnings {
        app.note(warning.clone());
    }
    // Start the plugin host when it is resolvable (see `crate::supervise`):
    // best-effort by contract, so a missing binary only means the player runs
    // detached — it never fails startup and never opens a window. The host
    // reads the operator's approval decisions from the store beside the
    // configuration file; without that argument it approves nothing and
    // refuses every load.
    let approvals = app.plugin_approvals_path();
    app.manage_host(crate::supervise::Supervisor::new(
        crate::supervise::ProcessBackend::from_search().with_approvals(approvals),
    ));

    match args.output {
        // ----- headless test mode: no window -----
        OutputChoice::Null => {
            println!("rustamp: output null (headless; no window)");
            if args.file.is_some() {
                app.submit(Command::Play)?;
            }
            app.run_headless()?;
            if let Err(error) = app.save_config() {
                eprintln!("rustamp: {error}");
            }
            Ok(())
        }
        // ----- the real player: window + device -----
        OutputChoice::Default | OutputChoice::NoAudio => {
            if args.output == OutputChoice::Default {
                println!(
                    "rustamp: output {} @ {} Hz, {} ch",
                    app.engine().output_name(),
                    app.engine().device_spec().sample_rate,
                    app.engine().device_spec().channels
                );
            } else {
                // The silent windowed mode: the null device needs no hardware,
                // so this line is also what the window tests pin to prove the
                // run reached the windowed path without a device.
                println!("rustamp: output null (windowed; silent, no audio device)");
            }
            if args.file.is_some() {
                // S02 §1: a file given on the command line starts playing.
                app.submit(Command::Play)?;
            }
            window::run(app)
        }
    }
}
