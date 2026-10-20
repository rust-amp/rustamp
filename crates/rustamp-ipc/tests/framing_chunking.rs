//! The framing against a byte stream that hands out its bytes in arbitrary
//! chunks.
//!
//! `wave296` covered the wire at the **value** level: a message that goes
//! through the codec comes back the same. This file covers the layer below
//! it - the byte stream - where a read may return fewer bytes than were
//! asked for, a write may accept fewer bytes than were offered, and a
//! stream may end in the middle of a frame. None of that may change what
//! the peer sees: a frame is all or nothing.
//!
//! The properties:
//!
//! 1. **A frame survives any chunking.** Read back through a stream that
//!    hands out 1, 3, 7, ... bytes per call, a frame is the message it was,
//!    and the reader stops exactly at the frame's end.
//! 2. **A frame fed one byte at a time is exactly one message.** The reader
//!    yields one message and then reports the clean end of the stream.
//! 3. **Two frames are two messages.** Two frames in one stream are read in
//!    order, with no bleed between them.
//! 4. **A short write is a whole frame.** A writer that accepts fewer bytes
//!    than offered still ends up with the whole frame; a writer that fails
//!    reports the failure and leaves a broken frame that is never read as a
//!    message.
//! 5. **A stream that ends mid-frame is a typed error.** Every cut of a
//!    frame is an error - never a shorter message - and a clean end stays
//!    distinguishable from a broken one.
//! 6. **An oversized length prefix is refused** before a buffer of that size
//!    exists (the allocation itself is measured in `framing_no_alloc.rs`).
//! 7. **No chunking panics.** The seeded sweep over chunk plans and cuts
//!    ends in a message or a typed error, never in a panic.
//!
//! The generator is this file's own SplitMix64: no new dependency, and the
//! crate keeps its dependency list and its license closure untouched. Every
//! case is reproducible from the seed the failure message prints.

use std::io::{self, Cursor, Read, Write};

use rustamp_ipc::{
    IpcError, MAX_FRAME_LEN, Message, StreamTransport, Transport, read_frame, write_frame,
};
use rustamp_plugin_abi::PluginKind;

/// The seed of the sweep: every case is reproducible from it.
const SEED: u64 = 0x5EED_2960_0000_0320;

/// A seeded generator: this file's own SplitMix64.
struct Rng(u64);

impl Rng {
    /// A generator for `seed`.
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    /// The next 64 bits.
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// A value in `0..bound`; `bound` must not be zero.
    fn below(&mut self, bound: u64) -> u64 {
        assert!(bound > 0, "a bound of zero has no value");
        self.next_u64() % bound
    }
    /// An arbitrary byte.
    fn u8(&mut self) -> u8 {
        self.next_u64() as u8
    }
}

/// A reader that hands out at most `chunk` bytes per call.
///
/// The bytes are the whole stream; the chunk size is the only thing that
/// changes between the runs.
struct ChunkReader {
    data: Vec<u8>,
    position: usize,
    chunk: usize,
}

impl ChunkReader {
    /// A reader over `data` that hands out at most `chunk` bytes per call.
    fn new(data: Vec<u8>, chunk: usize) -> Self {
        ChunkReader {
            data,
            position: 0,
            chunk,
        }
    }
    /// The bytes that were not read yet.
    fn remaining(&self) -> usize {
        self.data.len() - self.position
    }
}

