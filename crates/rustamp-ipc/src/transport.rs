//! The [`Transport`] trait and two implementations.

use std::io::{Read, Write};

use crossbeam_channel::{Receiver, Sender};

use crate::framing::{decode, encode, read_frame, write_frame};
use crate::message::Message;

/// IPC errors.
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    /// I/O error on the underlying stream.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    /// Encoding/decoding failure.
    #[error("codec error: {0}")]
    Codec(#[from] postcard::Error),
    /// Frame exceeds `MAX_FRAME_LEN`.
    #[error("frame too large: {0} bytes")]
    FrameTooLarge(u64),
    /// The stream ended in the middle of a frame.
    ///
    /// The length prefix or the payload was cut short, so the frame is
    /// incomplete and was **not** decoded. This is not an orderly end: the
    /// stream is inside the broken frame, and the bytes that follow belong
    /// to it rather than to a new one.
    #[error("truncated frame: the stream ended in the middle of a frame")]
    Truncated,
    /// The peer closed the connection.
    #[error("connection closed")]
    Closed,
    /// The stream cannot carry frames any more.
    ///
    /// A frame was started but neither finished nor refused, so the position
    /// in the stream is unknown. A transport refuses to read or write
    /// further frames instead of handing the peer bytes it would misread.
    #[error("the stream is out of sync: a frame was not completed")]
    Desynchronized,
    /// No message is available right now, and this transport cannot say
    /// whether one will arrive: only in-memory transports support a real
    /// non-blocking read. A [`StreamTransport`] over blocking stdio always
    /// reports this; its read half belongs to a reader thread that blocks in
    /// [`Transport::recv`] instead (see [`StreamTransport`]).
    ///
    /// The same variant reports a non-blocking send whose bounded queue is
    /// full (see `rustamp-app/src/supervise.rs`): the peer stopped reading,
    /// and the sender must detach rather than stall.
    #[error("no message available")]
    WouldBlock,
}

/// A bidirectional, message-oriented connection.
pub trait Transport: Send {
    /// Send one message.
    fn send(&mut self, msg: &Message) -> Result<(), IpcError>;
    /// Block until the next message arrives; `Err(Closed)` at end of stream.
    ///
    /// A stream that ends inside a frame is `Err(Truncated)`, and a stream
    /// that cannot carry frames any more is `Err(Desynchronized)`; neither
    /// is a message and neither is an orderly end.
    fn recv(&mut self) -> Result<Message, IpcError>;
    /// Non-blocking receive: `Ok(None)` when nothing is queued right now.
    ///
    /// The default reports [`IpcError::WouldBlock`]: byte-stream transports
    /// over blocking handles cannot peek without OS-specific support, so
    /// polling them would either block (forbidden on the app thread) or lie.
    /// Owners of such a transport read it on a reader thread and poll the
    /// thread's handover instead.
    fn try_recv(&mut self) -> Result<Option<Message>, IpcError> {
        Err(IpcError::WouldBlock)
    }
}

/// In-memory transport pair for tests. Messages pass through the real
/// codec, so it exercises encode/decode too.
#[derive(Debug)]
pub struct MemoryTransport {
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
}

impl MemoryTransport {
    /// Create two connected endpoints.
    pub fn pair() -> (MemoryTransport, MemoryTransport) {
        let (a_tx, b_rx) = crossbeam_channel::unbounded();
        let (b_tx, a_rx) = crossbeam_channel::unbounded();
        (
            MemoryTransport { tx: a_tx, rx: a_rx },
            MemoryTransport { tx: b_tx, rx: b_rx },
        )
    }

    /// Non-blocking receive; `Ok(None)` if nothing is queued.
    ///
    /// Inherent so existing callers keep compiling; it is the trait's
    /// [`Transport::try_recv`] for this transport.
    pub fn try_recv(&mut self) -> Result<Option<Message>, IpcError> {
        Transport::try_recv(self)
    }
}

