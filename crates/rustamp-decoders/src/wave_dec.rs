//! RIFF/WAVE decoder: the classic Winamp `in_wave` format family.
//!
//! Covers the RIFF/WAVE container with `WAVE_FORMAT_PCM` (8-bit unsigned,
//! 16/24/32-bit signed little-endian), `WAVE_FORMAT_IEEE_FLOAT` (32/64-bit) and
//! `WAVE_FORMAT_EXTENSIBLE` (sub-format PCM or IEEE float, including a valid-bit
//! count below the container width). Everything else is not claimed: the probe
//! only accepts `RIFF`/`WAVE`, and `open` rejects any other container or format
//! tag with a defined error. `RF64`/`BW64` (64-bit WAV), `RIFX` (big-endian
//! RIFF) and compressed WAV codecs (ADPCM, µ-law, ...) are out of scope and
//! named in `CONTRACT_NOTES.md`.
//!
//! Implemented from the public RIFF/WAVE container layout (chunked RIFF, the
//! `fmt ` and `data` chunks, the `WAVE_FORMAT_EXTENSIBLE` layout and the
//! standard KSDATAFORMAT sub-format GUIDs). No Winamp source, SDK header or
//! disassembly was consulted and nothing was copied.
//!
//! # Why this factory exists next to symphonia
//!
//! [`SymphoniaDecoderFactory`](crate::SymphoniaDecoderFactory) already decodes
//! WAV, so this is a deliberate second claimant rather than a gap-filler: it
//! claims the same extensions (`wav`, `wave`) and the same content signature
//! (`RIFF`/`WAVE`) at the same [`Confidence`] as symphonia. RFC 0013's selection
//! rule - content outranks extension, an equal-confidence tie goes to the
//! earlier registration, no claim at all is a refusal - is therefore exercised
//! in `default_registry` by two real decoders instead of a test double. It is
//! also a fallback that does not route the most basic format through the
//! symphonia dependency.

use std::fs::File;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::time::Duration;

use rustamp_core::audio::{
    AudioSpec, Confidence, Decoder, DecoderFactory, MediaSource, ProbeHint, ReadSeek,
};
use rustamp_core::error::{Error, Result};
use rustamp_core::meta::TrackMetadata;
use rustamp_core::state::StreamInfo;

use crate::symphonia_dec::{MAX_SEEK_FRAMES, frames_at};

/// Extensions this factory claims (lower-case, no dot).
const EXTENSIONS: &[&str] = &["wav", "wave"];

/// Frames converted per refill of the byte buffer.
const BLOCK_FRAMES: usize = 4096;

/// Upper bound on the conversion buffer.
///
/// A hostile `fmt ` chunk can claim up to 65535 channels; without this bound the
/// buffer would be sized from that number. A real file needs a few kilobytes.
const MAX_BUF_BYTES: usize = 64 * 1024;

/// `WAVE_FORMAT_PCM`.
const FORMAT_PCM: u16 = 0x0001;

/// `WAVE_FORMAT_IEEE_FLOAT`.
const FORMAT_FLOAT: u16 = 0x0003;

/// `WAVE_FORMAT_EXTENSIBLE`.
const FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// Bytes 2..16 of the standard KSDATAFORMAT sub-format GUID
/// (`xxxxxxxx-0000-0010-8000-00AA00389B71`); the first two bytes carry the
/// format tag.
const SUBFORMAT_TAIL: [u8; 14] = [
    0x00, 0x00, 0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
];

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for the RIFF/WAVE decoder.
#[derive(Debug, Default, Clone, Copy)]
pub struct WaveDecoderFactory;

impl WaveDecoderFactory {
    /// Create the factory.
    pub fn new() -> Self {
        WaveDecoderFactory
    }
}

/// Whether the header bytes carry the RIFF/WAVE signature.
///
/// The same 12-byte test symphonia applies, so a real WAV file is a genuine
/// equal-confidence tie between the two factories (RFC 0013).
fn is_riff_wave(h: &[u8]) -> bool {
    h.len() >= 12 && &h[0..4] == b"RIFF" && &h[8..12] == b"WAVE"
}

impl DecoderFactory for WaveDecoderFactory {
    fn name(&self) -> &str {
        "wave"
    }

    fn extensions(&self) -> &[&str] {
        EXTENSIONS
    }

    fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
        if is_riff_wave(hint.header) {
            return Confidence::CONTENT;
        }
        match hint.extension {
            Some(e) if EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()) => {
                Confidence::EXTENSION
            }
            _ => Confidence::NONE,
        }
    }

    fn open(&self, source: MediaSource) -> Result<Box<dyn Decoder>> {
        Ok(Box::new(WaveDecoder::open(source)?))
    }
}

// ---------------------------------------------------------------------------
// Container parsing
// ---------------------------------------------------------------------------

