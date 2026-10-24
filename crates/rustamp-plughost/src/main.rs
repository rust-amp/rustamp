//! # rustamp-plughost32
//!
//! The helper that loads classic Winamp plugin DLLs in an i686 build. It
//! serves a window, talks to `rustamp.exe` over `rustamp-ipc`, and answers a
//! `LoadPlugin` request on its UI thread through
//! [`rustamp_plughost::registry::PluginRegistry`]. Surface tests do not prove
//! DLL loading; the VM-only acceptance path is `docs/tasks/P3-dll-acceptance.md`
//! and `docs/tasks/W23-p3-acceptance.md`.
//!
//! The process owns exactly one window — the hidden fake `Winamp v1.x` main
//! window that plugins query — and answers the app over the framed stdio
//! control channel the app connects (RFC 0006 §8; spawned with piped stdio by
//! `rustamp-app/src/supervise.rs`).
//!
//! # Which plugins may execute here
//!
//! Only bytes the operator's approval store approves, named with
//! `--approvals <path>`. With no such argument the store is empty and **no**
//! plugin loads; a store that exists but cannot be read refuses every load and
//! says why, rather than degrading into an empty allowlist. Nothing is ever
//! approved because its path, name or architecture looks right.
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
//!    the state bridge, handle a queued load request against the approval
//!    store, open the visualization plug-in's configuration dialog when the
//!    player asks for it (RFC 0005 §5.4), pump one window message without
//!    blocking, sleep one millisecond. It stops when the reader is done (or on
//!    `WM_QUIT`).
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
use std::ffi::c_void;
#[cfg(windows)]
use std::io;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
#[cfg(windows)]
use std::sync::{Arc, mpsc};
#[cfg(windows)]
use std::time::Duration;

#[cfg(windows)]
use rustamp_ipc::{Message, StreamTransport, Transport};
#[cfg(windows)]
use rustamp_plughost::control::{self, ControlExit, ControlInbound};
#[cfg(windows)]
use rustamp_plughost::message_loop;
#[cfg(windows)]
use rustamp_plughost::registry::PluginRegistry;
#[cfg(windows)]
use rustamp_plughost::{HostWindow, IpcSurface};
#[cfg(windows)]
use rustamp_proto::state::PlayerState;

/// Poll quantum of the UI loop: one non-blocking window pump plus one state
/// drain per quantum. Small enough that a plugin `SendMessage` never waits
/// long, large enough to stay out of a busy spin.
#[cfg(windows)]
const UI_POLL: Duration = Duration::from_millis(1);

/// Maximum inbound messages processed per pump quantum (work budget).
#[cfg(windows)]
const PUMP_WORK_BUDGET: usize = 16;

/// Bounded queue capacities for inbound and outbound channels.
#[cfg(windows)]
const INBOUND_QUEUE_BOUND: usize = 16;
#[cfg(windows)]
const OUTBOX_QUEUE_BOUND: usize = 16;

#[cfg(windows)]
const DUPLICATE_SAME_ACCESS: u32 = 0x0000_0002;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThread() -> *mut c_void;
    fn GetCurrentProcess() -> *mut c_void;
    fn DuplicateHandle(
        source_process: *mut c_void,
        source_handle: *mut c_void,
        target_process: *mut c_void,
        target_handle: *mut *mut c_void,
        desired_access: u32,
        inherit_handle: i32,
        options: u32,
    ) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
    fn CancelSynchronousIo(thread: *mut c_void) -> i32;
}

/// Why the UI loop exited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UiLoopExit {
    /// The reader thread finished (shutdown requested, closed, or error).
    ReaderDone,
    /// `WM_QUIT` was received by the message loop.
    WmQuit,
}

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

/// Path of the approval store, from `--approvals <path>` or `--approvals=<path>`.
///
/// `None` means the operator named none, which approves nothing. The argument
/// names *where the operator's decision is recorded*; it is not itself an
/// approval, and no environment variable can stand in for it.
#[cfg(windows)]
fn approval_store_path() -> Option<std::path::PathBuf> {
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        if let Some(rest) = text.strip_prefix("--approvals=") {
            return Some(std::path::PathBuf::from(rest));
        }
        if text == "--approvals" {
            return args.next().map(std::path::PathBuf::from);
        }
    }
    None
}

