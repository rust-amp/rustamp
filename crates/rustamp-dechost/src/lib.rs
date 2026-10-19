//! # rustamp-dechost
//!
//! First real decoder resident behind the media host (RFC 0006 §5).
//!
//! [`RegistryResident`] opens a real file through the existing decoder
//! registry (`rustamp-decoders::default_registry`) and adapts the resulting
//! core `Decoder` to the framework boundary
//! (`rustamp-mediahost::decoder::{ResidentDecoder, DecoderResident}`), which
//! owns the format handshake, the ring fill, the drain-then-EOS order and the
//! control replies. The shipped binary is `rustamp-dechost-symphonia`
//! (`src/main.rs`); it is a different binary from the framework's placeholder
//! `rustamp-dechost`, which reports that no resident is linked.
//!
//! **License:** GPL-3.0-or-later, no §7 exception (RFC 0006 §8 erratum: a
//! resident crate carries its resident's license; this one links the GPL
//! decoder crate and Symphonia, so it cannot be permissive).
//!
//! Headless: stdio control plus shared-memory PCM only. No window, no audio
//! output, no downloads.
//!
//! EOS means the backend has no more decodable frames **and the file holds
//! everything its container declares**. A file that was cut short is a decode
//! failure, not an end of stream: the decoder reports it as
//! [`Error::Decode`](rustamp_core::error::Error::Decode), this adapter turns
//! that into a loud [`MediaHostError::OperationFailed`], and the framework
//! reports it as a terminal `DecoderError` instead of an end of stream. A
//! half-downloaded track therefore fails loudly instead of playing short.
//!
//! The limit of that rule is where the decoder has no oracle: only a
//! container that states its payload length exactly (RIFF/WAVE) can say that
//! a file was cut, so a truncated file of another kind still ends with an end
//! of stream. Durations may be estimates as well; comparing them to decoded
//! frames is not a format-independent truncation check.
//!
//! A file that cannot be decoded at all is refused when it is opened, and the
//! refusal carries the reason the decoder named: a FLAC file without a
//! `STREAMINFO` block states no audio parameters, so the reader cannot
//! recognise a frame and the decoder refuses the file by name instead of
//! letting the reader's bare "end of stream" stand.

use std::path::PathBuf;
use std::time::Duration;

use rustamp_core::audio::{AudioSpec, Decoder, MediaSource};
use rustamp_core::error::Error as CoreError;
use rustamp_decoders::default_registry;
use rustamp_mediahost::decoder::{DecoderInfo, DecoderResident, ResidentDecoder};
use rustamp_mediahost::{MediaHostError, Result};
use rustamp_proto::host::{MediaLocationWire, PcmSpecWire};

/// Opens real files through the shared decoder registry.
///
/// URLs are refused with a clean terminal error: network streaming is M4
/// work and `MediaSource::Reader` cannot cross the process boundary
/// (RFC 0006 §5.1), so there is no silent fallback.
#[derive(Debug, Default)]
pub struct RegistryResident;

impl RegistryResident {
    /// Create the resident. Stateless: every open probes the registry again.
    pub fn new() -> Self {
        Self
    }
}

/// A live registry decoder adapted to the resident boundary.
pub struct RegistryDecoder {
    inner: Box<dyn Decoder>,
    spec: PcmSpecWire,
}

impl RegistryDecoder {
    /// Wrap an open decoder; rejects a zero or unrepresentable spec loudly
    /// instead of offering a ring nobody can validate.
    fn new(inner: Box<dyn Decoder>) -> Result<Self> {
        let spec = spec_wire(inner.spec())?;
        Ok(Self { inner, spec })
    }

    /// Re-read the spec after a seek, which may land on a new format.
    fn refresh_spec(&mut self) -> Result<()> {
        self.spec = spec_wire(self.inner.spec())?;
        Ok(())
    }
}

/// Convert a core spec to the wire spec.
fn spec_wire(spec: AudioSpec) -> Result<PcmSpecWire> {
    let channels = u8::try_from(spec.channels).map_err(|_| {
        MediaHostError::InvalidMessage(format!("channel count out of range: {}", spec.channels))
    })?;
    if spec.sample_rate == 0 || channels == 0 {
        return Err(MediaHostError::InvalidMessage(
            "zero PCM channels/sample rate".into(),
        ));
    }
    Ok(PcmSpecWire {
        sample_rate_hz: spec.sample_rate,
        channels,
    })
}