/// Sample layout of the `fmt ` chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SampleKind {
    /// 8-bit unsigned (offset binary): the one WAV integer format that is not
    /// two's complement.
    U8,
    /// 16-bit signed little-endian.
    S16,
    /// 24-bit signed little-endian.
    S24,
    /// 32-bit signed little-endian.
    S32,
    /// 32-bit IEEE float little-endian.
    F32,
    /// 64-bit IEEE float little-endian.
    F64,
}

impl SampleKind {
    /// Bytes one sample occupies in the container.
    fn container_bytes(self) -> usize {
        match self {
            SampleKind::U8 => 1,
            SampleKind::S16 => 2,
            SampleKind::S24 => 3,
            SampleKind::S32 | SampleKind::F32 => 4,
            SampleKind::F64 => 8,
        }
    }

    /// Bits one sample occupies in the container.
    fn container_bits(self) -> u16 {
        (self.container_bytes() * 8) as u16
    }

    /// The sample kind for a resolved format tag and container width.
    fn from_tag(tag: u16, bits: u16) -> Option<SampleKind> {
        match (tag, bits) {
            (FORMAT_PCM, 8) => Some(SampleKind::U8),
            (FORMAT_PCM, 16) => Some(SampleKind::S16),
            (FORMAT_PCM, 24) => Some(SampleKind::S24),
            (FORMAT_PCM, 32) => Some(SampleKind::S32),
            (FORMAT_FLOAT, 32) => Some(SampleKind::F32),
            (FORMAT_FLOAT, 64) => Some(SampleKind::F64),
            _ => None,
        }
    }
}

/// The parts of the `fmt ` chunk the decoder needs.
#[derive(Debug, Clone, Copy)]
struct Fmt {
    kind: SampleKind,
    /// Right shift that removes the unused low bits of a container whose
    /// valid-bit count is smaller (0 for the common case).
    shift: u32,
    channels: u16,
    sample_rate: u32,
}

/// Where the sample data starts and how long it is.
#[derive(Debug, Clone, Copy)]
struct Data {
    /// Byte offset of the first sample.
    start: u64,
    /// Bytes of sample data, when known. `None` for a streaming `data` chunk
    /// (`0xFFFFFFFF`) in a source that cannot report its length: read to EOF.
    len: Option<u64>,
}

/// Little-endian `u16` at `at`.
fn le_u16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

/// Little-endian `u32` at `at`.
fn le_u32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Skips `n` bytes forward. `n` never exceeds `u32::MAX + 1`, so the `i64`
/// conversion is exact.
fn skip(reader: &mut dyn ReadSeek, n: u64) -> Result<()> {
    if n > 0 {
        reader.seek(SeekFrom::Current(n as i64))?;
    }
    Ok(())
}

/// Reads the RIFF/WAVE header and walks the chunk list to the `fmt ` and `data`
/// chunks.
///
/// `file_len` is the total source length when it could be determined; it is used
/// only to clamp a `data` size that reaches past the end of the source.
fn parse(reader: &mut dyn ReadSeek, file_len: Option<u64>) -> Result<(Fmt, Data)> {
    let mut head = [0u8; 12];
    reader.read_exact(&mut head)?;
    if !is_riff_wave(&head) {
        return Err(Error::UnsupportedFormat("not a RIFF/WAVE file".into()));
    }

    let mut fmt: Option<Fmt> = None;
    let mut data: Option<Data> = None;
    loop {
        // Each iteration consumes at least the 8-byte chunk header, so the walk
        // is bounded by the source length; a truncated header ends it.
        let mut chunk = [0u8; 8];
        match reader.read_exact(&mut chunk) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(Error::Io(e)),
        }
        let size = le_u32(&chunk, 4);
        if &chunk[0..4] == b"fmt " {
            if size < 16 {
                return Err(Error::UnsupportedFormat(
                    "fmt chunk shorter than 16 bytes".into(),
                ));
            }
            // Only the first 40 bytes are interesting; the rest is skipped.
            let want = size.min(40) as usize;
            let mut body = [0u8; 40];
            reader.read_exact(&mut body[..want])?;
            skip(reader, u64::from(size) - want as u64)?;
            fmt = Some(read_fmt(&body, want)?);
        } else if &chunk[0..4] == b"data" {
            if fmt.is_none() {
                return Err(Error::UnsupportedFormat(
                    "data chunk before fmt chunk".into(),
                ));
            }
            let start = reader.stream_position()?;
            let len = match file_len {
                Some(total) => Some(u64::from(size).min(total.saturating_sub(start))),
                // Without a length the declared size is the only bound, except
                // for the streaming marker, which is not a length at all.
                None if size == u32::MAX => None,
                None => Some(u64::from(size)),
            };
            data = Some(Data { start, len });
            // The sample data is bounded by `size`; anything after it (including
            // the RIFF pad byte of an odd `data` size) is not audio.
            break;
        } else {
            // Unknown chunk (LIST, fact, bext, ...).
            skip(reader, u64::from(size))?;
        }
        // A chunk of odd size is followed by one pad byte, which is not counted
        // in the chunk size.
        if size % 2 == 1 {
            skip(reader, 1)?;
        }
    }

    let fmt = fmt.ok_or_else(|| Error::UnsupportedFormat("no fmt chunk".into()))?;
    let data = data.ok_or_else(|| Error::UnsupportedFormat("no data chunk".into()))?;
    Ok((fmt, data))
}

