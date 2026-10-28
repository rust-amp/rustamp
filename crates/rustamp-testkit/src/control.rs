// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The client half of the local control channel (`rustamp --control`).
//!
//! The player opens a named pipe named after its instance key
//! (`\\.\pipe\rustamp-ctl-<key>`) and answers **one JSON object per line**:
//! a request carries a mandatory `id`, then either a `cmd` (a
//! [`Command`](rustamp_proto::command::Command)) or an `op`; an answer echoes
//! the `id` and carries `ok`, a `message`, the `state`
//! ([`PlayerState`](rustamp_proto::state::PlayerState)) and the events a
//! `wait` matched.
//!
//! The envelope types below are this crate's own **mirror** of the server's
//! format (`rustamp_app::control`): a testkit cannot depend on the app crate
//! (it is the app's dev-dependency), and an independent implementation of the
//! same wire is what makes the round trip a test — a drift between the two
//! fails the test instead of compiling. The payload types (`Command`, `Event`,
//! `PlayerState`) are the shared `rustamp-proto` wire types, not mirrors.
//!
//! Provenance: the idea comes from oofz/rustamp's dev_server/dev_client
//! (MIT OR Apache-2.0); this implementation is ours (named pipe, typed serde,
//! request ids) and no source text of that project was copied.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

use rustamp_proto::command::Command;
use rustamp_proto::event::Event;
use rustamp_proto::state::PlayerState;
use serde::{Deserialize, Serialize};

/// How long [`ControlClient::connect`] retries before giving up.
///
/// The channel is created during the player's start-up, so a client that was
/// spawned first sees the pipe a moment later.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long the client waits between connection attempts.
const CONNECT_RETRY: Duration = Duration::from_millis(20);

/// The longest answer line the client accepts, in bytes.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// An operation of the channel that sends no command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    /// Answer with the current snapshot.
    Ping,
    /// Answer with the current snapshot.
    Snapshot,
    /// Wait for a matching event.
    Wait,
}

/// Which event a `wait` request waits for (the server's closed filter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventFilter {
    /// Every event.
    Any,
    /// A transport state change.
    StateChanged,
    /// A track change.
    TrackChanged,
    /// A playback position update, including the one a seek publishes.
    PositionChanged,
    /// A volume or balance change.
    VolumeChanged,
    /// A playlist change.
    PlaylistChanged,
    /// The end of the playlist.
    PlaylistEnded,
}

/// One request line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    /// The client's request id; the answer echoes it.
    pub id: u64,
    /// The command to hand to the player.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<Command>,
    /// The operation to run when no command is sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub op: Option<Op>,
    /// The event filter of a `wait` request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<EventFilter>,
    /// The `wait` timeout in milliseconds.
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

    /// A request that waits for `event` for at most `timeout`.
    pub fn wait(id: u64, event: EventFilter, timeout: Duration) -> Self {
        Request {
            id,
            cmd: None,
            op: Some(Op::Wait),
            event: Some(event),
            timeout_ms: Some(timeout.as_millis().max(1) as u64),
        }
    }
}

/// One answer line.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Response {
    /// The id of the request this answers.
    pub id: u64,
    /// Whether the request was carried out.
    pub ok: bool,
    /// What happened, in words the caller can show.
    pub message: String,
    /// The player state at the time of the answer (`None` on a refusal).
    pub state: Option<PlayerState>,
    /// The event a `wait` matched; empty for every other request.
    pub events: Vec<Event>,
}

impl Response {
    /// The one event of a `wait` answer, if the wait matched one.
    pub fn event(&self) -> Option<&Event> {
        self.events.first()
    }
}

/// A connected control channel.
///
/// The client is synchronous: every request writes one line and reads the next
/// one, so requests of one client never overtake each other. The pipe is
/// per-connection, so two clients do not interfere with each other's
/// requests; they do share the player, which is the point.
#[derive(Debug)]
pub struct ControlClient {
    /// The duplex pipe connection.
    file: File,
    /// The id the next request gets.
    next_id: u64,
}

impl ControlClient {
    /// Connects to `pipe` (`\\.\pipe\rustamp-ctl-<key>`), retrying until the
    /// player listens or [`CONNECT_TIMEOUT`] passes.
    pub fn connect(pipe: &str) -> std::io::Result<Self> {
        Self::connect_within(pipe, CONNECT_TIMEOUT)
    }

    /// Connects to `pipe`, retrying for at most `timeout`.
    ///
    /// The pipe is not created by this call: the player creates it, so a
    /// client that was started first retries here instead of failing.
    pub fn connect_within(pipe: &str, timeout: Duration) -> std::io::Result<Self> {
        let deadline = Instant::now() + timeout;
        loop {
            match OpenOptions::new().read(true).write(true).open(pipe) {
                Ok(file) => {
                    return Ok(ControlClient { file, next_id: 1 });
                }
                Err(error) => {
                    if Instant::now() >= deadline {
                        return Err(std::io::Error::new(
                            error.kind(),
                            format!("{pipe} accepted no connection within {timeout:?}: {error}"),
                        ));
                    }
                    std::thread::sleep(CONNECT_RETRY);
                }
            }
        }
    }

