// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Spawn and supervision for `rustamp-plughost32` (RFC 0006 out-of-process hosting).
//!
//! [`crate::host::HostStateLink`] owns the player-state bridge and the
//! `Hello`/`Ping` liveness: it publishes
//! [`PlayerState`](rustamp_core::state::PlayerState) snapshots as
//! [`Message::PlayerStateUpdate`](rustamp_ipc::Message) over the **existing**
//! framed control channel, polls what the host writes back, and it
//! deliberately spawns nothing. This module is the process around that link:
//! it finds the i686 host binary, starts it, connects its control channel to
//! the link, and re-attaches (or restarts) when the host dies.
//!
//! # Path (RFC 0006 §8)
//!
//! The control channel is a framed [`Message`](rustamp_ipc::Message) stream
//! over any `Read`/`Write` pair via
//! [`StreamTransport`](rustamp_ipc::StreamTransport); the media hosts already
//! speak exactly that on stdin/stdout. The plugin host's binary does not name
//! a control pipe yet (`rustamp-plughost32`'s `main` is still an M0 stub whose
//! only TODO is "connect to the control pipe"), so the supervisor uses the
//! other documented transport: **piped stdio**. No channel is invented and no
//! protocol message is changed.
//!
//! # Rules the supervisor obeys
//!
//! * **No window may ever appear.** Every spawned child goes through
//!   [`no_window`], which sets `CREATE_NO_WINDOW` on Windows. A console window
//!   counts as a window on the host: without the flag a console-subsystem
//!   child (`rustamp-plughost32.exe`) gets a console window of its own when the
//!   parent has no console (see `crates/rustamp-app/README.md` and the
//!   spawn-site guard test below).
//! * **Never block the player.** [`Supervisor::supervise`] is called from
//!   [`App::pump`](crate::app::App::pump), i.e. off the audio path. It only
//!   ever calls the non-blocking [`Child::try_wait`], never a blocking read:
//!   the link drains the host through the non-blocking
//!   [`Transport::try_recv`](rustamp_ipc::Transport) (backed by the reader
//!   thread below for real pipes), and a failed send detaches the link as
//!   [`PublishOutcome::HostGone`](crate::host::PublishOutcome), never an
//!   error. A missed [`Message::Pong`](rustamp_ipc::Message) detaches it the
//!   same way (see [`HostStateLink::poll`](crate::host::HostStateLink::poll)).
//!   A missing, crashed or wedged host therefore degrades to "detached",
//!   never to a stall or a panic.
//! * **Convergence, not reuse.** The invariant is "link attached if and only
//!   if a live managed child exists". A dead child is reaped (dropping
//!   [`ChildHost`] kills what is left without waiting) and replaced by a
//!   fresh child plus a fresh pipe pair after [`DEFAULT_RESTART_COOLDOWN`];
//!   a live child with a detached link (a failed send, or a hand-detach) is
//!   replaced the same way rather than half-reused. Spawn attempts are
//!   cooldown-gated so a host that dies on start cannot turn into a spawn
//!   loop. [`Supervisor::stop`] is the deliberate off-switch.
//!
//! # Binary-path finding
//!
//! [`find_plugin_host`] reports honestly where the i686 host binary lives:
//!
//! 1. **Packaged layout** (exact): next to the running `rustamp.exe` (RFC 0006
//!    §8: "the helpers ship next to `rustamp.exe`"). This is the layout
//!    `cargo xtask package` must produce once it stops being a stub.
//! 2. **Dev-tree heuristic**: under the `target/` ancestor of the running
//!    binary — `target/i686-pc-windows-msvc/{debug,release}/` (where `cargo
//!    build -p rustamp-plughost --target i686-pc-windows-msvc` puts it in CI)
//!    and `target/{debug,release}/` (same-target builds) — preferring the
//!    profile the app itself runs from.
//!
//! Only (1) is exact; (2) is a development convenience. If no candidate
//! exists the supervisor stays detached. What would make the path fully
//! resolvable is the packaging step placing the helper next to `rustamp.exe`
//! (or, failing that, a build-time variable recording the host binary's path).
//!
//! # Findings (decisions the specs and RFCs do not make)
//!
//! * **F1 — no media-host precedent.** `rustamp-mediahost` explicitly defers
//!   spawning ("process spawning [is] out of scope and belong[s] to a later
//!   card"). This lane therefore establishes the first spawn/supervision
//!   pattern in the tree; media-host supervision should reuse
//!   [`HostBackend`]/[`Supervisor`] rather than invent a second one.
//! * **F2 — the handshake now exists on both ends, and the send cannot
//!   stall.** The host speaks it from `rustamp-plughost/src/control.rs`
//!   (one startup `Hello`, one same-token `Pong` per `Ping`, clean exit on
//!   close) and the app reads it here: the link greets and probes from
//!   [`HostStateLink::poll`](crate::host::HostStateLink::poll), a missed
//!   `Pong` detaches exactly like a dead child, and a foreign `Hello`
//!   detaches for replacement (finding F2 in [`crate::host`]). Publishing
//!   cannot stall the pump either: the link's `send` hands to the bounded
//!   writer-thread queue below and never touches a blocking pipe, so a
//!   live-but-not-reading host fills the queue and detaches within the
//!   liveness timeout instead of stalling a write (finding F3 in
//!   [`crate::host`]).
//! * **F3 — restart policy.** A dead host is retried forever, gated only by
//!   the cooldown. Give-up/backoff policy is undecided by the RFCs and is
//!   reported, not chosen.

use std::io;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use rustamp_ipc::{IpcError, Message, StreamTransport, Transport};

use crate::host::HostStateLink;

/// File stem of the plugin host binary (crate `rustamp-plughost`, binary
/// `rustamp-plughost32`, i686 only; the player itself is 64-bit).
pub const PLUGIN_HOST_STEM: &str = "rustamp-plughost32";

