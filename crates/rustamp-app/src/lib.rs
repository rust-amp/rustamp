// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # rustamp-app
//!
//! The application layer: the piece that finally assembles the player (task
//! card N1). It owns the clock and the OS; the `rustamp-ui` behaviour machine
//! stays headless.
//!
//! * [`cli`] — command-line parsing.
//! * [`config`] — config file loading with defaults (RFC 0001 defines the
//!   types, not the I/O). The save keeps every setting the player does not
//!   apply, so a hand-edited file is never silently trimmed.
//! * [`cursor`] — the skin's cursors: which one a window shows and the
//!   cache of the operating system's cursors. Headless; the `SetCursor`
//!   half lives in [`rustamp_platform::cursor`].
//! * [`skin`] — skin selection with a clear fallback to the repo's built-in
//!   default (Rust-themed) skin.
//! * [`skin_folder`] — the skins folder beside the configuration file: the
//!   skins the player can load from disk, the rules of the scan and the
//!   cache that keeps the list from being read again for every frame.
//! * [`routing`] — which layer owns a [`rustamp_core::command::Command`].
//! * [`region`] — the skin's window shape: the `region.txt` section a
//!   window uses and the rectangles its mask becomes. Pure; the operating
//!   system's window region lives in [`rustamp_platform::window_region`].
//! * [`app`] - [`app::App`]: engine + headless UI machine + command/event
//!   routing. Fully headless and unit-tested.
//! * [`eq_autoload`] — the equalizer's automatic per-track preset selection
//!   (the AUTO button): the rule table, the decision and the track-start
//!   wiring (S07 §12 item 13).
//! * [`fade_note`] — the report about a fade that does not fit the buffer it
//!   is realised in: the configuration it is about and the rule that shows it
//!   once per configuration (S18 §2.3).
//! * [`dialogs`] - the simple modal dialogs (About, file info, jump-to) on
//!   top of [`rustamp_ui::DialogManager`]: opening, closing, jump input and
//!   the body lines. Headless; the OS window lives in [`window`].
//! * [`directory`] - what a chosen directory contributes to the playlist:
//!   the media files of the tree below it, and the command that appends
//!   them. Pure; the folder dialog itself lives in [`window`].
//! * [`dsp`] — the app's DSP chain: the stages the app installs in the
//!   engine, the bypass the engine reports when a stage fails, and the
//!   manual recovery from it (W14 finding F4).
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
//! * [`html_playlist`] — the HTML playlist (S03 §2 `40292`): the pure
//!   document generator and the app half that writes it where the user says.
//!   Headless; the save dialog lives in `rustamp-platform`.
//! * [`plugin_inventory`] — what the host told us about its plug-ins: the
//!   host-assigned id, the name and the kind of every loaded plug-in. The
//!   ids belong to one host process (RFC 0020 Q6).
//! * [`menu`] — main-menu check flags, leaf activation (wave21b) and the
//!   popup's open state and click routing on top of [`rustamp_ui::menu`].
//!   Headless; the OS window lives in [`window`].
//! * [`library_window`] — the media library: the app's index, the view, the
//!   glue between them, the input map and the background scan. Headless; the
//!   OS window lives in [`window`].
//! * [`library_extract`] — the playlist extraction of the media library
//!   (spec S20 §4): the files the path template names are copied into the
//!   target directory, the playlist lists them, and the failures are
//!   reported in the wording of the reference. Headless; the configuration
//!   dialog lives in `rustamp-platform`.
//! * [`modern_shell`] — the window path for a window that is **not** skinned
//!   (the media library): the shared shell's attributes with the native
//!   frame and the resizability put back.
//! * [`supervise`] — spawn and supervision for `rustamp-plughost32`: finding
//!   the i686 host binary, starting it with `CREATE_NO_WINDOW` over piped
//!   stdio (with the operator's approval store, which the host needs before
//!   it loads anything), and re-attaching (or restarting) it when it dies —
//!   off the audio path, degrading to detached.
//! * [`stream_status`] — what the app shows about the stream that is playing:
//!   the status texts of S19 §2, the phase change that is a note and the
//!   counters that are a transient status. Headless; the window shows the
//!   status in its status area.
//! * [`title_format`] — the playlist's title format (spec S14) as a setting
//!   of the app: the format string, the check that says whether it can be
//!   used, and the fields a playlist row offers to it.
//! * [`playlist_tags`] — the tags of the playlist's rows, read off the UI
//!   thread so that painting a frame never touches the disk.
//! * [`window`] — the `winit`/`softbuffer` shell for the real player.
//! * [`winamp_ini`] — the import of a Winamp 2.95 `winamp.ini` (S12): the
//!   parser for the file's shape, the mapper onto our configuration and the
//!   table that says which key becomes which setting.
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

