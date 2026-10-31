//! Host side of the control channel (RFC 0006 §8).
//!
//! The app spawns `rustamp-plughost32` with piped stdio (see
//! `rustamp-app/src/supervise.rs`) and publishes [`PlayerState`] snapshots as
//! [`Message::PlayerStateUpdate`] over that framed stream. This module is the
//! receiving side: it reads that stream and routes each message.
//!
//! # Threading
//!
//! The host window (and therefore the [`IpcSurface`]) is owned by the UI thread
//! (`src/host_window.rs`). [`run_reader`] runs on a separate reader thread that
//! owns the [`Transport`]: blocking reads on stdin must never stall the UI
//! thread that answers plugin `SendMessage` calls. Player snapshots cross to
//! the UI thread as owned [`PlayerState`] values through the caller's
//! `on_state` callback (in `main`: an `mpsc` channel the UI loop drains and
//! applies via [`apply_player_state`](crate::state_bridge::apply_player_state)).
//! Replies (`Hello`, `Pong`) need no surface and go out on the reader thread.
//!
//! # Liveness (no protocol change)
//!
//! Liveness reuses the existing `Hello`/`Ping`/`Pong` variants (RFC 0006 §8
//! runs "`Hello`/`Ping` liveness on the control channel"; the media hosts
//! answer the same way):
//!
//! * at startup the reader sends one [`Message::hello`] over the existing
//!   channel, so a future app side can tell "alive and reading" apart from
//!   "spawned but stuck" (today the supervisor only polls death; see the
//!   finding below);
//! * every inbound [`Message::Ping`] is answered with [`Message::Pong`]
//!   carrying the same token.
//!
//! The host sends exactly one `Hello` and one `Pong` per `Ping` (the app never
//! pings today), so nothing it writes can fill the pipe: a live-but-unread
//! host cannot stall the app's send from this side.
//!
//! # Shutdown
//!
//! The reader returns when the app asks ([`Message::Shutdown`]), when the pipe
//! closes ([`IpcError::Closed`], i.e. the app died or detached without killing
//! us), or when a reply cannot be written (the app end is gone). In all three
//! cases the process exits right after: no orphan survives a dead app, and the
//! supervisor's `kill` on its side needs no cooperation from us. A corrupt
//! frame ([`ControlExit::ChannelError`]) likewise ends the process rather than
//! running desynchronised.
//!
//! # Finding: app-side reading (reported, not changed)
//!
//! The app's link (`rustamp-app/src/host.rs`) is send-only today: nothing reads
//! the host's stdout, so our `Hello`/`Pong` frames sit in the pipe unread. That
//! is the supervisor's F2 item (its `Hello` handshake, `Ping` liveness and
//! non-blocking send belong to the M3 control-pipe card). This module changes
//! no protocol message; it only makes the host side speak the handshake the
//! RFC already documents, so the app side can start listening without any host
//! change.
//!
//! # Load requests
//!
//! [`Message::LoadPlugin`] is routed to the UI thread as
//! [`ControlInbound::Load`] (P3 §5.3). It is **not** acted on here: mapping a
//! DLL calls the plugin's `init` and must happen on the thread that owns the
//! host window (S09 §3.5, §7.2), which the reader thread is not. The UI thread
//! answers it through [`crate::registry::PluginRegistry`] with the
//! `PluginLoaded`/`PluginError` variants the protocol already has, so no wire
//! format changes. [`handle_message`], the reader-thread variant that owns the
//! transport and has no window, therefore still ignores it.
//!
//! # Plug-in requests on the wire (RFC 0005 §5.4, RFC 0020)
//!
//! Six requests arrive as [`Message::CommandToHost`] payloads. All six are
//! routed to the UI thread, because only that thread may call into a plug-in
//! and the dialogs are opened with the parent window the plug-in was handed:
//!
//! | Payload | Routed as | Acted on by |
//! |---|---|---|
//! | [`Command::ConfigureVisPlugin`] | [`ControlInbound::ConfigureVis`] | [`PluginRegistry::configure_vis`](crate::registry::PluginRegistry::configure_vis) |
//! | [`Command::ConfigurePlugin`] | [`ControlInbound::ConfigurePlugin`] | [`PluginRegistry::configure_plugin`](crate::registry::PluginRegistry::configure_plugin) |
//! | [`Command::SeekPlugin`] | [`ControlInbound::SeekPlugin`] | [`PluginRegistry::seek_input`](crate::registry::PluginRegistry::seek_input) |
//! | [`Command::PausePlugin`] | [`ControlInbound::PausePlugin`] | [`PluginRegistry::pause_input`](crate::registry::PluginRegistry::pause_input) |
//! | [`Command::UnpausePlugin`] | [`ControlInbound::UnpausePlugin`] | [`PluginRegistry::unpause_input`](crate::registry::PluginRegistry::unpause_input) |
//! | [`Command::StopPlugin`] | [`ControlInbound::StopPlugin`] | [`PluginRegistry::stop_input`](crate::registry::PluginRegistry::stop_input) |
//!
//! The first is RFC 0005's "the running visualization plug-in"; the five after
//! it are RFC 0020's, and each **names** a plug-in by the id the host assigned
//! in its `PluginLoaded` answer (plus, for a configure request, the kind the
//! request claims). [`apply_plugin_request`] is where the UI loop acts on one:
//! it calls the registry route and turns a refusal — an unknown id, a wrong
//! kind, a plug-in with no `config` slot, a plug-in that is not live — into the
//! message `main` prints on stderr. Nothing is ever called through a null slot
//! (S09 §0), and nothing is answered on the channel: the requests are
//! **fire-and-forget** (RFC 0020 §12), so a refusal reaches the user as a
//! diagnostic, not as a protocol message.
//!
//! # Start/stop requests
//!
//! The registry can start and stop a visualization
//! ([`crate::registry::PluginRegistry::start_vis`],
//! [`crate::registry::PluginRegistry::stop_vis`]), but **no request on the
//! wire reaches that path yet**.
//!
//! The vis requests the protocol carries are the Configure ones above and
//! [`Command::ToggleVisPlugin`], and the latter is a **unit variant**: it names
//! neither the plug-in nor the module, so it cannot say *what* to start. The
//! host would have to guess which of the loaded visualization plug-ins and
//! which of their modules the player means, and a guess that starts the wrong
//! plug-in is worse than no answer at all. A request that carries the plug-in
//! id and the module index needs a new `Command` variant with a payload — a
//! wire change, which needs an RFC — and none is invented here.
//! `ToggleVisPlugin` therefore still falls into the catch-all arm, like every
//! other command the host cannot act on.
//!
//! # Messages this module deliberately ignores
//!
//! Any other variant returns [`ControlStep::Continue`] and touches neither the
//! surface nor the channel: `WinampIpc` has no reply variant in the protocol,
//! so answering one over the channel would invent protocol; classic plugins
//! keep querying the host window in-process instead. `CommandToHost` carries
//! the player's own commands, and only the six requests above are ones the host
//! can act on; `ToggleVisPlugin` is one it cannot, because the wire does not say
//! what to start. The ring offers and the RFC 0006 decoder/DSP messages
//! (including the RFC 0015 `DspResetAck` reply, which only a DSP session's
//! player side expects) belong to other hosts or to future cards; acting on
//! them here would be guessing. There is also no message for "quit plugin N":
//! a request for that needs an RFC and the contract lane, and none is
//! invented here. See [`crate::state_bridge`] for the same honesty on the
//! surface.

