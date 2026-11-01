// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The local control and inspection channel (`--control`).
//!
//! A running player can be driven and inspected from another process on the
//! same machine: `rustamp --control[=<name>]` opens a **named pipe**
//! (`\\.\pipe\rustamp-ctl-<key>`) and answers one JSON object per line.
//!
//! # Provenance
//!
//! idea from oofz/rustamp dev_server/dev_client (MIT OR Apache-2.0),
//! implementation ours (named pipe, typed serde, request ids) — no source text
//! copied. No code of that project is used, and no `THIRD_PARTY.md` row is
//! added for it: nothing was copied.
//!
//! # Why a named pipe and not a socket
//!
//! A named pipe is created in the local namespace of the logon session
//! (`CreateNamedPipeW` on `\\.\pipe\...`), so another user's session cannot
//! reach it, and nothing listens on a TCP port that a firewall or another
//! machine could see. The name carries the **instance key** of
//! [`rustamp_platform::single_instance::instance_key`], so two players with
//! different configurations (or `/CLASS=` names) never share a channel, and a
//! test that names its own channel never reaches the player the developer is
//! running.
//!
//! # The two stages
//!
//! * **Stage 1 — inspection (read-only):** `ping`, `snapshot` and `wait`.
//!   `snapshot` is answered from the [`PlayerState`] the pump already
//!   computes ([`App::host_player_state`](crate::app::App::host_player_state)),
//!   so it never round-trips through the UI thread: a modal dialog cannot
//!   block it. `wait` blocks on the connection thread over its own
//!   [`EventBus::subscribe`] receiver until the wanted event is **published**
//!   — a conditional wait on the real event stream, not a sleep.
//! * **Stage 2 — commands:** a `cmd` is a [`Command`] that goes through the
//!   **existing** [`CommandSender`], i.e. the same funnel a keystroke uses.
//!   The channel never applies anything itself.
//!
//! # The gate
//!
//! `--control` is its own switch (see [`crate::cli::Control`]), never an
//! environment variable and never `debug_assertions`. The channel is gated by
//! the same window rail as the run itself ([`gate`], which is
//! [`crate::window::ensure_gui_allowed`]): a windowed run that the rail
//! refuses never reaches the point where the pipe would be created, and a
//! headless run (`--null`) cannot open a window at all. So the channel can
//! never smuggle a window onto the host.
//!
//! # The wire format
//!
//! One JSON object per line, in both directions. A request is exactly one of:
//!
//! ```text
//! {"id":7,"cmd":<Command>}
//! {"id":8,"op":"snapshot"}
//! {"id":9,"op":"wait","event":<EventFilter>,"timeout_ms":5000}
//! {"id":10,"op":"ping"}
//! ```
//!
//! An answer is:
//!
//! ```text
//! {"id":7,"ok":true,"message":"...","state":<PlayerState|null>,"events":[<Event>...]}
//! ```
//!
//! The `id` is mandatory and echoed unchanged, so a client keeps its own
//! bookkeeping without a session. A line that is not a request at all has no
//! id to echo, so its refusal carries [`UNKNOWN_ID`] (0). The types are this
//! module's own; `Command`, `Event` and `PlayerState` are the `rustamp-proto`
//! wire types, unchanged (no contract change).

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rustamp_core::bus::{CommandSender, EventBus, EventReceiver};
use rustamp_core::command::Command;
use rustamp_core::event::Event;
use rustamp_core::state::PlayerState;
use serde::{Deserialize, Serialize};

/// The longest accepted request line, in bytes.
///
/// A request carries one command, so this is a generous ceiling that still
/// bounds what a client can make the listener allocate.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// The `wait` timeout a request gets when it names none.
pub const DEFAULT_WAIT_MS: u64 = 5_000;

/// The longest `wait` a request may ask for.
///
/// A wait holds one connection thread and nothing else, but an unbounded wait
/// would let a client pin that thread for the process's whole life.
pub const MAX_WAIT_MS: u64 = 600_000;

/// The id an answer carries when the request line named none.
pub const UNKNOWN_ID: u64 = 0;

