//! Audio pipeline contracts: [`Decoder`], [`DecoderFactory`] +
//! [`DecoderRegistry`], [`OutputDevice`] and [`DspStage`].
//!
//! The pipeline is `Decoder -> [DspStage]* -> OutputDevice`, all in
//! interleaved `f32` frames. The engine (`rustamp-audio`) owns the glue
//! (resampling, ring buffer, vis tap, gapless). Implementations live in
//! `rustamp-decoders`, `rustamp-dsp` and `rustamp-output`.
//!
//! Terminology: a *frame* is one sample per channel; a buffer of `n` frames
//! has `n * channels` `f32` values.

use std::fmt;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::meta::TrackMetadata;
use crate::state::StreamInfo;

/// Sample rate and channel count of interleaved `f32` audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioSpec {
    /// Sample rate in Hz.
    pub sample_rate: u32,
    /// Number of interleaved channels (>= 1).
    pub channels: u16,
}

impl AudioSpec {
    /// Create a spec.
    pub const fn new(sample_rate: u32, channels: u16) -> Self {
        AudioSpec {
            sample_rate,
            channels,
        }
    }

    /// Number of frames in `samples` interleaved values.
    pub fn frames_in(&self, samples: usize) -> usize {
        samples / usize::from(self.channels.max(1))
    }

    /// Number of interleaved values for `frames` frames.
    pub fn samples_for(&self, frames: usize) -> usize {
        frames * usize::from(self.channels)
    }

    /// Convert a frame count to a duration.
    pub fn duration_of(&self, frames: u64) -> Duration {
        Duration::from_secs_f64(frames as f64 / f64::from(self.sample_rate.max(1)))
    }
}

// ---------------------------------------------------------------------------
// Media sources & decoders
// ---------------------------------------------------------------------------

/// A readable, seekable byte stream.
///
/// `Sync` is required as well (RFC 0002, C2): symphonia's `MediaSource` is
/// `Send + Sync`, and a plain `Send` bound forced the decoder crate to wrap
/// the reader in a `Mutex`. A reader that is not `Sync` has to be wrapped by
/// the caller.
pub trait ReadSeek: Read + Seek + Send + Sync {}
impl<T: Read + Seek + Send + Sync> ReadSeek for T {}

/// Where a decoder reads from.
pub enum MediaSource {
    /// A local file; the decoder opens it itself.
    Path(PathBuf),
    /// A network stream; the decoder (or an I/O layer) opens it.
    Url(String),
    /// An already-open byte stream (archives, tests, HTTP buffers).
    Reader {
        /// The stream, positioned at 0. `Send + Sync` (RFC 0002, C2).
        reader: Box<dyn ReadSeek>,
        /// File extension hint without dot (e.g. `"mp3"`).
        extension: Option<String>,
    },
}

impl fmt::Debug for MediaSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaSource::Path(p) => f.debug_tuple("Path").field(p).finish(),
            MediaSource::Url(u) => f.debug_tuple("Url").field(u).finish(),
            MediaSource::Reader { extension, .. } => f
                .debug_struct("Reader")
                .field("extension", extension)
                .finish_non_exhaustive(),
        }
    }
}

impl MediaSource {
    /// Lower-case extension hint without the dot, if any.
    pub fn extension(&self) -> Option<String> {
        let ext = match self {
            MediaSource::Path(p) => p.extension().map(|e| e.to_string_lossy().into_owned()),
            MediaSource::Url(u) => {
                let tail = u.split(['?', '#']).next().unwrap_or(u);
                let name = tail.rsplit('/').next().unwrap_or(tail);
                name.rsplit_once('.').map(|(_, e)| e.to_owned())
            }
            MediaSource::Reader { extension, .. } => extension.clone(),
        };
        ext.map(|e| e.to_ascii_lowercase())
    }