use rustamp_ipc::{Message, Transport};
use rustamp_plugin_abi::PluginKind;
use rustamp_proto::command::Command;
use rustamp_proto::state::PlayerState;

use crate::registry::PluginRegistry;

/// What one handled message decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlStep {
    /// Keep reading.
    Continue,
    /// The app asked for shutdown: stop reading so the process can exit.
    Shutdown,
}

/// Why the reader thread returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlExit {
    /// The app sent [`Message::Shutdown`].
    ShutdownRequested,
    /// The pipe closed (clean EOF): the app died, detached, or was killed.
    /// Exiting now is what keeps no orphan behind.
    ChannelClosed,
    /// A frame failed to decode or the stream errored: the channel can no
    /// longer be trusted, so the process ends rather than running desynced.
    ChannelError,
}

/// Send the startup handshake: our [`Message::hello`] over the existing
/// channel. One small frame; it cannot fill the pipe.
pub fn send_hello(transport: &mut dyn Transport) -> ControlStep {
    match transport.send(&Message::hello()) {
        Ok(()) => ControlStep::Continue,
        Err(_) => ControlStep::Shutdown,
    }
}

/// Inbound events from the control channel routed to the UI loop.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlInbound {
    /// Player state snapshot update.
    State(PlayerState),
    /// What plays now.
    NowPlaying {
        /// The track's place in the playlist.
        position: u32,
        /// The track's title.
        title: String,
        /// The track's path or URL.
        file: String,
    },
    /// The sheet embed frames draw from; empty pixels when the skin has none.
    EmbedSkin {
        /// Width of the sheet in pixels.
        width: u32,
        /// Height of the sheet in pixels.
        height: u32,
        /// The pixels, 8-bit RGBA, top row first.
        rgba: Vec<u8>,
    },
    /// A ping from the parent with its token. The UI thread answers with Pong
    /// when it pumps messages, so a hung UI will not answer and trip the supervisor's timeout.
    Ping(u64),
    /// The parent asked for a plugin DLL to be loaded. Carried to the UI thread
    /// because that is where a plugin's `init` has to run (S09 §3.5, §7.2); the
    /// UI thread answers with `PluginLoaded` or `PluginError`.
    Load {
        /// Host-visible path of the DLL, exactly as the app sent it.
        path: String,
        /// The kind the app expects; a mismatch is refused before any mapping.
        kind: PluginKind,
    },
    /// The parent asked the host to open the running visualization plug-in's
    /// configuration dialog (RFC 0005 §5.4:
    /// [`Command::ConfigureVisPlugin`]). Carried to the UI thread because only
    /// that thread may call into a plugin, and the dialog is opened with the
    /// parent window the plug-in was handed.
    ConfigureVis,
    /// The parent asked the host to open the configuration dialog of the
    /// plug-in with this id (RFC 0020: [`Command::ConfigurePlugin`]). `kind` is
    /// the kind the request claims; the registry compares it with the kind it
    /// recorded when the plug-in was loaded and refuses a mismatch.
    ConfigurePlugin {
        /// Host-assigned plug-in id, as carried by the frame.
        plugin_id: u32,
        /// The kind the request claims the plug-in has.
        kind: PluginKind,
    },
    /// The parent asked the host to seek the input plug-in with this id
    /// (RFC 0020: [`Command::SeekPlugin`]). The plug-in flushes its own output;
    /// the host calls no output callback (S09 §3.7).
    SeekPlugin {
        /// Host-assigned plug-in id, as carried by the frame.
        plugin_id: u32,
        /// Target position in milliseconds, unchanged.
        to_ms: i32,
    },
    /// The parent asked the host to pause the input plug-in with this id
    /// (RFC 0020: [`Command::PausePlugin`]).
    PausePlugin {
        /// Host-assigned plug-in id, as carried by the frame.
        plugin_id: u32,
    },
    /// The parent asked the host to resume the input plug-in with this id
    /// (RFC 0020: [`Command::UnpausePlugin`]).
    UnpausePlugin {
        /// Host-assigned plug-in id, as carried by the frame.
        plugin_id: u32,
    },
    /// The parent asked the host to stop the input plug-in with this id
    /// (RFC 0020: [`Command::StopPlugin`]). The plug-in closes its own output
    /// (S09 §3.5 step 6).
    StopPlugin {
        /// Host-assigned plug-in id, as carried by the frame.
        plugin_id: u32,
    },
    /// The player asked the host to start or stop the visualization
    /// ([`Command::ToggleVisPlugin`]); the registry decides which plug-in and
    /// module (see [`PluginRegistry::toggle_vis`]). The UI loop acts on it,
    /// because it needs the registry mutably.
    ToggleVis,
    /// A frame of audio data for the running visualization
    /// ([`Message::Vis`]): the UI loop keeps the newest one.
    Vis(Box<rustamp_proto::vis::VisFrame>),
}

