//! The crossfade between two tracks.
//!
//! **Everything here is an own choice.** Whether the reference crossfades at
//! all is not established (S18 §7, item 9), so nothing in this module may be
//! cited as 2.95 behaviour.
//!
//! # Where the two tracks are mixed, and why
//!
//! ```text
//! decoder -> ReplayGain -> equalizer -> [stages] -> limiter -> vis tap -> converter
//!         -> [overlap + hold-back] -> sum limiter -> ring -> volume/balance -> fade -> device
//! ```
//!
//! The mix sits **behind the converter**, at the point where the decode thread
//! hands device-format audio to the ring:
//!
//! * **No stage ever sees a seam.** The outgoing track's tail has left the DSP
//!   chain completely before the chain is prepared for the incoming track, so
//!   every stage runs one track at a time, exactly as at a gapless seam: it is
//!   never fed a jump, and it needs no state across the two tracks. A mix in
//!   front of the chain would need the chain to run in one fixed format, which
//!   would change what an installed plug-in is given.
//! * **Each side keeps its own path.** ReplayGain is a per-track constant, so it
//!   has to be applied before the tracks meet; each track also has its own
//!   rate and channel count, and the converter has brought both to the device
//!   format by this point. Two rates cannot be added before that.
//! * **The sum is limited.** Each side leaves the chain below the limiter's
//!   ceiling, but two of them add up to as much as `sqrt(2)` times that. A second
//!   [`Limiter`], following the same switch as the chain's, runs on the sum for
//!   as long as a crossfade is set: being in the path all the time, it has no
//!   entry and no exit to click at. It passes a signal below its ceiling
//!   unchanged. With the limiter switched off the sum is clamped to full scale,
//!   the same bound the converter holds on every other path.
//!
//! The vis tap sits in front of the mix, so it shows the current track.
//!
//! # The rules
//!
//! * **The curve** is the equal-power pair [`crossfade_gains`].
//! * **The hold-back.** Once a track is within the duration of its stated end,
//!   its device frames are kept back. Where the decoder really ends decides, not
//!   the stated length: a length that is too short only starts the hold-back
//!   early, and one that is too long leaves a shorter tail.
//! * **The length of a fade** is the shortest of the duration, the tail held back
//!   and half the incoming track's stated length, so a short track is not used up
//!   by its own fade-in: its second half is heard behind the outgoing track's end.
//!   What the fade does not cover of the tail plays first, at unity. The run's
//!   length is the sum of the tracks minus the fades.
//! * **An incoming track that ends early:** the rest of the tail keeps fading,
//!   over silence.
//! * **No fade at all** on the last track of the traversal, with "stop after
//!   current" armed, or on either side of a track that states no length (a
//!   stream, whose end cannot be held back and whose start cannot be read ahead
//!   of): these play to their end at unity and join plainly.
//! * **The moment the incoming track takes over** is the start of the overlap, in
//!   the decode thread, the same place as a gapless seam: `TrackChanged` is
//!   published then, one ring backlog ahead of the ear. The track's
//!   `timeline_start` is the overlap's first frame, so its position stays at `0`
//!   until that frame is heard and then counts the track's own frames: it never
//!   jumps when the fade ends. From this moment on, transport commands mean the
//!   incoming track.
//! * **A seek** cancels a running fade: the tail goes with the ring, and the track
//!   plays on from the target at unity.
//! * **A manual skip** never crossfades. It drops the tail and the hold-back and
//!   starts the target track the way it always has.
//! * **Pause** freezes the fade: it runs on the output's frames, not on a wall
//!   clock. **Stop** drops all of it.
//! * **A change of the setting** applies from the next block on. Switched off, what
//!   is held back plays at unity and the sum limiter leaves at the next seam.
//!
//! # Threads
//!
//! All of this runs on the decode thread, where allocating is allowed. The render
//! callback has no part in it and stays allocation-free and lock-free.

use std::collections::VecDeque;
use std::sync::Arc;

use rustamp_core::audio::{AudioSpec, DspStage};
use rustamp_dsp::fade::crossfade_gains;
use rustamp_dsp::{Limiter, LimiterControl};

