//! Bounded control queues and liveness, independent of a blocked PCM worker.
//!
//! Reader/writer threads own blocking streams. Their monitor never performs
//! stream I/O: a stuck writer cannot prevent it from closing a ring. Dropping
//! this object stops the monitor but cannot cancel an arbitrary `Read`/`Write`;
//! the process owner must close the pipes (kill/reap the child) on failure.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use rustamp_ipc::{Message, PROTOCOL_VERSION, StreamTransport, Transport};

use crate::{MediaHostError, Result};

/// A close-only ring view. It must never read or publish PCM/counters.
pub trait RingAbort: Send {
    /// Store the ring's closed state and signal both wake events.
    fn abort(&mut self);
}

/// Control timing policy. PCM waits themselves may remain infinite.
///
/// `Debug` because the engine carries the policy in its own configuration
/// (RFC 0006 §8 puts the supervision policy on the engine side).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Liveness {
    /// Interval between probes (and monitor shutdown checks).
    pub interval: Duration,
    /// Deadline for the initial Hello and each outstanding Pong.
    pub timeout: Duration,
}

impl Default for Liveness {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(100),
            timeout: Duration::from_secs(5),
        }
    }
}

#[derive(Default)]
struct State {
    failure: Option<String>,
    abort: Option<Box<dyn RingAbort>>,
}

enum Outgoing {
    Message(Box<Message>),
    Barrier(mpsc::SyncSender<()>),
}

impl State {
    fn close(&mut self) {
        if let Some(abort) = &mut self.abort {
            abort.abort();
        }
    }
    fn fail(&mut self, message: String) {
        self.failure.get_or_insert(message);
        self.close();
    }
}

/// Sends messages on a control connection from any thread, without waiting: the
/// way the player reaches a resident that another owner drives. A full queue or
/// an ended connection is an error here and the owner's to diagnose.
#[derive(Clone)]
pub struct ControlSender(mpsc::SyncSender<Outgoing>);

impl ControlSender {
    /// Queues `message`.
    pub fn send(&self, message: Message) -> Result<()> {
        self.0
            .try_send(Outgoing::Message(Box::new(message)))
            .map_err(|_| MediaHostError::Disconnected("control writer unavailable".into()))
    }
}

/// What a resident's output plug-in has played, as it last reported it.
#[derive(Clone)]
pub struct OutputClock(Arc<AtomicU64>);

impl OutputClock {
    /// The value before the first report.
    const NO_REPORT: u64 = u64::MAX;

    fn new() -> Self {
        OutputClock(Arc::new(AtomicU64::new(Self::NO_REPORT)))
    }

    fn store(&self, played_ms: u64) {
        self.0.store(played_ms, Ordering::Release);
    }

    /// The last report in milliseconds; `None` before the first one.
    pub fn played_ms(&self) -> Option<u64> {
        Some(self.0.load(Ordering::Acquire)).filter(|ms| *ms != Self::NO_REPORT)
    }
}

/// One supervised control connection. All methods are off-render-thread only.
pub struct ControlPeer {
    outgoing: mpsc::SyncSender<Outgoing>,
    incoming: mpsc::Receiver<Message>,
    state: Arc<Mutex<State>>,
    output_clock: OutputClock,
    stop: mpsc::Sender<()>,
    monitor: Option<thread::JoinHandle<()>>,
}