/// Route one inbound message to an event sink.
///
/// * `PlayerStateUpdate` is forwarded as `ControlInbound::State`.
/// * `Ping` is forwarded as `ControlInbound::Ping` to the UI thread (so Pong is
///   driven by UI responsiveness rather than answering from the reader thread).
/// * `LoadPlugin` is forwarded as `ControlInbound::Load`, for the same reason:
///   only the UI thread may call into a plugin.
/// * `CommandToHost` carrying one of the six plug-in requests of the module
///   docs is forwarded as its `ControlInbound` variant, for the same reason:
///   the request calls into a plug-in on the thread that owns the parent
///   window. Every other `CommandToHost` payload falls into the catch-all arm.
/// * `Hello` validates protocol compatibility. If incompatible, triggers shutdown.
/// * `Shutdown` triggers shutdown.
/// * Other variants are ignored.
pub fn handle_message_inbound(
    message: Message,
    on_event: &mut dyn FnMut(ControlInbound),
) -> ControlStep {
    match message {
        Message::PlayerStateUpdate(state) => {
            on_event(ControlInbound::State(*state));
            ControlStep::Continue
        }
        Message::Ping(token) => {
            on_event(ControlInbound::Ping(token));
            ControlStep::Continue
        }
        Message::LoadPlugin { path, kind } => {
            on_event(ControlInbound::Load { path, kind });
            ControlStep::Continue
        }
        Message::NowPlaying {
            position,
            title,
            file,
        } => {
            on_event(ControlInbound::NowPlaying {
                position,
                title,
                file,
            });
            ControlStep::Continue
        }
        Message::EmbedSkin {
            width,
            height,
            rgba,
        } => {
            on_event(ControlInbound::EmbedSkin {
                width,
                height,
                rgba,
            });
            ControlStep::Continue
        }
        Message::CommandToHost(Command::ConfigureVisPlugin) => {
            on_event(ControlInbound::ConfigureVis);
            ControlStep::Continue
        }
        Message::CommandToHost(Command::ToggleVisPlugin) => {
            on_event(ControlInbound::ToggleVis);
            ControlStep::Continue
        }
        Message::Vis(wire) => {
            // A frame whose shape does not decode is dropped: the visualization
            // keeps the last good one (and the stale-frame rule of the loop).
            if let Some(frame) = wire.to_frame() {
                on_event(ControlInbound::Vis(Box::new(frame)));
            }
            ControlStep::Continue
        }
        Message::CommandToHost(Command::ConfigurePlugin { plugin_id, kind }) => {
            on_event(ControlInbound::ConfigurePlugin { plugin_id, kind });
            ControlStep::Continue
        }
        Message::CommandToHost(Command::SeekPlugin { plugin_id, to_ms }) => {
            on_event(ControlInbound::SeekPlugin { plugin_id, to_ms });
            ControlStep::Continue
        }
        Message::CommandToHost(Command::PausePlugin { plugin_id }) => {
            on_event(ControlInbound::PausePlugin { plugin_id });
            ControlStep::Continue
        }
        Message::CommandToHost(Command::UnpausePlugin { plugin_id }) => {
            on_event(ControlInbound::UnpausePlugin { plugin_id });
            ControlStep::Continue
        }
        Message::CommandToHost(Command::StopPlugin { plugin_id }) => {
            on_event(ControlInbound::StopPlugin { plugin_id });
            ControlStep::Continue
        }
        Message::Hello { .. } => ControlStep::Continue,
        Message::Shutdown => ControlStep::Shutdown,
        _ => ControlStep::Continue,
    }
}