/// Minimum time between two spawn attempts (see the module docs).
///
/// Bounds crash-loop churn: a host that dies on start costs one spawn per
/// cooldown, and the player runs detached in between.
pub const DEFAULT_RESTART_COOLDOWN: Duration = Duration::from_secs(5);

/// `CreateProcess` flag: run the child with a console but **no console window**.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Binary file name for this platform (`rustamp-plughost32.exe` on Windows).
pub fn plugin_host_exe_name() -> String {
    format!("{PLUGIN_HOST_STEM}{}", std::env::consts::EXE_SUFFIX)
}

/// Where the host binary may live, best first (see the module docs).
///
/// Only reads the filesystem for the running binary's own location; existence
/// is checked by [`find_plugin_host`], not here.
pub fn plugin_host_candidates() -> Vec<PathBuf> {
    let name = plugin_host_exe_name();
    let mut out: Vec<PathBuf> = Vec::new();
    let mut push = |path: PathBuf| {
        if !out.contains(&path) {
            out.push(path);
        }
    };
    let Ok(exe) = std::env::current_exe() else {
        return out;
    };
    let Some(dir) = exe.parent() else {
        return out;
    };
    // 1. Packaged layout (RFC 0006 §8): the helper ships next to `rustamp.exe`.
    push(dir.join(&name));
    // 2. Dev tree: the profile this binary runs from first (a `debug` app most
    // likely pairs with a `debug` host), then the other profile.
    let debug_first = dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_none_or(|name| name != "release");
    let profiles = if debug_first {
        ["debug", "release"]
    } else {
        ["release", "debug"]
    };
    for ancestor in exe.ancestors() {
        if ancestor.file_name().is_some_and(|name| name == "target") {
            for profile in profiles {
                // `cargo build -p rustamp-plughost --target
                // i686-pc-windows-msvc` (the CI shape).
                push(
                    ancestor
                        .join("i686-pc-windows-msvc")
                        .join(profile)
                        .join(&name),
                );
                // Same-target build (host arch, dev only).
                push(ancestor.join(profile).join(&name));
            }
            break;
        }
    }
    out
}

/// Machine type constant for i386 / 32-bit x86 in PE COFF header (IMAGE_FILE_MACHINE_I386).
pub const IMAGE_FILE_MACHINE_I386: u16 = 0x014c;

/// Validate that a seekable byte stream is a valid 32-bit (i686) PE image (Finding 11).
pub fn is_i686_reader<R: std::io::Read + std::io::Seek>(mut reader: R) -> bool {
    use std::io::SeekFrom;
    let mut dos = [0u8; 64];
    if reader.read_exact(&mut dos).is_err() {
        return false;
    }
    if dos[0] != b'M' || dos[1] != b'Z' {
        return false;
    }
    let pe_offset = u32::from_le_bytes([dos[0x3C], dos[0x3D], dos[0x3E], dos[0x3F]]);
    if reader.seek(SeekFrom::Start(pe_offset as u64)).is_err() {
        return false;
    }
    let mut pe_sig_and_header = [0u8; 6];
    if reader.read_exact(&mut pe_sig_and_header).is_err() {
        return false;
    }
    if &pe_sig_and_header[0..4] != b"PE\0\0" {
        return false;
    }
    let machine = u16::from_le_bytes([pe_sig_and_header[4], pe_sig_and_header[5]]);
    machine == IMAGE_FILE_MACHINE_I386
}

/// Validate that a binary file exists and is a valid 32-bit (i686) PE executable (Finding 11).
pub fn is_i686_executable(path: &std::path::Path) -> bool {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    is_i686_reader(file)
}

/// First existing candidate from [`plugin_host_candidates`], if any.
///
/// Enforces i686 architecture: an x64 binary with the same name is rejected (Finding 11).
/// `None` means "no compatible host binary is resolvable at runtime": the supervisor
/// stays detached (see the module docs for what would fix that).
pub fn find_plugin_host() -> Option<PathBuf> {
    plugin_host_candidates()
        .into_iter()
        .find(|path| path.is_file() && is_i686_executable(path))
}

/// Makes `command` incapable of raising a window.
///
/// On Windows this sets `CREATE_NO_WINDOW`: a console window counts as a
/// window on the host, and without the flag a console-subsystem child
/// (`rustamp-plughost32.exe`) gets a console window of its own when the parent
/// has no console. Elsewhere there is nothing to suppress. Every spawn site in
/// this module must go through this (enforced by
/// `every_spawn_site_uses_no_window` below).
fn no_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

/// A supervised host process: the only liveness query the supervisor needs.
///
/// Kept minimal on purpose so tests can substitute a scripted fake without
/// spawning anything, and so the supervisor never touches process handles
/// beyond a non-blocking poll.
pub trait HostProcess {
    /// Non-blocking: `true` while the host is still running. Must never block
    /// and never panic; an indeterminate state counts as dead.
    fn is_running(&mut self) -> bool;
}

/// How a [`Supervisor`] starts hosts: one method, so the production spawn
/// (a real child with piped stdio) and the test fake share the supervision
/// logic byte for byte.
pub trait HostBackend {
    /// The app's end of the control channel, moved into the [`HostStateLink`].
    type Link: Transport + 'static;
    /// The running host, kept by the supervisor for liveness polls.
    type Process: HostProcess;
    /// Start one host and return its control-channel transport plus its
    /// process handle. Any failure (missing binary, failed spawn, missing
    /// stdio) is an `Err` the supervisor absorbs into "detached".
    fn spawn(&mut self) -> io::Result<(Self::Link, Self::Process)>;
}

/// A real host child. Dropping kills what is left **without waiting**: `kill`
/// is best-effort and non-blocking, so reaping a dead host can never stall the
/// app thread that drives the supervisor.
pub struct ChildHost {
    child: Child,
}

