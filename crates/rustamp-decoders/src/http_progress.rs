//! What an HTTP transfer is doing, as the application can read it.
//!
//! S19 §2 lists the status texts the reference player shows while an
//! `http://` stream runs. This module makes the **facts** behind those texts
//! observable: the phase the transfer is in, the bytes that came in and the
//! length the server declared. The texts themselves belong to the
//! application, which owns the window - this crate shows none of them.
//!
//! # The shape
//!
//! The transfer writes into a shared state of atomics, and the application
//! reads a snapshot out of it:
//!
//! * A **callback** would run on the thread of the transfer - the decode
//!   thread - and a callback that can block would stall the transfer. One
//!   that cannot block cannot carry what a status line needs (a rate needs
//!   two samples, a text needs a decision), so it would have to be a queue:
//!   more machinery than the facts need.
//! * A **shared snapshot** costs four atomic loads and no allocation: the
//!   transfer writes the phase and the counters, the application reads them
//!   when it draws. The read never waits for the transfer, and the transfer
//!   never waits for the application.
//!
//! The snapshot carries the phase, the bytes read and the declared total -
//! and nothing else. The **rate** is not carried: the application derives it
//! from two samples of the byte count and its own clock, so the transfer
//! never reads a clock on the hot path:
//!
//! ```text
//! rate = (bytes2 - bytes1) / (t2 - t1)
//! ```
//!
//! # The phases
//!
//! The phases are the ones S19 §2 names, in the order the spec lists them.
//! The spec does not establish the order in which the player shows them
//! (§6, question 15): this enum follows the table of the spec, and the
//! application picks the text per phase.
//!
//! Two phases of the spec are **not** declared here: the ones of the proxy
//! path ("Connecting to proxy", "Error connecting to proxy"). This tree has
//! no proxy: the shape of the proxy setting is open (S19 §6, question 13),
//! the source refuses a URL that carries credentials, and there is no proxy
//! path to observe. A phase that no code can produce would be a promise the
//! tree cannot keep; a proxy path adds one variant when it lands.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

/// The phase of an HTTP transfer.
///
/// The phases are the ones S19 §2 names, in the order the spec lists them.
/// The spec does not establish the order in which the player shows them
/// (§6, question 15): this enum follows the table of the spec, and the
/// application chooses the text per phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HttpPhase {
    /// The host is being reached; the connection is not up yet.
    ///
    /// S19 §2: "Connecting to host...".
    Connecting = 0,
    /// The request is on its way to the host.
    ///
    /// S19 §2: "Sending HTTP request".
    SendingRequest = 1,
    /// The answer of the host is being read.
    ///
    /// S19 §2: "Reading HTTP response".
    ReadingResponse = 2,
    /// The body of the answer is being read.
    ///
    /// S19 §2: "Retrieving file..." and "Retrieving file (N% done)". The
    /// two texts differ in the total, not in the phase.
    Retrieving = 3,
    /// The connection died before the body was complete.
    ///
    /// S19 §2: "Connection to host lost".
    ConnectionLost = 4,
    /// The server closed the connection at the end of the body.
    ///
    /// S19 §2: "Connection closed.".
    Closed = 5,
    /// The connection could not be established.
    ///
    /// S19 §2: "Error connecting to host".
    ConnectFailed = 6,
}

impl HttpPhase {
    /// The number the phase is stored as.
    const fn code(self) -> u8 {
        self as u8
    }

    /// The phase a stored number names.
    ///
    /// A number that names no phase is read as [`HttpPhase::Connecting`]:
    /// only this crate writes the number, so a wrong one would mean a
    /// transfer that never left its first phase.
    fn from_code(code: u8) -> Self {
        match code {
            1 => Self::SendingRequest,
            2 => Self::ReadingResponse,
            3 => Self::Retrieving,
            4 => Self::ConnectionLost,
            5 => Self::Closed,
            6 => Self::ConnectFailed,
            _ => Self::Connecting,
        }
    }
}

/// The state of an HTTP transfer at one moment.
///
/// The application reads it and turns it into the status line. The texts
/// are the application's (S19 §2 names them, this crate shows none of them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HttpProgressSnapshot {
    /// The phase the transfer is in.
    pub phase: HttpPhase,
    /// The bytes of the body that came in.
    pub bytes_read: u64,
    /// The bytes the server declared, when it declared any.
    ///
    /// `None` while the head of the response is not read yet, and `None`
    /// for the rest of the transfer when the server declared no length: a
    /// live stream has no total, and the total is exactly what the two
    /// transfer texts of S19 §2 differ in.
    pub total: Option<u64>,
}

