//! Format conversion on the decode path: engine format -> output format.
//!
//! The output device may grant a different sample rate or channel count than
//! the decoder produced (RFC 0001: "the device may grant another rate/channels;
//! the engine adapts"). The converter sits between the DSP chain and the ring,
//! so the ring always holds audio in the **device** format and the render
//! callback never resamples.
//!
//! * Sample rate: linear interpolation with a fractional phase that carries
//!   across blocks, so the output is continuous. Equal sample rates take an
//!   exact remix path instead (no interpolation, no held-back frame). A
//!   higher-quality resampler (`rubato`) is an M2 item.
//! * Channels: mono -> stereo duplicates, stereo -> mono averages, extra input
//!   channels are dropped, missing output channels are zero-filled.
//!
//! `convert` allocates nothing; the caller owns both buffers.

use rustamp_core::audio::AudioSpec;

/// Converts interleaved `f32` frames from one [`AudioSpec`] to another.
#[derive(Debug, Clone)]
pub struct FormatConverter {
    src: AudioSpec,
    dst: AudioSpec,
    /// Source frames advanced per output frame.
    step: f64,
    /// Read position inside the extended input block (see [`Self::convert`]).
    /// `0` is the last frame of the previous block, `1` is the first frame of
    /// the current block.
    phase: f64,
    /// Last source frame of the previous block, for interpolation across
    /// blocks. `src.channels` values.
    prev: Vec<f32>,
    /// True when no resampling is needed (only a possible channel remap).
    same_rate: bool,
}

impl FormatConverter {
    /// A converter for `src -> dst`.
    pub fn new(src: AudioSpec, dst: AudioSpec) -> Self {
        let mut this = FormatConverter {
            src: AudioSpec::new(0, 0),
            dst,
            step: 1.0,
            phase: 1.0,
            prev: Vec::new(),
            same_rate: false,
        };
        this.prepare(src, dst);
        this
    }

    /// (Re)configure for a new pair of formats. Called at track start and on a
    /// mid-stream format change; may allocate.
    pub fn prepare(&mut self, src: AudioSpec, dst: AudioSpec) {
        let src = AudioSpec::new(src.sample_rate.max(1), src.channels.max(1));
        let dst = AudioSpec::new(dst.sample_rate.max(1), dst.channels.max(1));
        self.src = src;
        self.dst = dst;
        self.step = f64::from(src.sample_rate) / f64::from(dst.sample_rate);
        self.phase = 1.0;
        self.prev = vec![0.0; usize::from(src.channels)];
        self.same_rate = src.sample_rate == dst.sample_rate;
    }

    /// Formats in use.
    pub fn specs(&self) -> (AudioSpec, AudioSpec) {
        (self.src, self.dst)
    }

    /// Upper bound of output frames for `in_frames` input frames.
    pub fn max_out_frames(&self, in_frames: usize) -> usize {
        if self.same_rate {
            return in_frames;
        }
        // One extra frame for the carried phase plus one for interpolation.
        (in_frames as f64 / self.step).ceil() as usize + 2
    }

    /// Maximum number of output frames a single call can need for `in_frames`.
    pub fn capacity_frames(&self, in_frames: usize) -> usize {
        self.max_out_frames(in_frames)
    }

    /// Clear the interpolation state (after a seek): the next output frame is
    /// again the first frame of the next input block.
    pub fn reset(&mut self) {
        self.phase = 1.0;
        self.prev.iter_mut().for_each(|v| *v = 0.0);
    }

    /// Convert `input` (whole frames in the source format) into `out` (device
    /// frames). Returns the number of output frames written; `out` must hold at
    /// least [`Self::max_out_frames`] frames.
    ///
    /// The last input frame of the block is held back when interpolation would
    /// need the frame after it; it is used at the start of the next call. That
    /// keeps the output identical no matter how the input is split into blocks.
    ///
    /// Allocation-free.
    pub fn convert(&mut self, input: &[f32], out: &mut [f32]) -> usize {
        let in_ch = usize::from(self.src.channels);
        let out_ch = usize::from(self.dst.channels);
        let in_frames = input.len() / in_ch;
        if in_frames == 0 {
            return 0;
        }
        let out_capacity = out.len() / out_ch;
        if self.same_rate {
            let n = in_frames.min(out_capacity);
            remix(&input[..n * in_ch], &mut out[..n * out_ch], in_ch, out_ch);
            return n;
        }

        // The extended input block is `prev ++ input`: index 0 is the last frame
        // of the previous block, index k >= 1 is `input[k - 1]`.
        let step = self.step;
        let mut pos = self.phase;
        let mut written = 0usize;
        while pos < in_frames as f64 && written < out_capacity {
            let idx = pos.floor() as usize;
            let frac = (pos - idx as f64) as f32;
            let a = frame_at(&self.prev, input, in_ch, idx);
            let b = frame_at(&self.prev, input, in_ch, idx + 1);
            for c in 0..out_ch {
                let av = map_channel(a, in_ch, c);
                let bv = map_channel(b, in_ch, c);
                out[written * out_ch + c] = av + (bv - av) * frac;
            }
            written += 1;
            pos += step;
        }
        // Carry the position (and the last frame) into the next block.
        self.phase = pos - in_frames as f64;
        self.prev.copy_from_slice(&input[(in_frames - 1) * in_ch..]);
        written
    }
}

#[inline]
fn frame_at<'a>(prev: &'a [f32], input: &'a [f32], in_ch: usize, idx: usize) -> &'a [f32] {
    if idx == 0 {
        prev
    } else {
        let start = (idx - 1) * in_ch;
        &input[start..start + in_ch]
    }
}

