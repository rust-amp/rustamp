//! The fade against the buffer it is realised in: S18 §2.3's constraint.
//!
//! # The reference's constraint
//!
//! The audited spec S18 ("Output behaviors (fades, buffer, prebuffer)")
//! records in §2.3 the constraint the reference's DirectSound output plug-in
//! states in its own words:
//!
//! ```text
//! Some fade times are bigger than buffer length; in order to get what you
//! want, please increase buffer size to <N> ms.
//! ```
//!
//! The spec marks the consequence as a **reading** (§2.3): a fade that is
//! longer than the buffer length cannot be realised inside that buffer, and
//! the plug-in answers the conflict by naming the buffer size it would need.
//! It does not shorten the fade silently. What the plug-in then does with
//! the fade is **not** established (S18 §7, items 1–3: the curve, the
//! defaults and the fade unit's granularity are open), so the *rule* below is
//! ours, not the reference's.
//!
//! # The buffers of our pipeline
//!
//! Our fade is realised by the [`Fade`](rustamp_dsp::Fade) stage in the
//! **render callback** (see [`crate::ring`]): the ramp is applied to the
//! frames the callback hands to the device, after the ring and after the
//! volume/balance gain. Two buffers are involved:
//!
//! * the **device's own buffer** — [`OutputFormat::buffer_frames`], the buffer
//!   the device actually got. It is the counterpart of the reference's
//!   `cfg_buf_ms` (S18 §3), except that the device chooses it: our player
//!   does not ask the device for a buffer length (the app's `audio.buffer_ms`
//!   is not applied — see `docs/tasks/W14-wave-review.md` §5).
//! * the **engine's ring** — [`EngineConfig::ring_frames`], the buffer the
//!   audio waits in before the device pulls it. It is our own buffer, and it
//!   is what feeds the ramp.
//!
//! # The rule (our own choice)
//!
//! **A fade is never shortened silently, and a fade that does not fit the
//! buffer it is realised in is reported with the numbers.**
//!
//! 1. The engine computes what the requested fade needs at the rate the fade
//!    is realised at (the device's rate): `needed = round(ms * rate / 1000)`
//!    frames. A request of `0` ms needs no frames at all.
//! 2. It compares that with both buffers above. The **binding** one is the
//!    shorter of the two — the buffer the fade exceeds first.
//! 3. **The requested fade is kept in full.** The ramp is counted in frames
//!    of the output, not in buffers, so it crosses the device's callback
//!    boundary as often as it needs to, and the ring that feeds it is refilled
//!    by the decode thread while the ramp runs. Measured: a 500 ms fade on a
//!    371 ms ring is realised in full — the ramp reaches silence at exactly
//!    500 ms and the ring is refilled while it runs
//!    (`crates/rustamp-audio/tests/fade_budget.rs`).
//! 4. **The conflict is reported.** The engine hands the caller a
//!    [`FadeNote`] with the requested milliseconds, the available
//!    milliseconds of the binding buffer and the length a buffer would need to
//!    hold the whole fade. The caller shows it to the user; the player
//!    surfaces it as a note.
//!
//! # What the rule accepts in exchange
//!
//! The ramp is fed by the ring, so it can only fade the audio that is still
//! ahead of the device. **If the audio that remains is shorter than the
//! ramp, the ramp's tail falls on silence and the fade ends above zero** —
//! the music is cut instead of faded out. Measured: a 2000 ms fade with
//! 907 ms of audio left cuts the music at 74 % of unity
//! (`crates/rustamp-audio/tests/fade_budget.rs`). The note says so, because a
//! user who asks for a long fade-out on a short track deserves to know that
//! the fade will not reach silence.
//!
//! # What is *not* claimed here
//!
//! * The ramp law, the fade's default and the fade's placement are the
//!   engine's own choices (see [`crate::engine`]); S18 describes the
//!   reference's **output plug-in**, not the player, and leaves the player's
//!   own fade open (S18 §7, item 7).
//! * Nothing here is 2.95 behaviour. The reference's plug-in is a different
//!   piece of software with a different buffer, and its rule is a *reading*
//!   of one sentence in a string table.

use std::fmt;

/// The buffer a fade was compared against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeBuffer {
    /// The device's own buffer: the buffer the device actually got
    /// ([`OutputFormat::buffer_frames`](rustamp_core::audio::OutputFormat::buffer_frames)).
    Device,
    /// The engine's ring: the buffer the audio waits in before the device
    /// pulls it ([`EngineConfig::ring_frames`](crate::engine::EngineConfig::ring_frames)).
    Ring,
}