// Only the Win32 window subclasses need FFI: the external-control subclass
// on the main window and the window-command seam on the equalizer and the
// playlist editor. All other app code remains unsafe-free; the exceptions
// are scoped to those private modules.
#![deny(unsafe_code)]

pub mod app;
pub mod cli;
pub mod config;
mod config_preserve;
pub mod cursor;
pub mod dialogs;
pub mod directory;
pub mod double_click;
pub mod dsp;
pub mod eq_autoload;
pub mod eq_presets;
#[cfg(any(windows, test))]
mod external_control;
pub mod fade_note;
pub mod focus;
pub mod host;
pub mod html_playlist;
pub mod library_extract;
mod library_recovery;
pub mod library_window;
pub mod menu;
pub mod modern_shell;
mod output;
pub mod playlist_tags;
pub mod playlist_window;
pub mod plugin_inventory;
pub mod region;
pub mod routing;
pub mod skin;
pub mod skin_folder;
pub mod stream_status;
pub mod supervise;
pub mod title_format;
pub mod winamp_ini;
pub mod window;

use rustamp_core::command::Command;
use rustamp_core::playlist::MediaLocation;

use crate::app::{App, AppOptions};
use crate::cli::OutputChoice;
use crate::config::ConfigSource;

/// Imports a Winamp configuration and prints what the import did.
///
/// The file is only read: the import never writes to it, and what it
/// produces is our own configuration, which the player saves on clean exit.
fn import_winamp_ini(app: &mut App, path: &std::path::Path) {
    match app.import_winamp_ini(path) {
        Ok(report) => {
            println!(
                "rustamp: imported {} ({})",
                path.display(),
                report.summary()
            );
            for line in report.lines() {
                println!("rustamp: {line}");
            }
        }
        Err(error) => eprintln!("rustamp: the Winamp configuration was not imported: {error}"),
    }
}

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

    // ----- what the command line asks for -----
    // `/BOOKMARK <path>` adds the path and does not play it (S13 §1), and
    // `/ADD <path>` enqueues a path the same way. The bookmark store does
    // not exist yet (RFC 0005 O12), so the path is added to the playlist
    // and the run says so.
    let mut added: Vec<MediaLocation> = Vec::new();
    let mut bookmarked = false;
    match &args.bookmark {
        Some(cli::Bookmark::Location(location)) => {
            added.push(location.clone());
            bookmarked = true;
            eprintln!(
                "rustamp: the bookmark store does not exist yet (RFC 0005 O12): {} was added to \
                 the playlist instead",
                location.to_display_string()
            );
        }
        Some(cli::Bookmark::WithoutPath) => eprintln!(
            "rustamp: /BOOKMARK without a path is not defined (S13 §5.3); nothing was bookmarked"
        ),
        None => {}
    }
    // The enqueued media (S13 §1): `/ADD <path>` names its own media and
    // `--enqueue` names the media argument. The media is **added** to the
    // playlist and not played — the `Enqueue` shell verb is registered with
    // `/ADD`, so the verb and the switch ask for the same thing.
    let enqueued = match &args.enqueue {
        cli::Enqueue::None => None,
        cli::Enqueue::Argument => args.media.clone(),
        cli::Enqueue::Location(location) => Some(location.clone()),
        cli::Enqueue::WithoutPath => {
            eprintln!(
                "rustamp: /ADD without a path is not defined (S13 §5.3); nothing was enqueued"
            );
            None
        }
    };
    if let Some(location) = &enqueued {
        added.push(location.clone());
        eprintln!(
            "rustamp: {} was added to the playlist instead of being played",
            location.to_display_string()
        );
    }
    // The media plays unless it is the one that is only added.
    let plays = args.media.is_some() && !matches!(args.enqueue, cli::Enqueue::Argument);

    // ----- configuration -----
    // `/CONFIG=` replaces the default path (S13 §1).
    let config_path = args.resolved_config_path();
    // The error is returned, not printed here: the caller (`main`) prints it
    // once, so a config failure is not reported twice.
    let loaded = config::load(config_path.as_deref())?;
    for warning in &loaded.warnings {
        eprintln!("rustamp: {warning}");
    }
    if let ConfigSource::File(path) = &loaded.source {
        println!("rustamp: config {}", path.display());
    }
    // The playlist editor's runtime font size is the app's own setting (see
    // `config::PLAYLIST_FONT_SIZE_KEY`); it is restored after the app is
    // built, so it wins over the skin's size.
    let playlist_font_size = loaded.playlist_font_size;
    // The playlist's title format is the app's own setting as well (see
    // `config::PLAYLIST_TITLE_FORMAT_KEY`). A format string that cannot be
    // used is reported, and the rows keep their plain text.
    let playlist_title_format = loaded.playlist_title_format;
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
        // A media that is only added must not replace the queue, so it is not
        // handed to the app as the media to play.
        media: if plays { args.media.clone() } else { None },
        seconds: args.seconds,
    };
    // The device is built here, on the thread that calls `window::run` below.
    // On Windows that order is only safe because `rustamp-output` keeps every
    // COM initialisation on its own threads: `winit` needs this thread free to
    // enter a single-threaded apartment when it creates the window. See
    // `output::create_output`.
    let mut app = App::new(options)?;
    app.set_playlist_font_size(playlist_font_size);
    if let Some(setting) = playlist_title_format
        && let Err(reason) = app.set_playlist_title_format(setting.enabled, &setting.format)
    {
        app.note(format!("the playlist title format is not used: {reason}"));
    }
    app.set_config_path(config_path);
    for warning in &loaded.warnings {
        app.note(warning.clone());
    }
    if bookmarked {
        app.note(
            "the bookmark store does not exist yet (RFC 0005 O12): the path was added to the \
             playlist instead"
                .to_owned(),
        );
    } // Start the plugin host when it is resolvable (see `crate::supervise`):
    // best-effort by contract, so a missing binary only means the player runs
    // detached — it never fails startup and never opens a window. The host
    // reads the operator's approval decisions from the store beside the
    // configuration file; without that argument it approves nothing and
    // refuses every load.
    let approvals = app.plugin_approvals_path();
    app.manage_host(crate::supervise::Supervisor::new(
        crate::supervise::ProcessBackend::from_search().with_approvals(approvals),
    ));

    // ----- a Winamp configuration to import (S12) -----
    match &args.import_winamp_ini {
        cli::WinampIniImport::None => {}
        cli::WinampIniImport::File(path) => import_winamp_ini(&mut app, path),
        cli::WinampIniImport::Ask => {
            if args.output == OutputChoice::Null {
                // The native dialog needs an owner window, and the headless
                // run has none.
                eprintln!(
                    "rustamp: the Winamp configuration import needs a window; name the file on \
                     the command line"
                );
            } else {
                // The presenter opens the dialog and hands the file back.
                app.request_winamp_ini_import();
            }
        }
    }

    match args.output {
        // ----- headless test mode: no window -----
        OutputChoice::Null => {
            println!("rustamp: output null (headless; no window)");
            // The headless loop plays the queue to its end. When the command
            // line only asked for an item to be added (`--enqueue`, `/ADD`,
            // `/BOOKMARK`), nothing plays and the loop would wait forever.
            let only_added = !added.is_empty() && !plays;
            apply_startup(&mut app, plays, added)?;
            if only_added {
                // Nothing plays, so the headless loop never runs: the notes
                // the start-up raised are printed here, the way the windowed
                // presenter prints them.
                app.print_notes();
            } else {
                app.run_headless()?;
            }
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
            apply_startup(&mut app, plays, added)?;
            window::run(app)
        }
    }
}

/// Applies what the command line asked the player to do.
///
/// The items that are only **added** come first (a bookmark, an enqueued
/// file), then the playback of the file argument (S02 §1: a file given on the
/// command line starts playing it).
fn apply_startup(
    app: &mut App,
    plays: bool,
    added: Vec<MediaLocation>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !added.is_empty() {
        app.submit(Command::AddItems {
            items: added,
            at: None,
            play: false,
        })?;
    }
    if plays {
        app.submit(Command::Play)?;
    }
    Ok(())
}