/// How many connections the pipe serves at once.
///
/// The pipe itself accepts more clients (the operating system queues them);
/// this is the number of instances the listener creates, so a client that
/// connects while every instance is busy waits for the next one.
pub const MAX_INSTANCES: u32 = 8;

/// The pipe an instance key is served on.
///
/// The name is in the local pipe namespace and carries the instance key, so
/// only this logon session can reach it and two instances never collide.
#[cfg(windows)]
pub fn pipe_name(key: &str) -> String {
    format!(r"\\.\pipe\rustamp-ctl-{key}")
}

/// The socket an instance key is served on (off Windows the channel is a Unix
/// domain socket, the counterpart of the Windows named pipe).
///
/// It lives in the user's runtime directory when there is one, so only this
/// user can reach it, and carries the instance key, so two instances never
/// collide.
#[cfg(not(windows))]
pub fn pipe_name(key: &str) -> String {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    dir.join(format!("rustamp-ctl-{key}.sock"))
        .to_string_lossy()
        .into_owned()
}

/// Refuses the channel when the run's own window rail refuses it.
///
/// A windowed run (`Default`, `NoAudio`) opens a window, and
/// [`crate::window::ensure_gui_allowed`] is the rule that keeps a GUI run off
/// the host. A headless run (`Null`) opens no window, so there is nothing the
/// rail could refuse — and the rail variable is exactly what the app's own
/// test harness sets, so consulting it there would refuse every headless
/// control test.
pub fn gate(output: crate::cli::OutputChoice) -> Result<(), Box<dyn std::error::Error>> {
    match output {
        crate::cli::OutputChoice::Null => Ok(()),
        crate::cli::OutputChoice::Default | crate::cli::OutputChoice::NoAudio => {
            crate::window::ensure_gui_allowed()
        }
    }
}

/// The handles the channel answers from.
///
/// The app hands these over once ([`crate::app::App::control_handles`]); every
/// connection thread clones them. Nothing here is the app: the channel only
/// sends commands and reads state, so it can never diverge from the player.
#[derive(Clone)]
pub struct Handles {
    /// The app's command funnel (`rustamp-core`'s [`CommandSender`]).
    pub commands: CommandSender,
    /// The event bus, for `wait`.
    pub bus: EventBus,
    /// The snapshot the pump publishes.
    pub state: Arc<Mutex<PlayerState>>,
}

impl std::fmt::Debug for Handles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Handles")
            .field("subscribers", &self.bus.subscriber_count())
            .finish_non_exhaustive()
    }
}

/// The operations that do not send a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// Answer with the current snapshot.
    Ping,
    /// Answer with the current snapshot (the inspection request).
    Snapshot,
    /// Wait for an event that matches `event`.
    Wait,
}

/// Which events a `wait` request waits for.
///
/// The filter is typed and closed: a client names one of these values, so an
/// unknown filter is a refused request and not a silently ignored one. The
/// values are the variants of [`Event`] a control client waits for; a variant
/// that is not here can still be observed through `snapshot` or by waiting
/// for [`EventFilter::Any`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventFilter {
    /// Every event.
    Any,
    /// A transport state change ([`Event::StateChanged`]).
    StateChanged,
    /// A track change ([`Event::TrackChanged`]).
    TrackChanged,
    /// A playback position update, including the one a seek publishes
    /// ([`Event::PositionChanged`]).
    PositionChanged,
    /// A volume or balance change ([`Event::VolumeChanged`]).
    VolumeChanged,
    /// A playlist change ([`Event::PlaylistChanged`]).
    PlaylistChanged,
    /// The end of the playlist ([`Event::PlaylistEnded`]).
    PlaylistEnded,
}

impl EventFilter {
    /// Whether `event` is the kind this filter asks for.
    pub fn matches(self, event: &Event) -> bool {
        match self {
            EventFilter::Any => true,
            EventFilter::StateChanged => matches!(event, Event::StateChanged(_)),
            EventFilter::TrackChanged => matches!(event, Event::TrackChanged { .. }),
            EventFilter::PositionChanged => matches!(event, Event::PositionChanged { .. }),
            EventFilter::VolumeChanged => matches!(event, Event::VolumeChanged { .. }),
            EventFilter::PlaylistChanged => matches!(event, Event::PlaylistChanged { .. }),
            EventFilter::PlaylistEnded => matches!(event, Event::PlaylistEnded),
        }
    }
}