/// Create the host window, serve the control channel, run until shutdown.
#[cfg(windows)]
fn run_windows() -> i32 {
    // The fault filter goes up before anything else can run plugin code: from
    // now on an unhandled exception (a plugin that dereferences a null
    // pointer, say) is reported on stderr and ends the host with the exit
    // code the supervisor recognises (`rustamp_plughost::fault`).
    let _fault_guard = rustamp_plughost::fault::install_fault_guard();

    let surface = IpcSurface::new();
    let window = match HostWindow::create(surface) {
        Ok(window) => window,
        Err(error) => {
            eprintln!("rustamp-plughost32: cannot create host window: {error}");
            return 1;
        }
    };

    // Declared after the window on purpose: the registry quits its plugins
    // when it drops, and that must happen while the window they were handed
    // still exists (S09 §7.2; the window outlives every plugin).
    let store_path = approval_store_path();
    let mut registry = PluginRegistry::from_store_path(store_path.as_deref());
    match registry.approved_count() {
        Some(count) => eprintln!("rustamp-plughost32: {count} approved plugin(s)"),
        None => eprintln!(
            "rustamp-plughost32: the approval store could not be read; every load will be refused"
        ),
    }

    // Finding 9: Inbound and outbound channels are bounded.
    let (inbound_tx, inbound_rx) = mpsc::sync_channel::<ControlInbound>(INBOUND_QUEUE_BOUND);
    let (outbox_tx, outbox_rx) = mpsc::sync_channel::<Message>(OUTBOX_QUEUE_BOUND);

    // Dedicated writer thread for stdout. Pong is only queued when the UI
    // loop actually pumps messages (Finding 6), so a hung UI will not answer pings.
    let _writer = std::thread::Builder::new()
        .name("rustamp-host-writer".to_string())
        .spawn(move || {
            let mut writer = StreamTransport::new(io::empty(), io::stdout());
            for msg in outbox_rx {
                if writer.send(&msg).is_err() {
                    break;
                }
            }
        });

    // Send the startup Hello over the writer channel.
    let _ = outbox_tx.send(Message::hello());

    let reader_done = Arc::new(AtomicBool::new(false));
    let reader_flag = Arc::clone(&reader_done);
    let reader_thread_handle = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
    let reader_handle_clone = Arc::clone(&reader_thread_handle);
    let host_hwnd = window.hwnd_raw();

    // Reader thread: reads stdin, forwards events to UI thread via bounded channel.
    let reader = std::thread::spawn(move || {
        // Record duplicated thread handle so the UI thread can unblock I/O on WM_QUIT (Finding 7).
        let mut dup_raw = std::ptr::null_mut();
        let ok = unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                GetCurrentThread(),
                GetCurrentProcess(),
                &mut dup_raw,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        };
        if ok != 0 {
            reader_handle_clone.store(dup_raw, Ordering::SeqCst);
        }

        let mut transport = StreamTransport::new(io::stdin(), io::sink());
        let exit = control::run_reader_events(&mut transport, &mut |event| {
            let _ = inbound_tx.send(event);
        });

        reader_flag.store(true, Ordering::SeqCst);

        // Finding 6: Parent-death termination independent of the UI.
        // If the parent closed the pipe (or asked for shutdown), post WM_QUIT
        // so a responsive UI loop exits cleanly. Additionally start a timer:
        // if the UI thread is hung in a plugin DLL, terminate unconditionally
        // rather than lingering as an orphaned process.
        if host_hwnd != 0 {
            unsafe {
                let hwnd = windows::Win32::Foundation::HWND(host_hwnd as _);
                let _ = windows::Win32::UI::WindowsAndMessaging::PostMessageW(
                    Some(hwnd),
                    windows::Win32::UI::WindowsAndMessaging::WM_QUIT,
                    windows::Win32::Foundation::WPARAM(0),
                    windows::Win32::Foundation::LPARAM(0),
                );
            }
        }
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            std::process::exit(match exit {
                ControlExit::ChannelError => 1,
                _ => 0,
            });
        });

        exit
    });

    let exit_reason = run_ui_loop(&window, &mut registry, inbound_rx, &outbox_tx, &reader_done);

    // Quit the plugins while the window is still alive and on this thread.
    // Reverse load order; the DLLs stay mapped (S09 §1 line 46).
    registry.quit_all();

    // Finding 7: WM_QUIT can strand join on a blocked stdin reader.
    // Exit reasons are distinguished, and I/O is unblocked before joining.
    match exit_reason {
        UiLoopExit::ReaderDone => match reader.join() {
            Ok(ControlExit::ChannelError) => 1,
            Ok(_) => 0,
            Err(_) => 1,
        },
        UiLoopExit::WmQuit => {
            // Unblock I/O on the reader thread before joining.
            let raw_h = reader_thread_handle.load(Ordering::SeqCst);
            if !raw_h.is_null() {
                unsafe {
                    let _ = CancelSynchronousIo(raw_h);
                    let _ = CloseHandle(raw_h);
                }
            }
            let _ = reader.join();
            0
        }
    }
}

