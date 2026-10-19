// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! App side of the player-state bridge (RFC 0006 out-of-process hosting).
//!
//! The app (64-bit) owns the real player; the host (i686) owns the fake
//! `Winamp v1.x` window that plugins query through its answering surface. The
//! only state channel the [`rustamp_ipc::Message`] protocol provides in that
//! direction is [`Message::PlayerStateUpdate`], which carries a
//! [`PlayerState`] snapshot; the host's `state_bridge` copies that snapshot
//! into its surface fields. This module is the sending side: it assembles the
//! snapshot from the app's state and publishes it over the **existing**
//! [`Transport`] control channel. It invents no channel, spawns no process and
//! changes no protocol message.
//!
//! # Path (RFC 0006 §8)
//!
//! The helper talks to the player only over the documented protocol plus shared
//! memory. The control channel is a framed [`Message`] stream over any
//! `Read`/`Write` pair (piped stdio, a named pipe, a socket) via
//! [`StreamTransport`](rustamp_ipc::StreamTransport); the media hosts already
//! speak exactly that on stdin/stdout. The plugin host's binary does not name
//! a control pipe yet (its `main` is still a stub), so the supervisor (see
//! [`crate::supervise`]) uses the other documented transport — piped stdio.
//! [`HostStateLink`] therefore publishes through [`Transport::send`] — the
//! same path — and takes the transport as an injected dependency. Spawning
//! and supervising `rustamp-plughost32.exe` (finding the binary,
//! `CREATE_NO_WINDOW`, restart policy) lives in [`crate::supervise`], which
//! drives this link: it starts the host, attaches the fresh transport, and
//! re-attaches (or restarts) when the host dies. Until a supervisor is
//! installed (see [`App::manage_host`](crate::app::App::manage_host)) the
//! link stays detached and publishing is a no-op (see
//! [`PublishOutcome::Detached]).
//!
//! # Mapping (app source -> message field -> bridge surface + measured id)
//!
//! One place builds the message
//! ([`crate::app::App::player_state_update`]), one row per
//! field, auditable against `rustamp-plughost/src/state_bridge.rs`:
//!
//! * `engine.transport()` -> `state.transport` -> `surface.is_playing`
//!   (id 104 `IPC_ISPLAYING`; 0 stopped, 1 playing, 3 paused).
//! * `engine.position_ms()` -> `state.position_ms` ->
//!   `surface.output_position_ms` (id 105 mode 0).
//! * `engine` duration -> `state.duration_ms` -> `surface.output_length_ms`
//!   (id 105 modes 1/2; `None` means unknown, stored as 0).
//! * `engine` volume -> `state.volume` -> `surface.volume` (id 122, unsigned).
//! * `engine` balance -> `state.balance` -> `surface.pan` (id 123, signed).
//! * `engine` queue length -> `state.playlist_len` -> `surface.playlist_len`
//!   (id 124).
//! * `engine.current_index()` -> `state.current_index` -> `surface.playlist_pos`
//!   (id 125; `None` means no current entry, stored as 0).
//! * `engine` stream sample rate -> `state.stream.sample_rate_hz` ->
//!   `surface.sample_rate` (id 126 mode 0; `None` stream means 0).
//! * `engine` stream bitrate -> `state.stream.bitrate_kbps` -> `surface.bitrate`
//!   (id 126 mode 1; `None` stream or `None` bitrate means 0).
//! * `engine` stream channels -> `state.stream.channels` -> `surface.channels`
//!   (id 126 mode 2; `None` stream means 0).
//! * `engine` equalizer bands -> `state.eq.bands` -> `surface.eq_bands`
//!   (id 127 selectors 0..=9).
//! * `engine` equalizer preamp -> `state.eq.preamp` -> `surface.eq_preamp`
//!   (id 127 selector 10).
//! * `engine` equalizer enabled -> `state.eq.enabled` -> `surface.eq_enabled`
//!   (id 127 selector 11).
//! * `engine` equalizer autoload -> `state.eq.auto` -> `surface.eq_autoload`
//!   (id 127 selector 12).
//! * `App::shuffle` (app-owned; the engine deliberately reports `false`, RFC
//!   0001 / S15) -> `state.shuffle` -> `surface.shuffle` (ids 250/252).
//! * `App::repeat` (same) -> `state.repeat` -> `surface.repeat` (ids 251/253).
//! * `engine.stop_after_current()` -> `state.stop_after_current` -> ignored by
//!   the bridge (no surface field, no measured id reads it).
//! * `App::entry_ids` via [`App::entry_id_for`](crate::app::App::entry_id_for)
//!   (the engine reports `None`) -> `state.current_id` -> carried, but the
//!   bridge maps no surface field from it and no measured id reads it.
//!
//! The equalizer mapping is live: the engine owns the S07 stage (see
//! `rustamp_dsp::eq` and `rustamp_audio::Engine::eq`), so `state.eq` carries the
//! real bands, preamp, enabled and autoload. The app forwards the `SetBand`,
//! `SetPreamp`, `SetEqEnabled` and `SetEqAuto` commands to the engine and no
//! longer logs them as unimplemented. An untouched player still publishes the
//! fresh configuration's flat, engaged default, which the bridge renders as
//! the classic value 31.
//!
//! # Cadence and cost
//!
//! [`App::pump`](crate::app::App::pump) attempts a publication on every pass
//! (about 60 Hz windowed, every 5 ms headless), but the link sends only when
//! the snapshot changed since the last send: a discrete change (transport,
//! track, volume, modes, stream, EQ, stop-after-current) goes out immediately,
//! while a position-only drift — the only continuously moving field — goes out
//! at most every [`STATE_PUBLISH_INTERVAL`]. That interval mirrors the engine's
//! own `position_interval` (200 ms), so the host's time readout tracks the same
//! currency the UI machine already sees. Steady-state cost while playing is one
//! small postcard frame a few dozen bytes long at 5 Hz plus one frame per
//! discrete event; idle cost is a snapshot comparison per pump.
//!
//! Publishing is never on the audio path: it runs on the app thread inside
//! `pump`, touches only the engine's already-published snapshot accessors and
//! the app's own mode flags, and never takes a lock or allocates on the decode
//! or output threads. A stalled or absent host cannot block the player either:
//! with no transport attached publishing returns [`PublishOutcome::Detached`]
//! and does nothing, and a failed send detaches the link and reports
//! [`PublishOutcome::HostGone`] instead of propagating an error — `pump`
//! ignores the outcome outright.
//!
//! # Reading and liveness (RFC 0006 §8)
//!
//! Publishing is only half of the control channel: the host greets once with
//! [`Message::Hello`] at startup and answers every [`Message::Ping`] with a
//! same-token [`Message::Pong`] (see `rustamp-plughost/src/control.rs`). The
//! engine side of that handshake lives here, driven by [`HostStateLink::poll`],
//! which [`App::pump`](crate::app::App::pump) calls on every pass next to
//! [`publish`](HostStateLink::publish):
//!
//! * inbound traffic is drained through the non-blocking
//!   [`Transport::try_recv`](rustamp_ipc::Transport::try_recv) — never a
//!   blocking read, so the pump (and the player) cannot stall on it;
//! * the first step greets with our own [`Message::hello`], then a `Ping`
//!   goes out every [`PING_INTERVAL`] while no probe is outstanding;
//! * a `Pong` clears the probe it answers (tokens must match; a stale token
//!   is ignored rather than trusted); a probe unanswered for
//!   [`PONG_TIMEOUT`] is the RFC's missed-`Pong` failure, and the link
//!   detaches so the supervisor replaces the host.
//!
//! Which thread reads the real pipe depends on the transport, because a
//! blocking pipe cannot be polled:
//! [`MemoryTransport`](rustamp_ipc::MemoryTransport) answers `try_recv`
//! directly (tests drain it on the pump with no thread involved), while the
//! production pipe is split — the write half stays in the link and a reader
//! thread owns the read half, handing what it reads back through the same
//! `try_recv` (see [`crate::supervise`]). Either way reading and pinging stay
//! on the app thread or the reader thread: never on the audio path.
//!
//! # Commands to the host (RFC 0005 §5.4)
//!
//! The control channel carries both directions. The state direction is the
//! [`Message::PlayerStateUpdate`] postcard above; the command direction is
//! [`Message::CommandToHost`], which asks the host to do something only it
//! can do. The one command the app sends that way today is
//! [`Command::ConfigureVisPlugin`]: the visualization plug-in runs inside
//! `rustamp-plughost32.exe`, and only the host's UI thread may call into a
//! plug-in (S09 §3.5), so the app asks the host to open the plug-in's
//! configuration dialog instead of opening it itself. The request travels
//! over the **same** channel as the snapshots — no new transport, no new
//! message, no protocol change.
//!
//! Like publishing, sending a command never blocks the player: with no
//! transport attached nothing goes out ([`CommandOutcome::Detached`]), and a
//! failed send detaches the link and reports [`CommandOutcome::HostGone`].
//! The app turns the outcome into a note, so a request is never dropped
//! silently.
//!
//! # Findings (decisions the specs and RFCs do not make)
//!
//! * **F2 — a foreign greeting is rejected (decided).**
//!   [`poll`](HostStateLink::poll) records the host's version and pid
//!   ([`HostStateLink::greeting`]), and a greeting we do not speak
//!   ([`version_mismatch`](HostStateLink::version_mismatch)) detaches the
//!   link with [`PollOutcome::VersionMismatch`] — before our own greeting
//!   or any probe goes out — so the supervisor replaces the host. RFC 0006
//!   §8 puts the `Hello`/`Ping` liveness on the engine and turns a bad
//!   handshake into a failure (never a stall), and §10 demands a clean
//!   `Hello`-rejection test: running against a mismatched helper risks
//!   mis-decoded frames in both directions, so replacing it is the safe
//!   failure, not an invented policy. A persistently foreign binary
//!   therefore respawns on the cooldown — the same retry-forever policy as
//!   any dead host (see F3 in [`crate::supervise`]).
//! * **F3 — a non-reading host cannot stall the pump (bounded).** The pump
//!   never touches a blocking write handle: [`MemoryTransport`](rustamp_ipc::MemoryTransport)
//!   queues in memory, and the production pipe is split — the link keeps a
//!   bounded handover to a writer thread that owns the pipe's write half
//!   (see [`crate::supervise`]), mirroring the reader thread. A `send`
//!   therefore either queues or reports a full queue, never blocks; a full
//!   queue detaches like any failed send, and liveness replaces the host
//!   within [`PONG_TIMEOUT`]. A truly non-blocking OS write is impossible
//!   with `std` alone on the host platform (`ChildStdin` offers no
//!   non-blocking mode; overlapped I/O would need new dependencies), so the
//!   handover — not a syscall flag — is the bound.

