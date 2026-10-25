//! The data a classic visualization plug-in reads, made from the player's frame.
//!
//! The player publishes [`VisFrame`]s of neutral data (waveform in `[-1, 1]`,
//! spectrum as linear magnitude, a full-scale sine near 1.0). A classic plug-in
//! reads two rows of 576 **bytes** each, for the spectrum and the waveform
//! (S09 §5.2). S08 leaves the plug-in-facing scaling open, so these are **own
//! choices**, kept in this one place:
//!
//! * waveform: `128 + 127 * sample`, clamped to `0..=255` (silence is 128, the
//!   unsigned-byte midpoint classic plug-ins expect);
//! * spectrum: the magnitude in decibels, `-60 dB` mapped to 0 and `0 dB` to
//!   255, linear in between (a quiet bin is 0, a full-scale bin 255).

use rustamp_proto::vis::{VIS_CHANNELS, VIS_SAMPLES, VisFrame};

use crate::vis::VisRows;
use rustamp_plugin_abi::common::{VIS_MAX_ROWS, VIS_ROW_LEN};

/// The lowest level the spectrum mapping distinguishes, in decibels.
const SPECTRUM_FLOOR_DB: f32 = -60.0;

const _: () = assert!(VIS_ROW_LEN == VIS_SAMPLES && VIS_MAX_ROWS >= VIS_CHANNELS);

/// One waveform sample as the plug-in's unsigned byte.
fn waveform_byte(sample: f32) -> u8 {
    if !sample.is_finite() {
        return 128;
    }
    (128.0 + 127.0 * sample.clamp(-1.0, 1.0))
        .round()
        .clamp(0.0, 255.0) as u8
}

/// One spectrum magnitude as the plug-in's unsigned byte.
fn spectrum_byte(magnitude: f32) -> u8 {
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return 0;
    }
    let db = 20.0 * magnitude.log10();
    let level = ((db - SPECTRUM_FLOOR_DB) / -SPECTRUM_FLOOR_DB).clamp(0.0, 1.0);
    (level * 255.0).round() as u8
}

/// The two rows (per channel) a plug-in reads, from the player's frame.
pub fn rows_from_frame(frame: &VisFrame) -> VisRows {
    let mut rows = VisRows {
        spectrum: [[0; VIS_ROW_LEN]; VIS_MAX_ROWS],
        waveform: [[128; VIS_ROW_LEN]; VIS_MAX_ROWS],
    };
    for channel in 0..VIS_CHANNELS {
        for index in 0..VIS_SAMPLES {
            rows.waveform[channel][index] = waveform_byte(frame.waveform[channel][index]);
            rows.spectrum[channel][index] = spectrum_byte(frame.spectrum[channel][index]);
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_is_the_midpoint_and_an_empty_spectrum() {
        let rows = rows_from_frame(&VisFrame::default());
        assert!(rows.waveform[0].iter().all(|b| *b == 128));
        assert!(rows.waveform[1].iter().all(|b| *b == 128));
        assert!(rows.spectrum[0].iter().all(|b| *b == 0));
    }

    #[test]
    fn the_waveform_spans_the_byte_range_and_never_leaves_it() {
        assert_eq!(waveform_byte(0.0), 128);
        assert_eq!(waveform_byte(1.0), 255);
        assert_eq!(waveform_byte(-1.0), 1);
        assert_eq!(waveform_byte(5.0), 255, "clamped");
        assert_eq!(waveform_byte(-5.0), 1, "clamped");
        assert_eq!(
            waveform_byte(f32::NAN),
            128,
            "not a number reads as silence"
        );
    }

    #[test]
    fn the_spectrum_maps_sixty_decibels_onto_the_byte_range() {
        assert_eq!(spectrum_byte(0.0), 0);
        assert_eq!(spectrum_byte(f32::NAN), 0);
        assert_eq!(spectrum_byte(1.0), 255, "full scale");
        assert_eq!(spectrum_byte(0.001), 0, "-60 dB is the floor");
        let half = spectrum_byte(0.0316); // about -30 dB
        assert!((120..=135).contains(&half), "{half}");
        assert_eq!(spectrum_byte(10.0), 255, "above full scale is clamped");
    }

    #[test]
    fn a_frame_lands_in_the_right_row_and_column() {
        let mut frame = VisFrame::default();
        frame.waveform[1][10] = 1.0;
        frame.spectrum[0][3] = 1.0;
        let rows = rows_from_frame(&frame);
        assert_eq!(rows.waveform[1][10], 255);
        assert_eq!(rows.waveform[0][10], 128);
        assert_eq!(rows.spectrum[0][3], 255);
        assert_eq!(rows.spectrum[1][3], 0);
    }
}