impl HostProcess for ChildHost {
    fn is_running(&mut self) -> bool {
        // `try_wait` never blocks; anything but "still running" (exited, or an
        // error that should not happen) counts as dead rather than panicking.
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for ChildHost {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Read one host's stdout to the end, forwarding every frame to the app thread.
///
/// This is the reader-thread half of the app side (see
/// [`HostStateLink::poll`](crate::host::HostStateLink::poll) for the other
/// half): blocking pipe reads cannot run on the pump, so they run here — off
/// the audio path — while the app thread keeps the write half and polls this
/// thread's handover through the same non-blocking `try_recv`. The thread
/// exits when the pipe closes, when a frame corrupts, or when the link drops
/// the handover; any of those detaches the link, so the supervisor replaces
/// the host. Even spawning the thread cannot fail the supervisor: if it
/// cannot be spawned the handover is already disconnected, which reads as a
/// dead host.
/// How many inbound frames the reader thread queues for the app thread (Finding 9).
const HOST_RECV_QUEUE: usize = 16;

fn spawn_host_reader(transport: impl Transport + 'static) -> mpsc::Receiver<Message> {
    let (events, handover) = mpsc::sync_channel(HOST_RECV_QUEUE);
    let _ = std::thread::Builder::new()
        .name("rustamp-host-reader".to_owned())
        .spawn(move || {
            let mut transport = transport;
            loop {
                match transport.recv() {
                    Ok(message) => {
                        if events.send(message).is_err() {
                            return;
                        }
                    }
                    Err(_) => return,
                }
            }
        });
    handover
}

/// How many outbound frames the writer thread queues for a slow host (see
/// [`spawn_host_writer`]).
///
/// A pump step queues at most two small frames (one `Hello`/`Ping` from the
/// link's poll, one `PlayerStateUpdate` from its publish), and a live host
/// drains them at pipe speed — so a full queue means the host stopped
/// reading, and the next `send` detaches rather than stalls. Sixteen frames
/// bound the handover to a few kilobytes while absorbing any same-step
/// burst.
const HOST_SEND_QUEUE: usize = 16;

/// Run one host's stdin to the end, sending whatever the app thread queues.
///
/// This is the writer-thread half of the app side, mirroring
/// [`spawn_host_reader`]: blocking pipe writes cannot run on the pump, so
/// they run here — off the audio path — while the app thread keeps only the
/// [`mpsc::SyncSender`] handover and queues through a non-blocking
/// `try_send` (see `ProcessLink`). The thread exits when the link drops the
/// handover or when a write fails (the pipe broke); either way the link has
/// already detached or detaches on the next failed `send`, so the supervisor
/// replaces the host. A blocked writer (a host that stopped reading) stays
/// parked until its child is reaped — killing the child closes the pipe and
/// unblocks the write — exactly like a blocked reader. Even spawning the
/// thread cannot fail the supervisor: if it cannot be spawned the handover
/// is already disconnected, which reads as a dead host.
fn spawn_host_writer(writer: impl Transport + 'static) -> mpsc::SyncSender<Message> {
    let (outbox, queue) = mpsc::sync_channel(HOST_SEND_QUEUE);
    let _ = std::thread::Builder::new()
        .name("rustamp-host-writer".to_owned())
        .spawn(move || {
            let mut writer = writer;
            for message in queue {
                if writer.send(&message).is_err() {
                    return;
                }
            }
        });
    outbox
}

/// The app's end of a real host child: a bounded handover to the writer
/// thread for stdin plus the reader thread's handover for the stdout read
/// half.
///
/// Splitting both directions is what keeps the pump non-blocking: the link
/// queues (`PlayerStateUpdate`, `Hello`, `Ping`) through a `try_send` that
/// never touches the pipe, while [`spawn_host_writer`] blocks in `send` on
/// its own thread, and inbound frames (`Hello`, `Pong`) arrive through
/// [`spawn_host_reader`] the same way (see
/// [`HostStateLink::poll`](crate::host::HostStateLink::poll)). A full outbox
/// means the host stopped reading: `send` reports [`IpcError::WouldBlock`] —
/// which the link turns into a detach like any failed send — instead of
/// stalling the player. `recv` exists only for [`Transport`] completeness —
/// the app side never calls it; it would block the caller on the handover.
pub struct ProcessLink {
    outbox: mpsc::SyncSender<Message>,
    events: mpsc::Receiver<Message>,
}

impl Transport for ProcessLink {
    fn send(&mut self, msg: &Message) -> Result<(), IpcError> {
        // Never blocks: `try_send` queues at most one small frame, and a
        // full queue is a stalled host, not a reason to stall the player.
        match self.outbox.try_send(msg.clone()) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => Err(IpcError::WouldBlock),
            Err(mpsc::TrySendError::Disconnected(_)) => Err(IpcError::Closed),
        }
    }
    fn recv(&mut self) -> Result<Message, IpcError> {
        self.events.recv().map_err(|_| IpcError::Closed)
    }
    fn try_recv(&mut self) -> Result<Option<Message>, IpcError> {
        match self.events.try_recv() {
            Ok(message) => Ok(Some(message)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err(IpcError::Closed),
        }
    }
}

/// Starts the real `rustamp-plughost32` binary with piped stdio.
///
/// The pipes carry the framed [`Message`](rustamp_ipc::Message) control
/// channel (the RFC 0006 §8 stdio shape): the app writes stdin, the host
/// reads it, and the host's stdout comes back through [`spawn_host_reader`]
/// into the [`ProcessLink`] handover that
/// [`HostStateLink::poll`](crate::host::HostStateLink::poll) drains.
/// `stderr` is inherited so host diagnostics stay visible; nothing is ever
/// read on the app thread (see the module docs).
pub struct ProcessBackend {
    exe: Option<PathBuf>,
}

impl ProcessBackend {
    /// Resolve the binary with [`find_plugin_host`]. Never fails: an
    /// unresolvable path surfaces as a spawn `Err` (i.e. detached), not a
    /// panic.
    pub fn from_search() -> Self {
        ProcessBackend {
            exe: find_plugin_host(),
        }
    }

    /// Pin an explicit binary path (packaging layouts, tests).
    pub fn with_path(path: PathBuf) -> Self {
        ProcessBackend { exe: Some(path) }
    }
}

impl HostBackend for ProcessBackend {
    type Link = ProcessLink;
    type Process = ChildHost;

    fn spawn(&mut self) -> io::Result<(Self::Link, Self::Process)> {
        let exe = self.exe.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "rustamp-plughost32 not found: no packaged helper next to \
                 rustamp.exe and no dev-tree build; the host stays detached",
            )
        })?;
        if !is_i686_executable(&exe) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "rustamp-plughost32 binary at {} is not an i686 (32-bit) executable",
                    exe.display()
                ),
            ));
        }
        let mut command = Command::new(&exe);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        no_window(&mut command);
        let mut child = command.spawn()?;
        match (child.stdin.take(), child.stdout.take()) {
            (Some(stdin), Some(stdout)) => {
                // Both halves move to background threads; the link keeps
                // only non-blocking handovers (see `ProcessLink`).
                let reader = StreamTransport::new(stdout, io::sink());
                let events = spawn_host_reader(reader);
                let writer = StreamTransport::new(io::empty(), stdin);
                let outbox = spawn_host_writer(writer);
                Ok((ProcessLink { outbox, events }, ChildHost { child }))
            }
            _ => {
                let _ = child.kill();
                Err(io::Error::other(
                    "rustamp-plughost32 started without piped stdio; \
                     the host stays detached",
                ))
            }
        }
    }
}