/// The observer of an HTTP transfer.
///
/// The application creates it before the transfer starts, hands it to the
/// decoder and reads a [`HttpProgressSnapshot`] out of it whenever it draws
/// the status line. A clone is the same state and not a copy of it, so the
/// application can keep its own handle for the whole life of the transfer -
/// and read it even when the open fails, which is when the phase says that
/// the host could not be reached.
#[derive(Debug, Clone)]
pub struct HttpProgress {
    /// The state the transfer writes into.
    state: Arc<State>,
}

impl Default for HttpProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl HttpProgress {
    /// A new observer of a transfer that is about to start.
    ///
    /// The phase is [`HttpPhase::Connecting`]: the first thing a transfer
    /// does is to reach the host, and the first text of S19 §2 is the one
    /// shown before the connection is made.
    pub fn new() -> Self {
        HttpProgress {
            state: Arc::new(State::new()),
        }
    }

    /// What the transfer is doing right now.
    ///
    /// The read is four atomic loads at most: it never waits for the
    /// transfer and it never allocates.
    pub fn snapshot(&self) -> HttpProgressSnapshot {
        self.state.snapshot()
    }

    /// The phase the transfer is in.
    pub(crate) fn phase(&self) -> HttpPhase {
        self.state.phase()
    }

    /// Sets the phase the transfer is in.
    pub(crate) fn set_phase(&self, phase: HttpPhase) {
        self.state.set_phase(phase);
    }

    /// The bytes of the body that came in.
    pub(crate) fn bytes_read(&self) -> u64 {
        self.state.bytes_read()
    }

    /// Counts the bytes of the body that came in.
    pub(crate) fn add_bytes(&self, n: u64) {
        self.state.add_bytes(n);
    }

    /// The bytes the server declared, when it declared any.
    pub(crate) fn total(&self) -> Option<u64> {
        self.state.total()
    }

    /// States the bytes the server declared.
    ///
    /// The total is set once, when the head of the response is read: before
    /// that there is none, and a response that declares no length never has
    /// one.
    pub(crate) fn set_total(&self, total: Option<u64>) {
        self.state.set_total(total);
    }

    /// Notes that the connection died before the declared end.
    pub(crate) fn mark_cut(&self) {
        self.state.mark_cut();
    }

    /// Whether the connection died before the declared end.
    pub(crate) fn is_cut(&self) -> bool {
        self.state.is_cut()
    }
}

/// The state the transfer writes into and the application reads out of.
///
/// Every field is an atomic: the write must not block the transfer, and the
/// read must not block the application.
#[derive(Debug)]
struct State {
    /// The phase of the transfer.
    phase: AtomicU8,
    /// The bytes of the body that came in.
    bytes_read: AtomicU64,
    /// The bytes the server declared.
    total: AtomicU64,
    /// Whether the server declared a length at all.
    has_total: AtomicBool,
    /// Whether the connection died before the declared end.
    cut: AtomicBool,
}

impl State {
    /// The state of a transfer that is about to start.
    fn new() -> Self {
        State {
            phase: AtomicU8::new(HttpPhase::Connecting.code()),
            bytes_read: AtomicU64::new(0),
            total: AtomicU64::new(0),
            has_total: AtomicBool::new(false),
            cut: AtomicBool::new(false),
        }
    }

    /// The phase the transfer is in.
    fn phase(&self) -> HttpPhase {
        HttpPhase::from_code(self.phase.load(Ordering::Acquire))
    }

    /// Sets the phase the transfer is in.
    fn set_phase(&self, phase: HttpPhase) {
        self.phase.store(phase.code(), Ordering::Release);
    }

    /// The bytes of the body that came in.
    fn bytes_read(&self) -> u64 {
        self.bytes_read.load(Ordering::Relaxed)
    }

    /// Counts the bytes of the body that came in.
    fn add_bytes(&self, n: u64) {
        self.bytes_read.fetch_add(n, Ordering::Relaxed);
    }

    /// The bytes the server declared, when it declared any.
    fn total(&self) -> Option<u64> {
        if self.has_total.load(Ordering::Acquire) {
            Some(self.total.load(Ordering::Acquire))
        } else {
            None
        }
    }

    /// States the bytes the server declared.
    fn set_total(&self, total: Option<u64>) {
        if let Some(total) = total {
            self.total.store(total, Ordering::Release);
            self.has_total.store(true, Ordering::Release);
        }
    }

    /// Notes that the connection died before the declared end.
    fn mark_cut(&self) {
        self.cut.store(true, Ordering::Release);
    }

    /// Whether the connection died before the declared end.
    fn is_cut(&self) -> bool {
        self.cut.load(Ordering::Acquire)
    }

    /// What the transfer is doing right now.
    fn snapshot(&self) -> HttpProgressSnapshot {
        HttpProgressSnapshot {
            phase: self.phase(),
            bytes_read: self.bytes_read(),
            total: self.total(),
        }
    }
}
