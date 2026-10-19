// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The exclusive-mode part of the WASAPI backend.
//!
//! # What exclusive mode is
//!
//! In shared mode the Windows audio engine owns the endpoint: it mixes the
//! streams of every application, converts their formats and hands the result
//! to the driver. In exclusive mode the application drives the device
//! itself: the stream is opened on the device, no mixer is involved, and the
//! samples reach the hardware in the device's own format. Nothing resamples
//! or dithers them on the way — that is the "bit-exact" the mode is asked
//! for.
//!
//! The price is that the device accepts one stream at a time, that the
//! format has to be one the device really supports, and that the period has
//! to be one the device really accepts.
//!
//! # The three decisions
//!
//! 1. **The format.** The device's *mix format* is its native format. When
//!    the device accepts it in exclusive mode, the stream is opened with it
//!    and the engine converts the source to the device's rate (with its
//!    band-limited resampler) — or not at all, when the rates agree. When
//!    the device refuses its own format, a format that matches the source is
//!    used, so the engine needs no conversion at all. The player drives one
//!    or two channels, so a device whose own format has more channels is
//!    played in stereo at the device's own rate. See [`plan`].
//! 2. **The period.** The device's own default period, never below the
//!    device's minimum period (`IAudioClient::GetDevicePeriod`) and aligned
//!    to the buffer alignment the driver asks for. See [`choose_period`].
//! 3. **The downgrade.** A device that refuses the exclusive stream is
//!    played in shared mode instead, and the downgrade is reported — never
//!    silent. See [`crate::WasapiOutput::downgrade_reason`].
//!
//! # What is testable here and what is not
//!
//! Everything in this module that does not need a device is a pure function
//! over plain data: the format selection, the period selection and the
//! sample conversion. Those are unit tested without an audio endpoint.
//!
//! The real exclusive open — a device that really grants an exclusive
//! stream, or really refuses one — needs real hardware and is therefore
//! **VM-only**. It is deliberately *not* faked here.

use rustamp_core::audio::AudioSpec;
use wasapi::{SampleType, WaveFormat};

/// The buffer alignment the driver asks for, in bytes.
///
/// The Intel High Definition Audio specification asks for buffer sizes in
/// multiples of 128 bytes. The period is aligned to it, so the driver
/// accepts the buffer; a driver that does not care simply gets a period
/// that is a few frames longer than its default.
const ALIGN_BYTES: u32 = 128;

/// The channel counts the player drives: one or two.
///
/// The shared path asks for the same, and the engine's remix fills the
/// channels the source does not have. A device whose own format has more
/// channels is therefore played in stereo (or mono) in exclusive mode as
/// well — the surround channels of such a device would carry nothing but
/// silence.
pub(crate) const MAX_CHANNELS: u16 = 2;

/// The sample layout of a device stream.
///
/// Only the layouts the player can produce are listed: the engine's render
/// callback delivers `f32`, and the backend converts the samples into the
/// device's own representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SampleLayout {
    /// 32-bit IEEE float, 4 bytes per sample.
    Float32,
    /// 16-bit signed integer, 2 bytes per sample.
    Int16,
    /// 24-bit signed integer, packed in 3 bytes per sample.
    Int24Packed,
    /// 24-bit signed integer in a 32-bit container, left aligned.
    Int24Padded,
    /// 32-bit signed integer, 4 bytes per sample.
    Int32,
}

impl SampleLayout {
    /// Bytes one sample occupies in the device buffer.
    pub(crate) const fn bytes_per_sample(self) -> usize {
        match self {
            Self::Float32 => 4,
            Self::Int16 => 2,
            Self::Int24Packed => 3,
            Self::Int24Padded => 4,
            Self::Int32 => 4,
        }
    }

    /// The layout of a device format, or `None` when the format is one the
    /// player cannot produce (an exotic bit depth, for example).
    ///
    /// `bits` is the size of the container (`wBitsPerSample`), `valid_bits`
    /// the number of bits that carry the signal (`wValidBitsPerSample`), and
    /// `is_float` tells an IEEE float format from an integer one.
    pub(crate) const fn of(bits: u16, valid_bits: u16, is_float: bool) -> Option<Self> {
        if is_float {
            return if bits == 32 {
                Some(Self::Float32)
            } else {
                None
            };
        }
        match (bits, valid_bits) {
            (16, 16) => Some(Self::Int16),
            (24, 24) => Some(Self::Int24Packed),
            (32, 24) => Some(Self::Int24Padded),
            (32, 32) => Some(Self::Int32),
            _ => None,
        }
    }
}