impl ControlPeer {
    /// Start framed reader/writer threads and a Hello/Ping/Pong monitor.
    /// `interrupt_reoffer` is true on resident ends (decoder host, DSP host):
    /// a seek/reconfigure command then closes the supervised ring, so a worker
    /// blocked in infinite PCM I/O wakes. `DspReset` never closes rings.
    pub fn start<R: Read + Send + 'static, W: Write + Send + 'static>(
        reader: R,
        writer: W,
        policy: Liveness,
        interrupt_reoffer: bool,
    ) -> Result<Self> {
        if policy.interval.is_zero() || policy.timeout <= policy.interval {
            return Err(MediaHostError::InvalidMessage(
                "invalid liveness timing".into(),
            ));
        }
        let (events_tx, events_rx) = mpsc::sync_channel(32);
        let (outgoing, outgoing_rx) = mpsc::sync_channel(32);
        let (incoming_tx, incoming) = mpsc::sync_channel(32);
        let (stop, stopped) = mpsc::channel();
        let state = Arc::new(Mutex::new(State::default()));
        let read_tx = events_tx.clone();
        thread::spawn(move || {
            let mut transport = StreamTransport::new(reader, std::io::sink());
            loop {
                let event = transport.recv().map_err(|e| e.to_string());
                let ended = event.is_err();
                if read_tx.send(event).is_err() || ended {
                    break;
                }
            }
        });
        thread::spawn(move || {
            let mut transport = StreamTransport::new(std::io::empty(), writer);
            while let Ok(packet) = outgoing_rx.recv() {
                match packet {
                    Outgoing::Message(message) => {
                        if let Err(error) = transport.send(&message) {
                            let _ = events_tx.send(Err(error.to_string()));
                            break;
                        }
                    }
                    Outgoing::Barrier(done) => {
                        let _ = done.try_send(());
                    }
                }
            }
        });
        let monitor_state = state.clone();
        let output_clock = OutputClock::new();
        let monitor_clock = output_clock.clone();
        let monitor_tx = outgoing.clone();
        // Queue Hello before publishing this connection to the caller. Doing
        // this inside the monitor would race an immediate DecoderOpen send.
        outgoing
            .try_send(Outgoing::Message(Box::new(Message::hello())))
            .map_err(|_| MediaHostError::Disconnected("control writer unavailable".into()))?;
        let monitor = thread::spawn(move || {
            let fail = |message: String| monitor_state.lock().unwrap().fail(message);
            let send = |message| {
                monitor_tx
                    .try_send(Outgoing::Message(Box::new(message)))
                    .map_err(|_| "control writer stalled".to_string())
            };
            let mut hello = false;
            let mut deadline = Instant::now() + policy.timeout;
            let mut next_ping = Instant::now();
            let mut pending = None;
            let mut token = 0u64;
            loop {
                if stopped.try_recv().is_ok() {
                    break;
                }
                if Instant::now() >= deadline && (!hello || pending.is_some()) {
                    fail("Hello/Pong liveness deadline expired".into());
                    break;
                }
                if hello && pending.is_none() && Instant::now() >= next_ping {
                    token = token.wrapping_add(1);
                    if let Err(error) = send(Message::Ping(token)) {
                        fail(error);
                        break;
                    }
                    pending = Some(token);
                    deadline = Instant::now() + policy.timeout;
                }
                match events_rx.recv_timeout(policy.interval.min(Duration::from_millis(20))) {
                    Ok(Ok(Message::Hello {
                        protocol_version, ..
                    })) if !hello && protocol_version == PROTOCOL_VERSION => {
                        hello = true;
                    }
                    Ok(Ok(Message::Hello { .. })) => {
                        fail("incompatible or duplicate Hello".into());
                        break;
                    }
                    Ok(Ok(_)) if !hello => {
                        fail("message before Hello".into());
                        break;
                    }
                    Ok(Ok(Message::Ping(token))) => {
                        if let Err(error) = send(Message::Pong(token)) {
                            fail(error);
                            break;
                        }
                    }
                    Ok(Ok(Message::Pong(token))) => {
                        if pending == Some(token) {
                            pending = None;
                            next_ping = Instant::now() + policy.interval;
                        }
                    }
                    Ok(Ok(Message::OutputClock { played_ms })) => {
                        monitor_clock.store(played_ms);
                    }
                    Ok(Ok(Message::DecoderError { message })) => {
                        fail(format!("resident decoder failed: {message}"));
                        break;
                    }
                    Ok(Ok(Message::DspError { message })) => {
                        fail(format!("resident DSP failed: {message}"));
                        break;
                    }
                    Ok(Ok(message)) => {
                        let shutdown = matches!(message, Message::Shutdown);
                        if matches!(message, Message::Shutdown)
                            || (interrupt_reoffer
                                && matches!(
                                    message,
                                    Message::DecoderSeek { .. } | Message::DspConfigure { .. }
                                ))
                        {
                            monitor_state.lock().unwrap().close();
                        }
                        if incoming_tx.try_send(message).is_err() {
                            fail("control consumer stalled".into());
                            break;
                        }
                        if shutdown {
                            break;
                        }
                    }
                    Ok(Err(error)) => {
                        fail(format!("control stream ended: {error}"));
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        fail("control workers exited".into());
                        break;
                    }
                }
            }
        });
        Ok(Self {
            outgoing,
            incoming,
            state,
            output_clock,
            stop,
            monitor: Some(monitor),
        })
    }

