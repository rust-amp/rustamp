//! Wire types for out-of-process decoder and DSP hosting (RFC 0006).
//!
//! These types live in `rustamp-proto` (permissive) so that the control
//! protocol in `rustamp-ipc` (also permissive) can use them without pulling in
//! any GPL code. The RFC 0004 invariant is load-bearing here.

use serde::{Deserialize, Serialize};

/// Wire form of a media location for `DecoderOpen`.
///
/// Unlike [`crate::playlist::MediaLocation`], this uses a UTF-8 string for the
/// file path because `PathBuf` cannot be reliably encoded over the wire
/// (non-UTF-8 paths are refused until HANDOFF-3 is fixed). The host opens the
/// path/URL itself; `MediaSource::Reader` cannot cross processes.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MediaLocationWire {
    /// A local file path (UTF-8).
    File(String),
    /// A network stream or other URL.
    Url(String),
}

impl MediaLocationWire {
    /// Convert from the core `MediaLocation` (lossy for file paths).
    pub fn from_media_location(loc: &crate::playlist::MediaLocation) -> Self {
        match loc {
            crate::playlist::MediaLocation::File(p) => {
                MediaLocationWire::File(p.to_string_lossy().into_owned())
            }
            crate::playlist::MediaLocation::Url(u) => MediaLocationWire::Url(u.clone()),
        }
    }

    /// Final path component / URL tail, for display and sorting.
    pub fn file_name(&self) -> String {
        match self {
            MediaLocationWire::File(p) => std::path::Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            MediaLocationWire::Url(u) => u.rsplit('/').next().unwrap_or(u).to_owned(),
        }
    }
}

/// Description of a single DSP stage for `DspConfigure`.
///
/// The `params` are opaque to the transport (a stage's own serialization).
/// Unknown stage IDs are refused with `DspError` before any ring exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DspStageDesc {
    /// Stable identifier of the stage (e.g., "gain", "eq", "limiter").
    pub id: String,
    /// Opaque parameter bytes for this stage.
    pub params: Vec<u8>,
}

/// Wire specification for a PCM stream (sample rate + channels).
///
/// Used in `DecoderOpened`, `DecoderFormat`, and `DspConfigure`.
/// This is not `AudioSpec` itself (which lives in `rustamp-core`); it is the
/// minimal wire form needed to create a PCM ring (`frame_size = channels * 4`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PcmSpecWire {
    /// Sample rate in Hz.
    pub sample_rate_hz: u32,
    /// Channel count (1 = mono, 2 = stereo, etc.).
    pub channels: u8,
}

impl PcmSpecWire {
    /// Frame size in bytes for a PCM ring (`channels * 4` for `f32`).
    pub fn frame_size(&self) -> u32 {
        u32::from(self.channels) * 4
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playlist::MediaLocation;

    #[test]
    fn media_location_wire_from_file() {
        let loc = MediaLocation::File(std::path::PathBuf::from("C:\\Music\\track.mp3"));
        let wire = MediaLocationWire::from_media_location(&loc);
        assert_eq!(wire, MediaLocationWire::File("C:\\Music\\track.mp3".into()));
        assert_eq!(wire.file_name(), "track.mp3");
    }

    #[test]
    fn media_location_wire_from_url() {
        let loc = MediaLocation::Url("http://example.com/stream.mp3".into());
        let wire = MediaLocationWire::from_media_location(&loc);
        assert_eq!(
            wire,
            MediaLocationWire::Url("http://example.com/stream.mp3".into())
        );
        assert_eq!(wire.file_name(), "stream.mp3");
    }

    #[test]
    fn dsp_stage_desc_roundtrip() {
        let desc = DspStageDesc {
            id: "gain".into(),
            params: vec![0x00, 0x00, 0x80, 0x3f], // 1.0f32
        };
        let encoded = postcard::to_allocvec(&desc).unwrap();
        let decoded: DspStageDesc = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(desc, decoded);
    }

    #[test]
    fn pcm_spec_wire_frame_size() {
        let spec = PcmSpecWire {
            sample_rate_hz: 44100,
            channels: 2,
        };
        assert_eq!(spec.frame_size(), 8);
        let spec = PcmSpecWire {
            sample_rate_hz: 48000,
            channels: 1,
        };
        assert_eq!(spec.frame_size(), 4);
        let spec = PcmSpecWire {
            sample_rate_hz: 96000,
            channels: 6,
        };
        assert_eq!(spec.frame_size(), 24);
    }

    #[test]
    fn pcm_spec_wire_roundtrip() {
        let spec = PcmSpecWire {
            sample_rate_hz: 44100,
            channels: 2,
        };
        let encoded = postcard::to_allocvec(&spec).unwrap();
        let decoded: PcmSpecWire = postcard::from_bytes(&encoded).unwrap();
        assert_eq!(spec, decoded);
    }
}
