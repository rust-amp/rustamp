//! Deterministic resident fixture. No file, codec, device or audio output.

use rustamp_mediahost::decoder::{DecoderInfo, DecoderResident, ResidentDecoder};
use rustamp_mediahost::{MediaHostError, Result};
use rustamp_proto::host::{MediaLocationWire, PcmSpecWire};

pub const TOTAL: usize = 4099;
pub const CHANGE: usize = 513;

pub struct StubResident;

struct StubDecoder {
    mode: String,
    frame: usize,
    changed: bool,
}

impl DecoderResident for StubResident {
    fn open(&self, location: MediaLocationWire) -> Result<Box<dyn ResidentDecoder>> {
        let MediaLocationWire::Url(mode) = location else {
            return Err(MediaHostError::OperationFailed(
                "fixture accepts URLs only".into(),
            ));
        };
        Ok(Box::new(StubDecoder {
            mode,
            frame: 0,
            changed: false,
        }))
    }
}

impl ResidentDecoder for StubDecoder {
    fn info(&self) -> DecoderInfo {
        DecoderInfo {
            spec: PcmSpecWire {
                sample_rate_hz: if self.changed { 2000 } else { 1000 },
                channels: if self.changed { 1 } else { 2 },
            },
            duration_ms: Some(TOTAL as u64),
            seekable: true,
            metadata: Default::default(),
        }
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        if self.frame > 0 && self.mode == "fixture:exit" {
            // Only the VM-only example selects this mode. Deliberately do not
            // close the ring: this models a resident terminating its process.
            std::thread::sleep(std::time::Duration::from_millis(50));
            std::process::exit(23);
        }
        if self.frame > 0 && self.mode == "fixture:fail" {
            return Err(MediaHostError::OperationFailed(
                "fixture decode failure".into(),
            ));
        }
        if self.frame == 0 && self.mode == "fixture:slow" {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        if self.mode == "fixture:format" && !self.changed && self.frame == CHANGE {
            self.changed = true;
            return Err(MediaHostError::FormatChanged);
        }
        let channels = usize::from(self.info().spec.channels);
        let end = if self.mode == "fixture:format" && !self.changed {
            CHANGE
        } else {
            TOTAL
        };
        let frames = (end - self.frame).min(out.len() / channels).min(257);
        for (index, frame) in out[..frames * channels]
            .chunks_exact_mut(channels)
            .enumerate()
        {
            frame[0] = (self.frame + index) as f32 / 8.0;
            if channels == 2 {
                frame[1] = -frame[0];
            }
        }
        self.frame += frames;
        Ok(frames)
    }

    fn seek(&mut self, to_ms: u64) -> Result<u64> {
        self.frame = (to_ms as usize).min(TOTAL);
        self.changed = self.mode == "fixture:format" && self.frame >= CHANGE;
        Ok(self.frame as u64)
    }

    fn stats(&self) -> (u64, Option<u32>) {
        (7, Some(128))
    }
}
