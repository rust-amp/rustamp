// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Seeded property tests for the volume/balance math and the sample finishing.
//!
//! The unit tests next to [`crate::gain`] pin hand-written cases: a volume of
//! `0`, a balance at the centre, a hard left. A property test asks the other
//! question — *does the rule hold for every input?* — and it needs many
//! inputs to answer it.
//!
//! The inputs come from a small seeded generator, so a failure is
//! reproducible: the seed is part of the failure message, and the same seed
//! draws the same case on every run and on every machine. The crate takes no
//! dependency for it.
//!
//! # The documented mapping
//!
//! [`crate::gain`] is the one place where the control values become gains.
//! Its module docs are the contract. S07 §5 and §6 close the *slider* law
//! (five core units per pixel, the display percentage `floor(v * 100 / 255)`)
//! but leave the *engine-side* law open, so the law below is the crate's own
//! and provisional one:
//!
//! * **Volume** `u8` `0..=255` -> the linear gain `volume / 255`: `0` is
//!   silence, `255` is unity.
//! * **Balance** `i8` `-127..=127` -> `(left, right)`: `0` leaves both
//!   channels alone, `-127` mutes the right channel and `+127` the left one,
//!   linearly in between. Attenuation only, never a boost. Negative is left,
//!   as the UI's `balance_percent` agrees (`-127` is the display's `0`).
//! * **Mono** output cannot pan: the mean of the two channel gains is used.
//! * **More than two channels:** the even indices take the left gain, the odd
//!   ones the right gain.
//!
//! The converter's `finish_sample` is the last level decision of the chain
//! on the resampled path: a non-finite sample becomes silence, a subnormal
//! becomes zero, and a sample above full scale is clamped to `±1.0`.
//!
//! # The invariants
//!
//! ## The volume
//!
//! * **Zero is exactly silence.** A volume of `0` leaves every sample of
//!   every channel at exactly zero, whatever the balance and the channel
//!   count are.
//! * **The maximum is exactly unity.** A volume of `255` leaves the buffer
//!   bit for bit as it went in.
//! * **The law is linear and monotone.** The gain is `volume / 255`, it grows
//!   strictly with the slider, and it never leaves `0..=1`: the stage
//!   attenuates, it never amplifies past the documented maximum.
//!
//! ## The balance
//!
//! * **The centre is exactly unity** on both channels.
//! * **A hard left silences the right channel** exactly and leaves the left
//!   one alone; a hard right is the mirror image.
//! * **The law is linear and attenuates only**: the moved channel's gain is
//!   `1 - |balance| / 127`, the other one stays at `1.0`, and neither ever
//!   exceeds `1.0`.
//! * **A balance outside the range is clamped**, not wrapped.
//!
//! ## The stage
//!
//! * **Nothing non-finite leaves it.** A finite input gives a finite output,
//!   over the whole finite range of `f32` and not only over the audio
//!   domain.
//! * **It never amplifies.** The absolute value of a sample never grows.
//! * **It applies the documented gain per channel**: the left gain on the
//!   even channels, the right gain on the odd ones, the mean of the two on a
//!   mono stream.
//!
//! ## The finishing
//!
//! * **A non-finite sample finishes as exactly zero**, not as a small number
//!   and not as a NaN.
//! * **The finishing clamps without wrapping**: a sample above full scale
//!   comes out at exactly `1.0`, one below at exactly `-1.0`.
//! * **A subnormal sample finishes as zero.**
//! * **A finite sample inside the range passes through bit for bit.**
//! * **The finishing is monotone and idempotent**, and it never leaves a
//!   non-finite sample behind.
//!
//! # What is not claimed
//!
//! * The gain stage **multiplies; it does not sanitize**. A non-finite
//!   *input* stays non-finite on the way out, because the stage's policy is
//!   "a gain", and the non-finite policy of the chain lives at its input
//!   (the decode thread, RFC 0014 §4.5) and in the converter's finishing.
//!   Every property of the gain stage is therefore stated for a finite
//!   input. The same convention holds for the DSP stages (see `rustamp-dsp`'s
//!   property suite).
//! * The finishing runs on the **resampled path only**; the equal-rate path
//!   is a bit-exact copy (`crate::convert`). The missing clamp of the
//!   equal-rate path is the open finding F2 of
//!   `docs/tasks/W14-wave-review.md`, not a property of this suite.
//! * The stage says nothing about a buffer whose length is not a multiple of
//!   the channel count: the render callback always hands it whole frames.

/// How many random scenarios one property runs.
const CASES: usize = 512;

