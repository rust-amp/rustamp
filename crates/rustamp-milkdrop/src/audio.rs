// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The player's decoded audio, as the interleaved PCM the engine analyses.
//!
//! A [`VisFrame`] holds the most recent 576 samples of the stream, however much
//! of them the previous frame already held. The engine keeps a history of its
//! own, so feeding it whole windows would count the overlap twice: the
//! timestamps tell how many samples are new.

use rustamp_core::vis::{VIS_CHANNELS, VIS_SAMPLES, VisFrame};

/// Turns successive [`VisFrame`]s into the PCM that has not been fed yet.
#[derive(Debug, Default)]
pub struct PcmFeed {
    last_timestamp_us: Option<u64>,
    interleaved: Vec<f32>,
}

impl PcmFeed {
    /// The samples of `frame` that are new, interleaved left and right.
    ///
    /// `source_rate_hz` is the rate of the decoded stream; `0` (not known) feeds
    /// the whole window. A timestamp that went backwards is a seek or a new
    /// track, so nothing of the window is old. A repeated timestamp feeds
    /// nothing. A sample that is not finite is fed as silence: the analysis
    /// must not be poisoned by one.
    pub fn new_samples(&mut self, frame: &VisFrame, source_rate_hz: u32) -> &[f32] {
        let fresh = match self.last_timestamp_us {
            Some(last) if frame.timestamp_us == last => 0,
            Some(last) if frame.timestamp_us > last && source_rate_hz > 0 => {
                let elapsed = u128::from(frame.timestamp_us - last);
                let samples = (elapsed * u128::from(source_rate_hz) + 500_000) / 1_000_000;
                samples.min(VIS_SAMPLES as u128) as usize
            }
            _ => VIS_SAMPLES,
        };
        self.last_timestamp_us = Some(frame.timestamp_us);
        self.interleaved.clear();
        for index in VIS_SAMPLES - fresh..VIS_SAMPLES {
            for channel in 0..VIS_CHANNELS {
                let sample = frame.waveform[channel][index];
                self.interleaved
                    .push(if sample.is_finite() { sample } else { 0.0 });
            }
        }
        &self.interleaved
    }

    /// Forgets the last timestamp: the next frame counts as a new stream.
    pub fn reset(&mut self) {
        self.last_timestamp_us = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(timestamp_us: u64, left: f32, right: f32) -> VisFrame {
        VisFrame {
            timestamp_us,
            waveform: [[left; VIS_SAMPLES], [right; VIS_SAMPLES]],
            ..VisFrame::default()
        }
    }

    #[test]
    fn the_first_frame_feeds_the_whole_window_interleaved() {
        let mut feed = PcmFeed::default();
        let pcm = feed.new_samples(&frame(1_000, 0.25, -0.5), 44_100);
        assert_eq!(pcm.len(), VIS_SAMPLES * VIS_CHANNELS);
        assert_eq!(&pcm[..2], &[0.25, -0.5]);
    }

    #[test]
    fn a_later_frame_feeds_only_the_samples_the_timestamps_say_are_new() {
        let mut feed = PcmFeed::default();
        feed.new_samples(&frame(1_000_000, 0.0, 0.0), 44_100);
        // 5 ms at 44.1 kHz are 220.5 samples, rounded to 221 (half up).
        let mut next = frame(1_005_000, 0.0, 0.0);
        next.waveform[0][VIS_SAMPLES - 1] = 0.75;
        next.waveform[1][VIS_SAMPLES - 1] = -0.75;
        let pcm = feed.new_samples(&next, 44_100);
        assert_eq!(pcm.len(), 221 * VIS_CHANNELS);
        assert_eq!(&pcm[pcm.len() - 2..], &[0.75, -0.75], "newest last");
    }

    #[test]
    fn a_gap_longer_than_the_window_feeds_the_window_once() {
        let mut feed = PcmFeed::default();
        feed.new_samples(&frame(0, 0.0, 0.0), 44_100);
        let pcm = feed.new_samples(&frame(10_000_000, 0.0, 0.0), 44_100);
        assert_eq!(pcm.len(), VIS_SAMPLES * VIS_CHANNELS);
    }

    #[test]
    fn a_repeated_frame_feeds_nothing_and_a_seek_back_feeds_the_window() {
        let mut feed = PcmFeed::default();
        feed.new_samples(&frame(5_000, 0.0, 0.0), 44_100);
        assert!(feed.new_samples(&frame(5_000, 0.0, 0.0), 44_100).is_empty());
        assert_eq!(
            feed.new_samples(&frame(1_000, 0.0, 0.0), 44_100).len(),
            VIS_SAMPLES * VIS_CHANNELS
        );
    }

    #[test]
    fn an_unknown_rate_feeds_the_window_and_reset_starts_over() {
        let mut feed = PcmFeed::default();
        feed.new_samples(&frame(1_000, 0.0, 0.0), 0);
        assert_eq!(
            feed.new_samples(&frame(2_000, 0.0, 0.0), 0).len(),
            VIS_SAMPLES * VIS_CHANNELS
        );
        feed.reset();
        assert_eq!(
            feed.new_samples(&frame(2_000, 0.0, 0.0), 44_100).len(),
            VIS_SAMPLES * VIS_CHANNELS
        );
    }

    #[test]
    fn a_sample_that_is_not_finite_is_fed_as_silence() {
        let mut feed = PcmFeed::default();
        let pcm = feed.new_samples(&frame(0, f32::NAN, f32::INFINITY), 44_100);
        assert!(pcm.iter().all(|sample| *sample == 0.0));
    }
}