impl Read for ChunkReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.position == self.data.len() {
            return Ok(0);
        }
        let count = self
            .chunk
            .min(buf.len())
            .min(self.data.len() - self.position);
        buf[..count].copy_from_slice(&self.data[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

/// A reader that hands out a random number of bytes per call.
struct RandomChunkReader<'a> {
    data: Vec<u8>,
    position: usize,
    /// The largest number of bytes one call may hand out.
    limit: usize,
    rng: &'a mut Rng,
}

impl<'a> RandomChunkReader<'a> {
    /// A reader over `data` that hands out 1..=`limit` bytes per call.
    fn new(data: Vec<u8>, limit: usize, rng: &'a mut Rng) -> Self {
        RandomChunkReader {
            data,
            position: 0,
            limit,
            rng,
        }
    }
    /// The bytes that were not read yet.
    fn remaining(&self) -> usize {
        self.data.len() - self.position
    }
}

impl Read for RandomChunkReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.position == self.data.len() {
            return Ok(0);
        }
        let wanted = self.rng.below(self.limit as u64 + 1) as usize + 1;
        let count = wanted.min(buf.len()).min(self.data.len() - self.position);
        buf[..count].copy_from_slice(&self.data[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

/// A reader that hands out `budget` bytes and then fails.
struct FailingReader {
    data: Vec<u8>,
    position: usize,
    budget: usize,
}

impl FailingReader {
    /// A reader over `data` that fails after `budget` bytes.
    fn new(data: Vec<u8>, budget: usize) -> Self {
        FailingReader {
            data,
            position: 0,
            budget,
        }
    }
}

impl Read for FailingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.position == self.data.len() {
            return Ok(0);
        }
        if self.budget == 0 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "the peer is gone",
            ));
        }
        let count = self
            .budget
            .min(buf.len())
            .min(self.data.len() - self.position);
        buf[..count].copy_from_slice(&self.data[self.position..self.position + count]);
        self.position += count;
        self.budget -= count;
        Ok(count)
    }
}

/// A reader that hands out `budget` bytes and then reports that it would
/// block.
struct WouldBlockReader {
    data: Vec<u8>,
    position: usize,
    budget: usize,
}

impl WouldBlockReader {
    /// A reader over `data` that blocks after `budget` bytes.
    fn new(data: Vec<u8>, budget: usize) -> Self {
        WouldBlockReader {
            data,
            position: 0,
            budget,
        }
    }
}

impl Read for WouldBlockReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.budget == 0 {
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "no data yet"));
        }
        let count = self
            .budget
            .min(buf.len())
            .min(self.data.len() - self.position);
        buf[..count].copy_from_slice(&self.data[self.position..self.position + count]);
        self.position += count;
        self.budget -= count;
        Ok(count)
    }
}

/// A writer that accepts at most `chunk` bytes per call.
struct ShortWriter {
    sink: Vec<u8>,
    chunk: usize,
    /// The number of `write` calls the writer saw.
    calls: usize,
}

impl ShortWriter {
    /// A writer that accepts at most `chunk` bytes per call.
    fn new(chunk: usize) -> Self {
        ShortWriter {
            sink: Vec::new(),
            chunk,
            calls: 0,
        }
    }
}

impl Write for ShortWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.calls += 1;
        let count = self.chunk.min(buf.len());
        self.sink.extend_from_slice(&buf[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A writer that accepts `budget` bytes and then fails.
struct FailingWriter {
    sink: Vec<u8>,
    budget: usize,
}

impl FailingWriter {
    /// A writer that fails after `budget` bytes.
    fn new(budget: usize) -> Self {
        FailingWriter {
            sink: Vec::new(),
            budget,
        }
    }
}

impl Write for FailingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.budget == 0 {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the peer is gone",
            ));
        }
        let count = self.budget.min(buf.len());
        self.budget -= count;
        self.sink.extend_from_slice(&buf[..count]);
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A message with random field values, for the sweep.
fn random_message(rng: &mut Rng) -> Message {
    match rng.below(4) {
        0 => Message::Ping(rng.next_u64()),
        1 => Message::Pong(rng.next_u64()),
        2 => Message::Hello {
            protocol_version: rng.next_u64() as u32,
            pid: rng.next_u64() as u32,
        },
        _ => Message::LoadPlugin {
            path: format!("C:\\plugins\\p{}.dll", rng.u8()),
            kind: PluginKind::Dsp,
        },
    }
}

/// The bytes of one frame.
fn frame_of(message: &Message) -> Vec<u8> {
    let mut frame = Vec::new();
    write_frame(&mut frame, message).expect("writing a frame");
    frame
}

