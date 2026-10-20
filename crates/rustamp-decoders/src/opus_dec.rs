//! Opus decoding for the symphonia pipeline.
//!
//! Symphonia ships no Opus codec. No published version of the crate has an
//! `opus` feature - the feature lists of 0.1.0 through 0.6.1 name no Opus
//! codec at all - and `symphonia-codec-opus` does not exist on crates.io.
//! What symphonia does have are the mappings: its Ogg demuxer reads the Opus
//! identification header, states the codec parameters (48 kHz, the channel
//! count, the pre-skip) and hands the packets on, and its Matroska demuxer
//! hands the identification header of the track to the codec as the codec
//! private data. Only the codec itself is missing.
//!
//! This module fills that gap with [`opus_rs`], a pure-Rust port of the
//! reference implementation (libopus 1.6). The decoder implements symphonia's
//! `Decoder` trait, so the demuxer, the gapless trimming and the truncation
//! oracle of the crate work on an Opus file exactly as they work on a Vorbis
//! one.
//!
//! The Matroska mapping states the channel count of an Opus track as a
//! channel layout and not as a count, so the parameters the container reader
//! hands to the codec do not describe the stream. [`complete_parameters`]
//! completes them from the identification header the track carries - the
//! codec's own statement of its parameters (RFC 7845, §5.1) - so that a
//! stream in a Matroska file is decoded like a stream in an Ogg file.
//!
//! The decoder is registered in a codec registry of our own
//! ([`codec_registry`]) that holds symphonia's codecs next to it, so nothing
//! else in the crate has to know where the Opus codec comes from.

use std::sync::OnceLock;