#[inline]
fn map_channel(frame: &[f32], in_ch: usize, out_channel: usize) -> f32 {
    if in_ch == 1 {
        return frame[0];
    }
    frame.get(out_channel).copied().unwrap_or(0.0)
}

/// Channel remap without resampling.
fn remix(input: &[f32], out: &mut [f32], in_ch: usize, out_ch: usize) {
    if in_ch == out_ch {
        out.copy_from_slice(&input[..out.len()]);
        return;
    }
    for (i, frame) in input.chunks_exact(in_ch).enumerate() {
        let dst = &mut out[i * out_ch..(i + 1) * out_ch];
        if in_ch == 1 {
            dst.fill(frame[0]);
        } else if out_ch == 1 {
            let sum: f32 = frame.iter().sum();
            dst[0] = sum / in_ch as f32;
        } else {
            for (c, v) in dst.iter_mut().enumerate() {
                *v = frame.get(c).copied().unwrap_or(0.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp(n: usize) -> Vec<f32> {
        (0..n).map(|i| i as f32).collect()
    }

    #[test]
    fn same_format_is_a_bit_exact_passthrough() {
        let spec = AudioSpec::new(44100, 2);
        let mut c = FormatConverter::new(spec, spec);
        let input = ramp(20);
        let mut out = vec![0.0; c.max_out_frames(10) * 2];
        let n = c.convert(&input, &mut out);
        assert_eq!(n, 10);
        assert_eq!(&out[..20], &input[..]);
    }

    #[test]
    fn mono_to_stereo_duplicates() {
        let mut c = FormatConverter::new(AudioSpec::new(8000, 1), AudioSpec::new(8000, 2));
        let input = ramp(4);
        let mut out = vec![0.0; 16];
        let n = c.convert(&input, &mut out);
        assert_eq!(n, 4);
        assert_eq!(&out[..8], &[0.0, 0.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0]);
    }

    #[test]
    fn stereo_to_mono_averages() {
        let mut c = FormatConverter::new(AudioSpec::new(8000, 2), AudioSpec::new(8000, 1));
        let input = vec![1.0, 3.0, 2.0, 4.0];
        let mut out = vec![0.0; 8];
        let n = c.convert(&input, &mut out);
        assert_eq!(n, 2);
        assert_eq!(&out[..2], &[2.0, 3.0]);
    }

    #[test]
    fn upsample_2x_interpolates_the_midpoint() {
        // 4 kHz -> 8 kHz: output frame 2k is input frame k, 2k+1 the midpoint.
        // The last frame is held back for the next block (no lookahead loss).
        let mut c = FormatConverter::new(AudioSpec::new(4000, 1), AudioSpec::new(8000, 1));
        let input = vec![0.0, 10.0, 20.0, 30.0];
        let mut out = vec![0.0; c.max_out_frames(4)];
        let n = c.convert(&input, &mut out);
        assert_eq!(n, 6);
        assert_eq!(&out[..6], &[0.0, 5.0, 10.0, 15.0, 20.0, 25.0]);
        // The held-back frame comes out of the next block.
        let mut out2 = vec![0.0; c.max_out_frames(1)];
        let n2 = c.convert(&[40.0], &mut out2);
        assert_eq!(n2, 2);
        assert_eq!(&out2[..2], &[30.0, 35.0]);
    }

    #[test]
    fn downsample_2x_keeps_every_other_frame() {
        let mut c = FormatConverter::new(AudioSpec::new(8000, 1), AudioSpec::new(4000, 1));
        let input = ramp(8);
        let mut out = vec![0.0; c.max_out_frames(8)];
        let n = c.convert(&input, &mut out);
        assert_eq!(n, 4);
        assert_eq!(&out[..4], &[0.0, 2.0, 4.0, 6.0]);
    }

    #[test]
    fn phase_carries_across_blocks() {
        // 3:2 -> output frame k needs input frame 1.5k. Feed one frame at a
        // time and compare with a single call.
        let src = AudioSpec::new(3000, 1);
        let dst = AudioSpec::new(2000, 1);
        let input = ramp(12);

        let mut whole = FormatConverter::new(src, dst);
        let mut whole_out = vec![0.0; whole.max_out_frames(12)];
        let whole_n = whole.convert(&input, &mut whole_out);

        let mut split = FormatConverter::new(src, dst);
        let mut split_out = Vec::new();
        for chunk in input.chunks(1) {
            let mut buf = vec![0.0; split.max_out_frames(1)];
            let n = split.convert(chunk, &mut buf);
            split_out.extend_from_slice(&buf[..n]);
        }
        assert_eq!(whole_n, split_out.len());
        for (i, (a, b)) in whole_out[..whole_n]
            .iter()
            .zip(split_out.iter())
            .enumerate()
        {
            assert!((a - b).abs() < 1e-4, "frame {i}: {a} != {b}");
        }
    }

    #[test]
    fn reset_clears_the_history() {
        let mut c = FormatConverter::new(AudioSpec::new(4000, 1), AudioSpec::new(8000, 1));
        let mut out = vec![0.0; 16];
        c.convert(&[7.0, 9.0], &mut out);
        c.reset();
        // A fresh block after the reset starts at its own first frame.
        let mut out2 = vec![0.0; 16];
        let n = c.convert(&[0.0, 0.0], &mut out2);
        assert_eq!(n, 2);
        assert_eq!(&out2[..2], &[0.0, 0.0], "no leak from the previous block");
    }
}