/// Act on one plug-in request routed to the UI thread (the six requests of the
/// module docs, RFC 0005 §5.4 and RFC 0020 §3.4).
///
/// Runs on the thread that owns the host window, because the registry calls the
/// plug-in from there (S09 §3.5, §7.2). Returns `Ok(())` when the request
/// reached the plug-in and `Err(message)` with the reason when it did not: an
/// unknown id, a wrong kind, a plug-in with no `config` slot or a plug-in that
/// is not live. The caller (`main`) writes the message to stderr, because
/// stdout is the frame stream — the requests are fire-and-forget, so a refusal
/// is a diagnostic and not a protocol message (RFC 0020 §12).
///
/// The other [`ControlInbound`] variants are not plug-in requests: they carry
/// state, a liveness probe or a load, and the UI loop handles them itself. They
/// do nothing here, so calling this with one cannot turn into a plug-in call.
pub fn apply_plugin_request(
    registry: &PluginRegistry,
    event: &ControlInbound,
) -> Result<(), String> {
    match event {
        ControlInbound::ConfigureVis => registry
            .configure_vis()
            .map_err(|error| format!("cannot configure the visualization plug-in: {error}")),
        ControlInbound::ConfigurePlugin { plugin_id, kind } => registry
            .configure_plugin(*plugin_id, *kind)
            .map_err(|error| format!("cannot configure plugin {plugin_id} (as {kind:?}): {error}")),
        ControlInbound::SeekPlugin { plugin_id, to_ms } => registry
            .seek_input(*plugin_id, *to_ms)
            .map_err(|error| format!("cannot seek plugin {plugin_id} to {to_ms} ms: {error}")),
        ControlInbound::PausePlugin { plugin_id } => registry
            .pause_input(*plugin_id)
            .map_err(|error| format!("cannot pause plugin {plugin_id}: {error}")),
        ControlInbound::UnpausePlugin { plugin_id } => registry
            .unpause_input(*plugin_id)
            .map_err(|error| format!("cannot resume plugin {plugin_id}: {error}")),
        ControlInbound::StopPlugin { plugin_id } => registry
            .stop_input(*plugin_id)
            .map_err(|error| format!("cannot stop plugin {plugin_id}: {error}")),
        // Not plug-in requests of this function: the UI loop acts on them itself
        // (it needs the registry mutably, or it keeps the frame).
        ControlInbound::State(_)
        | ControlInbound::NowPlaying { .. }
        | ControlInbound::EmbedSkin { .. }
        | ControlInbound::Ping(_)
        | ControlInbound::Load { .. }
        | ControlInbound::ToggleVis
        | ControlInbound::Vis(_) => Ok(()),
    }
}

/// Route one inbound message (answers Ping directly on transport).
///
/// * `PlayerStateUpdate` is forwarded to `on_state` as an owned value.
/// * `Ping` is answered with `Pong` carrying the same token.
/// * `Shutdown` returns [`ControlStep::Shutdown`].
/// * A load request and the six plug-in requests (RFC 0005 §5.4, RFC 0020) are
///   ignored here: they call into a plugin, and only the thread that owns the
///   host window may do that.
pub fn handle_message(
    message: Message,
    transport: &mut dyn Transport,
    on_state: &mut dyn FnMut(PlayerState),
) -> ControlStep {
    match message {
        Message::PlayerStateUpdate(state) => {
            on_state(*state);
            ControlStep::Continue
        }
        Message::Ping(token) => match transport.send(&Message::Pong(token)) {
            Ok(()) => ControlStep::Continue,
            Err(_) => ControlStep::Shutdown,
        },
        Message::Hello { .. } => ControlStep::Continue,
        Message::Shutdown => ControlStep::Shutdown,
        _ => ControlStep::Continue,
    }
}

/// Read until shutdown, close, or error (see [`ControlExit`]), routing events to `on_event`.
///
/// Sends the [`send_hello`] handshake first.
pub fn run_reader_events(
    transport: &mut dyn Transport,
    on_event: &mut dyn FnMut(ControlInbound),
) -> ControlExit {
    use rustamp_ipc::IpcError;
    if send_hello(transport) == ControlStep::Shutdown {
        return ControlExit::ChannelClosed;
    }
    loop {
        match transport.recv() {
            Ok(message) => {
                if handle_message_inbound(message, on_event) == ControlStep::Shutdown {
                    return ControlExit::ShutdownRequested;
                }
            }
            Err(IpcError::Closed) => return ControlExit::ChannelClosed,
            Err(_) => return ControlExit::ChannelError,
        }
    }
}

