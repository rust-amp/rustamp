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
//! # Messages this module deliberately ignores
//!
//! Any other variant returns [`ControlStep::Continue`] and touches neither the
//! surface nor the channel: `WinampIpc` has no reply variant in the protocol,
//! so answering one over the channel would invent protocol; classic plugins
//! keep querying the host window in-process instead. `LoadPlugin`,
//! `CommandToHost`, the ring offers and the RFC 0006 decoder/DSP messages
//! belong to other hosts or to future cards; acting on them here would be
//! guessing. See [`crate::state_bridge`] for the same honesty on the surface.

use rustamp_ipc::{Message, Transport};
use rustamp_proto::state::PlayerState;

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

/// Route one inbound message.
///
/// * `PlayerStateUpdate` is forwarded to `on_state` as an owned value; the
///   caller (the UI thread) applies it to the surface through the state
///   bridge, so this function never touches a window or a surface and stays
///   testable without either.
/// * `Ping` is answered with `Pong` carrying the same token; `Hello` is
///   answered with our own `Hello` (the peer's version is its business to
///   check — rejecting a mismatch is the app's decision, reported as a
///   finding in the module docs, not taken here).
/// * `Shutdown` returns [`ControlStep::Shutdown`].
/// * Anything else is ignored (see the module docs): [`ControlStep::Continue`]
///   with no send and no callback.
///
/// A reply that cannot be written means the app end is gone, so the step is
/// [`ControlStep::Shutdown`] rather than a silent drop: the host must exit,
/// not linger as an orphan.
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
        Message::Hello { .. } => match transport.send(&Message::hello()) {
            Ok(()) => ControlStep::Continue,
            Err(_) => ControlStep::Shutdown,
        },
        Message::Shutdown => ControlStep::Shutdown,
        _ => ControlStep::Continue,
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
    fn version_mismatch_still_answers_with_our_hello() {
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
        // The peer learns our version from the reply and decides itself;
        // rejecting the mismatch is the app's job (see the module docs).
        assert_eq!(
            app_end.recv().unwrap(),
            Message::Hello {
                protocol_version: PROTOCOL_VERSION,
                pid: std::process::id(),
            }
        );
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

    /// Really creates the host window. **VM-only and `#[ignore]`d:** it owns a
    /// real (hidden) window and must never run as part of the ordinary suite.
    #[test]
    #[ignore = "creates a real window; VM-only, never part of the default suite"]
    #[cfg(windows)]
    fn creates_the_real_host_window_vm_only() {
        let mut surface = IpcSurface::new();
        let window = crate::host_window::HostWindow::create(&mut surface)
            .expect("the VM creates the host window");
        assert_ne!(window.hwnd_raw(), 0);
        assert_eq!(surface.main_hwnd, window.hwnd_raw() as u32);
        window.destroy();
    }
}
