//! Symphonia-backed decoder.
//!
//! Behaviour the core contract leaves open (also in `CONTRACT_NOTES.md`):
//!
//! * `spec()` is fixed at open time from the codec parameters (or the first
//!   decoded buffer when those are incomplete).
//! * Mid-stream **channel count** change: audio is remapped to the opened
//!   channel count (mono is duplicated, extra channels dropped, missing ones
//!   zero-filled).
//! * Mid-stream **sample rate** change: reported once as
//!   [`Error::FormatChanged`] (RFC 0002, C3). The switch happens in the failing
//!   `read` call, which writes no frames; the next call continues in the new
//!   format.
//! * A container reset (chained Ogg) cannot be continued, so the stream ends
//!   (`read` returns `Ok(0)`) as C3.4 requires.
//! * A **truncated** file is a decode failure, not an end of stream: when the
//!   container states how many frames the file holds and the stream ends
//!   before that count is reached, the file was cut and `read` reports
//!   [`Error::Decode`]. A file that holds everything its container declares
//!   ends with `Ok(0)` as before. The rule is limited to the containers that
//!   state their payload exactly (RIFF/WAVE, AIFF, MPEG audio with a
//!   Xing/Info header); see [`declared_payload`].
//! * **MP4/M4A states its payload exactly too**, but its container reader
//!   scans the whole file before it plays anything: a cut MP4 file is refused
//!   when it is opened, not at the end of the stream. The decoder names the
//!   cut instead of letting the bare "end of stream" of the reader stand (see
//!   [`mp4_truncation`]).
//! * **FLAC has no such oracle** (a named limit, see `CONTRACT_NOTES.md`):
//!   its `STREAMINFO` block states the number of samples exactly, but the
//!   format states no byte count anywhere, and a damaged FLAC frame is
//!   dropped by the reader - so a corrupt file is short of samples exactly
//!   like a cut one. A truncated FLAC file still ends with an end of stream.
//!   The **metadata** of a FLAC file is checked, though: a file without a
//!   `STREAMINFO` block and a file whose metadata chain was cut are named
//!   when they are opened (see [`flac_refusal`]).
//! * Corrupt packets are skipped; see [`SymphoniaDecoder::skipped_packets`].
//! * Hostile input (truncated, empty, lying headers, noise) is rejected with a
//!   defined error or degraded to a defined end of stream; the loop in
//!   [`SymphoniaDecoder::fill_pending`] is bounded, so it cannot spin without
//!   producing samples or ending the stream. See `tests/robustness.rs`.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::time::Duration;

