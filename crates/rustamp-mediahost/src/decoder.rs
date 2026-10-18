//! RFC 0006 decoder worker, resident interface and player-side PCM client.
//!
//! Residents adapt their real decoder here without linking codec code into the
//! permissive framework. All counts are **frames**, never interleaved samples.
//! A core `Decoder` adapter and engine registration belong to the audio lane.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use rustamp_ipc::Message;
use rustamp_ipc::shm::{
    MappedRegion, RingConsumer, RingError, RingKind, RingProducer, RingRefuseReason,
};
use rustamp_proto::host::{MediaLocationWire, PcmSpecWire};
use rustamp_proto::meta::TrackMetadata;

use crate::control::ControlPeer;
use crate::pcm::{PcmRings, RingCounters, read_frames, validate_spec, write_frames};
use crate::{MediaHostError, Result};

const CONTROL_WAIT: Duration = Duration::from_secs(5);
const BLOCK_FRAMES: usize = 1024;

/// Snapshot of the open resident decoder (no GPL contract types on this edge).
#[derive(Clone)]
pub struct DecoderInfo {
    /// Current interleaved-f32 format.
    pub spec: PcmSpecWire,
    /// Duration, when known.
    pub duration_ms: Option<u64>,
    /// Whether seeking is supported.
    pub seekable: bool,
    /// Initial metadata.
    pub metadata: TrackMetadata,
}

/// Decoder supplied by a separately licensed resident crate.
pub trait ResidentDecoder: Send {
    /// Current format and initial properties; re-read after FormatChanged/seek.
    fn info(&self) -> DecoderInfo;
    /// Read whole frames. Zero means EOS. FormatChanged must return no samples
    /// and update `info().spec` before returning the error.
    fn read(&mut self, out: &mut [f32]) -> Result<usize>;
    /// Seek and return the reached position in milliseconds.
    fn seek(&mut self, to_ms: u64) -> Result<u64>;
    /// Damage and bitrate telemetry, emitted on change only.
    fn stats(&self) -> (u64, Option<u32>) {
        (0, None)
    }
}

/// Opens one real decoder in the host process, from a reachable path or URL.
pub trait DecoderResident {
    /// No reader handles or lossy path conversion cross this boundary.
    fn open(&self, location: MediaLocationWire) -> Result<Box<dyn ResidentDecoder>>;
}

/// Explicit unavailable resident used by the framework's placeholder binary.
pub struct UnavailableResident;

impl DecoderResident for UnavailableResident {
    fn open(&self, _location: MediaLocationWire) -> Result<Box<dyn ResidentDecoder>> {
        Err(MediaHostError::NotImplemented(
            "no decoder resident linked; select a resident helper binary".into(),
        ))
    }
}

struct ActiveRing<R: MappedRegion>(RingProducer<R>);
impl<R: MappedRegion> Drop for ActiveRing<R> {
    fn drop(&mut self) {
        self.0.close();
    }
}

enum Transition {
    Seek(u64),
    Format,
    Eos,
    Stop,
}

fn command(message: Message) -> Result<Transition> {
    match message {
        Message::DecoderSeek { to_ms } => Ok(Transition::Seek(to_ms)),
        Message::Shutdown => Ok(Transition::Stop),
        message => Err(MediaHostError::InvalidMessage(format!(
            "unexpected decoder command: {message:?}"
        ))),
    }
}

fn wait_command(peer: &ControlPeer) -> Result<Message> {
    peer.recv_blocking()
}