/// State of the supervisor (Finding 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorState {
    /// Actively supervising, auto-spawns on startup and after cooldown when detached.
    Running,
    /// Deliberately stopped via [`Supervisor::stop`]. Will NOT restart until [`Supervisor::resume`] is called.
    Stopped,
    /// Persistent failure state: consecutive spawn or handshake failures exceeded the policy limit.
    PersistentFailure,
}

/// Default maximum consecutive spawn or handshake failures before entering [`SupervisorState::PersistentFailure`].
pub const DEFAULT_MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// Default timeout for waiting for the child's startup [`Message::Hello`].
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);

struct PendingHost<L, P> {
    transport: L,
    process: P,
    deadline: Instant,
}

/// Spawns and supervises one plugin host for one [`HostStateLink`].
///
/// Drive it from the app thread (e.g. [`App::pump`](crate::app::App::pump)):
/// [`supervise`](Supervisor::supervise) is non-blocking, never panics, and
/// converges the link to "attached to a live child, else detached". The
/// generic backend exists so unit tests run the same logic against a fake;
/// production uses [`ProcessBackend`] (the default).
pub struct Supervisor<B: HostBackend = ProcessBackend> {
    backend: B,
    process: Option<B::Process>,
    pending: Option<PendingHost<B::Link, B::Process>>,
    state: SupervisorState,
    last_attempt: Option<Instant>,
    cooldown: Duration,
    handshake_timeout: Duration,
    spawn_attempts: u64,
    consecutive_failures: u32,
    max_consecutive_failures: u32,
}

impl<B: HostBackend> Supervisor<B> {
    /// A supervisor that has started nothing yet (the link stays detached
    /// until the first [`supervise`](Supervisor::supervise)).
    pub fn new(backend: B) -> Self {
        Supervisor {
            backend,
            process: None,
            pending: None,
            state: SupervisorState::Running,
            last_attempt: None,
            cooldown: DEFAULT_RESTART_COOLDOWN,
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
            spawn_attempts: 0,
            consecutive_failures: 0,
            max_consecutive_failures: DEFAULT_MAX_CONSECUTIVE_FAILURES,
        }
    }

    /// Like [`new`](Supervisor::new) with an explicit restart cooldown
    /// (tests use [`Duration::ZERO`] to skip waiting).
    pub fn with_cooldown(backend: B, cooldown: Duration) -> Self {
        Supervisor {
            backend,
            process: None,
            pending: None,
            state: SupervisorState::Running,
            last_attempt: None,
            cooldown,
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
            spawn_attempts: 0,
            consecutive_failures: 0,
            max_consecutive_failures: DEFAULT_MAX_CONSECUTIVE_FAILURES,
        }
    }

    /// Explicit failure policy and handshake configuration.
    pub fn with_failure_policy(
        backend: B,
        cooldown: Duration,
        handshake_timeout: Duration,
        max_consecutive_failures: u32,
    ) -> Self {
        Supervisor {
            backend,
            process: None,
            pending: None,
            state: SupervisorState::Running,
            last_attempt: None,
            cooldown,
            handshake_timeout,
            spawn_attempts: 0,
            consecutive_failures: 0,
            max_consecutive_failures,
        }
    }

    /// How many spawn attempts have been made (diagnostics and tests).
    pub fn spawn_attempts(&self) -> u64 {
        self.spawn_attempts
    }

    /// The current state of the supervisor (Finding 12).
    pub fn state(&self) -> SupervisorState {
        self.state
    }

    /// Whether the supervisor is deliberately stopped (Finding 12).
    pub fn is_stopped(&self) -> bool {
        self.state == SupervisorState::Stopped
    }

    /// Whether the supervisor has halted due to persistent failures (Finding 12).
    pub fn is_failed(&self) -> bool {
        self.state == SupervisorState::PersistentFailure
    }

    /// How many consecutive failures have occurred.
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Reset any stopped or failed state and permit supervising/restarting again (Finding 12).
    pub fn resume(&mut self) {
        self.state = SupervisorState::Running;
        self.consecutive_failures = 0;
        self.pending = None;
    }

    /// Whether a live managed child exists right now (non-blocking).
    pub fn is_live(&mut self) -> bool {
        self.process.as_mut().is_some_and(|p| p.is_running())
    }