/// Drain queued player snapshots, pings and load requests, pumping window
/// messages with a per-pump work budget and coalescing snapshots (Finding 9).
///
/// A load request is the one event that can take arbitrarily long: it runs the
/// plugin's `DllMain` and `init` on this thread (S09 §3.5, §7.2), and there is
/// no isolation. That is why it is answered here rather than on the reader
/// thread, and why the reply goes out through `outbox_tx` instead of a print:
/// stdout is the frame stream.
#[cfg(windows)]
fn run_ui_loop(
    window: &HostWindow,
    registry: &mut PluginRegistry,
    inbound_rx: mpsc::Receiver<ControlInbound>,
    outbox_tx: &mpsc::SyncSender<Message>,
    reader_done: &AtomicBool,
) -> UiLoopExit {
    let exit_reason;

    loop {
        // Finding 9: Work budget per pump and coalesced snapshots.
        let mut budget = PUMP_WORK_BUDGET;
        let mut latest_state: Option<PlayerState> = None;
        while budget > 0 {
            match inbound_rx.try_recv() {
                Ok(ControlInbound::State(state)) => {
                    latest_state = Some(state);
                    budget -= 1;
                }
                Ok(ControlInbound::Ping(token)) => {
                    // Finding 6: Pong is generated here when UI pumps, confirming UI liveness.
                    let _ = outbox_tx.try_send(Message::Pong(token));
                    budget -= 1;
                }
                Ok(ControlInbound::Load { path, kind }) => {
                    // SAFETY: this is the UI thread owning `window`, and the
                    // approval store is the operator's decision about which
                    // code may execute in this process.
                    let reply = unsafe { registry.handle_load_request(&path, kind, window) };
                    let _ = outbox_tx.try_send(reply);
                    budget -= 1;
                }
                Ok(request) => {
                    // A plug-in request (RFC 0005 §5.4, RFC 0020): the running
                    // visualization's Configure request, or one of the five that
                    // name a plug-in by its id. They run here, on the thread that
                    // owns the window the plug-in was handed as its parent, and
                    // they are fire-and-forget: a refusal is a diagnostic on
                    // stderr (stdout is the frame stream), never a reply frame.
                    if let Err(message) = control::apply_plugin_request(registry, &request) {
                        eprintln!("rustamp-plughost32: {message}");
                    }
                    budget -= 1;
                }
                Err(mpsc::TryRecvError::Empty) | Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }

        // Apply only the latest state from the drained quantum (coalesced).
        if let Some(state) = latest_state {
            window
                .apply_player_state(&state)
                .expect("UI state updates do not reenter surface access");
        }

        if reader_done.load(Ordering::SeqCst) {
            exit_reason = UiLoopExit::ReaderDone;
            break;
        }
        if !message_loop::pump_once(window) {
            exit_reason = UiLoopExit::WmQuit;
            break;
        }
        // S09 §3.6: an input plug-in's decode thread posts the end-of-track
        // notification `0x402` to the host window, and the host reports it to
        // its caller. The player owns the playlist, so the host advances
        // nothing itself; the report goes to stderr, which the supervisor
        // inherits, because stdout is the frame stream.
        let ended = rustamp_plughost::handoff::take_end_of_track();
        if ended > 0 {
            eprintln!(
                "rustamp-plughost32: the input plug-in reported {ended} end-of-track notification(s)"
            );
        }
        std::thread::sleep(UI_POLL);
    }

    // Final drain: coalesce any remaining snapshots that arrived with shutdown.
    // A load or a Configure request that arrived with the shutdown is
    // deliberately dropped unanswered: the process is on its way out, and
    // mapping a DLL or opening a plugin's dialog now would run plugin code the
    // host is about to stop pumping messages for.
    let mut latest_state: Option<PlayerState> = None;
    for event in inbound_rx.try_iter() {
        if let ControlInbound::State(state) = event {
            latest_state = Some(state);
        }
    }
    if let Some(state) = latest_state {
        window
            .apply_player_state(&state)
            .expect("UI state updates do not reenter surface access");
    }

    exit_reason
}