/// A format the device can be opened with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DeviceFormat {
    /// Sample rate in Hz.
    pub(crate) sample_rate: u32,
    /// Number of interleaved channels.
    pub(crate) channels: u16,
    /// The sample layout of the stream.
    pub(crate) layout: SampleLayout,
}

impl DeviceFormat {
    /// The rate and the channel count of the format: the spec the engine
    /// works with.
    pub(crate) const fn spec(&self) -> AudioSpec {
        AudioSpec::new(self.sample_rate, self.channels)
    }

    /// The device format of a WASAPI format, or `None` when the player
    /// cannot produce it.
    pub(crate) fn of_wave(format: &WaveFormat) -> Option<Self> {
        let channels = format.get_nchannels();
        if channels == 0 {
            return None;
        }
        let is_float = format.get_subformat().ok()? == SampleType::Float;
        let layout = Self::layout_of(format, is_float)?;
        Some(Self {
            sample_rate: format.get_samplespersec(),
            channels,
            layout,
        })
    }

    /// The layout of a WASAPI format.
    fn layout_of(format: &WaveFormat, is_float: bool) -> Option<SampleLayout> {
        SampleLayout::of(
            format.get_bitspersample(),
            format.get_validbitspersample(),
            is_float,
        )
    }
}

/// The stream an exclusive mode device is opened with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExclusivePlan {
    /// The format of the stream.
    pub(crate) format: DeviceFormat,
    /// The period in 100 ns units.
    pub(crate) period_hns: i64,
    /// Whether the engine has to convert the source into the stream's
    /// format.
    ///
    /// `false` means the source already has the stream's rate and channel
    /// count, so the samples reach the device unconverted.
    pub(crate) converts: bool,
}

/// Choose the format and the period of an exclusive stream.
///
/// * `source` — the spec the engine asked for.
/// * `native` — the device's own (mix) format.
/// * `accepted` — the formats the device accepts in exclusive mode.
/// * `default_period_hns`, `min_period_hns` — the device's periods, from
///   `IAudioClient::GetDevicePeriod`.
///
/// The rules, in order:
///
/// 1. **The device's own format wins** when the device accepts it in
///    exclusive mode and the player can drive its channel count. The stream
///    then runs at the rate the device wants, and the engine converts the
///    source to it — or not at all, when the rates agree.
/// 2. **The device's own rate and layout win** when its own format has more
///    channels than the player drives: the stream is then opened with the
///    channel count the player can fill.
/// 3. **A format that matches the source wins** when the device's own
///    format is refused: the engine then needs no conversion at all.
/// 4. **The best remaining accepted format wins** otherwise, the one at the
///    device's own rate first.
/// 5. **No accepted format at all** means there is no exclusive stream. The
///    answer is `None`, and the caller falls back to shared mode and reports
///    the downgrade — it is never a silent success.
pub(crate) fn plan(
    source: AudioSpec,
    native: DeviceFormat,
    accepted: &[DeviceFormat],
    default_period_hns: i64,
    min_period_hns: i64,
) -> Option<ExclusivePlan> {
    let format = choose_format(source, native, accepted)?;
    let block_align = format.layout.bytes_per_sample() as u32 * format.channels as u32;
    Some(ExclusivePlan {
        format,
        period_hns: choose_period(
            default_period_hns,
            min_period_hns,
            format.sample_rate,
            block_align,
        ),
        converts: format.sample_rate != source.sample_rate || format.channels != source.channels,
    })
}