#[test]
fn a_frame_survives_any_chunking() {
    let message = Message::Ping(0x0123_4567_89AB_CDEF);
    let frame = frame_of(&message);

    // Every fixed chunk size, from one byte at a time to the whole frame.
    for chunk in 1..=frame.len() {
        let mut reader = ChunkReader::new(frame.clone(), chunk);
        let decoded = read_frame(&mut reader)
            .unwrap_or_else(|error| panic!("chunk {chunk}: reading the frame failed: {error}"));
        assert_eq!(decoded, message, "chunk {chunk}: the frame changed");
        assert_eq!(
            reader.remaining(),
            0,
            "chunk {chunk}: the reader read past the frame"
        );
    }

    // A chunk size that changes from call to call.
    for seed in 0..64 {
        let mut rng = Rng::new(SEED.wrapping_add(seed));
        let mut reader = RandomChunkReader::new(frame.clone(), 5, &mut rng);
        let decoded =
            read_frame(&mut reader).unwrap_or_else(|error| panic!("seed {seed:#x}: {error}"));
        assert_eq!(decoded, message, "seed {seed:#x}: the frame changed");
        assert_eq!(
            reader.remaining(),
            0,
            "seed {seed:#x}: the reader read past the frame"
        );
    }
}

#[test]
fn a_frame_fed_one_byte_at_a_time_is_exactly_one_message() {
    let message = Message::Pong(42);
    let frame = frame_of(&message);

    let mut reader = ChunkReader::new(frame, 1);
    assert_eq!(
        read_frame(&mut reader).expect("the whole frame"),
        message,
        "a frame that arrives one byte at a time is the message it was"
    );
    // The stream is at its end: the next frame is a clean close, not a
    // second message and not a complaint about a broken frame.
    assert!(matches!(read_frame(&mut reader), Err(IpcError::Closed)));
}

#[test]
fn two_frames_in_one_stream_are_two_messages_in_order() {
    let first = Message::Ping(1);
    let second = Message::LoadPlugin {
        path: "C:\\plugins\\vis.dll".into(),
        kind: PluginKind::Visualization,
    };
    let mut stream = Vec::new();
    write_frame(&mut stream, &first).expect("writing the first frame");
    let first_end = stream.len();
    write_frame(&mut stream, &second).expect("writing the second frame");

    let mut rng = Rng::new(SEED);
    let mut reader = RandomChunkReader::new(stream.clone(), 3, &mut rng);
    assert_eq!(
        read_frame(&mut reader).expect("the first frame"),
        first,
        "the first frame must not read into the second"
    );
    assert_eq!(
        reader.position, first_end,
        "the first frame must stop at its own end"
    );
    assert_eq!(
        read_frame(&mut reader).expect("the second frame"),
        second,
        "the second frame must not read into the first"
    );
    assert_eq!(reader.remaining(), 0, "both frames must be read");
    assert!(matches!(read_frame(&mut reader), Err(IpcError::Closed)));
}

#[test]
fn a_short_write_still_writes_the_whole_frame() {
    let message = Message::Ping(7);
    let reference = frame_of(&message);

    for chunk in 1..=8 {
        let mut writer = ShortWriter::new(chunk);
        write_frame(&mut writer, &message)
            .unwrap_or_else(|error| panic!("chunk {chunk}: writing the frame failed: {error}"));
        assert_eq!(
            writer.sink, reference,
            "chunk {chunk}: the frame is not whole"
        );
    }
}

#[test]
fn a_frame_goes_out_as_one_write() {
    // The length prefix and the payload belong together: a stream that
    // takes the frame whole must not see a prefix without its payload.
    let mut writer = ShortWriter::new(usize::MAX);
    write_frame(&mut writer, &Message::Ping(1)).expect("writing a frame");
    assert_eq!(writer.calls, 1, "the frame must go out in one piece");
}

#[test]
fn a_writer_that_accepts_nothing_reports_the_failure() {
    let mut writer = ShortWriter::new(0);
    let error = write_frame(&mut writer, &Message::Ping(1))
        .expect_err("a writer that accepts nothing must fail");
    assert!(
        matches!(&error, IpcError::Io(io_error) if io_error.kind() == io::ErrorKind::WriteZero),
        "got {error:?}"
    );
    assert!(writer.sink.is_empty(), "nothing may have been written");
}

#[test]
fn a_failed_write_leaves_a_broken_frame_that_is_never_a_message() {
    let message = Message::Ping(7);
    let frame = frame_of(&message);

    for budget in 0..frame.len() {
        let mut writer = FailingWriter::new(budget);
        let error = write_frame(&mut writer, &message).expect_err("the writer fails");
        assert!(matches!(&error, IpcError::Io(_)), "got {error:?}");

        // What the peer sees: a broken frame, never a message.
        let mut reader = ChunkReader::new(writer.sink.clone(), 1);
        let outcome = read_frame(&mut reader);
        assert!(
            outcome.is_err(),
            "budget {budget}: the broken frame was read as a message"
        );
        if budget == 0 {
            // Nothing went out, so the peer sees a clean end.
            assert!(
                matches!(&outcome, Err(IpcError::Closed)),
                "budget {budget}: got {outcome:?}"
            );
        } else {
            assert!(
                matches!(&outcome, Err(IpcError::Truncated)),
                "budget {budget}: got {outcome:?}"
            );
        }
    }
}

