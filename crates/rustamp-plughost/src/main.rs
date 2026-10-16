//! # rustamp-plughost32
//!
//! The i686 helper process that loads classic Winamp plugin DLLs and talks to
//! `rustamp.exe` over `rustamp-ipc`.
//!
//! The process owns exactly one window — the hidden fake `Winamp v1.x` main
//! window that plugins query — and answers the app over the framed stdio
//! control channel the app connects (RFC 0006 §8; spawned with piped stdio by
//! `rustamp-app/src/supervise.rs`).
//!
//! # Main flow
//!
//! 1. Build a fresh [`IpcSurface`] and create the host window on the UI thread
//!    (this thread). Creation publishes the window handle into the surface, so
//!    the window-selector ids report it from the first plugin query on.
//! 2. Wrap stdin/stdout in the framed [`StreamTransport`] the app already
//!    speaks and hand it to a reader thread running
//!    [`control::run_reader`]: it sends the startup `Hello`, answers each
//!    `Ping` with `Pong`, forwards `PlayerStateUpdate` snapshots to the UI
//!    thread over an `mpsc` channel, and returns on `Shutdown`, on a closed
//!    pipe, or on a channel error.
//! 3. The UI thread runs [`run_ui_loop`]: drain every queued snapshot through
//!    the state bridge, pump one window message without blocking, sleep one
//!    millisecond. It stops when the reader is done (or on `WM_QUIT`).
//! 4. The process exits: `0` on `Shutdown` or a closed channel, `1` when the
//!    window cannot be created or the channel errored.
//!
//! # What this process never does
//!
//! * It never prints to **stdout**: stdout is the framed control channel and a
//!   stray line would corrupt it. Diagnostics go to stderr (which the
//!   supervisor inherits) or nowhere.
//! * It never opens a console window ([`windows_subsystem`](https://doc.rust-lang.org/rustc/codegen-options/index.html#windows-subsystem)
//!   below) and no window besides the one host window; the app additionally
//!   spawns it with `CREATE_NO_WINDOW`.
//! * It invents no IPC answers: ids that need a plugin DLL or a playing
//!   reference keep the surface's honest behaviour (see `src/ipc.rs` and
//!   `src/state_bridge.rs`).
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004); see `LICENSE-MIT` and
//! `LICENSE-APACHE` at the repository root. The host binary is a separate
//! program from the GPL player and imposes no copyleft on the plugins it loads.

// A console window counts as a window on the host: without this a
// console-subsystem child gets a console window of its own when the parent has
// no console. Has no effect on non-Windows targets.
#![windows_subsystem = "windows"]

#[cfg(windows)]
use std::io;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::sync::{Arc, mpsc};
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use rustamp_ipc::StreamTransport;
#[cfg(windows)]
use rustamp_plughost::control::{self, ControlExit};
#[cfg(windows)]
use rustamp_plughost::message_loop;
#[cfg(windows)]
use rustamp_plughost::state_bridge;
#[cfg(windows)]
use rustamp_plughost::{HostWindow, IpcSurface};
#[cfg(windows)]
use rustamp_proto::state::PlayerState;

/// Poll quantum of the UI loop: one non-blocking window pump plus one state
/// drain per quantum. Small enough that a plugin `SendMessage` never waits
/// long, large enough to stay out of a busy spin.
#[cfg(windows)]
const UI_POLL: Duration = Duration::from_millis(1);

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    #[cfg(windows)]
    {
        run_windows()
    }
    #[cfg(not(windows))]
    {
        eprintln!("rustamp-plughost32: the plugin host runs on Windows only");
        1
    }
}

/// Create the host window, serve the control channel, run until shutdown.
#[cfg(windows)]
fn run_windows() -> i32 {
    let mut surface = IpcSurface::new();
    let _window = match HostWindow::create(&mut surface) {
        Ok(window) => window,
        Err(error) => {
            eprintln!("rustamp-plughost32: cannot create host window: {error}");
            return 1;
        }
    };

    // The app's end of this pipe pair is the `StreamTransport` the supervisor
    // builds over the child's stdout/stdin: we read what it writes (stdin)
    // and write where it reads (stdout). The transport moves to the reader
    // thread; see `control::run_reader` for why reads stay off the UI thread.
    let mut transport = StreamTransport::new(io::stdin(), io::stdout());
    let (state_tx, state_rx) = mpsc::channel::<PlayerState>();
    let reader_done = Arc::new(AtomicBool::new(false));
    let reader_flag = Arc::clone(&reader_done);
    let reader = std::thread::spawn(move || {
        let exit = control::run_reader(&mut transport, &mut |state| {
            let _ = state_tx.send(state);
        });
        reader_flag.store(true, Ordering::SeqCst);
        exit
    });

    run_ui_loop(&mut surface, state_rx, &reader_done);

    // The UI loop only leaves after the reader set the flag (nothing else
    // destroys the window), so the reader has returned and joining cannot
    // block: a blocked stdin read dies with the process, never with a join.
    match reader.join() {
        Ok(ControlExit::ChannelError) => 1,
        Ok(_) => 0,
        Err(_) => 1,
    }
}

/// Drain queued player snapshots into the surface and pump window messages
/// until the reader thread reports done (or `WM_QUIT` arrives).
///
/// Snapshots apply in arrival order; the last one wins. A final drain after
/// the loop applies anything that arrived with the shutdown itself.
#[cfg(windows)]
fn run_ui_loop(
    surface: &mut IpcSurface,
    state_rx: mpsc::Receiver<PlayerState>,
    reader_done: &AtomicBool,
) {
    loop {
        for state in state_rx.try_iter() {
            state_bridge::apply_player_state(surface, &state);
        }
        if reader_done.load(Ordering::SeqCst) {
            break;
        }
        if !message_loop::pump_once(surface) {
            break;
        }
        std::thread::sleep(UI_POLL);
    }
    for state in state_rx.try_iter() {
        state_bridge::apply_player_state(surface, &state);
    }
}