/// The format of an exclusive stream, or `None` when the device accepts none.
fn choose_format(
    source: AudioSpec,
    native: DeviceFormat,
    accepted: &[DeviceFormat],
) -> Option<DeviceFormat> {
    // The player drives one or two channels; a format with more channels
    // would carry silence in the channels the source does not have.
    let eligible: Vec<DeviceFormat> = accepted
        .iter()
        .copied()
        .filter(|f| (1..=MAX_CHANNELS).contains(&f.channels))
        .collect();
    if eligible.is_empty() {
        return None;
    }
    // Rule 1: the device's own format.
    if (1..=MAX_CHANNELS).contains(&native.channels) && eligible.contains(&native) {
        return Some(native);
    }
    // Rule 2: the device's own rate and layout, with a channel count the
    // player can drive.
    let adapted: Vec<DeviceFormat> = eligible
        .iter()
        .copied()
        .filter(|f| f.sample_rate == native.sample_rate && f.layout == native.layout)
        .collect();
    if let Some(best) = best_of(&adapted, native) {
        return Some(best);
    }
    // Rule 3: a format the source already has.
    let matching: Vec<DeviceFormat> = eligible
        .iter()
        .copied()
        .filter(|f| f.sample_rate == source.sample_rate && f.channels == source.channels)
        .collect();
    if let Some(best) = best_of(&matching, native) {
        return Some(best);
    }
    // Rule 4: the best of what is left, the device's own rate first.
    let at_native_rate: Vec<DeviceFormat> = eligible
        .iter()
        .copied()
        .filter(|f| f.sample_rate == native.sample_rate)
        .collect();
    best_of(&at_native_rate, native).or_else(|| best_of(&eligible, native))
}

/// The best format of a set: the device's own layout first, then the layout
/// that keeps the most of the engine's `f32` precision.
fn best_of(formats: &[DeviceFormat], native: DeviceFormat) -> Option<DeviceFormat> {
    formats
        .iter()
        .copied()
        .min_by_key(|f| layout_rank(f.layout, native.layout))
}

/// The preference of a layout: the device's own layout first, then 32-bit
/// float (the engine's own format, so nothing is lost), then the integer
/// layouts from the deepest to the shallowest.
fn layout_rank(layout: SampleLayout, native: SampleLayout) -> u8 {
    if layout == native {
        return 0;
    }
    match layout {
        SampleLayout::Float32 => 1,
        SampleLayout::Int32 => 2,
        SampleLayout::Int24Packed => 3,
        SampleLayout::Int24Padded => 4,
        SampleLayout::Int16 => 5,
    }
}

/// The period of an exclusive stream, in 100 ns units.
///
/// * `default_hns` and `min_hns` are the device's periods, from
///   `IAudioClient::GetDevicePeriod`.
/// * `sample_rate` is the rate of the stream.
/// * `block_align` is the size of one frame of the stream in bytes.
///
/// The period is the device's default period, never below the device's
/// minimum, and rounded up to a whole number of aligned frames — the
/// alignment the driver asks for is [`ALIGN_BYTES`]. The device buffer is
/// then exactly one period long.
///
/// The rounding is done on the **frame count the device derives** from the
/// period, not on the period itself: the audio engine computes the buffer
/// size as `hnsBufferDuration * nSamplesPerSec / 10 000 000` frames
/// (truncated), so a period that looks aligned can still yield a buffer that
/// is not. The period is therefore chosen so that the frame count the
/// device derives from it is the aligned one.
pub(crate) fn choose_period(
    default_hns: i64,
    min_hns: i64,
    sample_rate: u32,
    block_align: u32,
) -> i64 {
    let rate = i64::from(sample_rate.max(1));
    let period = default_hns.max(min_hns).max(1);
    // The frames the device derives from that period.
    let frames = frames_of(period, rate);
    // The smallest number of frames the device accepts.
    let min_frames = (min_hns.max(0) * rate + 9_999_999) / 10_000_000;
    let block = i64::from(block_align.max(1));
    // The buffer has to be a whole number of frames, and the driver asks for
    // a multiple of `ALIGN_BYTES` bytes.
    let align_frames = (lcm(block as u64, u64::from(ALIGN_BYTES)) / block as u64).max(1) as i64;
    let target = frames.max(min_frames).max(1);
    let aligned = (target + align_frames - 1) / align_frames * align_frames;
    // The period that makes the device derive exactly that many frames.
    (aligned * 10_000_000 + rate - 1) / rate
}