    fn record_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        if self.consecutive_failures >= self.max_consecutive_failures {
            self.state = SupervisorState::PersistentFailure;
        }
    }

    /// One non-blocking supervision step: reap a dead child, replace a
    /// desynced one, enforce the startup Hello barrier (Finding 8), and
    /// (re-)attach a fresh transport when due.
    ///
    /// Never blocks, never panics, never propagates an error: a host that
    /// cannot be started leaves the link detached and the player unaffected.
    pub fn supervise(&mut self, link: &mut HostStateLink) {
        // Finding 12: stopped state sticks, and persistent failures halt attempts.
        if self.state == SupervisorState::Stopped
            || self.state == SupervisorState::PersistentFailure
        {
            return;
        }

        let live = self.process.as_mut().is_some_and(|p| p.is_running());
        if live && link.is_attached() {
            return; // healthy and connected: nothing to do
        }

        // A dead child, or a desynced one (a live child whose link was detached:
        // a failed send detached it, or it was detached by hand), is reaped here
        // and counted as a failure. The spawn path below then decides in the
        // *same* pump whether a replacement is due, so a host that dies is
        // replaced on one supervision step rather than on the next one.
        if link.is_attached() {
            self.process = None;
            link.detach();
            self.record_failure();
        } else if self.process.is_some() {
            self.process = None;
            self.record_failure();
        }

        // The reap above may have tripped the failure policy: stop rather than
        // spawn into a state that is meant to hold.
        if self.state == SupervisorState::PersistentFailure {
            return;
        }

        // Handshake barrier (Finding 8): if we have a spawned child waiting for greeting.
        if let Some(mut pending) = self.pending.take() {
            if !pending.process.is_running() {
                self.record_failure();
                return;
            }
            if Instant::now() >= pending.deadline {
                self.record_failure();
                return;
            }
            match pending.transport.try_recv() {
                Ok(Some(Message::Hello {
                    protocol_version,
                    pid,
                })) => {
                    if protocol_version == rustamp_ipc::PROTOCOL_VERSION {
                        // Compatible Hello arrived! Barrier passed, attach and mark connected.
                        self.consecutive_failures = 0;
                        link.attach(pending.transport);
                        link.record_greeting(protocol_version, pid);
                        self.process = Some(pending.process);
                        return;
                    } else {
                        // Incompatible protocol version: reject and reap.
                        self.record_failure();
                        return;
                    }
                }
                Ok(Some(_)) => {
                    // Protocol violation: received message before startup Hello.
                    self.record_failure();
                    return;
                }
                Ok(None) => {
                    // Still waiting for Hello: keep link detached until greeting arrives.
                    self.pending = Some(pending);
                    return;
                }
                Err(_) => {
                    self.record_failure();
                    return;
                }
            }
        }

        // Cooling down: stay detached rather than spin on spawns.
        if self
            .last_attempt
            .is_some_and(|at| at.elapsed() < self.cooldown)
        {
            return;
        }

        self.last_attempt = Some(Instant::now());
        self.spawn_attempts += 1;
        match self.backend.spawn() {
            Ok((mut transport, mut process)) => {
                if !process.is_running() {
                    self.record_failure();
                    return;
                }
                // Check if Hello is already buffered on the transport (e.g. in-memory tests or fast child):
                match transport.try_recv() {
                    Ok(Some(Message::Hello {
                        protocol_version,
                        pid,
                    })) => {
                        if protocol_version == rustamp_ipc::PROTOCOL_VERSION {
                            self.consecutive_failures = 0;
                            link.attach(transport);
                            link.record_greeting(protocol_version, pid);
                            self.process = Some(process);
                        } else {
                            self.record_failure();
                        }
                    }
                    Ok(Some(_)) => {
                        self.record_failure();
                    }
                    Ok(None) => {
                        // Await greeting within handshake deadline. Link remains detached until Hello arrives!
                        let deadline = Instant::now() + self.handshake_timeout;
                        self.pending = Some(PendingHost {
                            transport,
                            process,
                            deadline,
                        });
                    }
                    Err(_) => {
                        self.record_failure();
                    }
                }
            }
            Err(_) => {
                self.record_failure();
            }
        }
    }

    /// The deliberate off-switch (Finding 12): kill the managed child (best-effort,
    /// without waiting) and detach the link without sending anything. Sets the
    /// supervisor into [`SupervisorState::Stopped`] so subsequent `supervise` calls
    /// will NOT restart it until [`Supervisor::resume`] is called.
    pub fn stop(&mut self, link: &mut HostStateLink) {
        self.state = SupervisorState::Stopped;
        self.process = None;
        self.pending = None;
        link.detach();
    }
}

impl Supervisor<ProcessBackend> {
    /// A supervisor that resolves the binary with [`find_plugin_host`].
    pub fn from_search() -> Self {
        Supervisor::new(ProcessBackend::from_search())
    }

    /// A supervisor pinned to an explicit binary path.
    pub fn with_path(path: PathBuf) -> Self {
        Supervisor::new(ProcessBackend::with_path(path))
    }
}