#[test]
fn a_stream_that_ends_mid_frame_is_a_typed_error() {
    let message = Message::LoadPlugin {
        path: "C:\\plugins\\p.dll".into(),
        kind: PluginKind::Dsp,
    };
    let frame = frame_of(&message);

    for cut in 0..frame.len() {
        let mut reader = ChunkReader::new(frame[..cut].to_vec(), 1);
        let outcome = read_frame(&mut reader);
        assert!(
            outcome.is_err(),
            "cut {cut}: a frame cut to {cut} of {} bytes was read as a message",
            frame.len()
        );
        if cut == 0 {
            // The stream ended at a frame boundary: an orderly end.
            assert!(
                matches!(&outcome, Err(IpcError::Closed)),
                "cut {cut}: got {outcome:?}"
            );
        } else {
            // The stream ended inside the frame: a broken peer, not an
            // orderly end.
            assert!(
                matches!(&outcome, Err(IpcError::Truncated)),
                "cut {cut}: got {outcome:?}"
            );
        }
    }
}

#[test]
fn an_oversized_length_prefix_is_refused() {
    for len in [MAX_FRAME_LEN + 1, u32::MAX] {
        let mut reader = Cursor::new(len.to_le_bytes().to_vec());
        let outcome = read_frame(&mut reader);
        assert!(
            matches!(&outcome, Err(IpcError::FrameTooLarge(_))),
            "len {len}: got {outcome:?}"
        );
    }
}

#[test]
fn a_frame_without_a_payload_is_not_a_message() {
    // A length prefix of zero carries no variant index: it is a broken
    // frame, not a message.
    let mut reader = Cursor::new(0u32.to_le_bytes().to_vec());
    let outcome = read_frame(&mut reader);
    assert!(
        matches!(&outcome, Err(IpcError::Codec(_))),
        "got {outcome:?}"
    );
}

#[test]
fn a_payload_with_extra_bytes_keeps_the_stream_framed() {
    // The length prefix bounds the payload, so bytes behind the message
    // stay inside the frame: the frame that follows starts where this one
    // ended.
    let mut stream = Vec::new();
    write_frame(&mut stream, &Message::Ping(1)).expect("writing a frame");
    let payload_len = stream.len() - 4;
    stream.push(0xAA);
    let announced = u32::try_from(payload_len + 1).expect("a small payload");
    stream[0..4].copy_from_slice(&announced.to_le_bytes());

    let mut transport = StreamTransport::new(Cursor::new(stream), io::sink());
    assert_eq!(
        transport.recv().expect("the frame"),
        Message::Ping(1),
        "the message behind the extra bytes is the message"
    );
    assert!(
        matches!(transport.recv(), Err(IpcError::Closed)),
        "the frame must end where its length prefix says"
    );
}

#[test]
fn a_reader_that_would_block_reports_the_block_and_not_a_closed_stream() {
    let mut frame = Vec::new();
    write_frame(&mut frame, &Message::Ping(1)).expect("writing a frame");
    // The length prefix is there, the payload is not: the stream would
    // block in the middle of the frame.
    let mut reader = WouldBlockReader::new(frame, 4);
    let outcome = read_frame(&mut reader);
    assert!(
        matches!(&outcome, Err(IpcError::Io(io_error)) if io_error.kind() == io::ErrorKind::WouldBlock),
        "got {outcome:?}"
    );
}

#[test]
fn a_reader_that_fails_mid_frame_is_not_read_as_a_message() {
    let mut frame = Vec::new();
    write_frame(&mut frame, &Message::Ping(9)).expect("writing a frame");

    let mut reader = FailingReader::new(frame.clone(), 2);
    let outcome = read_frame(&mut reader);
    assert!(matches!(&outcome, Err(IpcError::Io(_))), "got {outcome:?}");

    // The transport refuses to read on: the stream is inside the frame, so
    // the next read would take the rest of it for a length prefix.
    let mut transport = StreamTransport::new(FailingReader::new(frame, 2), io::sink());
    assert!(matches!(transport.recv(), Err(IpcError::Io(_))));
    assert!(matches!(transport.recv(), Err(IpcError::Desynchronized)));
}

