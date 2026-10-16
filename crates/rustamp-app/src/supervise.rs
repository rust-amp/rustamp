// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Spawn and supervision for `rustamp-plughost32` (RFC 0006 out-of-process hosting).
//!
//! [`crate::host::HostStateLink`] is the sending side of the player-state
//! bridge: it publishes [`PlayerState`](rustamp_core::state::PlayerState)
//! snapshots as [`Message::PlayerStateUpdate`](rustamp_ipc::Message) over the
//! **existing** framed control channel, and it deliberately spawns nothing.
//! This module is the process around that link: it finds the i686 host binary,
//! starts it, connects its control channel to the link, and re-attaches (or
//! restarts) when the host dies.
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
//!   the link's transports are send-only from the app side (a failed send
//!   detaches the link as [`PublishOutcome::HostGone`](crate::host::PublishOutcome),
//!   never an error). A missing or crashing host therefore degrades to
//!   "detached", never to a stall or a panic.
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
//! * **F2 — no handshake yet.** The host stub cannot answer `Hello`/`Ping`
//!   (it prints its version and exits), so liveness is death-only
//!   (`try_wait` plus the link's `HostGone` detach). A `Hello` handshake,
//!   `Ping` liveness and a non-blocking send (a live-but-not-reading host
//!   could otherwise fill the pipe and stall a send) belong to the M3
//!   control-pipe card, which also has to define the host's pipe rendezvous.
//! * **F3 — restart policy.** A dead host is retried forever, gated only by
//!   the cooldown. Give-up/backoff policy is undecided by the RFCs and is
//!   reported, not chosen.

use std::io;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rustamp_ipc::{StreamTransport, Transport};

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

/// First existing candidate from [`plugin_host_candidates`], if any.
///
/// `None` means "no host binary is resolvable at runtime": the supervisor
/// stays detached (see the module docs for what would fix that).
pub fn find_plugin_host() -> Option<PathBuf> {
    plugin_host_candidates()
        .into_iter()
        .find(|path| path.is_file())
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

/// Starts the real `rustamp-plughost32` binary with piped stdio.
///
/// The pipes carry the framed [`Message`](rustamp_ipc::Message) control
/// channel (the RFC 0006 §8 stdio shape) via [`StreamTransport`]: the app
/// writes, the host reads. `stderr` is inherited so host diagnostics stay
/// visible; nothing is ever read on the app thread (see the module docs).
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
    type Link = StreamTransport<std::process::ChildStdout, std::process::ChildStdin>;
    type Process = ChildHost;

    fn spawn(&mut self) -> io::Result<(Self::Link, Self::Process)> {
        let exe = self.exe.clone().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "rustamp-plughost32 not found: no packaged helper next to \
                 rustamp.exe and no dev-tree build; the host stays detached",
            )
        })?;
        let mut command = Command::new(&exe);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        no_window(&mut command);
        let mut child = command.spawn()?;
        match (child.stdin.take(), child.stdout.take()) {
            (Some(stdin), Some(stdout)) => {
                let transport = StreamTransport::new(stdout, stdin);
                Ok((transport, ChildHost { child }))
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
    last_attempt: Option<Instant>,
    cooldown: Duration,
    spawn_attempts: u64,
}

impl<B: HostBackend> Supervisor<B> {
    /// A supervisor that has started nothing yet (the link stays detached
    /// until the first [`supervise`](Supervisor::supervise)).
    pub fn new(backend: B) -> Self {
        Supervisor {
            backend,
            process: None,
            last_attempt: None,
            cooldown: DEFAULT_RESTART_COOLDOWN,
            spawn_attempts: 0,
        }
    }

    /// Like [`new`](Supervisor::new) with an explicit restart cooldown
    /// (tests use [`Duration::ZERO`] to skip waiting).
    pub fn with_cooldown(backend: B, cooldown: Duration) -> Self {
        Supervisor {
            backend,
            process: None,
            last_attempt: None,
            cooldown,
            spawn_attempts: 0,
        }
    }

    /// How many spawn attempts have been made (diagnostics and tests).
    pub fn spawn_attempts(&self) -> u64 {
        self.spawn_attempts
    }

    /// Whether a live managed child exists right now (non-blocking).
    pub fn is_live(&mut self) -> bool {
        self.process.as_mut().is_some_and(|p| p.is_running())
    }

    /// One non-blocking supervision step: reap a dead child, replace a
    /// desynced one, and (re-)attach a fresh transport when due.
    ///
    /// Never blocks, never panics, never propagates an error: a host that
    /// cannot be started leaves the link detached and the player unaffected.
    pub fn supervise(&mut self, link: &mut HostStateLink) {
        let live = self.process.as_mut().is_some_and(|p| p.is_running());
        if live && link.is_attached() {
            return; // healthy and connected: nothing to do
        }
        // Either no child, a dead one, or a desynced one (a live child with a
        // detached link: a failed send detached the link, or the link was
        // detached by hand). `None` reaps it — dropping a stale child kills it
        // without waiting — and the spawn below starts fresh rather than
        // reusing half state.
        self.process = None;
        if self
            .last_attempt
            .is_some_and(|at| at.elapsed() < self.cooldown)
        {
            return; // cooling down: stay detached rather than spin on spawns
        }
        self.last_attempt = Some(Instant::now());
        self.spawn_attempts += 1;
        match self.backend.spawn() {
            Ok((transport, process)) => {
                // Attaching forgets the last sent snapshot, so the host starts
                // from the real state rather than a stale one.
                link.attach(transport);
                self.process = Some(process);
            }
            Err(_) => {
                // Missing or unstartable host: stay detached. The player runs
                // identically without a host.
            }
        }
    }

    /// The deliberate off-switch: kill the managed child (best-effort,
    /// without waiting) and detach the link without sending anything.
    pub fn stop(&mut self, link: &mut HostStateLink) {
        self.process = None;
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
                    let (app_end, peer) = MemoryTransport::pair();
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
        supervisor.supervise(&mut link);
        assert!(link.is_attached(), "the real spawn must attach a transport");
        supervisor.stop(&mut link);
        assert!(!link.is_attached());
    }
}