/// One request line.
///
/// `id` has no default, so a request without it is refused instead of being
/// answered with an id the client never sent. Exactly one of `cmd` and `op`
/// must be present.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// The client's request id; echoed unchanged in the answer.
    pub id: u64,
    /// The command to hand to the player (stage 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<Command>,
    /// The operation to run when no command is sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<Op>,
    /// The event filter of a `wait` request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<EventFilter>,
    /// The `wait` timeout in milliseconds ([`DEFAULT_WAIT_MS`] when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

impl Request {
    /// A request that sends `cmd`.
    pub fn command(id: u64, cmd: Command) -> Self {
        Request {
            id,
            cmd: Some(cmd),
            op: None,
            event: None,
            timeout_ms: None,
        }
    }

    /// A request that runs `op`.
    pub fn operation(id: u64, op: Op) -> Self {
        Request {
            id,
            cmd: None,
            op: Some(op),
            event: None,
            timeout_ms: None,
        }
    }

    /// A `wait` request for `event`, giving up after `timeout_ms`.
    pub fn wait(id: u64, event: EventFilter, timeout_ms: u64) -> Self {
        Request {
            id,
            cmd: None,
            op: Some(Op::Wait),
            event: Some(event),
            timeout_ms: Some(timeout_ms),
        }
    }
}

/// One answer line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    /// The id of the request this answers.
    pub id: u64,
    /// Whether the request was carried out.
    pub ok: bool,
    /// What happened, in words the client can show.
    pub message: String,
    /// The player state at the time of the answer (`None` on a refusal).
    pub state: Option<PlayerState>,
    /// The event a `wait` matched; empty for every other request.
    pub events: Vec<Event>,
}

impl Response {
    /// A successful answer that carries the current snapshot.
    fn ok(id: u64, message: String, state: PlayerState) -> Self {
        Response {
            id,
            ok: true,
            message,
            state: Some(state),
            events: Vec::new(),
        }
    }

    /// A refusal: the request was read but not carried out.
    fn failed(id: u64, message: String) -> Self {
        Response {
            id,
            ok: false,
            message,
            state: None,
            events: Vec::new(),
        }
    }
}

/// The current snapshot, after copy.
///
/// A panic while a snapshot was written leaves the mutex poisoned; the
/// lock is recovered, so a panic in the player can never wedge the channel.
fn snapshot(handles: &Handles) -> PlayerState {
    let guard = handles
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.clone()
}