// ---------------------------------------------------------------------------
// The seeded generator
// ---------------------------------------------------------------------------

/// A seeded xorshift64 generator.
///
/// The property suite must be reproducible from its seed (a failure has to
/// be replayable) and the crate takes no dependency, so the generator lives
/// here. It is not a quality generator: it only has to spread the drawn
/// values evenly enough for the properties to mean something.
struct Rng(u64);

impl Rng {
    /// A generator for `seed`. The state must not be zero.
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    /// The next 64 random bits.
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// A uniform value in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u32 << 24) as f32
    }

    /// A uniform value in `[lo, hi)`.
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// A uniform index in `0..n`.
    fn index(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    /// A random core volume.
    fn volume(&mut self) -> u8 {
        self.index(256) as u8
    }

    /// A random core balance.
    fn balance(&mut self) -> i8 {
        (self.index(255) as i16 - 127) as i8
    }

    /// A random channel count: mono, stereo, or a layout with more channels.
    fn channels(&mut self) -> u16 {
        1 + self.index(4) as u16
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The bit patterns of a buffer, for an exact comparison.
fn bits(buf: &[f32]) -> Vec<u32> {
    buf.iter().map(|s| s.to_bits()).collect()
}

/// A random *finite* signal: the audio domain plus the edges of the `f32`
/// range, so that the "nothing non-finite leaves" properties cover the whole
/// finite range and not only the audio domain.
fn random_finite_signal(rng: &mut Rng, samples: usize) -> Vec<f32> {
    let mut out: Vec<f32> = (0..samples).map(|_| rng.range(-1.0, 1.0)).collect();
    for (i, edge) in [
        0.0_f32,
        -0.0,
        1.0,
        -1.0,
        f32::MIN_POSITIVE,
        f32::MAX,
        f32::MIN,
        -f32::MAX,
    ]
    .into_iter()
    .enumerate()
    {
        if let Some(slot) = out.get_mut(i * 13) {
            *slot = edge;
        }
    }
    out
}

/// A random finite `f32` over the whole finite range: the bit pattern is
/// drawn at random and a non-finite draw is refused.
fn random_finite(rng: &mut Rng) -> f32 {
    loop {
        let sample = f32::from_bits(rng.next_u64() as u32);
        if sample.is_finite() {
            return sample;
        }
    }
}

// ---------------------------------------------------------------------------
// The volume
// ---------------------------------------------------------------------------

/// The properties of the volume mapping.
mod volume {
    use super::{CASES, Rng, bits, random_finite_signal};
    use crate::gain::{apply, volume_gain};

    /// The volume law is the documented linear one, and its two ends are
    /// exact: `0` is exactly zero and `255` is exactly one.
    #[test]
    fn the_volume_mapping_is_linear_with_exact_ends() {
        assert_eq!(volume_gain(0).to_bits(), 0.0f32.to_bits());
        assert_eq!(volume_gain(255).to_bits(), 1.0f32.to_bits());
        for volume in 0..=u8::MAX {
            let gain = f64::from(volume_gain(volume));
            let law = f64::from(volume) / 255.0;
            assert!(
                (gain - law).abs() <= 1e-7,
                "volume {volume}: the gain {gain} is not the law {law}"
            );
        }
    }

    /// The mapping grows strictly with the slider and never leaves
    /// `0..=1`: the stage attenuates, it never amplifies past the documented
    /// maximum of unity.
    #[test]
    fn the_volume_mapping_is_monotone_and_never_amplifies() {
        let mut last = f32::NEG_INFINITY;
        for volume in 0..=u8::MAX {
            let gain = volume_gain(volume);
            assert!(
                gain > last,
                "volume {volume} is not louder than the volume below it"
            );
            assert!(
                (0.0..=1.0).contains(&gain),
                "volume {volume}: the gain {gain} leaves 0..=1"
            );
            last = gain;
        }
    }

    /// A volume of `0` is exactly silence: every sample of every channel
    /// comes out as zero, whatever the balance and the channel count are.
    #[test]
    fn a_volume_of_zero_is_exactly_silence() {
        let mut rng = Rng::new(0xA010_0001);
        for _ in 0..CASES {
            let channels = rng.channels();
            let balance = rng.balance();
            let frames = 1 + rng.index(16);
            let mut buf = random_finite_signal(&mut rng, frames * usize::from(channels));
            apply(&mut buf, channels, 0, balance);
            for (i, sample) in buf.iter().enumerate() {
                assert_eq!(*sample, 0.0, "sample {i} is {sample}, not silence");
            }
        }
    }

    /// A volume of `255` is exactly unity: the buffer comes out bit for bit
    /// as it went in.
    #[test]
    fn a_volume_of_maximum_is_exactly_unity() {
        let mut rng = Rng::new(0xA010_0002);
        for _ in 0..CASES {
            let channels = rng.channels();
            let frames = 1 + rng.index(16);
            let mut buf = random_finite_signal(&mut rng, frames * usize::from(channels));
            let before = bits(&buf);
            apply(&mut buf, channels, 255, 0);
            assert_eq!(bits(&buf), before, "unity must not touch a sample");
        }
    }
}

// ---------------------------------------------------------------------------
// The balance
// ---------------------------------------------------------------------------

/// The properties of the balance mapping.
mod balance {
    use super::{CASES, Rng, bits, random_finite_signal};
    use crate::gain::{apply, balance_gains};

    /// The balance at the centre leaves both channels exactly alone.
    #[test]
    fn a_balance_at_the_centre_is_exactly_unity() {
        assert_eq!(balance_gains(0), (1.0, 1.0));
        let mut rng = Rng::new(0xBA1A_0001);
        for _ in 0..CASES {
            let channels = rng.channels();
            let frames = 1 + rng.index(16);
            let mut buf = random_finite_signal(&mut rng, frames * usize::from(channels));
            let before = bits(&buf);
            apply(&mut buf, channels, 255, 0);
            assert_eq!(bits(&buf), before, "the centre must not touch a sample");
        }
    }

    /// A hard left mutes the right channel exactly and leaves the left one
    /// alone; a hard right is the mirror image.
    #[test]
    fn a_hard_side_silences_the_other_channel() {
        assert_eq!(balance_gains(-127), (1.0, 0.0), "a hard left");
        assert_eq!(balance_gains(127), (0.0, 1.0), "a hard right");

        let mut rng = Rng::new(0xBA1A_0002);
        for _ in 0..CASES {
            let frames = 1 + rng.index(16);
            let input = random_finite_signal(&mut rng, frames * 2);

            let mut left = input.clone();
            apply(&mut left, 2, 255, -127);
            let (left_frames, _) = left.as_chunks::<2>();
            let (input_frames, _) = input.as_chunks::<2>();
            for (frame, original) in left_frames.iter().zip(input_frames.iter()) {
                assert_eq!(
                    frame[0].to_bits(),
                    original[0].to_bits(),
                    "a hard left must leave the left channel alone"
                );
                assert_eq!(frame[1], 0.0, "a hard left must silence the right channel");
            }

            let mut right = input.clone();
            apply(&mut right, 2, 255, 127);
            let (right_frames, _) = right.as_chunks::<2>();
            for (frame, original) in right_frames.iter().zip(input_frames.iter()) {
                assert_eq!(
                    frame[1].to_bits(),
                    original[1].to_bits(),
                    "a hard right must leave the right channel alone"
                );
                assert_eq!(frame[0], 0.0, "a hard right must silence the left channel");
            }
        }
    }

    /// The balance law is linear and attenuates only: the moved channel's
    /// gain is `1 - |balance| / 127`, the other one stays at `1.0`, and
    /// neither ever exceeds `1.0`.
    #[test]
    fn the_balance_mapping_is_linear_and_attenuates_only() {
        for balance in -127..=127i8 {
            let (left, right) = balance_gains(balance);
            let moved = 1.0 - f64::from(balance.unsigned_abs()) / 127.0;
            let (expected_left, expected_right) = if balance < 0 {
                (1.0, moved)
            } else {
                (moved, 1.0)
            };
            assert!(
                (f64::from(left) - expected_left).abs() <= 1e-7,
                "balance {balance}: the left gain {left} is not {expected_left}"
            );
            assert!(
                (f64::from(right) - expected_right).abs() <= 1e-7,
                "balance {balance}: the right gain {right} is not {expected_right}"
            );
            assert!(
                (0.0..=1.0).contains(&left) && (0.0..=1.0).contains(&right),
                "balance {balance}: ({left}, {right}) leaves 0..=1"
            );
        }
    }

    /// A balance outside the range is clamped, not wrapped: the only value
    /// `i8` can hold outside `-127..=127` is `-128`.
    #[test]
    fn a_balance_outside_the_range_is_clamped() {
        assert_eq!(balance_gains(-128), balance_gains(-127));
    }
}

// ---------------------------------------------------------------------------
// The stage
// ---------------------------------------------------------------------------

/// The properties of the in-place application.
mod stage {
    use super::{CASES, Rng, random_finite_signal};
    use crate::gain::{apply, channel_gains};

    /// Nothing non-finite leaves the stage: a finite input and any setting
    /// give a finite output, over the whole finite range of `f32` and not
    /// only over the audio domain.
    #[test]
    fn nothing_non_finite_leaves_the_stage() {
        let mut rng = Rng::new(0x5746_0001);
        for _ in 0..CASES {
            let channels = rng.channels();
            let volume = rng.volume();
            let balance = rng.balance();
            let frames = 1 + rng.index(16);
            let mut buf = random_finite_signal(&mut rng, frames * usize::from(channels));
            apply(&mut buf, channels, volume, balance);
            for (i, sample) in buf.iter().enumerate() {
                assert!(sample.is_finite(), "sample {i} left as {sample}");
            }
        }
    }

    /// The stage never amplifies: the absolute value of a sample never
    /// grows.
    #[test]
    fn the_stage_never_amplifies() {
        let mut rng = Rng::new(0x5746_0002);
        for _ in 0..CASES {
            let channels = rng.channels();
            let volume = rng.volume();
            let balance = rng.balance();
            let frames = 1 + rng.index(16);
            let input = random_finite_signal(&mut rng, frames * usize::from(channels));
            let mut buf = input.clone();
            apply(&mut buf, channels, volume, balance);
            for (i, (out, &inp)) in buf.iter().zip(input.iter()).enumerate() {
                assert!(
                    out.abs() <= inp.abs(),
                    "sample {i} grew from {inp} to {out}"
                );
            }
        }
    }

    /// The stage applies the documented gain per channel: the left gain on
    /// the even channels, the right gain on the odd ones, and the mean of
    /// the two on a mono stream.
    #[test]
    fn the_stage_applies_the_documented_gain_per_channel() {
        let mut rng = Rng::new(0x5746_0003);
        for _ in 0..CASES {
            let channels = rng.channels();
            let volume = rng.volume();
            let balance = rng.balance();
            let (left, right) = channel_gains(volume, balance, channels);
            let mut buf = vec![1.0_f32; usize::from(channels) * 4];
            apply(&mut buf, channels, volume, balance);
            for (i, sample) in buf.iter().enumerate() {
                // The gain belongs to the channel inside the frame: the left
                // gain on the even channels, the right gain on the odd ones.
                // A mono stream carries the mean of the two gains, which is
                // what `channel_gains` puts in both of its slots.
                let expected = if i % usize::from(channels) % 2 == 0 {
                    left
                } else {
                    right
                };
                assert_eq!(
                    *sample, expected,
                    "channel {i} of {channels}: {sample} is not {expected}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The finishing
// ---------------------------------------------------------------------------

/// The properties of the chain's sample finishing.
mod finishing {
    use super::{Rng, random_finite};
    use crate::convert::{FormatConverter, finish_sample};
    use rustamp_core::audio::AudioSpec;

    /// A sample that is not a number, and an infinity, finish as exactly
    /// zero: not as a small number, and not as a NaN.
    #[test]
    fn a_non_finite_sample_finishes_as_exactly_zero() {
        for sample in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let out = finish_sample(sample);
            assert_eq!(
                out.to_bits(),
                0.0f32.to_bits(),
                "{sample} finished as {out}"
            );
        }
        // The same over random bit patterns: every non-finite input ends up
        // as exactly zero.
        let mut rng = Rng::new(0xF11A_0001);
        for _ in 0..4096 {
            let sample = f32::from_bits(rng.next_u64() as u32);
            if !sample.is_finite() {
                let out = finish_sample(sample);
                assert_eq!(
                    out.to_bits(),
                    0.0f32.to_bits(),
                    "{sample} finished as {out}"
                );
            }
        }
    }

    /// The finishing clamps without wrapping: a sample above full scale
    /// comes out at exactly `1.0`, one below at exactly `-1.0`. It never
    /// wraps around to the other end of the range.
    #[test]
    fn the_finishing_clamps_without_wrapping() {
        for sample in [2.0_f32, 1.000_000_1, 1e30, f32::MAX, -2.0, -1e30, f32::MIN] {
            let out = finish_sample(sample);
            let expected = if sample > 0.0 { 1.0f32 } else { -1.0f32 };
            assert_eq!(
                out.to_bits(),
                expected.to_bits(),
                "{sample} finished as {out}"
            );
        }
    }

    /// A subnormal sample finishes as zero: it is below the noise floor of
    /// the format, and the chain does not carry it.
    #[test]
    fn a_subnormal_sample_finishes_as_zero() {
        for sample in [
            f32::MIN_POSITIVE / 2.0,
            f32::from_bits(1),
            f32::from_bits(2),
            -f32::from_bits(1),
        ] {
            assert!(sample.is_subnormal(), "{sample} is not subnormal");
            assert_eq!(
                finish_sample(sample).to_bits(),
                0.0f32.to_bits(),
                "{sample} did not finish as zero"
            );
        }
    }

    /// A finite sample inside the range passes through bit for bit.
    #[test]
    fn a_finite_in_band_sample_passes_through_bit_for_bit() {
        for sample in [0.0_f32, -0.0, 1.0, -1.0, 0.5, -0.5] {
            assert_eq!(
                finish_sample(sample).to_bits(),
                sample.to_bits(),
                "{sample} did not pass through"
            );
        }
        let mut rng = Rng::new(0xF11A_0002);
        for _ in 0..4096 {
            let sample = rng.range(-1.0, 1.0);
            assert_eq!(
                finish_sample(sample).to_bits(),
                sample.to_bits(),
                "{sample} did not pass through"
            );
        }
    }

    /// The finishing is monotone: a louder sample never finishes quieter.
    /// It is idempotent, and it never leaves a non-finite sample behind.
    ///
    /// The monotonicity holds over the whole finite range, the subnormal
    /// rule included: a subnormal collapses to zero, and zero lies between
    /// the negative and the positive normal samples, so the order of two
    /// samples survives the rule.
    #[test]
    fn the_finishing_is_monotone_idempotent_and_finite() {
        let mut rng = Rng::new(0xF11A_0003);
        for _ in 0..4096 {
            let a = random_finite(&mut rng);
            let b = random_finite(&mut rng);
            let (low, high) = if a <= b { (a, b) } else { (b, a) };
            assert!(
                finish_sample(low) <= finish_sample(high),
                "{low} finished above {high}"
            );
            let once = finish_sample(a);
            assert!(once.is_finite(), "{a} finished as {once}");
            assert_eq!(
                finish_sample(once).to_bits(),
                once.to_bits(),
                "{a} did not finish idempotently"
            );
        }
    }

    /// The subnormal rule collapses the subnormals to zero, which is the
    /// one place where the finishing touches a sample it would otherwise
    /// pass through.
    #[test]
    fn the_subnormal_rule_collapses_the_subnormals_to_zero() {
        let mut rng = Rng::new(0xF11A_0004);
        let mut seen = 0;
        while seen < 64 {
            let sample = f32::from_bits(rng.next_u64() as u32);
            if !sample.is_subnormal() {
                continue;
            }
            seen += 1;
            assert_eq!(
                finish_sample(sample).to_bits(),
                0.0f32.to_bits(),
                "{sample} did not finish as zero"
            );
        }
    }

    /// The resampled path really finishes its samples: a NaN, an infinity
    /// and a sample far above full scale in the input come out finite and
    /// inside `±1.0`.
    #[test]
    fn the_resampled_path_finishes_its_samples() {
        let src = AudioSpec::new(44_100, 2);
        let dst = AudioSpec::new(48_000, 2);
        let mut converter = FormatConverter::new(src, dst);
        let frames = 4096;
        let channels = usize::from(dst.channels);
        let mut input = vec![0.0_f32; frames * channels];
        for (i, sample) in input.iter_mut().enumerate() {
            *sample = (i as f32 * 0.05).sin();
        }
        // The hostile values are spread over the stream, so that none of
        // them is cut away by the resampler's start-up delay.
        for (frame, left, right) in [
            (0usize, f32::NAN, f32::INFINITY),
            (1024, 12.0, -12.0),
            (2048, f32::NEG_INFINITY, 8.0),
            (3072, -9.0, f32::NAN),
        ] {
            input[frame * channels] = left;
            input[frame * channels + 1] = right;
        }
        // The buffer holds samples, the bound is a number of frames.
        let mut out = vec![0.0_f32; converter.max_out_frames(frames) * channels];
        let written = converter.convert(&input, &mut out);
        let mut tail =
            vec![0.0_f32; converter.max_out_frames(2 * converter.latency_frames()) * channels];
        let tail_frames = converter.flush(&mut tail);
        let finished = out[..written].iter().chain(tail[..tail_frames].iter());
        for (i, sample) in finished.clone().enumerate() {
            assert!(sample.is_finite(), "sample {i} left as {sample}");
            assert!(sample.abs() <= 1.0, "sample {i} left as {sample}");
        }
        // The clamp really fires: the samples far above full scale come out
        // at exactly full scale.
        assert!(
            finished.clone().any(|sample| sample.abs() == 1.0),
            "the finishing never clamped"
        );
    }
}