    /// Read up to `n` leading bytes for content sniffing without consuming
    /// the source (files are re-opened, readers are rewound). URLs yield an
    /// empty header.
    pub fn sniff(&mut self, n: usize) -> std::io::Result<Vec<u8>> {
        fn take(r: &mut dyn Read, n: usize) -> std::io::Result<Vec<u8>> {
            let mut buf = Vec::with_capacity(n);
            r.take(n as u64).read_to_end(&mut buf)?;
            Ok(buf)
        }
        match self {
            MediaSource::Path(p) => take(&mut std::fs::File::open(p)?, n),
            MediaSource::Url(_) => Ok(Vec::new()),
            MediaSource::Reader { reader, .. } => {
                let buf = take(reader, n)?;
                reader.seek(SeekFrom::Start(0))?;
                Ok(buf)
            }
        }
    }
}

/// A decoder producing interleaved `f32` PCM.
///
/// Decoders are `Send` (moved to the engine's decode thread) but need not be
/// `Sync`. `read` may block on I/O but must not block indefinitely on live
/// streams without returning data or an error.
pub trait Decoder: Send {
    /// Output format.
    ///
    /// Fixed for the lifetime of the decoder, with one exception: after `read`
    /// returned `Err(Error::FormatChanged { .. })` (RFC 0002, C3) this reports
    /// the new format and further `read` calls continue in it.
    fn spec(&self) -> AudioSpec;

    /// Fill `out` with whole frames; `out.len()` must be a multiple of
    /// `spec().channels`. Returns the number of **frames** written; `Ok(0)`
    /// means end of stream. May return fewer frames than requested.
    ///
    /// `Err(Error::FormatChanged { .. })` means the stream changed format
    /// (RFC 0002, C3): no frames were written, `spec()` reports the new format,
    /// and the caller may call `read` again to continue.
    fn read(&mut self, out: &mut [f32]) -> Result<usize>;

    /// Seek to `to`; returns the position actually reached (which may differ
    /// slightly for coarse formats). `Err(Error::Unsupported)` for live streams.
    fn seek(&mut self, to: Duration) -> Result<Duration>;

    /// Total duration if known.
    fn duration(&self) -> Option<Duration>;

    /// Metadata discovered by the decoder (tags, codec info).
    fn metadata(&self) -> TrackMetadata {
        TrackMetadata::default()
    }

    /// Technical info for the main window display.
    fn stream_info(&self) -> StreamInfo {
        let s = self.spec();
        StreamInfo {
            bitrate_kbps: None,
            sample_rate_hz: s.sample_rate,
            channels: s.channels,
        }
    }

    /// Whether [`Decoder::seek`] is expected to work.
    fn is_seekable(&self) -> bool {
        self.duration().is_some()
    }

    /// Number of packets/units skipped because they could not be decoded since
    /// the stream was opened (RFC 0002, C1). A damage counter, not an error:
    /// `0` for a clean stream and for decoders that cannot tell.
    ///
    /// Not reset by [`Decoder::seek`].
    fn skipped_packets(&self) -> u64 {
        0
    }
}

/// Confidence (0..=100) that a factory can decode some media.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Confidence(pub u8);

impl Confidence {
    /// Cannot decode.
    pub const NONE: Confidence = Confidence(0);
    /// Extension matches, content not verified.
    pub const EXTENSION: Confidence = Confidence(40);
    /// Content signature matches.
    pub const CONTENT: Confidence = Confidence(90);
}

/// What a factory may look at when probing.
#[derive(Debug, Clone, Copy)]
pub struct ProbeHint<'a> {
    /// Lower-case extension without dot.
    pub extension: Option<&'a str>,
    /// Leading bytes of the media (up to [`DecoderRegistry::SNIFF_BYTES`]); empty for URLs.
    pub header: &'a [u8],
}

/// Creates decoders for one family of formats.
pub trait DecoderFactory: Send + Sync {
    /// Short unique name, e.g. `"symphonia"`.
    fn name(&self) -> &str;

    /// Extensions (lower-case, no dot) this factory claims; used for file dialogs and associations.
    fn extensions(&self) -> &[&str];

