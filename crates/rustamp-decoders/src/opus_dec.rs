//! Opus decoding for the symphonia pipeline.
//!
//! Symphonia ships no Opus codec. No published version of the crate has an
//! `opus` feature - the feature lists of 0.1.0 through 0.6.1 name no Opus
//! codec at all - and `symphonia-codec-opus` does not exist on crates.io.
//! What symphonia does have is the Ogg mapping: its demuxer reads the Opus
//! identification header, states the codec parameters (48 kHz, the channel
//! count, the pre-skip) and hands the packets on. Only the codec itself is
//! missing.
//!
//! This module fills that gap with [`opus_rs`], a pure-Rust port of the
//! reference implementation (libopus 1.6). The decoder implements symphonia's
//! `Decoder` trait, so the demuxer, the gapless trimming and the truncation
//! oracle of the crate work on an Opus file exactly as they work on a Vorbis
//! one.
//!
//! The decoder is registered in a codec registry of our own
//! ([`codec_registry`]) that holds symphonia's codecs next to it, so nothing
//! else in the crate has to know where the Opus codec comes from.

use std::sync::OnceLock;

use opus_rs::OpusDecoder as OpusCodec;
use symphonia::core::audio::{AsAudioBufferRef, AudioBuffer, AudioBufferRef, Signal, SignalSpec};
use symphonia::core::codecs::{
    CODEC_TYPE_OPUS, CodecDescriptor, CodecParameters, CodecRegistry, Decoder, DecoderOptions,
    FinalizeResult,
};
use symphonia::core::errors::{Error, Result};
use symphonia::core::formats::Packet;
use symphonia::core::support_codec;

/// The rate an Opus stream is decoded at.
///
/// Opus runs at 48 kHz: the rate of the codec is fixed, and every container
/// that carries Opus states 48 kHz. The rate of the original material is
/// only a hint the encoder works with.
const OPUS_RATE: u32 = 48_000;

/// The longest audio a single Opus packet may hold: 120 ms (RFC 6716, §3.2.5).
///
/// The bound also caps what a hostile file can make the decoder allocate.
const MAX_PACKET_FRAMES: usize = OPUS_RATE as usize * 120 / 1000;

/// The number of frames one frame of a packet holds, at 48 kHz.
///
/// The table is RFC 6716: the configuration number (the top five bits of the
/// table-of-contents byte) names the mode and the bandwidth, and every
/// configuration comes with a frame duration.
fn frame_samples(config: u8) -> usize {
    match config {
        // SILK: 10, 20, 40 or 60 ms, the same for every bandwidth.
        0..=11 => [10, 20, 40, 60][(config & 0x03) as usize] * 48,
        // Hybrid: 10 or 20 ms.
        12..=15 => [10, 20][(config & 0x01) as usize] * 48,
        // CELT: 2.5, 5, 10 or 20 ms.
        _ => [120, 240, 480, 960][(config & 0x03) as usize],
    }
}

/// The number of frames a packet holds, from its table-of-contents byte.
///
/// `None` when the packet does not carry a usable table of contents: an empty
/// packet, or a code-3 packet without its frame count.
fn packet_frames(packet: &[u8]) -> Option<usize> {
    let toc = *packet.first()?;
    let frames = match toc & 0x03 {
        // One frame.
        0 => 1,
        // Two frames of the same length.
        1 | 2 => 2,
        // Two or more frames: the count is in the byte behind the table of
        // contents, and it must be in the range 1 to 48.
        _ => {
            let count = usize::from(*packet.get(1)?) & 0x3F;
            if count == 0 || count > 48 {
                return None;
            }
            count
        }
    };
    Some(frame_samples(toc >> 3) * frames)
}

/// The signature of the Opus identification header.
const OPUS_HEAD: &[u8] = b"OpusHead";

/// The number of samples at the start of an Opus stream that are the
/// encoder's look-ahead and not audio.
///
/// The identification header of the stream states the number; the samples
/// themselves are the first ones the codec delivers.
///
/// The header is only there for the streams that carry one (the Ogg
/// mapping), so a stream without one has no pre-skip to drop.
pub fn pre_skip(extra_data: Option<&[u8]>) -> usize {
    // The header: the signature, the version, the channel count, the pre-skip,
    // the rate of the original material, the gain and the channel mapping.
    const PRE_SKIP: usize = 10;
    const LENGTH: usize = 19;
    let Some(header) = extra_data else {
        return 0;
    };
    if header.len() < LENGTH || !header.starts_with(OPUS_HEAD) {
        return 0;
    }
    usize::from(u16::from_le_bytes([header[PRE_SKIP], header[PRE_SKIP + 1]]))
}

/// Decoder for the Opus codec.
pub struct OpusDecoder {
    codec: OpusCodec,
    params: CodecParameters,
    /// The number of channels of the stream.
    channels: usize,
    /// The signal the decoder delivers.
    spec: SignalSpec,
    /// The samples at the start of the stream that are the encoder's
    /// look-ahead and not audio.
    pre_skip: usize,
    /// Whether the pre-skip has been dropped.
    ///
    /// A seek starts the stream again, so the pre-skip is dropped again when
    /// the stream is played from its beginning.
    trimmed: bool,
    /// The interleaved output of the codec, copied into the planar buffer.
    scratch: Vec<f32>,
    /// The planar output buffer.
    buf: AudioBuffer<f32>,
}