use opus_rs::OpusDecoder as OpusCodec;
use symphonia::core::audio::{
    AsAudioBufferRef, AudioBuffer, AudioBufferRef, Channels, Signal, SignalSpec,
};
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
///
/// The function is crate-internal: the malformed-input suite
/// (`crate::probe_fuzz`) reads the table of contents of a packet that comes
/// from a file, and no caller outside the crate has a use for it.
pub(crate) fn packet_frames(packet: &[u8]) -> Option<usize> {
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

/// The length of the identification header (RFC 7845, §5.1).
///
/// The header is longer than that for a stream with a channel mapping table
/// (the mapping families 1 and 255), so this is the length that is always
/// there.
const OPUS_HEAD_LEN: usize = 19;

/// The identification header of an Opus stream, if the stream carries one.
///
/// The header is the codec's own statement of its parameters (RFC 7845,
/// §5.1). The Ogg mapping carries it as the first packet of the stream, the
/// Matroska mapping stores it as the codec private data of the track - in
/// both cases it reaches the codec as the extra data of the parameters.
fn head(extra_data: Option<&[u8]>) -> Option<&[u8]> {
    let data = extra_data?;
    (data.len() >= OPUS_HEAD_LEN && data.starts_with(OPUS_HEAD)).then_some(data)
}

/// The number of channels the identification header states.
fn head_channels(extra_data: Option<&[u8]>) -> Option<u8> {
    // The header: the signature, the version, the channel count, the
    // pre-skip, the rate of the original material, the gain and the channel
    // mapping.
    const CHANNELS: usize = 9;
    Some(head(extra_data)?[CHANNELS])
}

/// The channel mask of a stream with `count` channels.
///
/// The mask carries the number of channels of the stream, which is what the
/// decoder needs: the Opus codec decodes mono and stereo, and a wider stream
/// is refused by name. The flags are the channel flags of the WAVE format in
/// order, so the first `count` of them are the first `count` channels of the
/// stream.
fn channel_mask(count: u8) -> Channels {
    // A stream cannot have more channels than the mask can name.
    let count = u32::from(count).min(32);
    let bits = if count == 0 {
        0
    } else {
        u32::MAX >> (32 - count)
    };
    Channels::from_bits_truncate(bits)
}

/// The codec parameters of an Opus stream, completed.
///
/// The Matroska mapping states the channel count of an Opus track as a
/// channel layout and not as a count, and the container reader hands the
/// codec the layout alone: the decoder would then refuse the stream by name
/// ("the stream states no channel count"), although the track states its
/// channels in the identification header it carries.
///
/// The header is the codec's own statement of its parameters, so it is the
/// one that is used. A stream without a header keeps the count of its
/// container.
///
/// Completing the parameters twice changes nothing.
pub fn complete_parameters(params: &mut CodecParameters) {
    if let Some(count) = head_channels(params.extra_data.as_deref()) {
        params.channels = Some(channel_mask(count));
        return;
    }
    // No header: the layout of the container carries the count.
    if params.channels.is_none()
        && let Some(layout) = params.channel_layout
    {
        params.channels = Some(layout.into_channels());
    }
}

/// The number of samples at the start of an Opus stream that are the
/// encoder's look-ahead and not audio.
///
/// The identification header of the stream states the number; the samples
/// themselves are the first ones the codec delivers.
///
/// The header is only there for the streams that carry one (the Ogg and the
/// Matroska mappings), so a stream without one has no pre-skip to drop.
pub fn pre_skip(extra_data: Option<&[u8]>) -> usize {
    let Some(header) = head(extra_data) else {
        return 0;
    };
    // The header: the signature, the version, the channel count, the pre-skip,
    // the rate of the original material, the gain and the channel mapping.
    const PRE_SKIP: usize = 10;
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
        // The parameters of the stream are completed from the identification
        // header it carries: the Matroska mapping states the channel count of
        // an Opus track as a channel layout and not as a count.
        let mut params = params.clone();
        complete_parameters(&mut params);
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
        let pre_skip = pre_skip(params.extra_data.as_deref());
        Ok(Self {
            codec,
            params,
            channels: count,
            spec,
            pre_skip,
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

/// The number of audio frames a stream holds.
///
/// The samples at the start of an Opus stream are the encoder's look-ahead
/// and not audio, and the container counts them: the granule position of an
/// Ogg stream counts them (RFC 7845, §4) and the duration of a Matroska
/// file covers the whole encoded stream. The audio of such a file is
/// therefore the count of the container less the look-ahead.
///
/// `frames` is the number of frames the container states. A stream that
/// carries no identification header has no look-ahead, so its count is the
/// count of the container.
pub fn audio_frames(params: &CodecParameters, frames: Option<u64>) -> Option<u64> {
    let frames = frames?;
    let skip = pre_skip(params.extra_data.as_deref()) as u64;
    Some(frames.saturating_sub(skip))
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::audio::Layout;

    /// The identification header of a stereo stream (RFC 7845, §5.1).
    fn identification_header(channels: u8, pre_skip: u16) -> Box<[u8]> {
        let mut head = Vec::new();
        head.extend_from_slice(OPUS_HEAD);
        head.push(1); // the version
        head.push(channels);
        head.extend_from_slice(&pre_skip.to_le_bytes());
        head.extend_from_slice(&48_000u32.to_le_bytes());
        head.extend_from_slice(&0u16.to_le_bytes()); // the gain
        head.push(0); // the mapping family
        head.into_boxed_slice()
    }

    /// The parameters of a Matroska track: the channel count is missing, the
    /// layout and the codec private data are there.
    fn matroska_params(channels: u8) -> CodecParameters {
        let mut params = CodecParameters::new();
        params.for_codec(CODEC_TYPE_OPUS);
        params.with_sample_rate(48_000);
        params.with_channel_layout(if channels == 1 {
            Layout::Mono
        } else {
            Layout::Stereo
        });
        params.with_extra_data(identification_header(channels, 312));
        params
    }

    /// The channel count of a Matroska track comes from the identification
    /// header it carries.
    #[test]
    fn the_channel_count_comes_from_the_identification_header() {
        let mut params = matroska_params(2);
        assert!(params.channels.is_none());
        complete_parameters(&mut params);
        let channels = params.channels.expect("the count is there");
        assert_eq!(channels.count(), 2);
    }

    /// A mono stream states one channel, and the mask of a wider stream
    /// carries its width.
    #[test]
    fn the_mask_carries_the_width_of_the_stream() {
        assert_eq!(channel_mask(1).count(), 1);
        assert_eq!(channel_mask(2).count(), 2);
        assert_eq!(channel_mask(6).count(), 6);
    }

    /// A stream without an identification header keeps the count of its
    /// container.
    #[test]
    fn a_stream_without_a_header_keeps_the_count_of_its_container() {
        let mut params = CodecParameters::new();
        params.for_codec(CODEC_TYPE_OPUS);
        params.with_channel_layout(Layout::Stereo);
        complete_parameters(&mut params);
        let channels = params.channels.expect("the count is there");
        assert_eq!(channels.count(), 2);
    }

    /// The look-ahead is the number the header states, and a stream without a
    /// header has none.
    #[test]
    fn the_look_ahead_is_the_number_the_header_states() {
        assert_eq!(pre_skip(Some(&identification_header(2, 312))), 312);
        assert_eq!(pre_skip(None), 0);
        assert_eq!(pre_skip(Some(b"not an Opus header")), 0);
    }

    /// The frames of the audio are the frames of the container less the
    /// look-ahead.
    #[test]
    fn the_look_ahead_is_not_audio() {
        let params = matroska_params(2);
        assert_eq!(audio_frames(&params, Some(72_312)), Some(72_000));
    }
}