/// Waits on `events` until an event matches `filter`, or the timeout passes.
///
/// Events that were published after the subscription and not yet read are
/// drained first, so a `wait` that was issued right after a command does not
/// miss the event the command published in between.
fn wait_for(events: &EventReceiver, filter: EventFilter, timeout: Duration) -> Option<Event> {
    let deadline = Instant::now() + timeout;
    loop {
        while let Ok(Some(event)) = events.try_recv() {
            if filter.matches(&event) {
                return Some(event);
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match events.recv_timeout(remaining) {
            Ok(Some(event)) => {
                if filter.matches(&event) {
                    return Some(event);
                }
            }
            // The timeout of the last window, or a bus that is gone.
            Ok(None) | Err(_) => return None,
        }
    }
}

/// Answers one request.
///
/// Pure apart from the three handles: the command goes into the app's funnel,
/// the snapshot is read from the shared state, and a `wait` blocks on the
/// connection's own event receiver.
fn answer(request: &Request, handles: &Handles, events: &EventReceiver) -> Response {
    let id = request.id;
    match (&request.cmd, request.op) {
        // Stage 2: the command travels the same funnel a keystroke uses.
        (Some(cmd), None) => match handles.commands.send(cmd.clone()) {
            Ok(()) => Response::ok(id, "command accepted".to_owned(), snapshot(handles)),
            Err(error) => Response::failed(id, format!("the command was not accepted: {error}")),
        },
        (None, Some(Op::Ping)) => Response::ok(id, "pong".to_owned(), snapshot(handles)),
        (None, Some(Op::Snapshot)) => Response::ok(id, "snapshot".to_owned(), snapshot(handles)),
        (None, Some(Op::Wait)) => {
            let Some(filter) = request.event else {
                return Response::failed(id, "wait needs an \"event\" filter".to_owned());
            };
            let timeout_ms = request
                .timeout_ms
                .unwrap_or(DEFAULT_WAIT_MS)
                .min(MAX_WAIT_MS);
            match wait_for(events, filter, Duration::from_millis(timeout_ms)) {
                Some(event) => {
                    let mut response = Response::ok(id, "event".to_owned(), snapshot(handles));
                    response.events.push(event);
                    response
                }
                None => Response::failed(
                    id,
                    format!("no matching {filter:?} event within {timeout_ms} ms"),
                ),
            }
        }
        (Some(_), Some(_)) => Response::failed(
            id,
            "\"cmd\" and \"op\" are exclusive: name one of them".to_owned(),
        ),
        (None, None) => Response::failed(
            id,
            "the request names neither \"cmd\" nor \"op\"".to_owned(),
        ),
    }
}

/// Reads newline-terminated lines from a byte stream.
///
/// The pipe is in byte mode, so a line can arrive in several reads and two
/// lines in one. A line is returned without its `\r\n` (or `\n`), and a line
/// longer than [`MAX_LINE_BYTES`] is refused with an error that ends the
/// connection instead of being buffered without bound.
#[derive(Debug, Default)]
struct LineBuffer {
    /// Bytes read but not consumed yet.
    pending: Vec<u8>,
    /// How far into `pending` the next line starts.
    read: usize,
}

impl LineBuffer {
    /// The next line, or `None` at the end of the stream.
    fn next(&mut self, from: &mut impl std::io::Read) -> std::io::Result<Option<String>> {
        loop {
            if let Some(offset) = self.pending[self.read..]
                .iter()
                .position(|byte| *byte == b'\n')
            {
                let end = self.read + offset;
                let line = self.without_terminator(end);
                self.read = end + 1;
                self.compact();
                return Ok(Some(line));
            }
            if self.pending.len() - self.read > MAX_LINE_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("a request line may be at most {MAX_LINE_BYTES} bytes long"),
                ));
            }
            let mut chunk = [0u8; 4096];
            let count = from.read(&mut chunk)?;
            if count == 0 {
                // The client closed the connection (or the pipe broke).
                if self.read == self.pending.len() {
                    return Ok(None);
                }
                // A last line without its terminator is still a line.
                let line = self.without_terminator(self.pending.len());
                self.pending.clear();
                self.read = 0;
                return Ok(Some(line));
            }
            self.pending.extend_from_slice(&chunk[..count]);
        }
    }

    /// The line `[read, end)` as text, without the line terminator.
    fn without_terminator(&self, end: usize) -> String {
        let bytes = &self.pending[self.read..end];
        let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// Drops the consumed prefix once nothing is left to read.
    fn compact(&mut self) {
        if self.read == self.pending.len() {
            self.pending.clear();
            self.read = 0;
        }
    }
}

/// Serves one connection: one answer line per request line.
///
/// The connection's own event subscription lives as long as the connection, so
/// a `wait` sees every event published after the client connected.
fn serve(mut file: impl std::io::Read + std::io::Write, handles: Handles) {
    let events = handles.bus.subscribe();
    let mut lines = LineBuffer::default();
    // The loop ends at the end of the stream, on a line this build refuses,
    // or on a read error: the connection is done.
    while let Ok(Some(line)) = lines.next(&mut file) {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => answer(&request, &handles, &events),
            Err(error) => Response::failed(
                UNKNOWN_ID,
                format!("the line is not a control request: {error}"),
            ),
        };
        let Ok(mut text) = serde_json::to_string(&response) else {
            break;
        };
        text.push('\n');
        if file.write_all(text.as_bytes()).is_err() || file.flush().is_err() {
            break;
        }
    }
}