    /// Register a close-only view before exposing a new ring to the peer.
    pub fn set_abort(&self, mut abort: Box<dyn RingAbort>) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        if let Some(error) = &state.failure {
            abort.abort();
            return Err(MediaHostError::Disconnected(error.clone()));
        }
        state.abort = Some(abort);
        Ok(())
    }

    /// Relinquish local ring closure before sending a terminal DSP diagnostic.
    /// The remote monitor stores that diagnostic before closing its own abort
    /// handles. Local teardown must not race that store via shared ring state;
    /// if publication fails, remote EOF/liveness still closes the rings.
    pub(crate) fn release_abort(&self) {
        self.state.lock().unwrap().abort = None;
    }

    /// What the peer's output plug-in reports it has played.
    pub fn output_clock(&self) -> OutputClock {
        self.output_clock.clone()
    }

    /// A sender for this connection that other threads can keep.
    pub fn sender(&self) -> ControlSender {
        ControlSender(self.outgoing.clone())
    }

    /// Check the sticky failure reason; a dead host is never ordinary EOS.
    pub fn check(&self) -> Result<()> {
        match &self.state.lock().unwrap().failure {
            Some(error) => Err(MediaHostError::Disconnected(error.clone())),
            None => Ok(()),
        }
    }

    /// Queue a message without blocking on a child that stopped reading.
    pub fn send(&self, message: Message) -> Result<()> {
        self.check()?;
        self.outgoing
            .try_send(Outgoing::Message(Box::new(message)))
            .map_err(|_| {
                self.fail("control writer unavailable");
                MediaHostError::Disconnected("control writer unavailable".into())
            })
    }

    /// Wait a bounded time for previously queued messages to reach the stream.
    pub fn flush(&self, timeout: Duration) -> Result<()> {
        self.check()?;
        let (done, delivered) = mpsc::sync_channel(1);
        self.outgoing
            .try_send(Outgoing::Barrier(done))
            .map_err(|_| MediaHostError::Disconnected("control writer unavailable".into()))?;
        delivered
            .recv_timeout(timeout)
            .map_err(|_| MediaHostError::TimedOut)
    }

    /// Receive with a caller deadline (independent of PCM and heartbeat).
    pub fn recv(&self, timeout: Duration) -> Result<Message> {
        self.check()?;
        self.incoming.recv_timeout(timeout).map_err(|error| {
            if let Err(failure) = self.check() {
                return failure;
            }
            match error {
                mpsc::RecvTimeoutError::Timeout => MediaHostError::TimedOut,
                mpsc::RecvTimeoutError::Disconnected => {
                    MediaHostError::Disconnected("control monitor stopped".into())
                }
            }
        })
    }

    /// Wait for a command or terminal failure, without a caller deadline.
    /// Monitor exit disconnects the queue, so liveness failures still wake it.
    pub fn recv_blocking(&self) -> Result<Message> {
        self.check()?;
        self.incoming.recv().map_err(|_| {
            self.check()
                .err()
                .unwrap_or_else(|| MediaHostError::Disconnected("control monitor stopped".into()))
        })
    }

    /// Inspect commands between decoder blocks; does not sleep.
    pub fn try_recv(&self) -> Result<Option<Message>> {
        self.check()?;
        match self.incoming.try_recv() {
            Ok(message) => Ok(Some(message)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err(MediaHostError::Disconnected(
                "control monitor stopped".into(),
            )),
        }
    }

    /// Fail the conversation and wake any blocked PCM worker.
    pub fn fail(&self, message: &str) {
        self.state.lock().unwrap().fail(message.into());
    }
}

impl Drop for ControlPeer {
    fn drop(&mut self) {
        self.state.lock().unwrap().close();
        let _ = self.stop.send(());
        if let Some(monitor) = self.monitor.take() {
            let _ = monitor.join();
        }
    }
}