    /// Sends `cmd` and returns the answer.
    ///
    /// The answer only says the command was **accepted** into the player's
    /// command funnel; that it took effect is what [`Self::wait_for`] confirms.
    pub fn send(&mut self, cmd: Command) -> std::io::Result<Response> {
        let id = self.take_id();
        self.request(&Request::command(id, cmd))
    }

    /// Asks for the current snapshot.
    pub fn snapshot(&mut self) -> std::io::Result<PlayerState> {
        let id = self.take_id();
        let response = self.request(&Request::operation(id, Op::Snapshot))?;
        if !response.ok {
            return Err(std::io::Error::other(format!(
                "the snapshot was refused: {}",
                response.message
            )));
        }
        response.state.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the snapshot answer carries no state",
            )
        })
    }

    /// Sends a `ping`; the answer says whether the player is listening.
    pub fn ping(&mut self) -> std::io::Result<Response> {
        let id = self.take_id();
        self.request(&Request::operation(id, Op::Ping))
    }

    /// Waits for an event of `filter` for at most `timeout`.
    ///
    /// This is the channel's own typed filter; the answer's event is the first
    /// event of that kind published after the request (or, for events that
    /// arrived earlier on this connection, the first one still queued).
    pub fn wait(&mut self, filter: EventFilter, timeout: Duration) -> std::io::Result<Response> {
        let id = self.take_id();
        self.request(&Request::wait(id, filter, timeout))
    }

    /// Waits for the first event the `predicate` accepts, for at most
    /// `timeout`.
    ///
    /// A `wait` request covers the whole remaining time; an event that the
    /// predicate rejects is not an error, so the client waits again for the
    /// rest of the deadline. Nothing here sleeps: each round trip waits on the
    /// player's own event stream.
    pub fn wait_for<F>(&mut self, predicate: F, timeout: Duration) -> std::io::Result<Event>
    where
        F: Fn(&Event) -> bool,
    {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!("no event matched the predicate within {timeout:?}"),
                ));
            }
            let response = self.wait(EventFilter::Any, remaining)?;
            if let Some(event) = response.event() {
                if predicate(event) {
                    return Ok(event.clone());
                }
                continue;
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("no event arrived: {}", response.message),
            ));
        }
    }

    /// Sends one request and returns the answer it belongs to.
    pub fn request(&mut self, request: &Request) -> std::io::Result<Response> {
        let mut line = serde_json::to_string(request).map_err(std::io::Error::other)?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.flush()?;
        let text = self.read_line()?;
        let response: Response = serde_json::from_str(&text).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("the answer is not readable ({error}): {text}"),
            )
        })?;
        if response.id != request.id {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "the answer is for request {} and not for {}",
                    response.id, request.id
                ),
            ));
        }
        Ok(response)
    }

    /// The next request id (never 0, so a client's ids read clearly).
    fn take_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Reads one answer line.
    fn read_line(&mut self) -> std::io::Result<String> {
        let mut bytes = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match self.file.read(&mut byte) {
                Ok(0) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "the control channel closed the connection",
                    ));
                }
                Ok(_) => {
                    if byte[0] == b'\n' {
                        break;
                    }
                    bytes.push(byte[0]);
                    if bytes.len() > MAX_LINE_BYTES {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("the answer line is longer than {MAX_LINE_BYTES} bytes"),
                        ));
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
        String::from_utf8(bytes).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("the answer line is not UTF-8: {error}"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wait_request_carries_the_filter_and_the_timeout() {
        let request = Request::wait(5, EventFilter::StateChanged, Duration::from_millis(250));
        let text = serde_json::to_string(&request).unwrap();
        assert!(text.contains("\"id\":5"), "{text}");
        assert!(text.contains("\"op\":\"wait\""), "{text}");
        assert!(text.contains("\"event\":\"state_changed\""), "{text}");
        assert!(text.contains("\"timeout_ms\":250"), "{text}");
        assert!(!text.contains("cmd"), "a wait sends no command: {text}");
    }

    #[test]
    fn a_command_request_is_the_servers_shape() {
        let request = Request::command(1, Command::Play);
        let text = serde_json::to_string(&request).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap(),
            serde_json::json!({"id": 1, "cmd": "Play"}),
            "the request is one JSON object with the command in the wire form"
        );
    }

    #[test]
    fn an_answer_with_a_state_and_an_event_reads_back() {
        let text = r#"{"id":9,"ok":true,"message":"event","state":null,"events":[{"StateChanged":"Playing"}]}"#;
        let response: Response = serde_json::from_str(text).unwrap();
        assert_eq!(response.id, 9);
        assert!(response.ok);
        assert_eq!(
            response.event(),
            Some(&Event::StateChanged(
                rustamp_proto::state::TransportState::Playing
            ))
        );
        assert_eq!(response.state, None);
    }

    #[test]
    fn an_answer_is_matched_to_its_request_id() {
        // The id check lives in `request`; this pins the shape it compares.
        let text = r#"{"id":4,"ok":true,"message":"pong","state":null,"events":[]}"#;
        let response: Response = serde_json::from_str(text).unwrap();
        assert_ne!(response.id, 5);
    }
}