/// The longest crossfade, in milliseconds (own choice). A duration is kept in
/// memory as audio, so a number in a configuration file must not be able to ask
/// for hours of it.
pub const MAX_CROSSFADE_MS: u32 = 30_000;

/// The decode thread's crossfade state. See the module docs.
pub(crate) struct Crossfade {
    device: AudioSpec,
    ms: u32,
    /// Device-format samples kept back, oldest first.
    held: VecDeque<f32>,
    /// The outgoing track's samples the running overlap still has to mix.
    tail: VecDeque<f32>,
    /// Frames the running overlap covers in total.
    span: usize,
    /// Frames of the running overlap already mixed.
    mixed: usize,
    /// The limiter on the sum.
    limiter: Limiter,
    /// Frames the limiter still emits from its initial state; they are dropped.
    trim: usize,
    /// The limiter is in the path.
    live: bool,
}

impl Crossfade {
    /// The state for a run on a device with the format `device`.
    pub(crate) fn new(device: AudioSpec, ms: u32, limiter: Arc<LimiterControl>) -> Self {
        let mut limiter = Limiter::with_control(limiter);
        // The limiter does not fail on any format the device granted.
        let _ = limiter.prepare(device);
        Crossfade {
            device,
            ms,
            held: VecDeque::new(),
            tail: VecDeque::new(),
            span: 0,
            mixed: 0,
            limiter,
            trim: 0,
            live: false,
        }
    }

    fn channels(&self) -> usize {
        usize::from(self.device.channels.max(1))
    }

    /// Take a new duration over.
    pub(crate) fn set_ms(&mut self, ms: u32) {
        self.ms = ms;
    }

    /// The duration in device frames.
    pub(crate) fn hold_frames(&self) -> usize {
        (u64::from(self.ms) * u64::from(self.device.sample_rate) / 1000) as usize
    }

    /// The setting is off and nothing is left of it: the audio goes straight
    /// into the ring, as it does without a crossfade.
    pub(crate) fn is_idle(&self) -> bool {
        self.ms == 0 && !self.live && self.held.is_empty() && self.tail.is_empty()
    }

    /// Frames kept back.
    pub(crate) fn held_frames(&self) -> usize {
        self.held.len() / self.channels()
    }

    /// Frames between the next frame handed in and the ring.
    pub(crate) fn pending_frames(&self) -> u64 {
        let inside = if self.live {
            self.limiter_latency() - self.trim
        } else {
            0
        };
        (self.held_frames() + inside) as u64
    }

    /// The frames of the held-back tail a fade into a track of `incoming` frames
    /// covers.
    pub(crate) fn overlap_frames(&self, incoming: Option<u64>) -> usize {
        let Some(incoming) = incoming else {
            return 0;
        };
        let half = usize::try_from(incoming / 2).unwrap_or(usize::MAX);
        self.hold_frames().min(self.held_frames()).min(half)
    }

    /// What is held back becomes the tail of a new overlap.
    pub(crate) fn start_overlap(&mut self) {
        self.tail = std::mem::take(&mut self.held);
        self.span = self.tail.len() / self.channels();
        self.mixed = 0;
    }

    /// The overlap that was just started has no track to mix with: its tail goes
    /// back to where it was.
    pub(crate) fn abort_overlap(&mut self) {
        self.held = std::mem::take(&mut self.tail);
    }

    /// Frames the running overlap still has to mix.
    pub(crate) fn overlap_left(&self) -> usize {
        self.tail.len() / self.channels()
    }

    /// Mix the tail into the incoming track's `block`, in place.
    pub(crate) fn mix(&mut self, block: &mut [f32]) {
        let channels = self.channels();
        for frame in block.chunks_exact_mut(channels) {
            if self.tail.is_empty() {
                return;
            }
            // Neither end of the span is a silent frame: the first and the last
            // frame behind the fade are one curve step away from what is mixed.
            let t = (self.mixed + 1) as f64 / (self.span + 1) as f64;
            let (outgoing, incoming) = crossfade_gains(t);
            for sample in frame {
                let tail = self.tail.pop_front().unwrap_or(0.0);
                *sample = tail * outgoing + *sample * incoming;
            }
            self.mixed += 1;
        }
    }