impl FadeBuffer {
    /// The name of the buffer, for a user-facing text.
    pub fn name(self) -> &'static str {
        match self {
            FadeBuffer::Device => "device's buffer",
            FadeBuffer::Ring => "engine's buffer",
        }
    }
}

impl fmt::Display for FadeBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// What a requested fade needs and the buffer it has to fit into.
///
/// The budget is the whole of the check: it counts the frames the fade needs
/// at the rate it is realised at, it knows the buffer's length in the same
/// unit, and it says whether the one fits the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FadeBudget {
    /// The requested fade, in milliseconds.
    requested_ms: u32,
    /// The rate the fade is realised at (the device's rate).
    sample_rate: u32,
    /// The buffer the fade is compared against.
    buffer: FadeBuffer,
    /// The buffer's length, in frames.
    available_frames: u64,
}

impl FadeBudget {
    /// The budget of a fade of `requested_ms` milliseconds in a buffer of
    /// `available_frames` frames, realised at `sample_rate`.
    ///
    /// A `sample_rate` of zero is read as one, exactly like the fade stage's
    /// own frame count does (`rustamp_dsp::Fade` clamps the rate the same
    /// way), so the budget and the ramp always agree on how long the fade is.
    pub fn new(
        requested_ms: u32,
        sample_rate: u32,
        buffer: FadeBuffer,
        available_frames: u64,
    ) -> Self {
        FadeBudget {
            requested_ms,
            sample_rate: sample_rate.max(1),
            buffer,
            available_frames,
        }
    }

    /// The requested fade, in milliseconds.
    pub fn requested_ms(&self) -> u32 {
        self.requested_ms
    }

    /// The buffer the fade is compared against.
    pub fn buffer(&self) -> FadeBuffer {
        self.buffer
    }

    /// The frames the requested fade needs at the rate.
    ///
    /// A request of `0` ms needs no frames: there is no ramp. Every other
    /// request needs at least one frame, so that a fade shorter than a frame
    /// is still a fade.
    ///
    /// The count rounds half up, exactly like the fade stage's own frame
    /// count (`rustamp_dsp::Fade`), so the budget and the ramp agree on how
    /// long the fade is.
    pub fn needed_frames(&self) -> u64 {
        if self.requested_ms == 0 {
            return 0;
        }
        let frames = u64::from(self.requested_ms)
            .saturating_mul(u64::from(self.sample_rate))
            .saturating_add(500);
        (frames / 1000).max(1)
    }

    /// The buffer's length, in frames.
    pub fn available_frames(&self) -> u64 {
        self.available_frames
    }

    /// The buffer's length in milliseconds, rounded down.
    ///
    /// The frame count is the truth; this is the number the user reads.
    pub fn available_ms(&self) -> u64 {
        self.available_frames.saturating_mul(1000) / u64::from(self.sample_rate)
    }

    /// Whether the requested fade fits the buffer.
    pub fn fits(&self) -> bool {
        self.needed_frames() <= self.available_frames
    }

    /// The buffer length that would hold the whole fade, in milliseconds.
    ///
    /// This is the number the reference's constraint sentence names ("please
    /// increase buffer size to `<N>` ms"): the buffer has to be at least as
    /// long as the fade. It is rounded up, because a buffer that is a
    /// fraction of a millisecond too short does not hold the fade.
    pub fn needed_ms(&self) -> u64 {
        let frames = self.needed_frames();
        let rate = u64::from(self.sample_rate);
        frames.saturating_mul(1000).div_ceil(rate)
    }

    /// The report for this budget, if the fade does not fit the buffer.
    pub fn note(&self) -> Option<FadeNote> {
        if self.fits() {
            return None;
        }
        Some(FadeNote {
            requested_ms: self.requested_ms,
            available_ms: self.available_ms(),
            needed_ms: self.needed_ms(),
            buffer: self.buffer,
        })
    }
}

/// The report the engine hands the caller when a fade does not fit the buffer
/// it is realised in.
///
/// The numbers are the point: the requested fade, the buffer that is
/// available and the buffer length that would hold the whole fade. A caller
/// that shows them to the user has said everything the reference's constraint
/// sentence says, with our own numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FadeNote {
    /// The fade that was requested, in milliseconds.
    pub requested_ms: u32,
    /// The length of the buffer the fade does not fit, in milliseconds.
    pub available_ms: u64,
    /// The buffer length that would hold the whole fade, in milliseconds.
    pub needed_ms: u64,
    /// The buffer the fade does not fit.
    pub buffer: FadeBuffer,
}