#[test]
fn the_transport_refuses_frames_after_a_broken_one() {
    // The write half: a frame that failed part-way leaves a broken frame in
    // the stream, so no further frame may go out - the peer would read the
    // rest of the broken one as the next frame.
    let mut transport = StreamTransport::new(io::empty(), FailingWriter::new(3));
    let error = transport
        .send(&Message::Ping(1))
        .expect_err("the writer fails");
    assert!(matches!(&error, IpcError::Io(_)), "got {error:?}");
    assert!(
        matches!(
            transport.send(&Message::Ping(2)),
            Err(IpcError::Desynchronized)
        ),
        "a second frame would follow the broken one"
    );

    // The read half: a frame that was cut short leaves the stream inside
    // the broken frame.
    let frame = frame_of(&Message::Ping(1));
    let mut transport = StreamTransport::new(
        ChunkReader::new(frame[..frame.len() - 1].to_vec(), 1),
        io::sink(),
    );
    assert!(matches!(transport.recv(), Err(IpcError::Truncated)));
    assert!(matches!(transport.recv(), Err(IpcError::Desynchronized)));
}

#[test]
fn the_transport_refuses_frames_after_a_refused_one() {
    // An oversized frame is refused before its payload is read, so the
    // stream is inside the refused frame.
    let mut transport =
        StreamTransport::new(Cursor::new(u32::MAX.to_le_bytes().to_vec()), io::sink());
    assert!(matches!(transport.recv(), Err(IpcError::FrameTooLarge(_))));
    assert!(matches!(transport.recv(), Err(IpcError::Desynchronized)));
}

#[test]
fn a_codec_error_leaves_the_stream_framed() {
    // A frame that was read whole but does not decode leaves the stream at
    // a frame boundary: the next frame is the next one.
    let mut stream = frame_of(&Message::Ping(1));
    // The payload's variant index is out of range: the frame is complete,
    // but it is not a message.
    stream[4] = 0xFF;

    let mut transport = StreamTransport::new(Cursor::new(stream), io::sink());
    assert!(matches!(transport.recv(), Err(IpcError::Codec(_))));
    // The stream is still framed, so the transport did not close it: the
    // next frame is the clean end of the stream.
    assert!(matches!(transport.recv(), Err(IpcError::Closed)));
}

#[test]
fn no_chunking_panics() {
    for seed in 0..256 {
        let mut rng = Rng::new(SEED.wrapping_add(seed));
        let message = random_message(&mut rng);
        let frame = frame_of(&message);

        // Read the frame back with a random chunk plan.
        let limit = rng.below(9) as usize + 1;
        let mut reader = RandomChunkReader::new(frame.clone(), limit, &mut rng);
        let decoded = read_frame(&mut reader)
            .unwrap_or_else(|error| panic!("seed {seed:#x}: a whole frame was refused: {error}"));
        assert_eq!(decoded, message, "seed {seed:#x}: the frame changed");

        // Write the frame through a writer that accepts a random number of
        // bytes per call.
        let chunk = rng.below(9) as usize + 1;
        let mut writer = ShortWriter::new(chunk);
        write_frame(&mut writer, &message)
            .unwrap_or_else(|error| panic!("seed {seed:#x}: writing the frame failed: {error}"));
        assert_eq!(writer.sink, frame, "seed {seed:#x}: the frame is not whole");

        // Cut the frame at a random place: the outcome is a typed error,
        // never a message and never a panic.
        let cut = rng.below(frame.len() as u64) as usize;
        let mut reader = ChunkReader::new(frame[..cut].to_vec(), 1);
        match read_frame(&mut reader) {
            Ok(decoded) => {
                panic!("seed {seed:#x}: a frame cut to {cut} was read as {decoded:?}")
            }
            Err(IpcError::Closed | IpcError::Truncated) => {}
            Err(error) => panic!("seed {seed:#x}: a cut frame gave {error:?}"),
        }
    }
}