    /// Score how well this factory can handle the hinted media.
    fn probe(&self, hint: &ProbeHint<'_>) -> Confidence;

    /// Open a decoder. Called only after a successful probe.
    fn open(&self, source: MediaSource) -> Result<Box<dyn Decoder>>;
}

/// Ordered collection of [`DecoderFactory`]s; picks the best by probe score.
#[derive(Default, Clone)]
pub struct DecoderRegistry {
    factories: Vec<Arc<dyn DecoderFactory>>,
}

impl DecoderRegistry {
    /// Number of header bytes sniffed before probing.
    pub const SNIFF_BYTES: usize = 4096;

    /// Empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a factory.
    ///
    /// On equal confidence, earlier registrations win (RFC 0013).
    pub fn register(&mut self, factory: Arc<dyn DecoderFactory>) {
        self.factories.push(factory);
    }

    /// Registered factories.
    pub fn factories(&self) -> &[Arc<dyn DecoderFactory>] {
        &self.factories
    }

    /// All claimed extensions, sorted and de-duplicated.
    pub fn extensions(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .factories
            .iter()
            .flat_map(|f| f.extensions().iter().map(|e| (*e).to_owned()))
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// Best factory for `hint`, with its confidence, if any scores above zero.
    ///
    /// Guarantees (RFC 0013):
    /// 1. Content decides: a factory matching content signature (`Confidence::CONTENT`)
    ///    strictly outranks any factory matching only extension (`Confidence::EXTENSION`),
    ///    regardless of registration order.
    /// 2. Deterministic tie-break: on equal confidence, earlier registrations win.
    /// 3. Clear refusal: returns `None` if no factory scores above `Confidence::NONE`.
    pub fn probe(&self, hint: &ProbeHint<'_>) -> Option<(Arc<dyn DecoderFactory>, Confidence)> {
        let mut best: Option<(Arc<dyn DecoderFactory>, Confidence)> = None;
        for f in &self.factories {
            let c = f.probe(hint);
            if c > Confidence::NONE && best.as_ref().is_none_or(|(_, b)| c > *b) {
                best = Some((f.clone(), c));
            }
        }
        best
    }

    /// Sniff `source`, pick the best factory and open a decoder.
    ///
    /// Guarantees (RFC 0013):
    /// 1. Sniffs [`Self::SNIFF_BYTES`] from `source` to construct a [`ProbeHint`].
    /// 2. Selects the winning factory via [`Self::probe`].
    /// 3. Returns `Err(Error::UnsupportedFormat)` if no factory claims the source.
    /// 4. Dispatches directly to the winner's `open()` without backtracking.
    pub fn open(&self, mut source: MediaSource) -> Result<Box<dyn Decoder>> {
        let ext = source.extension();
        let header = source.sniff(Self::SNIFF_BYTES)?;
        let hint = ProbeHint {
            extension: ext.as_deref(),
            header: &header,
        };
        match self.probe(&hint) {
            Some((f, _)) => f.open(source),
            None => Err(Error::UnsupportedFormat(
                ext.unwrap_or_else(|| "unknown".into()),
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

/// Real-time render callback. The device calls it from its audio thread to
/// **pull** audio: fill the whole slice (interleaved, in the device's actual
/// [`AudioSpec`]) and write silence on underrun. Must not allocate, lock, or
/// block; the engine typically implements it as a read from a lock-free
/// SPSC ring (`rtrb`).
pub type RenderFn = Box<dyn FnMut(&mut [f32]) + Send + 'static>;

/// Format actually granted by an opened device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputFormat {
    /// Actual spec; may differ from the requested one (the engine resamples/remixes).
    pub spec: AudioSpec,
    /// Device buffer size in frames (upper bound of a single callback is device specific).
    pub buffer_frames: u32,
}

/// An audio output (WASAPI, cpal, null, disk writer, ...).
///
/// Lifecycle: `open(spec)` -> `start(render)` -> (`pause`/`resume`/`flush`)* -> `close`.
/// Methods are called from a control thread; only `render` runs on the
/// audio thread.
pub trait OutputDevice: Send {
    /// Human-readable device/driver name.
    fn name(&self) -> &str;

    /// Negotiate and open the device for (approximately) `spec`.
    fn open(&mut self, spec: AudioSpec) -> Result<OutputFormat>;

    /// Start pulling audio through `render` in the format returned by [`OutputDevice::open`].
    fn start(&mut self, render: RenderFn) -> Result<()>;

    /// Pause output (stops pulling; device buffer retained).
    fn pause(&mut self) -> Result<()>;

    /// Resume after [`OutputDevice::pause`].
    fn resume(&mut self) -> Result<()>;

    /// Drop everything buffered in the device (used on seek / stop).
    ///
    /// # What a backend must guarantee
    ///
    /// After a successful return, no frame that was already buffered can still
    /// reach the speakers: the next frame heard belongs to whatever the engine
    /// renders from then on.
    ///
    /// A backend that cannot do that portably - the cpal backend is the one we
    /// have, because a shared endpoint buffer cannot be reset from here - must
    /// **refuse** with [`Error::Unsupported`] rather than return `Ok` and let
    /// stale audio play. A silent `Ok` is the one answer that is worse than a
    /// failure, because the caller cannot tell the difference.
    ///
    /// # What a caller must assume
    ///
    /// The engine treats a refusal as terminal for the current output: it stops
    /// the run and does not resume into an output it cannot clean, so the
    /// caller (the application) has to recreate the engine and the output
    /// before it can play again. There is no automatic recovery, and a
    /// successful flush is not a promise that the hardware falls silent at that
    /// instant - only that what was buffered will not be heard as the next
    /// audio. (W10 finding F3, A13.)
    fn flush(&mut self) -> Result<()>;

    /// Current output latency (time between a frame being rendered and heard).
    fn latency(&self) -> Duration;

    /// Whether [`OutputDevice::played_frames`] answers what has *been heard*
    /// rather than what the device was handed. A backend whose clock only mirrors
    /// its input (a classic output plug-in answers `getoutputtime` with the
    /// position of the write cursor) says `false`.
    fn reports_played_position(&self) -> bool {
        true
    }

    /// How long the device still needs after the ring is empty until everything
    /// it was handed has been heard. The natural end of a run waits this long, so
    /// a device that buffers seconds of audio does not lose its tail.
    ///
    /// Default: the device's own latency, which is right for a backend that
    /// reports the time between rendering and hearing it.
    fn drain_time(&self) -> Duration {
        self.latency()
    }

    /// Total frames consumed by the device since `start` (the audio clock).
    /// Used for position display. Not reset by `flush`.
    fn played_frames(&self) -> u64;

    /// Device-level volume in `0.0..=1.0`. Default: unsupported (the engine applies volume itself).
    fn set_volume(&mut self, _volume: f32) -> Result<()> {
        Err(Error::Unsupported("device volume"))
    }

    /// Check if a fatal mid-stream error occurred on the output device.
    ///
    /// Real backends latch stream failures (e.g. endpoint removal, hardware
    /// format revocation, buffer write streak exhaustion) asynchronously from
    /// their audio/render callbacks.
    ///
    /// This method is called from the control/engine thread (e.g. in
    /// `Engine::poll`). It must be non-blocking and lock-free.
    ///
    /// Returns `None` if the device is healthy or stopped, or `Some(err)`
    /// describing the fatal failure that terminated output.
    fn stream_error(&self) -> Option<Error> {
        None
    }

    /// Stop and release the device.
    fn close(&mut self) -> Result<()>;
}

// ---------------------------------------------------------------------------
// DSP
// ---------------------------------------------------------------------------

/// An in-place audio effect stage (EQ, resampler wrapper, limiter, plugin bridge).
///
/// `process` runs on the audio/decode path and must not allocate.
pub trait DspStage: Send {
    /// Short name for diagnostics.
    fn name(&self) -> &str;

    /// (Re)configure for a new format; called before the first `process` and
    /// whenever the sample rate or channel count changes. May allocate.
    fn prepare(&mut self, spec: AudioSpec) -> Result<()>;

    /// Process interleaved frames in place. `buf.len()` is a multiple of the prepared channel count.
    ///
    /// `Err` is terminal for the current run (RFC 0014 §4): the engine ends
    /// the track loudly and the resident helper exits loudly. The Ok path
    /// must not allocate; the Err path may, the run ends there.
    fn process(&mut self, buf: &mut [f32]) -> Result<()>;

    /// Clear internal state (filter history); called on seek and track change.
    fn reset(&mut self);

    /// Latency introduced, in frames at the prepared sample rate.
    fn latency_frames(&self) -> u32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_parsing() {
        assert_eq!(
            MediaSource::Path("C:/x/Song.MP3".into())
                .extension()
                .as_deref(),
            Some("mp3")
        );
        assert_eq!(
            MediaSource::Url("http://h/a/b.ogg?x=1".into())
                .extension()
                .as_deref(),
            Some("ogg")
        );
    }

    #[test]
    fn spec_math() {
        let s = AudioSpec::new(44100, 2);
        assert_eq!(s.samples_for(10), 20);
        assert_eq!(s.frames_in(20), 10);
        assert_eq!(s.duration_of(44100), Duration::from_secs(1));
    }

    /// Minimal decoder used to check the provided trait methods.
    struct Minimal;

    impl Decoder for Minimal {
        fn spec(&self) -> AudioSpec {
            AudioSpec::new(8000, 1)
        }
        fn read(&mut self, _out: &mut [f32]) -> Result<usize> {
            Ok(0)
        }
        fn seek(&mut self, _to: Duration) -> Result<Duration> {
            Ok(Duration::ZERO)
        }
        fn duration(&self) -> Option<Duration> {
            None
        }
    }

    #[test]
    fn skipped_packets_defaults_to_zero() {
        // RFC 0002, C1: a decoder that cannot tell reports no damage.
        assert_eq!(Minimal.skipped_packets(), 0);
    }

    /// Minimal output used to check the provided trait methods.
    struct MinimalOutput;

    impl OutputDevice for MinimalOutput {
        fn name(&self) -> &str {
            "minimal"
        }
        fn open(&mut self, spec: AudioSpec) -> Result<OutputFormat> {
            Ok(OutputFormat {
                spec,
                buffer_frames: 1024,
            })
        }
        fn start(&mut self, _render: RenderFn) -> Result<()> {
            Ok(())
        }
        fn pause(&mut self) -> Result<()> {
            Ok(())
        }
        fn resume(&mut self) -> Result<()> {
            Ok(())
        }
        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
        fn latency(&self) -> Duration {
            Duration::ZERO
        }
        fn played_frames(&self) -> u64 {
            0
        }
        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn stream_error_defaults_to_none() {
        assert!(MinimalOutput.stream_error().is_none());
    }

    /// An output that latched a fatal mid-run failure.
    struct FailingOutput;

    impl OutputDevice for FailingOutput {
        fn name(&self) -> &str {
            "failing"
        }
        fn open(&mut self, spec: AudioSpec) -> Result<OutputFormat> {
            Ok(OutputFormat {
                spec,
                buffer_frames: 1024,
            })
        }
        fn start(&mut self, _render: RenderFn) -> Result<()> {
            Ok(())
        }
        fn pause(&mut self) -> Result<()> {
            Ok(())
        }
        fn resume(&mut self) -> Result<()> {
            Ok(())
        }
        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
        fn latency(&self) -> Duration {
            Duration::ZERO
        }
        fn played_frames(&self) -> u64 {
            0
        }
        fn stream_error(&self) -> Option<Error> {
            Some(Error::Output("the device disappeared".into()))
        }
        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_latched_failure_is_reachable_through_the_trait_object() {
        // RFC 0012 §1.2 and §4.1: the engine holds a `Box<dyn OutputDevice>`,
        // so a latched failure that only lived in an inherent backend method
        // would be invisible to it. The trait method is the contract, and it
        // has to work through the trait object the engine really uses.
        let device: Box<dyn OutputDevice> = Box::new(FailingOutput);
        let err = device.stream_error().expect("the latched failure");
        assert!(
            matches!(err, Error::Output(ref msg) if msg == "the device disappeared"),
            "{err}"
        );
    }
}