impl Transport for MemoryTransport {
    fn send(&mut self, msg: &Message) -> Result<(), IpcError> {
        self.tx.send(encode(msg)?).map_err(|_| IpcError::Closed)
    }
    fn recv(&mut self) -> Result<Message, IpcError> {
        let b = self.rx.recv().map_err(|_| IpcError::Closed)?;
        decode(&b)
    }
    fn try_recv(&mut self) -> Result<Option<Message>, IpcError> {
        match self.rx.try_recv() {
            Ok(b) => decode(&b).map(Some),
            Err(crossbeam_channel::TryRecvError::Empty) => Ok(None),
            Err(crossbeam_channel::TryRecvError::Disconnected) => Err(IpcError::Closed),
        }
    }
}

/// Transport over a byte-stream reader/writer pair using length-prefixed framing.
///
/// For a duplex pipe or socket, pass two handles to the same object (e.g.
/// `try_clone`). Use separate threads for reading and writing if the
/// protocol needs concurrent send/recv.
///
/// This transport keeps the [`Transport::try_recv`] default
/// ([`IpcError::WouldBlock`]): a blocking handle cannot be polled, so the
/// read half belongs to a reader thread that blocks in
/// [`Transport::recv`], while the app thread keeps the write half and polls
/// the reader's handover (see `rustamp-app/src/supervise.rs`).
///
/// # A broken frame ends the stream
///
/// A frame that was not completed leaves the byte stream at an unknown
/// position: the bytes that follow belong to the broken frame, not to a new
/// one. Reading or writing another frame would hand the peer bytes it would
/// misread - the tail of a broken frame read as the next length prefix. The
/// transport therefore refuses further frames with
/// [`IpcError::Desynchronized`] once a frame was cut short, refused before
/// its payload was read, or left half written.
#[derive(Debug)]
pub struct StreamTransport<R, W> {
    reader: R,
    writer: W,
    /// The read half lost its place in the stream.
    read_broken: bool,
    /// The write half may have left a half frame in the stream.
    write_broken: bool,
}

impl<R: Read + Send, W: Write + Send> StreamTransport<R, W> {
    /// Wrap a reader and writer.
    pub fn new(reader: R, writer: W) -> Self {
        StreamTransport {
            reader,
            writer,
            read_broken: false,
            write_broken: false,
        }
    }
}