/// The listener for one instance key.
///
/// `start` prepares the first pipe instance, so a name the operating system
/// refuses is reported to the caller instead of being swallowed by a thread.
/// Dropping the server stops the listener (like the single-instance guard);
/// a connection that is still open simply ends with the process.
pub struct Server {
    /// Held for its lifetime: the listener thread and the pipe, which the
    /// channel is reached through only from outside.
    #[allow(dead_code, reason = "held for the listener's lifetime and its drop")]
    imp: imp::Server,
}

impl Server {
    /// Starts listening on [`pipe_name`]`(key)`.
    pub fn start(key: &str, handles: Handles) -> std::io::Result<Self> {
        Ok(Server {
            imp: imp::Server::start(&pipe_name(key), handles)?,
        })
    }
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").finish_non_exhaustive()
    }
}

#[cfg(windows)]
#[allow(unsafe_code)] // the Win32 pipe boundary only; the rest of the app denies unsafe
mod imp {
    //! Win32 listener: one pipe instance per connection, byte mode, duplex.

    use std::fs::File;
    use std::os::windows::io::FromRawHandle;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, GetLastError, HANDLE};
    use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::core::PCWSTR;

    use super::{Handles, MAX_INSTANCES, serve};

    /// How long a drop waits for the listener thread to stop.
    const STOP_TIMEOUT: Duration = Duration::from_millis(500);

    /// The output buffer of one instance: a snapshot answer with events.
    const OUT_BUFFER_BYTES: u32 = 256 * 1024;

    /// The input buffer of one instance: a request line.
    const IN_BUFFER_BYTES: u32 = 64 * 1024;

    /// The listener and the state a drop needs to stop it.
    pub struct Server {
        /// The pipe the listener waits on.
        pipe: String,
        /// Set to ask the listener thread to stop.
        stop: Arc<AtomicBool>,
        /// The listener thread.
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Server {
        /// Creates the first instance and starts the listener.
        pub fn start(pipe: &str, handles: Handles) -> std::io::Result<Self> {
            let first = create_pipe(pipe)?;
            let stop = Arc::new(AtomicBool::new(false));
            let listener_stop = Arc::clone(&stop);
            let listener_pipe = pipe.to_owned();
            // The prepared handle crosses the thread boundary as an address:
            // a `HANDLE` is a raw pointer and therefore not `Send`, and the
            // thread rebuilds the handle from the value it is given.
            let first = first.0 as usize;
            let thread = std::thread::Builder::new()
                .name("rustamp-control".to_owned())
                .spawn(move || listen(&listener_pipe, Some(first), &listener_stop, handles))
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            Ok(Server {
                pipe: pipe.to_owned(),
                stop,
                thread: Some(thread),
            })
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            // Wake a listener blocked in `ConnectNamedPipe` by connecting to
            // its pipe ourselves; the connection is empty, so the listener
            // sees EOF, sees the stop flag and exits.
            let deadline = Instant::now() + STOP_TIMEOUT;
            while !self
                .thread
                .as_ref()
                .is_some_and(|thread| thread.is_finished())
            {
                if Instant::now() >= deadline {
                    break;
                }
                let _ = std::fs::OpenOptions::new().write(true).open(&self.pipe);
                std::thread::sleep(Duration::from_millis(10));
            }
            if let Some(thread) = self.thread.take()
                && thread.is_finished()
            {
                let _ = thread.join();
            }
        }
    }

    /// The listener loop: one instance per connection, one thread per client.
    ///
    /// `first` is the address of the instance [`Server::start`] already
    /// created (`None` after the first round); the listener owns it from here.
    fn listen(pipe: &str, first: Option<usize>, stop: &AtomicBool, handles: Handles) {
        let mut prepared = first.map(|address| HANDLE(address as *mut core::ffi::c_void));
        while !stop.load(Ordering::SeqCst) {
            let handle = match prepared.take() {
                Some(handle) => handle,
                None => match create_pipe(pipe) {
                    Ok(handle) => handle,
                    Err(error) => {
                        eprintln!("rustamp: the control channel stopped: {error}");
                        break;
                    }
                },
            };
            if !connect(handle) {
                // SAFETY: `handle` is a valid owned pipe handle from
                // `create_pipe`; it is closed exactly once.
                let _ = unsafe { CloseHandle(handle) };
                continue;
            }
            if stop.load(Ordering::SeqCst) {
                // Our own shutdown connection: close and leave.
                // SAFETY: `handle` is a valid owned pipe handle; closed once.
                let _ = unsafe { DisconnectNamedPipe(handle) };
                let _ = unsafe { CloseHandle(handle) };
                break;
            }
            // SAFETY: `handle` is a valid owned duplex pipe handle, and
            // ownership moves into the `File` exactly once; the `File` closes
            // (and so disconnects) it when the connection thread ends.
            let file = unsafe { File::from_raw_handle(handle.0) };
            let handles = handles.clone();
            let spawned = std::thread::Builder::new()
                .name("rustamp-control-client".to_owned())
                .spawn(move || serve(file, handles));
            if spawned.is_err() {
                // The connection is dropped with the `File`; the listener
                // goes on and serves the next client.
                continue;
            }
        }
    }

    /// Creates one listening instance of the pipe.
    fn create_pipe(pipe: &str) -> std::io::Result<HANDLE> {
        let wide = to_wide(pipe);
        let mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT;
        // SAFETY: `wide` is a NUL-terminated UTF-16 name that outlives the
        // call, the buffer sizes are constants, and no security attributes
        // are wanted (None). The returned handle is owned by us.
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(wide.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                mode,
                MAX_INSTANCES,
                OUT_BUFFER_BYTES,
                IN_BUFFER_BYTES,
                0,
                None,
            )
        };
        if handle.is_invalid() {
            // SAFETY: reading the last-error value is always sound; the value
            // is meaningful because `CreateNamedPipeW` set it.
            let code = unsafe { GetLastError() };
            return Err(std::io::Error::from_raw_os_error(code.0 as i32));
        }
        Ok(handle)
    }

    /// Blocks until a client connects; `false` on a real failure.
    fn connect(handle: HANDLE) -> bool {
        // SAFETY: `handle` is a valid pipe handle; `None` requests a blocking
        // call on the calling thread (no OVERLAPPED).
        match unsafe { ConnectNamedPipe(handle, None) } {
            Ok(()) => true,
            Err(error) => {
                // A client that connected between create and connect makes
                // `ConnectNamedPipe` report ERROR_PIPE_CONNECTED, which the
                // windows crate surfaces as an error; that is success here.
                (error.code().0 as u32) & 0xFFFF == ERROR_PIPE_CONNECTED.0
            }
        }
    }

    /// UTF-16, NUL-terminated, for the `*W` Win32 calls.
    fn to_wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(windows))]