    /// Keep `block` back.
    pub(crate) fn hold(&mut self, block: &[f32]) {
        self.held.extend(block);
    }

    /// Move the `frames` oldest frames kept back to the start of `out`.
    pub(crate) fn take_held(&mut self, out: &mut [f32], frames: usize) {
        let samples = frames * self.channels();
        for (slot, sample) in out[..samples].iter_mut().zip(self.held.drain(..samples)) {
            *slot = sample;
        }
    }

    /// Run `block` through the sum limiter and hold full scale on it. Returns the
    /// frames at the start of `block` to drop (the limiter's initial state).
    pub(crate) fn limit(&mut self, block: &mut [f32]) -> usize {
        if !self.live {
            self.reset_limiter();
            self.live = true;
        }
        // The limiter does not fail.
        let _ = self.limiter.process(block);
        for sample in block.iter_mut() {
            *sample = sample.clamp(-1.0, 1.0);
        }
        let trimmed = self.trim.min(block.len() / self.channels());
        self.trim -= trimmed;
        trimmed
    }

    /// The sum limiter's look-ahead in frames.
    pub(crate) fn limiter_latency(&self) -> usize {
        self.limiter.latency_frames() as usize
    }

    /// The sum limiter is in the path.
    pub(crate) fn limiter_live(&self) -> bool {
        self.live
    }

    /// The sum limiter has been flushed and leaves the path.
    pub(crate) fn retire_limiter(&mut self) {
        self.live = false;
        self.reset_limiter();
    }

    fn reset_limiter(&mut self) {
        self.limiter.reset();
        self.trim = self.limiter_latency();
    }

    /// Drop everything that was held back and whatever the running overlap had
    /// left: the audio is discontinuous here anyway (a seek, a skip).
    pub(crate) fn clear(&mut self) {
        self.held.clear();
        self.tail.clear();
        if self.live {
            self.reset_limiter();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: AudioSpec = AudioSpec::new(1000, 2);

    fn state(ms: u32) -> Crossfade {
        Crossfade::new(DEVICE, ms, Arc::new(LimiterControl::new(false)))
    }

    #[test]
    fn a_fade_is_the_shortest_of_the_duration_the_tail_and_half_the_next_track() {
        let mut fade = state(100);
        fade.hold(&[0.0; 2 * 80]);
        assert_eq!(fade.overlap_frames(Some(1000)), 80, "the tail");
        assert_eq!(fade.overlap_frames(Some(60)), 30, "half the next track");
        assert_eq!(fade.overlap_frames(None), 0, "a stream takes no part");
        fade.hold(&[0.0; 2 * 80]);
        assert_eq!(fade.overlap_frames(Some(1000)), 100, "the duration");
    }

    #[test]
    fn the_mix_holds_the_power_of_two_uncorrelated_sides_at_every_frame() {
        let mut fade = state(100);
        fade.hold(&[1.0, 0.0].repeat(100));
        fade.start_overlap();
        let mut block = [0.0, 1.0].repeat(100);
        fade.mix(&mut block);
        for frame in block.as_chunks::<2>().0 {
            let power = frame[0] * frame[0] + frame[1] * frame[1];
            assert!((power - 1.0).abs() < 1e-6, "{frame:?}");
        }
        assert_eq!(fade.overlap_left(), 0);
    }

    #[test]
    fn a_block_behind_the_overlap_is_not_touched() {
        let mut fade = state(100);
        fade.hold(&[0.5; 2 * 4]);
        fade.start_overlap();
        let mut block = [0.25; 2 * 10];
        fade.mix(&mut block);
        assert_eq!(&block[8..], &[0.25; 12]);
    }

    #[test]
    fn with_the_limiter_off_the_sum_is_held_at_full_scale() {
        let mut fade = state(100);
        let mut block = [1.5, -1.5, 0.5, -0.5];
        assert_eq!(
            fade.limit(&mut block),
            0,
            "a bypassed limiter holds nothing"
        );
        assert_eq!(block, [1.0, -1.0, 0.5, -0.5]);
    }

    #[test]
    fn with_the_setting_off_nothing_is_in_the_way() {
        assert!(state(0).is_idle());
        assert!(!state(1).is_idle());
    }
}