impl Decoder for OpusDecoder {
    fn try_new(params: &CodecParameters, _: &DecoderOptions) -> Result<Self> {
        // This decoder only supports Opus.
        if params.codec != CODEC_TYPE_OPUS {
            return Err(Error::Unsupported("opus: invalid codec type"));
        }
        // The channel count comes from the identification header of the stream.
        let channels = params.channels.ok_or(Error::Unsupported(
            "opus: the stream states no channel count",
        ))?;
        let count = channels.count();
        // The Opus codec decodes one or two channels; a stream with more needs
        // the multistream decoder, which this crate does not have.
        if !(1..=2).contains(&count) {
            return Err(Error::Unsupported(
                "opus: only mono and stereo streams are supported",
            ));
        }
        // Opus always runs at 48 kHz; a container that states another rate
        // does not describe an Opus stream this decoder can read.
        if let Some(rate) = params.sample_rate
            && rate != OPUS_RATE
        {
            return Err(Error::Unsupported(
                "opus: only 48 kHz streams are supported",
            ));
        }
        let spec = SignalSpec::new(OPUS_RATE, channels);
        let codec = OpusCodec::new(OPUS_RATE as i32, count)
            .map_err(|_| Error::Unsupported("opus: the codec refused the stream"))?;
        Ok(Self {
            codec,
            params: params.clone(),
            channels: count,
            spec,
            pre_skip: pre_skip(params.extra_data.as_deref()),
            trimmed: false,
            scratch: Vec::new(),
            buf: AudioBuffer::new(0, spec),
        })
    }

    fn supported_codecs() -> &'static [CodecDescriptor] {
        &[support_codec!(CODEC_TYPE_OPUS, "opus", "Opus")]
    }

    fn reset(&mut self) {
        // The codec carries the state of the frame before it; a seek starts a
        // new stream, so that state is dropped.
        if let Ok(codec) = OpusCodec::new(OPUS_RATE as i32, self.channels) {
            self.codec = codec;
        }
        // The pre-skip is at the start of the stream: a seek that lands there
        // has to drop it again.
        self.trimmed = false;
    }

    fn codec_params(&self) -> &CodecParameters {
        &self.params
    }

    fn decode(&mut self, packet: &Packet) -> Result<AudioBufferRef<'_>> {
        let data: &[u8] = &packet.data;
        let Some(frames) = packet_frames(data) else {
            self.buf.clear();
            return Err(Error::DecodeError("opus: invalid packet header"));
        };
        if frames > MAX_PACKET_FRAMES {
            self.buf.clear();
            return Err(Error::DecodeError(
                "opus: the packet holds more than 120 ms of audio",
            ));
        }
        // The buffer is reused between packets and only grows when a packet
        // needs more room than the last one did.
        if self.buf.capacity() < frames {
            self.buf = AudioBuffer::new(frames as u64, self.spec);
        }
        self.scratch.resize(frames * self.channels, 0.0);
        let written = match self.codec.decode(data, frames, &mut self.scratch) {
            Ok(written) => written,
            // The codec names its own failures; they are static strings.
            Err(m) => {
                self.buf.clear();
                return Err(Error::DecodeError(m));
            }
        };
        self.buf.clear();
        self.buf.render_reserved(Some(written));
        // The codec delivers interleaved samples, the buffer holds them
        // planar: one plane per channel.
        for c in 0..self.channels {
            let plane = self.buf.chan_mut(c);
            for (i, sample) in plane.iter_mut().enumerate() {
                *sample = self.scratch[i * self.channels + c];
            }
        }
        // Gapless: the container states how many samples at the end of the
        // stream are padding. They are not audio.
        //
        // The samples at the start of the stream that are the encoder's
        // look-ahead are dropped here: the packet that carries the start of
        // the stream is the one whose timestamp is zero, and the Ogg reader of
        // symphonia leaves those samples in the stream (its Opus support is
        // unfinished).
        let trim_start = if !self.trimmed && packet.ts == 0 {
            self.trimmed = true;
            self.pre_skip.min(written)
        } else {
            0
        };
        self.buf.trim(trim_start, packet.trim_end() as usize);
        Ok(self.buf.as_audio_buffer_ref())
    }

    fn finalize(&mut self) -> FinalizeResult {
        FinalizeResult { verify_ok: None }
    }

    fn last_decoded(&self) -> AudioBufferRef<'_> {
        self.buf.as_audio_buffer_ref()
    }
}

/// The codec registry the symphonia decoder uses.
///
/// It holds every codec symphonia itself enables and the Opus decoder of this
/// module, which symphonia does not ship.
pub fn codec_registry() -> &'static CodecRegistry {
    static REGISTRY: OnceLock<CodecRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = CodecRegistry::new();
        symphonia::default::register_enabled_codecs(&mut registry);
        registry.register_all::<OpusDecoder>();
        registry
    })
}

/// The number of audio frames a stream of Opus holds.
///
/// The Ogg Opus mapping states the granule position of the last page, and
/// that granule counts the pre-skip samples - the encoder's look-ahead, which
/// is not audio (RFC 7845, §4). The audio such a file holds is therefore the
/// granule minus the pre-skip.
pub fn audio_frames(params: &CodecParameters, ogg: bool) -> Option<u64> {
    let frames = params.n_frames?;
    if ogg && params.codec == CODEC_TYPE_OPUS {
        let skip = pre_skip(params.extra_data.as_deref()) as u64;
        return Some(frames.saturating_sub(skip));
    }
    Some(frames)
}