// ---- spawn-site guard: the source check below ignores everything after this
// line, so its own needles cannot match themselves. ----

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    use rustamp_core::state::PlayerState;
    use rustamp_ipc::{MemoryTransport, Message};

    use super::*;

    /// Scripted backend: no process is ever spawned.
    struct FakeBackend {
        /// Queued outcomes: `Ok(alive)` starts a host with that liveness,
        /// `Err` fails the spawn; an exhausted script fails closed (detached).
        script: VecDeque<io::Result<bool>>,
        /// Peer ends of the transports handed out, for assertions.
        peers: Vec<MemoryTransport>,
        /// Liveness flags of the spawned processes: the test flips these to
        /// simulate host death.
        flags: Vec<Rc<Cell<bool>>>,
        /// Whether spawn should automatically send `Message::hello()`.
        auto_hello: bool,
    }

    struct FakeProcess {
        alive: Rc<Cell<bool>>,
    }

    impl HostProcess for FakeProcess {
        fn is_running(&mut self) -> bool {
            self.alive.get()
        }
    }

    impl HostBackend for FakeBackend {
        type Link = MemoryTransport;
        type Process = FakeProcess;

        fn spawn(&mut self) -> io::Result<(MemoryTransport, FakeProcess)> {
            match self.script.pop_front() {
                Some(Ok(alive)) => {
                    let (app_end, mut peer) = MemoryTransport::pair();
                    if alive && self.auto_hello {
                        let _ = peer.send(&Message::hello());
                    }
                    self.peers.push(peer);
                    let alive = Rc::new(Cell::new(alive));
                    self.flags.push(Rc::clone(&alive));
                    Ok((app_end, FakeProcess { alive }))
                }
                Some(Err(error)) => Err(error),
                None => Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "fake script exhausted",
                )),
            }
        }
    }

    fn backend(first: bool) -> FakeBackend {
        FakeBackend {
            script: VecDeque::from([Ok(first)]),
            peers: Vec::new(),
            flags: Vec::new(),
            auto_hello: true,
        }
    }
    fn supervised(first: bool) -> (Supervisor<FakeBackend>, HostStateLink) {
        (
            Supervisor::with_cooldown(backend(first), Duration::ZERO),
            HostStateLink::detached(),
        )
    }

    #[test]
    fn first_supervise_attaches_and_publishes_over_the_same_channel() {
        let (mut supervisor, mut link) = supervised(true);
        assert!(!link.is_attached());
        supervisor.supervise(&mut link);
        assert!(link.is_attached());
        assert!(supervisor.is_live());
        assert_eq!(supervisor.spawn_attempts(), 1);

        let state = PlayerState::default();
        assert_eq!(link.publish(&state), crate::host::PublishOutcome::Sent);
        let peer = &mut supervisor.backend.peers[0];
        assert_eq!(
            peer.recv().expect("one framed message"),
            Message::PlayerStateUpdate(Box::new(state))
        );
    }

    #[test]
    fn a_dead_host_is_reaped_and_replaced_with_a_fresh_transport() {
        let (mut supervisor, mut link) = supervised(true);
        supervisor.supervise(&mut link);
        assert!(link.is_attached());

        // The host dies. The next step reaps it and starts a fresh pair.
        supervisor.backend.flags[0].set(false);
        supervisor.backend.script.push_back(Ok(true));
        supervisor.supervise(&mut link);
        assert_eq!(supervisor.spawn_attempts(), 2);
        assert!(link.is_attached());
        assert!(supervisor.is_live());

        // Publication flows over the *new* pipe pair, not the dead one.
        let state = PlayerState::default();
        assert_eq!(link.publish(&state), crate::host::PublishOutcome::Sent);
        let peer = supervisor.backend.peers.last_mut().expect("fresh peer");
        assert_eq!(
            peer.recv().expect("one framed message"),
            Message::PlayerStateUpdate(Box::new(state))
        );
    }

    #[test]
    fn a_failed_spawn_stays_detached_and_the_cooldown_bounds_retries() {
        let mut supervisor = Supervisor::with_cooldown(
            FakeBackend {
                script: VecDeque::from([Err(io::Error::new(io::ErrorKind::NotFound, "no binary"))]),
                peers: Vec::new(),
                flags: Vec::new(),
                auto_hello: true,
            },
            Duration::from_secs(3600),
        );
        let mut link = HostStateLink::detached();
        supervisor.supervise(&mut link);
        assert!(!link.is_attached());
        assert_eq!(supervisor.spawn_attempts(), 1);
        // Still cooling down: no second attempt, still detached, no panic.
        supervisor.supervise(&mut link);
        assert_eq!(supervisor.spawn_attempts(), 1);
        assert!(!link.is_attached());
    }

    #[test]
    fn a_live_but_detached_child_is_replaced_not_reused() {
        let (mut supervisor, mut link) = supervised(true);
        supervisor.supervise(&mut link);
        supervisor.backend.script.push_back(Ok(true));
        // Desync the pair by hand (what a `HostGone` send does on its own).
        link.detach();
        supervisor.supervise(&mut link);
        assert_eq!(supervisor.spawn_attempts(), 2);
        assert!(link.is_attached());
    }

    #[test]
    fn stop_kills_the_child_and_detaches_without_a_send() {
        let (mut supervisor, mut link) = supervised(true);
        supervisor.supervise(&mut link);
        assert!(supervisor.is_live());
        supervisor.stop(&mut link);
        assert!(!link.is_attached());
        assert!(!supervisor.is_live());
        assert_eq!(
            link.publish(&PlayerState::default()),
            crate::host::PublishOutcome::Detached
        );
    }

    #[test]
    fn stop_sticks_and_subsequent_supervise_does_not_restart_until_resumed() {
        let (mut supervisor, mut link) = supervised(true);
        supervisor.supervise(&mut link);
        assert!(supervisor.is_live());
        assert!(link.is_attached());

        supervisor.stop(&mut link);
        assert!(!link.is_attached());
        assert!(!supervisor.is_live());
        assert!(supervisor.is_stopped());

        // Subsequent supervise calls do NOT restart while stopped (Finding 12)
        supervisor.backend.script.push_back(Ok(true));
        supervisor.supervise(&mut link);
        assert!(!link.is_attached());
        assert!(!supervisor.is_live());
        assert_eq!(supervisor.spawn_attempts(), 1);

        // Resume re-enables supervision
        supervisor.resume();
        assert_eq!(supervisor.state(), SupervisorState::Running);
        supervisor.supervise(&mut link);
        assert!(link.is_attached());
        assert!(supervisor.is_live());
        assert_eq!(supervisor.spawn_attempts(), 2);
    }

    #[test]
    fn persistent_failure_halts_retries_after_max_failures() {
        let mut supervisor = Supervisor::with_cooldown(
            FakeBackend {
                script: VecDeque::from([
                    Err(io::Error::new(io::ErrorKind::NotFound, "err1")),
                    Err(io::Error::new(io::ErrorKind::NotFound, "err2")),
                    Err(io::Error::new(io::ErrorKind::NotFound, "err3")),
                    Err(io::Error::new(io::ErrorKind::NotFound, "err4")),
                ]),
                peers: Vec::new(),
                flags: Vec::new(),
                auto_hello: true,
            },
            Duration::ZERO,
        );
        let mut link = HostStateLink::detached();
        // Failure 1
        supervisor.supervise(&mut link);
        assert_eq!(supervisor.consecutive_failures(), 1);
        assert_eq!(supervisor.state(), SupervisorState::Running);

        // Failure 2
        supervisor.supervise(&mut link);
        assert_eq!(supervisor.consecutive_failures(), 2);
        assert_eq!(supervisor.state(), SupervisorState::Running);

        // Failure 3: trips DEFAULT_MAX_CONSECUTIVE_FAILURES (3) -> PersistentFailure (Finding 12)
        supervisor.supervise(&mut link);
        assert_eq!(supervisor.consecutive_failures(), 3);
        assert!(supervisor.is_failed());

        // Subsequent supervise calls do NOT spawn
        supervisor.supervise(&mut link);
        assert_eq!(supervisor.spawn_attempts(), 3);
        assert!(supervisor.is_failed());

        // Resume clears failure count and restores Running state
        supervisor.resume();
        assert_eq!(supervisor.state(), SupervisorState::Running);
        assert_eq!(supervisor.consecutive_failures(), 0);
    }

    #[test]
    fn hello_barrier_prevents_attach_until_compatible_greeting() {
        let mut backend = backend(true);
        backend.auto_hello = false;
        let mut supervisor = Supervisor::with_cooldown(backend, Duration::ZERO);
        let mut link = HostStateLink::detached();

        // 1. First supervise spawns child, but greeting is not sent yet (Finding 8)
        supervisor.supervise(&mut link);
        assert_eq!(supervisor.spawn_attempts(), 1);
        assert!(!link.is_attached(), "must not attach before Hello arrives");
        assert_eq!(
            link.publish(&PlayerState::default()),
            crate::host::PublishOutcome::Detached
        );

        // 2. Child writes compatible Hello.
        let peer = &mut supervisor.backend.peers[0];
        peer.send(&Message::hello()).expect("send greeting");

        // 3. Next supervise pump processes Hello, passes the barrier and attaches.
        supervisor.supervise(&mut link);
        assert!(link.is_attached(), "must attach after compatible Hello");
        assert!(
            link.greeting().is_some(),
            "must record greeting upon passing Hello barrier"
        );
        assert_eq!(
            link.publish(&PlayerState::default()),
            crate::host::PublishOutcome::Sent
        );
    }

    #[test]
    fn hello_barrier_rejects_incompatible_protocol_version() {
        let mut backend = backend(true);
        backend.auto_hello = false;
        let mut supervisor = Supervisor::with_cooldown(backend, Duration::ZERO);
        let mut link = HostStateLink::detached();

        supervisor.supervise(&mut link);
        assert!(!link.is_attached());

        // Child writes greeting with incompatible protocol version (Finding 8).
        let peer = &mut supervisor.backend.peers[0];
        peer.send(&Message::Hello {
            protocol_version: rustamp_ipc::PROTOCOL_VERSION + 1,
            pid: 4321,
        })
        .expect("send incompatible greeting");

        supervisor.supervise(&mut link);
        assert!(!link.is_attached(), "incompatible Hello must be rejected");
        assert_eq!(supervisor.consecutive_failures(), 1);
    }

    #[test]
    fn pe_architecture_validation_accepts_i686_and_rejects_other() {
        // Construct valid PE header for i386 (machine 0x014c) (Finding 11)
        let mut buf = vec![0u8; 128];
        buf[0] = b'M';
        buf[1] = b'Z';
        buf[0x3C] = 0x40; // PE offset at 64
        buf[0x40..0x44].copy_from_slice(b"PE\0\0");
        buf[0x44..0x46].copy_from_slice(&0x014cu16.to_le_bytes()); // IMAGE_FILE_MACHINE_I386
        assert!(is_i686_reader(std::io::Cursor::new(&buf)));

        // Change machine type to x86_64 (IMAGE_FILE_MACHINE_AMD64 = 0x8664)
        buf[0x44..0x46].copy_from_slice(&0x8664u16.to_le_bytes());
        assert!(!is_i686_reader(std::io::Cursor::new(&buf)));

        // Invalid PE signature
        buf[0x40..0x44].copy_from_slice(b"NE\0\0");
        assert!(!is_i686_reader(std::io::Cursor::new(&buf)));

        // Non-MZ
        buf[0] = b'E';
        buf[1] = b'L';
        assert!(!is_i686_reader(std::io::Cursor::new(&buf)));
    }

    /// A writer-side transport whose `send` parks until released: stands in
    /// for a pipe whose reader stopped (a full kernel buffer that never
    /// drains).
    struct GatedWriter {
        release: mpsc::Receiver<()>,
    }

    impl Transport for GatedWriter {
        fn send(&mut self, _msg: &Message) -> Result<(), IpcError> {
            self.release.recv().map_err(|_| IpcError::Closed)?;
            Ok(())
        }
        fn recv(&mut self) -> Result<Message, IpcError> {
            Err(IpcError::Closed)
        }
    }

    /// The writer thread owns the blocking handle, so the link's `send`
    /// stays non-blocking even against a host that stopped reading.
    #[test]
    fn a_stalled_host_fills_the_outbox_without_stalling_the_send() {
        let (release, gate) = mpsc::channel();
        let outbox = spawn_host_writer(GatedWriter { release: gate });
        // The inbound half is not under test; keep its sender alive so the
        // handover reads as empty rather than closed.
        let (_stray, events) = mpsc::channel();
        let mut link = ProcessLink { outbox, events };

        // The writer takes the first frame and parks in it forever, so the
        // queue holds everything after: the first `HOST_SEND_QUEUE` sends
        // always fit (at most one was taken), and every send returns — a
        // blocking `send` would hang this test instead.
        for _ in 0..HOST_SEND_QUEUE {
            link.send(&Message::Ping(1)).expect("the queue has room");
        }
        // One filler covers the race where the writer already took the first
        // frame: after this the queue is full either way (the writer never
        // drains), so every further send reports it instead of blocking.
        drop(link.send(&Message::Ping(1)));
        // Past the queue's capacity the handover is full and stays full
        // (the writer never drains), so the send reports it instead of
        // blocking — and the link turns that into a detach like any failed
        // send.
        for _ in 0..HOST_SEND_QUEUE {
            assert!(matches!(
                link.send(&Message::Ping(1)),
                Err(IpcError::WouldBlock)
            ));
        }
        // Release the parked writer so its thread exits cleanly.
        drop(release);
        drop(link);
    }

    #[test]
    fn the_writer_thread_forwards_in_order() {
        let (writer_end, mut peer) = MemoryTransport::pair();
        let outbox = spawn_host_writer(writer_end);
        let (_stray, events) = mpsc::channel();
        let mut link = ProcessLink { outbox, events };
        link.send(&Message::Ping(1)).unwrap();
        link.send(&Message::Ping(2)).unwrap();
        link.send(&Message::hello()).unwrap();
        // The peer blocks in `recv` until the writer delivers; order is the
        // queue's FIFO, so the host sees `Hello` before `Ping` before state,
        // exactly as with a direct write.
        assert_eq!(peer.recv().unwrap(), Message::Ping(1));
        assert_eq!(peer.recv().unwrap(), Message::Ping(2));
        assert!(matches!(peer.recv().unwrap(), Message::Hello { .. }));
    }

    #[test]
    fn the_reader_thread_forwards_until_the_channel_closes() {
        let (mut app_end, host_end) = MemoryTransport::pair();
        let handover = spawn_host_reader(host_end);
        // Whatever the host writes arrives on the handover, in order...
        app_end.send(&Message::hello()).unwrap();
        app_end.send(&Message::Pong(11)).unwrap();
        assert!(matches!(
            handover.recv().expect("the greeting"),
            Message::Hello { .. }
        ));
        assert_eq!(handover.recv().expect("the pong"), Message::Pong(11));
        // ...and when the host end goes away the thread exits instead of
        // hanging, so the handover disconnects and the link detaches.
        drop(app_end);
        assert!(handover.recv().is_err());
    }

    #[test]
    fn candidates_name_the_i686_host_binary() {
        let name = plugin_host_exe_name();
        assert!(name.starts_with(PLUGIN_HOST_STEM), "{name}");
        #[cfg(windows)]
        assert!(name.ends_with(".exe"), "{name}");
        // The packaged candidate is exact: next to the running binary.
        let candidates = plugin_host_candidates();
        let exe_dir = std::env::current_exe()
            .expect("tests run from a binary")
            .parent()
            .expect("a binary has a directory")
            .to_path_buf();
        assert_eq!(candidates.first(), Some(&exe_dir.join(&name)));
        assert!(candidates.iter().all(|p| p.ends_with(&name)));
        // Merely resolves (or not) without touching anything: must not panic
        // when the dev tree has no i686 build.
        let _ = find_plugin_host();
    }

    /// The spawn helper really applies `CREATE_NO_WINDOW`, **without spawning
    /// anything** (it only builds a `Command` and never calls `spawn`).
    #[test]
    fn the_spawn_helper_applies_create_no_window() {
        let mut command = Command::new("never-spawned-probe");
        no_window(&mut command);
        #[cfg(windows)]
        {
            // `std` exposes no getter for creation flags, so this asserts the
            // constant the helper must pass; `every_spawn_site_uses_no_window`
            // covers the call itself.
            assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
        }
    }

    /// Guards the spawn sites: exactly one place constructs a child, and it
    /// goes through [`no_window`], so no spawn can raise a console window on
    /// the host.
    ///
    /// This inspects its own source because `std::process::Command` has no
    /// `creation_flags` getter. If this test fails, a spawned child can raise
    /// a window on the host: restore `CREATE_NO_WINDOW` / route the spawn
    /// through the guarded site.
    #[test]
    fn every_spawn_site_uses_no_window() {
        let source = include_str!("supervise.rs");
        // The production half only; the needles below live after the marker.
        let production = source
            .split("// ---- spawn-site guard")
            .next()
            .unwrap_or(source);

        assert_eq!(
            production.matches("Command::new(").count(),
            1,
            "there must be exactly one spawn site (`ProcessBackend::spawn`); \
             route new spawns through it so they get `no_window`"
        );
        assert!(
            production.contains("creation_flags(CREATE_NO_WINDOW)"),
            "`no_window` must set `CREATE_NO_WINDOW`"
        );
        assert!(
            production.contains("no_window(&mut command)"),
            "the spawn site must apply the no-window flag"
        );
    }

    /// Spawns the real host binary. **VM-only and `#[ignore]`d by default:**
    /// it starts a real process (with `CREATE_NO_WINDOW`, so no console window
    /// appears) and must never run as part of the ordinary suite.
    ///
    /// Today the host is an M0 stub that prints its version and exits, so the
    /// test asserts the spawn path only (start, attach, stop) — not that the
    /// child stays alive. If no binary is resolvable (no packaged helper and
    /// no dev-tree build) the test reports that and returns instead of
    /// failing: the finding, not the supervision, is what is missing.
    #[test]
    #[ignore = "spawns a real process; VM-only, never part of the default suite"]
    fn spawns_the_real_host_without_a_window() {
        crate::window::require_test_vm();
        let Some(exe) = find_plugin_host() else {
            eprintln!(
                "SKIP: no rustamp-plughost32 binary resolvable (see \
                 `plugin_host_candidates`); build one with `cargo build -p \
                 rustamp-plughost --target i686-pc-windows-msvc`"
            );
            return;
        };
        let mut supervisor = Supervisor::with_path(exe);
        let mut link = HostStateLink::detached();
        let start = std::time::Instant::now();
        while !link.is_attached() && start.elapsed() < std::time::Duration::from_secs(5) {
            supervisor.supervise(&mut link);
            if !link.is_attached() {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        assert!(link.is_attached(), "the real spawn must attach a transport");
        supervisor.stop(&mut link);
        assert!(!link.is_attached());
    }
}
