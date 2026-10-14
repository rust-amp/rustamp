//! Symphonia-backed decoder.
//!
//! Behaviour the core contract leaves open (also in `CONTRACT_NOTES.md`):
//!
//! * `spec()` is fixed at open time from the codec parameters (or the first
//!   decoded buffer when those are incomplete).
//! * Mid-stream **channel count** change: audio is remapped to the opened
//!   channel count (mono is duplicated, extra channels dropped, missing ones
//!   zero-filled).
//! * Mid-stream **sample rate** change or a container reset (chained Ogg):
//!   the stream ends (`read` returns `Ok(0)`); the engine can open the rest
//!   as a new track if it wants chaining.
//! * Corrupt packets are skipped; see [`SymphoniaDecoder::skipped_packets`].

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
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey};
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

/// Consecutive undecodable packets tolerated before giving up.
const MAX_CONSECUTIVE_ERRORS: u32 = 512;

/// Frames of decoder lead-in decoded and discarded after a seek.
const SEEK_PREROLL_FRAMES: u64 = 4096;

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
/// `ReadSeek` requires `Send + Sync` (RFC 0002, C2), which is exactly what
/// symphonia's `MediaSource` needs, so the reader can be held directly; the
/// former `Mutex` wrapper is gone.
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

fn dec_err(e: SymError) -> Error {
    match e {
        SymError::IoError(e) => Error::Io(e),
        SymError::Unsupported(m) => Error::UnsupportedFormat(m.to_owned()),
        other => Error::Decode(other.to_string()),
    }
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
        let (mss, byte_len) = match source {
            MediaSource::Url(_) => {
                return Err(Error::Unsupported(
                    "URL sources are not supported yet; network streaming comes in M4",
                ));
            }
            MediaSource::Path(p) => {
                let f = File::open(&p)?;
                let len = f.metadata().ok().map(|m| m.len());
                let mss = MediaSourceStream::new(Box::new(f), MediaSourceStreamOptions::default());
                (mss, len)
            }
            MediaSource::Reader { mut reader, .. } => {
                let len = reader.seek(SeekFrom::End(0)).ok();
                reader.seek(SeekFrom::Start(0))?;
                let src = ReaderSource { inner: reader, len };
                let mss =
                    MediaSourceStream::new(Box::new(src), MediaSourceStreamOptions::default());
                (mss, len)
            }
        };

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
            .map_err(dec_err)?;

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
            .map_err(dec_err)?;

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
    fn fill_pending(&mut self) -> Result<bool> {
        if self.finished {
            return Ok(false);
        }
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    self.finished = true;
                    return Ok(false);
                }
                Err(SymError::ResetRequired) => {
                    self.finished = true;
                    return Ok(false);
                }
                Err(SymError::DecodeError(_)) => {
                    self.note_skip()?;
                    continue;
                }
                Err(e) => return Err(dec_err(e)),
            };
            if packet.track_id() != self.track_id {
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
                Err(e) => return Err(dec_err(e)),
            };
            self.consecutive_errors = 0;
            if decoded.frames() == 0 {
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
            let ok = convert(
                decoded,
                &mut self.spec,
                &mut self.sample_buf,
                &mut self.pending,
                skip,
            );
            self.pending_pos = 0;
            if !ok {
                self.finished = true;
                return Ok(false);
            }
            if self.pending_pos < self.pending.len() {
                return Ok(true);
            }
        }
    }

    fn note_skip(&mut self) -> Result<()> {
        self.skipped += 1;
        self.consecutive_errors += 1;
        if self.consecutive_errors > MAX_CONSECUTIVE_ERRORS {
            return Err(Error::Decode("too many consecutive corrupt packets".into()));
        }
        Ok(())
    }
}

impl Decoder for SymphoniaDecoder {
    fn spec(&self) -> AudioSpec {
        self.spec
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
        }
        Ok(written / ch)
    }

    fn seek(&mut self, to: Duration) -> Result<Duration> {
        let rate = u64::from(self.spec.sample_rate.max(1));
        let mut target = to;
        if let Some(d) = self.duration() {
            target = target.min(d);
        }
        let target_frames =
            target.as_secs() * rate + u64::from(target.subsec_nanos()) * rate / 1_000_000_000;
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
                other => dec_err(other),
            })?;
        self.codec.reset();
        self.pending.clear();
        self.pending_pos = 0;
        self.finished = false;
        self.consecutive_errors = 0;
        self.seek_target = Some(seeked.required_ts + pre);
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

/// Convert a decoded buffer into interleaved `pending` samples of the opened spec.
/// Returns `false` on an unsupported mid-stream sample rate change.
fn convert(
    decoded: AudioBufferRef<'_>,
    spec: &mut AudioSpec,
    sample_buf: &mut Option<SampleBuffer<f32>>,
    pending: &mut Vec<f32>,
    skip: usize,
) -> bool {
    let sig = *decoded.spec();
    let ch = sig.channels.count();
    if spec.sample_rate == 0 {
        *spec = AudioSpec::new(sig.rate, ch as u16);
    } else if sig.rate != spec.sample_rate {
        return false;
    }
    let frames = decoded.frames();
    if sample_buf
        .as_ref()
        .is_none_or(|b| b.capacity() < frames * ch)
    {
        *sample_buf = Some(SampleBuffer::new(frames as u64, sig));
    }
    let Some(buf) = sample_buf.as_mut() else {
        return false;
    };
    buf.copy_interleaved_ref(decoded);

    pending.clear();
    let out_ch = usize::from(spec.channels);
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
    true
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