/// Reads the format fields out of an already-read `fmt ` body (a WAVEFORMATEX).
///
/// `len` is how many bytes of the body were read (`min(size, 40)`).
fn read_fmt(body: &[u8], len: usize) -> Result<Fmt> {
    let mut tag = le_u16(body, 0);
    let channels = le_u16(body, 2);
    let sample_rate = le_u32(body, 4);
    let bits = le_u16(body, 14);
    let mut valid = bits;
    if tag == FORMAT_EXTENSIBLE {
        if len < 40 {
            return Err(Error::UnsupportedFormat(
                "truncated WAVE_FORMAT_EXTENSIBLE fmt chunk".into(),
            ));
        }
        if le_u16(body, 16) < 22 {
            return Err(Error::UnsupportedFormat(
                "WAVE_FORMAT_EXTENSIBLE without a sub-format".into(),
            ));
        }
        if body[26..40] != SUBFORMAT_TAIL {
            return Err(Error::UnsupportedFormat(
                "unknown WAVE sub-format GUID".into(),
            ));
        }
        tag = le_u16(body, 24);
        valid = le_u16(body, 18);
    }
    if channels == 0 || sample_rate == 0 {
        return Err(Error::UnsupportedFormat(
            "fmt chunk with zero channels or sample rate".into(),
        ));
    }
    let kind = SampleKind::from_tag(tag, bits).ok_or_else(|| {
        Error::UnsupportedFormat(format!("unsupported WAVE format tag {tag} at {bits} bits"))
    })?;
    let shift = match kind {
        SampleKind::F32 | SampleKind::F64 => {
            if valid != bits {
                return Err(Error::UnsupportedFormat(
                    "float samples with a valid-bit count below the container".into(),
                ));
            }
            0
        }
        _ => {
            if valid == 0 || valid > bits {
                return Err(Error::UnsupportedFormat(
                    "invalid WAVE valid-bit count".into(),
                ));
            }
            // 8-bit WAV is offset binary and its container byte *is* the sample
            // byte, so the mapping in `convert` needs no shift; every other
            // integer container holds the sample left-justified.
            if kind == SampleKind::U8 {
                0
            } else {
                u32::from(bits - valid)
            }
        }
    };
    Ok(Fmt {
        kind,
        shift,
        channels,
        sample_rate,
    })
}

// ---------------------------------------------------------------------------
// Sample conversion
// ---------------------------------------------------------------------------

/// Full-scale divisor of a signed container: `2^(bits - 1 - shift)`.
fn signed_scale(bits: u32, shift: u32) -> f32 {
    (1u32 << (bits - 1 - shift)) as f32
}