use rustamp_core::audio::{
    AudioSpec, Confidence, Decoder, DecoderFactory, MediaSource, ProbeHint, ReadSeek,
};
use rustamp_core::error::{Error, Result};
use rustamp_core::meta::TrackMetadata;
use rustamp_core::state::StreamInfo;
use symphonia::core::audio::{AudioBufferRef, SampleBuffer};
use symphonia::core::codecs::{CODEC_TYPE_NULL, CodecParameters, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// Consecutive undecodable packets tolerated before giving up.
const MAX_CONSECUTIVE_ERRORS: u32 = 512;

/// Packets read in a row that yielded neither samples nor a counted skip before
/// [`SymphoniaDecoder::fill_pending`] gives up.
///
/// Every other exit from that loop is bounded (end of stream, a counted error,
/// or samples); this bounds the two `continue` arms (a packet of another track,
/// a buffer that decodes to zero frames) so a source that keeps handing out
/// packets we cannot use degrades to an error instead of spinning forever.
/// Generously above anything a real multi-track container interleaves between
/// two audio packets.
const MAX_NO_PROGRESS_PACKETS: u32 = 1 << 16;

/// Frames of decoder lead-in decoded and discarded after a seek.
const SEEK_PREROLL_FRAMES: u64 = 4096;

/// Upper bound on a seek target in frames (shared with the wave decoder).
///
/// [`Decoder::seek`] clamps the target to the track duration when that is known.
/// A stream without a duration (a container that reports no frame count after a
/// non-seekable probe, e.g. MP3 whose length could not be estimated) has no
/// clamp, so a hostile or simply wrong `Duration` reaches the frame conversion
/// directly. 2^53 is exact in `f64` and, at the lowest sample rate
/// [`AudioSpec`] can carry, still far inside `Duration`'s range; at 44.1 kHz it
/// is millions of years, so no real seek is affected.
pub(crate) const MAX_SEEK_FRAMES: u64 = 1 << 53;

const EXTENSIONS: &[&str] = &[
    "mp3", "mp2", "mp1", "flac", "ogg", "oga", "wav", "wave", "aif", "aiff", "aifc", "caf", "m4a",
    "m4b", "mp4", "aac", "mkv", "mka",
];

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Factory for the symphonia decoder.
#[derive(Debug, Default, Clone, Copy)]
pub struct SymphoniaDecoderFactory;

impl SymphoniaDecoderFactory {
    /// Create the factory.
    pub fn new() -> Self {
        SymphoniaDecoderFactory
    }
}

fn is_mpeg_audio_sync(h: &[u8]) -> bool {
    if h.len() < 4 || h[0] != 0xFF || (h[1] & 0xE0) != 0xE0 {
        return false;
    }
    let version = (h[1] >> 3) & 3;
    let layer = (h[1] >> 1) & 3;
    let bitrate = h[2] >> 4;
    let rate = (h[2] >> 2) & 3;
    version != 1 && layer != 0 && bitrate != 15 && rate != 3
}

fn is_adts_sync(h: &[u8]) -> bool {
    h.len() >= 2 && h[0] == 0xFF && (h[1] & 0xF6) == 0xF0
}

/// Whether the header bytes carry a signature of a format symphonia handles.
fn sniff_content(h: &[u8]) -> bool {
    h.starts_with(b"ID3")
        || h.starts_with(b"fLaC")
        || h.starts_with(b"OggS")
        || h.starts_with(b"caff")
        || h.starts_with(&[0x1A, 0x45, 0xDF, 0xA3])
        || (h.len() >= 12 && &h[0..4] == b"RIFF" && &h[8..12] == b"WAVE")
        || (h.len() >= 12 && &h[0..4] == b"FORM" && (&h[8..12] == b"AIFF" || &h[8..12] == b"AIFC"))
        || (h.len() >= 8 && &h[4..8] == b"ftyp")
        || is_adts_sync(h)
        || is_mpeg_audio_sync(h)
}

impl DecoderFactory for SymphoniaDecoderFactory {
    fn name(&self) -> &str {
        "symphonia"
    }

    fn extensions(&self) -> &[&str] {
        EXTENSIONS
    }

    fn probe(&self, hint: &ProbeHint<'_>) -> Confidence {
        if sniff_content(hint.header) {
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
        Ok(Box::new(SymphoniaDecoder::open(source)?))
    }
}

// ---------------------------------------------------------------------------
// Byte source adapter
// ---------------------------------------------------------------------------

/// Adapts a [`ReadSeek`] reader to symphonia's `MediaSource` trait.
///
/// The reader is held directly and needs no interior synchronisation: the
/// `Send + Sync` requirement of [`ReadSeek`] (RFC 0002, C2) is what symphonia's
/// `MediaSource` asks for, and `ReaderSource` inherits both auto traits from
/// `Box<dyn ReadSeek>`. That bound — not a lock — carries the thread-safety
/// guarantee; the tests at the bottom of this file pin it.
struct ReaderSource {
    inner: Box<dyn ReadSeek>,
    len: Option<u64>,
}

impl Read for ReaderSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Seek for ReaderSource {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl symphonia::core::io::MediaSource for ReaderSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

/// Maps an error raised while **opening**: the container was not recognized, or
/// it was recognized but this build has no codec for it.
///
/// `Unsupported` at this point is the one place where [`Error::UnsupportedFormat`]
/// is the honest kind: nothing was opened, so "this is not something I handle"
/// is what happened. (A container we can read but whose codec we cannot is
/// folded into the same kind; telling those two apart needs a new error kind.)
fn open_err(e: SymError) -> Error {
    match e {
        SymError::IoError(e) => Error::Io(e),
        SymError::Unsupported(m) => Error::UnsupportedFormat(m.to_owned()),
        other => Error::Decode(other.to_string()),
    }
}

/// Maps an error raised **after** a decoder exists (reading or seeking).
///
/// `Unsupported` from the container or the codec at this point does not mean
/// "not my format": the container was already accepted, so a caller that saw
/// [`Error::UnsupportedFormat`] would be told the wrong thing. Such an error is
/// reported as damage instead, keeping the original message.
fn stream_err(e: SymError) -> Error {
    match e {
        SymError::IoError(e) => Error::Io(e),
        SymError::Unsupported(m) => Error::Decode(format!("mid-stream unsupported input: {m}")),
        other => Error::Decode(other.to_string()),
    }
}

/// Frame index of `at` at `rate` Hz, saturating instead of overflowing.
///
/// Shared with the wave decoder, which needs the same saturating conversion.
///
/// `rate` is a sample rate (`<= u32::MAX`) and `subsec_nanos() < 1e9`, so only
/// the seconds term can overflow `u64` — which it does for the hostile
/// `Duration::MAX` a caller can pass to a stream whose duration is unknown.
/// A plain multiplication panics in a debug build and wraps in a release build;
/// both are wrong, one of them loudly so.
pub(crate) fn frames_at(rate: u64, at: Duration) -> u64 {
    let secs = at.as_secs().saturating_mul(rate);
    let frac = u64::from(at.subsec_nanos()).saturating_mul(rate) / 1_000_000_000;
    secs.saturating_add(frac)
}

/// The beginning of a file, as the header parse needs it.
struct HeaderProbe {
    /// The length of the ID3v2 tag at the start of the file (0 when there is
    /// none).
    id3v2: usize,
    /// The bytes behind that tag.
    window: Vec<u8>,
    /// The AIFF header, when the file is an AIFF file.
    ///
    /// The AIFF chunks are read from the file itself: the `COMM` and `SSND`
    /// chunks state the payload, and they are not always inside the header
    /// window.
    aiff: Option<crate::aiff_header::AiffHeader>,
    /// What an MP4 file states about its payload, when the file is an MP4
    /// file.
    ///
    /// The movie box can be at the end of the file and larger than the header
    /// window, so the boxes are read from the file itself.
    mp4: Option<crate::mp4_header::Mp4Header>,
    /// What the metadata of a FLAC file says, when the file is a FLAC file.
    ///
    /// The chain of metadata blocks states the length of every block, so it
    /// tells a cut file from a complete one - and it holds the
    /// `STREAMINFO` block, the one that states the audio parameters. The
    /// chain is read from the file itself: a block can be larger than the
    /// header window.
    flac: Option<crate::flac_header::FlacMetadata>,
}

/// The byte claim of a container: how many bytes the payload occupies and how
/// many the file holds.
#[derive(Debug, Clone, Copy)]
struct ByteClaim {
    /// The number of bytes the container declares.
    declared: u64,
    /// The number of bytes the file actually holds.
    available: u64,
}

/// The payload a container declares.
///
/// The declared frame count is the oracle for a cut file only where the
/// container *states* it exactly. A RIFF/WAVE `data` chunk states the number
/// of samples it holds, and raw PCM has no unit that can be lost: a damaged
/// file changes the samples, not the frame count, so a shortfall of frames is
/// a cut file.
///
/// MPEG audio is different: a damaged MPEG frame is resynchronised by the
/// reader, so a damaged file delivers fewer frames than it declares too. The
/// Xing/Info tag states the byte count as well, and that is what tells the
/// two apart - a cut file is short of bytes, a damaged file is short of
/// frames only.
///
/// AIFF states both counts as well: the `COMM` chunk states the number of
/// sample frames, the `SSND` chunk the number of bytes of sound data. The
/// reader counts the eight bytes of the `SSND` header as audio, so its own
/// frame count is not the count of the file (see `aiff_header`); the count
/// the file states is the one to compare with.
///
/// MP4 states both counts as well: the media header states the duration of
/// the track in units of the media timescale, and the sample tables state
/// where the samples of the track are and how many bytes they occupy. The
/// duration is a frame count only when the media timescale is the sample
/// rate, and the reader's own count has to be the one the media header
/// states - otherwise the two counts are about different tracks.
///
/// Every other container this crate reads either estimates the count (MPEG
/// audio without a Xing/Info tag), counts it differently from what it
/// delivers (the AIFF reader includes the `SSND` header in its frame count)
/// or can lose frames to damage without losing any (FLAC), so their count is
/// no evidence of a cut file.
///
/// FLAC is the case worth spelling out: its `STREAMINFO` block states the
/// number of samples exactly, and the reader takes its frame count from that
/// block, so the two agree without a correction. What the format does not
/// state is a byte count, and a damaged FLAC frame is dropped by the reader
/// (its CRC-16 does not match) while the reader resynchronises at the next
/// frame - so a corrupt file is short of samples exactly like a cut one,
/// while it holds all of its bytes. Without the byte count the two cannot be
/// told apart, so FLAC has no oracle at all. Its *metadata* is a different
/// matter: the chain of metadata blocks states the length of every block and
/// is checked when the file is opened (see [`flac_refusal`]).
#[derive(Debug, Clone, Copy)]
struct Declared {
    /// The number of frames the container declares the file holds.
    frames: u64,
    /// The byte claim, when the container makes one.
    ///
    /// `None` when the container states no byte count: then a shortfall of
    /// frames is a cut file on its own.
    bytes: Option<ByteClaim>,
    /// What states the frame count, named in the failure message.
    source: &'static str,
}

/// The payload the container declares, if it states one exactly.
///
/// `header` is the beginning of the file, `params` the codec parameters the
/// container reader reported and `byte_len` the length of the file.
fn declared_payload(
    header: &HeaderProbe,
    params: &CodecParameters,
    byte_len: Option<u64>,
) -> Option<Declared> {
    if crate::wave_dec::is_riff_wave(&header.window) {
        // The `data` chunk states the number of samples it holds. Damaged
        // samples are still samples, so a shortfall of frames can only mean
        // that the file was cut.
        return params.n_frames.map(|frames| Declared {
            frames,
            bytes: None,
            source: "its container",
        });
    }
    // FLAC states no byte count of its audio, so it is no oracle: the
    // sample count of its `STREAMINFO` block is short in a damaged file just
    // as it is in a cut one (see the module docs). The check is explicit
    // here, so that no other header of a FLAC file can make one up.
    if header.flac.is_some() {
        return None;
    }
    // AIFF: the `COMM` chunk states the number of sample frames and the
    // `SSND` chunk states the number of bytes of sound data. Both are exact,
    // so a file that is short of frames *and* of bytes was cut.
    if let Some(aiff) = &header.aiff {
        // The reader counts the eight bytes of the `SSND` header as audio, so
        // its frame count is the one the correction is about. It has to be the
        // count this parse expects: otherwise the reader read another chunk
        // and the two counts are about different things.
        if params.n_frames != Some(aiff.reader_frames()) {
            return None;
        }
        // The byte count is what tells a cut file from a damaged one, so a
        // file whose length is unknown has no oracle.
        let byte_len = byte_len?;
        return Some(Declared {
            frames: aiff.declared_frames(),
            bytes: Some(ByteClaim {
                declared: aiff.sound_bytes(),
                available: byte_len.saturating_sub(aiff.data_start()),
            }),
            source: "its COMM chunk",
        });
    }
    // MP4/M4A: the media header states the number of frames and the sample
    // tables state where the samples are and how many bytes they occupy.
    if let Some(mp4) = &header.mp4 {
        // The reader's own count has to be the one the media header states:
        // otherwise the two counts are about different tracks.
        let n_frames = params.n_frames?;
        let mut matching = mp4.tracks().iter().filter(|t| t.frames() == n_frames);
        let track = matching.next()?;
        if matching.next().is_some() {
            // Two sound tracks state the same number of frames: which one the
            // reader picked cannot be told apart.
            return None;
        }
        // The duration is a number of frames only when the media timescale is
        // the sample rate. Otherwise it is a number of timescale units, and
        // that is not what the reader delivers.
        if track.timescale() != params.sample_rate? {
            return None;
        }
        // The byte count is what tells a cut file from a damaged one, so a
        // file whose length is unknown has no oracle.
        let byte_len = byte_len?;
        return Some(Declared {
            frames: track.frames(),
            bytes: Some(ByteClaim {
                declared: track.data_bytes(),
                available: byte_len.saturating_sub(track.data_start()),
            }),
            source: "its media header",
        });
    }
    // MPEG audio: the Xing/Info tag states the frame count and the byte count
    // of the audio data.
    let xing = crate::mpeg_header::parse(header.id3v2, &header.window)?;
    let trim = u64::from(params.delay.unwrap_or(0)) + u64::from(params.padding.unwrap_or(0));
    let frames = u64::from(xing.frames)
        .saturating_mul(xing.samples_per_frame)
        .saturating_sub(trim);
    // The container reader states the same number when it read the same tag.
    // If it does not, it fell back to its own estimate - and an estimate is
    // not evidence of a cut file.
    if params.n_frames != Some(frames) {
        return None;
    }
    let byte_len = byte_len?;
    Some(Declared {
        frames,
        bytes: Some(ByteClaim {
            declared: u64::from(xing.bytes),
            available: byte_len.saturating_sub(xing.start),
        }),
        source: "its Xing/Info header",
    })
}

/// How many bytes of the file the header parse reads.
///
/// The Xing/Info tag sits behind the side information of the first MPEG
/// frame: the frame header, the side information and the tag itself.
const HEADER_WINDOW: usize = 512;

/// Reads the bytes behind an ID3v2 tag, with the length of that tag.
///
/// The tag is skipped, not read: it can be large (cover art) and the header
/// parse only needs the bytes behind it. The reader is left where it was
/// found.
///
/// `len` is the length of the file when it is known; it bounds the AIFF chunk
/// walk.
fn header_probe(reader: &mut dyn ReadSeek, len: Option<u64>) -> HeaderProbe {
    let mut head = [0u8; 10];
    if reader.read_exact(&mut head).is_err() {
        return HeaderProbe {
            id3v2: 0,
            window: Vec::new(),
            aiff: None,
            mp4: None,
            flac: None,
        };
    }
    let id3v2 = crate::mpeg_header::id3v2_len(&head);
    if reader.seek(SeekFrom::Start(id3v2 as u64)).is_err() {
        return HeaderProbe {
            id3v2,
            window: Vec::new(),
            aiff: None,
            mp4: None,
            flac: None,
        };
    }
    let mut window = vec![0u8; HEADER_WINDOW];
    let mut filled = 0;
    while filled < window.len() {
        match reader.read(&mut window[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    window.truncate(filled);
    // An AIFF file states its payload in its `COMM` and `SSND` chunks. They
    // are read from the file itself: the chunks before the sound data can be
    // larger than the header window.
    let aiff = crate::aiff_header::parse(reader, len);
    // An MP4 file states its payload in its media data box and in its sample
    // tables. They are read from the file itself: the movie box can be at
    // the end of the file and larger than the header window.
    let mp4 = if crate::mp4_header::looks_like_mp4(&window) {
        crate::mp4_header::parse(reader, len)
    } else {
        None
    };
    // A FLAC file states its audio parameters in its `STREAMINFO` block, and
    // its metadata chain states the length of every block. Both are read
    // from the file itself: a metadata block can be larger than the header
    // window.
    let flac = if crate::flac_header::looks_like_flac(&window) {
        crate::flac_header::parse(reader, id3v2 as u64, len)
    } else {
        None
    };
    HeaderProbe {
        id3v2,
        window,
        aiff,
        mp4,
        flac,
    }
}

/// The message for a file that ends before the media data box says it does.
///
/// The media data box of an MP4 file states how many bytes of media data the
/// file holds. A file that is shorter than that was cut. The container reader
/// scans the whole file before it plays anything, so it refuses such a file
/// with a bare "end of stream" - naming the cut is the point of the oracle.
fn mp4_truncation(header: &HeaderProbe, byte_len: Option<u64>) -> Option<String> {
    let (start, end) = header.mp4.as_ref()?.media_data()?;
    let byte_len = byte_len?;
    if byte_len >= end {
        return None;
    }
    Some(format!(
        "truncated stream: the file holds {} of the {} bytes its media data box declares",
        byte_len.saturating_sub(start),
        end - start
    ))
}

/// The message for a FLAC file that cannot be decoded, named.
///
/// A FLAC file states its audio parameters in the `STREAMINFO` block of its
/// metadata chain. A file without that block has nothing to decode: the
/// reader cannot recognise a frame without the parameters and runs into the
/// end of the file, which it reports as an end of stream - for a file that
/// is not at its end. The chain states the length of every block, so a file
/// that ends inside one was cut, and that is a truncation rather than an end
/// of stream.
///
/// `None` when the file is not a FLAC file or when its metadata says
/// everything a reader needs.
fn flac_refusal(header: &HeaderProbe) -> Option<String> {
    use crate::flac_header::FlacMetadata;
    match header.flac? {
        FlacMetadata::Complete { stream_info: true } => None,
        FlacMetadata::Complete { stream_info: false } => {
            Some("the FLAC file has no STREAMINFO block".to_string())
        }
        FlacMetadata::Cut {
            offset,
            claimed,
            available,
        } => {
            let detail = match (claimed, available) {
                (Some(claimed), Some(available)) => format!(
                    "the block at offset {offset} claims {claimed} bytes, the file holds {available}"
                ),
                (Some(claimed), None) => format!(
                    "the block at offset {offset} claims {claimed} bytes, which the file does not hold"
                ),
                (None, _) => format!("no block header at offset {offset}"),
            };
            Some(format!(
                "truncated stream: the FLAC file ends inside its metadata blocks ({detail})"
            ))
        }
    }
}

/// Queues a detected mid-stream format change.
///
/// `spec` is deliberately not switched here: RFC 0002 C3 has `spec()` report the
/// new format only once the change has been reported through `read`, and a
/// `read` that still writes frames of the old format must keep seeing it.
fn queue_format_change(pending: &mut Option<AudioSpec>, to: AudioSpec) {
    *pending = Some(to);
}

/// Applies a queued format change: switches `spec` and returns the error to
/// raise. Returns `None` when nothing is queued.
///
/// Called by `read` only when that call has written no frames, which is what
/// C3.2 requires of the failed call. The queue is cleared, so a change is
/// reported exactly once.
fn take_format_change(spec: &mut AudioSpec, pending: &mut Option<AudioSpec>) -> Option<Error> {
    let to = pending.take()?;
    let from = std::mem::replace(spec, to);
    Some(Error::FormatChanged { from, to })
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// Decoder backed by symphonia 0.5 with gapless trimming enabled.
pub struct SymphoniaDecoder {
    format: Box<dyn FormatReader>,
    codec: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    spec: AudioSpec,
    n_frames: Option<u64>,
    sample_buf: Option<SampleBuffer<f32>>,
    /// Interleaved samples decoded but not yet handed out.
    pending: Vec<f32>,
    pending_pos: usize,
    /// Frames to drop after a seek (to reach the exact target).
    seek_target: Option<u64>,
    /// A detected mid-stream format change, reported by the next `read` that
    /// writes no frames (RFC 0002, C3).
    pending_format_change: Option<AudioSpec>,
    /// Frames handed out since the last seek.
    delivered: u64,
    /// The payload the container states, when it states an exact one.
    ///
    /// `None` means "no oracle": either the container does not state a count
    /// or it states an estimate (see [`declared_payload`]).
    declared: Option<Declared>,
    /// A truncation found at the end of the stream.
    ///
    /// Reported by the next `read` that writes no frames, so the frames that
    /// were decoded before the cut are still delivered.
    truncation: Option<String>,
    skipped: u64,
    consecutive_errors: u32,
    finished: bool,
    meta: TrackMetadata,
    bitrate_kbps: Option<u32>,
}

impl SymphoniaDecoder {
    /// Open a decoder for `source` (`Path` or `Reader`).
    pub fn open(source: MediaSource) -> Result<Self> {
        let ext = source.extension();
        // The container's own header, read before the source is handed to
        // symphonia: it decides whether the payload the file declares is an
        // oracle for a cut file (see [`declared_payload`]).
        let (mss, byte_len, header) = match source {
            MediaSource::Url(_) => {
                return Err(Error::Unsupported(
                    "URL sources are not supported yet; network streaming comes in M4",
                ));
            }
            MediaSource::Path(p) => {
                let mut f = File::open(&p)?;
                let len = f.metadata().ok().map(|m| m.len());
                let header = header_probe(&mut f, len);
                // The file is the source: it starts at the beginning.
                f.seek(SeekFrom::Start(0))?;
                let mss = MediaSourceStream::new(Box::new(f), MediaSourceStreamOptions::default());
                (mss, len, header)
            }
            MediaSource::Reader { mut reader, .. } => {
                let len = reader.seek(SeekFrom::End(0)).ok();
                reader.seek(SeekFrom::Start(0))?;
                let header = header_probe(&mut *reader, len);
                // The reader is the source: it starts at the beginning.
                reader.seek(SeekFrom::Start(0))?;
                let src = ReaderSource { inner: reader, len };
                let mss =
                    MediaSourceStream::new(Box::new(src), MediaSourceStreamOptions::default());
                (mss, len, header)
            }
        };

        // An empty source can never hold a container. Naming that directly is
        // more useful to a caller than the probe's "no suitable format reader
        // found", and it is the one case where "the file itself is the problem"
        // is certain.
        if byte_len == Some(0) {
            return Err(Error::Decode("empty source: 0 bytes to decode".into()));
        }

        // A file that ends before the media data box of an MP4 file says it
        // does was cut. The container reader scans the whole file before it
        // plays anything, so it refuses such a file with a bare "end of
        // stream" - naming the cut is the point of the oracle.
        if let Some(message) = mp4_truncation(&header, byte_len) {
            return Err(Error::Decode(message));
        }

        // A FLAC file without a `STREAMINFO` block states no audio
        // parameters, and a FLAC file whose metadata chain was cut never
        // reaches its audio. The reader reports both as an end of stream,
        // which for a file that is not at its end names the wrong problem.
        if let Some(message) = flac_refusal(&header) {
            return Err(Error::Decode(message));
        }

        let mut hint = Hint::new();
        if let Some(e) = &ext {
            hint.with_extension(e);
        }
        let fmt_opts = FormatOptions {
            enable_gapless: true,
            ..FormatOptions::default()
        };
        let mut probed = symphonia::default::get_probe()
            .format(&hint, mss, &fmt_opts, &MetadataOptions::default())
            .map_err(open_err)?;

        let mut format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| Error::UnsupportedFormat("no decodable audio track".into()))?;
        let track_id = track.id;
        let params = track.codec_params.clone();
        let codec = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .map_err(open_err)?;

        // Container-level tags (e.g. ID3v2 ahead of the stream) first; the format's own
        // tags override.
        let mut meta = TrackMetadata::default();
        if let Some(m) = probed.metadata.get()
            && let Some(rev) = m.current()
        {
            apply_tags(&mut meta, rev);
        }
        if let Some(rev) = format.metadata().current() {
            apply_tags(&mut meta, rev);
        }

        let mut this = SymphoniaDecoder {
            format,
            codec,
            track_id,
            spec: AudioSpec::new(params.sample_rate.unwrap_or(0), 0),
            n_frames: params.n_frames,
            sample_buf: None,
            pending: Vec::new(),
            pending_pos: 0,
            seek_target: None,
            pending_format_change: None,
            delivered: 0,
            declared: declared_payload(&header, &params, byte_len),
            truncation: None,
            skipped: 0,
            consecutive_errors: 0,
            finished: false,
            meta,
            bitrate_kbps: None,
        };
        if let Some(ch) = params.channels {
            this.spec.channels = ch.count() as u16;
        }
        if this.spec.sample_rate == 0 || this.spec.channels == 0 {
            // Incomplete codec parameters: learn the spec from the first buffer.
            this.spec = AudioSpec::new(0, 0);
            if !this.fill_pending()? {
                return Err(Error::Decode("stream contains no audio".into()));
            }
        }

        if let (Some(len), Some(n)) = (byte_len, this.n_frames)
            && n > 0
            && this.spec.sample_rate > 0
        {
            let secs = n as f64 / f64::from(this.spec.sample_rate);
            this.bitrate_kbps = Some((len as f64 * 8.0 / secs / 1000.0).round() as u32);
        }
        this.meta.bitrate_kbps = this.bitrate_kbps;
        this.meta.duration_ms = this.duration().map(|d| d.as_millis() as u64);
        Ok(this)
    }

    /// Number of corrupt packets skipped so far.
    pub fn skipped_packets(&self) -> u64 {
        self.skipped
    }

    /// Decode packets until some samples are pending. `Ok(false)` at end of stream.
    ///
    /// Every iteration either produces samples, counts a skipped packet, or ends
    /// the stream. The two arms that do none of those (`continue` for a foreign
    /// track id, `continue` for an empty buffer) are bounded by
    /// [`MAX_NO_PROGRESS_PACKETS`], so the loop cannot spin on a source that
    /// keeps handing out packets without progress.
    fn fill_pending(&mut self) -> Result<bool> {
        if self.finished {
            return Ok(false);
        }
        let mut no_progress = 0u32;
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    self.finished = true;
                    self.note_truncation();
                    return Ok(false);
                }
                Err(SymError::ResetRequired) => {
                    // A container reset (chained Ogg) cannot be continued in
                    // place; RFC 0002 C3.4 says end the stream here rather than
                    // report a format change.
                    self.finished = true;
                    return Ok(false);
                }
                Err(SymError::DecodeError(_)) => {
                    self.note_skip()?;
                    continue;
                }
                Err(e) => return Err(stream_err(e)),
            };
            if packet.track_id() != self.track_id {
                no_progress += 1;
                self.note_no_progress(no_progress)?;
                continue;
            }
            let decoded = match self.codec.decode(&packet) {
                Ok(d) => d,
                Err(SymError::DecodeError(_) | SymError::IoError(_) | SymError::Unsupported(_)) => {
                    self.note_skip()?;
                    continue;
                }
                Err(SymError::ResetRequired) => {
                    self.finished = true;
                    return Ok(false);
                }
                Err(e) => return Err(stream_err(e)),
            };
            self.consecutive_errors = 0;
            if decoded.frames() == 0 {
                no_progress += 1;
                self.note_no_progress(no_progress)?;
                continue;
            }
            // After a seek, drop the frames before the target. Buffers are aligned by their
            // end timestamp because codecs with overlap (Vorbis, MP3, AAC) return short
            // buffers right after a reset.
            let mut skip = 0;
            if let Some(target) = self.seek_target {
                let end = packet.ts().saturating_add(packet.dur());
                let start = end.saturating_sub(decoded.frames() as u64);
                skip = target.saturating_sub(start) as usize;
                if end > target {
                    self.seek_target = None;
                }
            }
            match convert(
                decoded,
                &mut self.spec,
                &mut self.sample_buf,
                &mut self.pending,
                skip,
            ) {
                Converted::Samples => {}
                Converted::Unusable => {
                    // Nothing to hand out and nothing to convert; stop instead
                    // of looping on this source.
                    self.finished = true;
                    return Ok(false);
                }
                Converted::RateChanged(new_spec) => {
                    // RFC 0002 C3.1: switch the format, but report it from a
                    // `read` call that writes no frames (see `read`).
                    queue_format_change(&mut self.pending_format_change, new_spec);
                    self.finished = true;
                    return Ok(false);
                }
            }
            self.pending_pos = 0;
            if self.pending_pos < self.pending.len() {
                return Ok(true);
            }
        }
    }

    /// Records that the stream ended before the container's declared payload
    /// was delivered: the file was cut, not finished.
    ///
    /// Only a container that states an exact frame count can say this (see
    /// [`declared_payload`]); without that oracle the end of the stream stays
    /// an end of stream.
    fn note_truncation(&mut self) {
        let Some(declared) = &self.declared else {
            return;
        };
        if self.delivered >= declared.frames {
            return;
        }
        // The frames are short. That is a cut file only when the bytes are
        // missing as well: a damaged file delivers fewer frames than it
        // declares too, but it holds all of its bytes.
        if let Some(bytes) = &declared.bytes
            && bytes.available >= bytes.declared
        {
            return;
        }
        self.truncation = Some(match &declared.bytes {
            Some(bytes) => format!(
                "truncated stream: the file ends after {} of the {} frames {} declares and holds {} of the {} bytes it declares",
                self.delivered, declared.frames, declared.source, bytes.available, bytes.declared
            ),
            None => format!(
                "truncated stream: the file ends after {} of the {} frames {} declares",
                self.delivered, declared.frames, declared.source
            ),
        });
    }

    fn note_skip(&mut self) -> Result<()> {
        self.skipped += 1;
        self.consecutive_errors += 1;
        if self.consecutive_errors > MAX_CONSECUTIVE_ERRORS {
            return Err(Error::Decode("too many consecutive corrupt packets".into()));
        }
        Ok(())
    }

    /// Counts an iteration of [`SymphoniaDecoder::fill_pending`] that produced
    /// neither samples nor a counted skip, and fails once a source keeps
    /// producing such packets: without this the loop would never reach an end of
    /// stream and `read` would hang.
    fn note_no_progress(&mut self, count: u32) -> Result<()> {
        if count > MAX_NO_PROGRESS_PACKETS {
            return Err(Error::Decode(
                "no audio produced after many packets without progress".into(),
            ));
        }
        Ok(())
    }
}

impl Decoder for SymphoniaDecoder {
    fn spec(&self) -> AudioSpec {
        self.spec
    }

    /// Forwards to the inherent method so that a `Box<dyn Decoder>` sees the
    /// real count instead of the trait default. Without this override the
    /// engine always reported zero skipped packets.
    fn skipped_packets(&self) -> u64 {
        self.skipped
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        let ch = usize::from(self.spec.channels.max(1));
        let want = out.len() / ch * ch;
        let mut written = 0;
        while written < want {
            if self.pending_pos >= self.pending.len() && !self.fill_pending()? {
                break;
            }
            let n = (self.pending.len() - self.pending_pos).min(want - written);
            out[written..written + n]
                .copy_from_slice(&self.pending[self.pending_pos..self.pending_pos + n]);
            self.pending_pos += n;
            written += n;
            // Counted here, not after the loop: the end of the stream is
            // detected inside the loop, and it has to see the frames this
            // call already handed out.
            self.delivered += (n / ch) as u64;
        }
        if written == 0
            && let Some(message) = &self.truncation
        {
            // The file was cut: the frames it holds were delivered, and the
            // failure comes now, in a call that writes no frames.
            return Err(Error::Decode(message.clone()));
        }
        if written == 0
            && let Some(e) = take_format_change(&mut self.spec, &mut self.pending_format_change)
        {
            // RFC 0002 C3.2: the failed call reports no frames. `spec()` already
            // reports the new format, and the next call continues in it.
            self.pending.clear();
            self.pending_pos = 0;
            self.finished = false;
            return Err(e);
        }
        Ok(written / ch)
    }

    fn seek(&mut self, to: Duration) -> Result<Duration> {
        let rate = u64::from(self.spec.sample_rate.max(1));
        let mut target = to;
        if let Some(d) = self.duration() {
            target = target.min(d);
        }
        // A stream without a known duration has no clamp above, so the target is
        // bounded here instead: `frames_at` saturates and MAX_SEEK_FRAMES keeps
        // the frame count convertible back to a `Duration`.
        let target_frames = frames_at(rate, target).min(MAX_SEEK_FRAMES);
        // Seek slightly early and discard the lead-in: overlapping transforms (MP3, Vorbis,
        // AAC) need previous blocks to reconstruct the first samples exactly.
        let pre = target_frames.min(SEEK_PREROLL_FRAMES);
        let from = target_frames - pre;
        // Half a frame of slack so the time-to-timestamp conversion cannot round down.
        let frac = ((from % rate) as f64 + 0.5) / rate as f64;
        let time = Time::new(from / rate, frac);
        let seeked = self
            .format
            .seek(
                SeekMode::Accurate,
                SeekTo::Time {
                    time,
                    track_id: Some(self.track_id),
                },
            )
            .map_err(|e| match e {
                SymError::Unsupported(_) | SymError::SeekError(_) => {
                    Error::Unsupported("seek not supported for this stream")
                }
                other => stream_err(other),
            })?;
        self.codec.reset();
        self.pending.clear();
        self.pending_pos = 0;
        self.finished = false;
        self.consecutive_errors = 0;
        // A seek restarts the packet walk, so a change detected before it is
        // stale; the format of the reopened position is what matters.
        self.pending_format_change = None;
        self.seek_target = Some(seeked.required_ts + pre);
        // After a seek the stream is read from the target, so the frames still
        // to come are counted from there and the declared count is what is
        // left of it. A seek that lands before the target only makes the
        // declared count larger, which cannot turn a complete file into a
        // truncated one.
        self.delivered = 0;
        self.truncation = None;
        if let Some(declared) = &mut self.declared {
            let target = seeked.required_ts.saturating_add(pre);
            declared.frames = declared.frames.saturating_sub(target);
        }
        Ok(self.spec.duration_of(target_frames))
    }

    fn duration(&self) -> Option<Duration> {
        let n = self.n_frames?;
        (self.spec.sample_rate > 0).then(|| self.spec.duration_of(n))
    }

    fn metadata(&self) -> TrackMetadata {
        self.meta.clone()
    }

    fn stream_info(&self) -> StreamInfo {
        StreamInfo {
            bitrate_kbps: self.bitrate_kbps,
            sample_rate_hz: self.spec.sample_rate,
            channels: self.spec.channels,
        }
    }

    fn is_seekable(&self) -> bool {
        true
    }
}

/// What [`convert`] did with one decoded buffer.
enum Converted {
    /// The samples are in `pending`, in the decoder's current spec.
    Samples,
    /// The buffer could not be converted (the sample buffer could not be
    /// created); the stream ends here.
    Unusable,
    /// The buffer's sample rate differs from the decoder's spec: the caller must
    /// switch to this spec and report it (RFC 0002, C3).
    RateChanged(AudioSpec),
}

/// Convert a decoded buffer into interleaved `pending` samples of the opened spec.
///
/// A buffer whose sample count differs is remapped to the opened channel count
/// (mono is duplicated, extra channels dropped, missing channels zero-filled);
/// a buffer whose sample **rate** differs is reported back to the caller so it
/// can switch specs and signal the change instead of silently ending the stream.
fn convert(
    decoded: AudioBufferRef<'_>,
    spec: &mut AudioSpec,
    sample_buf: &mut Option<SampleBuffer<f32>>,
    pending: &mut Vec<f32>,
    skip: usize,
) -> Converted {
    let sig = *decoded.spec();
    let ch = sig.channels.count();
    if spec.sample_rate == 0 {
        *spec = AudioSpec::new(sig.rate, ch as u16);
    } else if sig.rate != spec.sample_rate {
        return Converted::RateChanged(AudioSpec::new(sig.rate, ch as u16));
    }
    let frames = decoded.frames();
    if sample_buf
        .as_ref()
        .is_none_or(|b| b.capacity() < frames * ch)
    {
        *sample_buf = Some(SampleBuffer::new(frames as u64, sig));
    }
    let Some(buf) = sample_buf.as_mut() else {
        // Unreachable: the buffer is created just above. Reporting it as
        // unusable rather than panicking keeps a hostile input from reaching a
        // `panic!` inside the crate.
        return Converted::Unusable;
    };
    buf.copy_interleaved_ref(decoded);

    pending.clear();
    // `.max(1)` is defensive: `read` and `AudioSpec::samples_for` already guard
    // a zero channel count, but `chunks_exact(0)` would panic here.
    let out_ch = usize::from(spec.channels.max(1));
    let skip = skip.min(frames);
    let samples = &buf.samples()[skip * ch..];
    if ch == out_ch {
        pending.extend_from_slice(samples);
    } else {
        for frame in samples.chunks_exact(ch) {
            for c in 0..out_ch {
                pending.push(if ch == 1 {
                    frame[0]
                } else {
                    frame.get(c).copied().unwrap_or(0.0)
                });
            }
        }
    }
    Converted::Samples
}

// ---------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------

/// `"3/12"`-style strings: the leading number and the optional total.
fn parse_pair(s: &str) -> (Option<u32>, Option<u32>) {
    let mut it = s.split('/');
    let a = it.next().and_then(|x| x.trim().parse().ok());
    let b = it.next().and_then(|x| x.trim().parse().ok());
    (a, b)
}

fn parse_year(s: &str) -> Option<u32> {
    let t = s.trim();
    t.get(..4).and_then(|y| y.parse().ok())
}

fn apply_tags(m: &mut TrackMetadata, rev: &MetadataRevision) {
    use StandardTagKey as K;
    for tag in rev.tags() {
        let v = tag.value.to_string();
        if v.is_empty() {
            continue;
        }
        match tag.std_key {
            Some(K::TrackTitle) => m.title = Some(v),
            Some(K::Artist) => m.artist = Some(v),
            Some(K::Album) => m.album = Some(v),
            Some(K::AlbumArtist) => m.album_artist = Some(v),
            Some(K::Date | K::ReleaseDate | K::OriginalDate) => {
                if let Some(y) = parse_year(&v) {
                    m.year = Some(y);
                }
            }
            Some(K::Genre) => m.genre = Some(v),
            Some(K::Comment) => m.comment = Some(v),
            // ffmpeg and some taggers write the comment as DESCRIPTION.
            Some(K::Description) => {
                m.comment.get_or_insert(v);
            }
            Some(K::TrackNumber) => {
                let (n, t) = parse_pair(&v);
                m.track_no = n.or(m.track_no);
                m.track_total = t.or(m.track_total);
            }
            Some(K::TrackTotal) => m.track_total = parse_pair(&v).0.or(m.track_total),
            Some(K::DiscNumber) => m.disc_no = parse_pair(&v).0.or(m.disc_no),
            _ => {
                m.extra.insert(tag.key.to_ascii_uppercase(), v);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use std::io::Cursor;
    use symphonia::core::audio::{AudioBuffer, Channels, Signal, SignalSpec};
    use symphonia::core::io::MediaSource as _;

    /// Compile-time proof for the `Send + Sync` requirement of symphonia's
    /// `MediaSource` (RFC 0002, C2). `ReaderSource` satisfies it through
    /// `Box<dyn ReadSeek>`, because `ReadSeek` itself requires `Send + Sync`;
    /// nothing else contributes to it.
    fn assert_send_sync<T: Send + Sync>() {}

    fn reader_source(len: usize) -> ReaderSource {
        let bytes: Vec<u8> = (0..len).map(|i| i as u8).collect();
        ReaderSource {
            inner: Box::new(Cursor::new(bytes)),
            len: Some(len as u64),
        }
    }

    #[test]
    fn reader_source_is_send_and_sync() {
        assert_send_sync::<ReaderSource>();
        // The boxed trait object the wrapper holds carries the same bounds.
        assert_send_sync::<Box<dyn ReadSeek>>();
    }

    /// The shared-`&` case: several threads use the same wrapper at the same
    /// time. This is the guarantee the wrapper's `Sync` bound has to give, and
    /// the test stops compiling if `ReaderSource` (or the `ReadSeek` bound)
    /// loses `Sync`.
    #[test]
    fn reader_source_is_shared_by_several_threads_at_once() {
        let src = reader_source(256);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    assert!(src.is_seekable());
                    assert_eq!(src.byte_len(), Some(256));
                });
            }
        });
    }

    /// The other half, `Send`: the wrapper is moved to another thread and read
    /// and seeked there.
    #[test]
    fn reader_source_is_moved_to_another_thread() {
        let mut src = reader_source(256);
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8];
            src.read_exact(&mut buf).expect("read");
            assert_eq!(src.seek(SeekFrom::End(0)).expect("seek"), 256);
            buf
        });
        assert_eq!(
            handle.join().expect("decode thread"),
            [0, 1, 2, 3, 4, 5, 6, 7]
        );
    }

    // -----------------------------------------------------------------------
    // B2: hostile input, error classification and format changes
    // -----------------------------------------------------------------------

    /// The declared payload is the oracle for a cut file only where the
    /// container states it exactly: a RIFF/WAVE `data` chunk does, the other
    /// containers do not.
    #[test]
    fn only_containers_that_state_their_payload_are_checked() {
        let mut wave = b"RIFF".to_vec();
        wave.extend_from_slice(&0u32.to_le_bytes());
        wave.extend_from_slice(b"WAVEfmt ");
        let mut params = CodecParameters::new();
        params.with_n_frames(8000);
        let declared = declared_payload(
            &HeaderProbe {
                id3v2: 0,
                window: wave.clone(),
                aiff: None,
                mp4: None,
                flac: None,
            },
            &params,
            Some(4096),
        )
        .expect("a wav states its payload");
        assert_eq!(declared.frames, 8000);
        assert!(
            declared.bytes.is_none(),
            "a wav states no byte count: a damaged sample is still a sample"
        );

        // A RIFF container that is not WAVE is not a WAV file.
        let mut avi = b"RIFF".to_vec();
        avi.extend_from_slice(&0u32.to_le_bytes());
        avi.extend_from_slice(b"AVI LIST");
        assert!(
            declared_payload(
                &HeaderProbe {
                    id3v2: 0,
                    window: avi,
                    aiff: None,
                    mp4: None,
                    flac: None,
                },
                &params,
                Some(4096)
            )
            .is_none()
        );

        // A FLAC file states its frame count, but a damaged FLAC frame is
        // resynchronised by the reader, so its count is no evidence of a cut
        // file.
        assert!(
            declared_payload(
                &HeaderProbe {
                    id3v2: 0,
                    window: b"fLaC\0\0\0\0\0\0\0\0".to_vec(),
                    aiff: None,
                    mp4: None,
                    flac: Some(crate::flac_header::FlacMetadata::Complete { stream_info: true }),
                },
                &params,
                Some(4096)
            )
            .is_none()
        );
    }

    /// A FLAC file is no oracle, whatever its bytes happen to look like.
    ///
    /// The header of a FLAC file can hold anything - a false MPEG sync, a
    /// "Xing" tag - and none of it states the length of the audio. The
    /// check is explicit, so that no header of a FLAC file can make one up.
    #[test]
    fn a_flac_file_is_no_oracle_even_when_its_window_wears_a_xing_tag() {
        // A window that starts with the FLAC marker and carries a Xing tag
        // behind a MPEG frame header.
        let mut window = b"fLaC".to_vec();
        window.extend_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
        window.extend_from_slice(&[0u8; 32]);
        window.extend_from_slice(b"Xing");
        window.extend_from_slice(&0x03u32.to_be_bytes());
        window.extend_from_slice(&59u32.to_be_bytes());
        window.extend_from_slice(&12_537u32.to_be_bytes());

        let mut params = CodecParameters::new();
        params.with_n_frames(59 * 1152 - 1105 - 713);
        let header = HeaderProbe {
            id3v2: 0,
            window,
            aiff: None,
            mp4: None,
            flac: Some(crate::flac_header::FlacMetadata::Complete { stream_info: true }),
        };
        assert!(
            declared_payload(&header, &params, Some(12_537)).is_none(),
            "a FLAC file states no byte count of its audio: it is no oracle"
        );
    }

    /// An MPEG file with a Xing/Info tag states its frame count *and* its byte
    /// count: that is the oracle for a cut MPEG file.
    #[test]
    fn an_mpeg_file_with_a_xing_header_states_its_payload() {
        // A MPEG 1 layer III stereo frame with a Xing tag behind its side
        // information: 59 frames of 1152 samples in 12_537 bytes.
        let mut window = vec![0xFF, 0xFB, 0x90, 0x00];
        window.extend_from_slice(&[0u8; 32]);
        window.extend_from_slice(b"Xing");
        window.extend_from_slice(&0x03u32.to_be_bytes());
        window.extend_from_slice(&59u32.to_be_bytes());
        window.extend_from_slice(&12_537u32.to_be_bytes());
        let header = HeaderProbe {
            id3v2: 0,
            window,
            aiff: None,
            mp4: None,
            flac: None,
        };

        let mut params = CodecParameters::new();
        params.with_n_frames(59 * 1152 - 1105 - 713);
        // The LAME extension of the tag states the encoder delay and padding.
        params.with_delay(1105);
        params.with_padding(713);
        let declared = declared_payload(&header, &params, Some(12_537))
            .expect("the tag states the payload of the file");
        assert_eq!(declared.frames, 59 * 1152 - 1105 - 713);
        let bytes = declared.bytes.expect("the tag states the byte count too");
        assert_eq!(bytes.declared, 12_537);
        assert_eq!(bytes.available, 12_537);

        // A reader that fell back to its own estimate states another number.
        // An estimate is not evidence of a cut file.
        let mut estimated = CodecParameters::new();
        estimated.with_n_frames(69_120);
        assert!(declared_payload(&header, &estimated, Some(12_537)).is_none());
    }

    /// A decoded-buffer stand-in with `frames` frames and `channels` channels,
    /// all silence. The first channel is what the `fill_*` helpers below write.
    fn buffer(rate: u32, channels: Channels, frames: usize) -> AudioBuffer<f32> {
        let mut b = AudioBuffer::<f32>::new(frames as u64, SignalSpec::new(rate, channels));
        b.render_reserved(Some(frames));
        b
    }

    fn as_ref(b: &AudioBuffer<f32>) -> AudioBufferRef<'_> {
        AudioBufferRef::F32(Cow::Borrowed(b))
    }

    /// The seek target is a plain product today; a caller-supplied
    /// `Duration::MAX` against a stream whose duration is unknown overflows it.
    #[test]
    fn frames_at_is_exact_and_saturates_at_the_extremes() {
        assert_eq!(frames_at(44_100, Duration::ZERO), 0);
        assert_eq!(frames_at(44_100, Duration::from_millis(1_500)), 66_150);
        assert_eq!(frames_at(8_000, Duration::from_millis(1_250)), 10_000);
        assert_eq!(frames_at(1, Duration::from_secs(3)), 3);
        // Hostile: must saturate, not overflow (which panics in a debug build)
        // and not wrap in release.
        assert_eq!(frames_at(44_100, Duration::MAX), u64::MAX);
        assert_eq!(frames_at(1, Duration::from_secs(u64::MAX)), u64::MAX);
        // What `seek` actually derives from that: bounded and convertible to a
        // `Duration`.
        assert_eq!(
            frames_at(44_100, Duration::MAX).min(MAX_SEEK_FRAMES),
            MAX_SEEK_FRAMES
        );
        assert!(
            frames_at(44_100, Duration::MAX).min(MAX_SEEK_FRAMES) < 1 << 54,
            "the capped target stays exact in f64"
        );
    }

    /// `open_err` is the "not my format" side, `stream_err` must not collapse a
    /// post-open `Unsupported` into it (that would claim the file is foreign).
    #[test]
    fn unsupported_input_is_classified_by_the_phase_it_happened_in() {
        let open = open_err(SymError::Unsupported("core (probe): nothing found"));
        assert!(matches!(open, Error::UnsupportedFormat(m) if m.contains("probe")));

        let stream = stream_err(SymError::Unsupported("mpa: free bit-rate"));
        assert!(matches!(stream, Error::Decode(m) if m.contains("free bit-rate")));

        // I/O keeps its own kind in both phases.
        for e in [
            open_err(SymError::IoError(io::Error::other("x"))),
            stream_err(SymError::IoError(io::Error::other("x"))),
        ] {
            assert!(matches!(e, Error::Io(_)), "{e:?}");
        }
    }

    /// RFC 0002 C3: the change must be reported through the error, and it must
    /// be `spec()` that names the new format — but only once the change has been
    /// reported, so a `read` that still writes old-format frames is consistent.
    #[test]
    fn a_queued_format_change_is_reported_exactly_once_and_switches_the_spec() {
        let mut spec = AudioSpec::new(44_100, 2);
        let mut pending = None;
        assert!(take_format_change(&mut spec, &mut pending).is_none());
        assert_eq!(spec, AudioSpec::new(44_100, 2));

        queue_format_change(&mut pending, AudioSpec::new(48_000, 1));
        assert_eq!(
            spec,
            AudioSpec::new(44_100, 2),
            "queuing must not switch yet"
        );
        match take_format_change(&mut spec, &mut pending) {
            Some(Error::FormatChanged { from, to }) => {
                assert_eq!(from, AudioSpec::new(44_100, 2));
                assert_eq!(to, AudioSpec::new(48_000, 1));
            }
            other => panic!("expected FormatChanged, got {other:?}"),
        }
        assert_eq!(spec, AudioSpec::new(48_000, 1));
        assert!(
            take_format_change(&mut spec, &mut pending).is_none(),
            "reported once, not on every read"
        );
        assert_eq!(spec, AudioSpec::new(48_000, 1));
    }

    /// A mid-stream sample-rate change is a *change*, not an end of stream: the
    /// caller gets the new spec back and can keep reading (RFC 0002, C3.1).
    #[test]
    fn convert_reports_a_sample_rate_change_instead_of_ending_the_stream() {
        let mut spec = AudioSpec::new(44_100, 2);
        let mut sample_buf = None;
        let mut pending = vec![9.0f32];
        let buf = buffer(48_000, Channels::FRONT_LEFT | Channels::FRONT_RIGHT, 4);

        let out = convert(as_ref(&buf), &mut spec, &mut sample_buf, &mut pending, 0);
        assert!(
            matches!(out, Converted::RateChanged(s) if s == AudioSpec::new(48_000, 2)),
            "expected the new spec back"
        );
        assert_eq!(
            spec,
            AudioSpec::new(44_100, 2),
            "caller switches, not convert"
        );
        assert_eq!(pending, vec![9.0f32], "no frames are added by a change");
    }

    /// The channel remap the module doc promises: mono is duplicated to the
    /// opened count, extra channels are dropped, missing ones zero-filled.
    #[test]
    fn convert_remaps_the_channel_count_to_the_opened_spec() {
        // mono buffer, stereo decoder: every sample appears twice.
        let mut spec = AudioSpec::new(44_100, 2);
        let mut sample_buf = None;
        let mut pending = Vec::new();
        let mut mono = buffer(44_100, Channels::FRONT_LEFT, 3);
        mono.chan_mut(0).copy_from_slice(&[1.0, 2.0, 3.0]);
        assert!(matches!(
            convert(as_ref(&mono), &mut spec, &mut sample_buf, &mut pending, 0),
            Converted::Samples
        ));
        assert_eq!(pending, vec![1.0, 1.0, 2.0, 2.0, 3.0, 3.0]);

        // stereo buffer, mono decoder: the extra channel is dropped.
        let mut spec = AudioSpec::new(44_100, 1);
        let mut sample_buf = None;
        let mut pending = Vec::new();
        let mut stereo = buffer(44_100, Channels::FRONT_LEFT | Channels::FRONT_RIGHT, 2);
        stereo.chan_mut(0).copy_from_slice(&[1.0, 2.0]);
        stereo.chan_mut(1).copy_from_slice(&[-1.0, -2.0]);
        assert!(matches!(
            convert(as_ref(&stereo), &mut spec, &mut sample_buf, &mut pending, 0),
            Converted::Samples
        ));
        assert_eq!(pending, vec![1.0, 2.0]);
    }

    /// The seek lead-in is dropped from the buffer that carries the target.
    #[test]
    fn convert_drops_the_requested_lead_in_frames() {
        let mut spec = AudioSpec::new(44_100, 1);
        let mut sample_buf = None;
        let mut pending = Vec::new();
        let mut b = buffer(44_100, Channels::FRONT_LEFT, 4);
        b.chan_mut(0).copy_from_slice(&[1.0, 2.0, 3.0, 4.0]);
        assert!(matches!(
            convert(as_ref(&b), &mut spec, &mut sample_buf, &mut pending, 3),
            Converted::Samples
        ));
        assert_eq!(pending, vec![4.0]);
        // A skip beyond the buffer cannot panic; it yields nothing.
        assert!(matches!(
            convert(as_ref(&b), &mut spec, &mut sample_buf, &mut pending, 99),
            Converted::Samples
        ));
        assert!(pending.is_empty());
    }

    /// Incomplete codec parameters (rate unknown at open time) are learned from
    /// the first buffer instead of leaving a zero spec behind.
    #[test]
    fn convert_learns_the_spec_from_the_first_buffer() {
        let mut spec = AudioSpec::new(0, 0);
        let mut sample_buf = None;
        let mut pending = Vec::new();
        let mut b = buffer(8_000, Channels::FRONT_LEFT, 2);
        b.chan_mut(0).copy_from_slice(&[0.5, -0.5]);
        assert!(matches!(
            convert(as_ref(&b), &mut spec, &mut sample_buf, &mut pending, 0),
            Converted::Samples
        ));
        assert_eq!(spec, AudioSpec::new(8_000, 1));
        assert_eq!(pending, vec![0.5, -0.5]);
    }
}
