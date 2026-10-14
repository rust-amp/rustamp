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
    /// The peer closed the connection.
    #[error("connection closed")]
    Closed,
}

/// A bidirectional, message-oriented connection.
pub trait Transport: Send {
    /// Send one message.
    fn send(&mut self, msg: &Message) -> Result<(), IpcError>;
    /// Block until the next message arrives; `Err(Closed)` at end of stream.
    fn recv(&mut self) -> Result<Message, IpcError>;
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
    pub fn try_recv(&mut self) -> Result<Option<Message>, IpcError> {
        match self.rx.try_recv() {
            Ok(b) => decode(&b).map(Some),
            Err(crossbeam_channel::TryRecvError::Empty) => Ok(None),
            Err(crossbeam_channel::TryRecvError::Disconnected) => Err(IpcError::Closed),
        }
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
}

/// Transport over a byte-stream reader/writer pair using length-prefixed framing.
///
/// For a duplex pipe or socket, pass two handles to the same object (e.g.
/// `try_clone`). Use separate threads for reading and writing if the
/// protocol needs concurrent send/recv.
#[derive(Debug)]
pub struct StreamTransport<R, W> {
    reader: R,
    writer: W,
}

impl<R: Read + Send, W: Write + Send> StreamTransport<R, W> {
    /// Wrap a reader and writer.
    pub fn new(reader: R, writer: W) -> Self {
        StreamTransport { reader, writer }
    }
}

impl<R: Read + Send, W: Write + Send> Transport for StreamTransport<R, W> {
    fn send(&mut self, msg: &Message) -> Result<(), IpcError> {
        write_frame(&mut self.writer, msg)
    }
    fn recv(&mut self) -> Result<Message, IpcError> {
        read_frame(&mut self.reader)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::PathBuf;

    use rustamp_core::command::{Command, SelectOp};
    use rustamp_core::playlist::{EntryId, MediaLocation, SortKey};
    use rustamp_core::state::WindowId;
    use rustamp_core::vis::VisFrame;
    use rustamp_plugin_abi::PluginKind;

    use super::*;
    use crate::message::VisFrameWire;

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