/// The frames the audio engine derives from a period in 100 ns units.
///
/// The engine truncates the division, so this is the frame count the device
/// really uses.
fn frames_of(period_hns: i64, sample_rate: i64) -> i64 {
    (period_hns * sample_rate) / 10_000_000
}

/// The least common multiple of two numbers.
fn lcm(a: u64, b: u64) -> u64 {
    a / gcd(a, b) * b
}

/// The greatest common divisor of two numbers.
fn gcd(a: u64, b: u64) -> u64 {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    a
}

/// Write `samples` into `out` in the device's own representation.
///
/// `out` must hold at least `samples.len() * layout.bytes_per_sample()`
/// bytes; the extra bytes are left alone. The function allocates nothing
/// and takes no lock: it runs on the audio thread.
pub(crate) fn write_samples(samples: &[f32], layout: SampleLayout, out: &mut [u8]) {
    match layout {
        SampleLayout::Float32 => {
            for (chunk, value) in out.as_chunks_mut::<4>().0.iter_mut().zip(samples) {
                chunk.copy_from_slice(&value.to_le_bytes());
            }
        }
        SampleLayout::Int16 => {
            for (chunk, value) in out.as_chunks_mut::<2>().0.iter_mut().zip(samples) {
                chunk.copy_from_slice(&to_i16(*value).to_le_bytes());
            }
        }
        SampleLayout::Int24Packed => {
            for (chunk, value) in out.as_chunks_mut::<3>().0.iter_mut().zip(samples) {
                let bits = to_i24(*value) as u32;
                chunk[0] = bits as u8;
                chunk[1] = (bits >> 8) as u8;
                chunk[2] = (bits >> 16) as u8;
            }
        }
        SampleLayout::Int24Padded => {
            for (chunk, value) in out.as_chunks_mut::<4>().0.iter_mut().zip(samples) {
                // The 24 valid bits sit in the high bits of the 32-bit
                // container: that is what `wValidBitsPerSample` means in a
                // WAVEFORMATEXTENSIBLE.
                let bits = to_i24(*value) << 8;
                chunk.copy_from_slice(&bits.to_le_bytes());
            }
        }
        SampleLayout::Int32 => {
            for (chunk, value) in out.as_chunks_mut::<4>().0.iter_mut().zip(samples) {
                chunk.copy_from_slice(&to_i32(*value).to_le_bytes());
            }
        }
    }
}

/// A `f32` sample as a 16-bit integer.
///
/// The sample is scaled to the full-scale value of the format and clamped,
/// so a sample outside `-1.0..=1.0` cannot wrap around.
fn to_i16(value: f32) -> i16 {
    let scaled = (f64::from(value) * 32_767.0).round();
    scaled.clamp(-32_768.0, 32_767.0) as i16
}

/// A `f32` sample as a 24-bit integer.
fn to_i24(value: f32) -> i32 {
    let scaled = (f64::from(value) * 8_388_607.0).round();
    scaled.clamp(-8_388_608.0, 8_388_607.0) as i32
}