fn pump<R: MappedRegion>(
    peer: &ControlPeer,
    decoder: &mut dyn ResidentDecoder,
    ring: &mut RingProducer<R>,
    spec: PcmSpecWire,
    last_stats: &mut (u64, Option<u32>),
) -> Result<Transition> {
    let channels = usize::from(spec.channels);
    let mut block = vec![0.0; BLOCK_FRAMES * channels];
    loop {
        if let Some(message) = peer.try_recv()? {
            return command(message);
        }
        let frames = match decoder.read(&mut block) {
            Ok(0) => return Ok(Transition::Eos),
            Ok(frames) if frames <= BLOCK_FRAMES => frames,
            Ok(_) => {
                return Err(MediaHostError::OperationFailed(
                    "resident over-reported its read buffer".into(),
                ));
            }
            Err(MediaHostError::FormatChanged) => return Ok(Transition::Format),
            Err(error) => return Err(error),
        };
        if decoder.info().spec != spec {
            return Err(MediaHostError::OperationFailed(
                "resident changed format without FormatChanged".into(),
            ));
        }
        let stats = decoder.stats();
        if stats != *last_stats {
            peer.send(Message::DecoderStats {
                skipped_packets: stats.0,
                bitrate_kbps: stats.1,
            })?;
            *last_stats = stats;
        }
        let mut offset = 0;
        while offset < frames {
            if let Some(message) = peer.try_recv()? {
                return command(message);
            }
            // Infinite, lossless wait, sliced into `RING_POLL` steps so that
            // space the player has already freed is picked up from the
            // counters even if the `Space` signal was skipped (see
            // `crate::pcm::RING_POLL`). The independent control monitor owns
            // a close-only view and wakes this worker on
            // death/seek/shutdown.
            match write_frames(ring, &block[offset * channels..frames * channels], None) {
                Ok(written) => offset += written,
                Err(RingError::Closed) => {
                    peer.check()?;
                    return command(wait_command(peer)?);
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

/// Serve one decoder until shutdown. Native decoder panics/crashes remain in
/// this helper process; errors become DecoderError, never player-process exit.
pub fn serve_decoder<F: PcmRings>(
    peer: &ControlPeer,
    rings: &F,
    resident: &dyn DecoderResident,
) -> Result<()> {
    let result = serve_inner(peer, rings, resident);
    if let Err(error) = &result {
        let _ = peer.send(Message::DecoderError {
            message: error.to_string(),
        });
        // Give the writer a bounded chance to publish the terminal diagnostic.
        let _ = peer.flush(CONTROL_WAIT);
    }
    result
}

fn serve_inner<F: PcmRings>(
    peer: &ControlPeer,
    rings: &F,
    resident: &dyn DecoderResident,
) -> Result<()> {
    let (location, capacity) = match wait_command(peer)? {
        Message::DecoderOpen {
            location,
            prefetch_frames,
        } if prefetch_frames > 0 => (location, prefetch_frames),
        Message::Shutdown => return Ok(()),
        message => {
            return Err(MediaHostError::InvalidMessage(format!(
                "expected DecoderOpen: {message:?}"
            )));
        }
    };
    let mut decoder = resident.open(location)?;
    let info = decoder.info();
    validate_spec(info.spec)?;
    peer.send(Message::DecoderOpened {
        spec: info.spec,
        duration_ms: info.duration_ms,
        seekable: info.seekable,
        metadata: info.metadata,
    })?;
    let mut spec = info.spec;
    let mut reached = None;
    let mut previous_generation = None;
    let mut last_stats = (0, None);
    loop {
        let offer = rings.create(spec, capacity)?;
        let generation = offer.producer.generation();
        if previous_generation == Some(generation) {
            return Err(MediaHostError::OperationFailed(
                "ring factory reused a generation".into(),
            ));
        }
        previous_generation = Some(generation);
        let mut ring = ActiveRing(offer.producer);
        peer.set_abort(offer.abort)?;
        peer.send(offer.message)?;
        if let Some(reached_ms) = reached.take() {
            peer.send(Message::DecoderSeeked { reached_ms })?;
        }
        let transition = match wait_command(peer)? {
            Message::RingReady {
                kind: RingKind::Pcm,
                generation: ready,
            } if ready == generation => {
                pump(peer, decoder.as_mut(), &mut ring.0, spec, &mut last_stats)?
            }
            Message::RingRefused { reason, .. } => {
                return Err(MediaHostError::OperationFailed(format!(
                    "player refused PCM ring: {reason:?}"
                )));
            }
            message => command(message)?,
        };
        // Close before replacement, including every early error via Drop.
        // The client drains the old generation before accepting a new format.
        ring.0.close();
        let transition = if matches!(transition, Transition::Eos) {
            // Eos is a control-plane terminal marker, not a frame. The player
            // exposes Ok(0) only after draining this now-closed ring.
            peer.send(Message::DecoderEos)?;
            command(wait_command(peer)?)?
        } else {
            transition
        };
        match transition {
            Transition::Seek(to_ms) => {
                reached = Some(decoder.seek(to_ms)?);
                let next = decoder.info().spec;
                validate_spec(next)?;
                if next != spec {
                    peer.send(Message::DecoderFormat { spec: next })?;
                    spec = next;
                }
            }
            Transition::Format => {
                let next = decoder.info().spec;
                validate_spec(next)?;
                if next == spec {
                    return Err(MediaHostError::OperationFailed(
                        "FormatChanged without a new format".into(),
                    ));
                }
                spec = next;
                peer.send(Message::DecoderFormat { spec })?;
            }
            Transition::Stop => {
                peer.flush(CONTROL_WAIT)?;
                return Ok(());
            }
            Transition::Eos => unreachable!(),
        }
    }
}

/// Player-side worker client. No codec or GPL core dependency is needed here.
pub struct DecoderSession<F: PcmRings> {
    peer: ControlPeer,
    rings: F,
    input: RingConsumer<F::Region>,
    info: DecoderInfo,
    stats: (u64, Option<u32>),
    pending: VecDeque<Message>,
    eos: bool,
    generation: u64,
    pending_format: Option<PcmSpecWire>,
}

impl<F: PcmRings> DecoderSession<F> {
    /// Open and complete the initial format/ring/wake handshake.
    pub fn open(
        peer: ControlPeer,
        rings: F,
        location: MediaLocationWire,
        prefetch_frames: u32,
    ) -> Result<Self> {
        if prefetch_frames == 0 {
            return Err(MediaHostError::InvalidMessage(
                "zero prefetch frames".into(),
            ));
        }
        peer.send(Message::DecoderOpen {
            location,
            prefetch_frames,
        })?;
        let info = match peer.recv(CONTROL_WAIT)? {
            Message::DecoderOpened {
                spec,
                duration_ms,
                seekable,
                metadata,
            } => {
                validate_spec(spec)?;
                DecoderInfo {
                    spec,
                    duration_ms,
                    seekable,
                    metadata,
                }
            }
            message => {
                return Err(MediaHostError::InvalidMessage(format!(
                    "expected DecoderOpened: {message:?}"
                )));
            }
        };
        let offer = peer.recv(CONTROL_WAIT)?;
        let input = Self::attach(&peer, &rings, &offer, info.spec)?;
        let generation = input.generation();
        Ok(Self {
            peer,
            rings,
            input,
            info,
            stats: (0, None),
            pending: VecDeque::new(),
            eos: false,
            generation,
            pending_format: None,
        })
    }

    fn attach(
        peer: &ControlPeer,
        rings: &F,
        offer: &Message,
        spec: PcmSpecWire,
    ) -> Result<RingConsumer<F::Region>> {
        match rings.open(offer, spec) {
            Ok(input) => {
                let generation = input.consumer.generation();
                peer.set_abort(input.abort)?;
                peer.send(Message::RingReady {
                    kind: RingKind::Pcm,
                    generation,
                })?;
                Ok(input.consumer)
            }
            Err(error) => {
                if let Some(expect) = offer.ring_expect() {
                    let reason = match &error {
                        MediaHostError::Ring(error) => {
                            error.reason().unwrap_or(RingRefuseReason::InvalidLayout)
                        }
                        _ => RingRefuseReason::InvalidLayout,
                    };
                    let _ = peer.send(Message::ring_refused(
                        expect.kind,
                        expect.generation,
                        reason,
                    ));
                    let _ = peer.flush(CONTROL_WAIT);
                }
                Err(error)
            }
        }
    }

    /// Current properties; updated before returning FormatChanged.
    pub fn info(&self) -> &DecoderInfo {
        &self.info
    }
    /// Last damage and bitrate telemetry.
    pub fn stats(&self) -> (u64, Option<u32>) {
        self.stats
    }
    /// Transport drop counter: must stay zero for a valid resident.
    pub fn frames_dropped(&self) -> u64 {
        self.input.frames_dropped()
    }

    /// PCM ring counters right now, for stall diagnostics (see
    /// [`RingCounters::available`]): a read that returned
    /// [`MediaHostError::TimedOut`] while `available` is nonzero waited on
    /// data the ring already held, which is a skipped wake and not a slow
    /// resident.
    pub fn ring_counters(&self) -> RingCounters {
        RingCounters {
            published: self.input.frames_published(),
            read: self.input.frames_read(),
            closed: self.input.is_closed(),
        }
    }

    fn remember(&mut self, message: Message) -> Result<()> {
        match message {
            Message::DecoderStats {
                skipped_packets,
                bitrate_kbps,
            } => self.stats = (skipped_packets, bitrate_kbps),
            message => {
                if self.pending.len() >= 8 {
                    self.peer.fail("too many pending decoder transitions");
                    self.peer.check()?;
                }
                self.pending.push_back(message);
            }
        }
        Ok(())
    }

    fn next(&mut self, deadline: Option<Instant>) -> Result<Message> {
        if let Some(message) = self.pending.pop_front() {
            return Ok(message);
        }
        match remaining(deadline)? {
            Some(timeout) => self.peer.recv(timeout),
            None => self.peer.recv_blocking(),
        }
    }

    fn replace(&mut self, offer: Message, spec: PcmSpecWire) -> Result<()> {
        if offer
            .ring_expect()
            .is_some_and(|expect| expect.generation == self.generation)
        {
            return Err(MediaHostError::InvalidMessage(
                "replacement reused the old ring generation".into(),
            ));
        }
        let input = Self::attach(&self.peer, &self.rings, &offer, spec)?;
        self.generation = input.generation();
        self.input = input;
        Ok(())
    }

    /// Pull whole frames on a decode worker. `None` waits until data or control
    /// failure; an explicit timeout returns TimedOut, **never false EOS**.
    /// Empty/misaligned buffers are errors. EOS requires both a drained ring
    /// and DecoderEos. A format transition returns FormatChanged after reattach.
    pub fn read(&mut self, out: &mut [f32], timeout: Option<Duration>) -> Result<usize> {
        self.peer.check()?;
        if out.is_empty()
            || !out
                .len()
                .is_multiple_of(usize::from(self.info.spec.channels))
        {
            return Err(MediaHostError::InvalidMessage(
                "empty or non-frame-aligned read".into(),
            ));
        }
        if self.eos {
            return Ok(0);
        }
        let deadline = timeout.map(|duration| Instant::now() + duration);
        while let Some(message) = self.peer.try_recv()? {
            self.remember(message)?;
        }
        match read_frames(&mut self.input, out, deadline) {
            Ok(0) => Err(MediaHostError::TimedOut),
            Ok(frames) => {
                self.peer.check()?;
                Ok(frames)
            }
            Err(RingError::Closed) => {
                self.peer.check()?;
                loop {
                    let message = self.next(deadline)?;
                    if let Some(spec) = self.pending_format {
                        match message {
                            message @ Message::DecoderStats { .. } => {
                                self.remember(message)?;
                                continue;
                            }
                            offer => self.replace(offer, spec)?,
                        }
                        self.pending_format = None;
                        self.info.spec = spec;
                        return Err(MediaHostError::FormatChanged);
                    }
                    match message {
                        Message::DecoderEos => {
                            self.eos = true;
                            return Ok(0);
                        }
                        Message::DecoderFormat { spec } => {
                            validate_spec(spec)?;
                            self.pending_format = Some(spec);
                        }
                        message @ Message::DecoderStats { .. } => self.remember(message)?,
                        message => {
                            return Err(MediaHostError::InvalidMessage(format!(
                                "unexpected closed-ring response: {message:?}"
                            )));
                        }
                    }
                }
            }
            Err(error) => {
                self.peer.fail(&error.to_string());
                Err(error.into())
            }
        }
    }

    /// Seek, discard old prefetched PCM, and attach only the offer immediately
    /// preceding DecoderSeeked (earlier in-flight format/EOS messages are stale).
    pub fn seek(&mut self, to_ms: u64, timeout: Duration) -> Result<u64> {
        if !self.info.seekable {
            return Err(MediaHostError::OperationFailed(
                "decoder is not seekable".into(),
            ));
        }
        let result = self.seek_inner(to_ms, timeout);
        if let Err(error) = &result {
            self.peer.fail(&format!("seek failed: {error}"));
        }
        result
    }

    fn seek_inner(&mut self, to_ms: u64, timeout: Duration) -> Result<u64> {
        self.peer.send(Message::DecoderSeek { to_ms })?;
        self.input.close();
        let deadline = Some(Instant::now() + timeout);
        let mut offer = None;
        let mut spec = self.pending_format.take().unwrap_or(self.info.spec);
        loop {
            match self.next(deadline)? {
                message @ Message::RingOffer { .. } => offer = Some(message),
                Message::DecoderFormat { spec: next } => {
                    validate_spec(next)?;
                    spec = next;
                }
                Message::DecoderSeeked { reached_ms } => {
                    self.replace(
                        offer.ok_or_else(|| {
                            MediaHostError::InvalidMessage(
                                "seek acknowledgement without a ring".into(),
                            )
                        })?,
                        spec,
                    )?;
                    self.info.spec = spec;
                    self.eos = false;
                    return Ok(reached_ms);
                }
                Message::DecoderEos => {}
                message @ Message::DecoderStats { .. } => self.remember(message)?,
                message => {
                    return Err(MediaHostError::InvalidMessage(format!(
                        "unexpected seek response: {message:?}"
                    )));
                }
            }
        }
    }
}

impl<F: PcmRings> Drop for DecoderSession<F> {
    fn drop(&mut self) {
        self.input.close();
        let _ = self.peer.send(Message::Shutdown);
    }
}

fn remaining(deadline: Option<Instant>) -> Result<Option<Duration>> {
    deadline
        .map(|deadline| {
            deadline
                .checked_duration_since(Instant::now())
                .ok_or(MediaHostError::TimedOut)
        })
        .transpose()
}

/// Convert a local path without calling the proto helper's lossy conversion.
pub fn file_location(path: &std::path::Path) -> Result<MediaLocationWire> {
    path.to_str()
        .map(|path| MediaLocationWire::File(path.into()))
        .ok_or_else(|| {
            MediaHostError::InvalidMessage("non-UTF-8 paths require in-process decoding".into())
        })
}

/// Owned child plus worker session. Drop always kills and reaps the helper.
#[cfg(windows)]
pub struct DecoderProcess {
    session: Option<DecoderSession<crate::pcm::NamedPcmRings>>,
    child: std::process::Child,
}

#[cfg(windows)]
impl DecoderProcess {
    /// Spawn a separately licensed resident binary without a console/window.
    /// A missing binary is an ordinary unavailable-feature error.
    pub fn spawn(
        executable: &std::path::Path,
        location: MediaLocationWire,
        prefetch_frames: u32,
        liveness: crate::control::Liveness,
    ) -> Result<Self> {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        let mut child = Command::new(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|error| {
                MediaHostError::OperationFailed(format!("decoder helper unavailable: {error}"))
            })?;
        let result = (|| {
            let peer = ControlPeer::start(
                child.stdout.take().unwrap(),
                child.stdin.take().unwrap(),
                liveness,
                false,
            )?;
            DecoderSession::open(peer, crate::pcm::NamedPcmRings, location, prefetch_frames)
        })();
        match result {
            Ok(session) => Ok(Self {
                session: Some(session),
                child,
            }),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }

    /// Decode-worker session; never use from the device callback.
    pub fn session(&mut self) -> &mut DecoderSession<crate::pcm::NamedPcmRings> {
        self.session.as_mut().unwrap()
    }

    /// OS child id (for diagnostics and VM crash-isolation tests).
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Terminate and reap a stalled/crashed helper; the next read fails loudly.
    pub fn terminate(&mut self) -> Result<()> {
        self.child
            .kill()
            .map_err(|error| MediaHostError::OperationFailed(error.to_string()))?;
        self.child
            .wait()
            .map_err(|error| MediaHostError::OperationFailed(error.to_string()))?;
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for DecoderProcess {
    fn drop(&mut self) {
        self.session.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::Liveness;
    use rustamp_ipc::Transport;
    use std::io::{Cursor, Read, Write};
    use std::sync::mpsc;
    use std::time::Duration;

    struct ByteReader {
        rx: mpsc::Receiver<Vec<u8>>,
        bytes: Cursor<Vec<u8>>,
    }
    struct ByteWriter(mpsc::Sender<Vec<u8>>);

    impl Read for ByteReader {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if out.is_empty() {
                return Ok(0);
            }
            loop {
                let count = self.bytes.read(out)?;
                if count > 0 {
                    return Ok(count);
                }
                match self.rx.recv() {
                    Ok(bytes) => self.bytes = Cursor::new(bytes),
                    Err(_) => return Ok(0),
                }
            }
        }
    }

    impl Write for ByteWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .send(bytes.to_vec())
                .map_err(|_| std::io::ErrorKind::BrokenPipe)?;
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn pipe() -> (ByteReader, ByteWriter) {
        let (tx, rx) = mpsc::channel();
        (
            ByteReader {
                rx,
                bytes: Cursor::new(Vec::new()),
            },
            ByteWriter(tx),
        )
    }

    #[test]
    fn decoder_error_fails_control_peer_sticky_as_disconnected() {
        let (client_r, server_w) = pipe();
        let (server_r, client_w) = pipe();
        let liveness = Liveness {
            interval: Duration::from_millis(50),
            timeout: Duration::from_millis(500),
        };
        let client_peer = ControlPeer::start(client_r, client_w, liveness, false).unwrap();
        let mut server_transport = rustamp_ipc::StreamTransport::new(server_r, server_w);

        // Server receives client's initial Hello.
        let msg = server_transport.recv().unwrap();
        assert!(matches!(msg, Message::Hello { .. }));

        // Server responds with Hello.
        server_transport.send(&Message::hello()).unwrap();

        // Server sends a terminal DecoderError.
        server_transport
            .send(&Message::DecoderError {
                message: "refused corrupt stream".into(),
            })
            .unwrap();

        // The client peer's monitor intercepts DecoderError, calls fail(), and breaks.
        // It never places DecoderError in incoming_tx; instead peer.recv returns Disconnected.
        let err = client_peer.recv(Duration::from_millis(500)).unwrap_err();
        match err {
            MediaHostError::Disconnected(msg) => {
                assert!(
                    msg.contains("resident decoder failed: refused corrupt stream"),
                    "unexpected disconnect reason: {msg}"
                );
            }
            other => panic!("expected Disconnected, got {other:?}"),
        }

        // Subsequent check() also sticky-fails with Disconnected.
        assert!(client_peer.check().is_err());
    }

    #[test]
    fn decoder_session_open_fails_sticky_as_disconnected_when_server_reports_decoder_error() {
        let (client_r, server_w) = pipe();
        let (server_r, client_w) = pipe();
        let liveness = Liveness {
            interval: Duration::from_millis(50),
            timeout: Duration::from_millis(500),
        };
        let client_peer = ControlPeer::start(client_r, client_w, liveness, false).unwrap();
        let mut server_transport = rustamp_ipc::StreamTransport::new(server_r, server_w);

        let msg = server_transport.recv().unwrap();
        assert!(matches!(msg, Message::Hello { .. }));
        server_transport.send(&Message::hello()).unwrap();

        let server_handle = std::thread::spawn(move || {
            // The client's liveness monitor answers the handshake with an
            // immediate Ping that races the session's DecoderOpen; both orders
            // are legitimate wire traffic (production residents answer Pong
            // through their own monitor and never surface either frame). Skip
            // the liveness chatter and pin the open itself; anything else is
            // still a loud failure so a genuinely wrong first message cannot
            // slip through.
            loop {
                match server_transport.recv().unwrap() {
                    Message::DecoderOpen { .. } => break,
                    Message::Ping(_) | Message::Pong(_) => continue,
                    other => panic!("expected DecoderOpen, got {other:?}"),
                }
            }
            server_transport
                .send(&Message::DecoderError {
                    message: "refused corrupt stream".into(),
                })
                .unwrap();
            let _ = server_transport.recv();
        });

        let rings = crate::pcm::NamedPcmRings;
        let location = MediaLocationWire::File("bad.bin".into());
        let res = DecoderSession::open(client_peer, rings, location, 1024);
        let err = res.err().expect("open must fail");
        match err {
            MediaHostError::Disconnected(msg) => {
                assert!(
                    msg.contains("resident decoder failed: refused corrupt stream"),
                    "unexpected disconnect reason: {msg}"
                );
            }
            other => panic!("expected Disconnected, got {other:?}"),
        }
        let _ = server_handle.join();
    }
}