use std::time::{Duration, Instant};

use rustamp_core::command::Command;
use rustamp_core::state::PlayerState;
use rustamp_ipc::{IpcError, Message, PROTOCOL_VERSION, Transport};

/// Maximum rate of position-only publications (see the module docs).
///
/// The value mirrors the engine's `position_interval`: the host's time readout
/// moves at the same currency as the UI machine's, and anything faster would
/// spend pipe bandwidth on milliseconds no plugin polls that often.
pub const STATE_PUBLISH_INTERVAL: Duration = Duration::from_millis(200);

/// How often an attached host gets a [`Message::Ping`] while no probe is
/// outstanding (see [`HostStateLink::poll`]).
///
/// Pipe round-trips are sub-millisecond locally, so one small frame a second
/// is plenty of liveness evidence and stays noise next to the 5 Hz state
/// postcards — while still noticing a wedged host within seconds.
pub const PING_INTERVAL: Duration = Duration::from_secs(1);

/// How long a [`Message::Ping`] may go unanswered before the host counts as
/// failed (see [`HostStateLink::poll`]).
///
/// Three times [`PING_INTERVAL`]: orders of magnitude above any healthy
/// scheduling stall on a local pipe, so a live host never trips it, but a
/// host that stopped answering (wedged reader, deadlocked UI thread) is
/// detached within seconds rather than left serving stale answers.
pub const PONG_TIMEOUT: Duration = Duration::from_secs(3);