impl FadeNote {
    /// The user-facing text of the report.
    ///
    /// It names the three numbers and the consequence: the fade is played in
    /// full, and it only reaches silence if the audio lasts as long as the
    /// ramp.
    pub fn message(&self) -> String {
        format!(
            "the fade of {} ms does not fit the {} ({} ms): it is played in full, but a buffer of \
             {} ms would hold the whole fade; if the audio ends before the ramp does, the fade \
             ends above zero",
            self.requested_ms, self.buffer, self.available_ms, self.needed_ms
        )
    }
}

impl fmt::Display for FadeNote {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The budget of a fade in the engine's ring.
    fn ring(requested_ms: u32, rate: u32, frames: u64) -> FadeBudget {
        FadeBudget::new(requested_ms, rate, FadeBuffer::Ring, frames)
    }

    #[test]
    fn the_frames_are_rate_correct() {
        // 50 ms is 2205 frames at 44.1 kHz and 2400 frames at 48 kHz.
        assert_eq!(ring(50, 44_100, 16_384).needed_frames(), 2_205);
        assert_eq!(ring(50, 48_000, 16_384).needed_frames(), 2_400);
        // The same buffer is a different length in milliseconds at each rate.
        assert_eq!(ring(50, 44_100, 16_384).available_ms(), 371);
        assert_eq!(ring(50, 48_000, 16_384).available_ms(), 341);
    }

    #[test]
    fn a_fade_that_fits_is_no_conflict() {
        let budget = ring(50, 44_100, 16_384);
        assert!(budget.fits());
        assert_eq!(budget.note(), None);
    }

    #[test]
    fn a_fade_that_does_not_fit_is_a_conflict() {
        let budget = ring(500, 44_100, 16_384);
        assert!(!budget.fits());
        let note = budget.note().expect("a note");
        assert_eq!(note.requested_ms, 500);
        assert_eq!(note.available_ms, 371);
        assert_eq!(note.needed_ms, 500);
        assert_eq!(note.buffer, FadeBuffer::Ring);
    }

    #[test]
    fn a_zero_fade_needs_no_frames_and_is_no_conflict() {
        let budget = ring(0, 44_100, 16_384);
        assert_eq!(budget.needed_frames(), 0);
        assert!(budget.fits());
        assert_eq!(budget.note(), None);
    }

    #[test]
    fn a_fade_shorter_than_a_frame_still_needs_one() {
        // A rate of 100 Hz makes a millisecond a tenth of a frame: the fade
        // must not vanish.
        let budget = ring(1, 100, 1);
        assert_eq!(budget.needed_frames(), 1);
        assert!(budget.fits());
    }

    #[test]
    fn the_needed_buffer_is_rounded_up() {
        // 2205 frames at 44.1 kHz are exactly 50 ms.
        let exact = FadeBudget::new(50, 44_100, FadeBuffer::Device, 2_205);
        assert_eq!(exact.needed_ms(), 50);
        // A fade of 1 ms at 1500 Hz is 2 frames, and 2 frames are 1.33 ms:
        // the buffer has to be 2 ms to hold them.
        let rounded = FadeBudget::new(1, 1_500, FadeBuffer::Device, 2);
        assert_eq!(rounded.needed_frames(), 2);
        assert_eq!(rounded.needed_ms(), 2);
    }

    #[test]
    fn a_rate_of_zero_is_read_as_one_frame_per_millisecond() {
        // The fade stage clamps the rate the same way (`rustamp_dsp::Fade`),
        // so the budget and the ramp agree on how long the fade is.
        let budget = FadeBudget::new(500, 0, FadeBuffer::Ring, 16_384);
        assert_eq!(budget.needed_frames(), 1);
        assert_eq!(budget.available_ms(), 16_384_000);
    }

    #[test]
    fn the_message_names_the_numbers() {
        let note = ring(500, 44_100, 16_384).note().expect("a note");
        let message = note.message();
        assert!(message.contains("500 ms"), "{message}");
        assert!(message.contains("371 ms"), "{message}");
        assert!(message.contains("engine's buffer"), "{message}");
    }
}