/// A `f32` sample as a 32-bit integer.
fn to_i32(value: f32) -> i32 {
    let scaled = (f64::from(value) * 2_147_483_647.0).round();
    scaled.clamp(-2_147_483_648.0, 2_147_483_647.0) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The device's own format: 48 kHz stereo, 32-bit float.
    fn native() -> DeviceFormat {
        DeviceFormat {
            sample_rate: 48_000,
            channels: 2,
            layout: SampleLayout::Float32,
        }
    }

    /// The source the engine asks for: 44.1 kHz stereo.
    fn source() -> AudioSpec {
        AudioSpec::new(44_100, 2)
    }

    #[test]
    fn the_devices_own_format_wins_when_the_device_accepts_it() {
        let accepted = vec![
            native(),
            DeviceFormat {
                sample_rate: 44_100,
                channels: 2,
                layout: SampleLayout::Float32,
            },
        ];
        let plan = plan(source(), native(), &accepted, 100_000, 50_000).expect("a plan");
        assert_eq!(plan.format, native());
        assert!(
            plan.converts,
            "the source is 44.1 kHz and the device runs at 48 kHz, so the engine converts"
        );
    }

    #[test]
    fn a_format_that_matches_the_source_wins_when_the_own_one_is_refused() {
        // The device refuses its own 48 kHz format but accepts 44.1 kHz.
        let accepted = vec![DeviceFormat {
            sample_rate: 44_100,
            channels: 2,
            layout: SampleLayout::Float32,
        }];
        let plan = plan(source(), native(), &accepted, 100_000, 50_000).expect("a plan");
        assert_eq!(plan.format.sample_rate, 44_100);
        assert!(
            !plan.converts,
            "the source already has the stream's rate, so nothing is converted"
        );
    }

    #[test]
    fn the_devices_own_rate_wins_over_a_foreign_one_when_neither_matches_the_source() {
        // The source is 96 kHz, which the device does not accept; the device
        // runs at 48 kHz and accepts 44.1 kHz as well.
        let accepted = vec![
            native(),
            DeviceFormat {
                sample_rate: 44_100,
                channels: 2,
                layout: SampleLayout::Float32,
            },
        ];
        let plan = plan(
            AudioSpec::new(96_000, 2),
            native(),
            &accepted,
            100_000,
            50_000,
        )
        .expect("a plan");
        assert_eq!(plan.format, native(), "the device's own rate is preferred");
        assert!(plan.converts);
    }

    #[test]
    fn the_devices_own_layout_wins_among_the_formats_of_one_rate() {
        // Two formats at the same rate: the device's own layout and float.
        let int24 = DeviceFormat {
            sample_rate: 48_000,
            channels: 2,
            layout: SampleLayout::Int24Packed,
        };
        let float = DeviceFormat {
            sample_rate: 48_000,
            channels: 2,
            layout: SampleLayout::Float32,
        };
        let native_int24 = int24;
        let plan = plan(
            AudioSpec::new(96_000, 2),
            native_int24,
            &[float, int24],
            100_000,
            50_000,
        )
        .expect("a plan");
        assert_eq!(plan.format.layout, SampleLayout::Int24Packed);
    }

    #[test]
    fn a_device_that_accepts_nothing_has_no_exclusive_plan() {
        assert!(plan(source(), native(), &[], 100_000, 50_000).is_none());
    }

    #[test]
    fn a_device_with_more_channels_than_the_player_drives_is_played_in_stereo() {
        // A surround device: six channels at 48 kHz, and it accepts the same
        // rate and layout with two channels.
        let surround = DeviceFormat {
            sample_rate: 48_000,
            channels: 6,
            layout: SampleLayout::Float32,
        };
        let stereo = DeviceFormat {
            sample_rate: 48_000,
            channels: 2,
            layout: SampleLayout::Float32,
        };
        let plan = plan(source(), surround, &[surround, stereo], 100_000, 50_000).expect("a plan");
        assert_eq!(
            plan.format.channels, 2,
            "the player drives one or two channels"
        );
        assert_eq!(plan.format.sample_rate, 48_000, "the device's own rate");
    }

    #[test]
    fn a_device_that_only_accepts_more_channels_than_the_player_drives_has_no_plan() {
        // A surround device that accepts nothing but six channels: the
        // player cannot fill the surround channels, so there is no exclusive
        // stream and the caller downgrades to shared mode.
        let surround = DeviceFormat {
            sample_rate: 48_000,
            channels: 6,
            layout: SampleLayout::Float32,
        };
        assert!(plan(source(), surround, &[surround], 100_000, 50_000).is_none());
    }

    #[test]
    fn the_period_is_the_device_default_when_it_is_above_the_minimum() {
        // 10 ms at 48 kHz is 480 frames, which is a multiple of 32 (the
        // 128-byte alignment of a 32-bit stereo stream).
        let period = choose_period(100_000, 50_000, 48_000, 8);
        assert_eq!(period, 100_000);
    }

    #[test]
    fn the_period_is_never_below_the_device_minimum() {
        // The device wants 5 ms at least, the caller asks for 1 ms.
        let period = choose_period(10_000, 50_000, 48_000, 8);
        assert!(
            period >= 50_000,
            "the period is {period}, below the minimum"
        );
        assert_eq!(period, 50_000);
    }

    #[test]
    fn the_period_is_aligned_to_the_buffer_alignment() {
        // 24-bit stereo has a 6-byte frame; the driver asks for multiples of
        // 128 bytes, so the buffer has to be a multiple of 384 bytes, which
        // is 64 frames.
        let period = choose_period(100_000, 50_000, 48_000, 6);
        let frames = period * 48_000 / 10_000_000;
        assert_eq!(frames % 64, 0, "the period is {frames} frames");
        assert!(period >= 100_000, "the period grew, it did not shrink");
    }

    #[test]
    fn the_period_of_a_minimum_requirement_is_rounded_up_to_whole_frames() {
        // 1.5 ms at 48 kHz is 72 frames, which is not a multiple of 16.
        let period = choose_period(15_000, 15_000, 48_000, 8);
        let frames = period * 48_000 / 10_000_000;
        assert!(
            frames >= 72,
            "the period holds at least 72 frames, got {frames}"
        );
        assert_eq!(frames % 16, 0, "the period is {frames} frames");
    }

    #[test]
    fn the_period_is_never_below_the_minimum_period_of_the_device() {
        // A device that wants 3 ms at least, at a rate where 3 ms is not a
        // whole number of frames.
        for rate in [44_100u32, 48_000, 96_000] {
            let period = choose_period(1_000, 30_000, rate, 8);
            assert!(
                period >= 30_000,
                "at {rate} Hz the period is {period}, below the minimum"
            );
            let frames = period * i64::from(rate) / 10_000_000;
            assert!(
                frames * 10_000_000 <= period * i64::from(rate),
                "the derived frame count is consistent"
            );
        }
    }

    #[test]
    fn a_float_sample_is_written_as_its_own_bytes() {
        let samples = [0.0f32, 0.5, -0.5, 1.0];
        let mut out = vec![0u8; samples.len() * 4];
        write_samples(&samples, SampleLayout::Float32, &mut out);
        let read: Vec<f32> = out
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        assert_eq!(read, samples, "a float stream is bit-exact");
    }

    #[test]
    fn a_16_bit_sample_is_scaled_and_clamped() {
        let samples = [0.0f32, 1.0, -1.0, 2.0, -2.0];
        let mut out = vec![0u8; samples.len() * 2];
        write_samples(&samples, SampleLayout::Int16, &mut out);
        let read: Vec<i16> = out
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect();
        assert_eq!(read, vec![0, 32_767, -32_767, 32_767, -32_768]);
    }

    #[test]
    fn a_24_bit_sample_is_packed_in_three_bytes() {
        let samples = [0.0f32, 1.0, -1.0];
        let mut out = vec![0u8; samples.len() * 3];
        write_samples(&samples, SampleLayout::Int24Packed, &mut out);
        assert_eq!(&out[0..3], &[0, 0, 0], "silence is zero");
        assert_eq!(&out[3..6], &[0xff, 0xff, 0x7f], "full scale is 0x7fffff");
        assert_eq!(
            &out[6..9],
            &[0x01, 0x00, 0x80],
            "minus full scale is 0x800001"
        );
    }

    #[test]
    fn a_24_bit_sample_in_a_32_bit_container_is_left_aligned() {
        let samples = [1.0f32];
        let mut out = vec![0u8; 4];
        write_samples(&samples, SampleLayout::Int24Padded, &mut out);
        // 0x7fffff shifted left by eight bits.
        assert_eq!(out, vec![0x00, 0xff, 0xff, 0x7f]);
    }

    #[test]
    fn a_32_bit_sample_is_scaled_and_clamped() {
        let samples = [1.0f32, -1.0f32, 0.0f32];
        let mut out = vec![0u8; samples.len() * 4];
        write_samples(&samples, SampleLayout::Int32, &mut out);
        let read: Vec<i32> = out
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| i32::from_le_bytes(*c))
            .collect();
        assert_eq!(read, vec![2_147_483_647, -2_147_483_647, 0]);
    }

    #[test]
    fn a_non_finite_sample_does_not_become_noise() {
        let samples = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];
        let mut out = vec![0u8; samples.len() * 2];
        write_samples(&samples, SampleLayout::Int16, &mut out);
        let read: Vec<i16> = out
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect();
        assert_eq!(read, vec![0, 32_767, -32_768]);
    }
}