/// What one [`HostStateLink::poll`] step decided.
///
/// Like [`PublishOutcome`], none of these is an error the player must
/// handle: [`App::pump`](crate::app::App::pump) ignores the outcome.
/// Detaching outcomes converge through the supervisor: the next
/// [`supervise`](crate::supervise::Supervisor::supervise) step sees a live
/// child with a detached link (or a dead one) and replaces it after the
/// restart cooldown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollOutcome {
    /// Attached and live: no probe is overdue. A `Ping` or the startup
    /// `Hello` may have gone out on this step.
    Healthy,
    /// No transport is attached; nothing was read and nothing was sent.
    Detached,
    /// The channel is gone (closed pipe, failed send, corrupt frame). The
    /// link detached itself; the supervisor replaces the host.
    HostGone,
    /// A `Ping` went unanswered for [`PONG_TIMEOUT`]: the RFC 0006 §8
    /// missed-`Pong` failure. The link detached itself; the supervisor
    /// replaces the host.
    MissedPong,
    /// The host greeted with a foreign protocol version: the RFC 0006 §8 bad
    /// handshake. The link detached itself before greeting or probing; the
    /// supervisor replaces the host (finding F2 in the module docs).
    VersionMismatch,
}

/// The host's startup [`Message::Hello`], as recorded by
/// [`HostStateLink::poll`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostGreeting {
    /// The host's [`PROTOCOL_VERSION`](rustamp_ipc::PROTOCOL_VERSION).
    pub protocol_version: u32,
    /// The host's process id.
    pub pid: u32,
}

impl HostGreeting {
    /// Whether the host speaks our protocol version. A mismatch detaches
    /// the link in [`poll_at`](HostStateLink::poll_at): running against a
    /// helper we cannot decode is the RFC 0006 §8 bad handshake, not a
    /// warning (finding F2 in the module docs).
    pub fn is_supported(&self) -> bool {
        self.protocol_version == PROTOCOL_VERSION
    }
}

/// What one [`HostStateLink::publish`] attempt did.
///
/// None of these is an error: the player treats them all as information, and
/// [`App::pump`](crate::app::App::pump) ignores the outcome. A dead host
/// becomes [`PublishOutcome::Detached`] on the next attempt (see
/// [`PublishOutcome::HostGone`]), never a failure the player must handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishOutcome {
    /// The snapshot went out on the transport.
    Sent,
    /// The snapshot equals the last one sent; nothing went out.
    Unchanged,
    /// Only `position_ms` moved and [`STATE_PUBLISH_INTERVAL`] has not elapsed
    /// since the last send; nothing went out.
    Throttled,
    /// No transport is attached; nothing went out and nothing was touched.
    Detached,
    /// The send failed (stalled or dead host). The link detached itself, so
    /// the next attempt reports [`PublishOutcome::Detached`]; the player was
    /// not blocked and saw no error.
    HostGone,
}

/// What one [`HostStateLink::send_command`] attempt did.
///
/// Like [`PublishOutcome`], none of these is an error the player must
/// handle: the app reports the outcome to the user instead of failing, so a
/// request is never dropped silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandOutcome {
    /// The command went out on the transport.
    Sent,
    /// No transport is attached; nothing went out and nothing was touched.
    Detached,
    /// The send failed (stalled or dead host). The link detached itself, so
    /// the next attempt reports [`CommandOutcome::Detached`].
    HostGone,
}

/// The app's end of the state bridge: an optional control-channel transport
/// plus the change detection that keeps publishing cheap, and the
/// `Hello`/`Ping` liveness state that keeps a wedged host from lingering
/// (see [`poll`](HostStateLink::poll)).
///
/// A detached link (the initial state, and the state after
/// [`PublishOutcome::HostGone`], [`PollOutcome::HostGone`],
/// [`PollOutcome::MissedPong`] or [`PollOutcome::VersionMismatch`]) makes [`publish`](HostStateLink::publish) a
/// no-op, so the player runs identically with and without a host.
pub struct HostStateLink {
    transport: Option<Box<dyn Transport>>,
    last_sent: Option<PlayerState>,
    last_send: Instant,
    greeting: Option<HostGreeting>,
    hello_sent: bool,
    next_token: u64,
    pending_pong: Option<(u64, Instant)>,
    next_ping_at: Option<Instant>,
}