/// Read until shutdown, close, or error (see [`ControlExit`]).
///
/// Sends the [`send_hello`] handshake first. Designed for the reader thread:
/// it blocks in `recv`, owns `transport`, and never touches the surface —
/// snapshots leave through `on_state`.
pub fn run_reader(
    transport: &mut dyn Transport,
    on_state: &mut dyn FnMut(PlayerState),
) -> ControlExit {
    use rustamp_ipc::IpcError;
    if send_hello(transport) == ControlStep::Shutdown {
        return ControlExit::ChannelClosed;
    }
    loop {
        match transport.recv() {
            Ok(message) => {
                if handle_message(message, transport, on_state) == ControlStep::Shutdown {
                    return ControlExit::ShutdownRequested;
                }
            }
            Err(IpcError::Closed) => return ControlExit::ChannelClosed,
            Err(_) => return ControlExit::ChannelError,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{IpcAnswer, IpcSurface};
    use crate::state_bridge::apply_player_state;
    use rustamp_ipc::{MemoryTransport, PROTOCOL_VERSION};
    use rustamp_proto::command::Command;
    use rustamp_proto::state::{EqState, StreamInfo, TransportState};

    fn playing_state() -> PlayerState {
        PlayerState {
            transport: TransportState::Playing,
            position_ms: 12_345,
            duration_ms: Some(200_000),
            volume: 200,
            balance: -10,
            shuffle: true,
            repeat: true,
            current_index: Some(7),
            current_id: None,
            playlist_len: 60,
            stream: Some(StreamInfo {
                bitrate_kbps: Some(128),
                sample_rate_hz: 44_100,
                channels: 2,
            }),
            eq: EqState {
                bands: [12.0, -12.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                preamp: 0.0,
                enabled: true,
                auto: false,
            },
            stop_after_current: false,
        }
    }

    #[test]
    fn state_ping_and_hello_flow_over_a_fake_channel_and_shutdown_stops() {
        let (mut app_end, mut host_end) = MemoryTransport::pair();
        let state = playing_state();
        app_end
            .send(&Message::Hello {
                protocol_version: PROTOCOL_VERSION,
                pid: 1234,
            })
            .unwrap();
        app_end.send(&Message::Ping(9)).unwrap();
        app_end
            .send(&Message::PlayerStateUpdate(Box::new(state.clone())))
            .unwrap();
        app_end.send(&Message::Shutdown).unwrap();

        let mut received: Vec<PlayerState> = Vec::new();
        let exit = run_reader(&mut host_end, &mut |s| received.push(s));

        assert_eq!(exit, ControlExit::ShutdownRequested);
        // The handshake went out first, then the pong for token 9, in order.
        assert!(matches!(app_end.recv().unwrap(), Message::Hello { .. }));
        assert_eq!(app_end.recv().unwrap(), Message::Pong(9));
        // Exactly the one state snapshot crossed to the (fake) UI thread.
        assert_eq!(received, vec![state.clone()]);

        // The bridge applies it, and the surface answers through the measured
        // ids: 104 playing, 105 mode 0 position, 122 volume query, 250 shuffle.
        let mut surface = IpcSurface::new();
        apply_player_state(&mut surface, &received[0]);
        assert_eq!(surface.answer(104, 0).raw(), 1);
        assert_eq!(surface.answer(105, 0).raw(), 12_345);
        assert_eq!(surface.answer(122, -666).raw(), 200);
        assert_eq!(surface.answer(250, 0).raw(), 1);
    }

    #[test]
    fn honest_ids_keep_their_behaviour_after_state_arrives() {
        let (mut app_end, mut host_end) = MemoryTransport::pair();
        app_end
            .send(&Message::PlayerStateUpdate(Box::new(playing_state())))
            .unwrap();
        app_end.send(&Message::Shutdown).unwrap();

        let mut received: Vec<PlayerState> = Vec::new();
        assert_eq!(
            run_reader(&mut host_end, &mut |s| received.push(s)),
            ControlExit::ShutdownRequested
        );
        // Consume the startup Hello so later assertions see only test traffic.
        assert!(matches!(app_end.recv().unwrap(), Message::Hello { .. }));

        let mut surface = IpcSurface::new();
        apply_player_state(&mut surface, &received[0]);
        // While playing, 106 (seek) and 501 (video) need a playing reference
        // the protocol does not carry: nothing is invented.
        assert_eq!(
            surface.answer(106, 0),
            IpcAnswer::NeedsPlayingReference { id: 106 }
        );
        assert_eq!(
            surface.answer(501, 0),
            IpcAnswer::NeedsPlayingReference { id: 501 }
        );
        // Unknown ids still fall through to the S10 convention of 1.
        assert_eq!(surface.answer(0xDEAD_BEEF, 0), IpcAnswer::Unknown(1));
    }

    #[test]
    fn a_load_request_is_routed_to_the_ui_thread_and_answered_nowhere_else() {
        let (mut app_end, mut host_end) = MemoryTransport::pair();
        let request = Message::LoadPlugin {
            path: r"C:\plugins\gen_example.dll".to_string(),
            kind: rustamp_plugin_abi::PluginKind::General,
        };
        app_end.send(&request).unwrap();
        app_end.send(&Message::Shutdown).unwrap();

        let mut events = Vec::new();
        assert_eq!(
            run_reader_events(&mut host_end, &mut |ev| events.push(ev)),
            ControlExit::ShutdownRequested
        );
        assert_eq!(
            events,
            vec![ControlInbound::Load {
                path: r"C:\plugins\gen_example.dll".to_string(),
                kind: rustamp_plugin_abi::PluginKind::General,
            }],
            "a load request must reach the thread that owns the window"
        );
        // Only the startup Hello went out: the reader thread, which has no
        // window, answers a load request neither with PluginLoaded nor with
        // PluginError. That reply is the UI thread's (see run_ui_loop).
        assert!(matches!(app_end.recv().unwrap(), Message::Hello { .. }));
        assert!(app_end.try_recv().unwrap().is_none());

        // The transport-owning variant still does not act on it at all.
        let mut states = 0;
        assert_eq!(
            handle_message(request, &mut host_end, &mut |_| states += 1),
            ControlStep::Continue
        );
        assert_eq!(states, 0);
        assert!(app_end.try_recv().unwrap().is_none());
    }

    #[test]
    fn a_configure_request_reaches_the_ui_thread_and_other_host_commands_do_not() {
        let (mut app_end, mut host_end) = MemoryTransport::pair();
        // The player's Configure request for the visualization plug-in
        // (RFC 0005 §5.4). It travels in the message the protocol already has.
        app_end
            .send(&Message::CommandToHost(Command::ConfigureVisPlugin))
            .unwrap();
        // The toggle starts or stops the visualization: it reaches the UI
        // thread too (the registry decides what to start).
        app_end
            .send(&Message::CommandToHost(Command::ToggleVisPlugin))
            .unwrap();
        // A player→host command the host cannot act on: it must not reach the
        // UI thread as an event.
        app_end
            .send(&Message::CommandToHost(Command::Play))
            .unwrap();
        app_end.send(&Message::Shutdown).unwrap();

        let mut events = Vec::new();
        assert_eq!(
            run_reader_events(&mut host_end, &mut |ev| events.push(ev)),
            ControlExit::ShutdownRequested
        );
        assert_eq!(
            events,
            vec![ControlInbound::ConfigureVis, ControlInbound::ToggleVis],
            "the Configure and the toggle request must reach the thread that owns the window"
        );
        // Only the startup Hello went out: the reader thread has no window and
        // opens no dialog.
        assert!(matches!(app_end.recv().unwrap(), Message::Hello { .. }));
        assert!(app_end.try_recv().unwrap().is_none());
    }

    #[test]
    fn the_named_plugin_requests_reach_the_ui_thread_and_nothing_else_does() {
        let (mut app_end, mut host_end) = MemoryTransport::pair();
        // RFC 0020 §3.1.1: the five requests that name a plug-in, each with its
        // own payload, in one frame each.
        let requests = [
            Command::ConfigurePlugin {
                plugin_id: 7,
                kind: PluginKind::General,
            },
            Command::SeekPlugin {
                plugin_id: 8,
                to_ms: 12_345,
            },
            Command::PausePlugin { plugin_id: 9 },
            Command::UnpausePlugin { plugin_id: 10 },
            Command::StopPlugin { plugin_id: 11 },
        ];
        for request in &requests {
            app_end
                .send(&Message::CommandToHost(request.clone()))
                .unwrap();
        }
        // A player→host command the host cannot act on. It must not reach the
        // UI thread as an event.
        for ignored in [Command::Play, Command::Pause] {
            app_end.send(&Message::CommandToHost(ignored)).unwrap();
        }
        app_end.send(&Message::Shutdown).unwrap();

        let mut events = Vec::new();
        assert_eq!(
            run_reader_events(&mut host_end, &mut |ev| events.push(ev)),
            ControlExit::ShutdownRequested
        );
        assert_eq!(
            events,
            vec![
                ControlInbound::ConfigurePlugin {
                    plugin_id: 7,
                    kind: PluginKind::General,
                },
                ControlInbound::SeekPlugin {
                    plugin_id: 8,
                    to_ms: 12_345,
                },
                ControlInbound::PausePlugin { plugin_id: 9 },
                ControlInbound::UnpausePlugin { plugin_id: 10 },
                ControlInbound::StopPlugin { plugin_id: 11 },
            ],
            "every named plug-in request must reach the thread that owns the window"
        );
        // Fire-and-forget (RFC 0020 §12): nothing was answered. Only the
        // startup Hello went out.
        assert!(matches!(app_end.recv().unwrap(), Message::Hello { .. }));
        assert!(app_end.try_recv().unwrap().is_none());
    }

    // ---- applying a routed request on the UI thread (RFC 0020 §3.4) ----

    thread_local! {
        /// The general plug-in's `config` calls, in order.
        static CONFIG_LOG: std::cell::RefCell<Vec<&'static str>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    unsafe extern "C" fn general_config() {
        CONFIG_LOG.with(|log| log.borrow_mut().push("config"));
    }

    /// A synthetic general plug-in with (or without) a `config` slot.
    ///
    /// Leaked on purpose: the registry holds a raw pointer to it, exactly as it
    /// does into a mapped DLL's static data (S09 §0).
    fn general_plugin(
        name: &str,
        config: Option<unsafe extern "C" fn()>,
    ) -> crate::loader::LoadedPlugin {
        let module: &'static mut rustamp_plugin_abi::general::GeneralPlugin =
            Box::leak(Box::new(rustamp_plugin_abi::general::GeneralPlugin {
                version: rustamp_plugin_abi::common::GEN_VERSION,
                description: c"synthetic general".as_ptr(),
                config,
                ..rustamp_plugin_abi::general::GeneralPlugin::default()
            }));
        crate::loader::LoadedPlugin::general_for_test(name, module, true, 0)
    }

    /// A registry with a scripted input plug-in and a scripted output plug-in.
    ///
    /// The doubles are real `In_Module`/`Out_Module` structures with real cdecl
    /// callbacks (`crate::handoff::doubles`); no DLL is mapped and no window is
    /// created. The scripted track keeps playing, so the plug-in holds its
    /// output open until it is told to stop (S09 §3.1 offset 0x48, §3.5 step 6).
    fn registry_with_input() -> (PluginRegistry, u32) {
        crate::handoff::doubles::clear();
        let input = crate::loader::LoadedPlugin::input_for_test(
            "scripted_in",
            crate::handoff::doubles::input_module(crate::handoff::doubles::InputScript {
                track_ends: false,
                ..crate::handoff::doubles::InputScript::default()
            }),
            0x1234,
        );
        let output = crate::loader::LoadedPlugin::output_for_test(
            "scripted_out",
            crate::handoff::doubles::output_module(),
            true,
            0x1234,
        );
        let mut registry = PluginRegistry::new(crate::approval::ApprovalStore::new());
        let output_id = registry.insert_for_test(output);
        let input_id = registry.insert_for_test(input);
        registry
            .select_output(output_id)
            .expect("the scripted output must be selectable");
        // Hand the output over exactly as a play does, so the plug-in has one
        // to drive (S09 §3.5 step 2, §3.7).
        registry
            .play_input(
                input_id,
                std::path::Path::new(r"C:\music\track.mp3"),
                200,
                -10,
            )
            .expect("the hand-over and the play must succeed");
        crate::handoff::doubles::clear();
        (registry, input_id)
    }

    #[test]
    fn the_transport_requests_reach_the_input_plugin_through_the_abi() {
        let (registry, input_id) = registry_with_input();

        assert_eq!(
            apply_plugin_request(
                &registry,
                &ControlInbound::SeekPlugin {
                    plugin_id: input_id,
                    to_ms: 12_345,
                }
            ),
            Ok(())
        );
        // The host's whole part of a seek is the input plug-in's own `seek`,
        // and the plug-in flushes the output at the new time itself (S09 §3.7):
        // exactly one flush, and it came from the plug-in.
        assert_eq!(
            crate::handoff::doubles::calls(),
            vec!["seek 12345", "flush 12345 input"]
        );

        crate::handoff::doubles::clear();
        assert_eq!(
            apply_plugin_request(
                &registry,
                &ControlInbound::PausePlugin {
                    plugin_id: input_id
                }
            ),
            Ok(())
        );
        assert_eq!(
            apply_plugin_request(
                &registry,
                &ControlInbound::UnpausePlugin {
                    plugin_id: input_id
                }
            ),
            Ok(())
        );
        assert_eq!(
            crate::handoff::doubles::calls(),
            vec!["pause", "pause 1 input", "unpause", "pause 0 input"],
            "the input plug-in pauses and resumes the output it drives"
        );

        crate::handoff::doubles::clear();
        assert_eq!(
            apply_plugin_request(
                &registry,
                &ControlInbound::StopPlugin {
                    plugin_id: input_id
                }
            ),
            Ok(())
        );
        assert_eq!(
            crate::handoff::doubles::calls(),
            vec!["stop", "close input"],
            "the plug-in closes its own output on a stop"
        );
    }

    #[test]
    fn a_configure_request_reaches_the_named_plugins_config_slot() {
        CONFIG_LOG.with(|log| log.borrow_mut().clear());
        let mut registry = PluginRegistry::new(crate::approval::ApprovalStore::new());
        let id = registry.insert_for_test(general_plugin("gen_with_config", Some(general_config)));

        assert_eq!(
            apply_plugin_request(
                &registry,
                &ControlInbound::ConfigurePlugin {
                    plugin_id: id,
                    kind: PluginKind::General,
                }
            ),
            Ok(())
        );
        assert_eq!(CONFIG_LOG.with(|log| log.borrow().clone()), vec!["config"]);
    }

    #[test]
    fn the_three_refusals_are_reported_and_call_nothing() {
        CONFIG_LOG.with(|log| log.borrow_mut().clear());
        let mut registry = PluginRegistry::new(crate::approval::ApprovalStore::new());
        let with_config = registry.insert_for_test(general_plugin("gen_cfg", Some(general_config)));
        let without = registry.insert_for_test(general_plugin("gen_nocfg", None));

        // An unknown id: the host has loaded nothing under it.
        let unknown = apply_plugin_request(
            &registry,
            &ControlInbound::ConfigurePlugin {
                plugin_id: 9_999,
                kind: PluginKind::General,
            },
        )
        .expect_err("an unknown id must be refused");
        assert!(unknown.contains("no plugin with id 9999"), "{unknown}");

        // A wrong kind: the request claims Dsp, the loaded plug-in is a
        // general plug-in. Nothing may be called.
        let wrong_kind = apply_plugin_request(
            &registry,
            &ControlInbound::ConfigurePlugin {
                plugin_id: with_config,
                kind: PluginKind::Dsp,
            },
        )
        .expect_err("a wrong kind must be refused");
        assert!(
            wrong_kind.contains("cannot configure plugin")
                && wrong_kind.contains("asked for a Dsp plugin"),
            "{wrong_kind}"
        );

        // A plug-in with no `config` slot: a fact about that plug-in, and
        // still nothing called through a null slot (S09 §0).
        let no_slot = apply_plugin_request(
            &registry,
            &ControlInbound::ConfigurePlugin {
                plugin_id: without,
                kind: PluginKind::General,
            },
        )
        .expect_err("a missing config slot must be refused");
        assert!(no_slot.contains("no config callback"), "{no_slot}");

        assert!(
            CONFIG_LOG.with(|log| log.borrow().is_empty()),
            "a refused request must not have called the plug-in's config"
        );
    }

    #[test]
    fn an_unknown_id_is_refused_by_every_transport_route_and_calls_nothing() {
        crate::handoff::doubles::clear();
        let mut registry = PluginRegistry::new(crate::approval::ApprovalStore::new());
        let output = crate::loader::LoadedPlugin::output_for_test(
            "scripted_out",
            crate::handoff::doubles::output_module(),
            true,
            0x1234,
        );
        // An output plug-in under a known id: the transport routes need an
        // input plug-in, so the id/kind refusal is exercised without a window.
        let output_id = registry.insert_for_test(output);

        let refusals = [
            ControlInbound::SeekPlugin {
                plugin_id: 9_999,
                to_ms: 0,
            },
            ControlInbound::PausePlugin { plugin_id: 9_999 },
            ControlInbound::UnpausePlugin { plugin_id: 9_999 },
            ControlInbound::StopPlugin { plugin_id: 9_999 },
        ];
        for refused in refusals {
            let message = apply_plugin_request(&registry, &refused)
                .expect_err("an unknown id must be refused");
            assert!(message.contains("no plugin with id 9999"), "{message}");
        }

        // A known id of the wrong kind is refused by the same routes.
        for refused in [
            ControlInbound::SeekPlugin {
                plugin_id: output_id,
                to_ms: 0,
            },
            ControlInbound::PausePlugin {
                plugin_id: output_id,
            },
            ControlInbound::UnpausePlugin {
                plugin_id: output_id,
            },
            ControlInbound::StopPlugin {
                plugin_id: output_id,
            },
        ] {
            let message = apply_plugin_request(&registry, &refused)
                .expect_err("a wrong kind must be refused");
            assert!(message.contains("expected a Input plugin"), "{message}");
        }

        assert!(
            crate::handoff::doubles::calls().is_empty(),
            "a refused request must call nothing: {:?}",
            crate::handoff::doubles::calls()
        );
    }

    #[test]
    fn a_routed_state_ping_or_load_is_not_a_plugin_request() {
        // `apply_plugin_request` only acts on the six plug-in requests; the
        // other inbound variants carry no plug-in call and do nothing here.
        let registry = PluginRegistry::new(crate::approval::ApprovalStore::new());
        assert_eq!(
            apply_plugin_request(&registry, &ControlInbound::Ping(3)),
            Ok(())
        );
        assert_eq!(
            apply_plugin_request(
                &registry,
                &ControlInbound::Load {
                    path: r"C:\plugins\gen_x.dll".to_string(),
                    kind: PluginKind::General,
                }
            ),
            Ok(())
        );
    }

    #[test]
    fn foreign_messages_are_ignored_and_send_nothing() {
        let (mut app_end, mut host_end) = MemoryTransport::pair();
        for foreign in [
            Message::DecoderEos,
            Message::WinampIpc {
                msg_id: 104,
                wparam: 0,
                lparam: 0,
            },
            Message::DspReset,
            // The RFC 0015 acknowledgement is a host-to-player reply a plugin
            // host never expects; like DspReset it is ignored and sends nothing.
            Message::DspResetAck,
        ] {
            let mut states = 0;
            assert_eq!(
                handle_message(foreign, &mut host_end, &mut |_| states += 1),
                ControlStep::Continue
            );
            assert_eq!(states, 0);
        }
        // Nothing was sent except nothing: the peer sees no reply at all.
        // (The startup Hello is sent by run_reader, not handle_message.)
        assert!(app_end.try_recv().unwrap().is_none());
    }

    #[test]
    fn an_incoming_hello_is_not_answered_with_another_hello() {
        let (mut app_end, mut host_end) = MemoryTransport::pair();
        assert_eq!(
            handle_message(
                Message::Hello {
                    protocol_version: 0,
                    pid: 7,
                },
                &mut host_end,
                &mut |_| panic!("no state in a Hello"),
            ),
            ControlStep::Continue
        );
        // The peer's greeting is not answered: our own greeting already went out
        // at startup (`run_reader`), and a reply would let two peers greet each
        // other forever. Rejecting a version mismatch is the engine's job
        // (RFC 0006 section 8), so nothing is written here.
        assert!(app_end.try_recv().unwrap().is_none());
    }

    #[test]
    fn a_closed_channel_ends_the_loop_without_hanging() {
        let (app_end, mut host_end) = MemoryTransport::pair();
        drop(app_end);
        let mut received: Vec<PlayerState> = Vec::new();
        // The startup Hello fails against the dropped peer, which already
        // means the app end is gone.
        assert_eq!(
            run_reader(&mut host_end, &mut |s| received.push(s)),
            ControlExit::ChannelClosed
        );
        assert!(received.is_empty());
    }

    #[test]
    fn an_unwritable_reply_shuts_the_loop_down() {
        let (app_end, mut host_end) = MemoryTransport::pair();
        drop(app_end);
        // No recv is involved: the Ping is already here, but its Pong has
        // nowhere to go. Rather than linger as an orphan the host stops.
        let mut states = 0;
        assert_eq!(
            handle_message(Message::Ping(3), &mut host_end, &mut |_| states += 1),
            ControlStep::Shutdown
        );
        assert_eq!(states, 0);
    }

    #[test]
    fn run_reader_events_forwards_pings_and_states_without_auto_answering_pong() {
        let (mut app_end, mut host_end) = MemoryTransport::pair();
        app_end.send(&Message::Ping(42)).unwrap();
        let state = playing_state();
        app_end
            .send(&Message::PlayerStateUpdate(Box::new(state.clone())))
            .unwrap();
        app_end.send(&Message::Shutdown).unwrap();

        let mut events = Vec::new();
        let exit = run_reader_events(&mut host_end, &mut |ev| events.push(ev));
        assert_eq!(exit, ControlExit::ShutdownRequested);

        // Ping(42) and State were forwarded to the event sink:
        assert_eq!(
            events,
            vec![ControlInbound::Ping(42), ControlInbound::State(state)]
        );

        // The reader only sent the startup Hello — it did NOT send Pong(42).
        // Pong must be driven by the UI loop so a hung UI will not answer.
        assert!(matches!(app_end.recv().unwrap(), Message::Hello { .. }));
        assert!(app_end.try_recv().unwrap().is_none());
    }

    /// Really creates the host window. **VM-only and `#[ignore]`d:** it owns a
    /// real (hidden) window and must never run as part of the ordinary suite.
    /// Gated on the test VM ([`crate::host_window::require_test_vm`]), before
    /// any window exists.
    #[test]
    #[ignore = "creates a real window; VM-only, never part of the default suite"]
    #[cfg(windows)]
    fn creates_the_real_host_window_vm_only() {
        crate::host_window::require_test_vm();
        let surface = IpcSurface::new();
        let window = crate::host_window::HostWindow::create(surface)
            .expect("the VM creates the host window");
        assert_ne!(window.hwnd_raw(), 0);
        assert_eq!(
            window.snapshot().unwrap().main_hwnd,
            window.hwnd_raw() as u32
        );
        window.destroy();
    }
}

#[cfg(test)]
mod vis_feed_tests {
    use super::*;
    use rustamp_ipc::message::VisFrameWire;
    use rustamp_ipc::{MemoryTransport, Message};
    use rustamp_proto::vis::VisFrame;

    #[test]
    fn a_visualization_frame_reaches_the_ui_thread_decoded() {
        let (mut app_end, mut host_end) = MemoryTransport::pair();
        let mut frame = VisFrame {
            timestamp_us: 42,
            ..VisFrame::default()
        };
        frame.waveform[0][5] = 0.5;
        app_end
            .send(&Message::Vis(Box::new(VisFrameWire::from(&frame))))
            .unwrap();
        app_end.send(&Message::Shutdown).unwrap();

        let mut events = Vec::new();
        assert_eq!(
            run_reader_events(&mut host_end, &mut |ev| events.push(ev)),
            ControlExit::ShutdownRequested
        );
        assert_eq!(events, vec![ControlInbound::Vis(Box::new(frame))]);
    }
}