mod imp {
    //! Unix listener: one Unix domain socket, one thread per connection.

    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    use super::{Handles, serve};

    /// How often the listener looks at its stop flag while nobody connects.
    const POLL: Duration = Duration::from_millis(20);

    /// The listener and the state a drop needs to stop it.
    #[derive(Debug)]
    pub struct Server {
        /// The socket file, removed again when the server stops.
        path: PathBuf,
        /// Set to ask the listener thread to stop.
        stop: Arc<AtomicBool>,
        /// The listener thread.
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl Server {
        /// Binds the socket and starts the listener.
        pub fn start(pipe: &str, handles: Handles) -> std::io::Result<Self> {
            let path = PathBuf::from(pipe);
            // A socket file left by a run that did not exit cleanly would make
            // `bind` fail. A live listener behind it is the other instance of
            // the same key, which the single-instance guard already refuses.
            if path.exists() && std::os::unix::net::UnixStream::connect(&path).is_err() {
                let _ = std::fs::remove_file(&path);
            }
            let listener = UnixListener::bind(&path)?;
            // Owner only: the channel can drive the player.
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            listener.set_nonblocking(true)?;
            let stop = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&stop);
            let thread = std::thread::Builder::new()
                .name("rustamp-control".to_owned())
                .spawn(move || {
                    while !flag.load(Ordering::Relaxed) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                // The accepted stream must block: `serve` reads lines.
                                if stream.set_nonblocking(false).is_err() {
                                    continue;
                                }
                                let handles = handles.clone();
                                let _ = std::thread::Builder::new()
                                    .name("rustamp-control-conn".to_owned())
                                    .spawn(move || serve(stream, handles));
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                std::thread::sleep(POLL);
                            }
                            Err(_) => std::thread::sleep(POLL),
                        }
                    }
                })?;
            Ok(Server {
                path,
                stop,
                thread: Some(thread),
            })
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::bus::command_channel;
    use rustamp_core::state::TransportState;

    fn handles() -> (Handles, rustamp_core::bus::CommandReceiver, EventBus) {
        let (commands, receiver) = command_channel();
        let bus = EventBus::new();
        (
            Handles {
                commands,
                bus: bus.clone(),
                state: Arc::new(Mutex::new(PlayerState::default())),
            },
            receiver,
            bus,
        )
    }

    fn events(bus: &EventBus) -> EventReceiver {
        bus.subscribe()
    }

    #[test]
    fn a_ping_answers_pong_with_the_snapshot() {
        let (handles, _commands, bus) = handles();
        let events = events(&bus);
        let response = answer(&Request::operation(7, Op::Ping), &handles, &events);
        assert!(response.ok, "{}", response.message);
        assert_eq!(response.id, 7);
        assert_eq!(response.message, "pong");
        assert_eq!(response.state, Some(PlayerState::default()));
        assert!(response.events.is_empty());
    }

    #[test]
    fn a_snapshot_is_answered_from_the_shared_state() {
        // The pump publishes the snapshot; the channel never asks the app.
        let (handles, _commands, bus) = handles();
        let events = events(&bus);
        {
            let mut guard = handles.state.lock().unwrap();
            guard.playlist_len = 3;
            guard.transport = TransportState::Playing;
            guard.position_ms = 1_500;
        }
        let response = answer(&Request::operation(8, Op::Snapshot), &handles, &events);
        let state = response.state.expect("the snapshot is attached");
        assert_eq!(state.playlist_len, 3);
        assert_eq!(state.transport, TransportState::Playing);
        assert_eq!(state.position_ms, 1_500);
    }

    #[test]
    fn a_command_goes_into_the_apps_command_channel() {
        let (handles, commands, bus) = handles();
        let events = events(&bus);
        let response = answer(
            &Request::command(9, Command::SetVolume(42)),
            &handles,
            &events,
        );
        assert!(response.ok, "{}", response.message);
        assert_eq!(commands.recv().unwrap(), Command::SetVolume(42));
        assert!(
            commands.try_recv().unwrap().is_none(),
            "exactly one command per request"
        );
    }

    #[test]
    fn a_wait_returns_the_matching_event_and_ignores_the_others() {
        let (handles, _commands, bus) = handles();
        let events = events(&bus);
        bus.publish(Event::VolumeChanged {
            volume: 10,
            balance: 0,
        });
        bus.publish(Event::StateChanged(TransportState::Playing));
        let response = answer(
            &Request::wait(10, EventFilter::StateChanged, 1_000),
            &handles,
            &events,
        );
        assert!(response.ok, "{}", response.message);
        assert_eq!(
            response.events,
            vec![Event::StateChanged(TransportState::Playing)],
            "the wait must skip the event it was not asked for"
        );
    }

    #[test]
    fn a_wait_blocks_until_the_event_is_published() {
        let (handles, _commands, bus) = handles();
        let events = events(&bus);
        let publisher = bus.clone();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            publisher.publish(Event::PlaylistEnded);
        });
        let response = answer(
            &Request::wait(11, EventFilter::PlaylistEnded, 2_000),
            &handles,
            &events,
        );
        thread.join().unwrap();
        assert!(response.ok, "{}", response.message);
        assert_eq!(response.events, vec![Event::PlaylistEnded]);
    }

    #[test]
    fn a_wait_that_matches_nothing_times_out() {
        let (handles, _commands, bus) = handles();
        let events = events(&bus);
        let started = Instant::now();
        let response = answer(
            &Request::wait(12, EventFilter::StateChanged, 30),
            &handles,
            &events,
        );
        assert!(!response.ok, "a timeout is a refusal");
        assert!(response.events.is_empty());
        assert!(
            started.elapsed() >= Duration::from_millis(25),
            "the wait must actually wait"
        );
        assert!(
            response.message.contains("30 ms"),
            "the refusal names the timeout: {}",
            response.message
        );
    }

    #[test]
    fn a_wait_without_a_filter_is_refused() {
        let (handles, _commands, bus) = handles();
        let events = events(&bus);
        let request = Request {
            event: None,
            ..Request::operation(13, Op::Wait)
        };
        let response = answer(&request, &handles, &events);
        assert!(!response.ok);
        assert!(response.message.contains("event"), "{}", response.message);
    }

    #[test]
    fn a_request_with_both_a_command_and_an_op_is_refused() {
        let (handles, commands, bus) = handles();
        let events = events(&bus);
        let request = Request {
            op: Some(Op::Ping),
            ..Request::command(14, Command::Play)
        };
        let response = answer(&request, &handles, &events);
        assert!(!response.ok);
        assert!(
            commands.try_recv().unwrap().is_none(),
            "a refused request must send nothing"
        );
    }

    #[test]
    fn a_request_with_neither_a_command_nor_an_op_is_refused() {
        let (handles, _commands, bus) = handles();
        let events = events(&bus);
        let request = Request {
            id: 15,
            cmd: None,
            op: None,
            event: None,
            timeout_ms: None,
        };
        let response = answer(&request, &handles, &events);
        assert!(!response.ok);
        assert_eq!(response.id, 15);
    }

    #[test]
    fn the_request_line_needs_an_id() {
        assert!(
            serde_json::from_str::<Request>(r#"{"op":"ping"}"#).is_err(),
            "a request id is mandatory"
        );
        let parsed: Request = serde_json::from_str(r#"{"id":4,"op":"ping"}"#).unwrap();
        assert_eq!(parsed.id, 4);
        assert_eq!(parsed.op, Some(Op::Ping));
        assert_eq!(parsed.cmd, None);
    }

    #[test]
    fn a_command_request_round_trips_through_the_line_format() {
        let request = Request::command(
            21,
            Command::AddItems {
                items: vec![rustamp_core::playlist::MediaLocation::File(
                    std::path::PathBuf::from("C:/music/a.mp3"),
                )],
                at: None,
                play: false,
            },
        );
        let text = serde_json::to_string(&request).unwrap();
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), request);
    }

    #[test]
    fn an_answer_carries_the_documented_fields() {
        let response = Response::failed(3, "no".to_owned());
        let text = serde_json::to_string(&response).unwrap();
        for field in [
            "\"id\":3",
            "\"ok\":false",
            "\"message\":\"no\"",
            "\"state\":null",
            "\"events\":[]",
        ] {
            assert!(text.contains(field), "{field} is missing from {text}");
        }
    }

    #[test]
    fn the_line_reader_splits_on_newlines_and_tolerates_crlf() {
        let mut lines = LineBuffer::default();
        let mut input = &b"{\"id\":1}\r\n{\"id\":2}\n\n{\"id\":3}"[..];
        assert_eq!(
            lines.next(&mut input).unwrap().as_deref(),
            Some("{\"id\":1}")
        );
        assert_eq!(
            lines.next(&mut input).unwrap().as_deref(),
            Some("{\"id\":2}")
        );
        assert_eq!(lines.next(&mut input).unwrap().as_deref(), Some(""));
        assert_eq!(
            lines.next(&mut input).unwrap().as_deref(),
            Some("{\"id\":3}")
        );
        assert_eq!(lines.next(&mut input).unwrap(), None);
    }

    #[test]
    fn the_line_reader_refuses_a_line_that_is_too_long() {
        let mut lines = LineBuffer::default();
        let input = vec![b'x'; MAX_LINE_BYTES + 2];
        let error = lines.next(&mut &input[..]).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn every_pipe_name_is_the_control_channel_of_its_key() {
        #[cfg(windows)]
        assert_eq!(
            pipe_name("0123456789abcdef"),
            r"\\.\pipe\rustamp-ctl-0123456789abcdef"
        );
        #[cfg(not(windows))]
        assert!(pipe_name("0123456789abcdef").ends_with("rustamp-ctl-0123456789abcdef.sock"));
        assert_ne!(pipe_name("a"), pipe_name("b"));
    }
}