impl HostStateLink {
    /// A link with no host: every [`publish`](HostStateLink::publish) returns
    /// [`PublishOutcome::Detached`] until [`attach`](HostStateLink::attach).
    pub fn detached() -> Self {
        HostStateLink {
            transport: None,
            last_sent: None,
            last_send: Instant::now(),
            greeting: None,
            hello_sent: false,
            next_token: 1,
            pending_pong: None,
            next_ping_at: None,
        }
    }

    /// Attach the control-channel transport (the RFC 0006 path: a framed
    /// [`Message`] stream, e.g. [`StreamTransport`](rustamp_ipc::StreamTransport)
    /// over piped stdio or the control pipe, or
    /// [`MemoryTransport`](rustamp_ipc::MemoryTransport) in tests).
    ///
    /// Attaching forgets the last sent snapshot, so the next
    /// [`publish`](HostStateLink::publish) sends unconditionally and the host
    /// starts from the real state rather than from a stale one.
    ///
    /// The pump calls `send` on this transport, so attach a handover-backed
    /// one ([`MemoryTransport`](rustamp_ipc::MemoryTransport), or the
    /// supervisor's split pipe link): a raw blocking pipe would stall
    /// `publish` on a stalled host (finding F3 in the module docs).
    pub fn attach(&mut self, transport: impl Transport + 'static) {
        self.transport = Some(Box::new(transport));
        self.last_sent = None;
        self.reset_liveness();
    }

    /// Drop the transport without sending anything. The next
    /// [`publish`](HostStateLink::publish) returns [`PublishOutcome::Detached`].
    pub fn detach(&mut self) {
        self.transport = None;
        self.last_sent = None;
        self.reset_liveness();
    }

    /// Forget the handshake state: a fresh transport means a fresh peer, so
    /// the next [`poll`](HostStateLink::poll) greets and probes from scratch.
    /// The token counter keeps running — tokens only need to be fresh within
    /// one peer's lifetime, and monotonicity across peers is the stronger
    /// property.
    fn reset_liveness(&mut self) {
        self.greeting = None;
        self.hello_sent = false;
        self.pending_pong = None;
        self.next_ping_at = None;
    }

    /// Whether a transport is currently attached.
    pub fn is_attached(&self) -> bool {
        self.transport.is_some()
    }

    /// The host's startup greeting, once [`poll`](HostStateLink::poll) has
    /// read it. `None` until the first `Hello` arrives.
    pub fn greeting(&self) -> Option<HostGreeting> {
        self.greeting
    }

    /// Record the host greeting obtained during handshake supervision.
    pub(crate) fn record_greeting(&mut self, protocol_version: u32, pid: u32) {
        if self.greeting.is_none() {
            self.greeting = Some(HostGreeting {
                protocol_version,
                pid,
            });
        }
    }

    /// Whether the recorded greeting speaks a foreign protocol version.
    /// [`poll_at`](HostStateLink::poll_at) detaches on this with
    /// [`PollOutcome::VersionMismatch`] (see finding F2 in the module docs);
    /// detaching forgets the greeting, so this reads `false` again
    /// afterwards.
    pub fn version_mismatch(&self) -> bool {
        self.greeting
            .is_some_and(|greeting| !greeting.is_supported())
    }

    /// One non-blocking liveness step: drain what the host wrote, greet and
    /// probe it, and detach on a dead channel or a missed `Pong`.
    ///
    /// Drive it from the app thread (e.g. [`App::pump`](crate::app::App::pump),
    /// next to [`publish`](HostStateLink::publish)): it only ever calls the
    /// non-blocking [`Transport::try_recv`], never `recv`, and sends at most
    /// one small frame, so it stays off the audio path and cannot stall the
    /// player. A transport that cannot be polled (a stream's write half)
    /// reports [`IpcError::WouldBlock`], which simply ends the drain — its
    /// inbound traffic arrives through the reader thread's handover instead
    /// (see [`crate::supervise`]), which surfaces through the same
    /// `try_recv`.
    pub fn poll(&mut self) -> PollOutcome {
        self.poll_at(Instant::now())
    }