impl<R: Read + Send, W: Write + Send> Transport for StreamTransport<R, W> {
    fn send(&mut self, msg: &Message) -> Result<(), IpcError> {
        if self.write_broken {
            return Err(IpcError::Desynchronized);
        }
        match write_frame(&mut self.writer, msg) {
            Ok(()) => Ok(()),
            // Nothing went out, so the stream is still framed.
            Err(error @ (IpcError::FrameTooLarge(_) | IpcError::Codec(_))) => Err(error),
            // The frame may be half written: the peer would read the rest of
            // it as the next frame, so no further frame goes out.
            Err(error) => {
                self.write_broken = true;
                Err(error)
            }
        }
    }
    fn recv(&mut self) -> Result<Message, IpcError> {
        if self.read_broken {
            return Err(IpcError::Desynchronized);
        }
        match read_frame(&mut self.reader) {
            Ok(message) => Ok(message),
            // The frame was refused before its payload was read, or it was
            // cut short: the stream is inside the broken frame.
            Err(error @ (IpcError::Truncated | IpcError::FrameTooLarge(_))) => {
                self.read_broken = true;
                Err(error)
            }
            // A read that failed leaves the stream where the failure hit it,
            // which is inside the frame.
            Err(error @ IpcError::Io(_)) => {
                self.read_broken = true;
                Err(error)
            }
            // A codec error means the frame was read whole: the stream is
            // still framed and the next frame is the next one.
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::PathBuf;

    use rustamp_plugin_abi::PluginKind;
    use rustamp_proto::command::{Command, SelectOp};
    use rustamp_proto::playlist::{EntryId, MediaLocation, SortKey};
    use rustamp_proto::state::WindowId;
    use rustamp_proto::vis::VisFrame;

    use super::*;
    use crate::message::VisFrameWire;
    use crate::shm::{RingDirection, RingKind, RingRefuseReason};

    fn samples() -> Vec<Message> {
        let mut vis = VisFrame {
            timestamp_us: 42,
            ..VisFrame::default()
        };
        vis.waveform[1][575] = 0.25;
        vec![
            Message::hello(),
            Message::Ping(7),
            Message::Pong(7),
            Message::LoadPlugin {
                path: "C:\\plugins\\vis_x.dll".into(),
                kind: PluginKind::Visualization,
            },
            Message::PluginLoaded {
                plugin_id: 1,
                name: "X".into(),
                kind: PluginKind::Dsp,
            },
            Message::PluginError {
                path: "p".into(),
                message: "boom".into(),
            },
            Message::PlayerStateUpdate(Box::default()),
            Message::CommandFromPlugin(Command::Seek { to_ms: 1234 }),
            Message::CommandFromPlugin(Command::AddItems {
                items: vec![
                    MediaLocation::File(PathBuf::from("a.mp3")),
                    MediaLocation::Url("http://x/y".into()),
                ],
                at: Some(3),
                play: true,
            }),
            Message::CommandFromPlugin(Command::Select(SelectOp::Set(vec![
                EntryId(1),
                EntryId(9),
            ]))),
            Message::CommandFromPlugin(Command::Sort(SortKey::Title)),
            Message::CommandFromPlugin(Command::ToggleWindow(WindowId::Gen(2))),
            Message::CommandFromPlugin(Command::SetBand {
                band: 3,
                gain_db: -2.5,
            }),
            Message::Vis(Box::new(VisFrameWire::from(&vis))),
            Message::RingOffer {
                name: "rustamp-pcm-1-0000000000000002".into(),
                kind: RingKind::Pcm,
                frame_size: 8,
                capacity: 4096,
                size: 0xC0 + 4096 * 8,
                generation: 2,
                direction: RingDirection::HostToMain,
            },
            Message::RingReady {
                kind: RingKind::Vis,
                generation: 7,
            },
            Message::ring_refused(RingKind::Vis, 7, RingRefuseReason::Stale),
            Message::Shutdown,
            Message::WinampIpc {
                msg_id: 0x400 + 104,
                wparam: u64::MAX,
                lparam: 1,
            },
        ]
    }

    #[test]
    fn codec_roundtrip() {
        for m in samples() {
            assert_eq!(decode(&encode(&m).unwrap()).unwrap(), m);
        }
    }

    #[test]
    fn memory_transport_roundtrip() {
        let (mut a, mut b) = MemoryTransport::pair();
        for m in samples() {
            a.send(&m).unwrap();
            assert_eq!(b.recv().unwrap(), m);
            b.send(&m).unwrap();
            assert_eq!(a.recv().unwrap(), m);
        }
        assert!(b.try_recv().unwrap().is_none());
        drop(a);
        assert!(matches!(b.recv(), Err(IpcError::Closed)));
    }

    #[test]
    fn framing_roundtrip_and_errors() {
        let mut buf = Vec::new();
        for m in samples() {
            write_frame(&mut buf, &m).unwrap();
        }
        let mut cur = Cursor::new(buf);
        for m in samples() {
            assert_eq!(read_frame(&mut cur).unwrap(), m);
        }
        assert!(matches!(read_frame(&mut cur), Err(IpcError::Closed)));

        // oversized length prefix is rejected without allocating
        let mut bad = Cursor::new(u32::MAX.to_le_bytes().to_vec());
        assert!(matches!(
            read_frame(&mut bad),
            Err(IpcError::FrameTooLarge(_))
        ));
    }

    #[test]
    fn stream_transport_over_buffer() {
        let mut out = Vec::new();
        StreamTransport::new(std::io::empty(), &mut out)
            .send(&Message::Ping(1))
            .unwrap();
        let mut t = StreamTransport::new(Cursor::new(out), std::io::sink());
        assert_eq!(t.recv().unwrap(), Message::Ping(1));
    }

    #[test]
    fn vis_wire_roundtrip() {
        let mut f = VisFrame::default();
        f.spectrum[1][10] = 0.5;
        f.timestamp_us = 99;
        let w = VisFrameWire::from(&f);
        assert_eq!(w.to_frame().unwrap(), f);
        let mut bad = w.clone();
        bad.waveform.pop();
        assert!(bad.to_frame().is_none());
    }
}