/// Map a failed core read: a format change stores the already-switched spec
/// and becomes the framework's unit signal; anything else is a loud terminal
/// error that the framework reports as `DecoderError`, never as EOS.
fn map_read_error(error: CoreError, spec: &mut PcmSpecWire) -> MediaHostError {
    match error {
        CoreError::FormatChanged { to, .. } => match spec_wire(to) {
            Ok(next) => {
                *spec = next;
                MediaHostError::FormatChanged
            }
            Err(failed) => {
                MediaHostError::OperationFailed(format!("new format is invalid: {failed}"))
            }
        },
        other => MediaHostError::OperationFailed(other.to_string()),
    }
}

/// Saturating `Duration` to whole milliseconds for the wire.
fn reached_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

impl DecoderResident for RegistryResident {
    fn open(&self, location: MediaLocationWire) -> Result<Box<dyn ResidentDecoder>> {
        match location {
            MediaLocationWire::File(path) => {
                let inner = default_registry()
                    .open(MediaSource::Path(PathBuf::from(path)))
                    .map_err(|error| MediaHostError::OperationFailed(error.to_string()))?;
                Ok(Box::new(RegistryDecoder::new(inner)?))
            }
            MediaLocationWire::Url(url) => Err(MediaHostError::OperationFailed(format!(
                "URL decoding is not supported yet (M4): {url}"
            ))),
        }
    }
}

impl ResidentDecoder for RegistryDecoder {
    fn info(&self) -> DecoderInfo {
        DecoderInfo {
            spec: self.spec,
            duration_ms: self.inner.duration().map(reached_ms),
            seekable: self.inner.is_seekable(),
            metadata: self.inner.metadata(),
        }
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize> {
        match self.inner.read(out) {
            Ok(frames) => Ok(frames),
            Err(error) => Err(map_read_error(error, &mut self.spec)),
        }
    }

    fn seek(&mut self, to_ms: u64) -> Result<u64> {
        let reached = self
            .inner
            .seek(Duration::from_millis(to_ms))
            .map_err(|error| MediaHostError::OperationFailed(error.to_string()))?;
        self.refresh_spec()?;
        Ok(reached_ms(reached))
    }

    fn stats(&self) -> (u64, Option<u32>) {
        (
            self.inner.skipped_packets(),
            self.inner.stream_info().bitrate_kbps,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::audio::AudioSpec;

    #[test]
    fn url_locations_are_refused_without_opening_a_decoder() {
        let result = RegistryResident.open(MediaLocationWire::Url(
            "https://example.invalid/audio.mp3".into(),
        ));
        assert!(matches!(
            result,
            Err(MediaHostError::OperationFailed(message))
                if message == "URL decoding is not supported yet (M4): https://example.invalid/audio.mp3"
        ));
    }

    #[test]
    fn spec_wire_accepts_stereo() {
        let wire = spec_wire(AudioSpec::new(44_100, 2)).unwrap();
        assert_eq!(
            wire,
            PcmSpecWire {
                sample_rate_hz: 44_100,
                channels: 2,
            }
        );
        assert_eq!(wire.frame_size(), 8);
    }

    #[test]
    fn spec_wire_rejects_zero_rate_or_channels() {
        assert!(spec_wire(AudioSpec::new(0, 2)).is_err());
        assert!(spec_wire(AudioSpec::new(44_100, 0)).is_err());
    }

    #[test]
    fn spec_wire_rejects_counts_above_u8() {
        assert!(spec_wire(AudioSpec::new(48_000, 300)).is_err());
    }

    #[test]
    fn read_error_maps_format_change_and_damage() {
        let mut spec = PcmSpecWire {
            sample_rate_hz: 44_100,
            channels: 2,
        };
        let change = CoreError::FormatChanged {
            from: AudioSpec::new(44_100, 2),
            to: AudioSpec::new(48_000, 1),
        };
        assert!(matches!(
            map_read_error(change, &mut spec),
            MediaHostError::FormatChanged
        ));
        assert_eq!(spec.sample_rate_hz, 48_000);
        assert_eq!(spec.channels, 1);
        assert!(matches!(
            map_read_error(CoreError::Decode("boom".into()), &mut spec),
            MediaHostError::OperationFailed(_)
        ));
    }
}