    /// [`poll`](HostStateLink::poll) against an explicit clock, so tests can
    /// drive liveness deterministically without sleeping.
    pub fn poll_at(&mut self, now: Instant) -> PollOutcome {
        if self.transport.is_none() {
            return PollOutcome::Detached;
        }
        // Drain everything the host wrote since the last step: the startup
        // greeting and same-token pongs. Anything else is future traffic
        // (plugin answers belong to a later card) and is ignored, never
        // answered — answering an unknown variant would invent protocol.
        loop {
            let inbound = match self.transport.as_mut() {
                Some(transport) => transport.try_recv(),
                None => return PollOutcome::Detached,
            };
            match inbound {
                Ok(Some(message)) => self.on_inbound(message),
                Ok(None) => break,
                Err(IpcError::WouldBlock) => break,
                // Closed pipe, corrupt frame, wedged handover: the channel
                // can no longer be trusted. Detach and let the supervisor
                // replace the host, exactly as a failed send does in
                // `publish`.
                Err(_) => {
                    self.detach();
                    return PollOutcome::HostGone;
                }
            }
        }
        // A foreign greeting is the RFC 0006 §8 bad handshake (finding F2):
        // detach before greeting or probing, so the supervisor replaces a
        // host neither side can trust instead of running against it.
        if self.version_mismatch() {
            self.detach();
            return PollOutcome::VersionMismatch;
        }
        // A probe that outlived its deadline is the RFC 0006 §8 failure: the
        // host stopped answering. Detach so the supervisor replaces it.
        if self
            .pending_pong
            .is_some_and(|(_, deadline)| now >= deadline)
        {
            self.detach();
            return PollOutcome::MissedPong;
        }
        // Greet once per attach, then probe on the cadence. At most one probe
        // is ever outstanding: while one is unanswered the deadline above is
        // the only thing that moves, so a struggling host is failed rather
        // than flooded.
        if !self.hello_sent {
            if self.send(&Message::hello()).is_err() {
                self.detach();
                return PollOutcome::HostGone;
            }
            self.hello_sent = true;
            self.next_ping_at = Some(now + PING_INTERVAL);
        } else if self.pending_pong.is_none() && self.next_ping_at.is_some_and(|due| now >= due) {
            let token = self.next_token;
            self.next_token = self.next_token.wrapping_add(1);
            if self.send(&Message::Ping(token)).is_err() {
                self.detach();
                return PollOutcome::HostGone;
            }
            self.pending_pong = Some((token, now + PONG_TIMEOUT));
            self.next_ping_at = Some(now + PING_INTERVAL);
        }
        PollOutcome::Healthy
    }

    /// Route one message the host wrote.
    ///
    /// * `Hello` records the greeting (the first one wins: the host greets
    ///   once at startup, and a fresh peer means a fresh transport).
    /// * `Pong` clears the matching outstanding probe; a stale token is
    ///   ignored rather than trusted.
    /// * Anything else is ignored (see
    ///   [`poll_at`](HostStateLink::poll_at)).
    fn on_inbound(&mut self, message: Message) {
        match message {
            Message::Hello {
                protocol_version,
                pid,
            } => {
                if self.greeting.is_none() {
                    self.greeting = Some(HostGreeting {
                        protocol_version,
                        pid,
                    });
                }
            }
            Message::Pong(token)
                if self
                    .pending_pong
                    .is_some_and(|(pending, _)| pending == token) =>
            {
                self.pending_pong = None;
            }
            Message::Pong(_) => {}
            _ => {}
        }
    }

    /// Send one control message (`Hello`/`Ping`); the caller detaches on
    /// failure. `None` when detached, which [`poll_at`](HostStateLink::poll_at)
    /// rules out before calling.
    fn send(&mut self, message: &Message) -> Result<(), IpcError> {
        match self.transport.as_mut() {
            Some(transport) => transport.send(message),
            None => Err(IpcError::Closed),
        }
    }

    /// Publish `state` when the cadence rules say so (see the module docs).
    ///
    /// Never blocks the player: with no host it returns
    /// [`PublishOutcome::Detached`], and a failed send detaches the link and
    /// returns [`PublishOutcome::HostGone`] instead of an error. "Never
    /// blocks" holds through a handover-backed transport (see
    /// [`attach`](HostStateLink::attach)): the send either queues or fails,
    /// so a stalled host costs a detach, never a stall.
    pub fn publish(&mut self, state: &PlayerState) -> PublishOutcome {
        let Some(transport) = self.transport.as_mut() else {
            return PublishOutcome::Detached;
        };
        if self.last_sent.as_ref() == Some(state) {
            return PublishOutcome::Unchanged;
        }
        let position_only = self
            .last_sent
            .as_ref()
            .is_some_and(|last| position_only_change(last, state));
        if position_only && self.last_send.elapsed() < STATE_PUBLISH_INTERVAL {
            return PublishOutcome::Throttled;
        }
        match transport.send(&player_state_message(state.clone())) {
            Ok(()) => {
                self.last_sent = Some(state.clone());
                self.last_send = Instant::now();
                PublishOutcome::Sent
            }
            Err(_) => {
                // A stalled or dead host must never block the player and must
                // never wedge the link on a broken transport: drop it and let
                // the supervisor (see `crate::supervise`) re-attach a live one.
                self.transport = None;
                self.last_sent = None;
                PublishOutcome::HostGone
            }
        }
    }

    /// Send one command to the host (RFC 0005 §5.4, the player -> host
    /// direction).
    ///
    /// The visualization plug-in runs in the host process, and only the
    /// host's UI thread may call into a plug-in (S09 §3.5), so the app asks
    /// the host to open the plug-in's configuration dialog instead of opening
    /// it itself: [`Message::CommandToHost`] carries the request over the
    /// same control channel as the state snapshots.
    ///
    /// Never blocks the player: with no host attached nothing goes out
    /// ([`CommandOutcome::Detached`]), and a failed send detaches the link
    /// and reports [`CommandOutcome::HostGone`] instead of propagating an
    /// error. The caller turns the outcome into a note, so a request is
    /// never dropped silently.
    pub fn send_command(&mut self, command: Command) -> CommandOutcome {
        if self.transport.is_none() {
            return CommandOutcome::Detached;
        }
        match self.send(&Message::CommandToHost(command)) {
            Ok(()) => CommandOutcome::Sent,
            Err(_) => {
                // Same rule as a failed publish: the channel cannot be
                // trusted any more, so the link detaches and the supervisor
                // replaces the host.
                self.detach();
                CommandOutcome::HostGone
            }
        }
    }
}