/// Converts interleaved little-endian samples to interleaved `f32`.
///
/// `src.len()` must be a whole number of frames and `out.len()` must be the
/// sample count of those frames; `zip` then pairs every sample with its value.
/// The integer divisors (`2^(bits-1)`) are the ones symphonia uses, so both
/// decoders produce the same samples for the same file.
fn convert(kind: SampleKind, shift: u32, src: &[u8], out: &mut [f32]) {
    match kind {
        // 8-bit WAV is unsigned with midpoint 128 (offset binary).
        SampleKind::U8 => {
            for (o, b) in out.iter_mut().zip(src.iter()) {
                *o = (f32::from(*b) - 128.0) / 128.0;
            }
        }
        SampleKind::S16 => {
            let scale = signed_scale(16, shift);
            let (chunks, _) = src.as_chunks::<2>();
            for (o, c) in out.iter_mut().zip(chunks) {
                let v = i16::from_le_bytes(*c);
                *o = f32::from(v >> shift) / scale;
            }
        }
        SampleKind::S24 => {
            let scale = signed_scale(24, shift);
            let (chunks, _) = src.as_chunks::<3>();
            for (o, c) in out.iter_mut().zip(chunks) {
                let sign = if c[2] & 0x80 == 0 { 0x00 } else { 0xFF };
                let v = i32::from_le_bytes([c[0], c[1], c[2], sign]);
                *o = (v >> shift) as f32 / scale;
            }
        }
        SampleKind::S32 => {
            let scale = signed_scale(32, shift);
            let (chunks, _) = src.as_chunks::<4>();
            for (o, c) in out.iter_mut().zip(chunks) {
                let v = i32::from_le_bytes(*c);
                *o = (v >> shift) as f32 / scale;
            }
        }
        SampleKind::F32 => {
            let (chunks, _) = src.as_chunks::<4>();
            for (o, c) in out.iter_mut().zip(chunks) {
                *o = f32::from_le_bytes(*c);
            }
        }
        SampleKind::F64 => {
            let (chunks, _) = src.as_chunks::<8>();
            for (o, c) in out.iter_mut().zip(chunks) {
                *o = f64::from_le_bytes(*c) as f32;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// Decoder for RIFF/WAVE PCM and IEEE-float audio.
pub struct WaveDecoder {
    reader: Box<dyn ReadSeek>,
    spec: AudioSpec,
    kind: SampleKind,
    shift: u32,
    /// Bytes one frame occupies.
    frame_bytes: usize,
    /// Byte offset of the first sample in the source.
    data_start: u64,
    /// Declared length of the sample data, when known.
    data_len: Option<u64>,
    /// Bytes of the sample data already read from the source.
    data_read: u64,
    /// Frames of sample data in the source, when known.
    total_frames: Option<u64>,
    /// Byte buffer holding the samples not yet converted.
    buf: Vec<u8>,
    buf_len: usize,
    buf_pos: usize,
    skipped: u64,
    meta: TrackMetadata,
}

impl WaveDecoder {
    /// Open a decoder for `source` (`Path` or `Reader`).
    pub fn open(source: MediaSource) -> Result<Self> {
        let (mut reader, file_len) = match source {
            MediaSource::Url(_) => {
                return Err(Error::Unsupported(
                    "URL sources are not supported by the wave decoder; network streaming comes in M4",
                ));
            }
            MediaSource::Path(p) => {
                let f = File::open(&p)?;
                let len = f.metadata().ok().map(|m| m.len());
                (Box::new(f) as Box<dyn ReadSeek>, len)
            }
            MediaSource::Reader { reader, .. } => (reader, None),
        };
        // A reader that cannot report its end leaves `file_len` unknown; the
        // declared `data` size is then the only bound (see `parse`).
        let file_len = match file_len {
            Some(len) => Some(len),
            None => {
                let len = reader.seek(SeekFrom::End(0)).ok();
                reader.seek(SeekFrom::Start(0))?;
                len
            }
        };

        let (fmt, data) = parse(&mut *reader, file_len)?;
        let frame_bytes = usize::from(fmt.channels) * fmt.kind.container_bytes();
        let total_frames = data.len.map(|len| len / frame_bytes as u64);
        let spec = AudioSpec::new(fmt.sample_rate, fmt.channels);
        // WAV is uncompressed, so the bitrate is the sample format's own.
        let bitrate = u64::from(fmt.sample_rate)
            .saturating_mul(u64::from(fmt.channels))
            .saturating_mul(u64::from(fmt.kind.container_bits()))
            / 1000;
        let mut meta = TrackMetadata {
            bitrate_kbps: Some(bitrate.min(u64::from(u32::MAX)) as u32),
            ..TrackMetadata::default()
        };
        meta.duration_ms = total_frames.map(|n| spec.duration_of(n).as_millis() as u64);

        let cap = ((BLOCK_FRAMES * frame_bytes).min(MAX_BUF_BYTES) / frame_bytes * frame_bytes)
            .max(frame_bytes);
        let mut this = WaveDecoder {
            reader,
            spec,
            kind: fmt.kind,
            shift: fmt.shift,
            frame_bytes,
            data_start: data.start,
            data_len: data.len,
            data_read: 0,
            total_frames,
            buf: vec![0u8; cap],
            buf_len: 0,
            buf_pos: 0,
            skipped: 0,
            meta,
        };
        this.reader.seek(SeekFrom::Start(data.start))?;
        Ok(this)
    }

    /// Number of corrupt units skipped so far (currently: a trailing partial
    /// frame that the declared `data` size cut in half).
    pub fn skipped_packets(&self) -> u64 {
        self.skipped
    }

    /// Refills the byte buffer from the source, bounded by the declared `data`
    /// length. Leaves `buf_len == 0` at the end of the sample data.
    fn refill(&mut self) -> Result<()> {
        self.buf_pos = 0;
        self.buf_len = 0;
        let want = match self.data_len {
            Some(len) => len
                .saturating_sub(self.data_read)
                .min(self.buf.len() as u64) as usize,
            None => self.buf.len(),
        };
        if want == 0 {
            return Ok(());
        }
        let mut filled = 0usize;
        // Loop until the request is met or the source ends: a reader that
        // returns short reads must not be mistaken for the end of the stream,
        // and a short buffer would misalign the frame boundary.
        while filled < want {
            match self.reader.read(&mut self.buf[filled..want]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(Error::Io(e)),
            }
        }
        self.data_read += filled as u64;
        self.buf_len = filled;
        Ok(())
    }
}

impl Decoder for WaveDecoder {
    fn spec(&self) -> AudioSpec {
        self.spec
    }

    /// Forwards to the inherent method, so a `Box<dyn Decoder>` sees the real
    /// count instead of the trait default.
    fn skipped_packets(&self) -> u64 {
        self.skipped
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let ch = usize::from(self.spec.channels.max(1));
        let want = out.len() / ch;
        let mut done = 0usize;
        while done < want {
            if self.buf_pos >= self.buf_len {
                self.refill()?;
                if self.buf_len == 0 {
                    break;
                }
            }
            let frames = (self.buf_len - self.buf_pos) / self.frame_bytes;
            if frames == 0 {
                // Fewer bytes than one frame are left: the declared `data` size
                // cut the last frame. Drop it and end the stream; the next
                // refill reports the end.
                self.skipped += 1;
                self.buf_pos = self.buf_len;
                continue;
            }
            let n = frames.min(want - done);
            let src = &self.buf[self.buf_pos..self.buf_pos + n * self.frame_bytes];
            convert(
                self.kind,
                self.shift,
                src,
                &mut out[done * ch..(done + n) * ch],
            );
            self.buf_pos += n * self.frame_bytes;
            done += n;
        }
        Ok(done)
    }

    fn seek(&mut self, to: Duration) -> Result<Duration> {
        let rate = u64::from(self.spec.sample_rate.max(1));
        let mut target = to;
        if let Some(d) = self.duration() {
            target = target.min(d);
        }
        // A `Duration` has nanosecond precision, so a target derived from a
        // frame index (`duration_of(n)`) can sit a hair below frame `n`. Half a
        // frame of slack makes such a target land on its frame instead of the
        // one before it (the same reasoning as the symphonia pre-roll).
        let slack = Duration::from_secs_f64(0.5 / f64::from(self.spec.sample_rate.max(1)));
        // WAV is uncompressed, so a seek is a byte offset: sample exact, no
        // pre-roll. `frames_at` saturates and `MAX_SEEK_FRAMES` keeps the frame
        // count convertible back to a `Duration` for a source of unknown length.
        let mut frames = frames_at(rate, target.saturating_add(slack)).min(MAX_SEEK_FRAMES);
        if let Some(total) = self.total_frames {
            frames = frames.min(total);
        }
        let offset = frames
            .saturating_mul(self.frame_bytes as u64)
            .saturating_add(self.data_start);
        self.reader.seek(SeekFrom::Start(offset))?;
        self.buf_pos = 0;
        self.buf_len = 0;
        self.data_read = frames.saturating_mul(self.frame_bytes as u64);
        Ok(self.spec.duration_of(frames))
    }

    fn duration(&self) -> Option<Duration> {
        self.total_frames.map(|n| self.spec.duration_of(n))
    }

    fn metadata(&self) -> TrackMetadata {
        self.meta.clone()
    }

    fn stream_info(&self) -> StreamInfo {
        StreamInfo {
            bitrate_kbps: self.meta.bitrate_kbps,
            sample_rate_hz: self.spec.sample_rate,
            channels: self.spec.channels,
        }
    }

    fn is_seekable(&self) -> bool {
        // `MediaSource` is seekable by contract and a WAV seek is a byte
        // offset, so this holds even for a streaming `data` chunk of unknown
        // length.
        true
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    const RATE: u32 = 44_100;

    /// Assembles a RIFF/WAVE file from chunk payloads. Odd payloads get the RIFF
    /// pad byte, so the builder cannot accidentally produce a misaligned file.
    fn riff_wave(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut body = b"WAVE".to_vec();
        for (id, payload) in chunks {
            body.extend_from_slice(*id);
            body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            body.extend_from_slice(payload);
            if payload.len() % 2 == 1 {
                body.push(0);
            }
        }
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// The standard KSDATAFORMAT sub-format GUID for `tag`.
    fn subformat(tag: u16) -> Vec<u8> {
        let mut g = tag.to_le_bytes().to_vec();
        g.extend_from_slice(&SUBFORMAT_TAIL);
        g
    }

    /// A WAVEFORMATEX body (without the chunk size field). `sub_tag` is only
    /// used for `WAVE_FORMAT_EXTENSIBLE`.
    fn fmt_body(
        tag: u16,
        sub_tag: u16,
        channels: u16,
        rate: u32,
        container_bits: u16,
        valid_bits: u16,
    ) -> Vec<u8> {
        let sample_bytes = u32::from(container_bits.div_ceil(8));
        let block_align = channels * container_bits.div_ceil(8);
        let mut b = Vec::new();
        let extensible = tag == FORMAT_EXTENSIBLE;
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&channels.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * sample_bytes * u32::from(channels)).to_le_bytes());
        b.extend_from_slice(&block_align.to_le_bytes());
        b.extend_from_slice(&container_bits.to_le_bytes());
        if extensible {
            b.extend_from_slice(&22u16.to_le_bytes()); // cbSize
            b.extend_from_slice(&valid_bits.to_le_bytes());
            b.extend_from_slice(&0u32.to_le_bytes()); // dwChannelMask
            b.extend_from_slice(&subformat(sub_tag));
        }
        b
    }

    /// A WAV file with one mono/stereo `fmt ` chunk and a `data` payload.
    fn wav_with(channels: u16, rate: u32, tag: u16, container_bits: u16, data: Vec<u8>) -> Vec<u8> {
        riff_wave(&[
            (
                b"fmt ",
                fmt_body(tag, tag, channels, rate, container_bits, container_bits),
            ),
            (b"data", data),
        ])
    }

    /// A 16-bit stereo PCM file whose samples are `samples`.
    fn pcm16(samples: &[i16]) -> Vec<u8> {
        let mut data = Vec::new();
        for s in samples {
            data.extend_from_slice(&s.to_le_bytes());
        }
        wav_with(2, RATE, FORMAT_PCM, 16, data)
    }

    /// Overwrites the declared size of the `data` chunk, so a test can claim a
    /// length the file does not really hold.
    ///
    /// The chunk id is located instead of using `bytes.len() - 8`: that offset
    /// is the size field only when the payload happens to be exactly four bytes
    /// long, and a stereo fixture overwrote its own audio instead (the duration
    /// then stayed known, because the size field still said 8). The walk starts
    /// after the 12-byte RIFF header, where the first chunk id is.
    fn patch_data_size(bytes: &mut [u8], size: u32) {
        // The first `data` id after the header is the real one: the payload
        // follows it, so a look-alike sequence inside a sample cannot win.
        let at = bytes[12..]
            .windows(4)
            .position(|window| window == b"data")
            .map(|offset| offset + 12)
            .expect("the fixture has a data chunk");
        bytes[at + 4..at + 8].copy_from_slice(&size.to_le_bytes());
    }

    /// A 16-bit mono PCM file whose samples are `samples`.
    fn pcm16_mono(samples: &[i16]) -> Vec<u8> {
        let mut data = Vec::new();
        for s in samples {
            data.extend_from_slice(&s.to_le_bytes());
        }
        wav_with(1, RATE, FORMAT_PCM, 16, data)
    }

    fn open_bytes(bytes: Vec<u8>) -> Result<WaveDecoder> {
        WaveDecoder::open(MediaSource::Reader {
            reader: Box::new(Cursor::new(bytes)),
            extension: Some("wav".into()),
        })
    }

    fn read_all(d: &mut WaveDecoder) -> Vec<f32> {
        let ch = usize::from(d.spec().channels.max(1));
        let mut out = Vec::new();
        let mut buf = vec![0.0f32; 512 * ch];
        loop {
            match d.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n * ch]),
                Err(e) => panic!("read: {e}"),
            }
        }
        out
    }

    #[test]
    fn probe_claims_riff_wave_content_and_wav_extensions_only() {
        let f = WaveDecoderFactory::new();
        let wav = pcm16(&[0, 0]);
        assert_eq!(
            f.probe(&ProbeHint {
                extension: None,
                header: &wav,
            }),
            Confidence::CONTENT
        );
        // A lying extension does not outrank the signature.
        assert_eq!(
            f.probe(&ProbeHint {
                extension: Some("mp3"),
                header: &wav,
            }),
            Confidence::CONTENT
        );
        // An extension claim without a signature.
        assert_eq!(
            f.probe(&ProbeHint {
                extension: Some("WAVE"),
                header: b"no signature here",
            }),
            Confidence::EXTENSION
        );
        // A RIFF container that is not WAVE is not claimed at all.
        assert_eq!(
            f.probe(&ProbeHint {
                extension: None,
                header: b"RIFF\x00\x00\x00\x00AVI LIST",
            }),
            Confidence::NONE
        );
        assert_eq!(
            f.probe(&ProbeHint {
                extension: Some("flac"),
                header: &[0x66, 0x4C, 0x61, 0x43],
            }),
            Confidence::NONE
        );
    }

    #[test]
    fn plain_16_bit_pcm_decodes_exactly() {
        let samples: Vec<i16> = vec![-32768, 32767, 0, -1, 1, 16384];
        let mut d = open_bytes(pcm16(&samples)).expect("open");
        assert_eq!(d.spec(), AudioSpec::new(RATE, 2));
        assert_eq!(d.duration(), Some(d.spec().duration_of(3)));
        assert_eq!(d.stream_info().bitrate_kbps, Some(1411));
        let want: Vec<f32> = samples.iter().map(|v| f32::from(*v) / 32_768.0).collect();
        assert_eq!(read_all(&mut d), want);
        assert_eq!(d.skipped_packets(), 0);
    }

    #[test]
    fn eight_bit_pcm_is_offset_binary() {
        let mut d =
            open_bytes(wav_with(1, RATE, FORMAT_PCM, 8, vec![0x00, 0x80, 0xFF])).expect("open");
        assert_eq!(d.spec(), AudioSpec::new(RATE, 1));
        assert_eq!(d.stream_info().bitrate_kbps, Some(352));
        assert_eq!(read_all(&mut d), vec![-1.0, 0.0, 0.992_187_5]);
    }

    #[test]
    fn extensible_24_bit_with_20_valid_bits_shifts_the_sample() {
        // The sample is left-justified in the container: the 20-bit value sits
        // in the top 20 bits of three bytes.
        let max = 524_287i32; // 2^19 - 1
        let min = -524_288i32; // -2^19
        let mut data = Vec::new();
        for v in [max, min, 0] {
            let container = (v << 4) as u32;
            data.extend_from_slice(&container.to_le_bytes()[..3]);
        }
        let bytes = riff_wave(&[
            (
                b"fmt ",
                fmt_body(FORMAT_EXTENSIBLE, FORMAT_PCM, 1, RATE, 24, 20),
            ),
            (b"data", data),
        ]);
        let mut d = open_bytes(bytes).expect("open");
        assert_eq!(d.spec(), AudioSpec::new(RATE, 1));
        assert_eq!(
            read_all(&mut d),
            vec![524_287.0 / 524_288.0, -1.0, 0.0],
            "the valid-bit shift must be undone"
        );
    }

    #[test]
    fn extensible_float_32_and_64_bit_samples_decode() {
        let mut data32 = Vec::new();
        for v in [0.5f32, -0.25] {
            data32.extend_from_slice(&v.to_le_bytes());
        }
        let mut d = open_bytes(riff_wave(&[
            (
                b"fmt ",
                fmt_body(FORMAT_EXTENSIBLE, FORMAT_FLOAT, 1, RATE, 32, 32),
            ),
            (b"data", data32),
        ]))
        .expect("open");
        assert_eq!(read_all(&mut d), vec![0.5, -0.25]);

        let mut data64 = Vec::new();
        for v in [0.5f64, -1.5] {
            data64.extend_from_slice(&v.to_le_bytes());
        }
        let mut d = open_bytes(riff_wave(&[
            (
                b"fmt ",
                fmt_body(FORMAT_EXTENSIBLE, FORMAT_FLOAT, 1, RATE, 64, 64),
            ),
            (b"data", data64),
        ]))
        .expect("open");
        assert_eq!(read_all(&mut d), vec![0.5, -1.5]);
    }

    #[test]
    fn unknown_chunks_and_odd_padding_are_skipped() {
        // A 3-byte unknown chunk before `data` forces the RIFF pad byte.
        let bytes = riff_wave(&[
            (b"fmt ", fmt_body(FORMAT_PCM, 0, 1, RATE, 16, 16)),
            (b"junk", vec![1, 2, 3]),
            (b"LIST", vec![0, 0, 0, 0]),
            (b"data", vec![0x00, 0x40]),
        ]);
        let mut d = open_bytes(bytes).expect("open");
        assert_eq!(read_all(&mut d), vec![0.5]);
    }

    #[test]
    fn a_riff_container_that_is_not_wave_is_refused() {
        let mut avi = b"RIFF".to_vec();
        avi.extend_from_slice(&4u32.to_le_bytes());
        avi.extend_from_slice(b"AVI ");
        let err = open_bytes(avi).err().expect("AVI is not a WAV");
        assert!(
            matches!(&err, Error::UnsupportedFormat(m) if m.contains("not a RIFF/WAVE")),
            "{err:?}"
        );
    }

    #[test]
    fn a_data_size_past_the_end_of_the_source_is_clamped() {
        // The header claims 4 GiB of samples; only two frames are present.
        let mut bytes = pcm16_mono(&[16_384, -16_384]);
        patch_data_size(&mut bytes, u32::MAX);
        let mut d = open_bytes(bytes).expect("open");
        assert_eq!(d.duration(), Some(d.spec().duration_of(2)));
        assert_eq!(read_all(&mut d), vec![0.5, -0.5]);
    }

    #[test]
    fn a_declared_data_size_larger_than_the_file_ends_early() {
        let mut bytes = pcm16_mono(&[16_384, -16_384]);
        patch_data_size(&mut bytes, 1000);
        let mut d = open_bytes(bytes).expect("open");
        // The source length clamps the claim, so even the duration is honest.
        assert_eq!(d.duration(), Some(d.spec().duration_of(2)));
        assert_eq!(read_all(&mut d), vec![0.5, -0.5]);
    }

    #[test]
    fn a_trailing_partial_frame_is_dropped_and_counted() {
        // 16-bit mono with a five-byte `data` chunk: one frame, one half frame.
        let bytes = riff_wave(&[
            (b"fmt ", fmt_body(FORMAT_PCM, 0, 1, RATE, 16, 16)),
            (b"data", vec![0x00, 0x40, 0x00, 0xC0, 0x7F]),
        ]);
        let mut d = open_bytes(bytes).expect("open");
        assert_eq!(read_all(&mut d), vec![0.5, -0.5]);
        assert_eq!(d.skipped_packets(), 1);
    }

    #[test]
    fn a_stream_of_unknown_length_reads_to_the_end_without_a_duration() {
        /// A cursor that refuses `SeekFrom::End`, so the decoder cannot learn
        /// the source length and the streaming `data` marker is all it has.
        struct NoEndSeek(Cursor<Vec<u8>>);

        impl Read for NoEndSeek {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.0.read(buf)
            }
        }

        impl Seek for NoEndSeek {
            fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
                match pos {
                    SeekFrom::End(_) => Err(std::io::Error::new(
                        ErrorKind::Unsupported,
                        "this stream cannot report its end",
                    )),
                    other => self.0.seek(other),
                }
            }
        }

        let mut bytes = pcm16(&[16_384, -16_384, 0, 0]);
        patch_data_size(&mut bytes, u32::MAX);
        let source = MediaSource::Reader {
            reader: Box::new(NoEndSeek(Cursor::new(bytes))),
            extension: None,
        };
        let mut d = WaveDecoder::open(source).expect("open");
        // The marker patched above is what makes the length unknown: without it
        // the declared size would give a duration. The eight data bytes are two
        // stereo frames (16-bit, two channels), so four samples.
        assert_eq!(d.duration(), None, "the length is genuinely unknown");
        assert_eq!(read_all(&mut d), vec![0.5, -0.5, 0.0, 0.0]);
    }

    #[test]
    fn seek_is_sample_exact_and_clamped() {
        let samples: Vec<i16> = (0..100).map(|i| i as i16).collect();
        let mut d = open_bytes(pcm16_mono(&samples)).expect("open");
        let duration = d.duration().expect("a wav knows its length");
        assert_eq!(duration, d.spec().duration_of(100));

        let reached = d
            .seek(d.spec().duration_of(10))
            .expect("a frame-derived target lands on its frame");
        assert_eq!(reached, d.spec().duration_of(10));
        assert_eq!(read_all(&mut d)[0], 10.0 / 32_768.0);

        // Past the end: clamped, and the read after it is a clean end.
        assert_eq!(d.seek(Duration::MAX).expect("clamped seek"), duration);
        let mut buf = [0.0f32; 4];
        assert_eq!(d.read(&mut buf).expect("read at the end"), 0);

        // And back to the start.
        assert_eq!(d.seek(Duration::ZERO).expect("rewind"), Duration::ZERO);
        assert_eq!(read_all(&mut d)[0], 0.0);
    }

    #[test]
    fn a_hostile_or_unsupported_fmt_chunk_is_refused() {
        let cases: &[(&str, Vec<u8>)] = &[
            ("zero channels", fmt_body(FORMAT_PCM, 0, 0, RATE, 16, 16)),
            ("zero rate", fmt_body(FORMAT_PCM, 0, 1, 0, 16, 16)),
            // MS ADPCM: a real WAV format tag this decoder does not claim.
            ("adpcm", fmt_body(0x0002, 0, 1, RATE, 4, 4)),
            (
                "float with valid bits",
                fmt_body(FORMAT_EXTENSIBLE, FORMAT_FLOAT, 1, RATE, 32, 24),
            ),
            (
                "valid bits above the container",
                fmt_body(FORMAT_EXTENSIBLE, FORMAT_PCM, 1, RATE, 16, 17),
            ),
            (
                "valid bits of zero",
                fmt_body(FORMAT_EXTENSIBLE, FORMAT_PCM, 1, RATE, 16, 0),
            ),
            ("unknown sub-format GUID", {
                let mut f = fmt_body(FORMAT_EXTENSIBLE, FORMAT_PCM, 1, RATE, 16, 16);
                f[26] = 0x11; // corrupt the GUID tail
                f
            }),
            ("extensible without a sub-format", {
                let mut f = fmt_body(FORMAT_EXTENSIBLE, FORMAT_PCM, 1, RATE, 16, 16);
                f[16..18].copy_from_slice(&0u16.to_le_bytes()); // cbSize
                f
            }),
        ];
        for (label, fmt) in cases {
            let bytes = riff_wave(&[(b"fmt ", fmt.clone()), (b"data", vec![0, 0, 0, 0])]);
            let err = open_bytes(bytes).err().unwrap_or_else(|| panic!("{label}"));
            assert!(
                matches!(err, Error::UnsupportedFormat(_)),
                "{label}: {err:?}"
            );
        }
    }

    #[test]
    fn a_truncated_fmt_chunk_is_refused() {
        // A `fmt ` chunk that claims 16 bytes but stops after 8.
        let mut body = b"WAVE".to_vec();
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&16u32.to_le_bytes());
        body.extend_from_slice(&[0u8; 8]);
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&(body.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&body);
        let err = open_bytes(bytes)
            .err()
            .expect("a truncated fmt cannot open");
        assert!(matches!(err, Error::Io(_)), "{err:?}");
    }

    #[test]
    fn a_url_source_is_refused_with_the_usual_error() {
        let err = WaveDecoder::open(MediaSource::Url("http://h/a.wav".into()))
            .err()
            .expect("no network");
        assert!(matches!(err, Error::Unsupported(_)), "{err:?}");
    }
}
