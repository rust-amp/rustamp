// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! # projectM PCM audio adaptation
//!
//! Adapts Winamp classic visualization waveform data (S09 §5.2, 2 × 576 bytes)
//! into interleaved PCM audio buffers suitable for `projectm_pcm_add_float`.
//!
//! Clean-room implementation:
//! - Waveform rows represent signed 8-bit amplitudes in two's complement (-128..127).
//! - Waveform samples are normalized to floating-point values in `[-1.0, 1.0]`.
//! - Interleaved layout: L0, R0, L1, R1, ..., L575, R575 (total 1152 floats).
//! - Spectrum rows are never fed into projectM; projectM performs its own FFT
//!   analysis directly from PCM waveform data.

use rustamp_plugin_abi::common::{VIS_MAX_ROWS, VIS_ROW_LEN};

use super::ffi::ProjectmChannels;

/// Maximum number of interleaved float samples for a 2-channel 576-sample frame.
pub const MAX_INTERLEAVED_SAMPLES: usize = VIS_ROW_LEN * VIS_MAX_ROWS;

/// Audio converter for adapting classic vis waveform data to projectM PCM format.
#[derive(Debug)]
pub struct PcmAudioAdapter {
    interleaved_buffer: [f32; MAX_INTERLEAVED_SAMPLES],
}

impl Default for PcmAudioAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl PcmAudioAdapter {
    /// Creates a new PCM audio adapter.
    pub const fn new() -> Self {
        Self {
            interleaved_buffer: [0.0; MAX_INTERLEAVED_SAMPLES],
        }
    }

    /// Converts classic `VisModule::waveform_data` into interleaved floating-point PCM samples.
    ///
    /// # Arguments
    ///
    /// * `waveform_data` - The 2 × 576 raw waveform bytes provided by the host.
    /// * `channels` - Channel count (1 for mono, 2 for stereo).
    ///
    /// # Returns
    ///
    /// A slice of interleaved float samples normalized to `[-1.0, 1.0]`, along with
    /// the corresponding `ProjectmChannels` indicator.
    pub fn convert_waveform(
        &mut self,
        waveform_data: &[[u8; VIS_ROW_LEN]; VIS_MAX_ROWS],
        channels: i32,
    ) -> (&[f32], ProjectmChannels) {
        if channels <= 1 {
            // Mono: use left channel only (576 samples)
            let mono_slice = &waveform_data[0];
            for (i, &byte) in mono_slice.iter().enumerate() {
                // S09 §5.2: signed 8-bit amplitude, 0 = silence
                let sample_i8 = byte as i8;
                self.interleaved_buffer[i] = (sample_i8 as f32) / 128.0;
            }
            (
                &self.interleaved_buffer[..VIS_ROW_LEN],
                ProjectmChannels::Mono,
            )
        } else {
            // Stereo: interleave Left (row 0) and Right (row 1)
            let left = &waveform_data[0];
            let right = &waveform_data[1];
            for i in 0..VIS_ROW_LEN {
                let l_sample = (left[i] as i8 as f32) / 128.0;
                let r_sample = (right[i] as i8 as f32) / 128.0;
                self.interleaved_buffer[i * 2] = l_sample;
                self.interleaved_buffer[i * 2 + 1] = r_sample;
            }
            (
                &self.interleaved_buffer[..MAX_INTERLEAVED_SAMPLES],
                ProjectmChannels::Stereo,
            )
        }
    }
}