/// Whether `after` differs from `before` in `position_ms` only.
///
/// `position_ms` is the one continuously moving field, so it gets the
/// throttled cadence while every other change goes out immediately.
fn position_only_change(before: &PlayerState, after: &PlayerState) -> bool {
    let mut settled_before = before.clone();
    let mut settled_after = after.clone();
    settled_before.position_ms = 0;
    settled_after.position_ms = 0;
    settled_before == settled_after
}

/// Wrap a snapshot in the single protocol message the bridge reads.
///
/// This is the only construction site: every `PlayerStateUpdate` the app sends
/// passes through here, so the field-by-field audit in the module docs stays
/// checkable in one place. The protocol change rule holds — the variant
/// already exists and is untouched.
pub fn player_state_message(state: PlayerState) -> Message {
    Message::PlayerStateUpdate(Box::new(state))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::command::Command;
    use rustamp_core::config::Config;
    use rustamp_core::playlist::EntryId;
    use rustamp_core::state::{EqState, StreamInfo, TransportState};

    use crate::app::{App, AppOptions};
    use crate::cli::OutputChoice;

    fn sample_state() -> PlayerState {
        PlayerState {
            transport: TransportState::Playing,
            position_ms: 12_345,
            duration_ms: Some(200_000),
            volume: 200,
            balance: -10,
            shuffle: true,
            repeat: true,
            current_index: Some(7),
            current_id: Some(EntryId(9)),
            playlist_len: 60,
            stream: Some(StreamInfo {
                bitrate_kbps: Some(128),
                sample_rate_hz: 44_100,
                channels: 2,
            }),
            eq: EqState {
                bands: [12.0, -12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                preamp: 1.5,
                enabled: true,
                auto: true,
            },
            stop_after_current: true,
        }
    }

    fn headless_app(volume: u8) -> App {
        let options = AppOptions {
            config: Config::default(),
            skin: crate::skin::builtin(),
            output: OutputChoice::Null,
            volume,
            file: None,
            seconds: None,
        };
        App::new(options).expect("headless app builds without media")
    }

    #[test]
    fn construction_carries_every_message_field() {
        let state = sample_state();
        let Message::PlayerStateUpdate(boxed) = player_state_message(state.clone()) else {
            panic!("the app builds PlayerStateUpdate, nothing else");
        };
        // The whole snapshot first, then each field by name so a dropped field
        // fails loudly against the bridge audit in the module docs.
        assert_eq!(*boxed, state);
        assert_eq!(boxed.transport, TransportState::Playing);
        assert_eq!(boxed.position_ms, 12_345);
        assert_eq!(boxed.duration_ms, Some(200_000));
        assert_eq!(boxed.volume, 200);
        assert_eq!(boxed.balance, -10);
        assert!(boxed.shuffle);
        assert!(boxed.repeat);
        assert_eq!(boxed.current_index, Some(7));
        assert_eq!(boxed.current_id, Some(EntryId(9)));
        assert_eq!(boxed.playlist_len, 60);
        assert_eq!(
            boxed.stream,
            Some(StreamInfo {
                bitrate_kbps: Some(128),
                sample_rate_hz: 44_100,
                channels: 2,
            })
        );
        assert_eq!(boxed.eq.bands[0], 12.0);
        assert_eq!(boxed.eq.bands[1], -12.0);
        assert_eq!(boxed.eq.preamp, 1.5);
        assert!(boxed.eq.enabled);
        assert!(boxed.eq.auto);
        assert!(boxed.stop_after_current);
    }

    #[test]
    fn the_assembled_snapshot_merges_engine_and_app_state() {
        let mut app = headless_app(200);
        app.submit(Command::SetShuffle(true))
            .expect("shuffle is app-owned");
        app.submit(Command::SetRepeat(true))
            .expect("repeat is app-owned");
        let snapshot = app.host_player_state();
        // Engine-owned: configured volume, stopped transport, empty queue.
        assert_eq!(snapshot.volume, 200);
        assert_eq!(snapshot.balance, 0);
        assert_eq!(snapshot.transport, TransportState::Stopped);
        assert_eq!(snapshot.playlist_len, 0);
        assert_eq!(snapshot.current_index, None);
        // App-owned: the modes the engine deliberately reports as false...
        assert!(snapshot.shuffle);
        assert!(snapshot.repeat);
        // ...and no invented entry id for an empty queue.
        assert_eq!(snapshot.current_id, None);
        // The engine's equalizer is untouched here, so its published state is
        // still the fresh configuration's honest flat, engaged default
        // (`EqConfig::default`, not the wire contract's disengaged default).
        assert_eq!(snapshot.eq, Config::default().eq.state);
        // The message built from the app carries exactly that snapshot.
        let Message::PlayerStateUpdate(boxed) = app.player_state_update() else {
            panic!("the app builds PlayerStateUpdate, nothing else");
        };
        assert_eq!(*boxed, snapshot);
    }

    #[test]
    fn detached_link_sends_nothing_and_never_fails() {
        let mut link = HostStateLink::detached();
        assert!(!link.is_attached());
        assert_eq!(link.publish(&sample_state()), PublishOutcome::Detached);
        assert!(!link.is_attached());
        // The app-level path is a no-op too, and the player keeps pumping.
        let mut app = headless_app(200);
        assert!(!app.host_attached());
        assert_eq!(app.publish_host_state(), PublishOutcome::Detached);
        app.pump()
            .expect("publishing without a host never fails pump");
    }

    #[test]
    fn attached_link_sends_once_and_throttles_position_only() {
        use rustamp_ipc::MemoryTransport;

        let (app_end, mut host_end) = MemoryTransport::pair();
        let mut link = HostStateLink::detached();
        link.attach(app_end);
        assert!(link.is_attached());

        let mut state = sample_state();
        assert_eq!(link.publish(&state), PublishOutcome::Sent);
        // The peer decodes the real codec bytes into the same message.
        assert_eq!(
            host_end.recv().expect("one framed message"),
            player_state_message(state.clone())
        );
        // An unchanged snapshot sends nothing.
        assert_eq!(link.publish(&state), PublishOutcome::Unchanged);
        assert!(host_end.try_recv().expect("codec reads").is_none());

        // Position-only drift inside the interval is throttled...
        state.position_ms += 30;
        assert_eq!(link.publish(&state), PublishOutcome::Throttled);
        assert!(host_end.try_recv().expect("codec reads").is_none());

        // ...but a discrete change (volume) goes out immediately.
        state.volume = state.volume.saturating_sub(1);
        assert_eq!(link.publish(&state), PublishOutcome::Sent);
        let received = host_end.recv().expect("the discrete change");
        let Message::PlayerStateUpdate(boxed) = received else {
            panic!("the app builds PlayerStateUpdate, nothing else");
        };
        assert_eq!(boxed.volume, state.volume);
        assert_eq!(boxed.position_ms, state.position_ms);
    }

    #[test]
    fn a_dead_host_detaches_instead_of_blocking() {
        use rustamp_ipc::MemoryTransport;

        let (app_end, host_end) = MemoryTransport::pair();
        drop(host_end);
        let mut link = HostStateLink::detached();
        link.attach(app_end);
        // The failed send is absorbed: the link detaches and says so.
        assert_eq!(link.publish(&sample_state()), PublishOutcome::HostGone);
        assert!(!link.is_attached());
        // From here on it is the ordinary no-host path.
        assert_eq!(link.publish(&sample_state()), PublishOutcome::Detached);
    }

    #[test]
    fn detaching_stops_publication_without_a_send() {
        use rustamp_ipc::MemoryTransport;

        let (app_end, mut host_end) = MemoryTransport::pair();
        let mut link = HostStateLink::detached();
        link.attach(app_end);
        link.detach();
        assert!(!link.is_attached());
        assert_eq!(link.publish(&sample_state()), PublishOutcome::Detached);
        // What matters is that nothing was sent. After `detach` the peer's write
        // end is gone, so the read may legitimately report the closed end rather
        // than an empty queue - but it must never yield a message.
        match host_end.try_recv() {
            Ok(None) => {}
            Ok(Some(_)) => panic!("detaching must not publish anything"),
            Err(_) => {} // the peer end was dropped by `detach`, which is the point
        }
    }

    #[test]
    fn a_healthy_host_stays_attached_across_hellos_and_pings() {
        use rustamp_ipc::MemoryTransport;

        let (app_end, mut host_end) = MemoryTransport::pair();
        let mut link = HostStateLink::detached();
        link.attach(app_end);
        let t0 = Instant::now();

        // The host greeted at startup; the first poll consumes it and answers
        // with our own greeting.
        host_end
            .send(&Message::Hello {
                protocol_version: PROTOCOL_VERSION,
                pid: 4321,
            })
            .unwrap();
        assert_eq!(link.poll_at(t0), PollOutcome::Healthy);
        assert!(link.is_attached());
        let greeting = link.greeting().expect("the Hello is consumed");
        assert_eq!(greeting.protocol_version, PROTOCOL_VERSION);
        assert_eq!(greeting.pid, 4321);
        assert!(greeting.is_supported());
        assert!(!link.version_mismatch());
        assert!(matches!(host_end.recv().unwrap(), Message::Hello { .. }));

        // No probe is due yet: silence on the channel.
        assert!(host_end.try_recv().expect("codec reads").is_none());

        // At the cadence a Ping goes out; its same-token Pong keeps the link
        // attached, round after round.
        assert_eq!(link.poll_at(t0 + PING_INTERVAL), PollOutcome::Healthy);
        assert_eq!(host_end.recv().unwrap(), Message::Ping(1));
        host_end.send(&Message::Pong(1)).unwrap();
        assert_eq!(
            link.poll_at(t0 + PING_INTERVAL + Duration::from_millis(10)),
            PollOutcome::Healthy
        );
        assert!(link.is_attached());

        assert_eq!(link.poll_at(t0 + 2 * PING_INTERVAL), PollOutcome::Healthy);
        assert_eq!(host_end.recv().unwrap(), Message::Ping(2));
        host_end.send(&Message::Pong(2)).unwrap();
        assert_eq!(
            link.poll_at(t0 + 2 * PING_INTERVAL + Duration::from_millis(10)),
            PollOutcome::Healthy
        );
        assert!(link.is_attached());
    }

    #[test]
    fn a_host_that_stops_ponging_detaches_after_the_timeout() {
        use rustamp_ipc::MemoryTransport;

        let (app_end, mut host_end) = MemoryTransport::pair();
        let mut link = HostStateLink::detached();
        link.attach(app_end);
        let t0 = Instant::now();

        assert_eq!(link.poll_at(t0), PollOutcome::Healthy);
        assert!(matches!(host_end.recv().unwrap(), Message::Hello { .. }));

        // The Ping goes out on cadence, but this host never answers it.
        let ping_at = t0 + PING_INTERVAL;
        assert_eq!(link.poll_at(ping_at), PollOutcome::Healthy);
        assert_eq!(host_end.recv().unwrap(), Message::Ping(1));

        // A stale token does not clear the probe: only the same-token Pong
        // counts, per the RFC.
        host_end.send(&Message::Pong(999)).unwrap();
        assert_eq!(
            link.poll_at(ping_at + PONG_TIMEOUT - Duration::from_millis(1)),
            PollOutcome::Healthy
        );
        assert!(link.is_attached());

        // Past the deadline the missed-Pong failure detaches the link, and
        // the supervisor replaces the host on its next step.
        assert_eq!(
            link.poll_at(ping_at + PONG_TIMEOUT),
            PollOutcome::MissedPong
        );
        assert!(!link.is_attached());
        assert_eq!(
            link.poll_at(ping_at + PONG_TIMEOUT + Duration::from_secs(1)),
            PollOutcome::Detached
        );
    }

    #[test]
    fn a_foreign_greeting_version_detaches_for_replacement() {
        use rustamp_ipc::MemoryTransport;

        let (app_end, mut host_end) = MemoryTransport::pair();
        let mut link = HostStateLink::detached();
        link.attach(app_end);
        host_end
            .send(&Message::Hello {
                protocol_version: PROTOCOL_VERSION + 1000,
                pid: 7,
            })
            .unwrap();

        // A foreign version is the RFC 0006 §8 bad handshake (finding F2):
        // the link detaches so the supervisor replaces the host instead of
        // running against frames neither side can trust.
        assert_eq!(link.poll_at(Instant::now()), PollOutcome::VersionMismatch);
        assert!(!link.is_attached());
        // Detaching forgets the handshake, so the report reads clean and the
        // next step is the ordinary no-host path.
        assert!(link.greeting().is_none());
        assert!(!link.version_mismatch());
        assert_eq!(link.poll_at(Instant::now()), PollOutcome::Detached);
        // Nothing went out to the rejected peer: the drain runs before the
        // greeting, so not even our own `Hello` was sent. After `detach`
        // the peer's write end is gone, so the read may legitimately report
        // the closed end rather than an empty queue — but it must never
        // yield a message.
        match host_end.try_recv() {
            Ok(None) => {}
            Ok(Some(_)) => panic!("a rejected host must not be greeted"),
            Err(_) => {} // the peer end was dropped by `detach`, which is the point
        }
    }

    #[test]
    fn foreign_inbound_traffic_is_ignored_never_answered() {
        use rustamp_ipc::MemoryTransport;

        let (app_end, mut host_end) = MemoryTransport::pair();
        let mut link = HostStateLink::detached();
        link.attach(app_end);
        // Traffic a later card owns: plugin answers the app never solicited.
        host_end
            .send(&Message::WinampIpc {
                msg_id: 104,
                wparam: 0,
                lparam: 0,
            })
            .unwrap();
        host_end.send(&Message::DecoderEos).unwrap();

        assert_eq!(link.poll_at(Instant::now()), PollOutcome::Healthy);
        assert!(link.is_attached());
        assert!(link.greeting().is_none());
        // Only our own startup greeting went out: nothing was answered, so no
        // protocol was invented.
        assert!(matches!(host_end.recv().unwrap(), Message::Hello { .. }));
        assert!(host_end.try_recv().expect("codec reads").is_none());
    }

    #[test]
    fn a_closed_channel_detaches_without_a_panic() {
        use rustamp_ipc::MemoryTransport;

        let (app_end, host_end) = MemoryTransport::pair();
        drop(host_end);
        let mut link = HostStateLink::detached();
        link.attach(app_end);
        // The peer is gone: the drain reports Closed, so the link detaches
        // instead of panicking — and stays detached afterwards.
        assert_eq!(link.poll_at(Instant::now()), PollOutcome::HostGone);
        assert!(!link.is_attached());
        assert_eq!(link.poll_at(Instant::now()), PollOutcome::Detached);
    }

    #[test]
    fn a_command_reaches_the_host_over_the_same_channel() {
        use rustamp_ipc::MemoryTransport;

        let (app_end, mut host_end) = MemoryTransport::pair();
        let mut link = HostStateLink::detached();
        link.attach(app_end);

        assert_eq!(
            link.send_command(Command::ConfigureVisPlugin),
            CommandOutcome::Sent
        );
        // The peer decodes the real codec bytes into the same message.
        assert_eq!(
            host_end.recv().expect("one framed message"),
            Message::CommandToHost(Command::ConfigureVisPlugin)
        );
        // Nothing else went out: one request, one frame.
        assert!(host_end.try_recv().expect("codec reads").is_none());
    }

    #[test]
    fn a_detached_link_refuses_the_command_instead_of_dropping_it() {
        let mut link = HostStateLink::detached();
        assert_eq!(
            link.send_command(Command::ConfigureVisPlugin),
            CommandOutcome::Detached
        );
        assert!(!link.is_attached());
    }

    #[test]
    fn a_dead_host_detaches_on_a_failed_command() {
        use rustamp_ipc::MemoryTransport;

        let (app_end, host_end) = MemoryTransport::pair();
        drop(host_end);
        let mut link = HostStateLink::detached();
        link.attach(app_end);
        // The failed send is absorbed: the link detaches and says so.
        assert_eq!(
            link.send_command(Command::ConfigureVisPlugin),
            CommandOutcome::HostGone
        );
        assert!(!link.is_attached());
        // From here on it is the ordinary no-host path.
        assert_eq!(
            link.send_command(Command::ConfigureVisPlugin),
            CommandOutcome::Detached
        );
    }
}
