//! # rustamp-dsphost
//!
//! First real DSP resident behind the media host (RFC 0006 §6).
//!
//! [`StageResident`] builds a chain of [`DspStage`]s from a `DspConfigure`
//! stage list and adapts it to the framework boundary
//! (`rustamp-mediahost::dsp::{DspResident, ResidentChain}`), which owns the
//! two-ring handshake, the block pump, the drain-then-sticky-EOS order and the
//! control replies. The shipped binary is `rustamp-dsphost-stages`
//! (`src/main.rs`); it is a different binary from the framework's placeholder
//! `rustamp-dsphost`, which reports that no resident is linked.
//!
//! Stage ids today: `gain` (params are one finite `f32` in LE bytes, the
//! plain factor the chain multiplies every sample by) and `passthrough`
//! (empty params; bit-exact bypass). Both report zero latency. Unknown ids
//! and undecodable params are refused with `DspError` before any ring exists.
//!
//! **License:** GPL-3.0-or-later, no §7 exception (RFC 0006 §8 erratum: a
//! resident crate carries its resident's license; this one links GPL-only
//! `rustamp-core` DSP contracts and exists to isolate DSP code, so it cannot
//! be permissive).
//!
//! Headless: stdio control plus shared-memory PCM only. No window, no audio
//! output, no downloads.

use rustamp_core::audio::{AudioSpec, DspStage};
use rustamp_core::error::{Error as CoreError, Result as CoreResult};
use rustamp_mediahost::dsp::{DspResident, ResidentChain};
use rustamp_mediahost::{MediaHostError, Result};
use rustamp_proto::host::{DspStageDesc, PcmSpecWire};

/// Stage id for the plain gain stage. Params: one finite LE `f32`.
pub const GAIN_STAGE_ID: &str = "gain";
/// Stage id for the bit-exact bypass stage. Params must be empty.
pub const PASSTHROUGH_STAGE_ID: &str = "passthrough";

/// A plain gain: multiplies every sample by a fixed finite factor.
#[derive(Debug)]
pub struct Gain {
    factor: f32,
    channels: u16,
}

impl Gain {
    /// Create a gain stage; the factor must already be validated finite.
    pub fn new(factor: f32) -> Self {
        Self {
            factor,
            channels: 0,
        }
    }
}

impl DspStage for Gain {
    fn name(&self) -> &str {
        GAIN_STAGE_ID
    }

    fn prepare(&mut self, spec: AudioSpec) -> CoreResult<()> {
        if spec.sample_rate == 0 || spec.channels == 0 {
            return Err(CoreError::Unsupported("gain needs a non-zero spec"));
        }
        self.channels = spec.channels;
        Ok(())
    }

    fn process(&mut self, buf: &mut [f32]) {
        for sample in buf.iter_mut() {
            *sample *= self.factor;
        }
    }

    fn reset(&mut self) {}
}

/// A bit-exact bypass: `process` never touches the buffer.
#[derive(Debug, Default)]
pub struct Passthrough {
    channels: u16,
}

impl DspStage for Passthrough {
    fn name(&self) -> &str {
        PASSTHROUGH_STAGE_ID
    }

    fn prepare(&mut self, spec: AudioSpec) -> CoreResult<()> {
        if spec.sample_rate == 0 || spec.channels == 0 {
            return Err(CoreError::Unsupported("passthrough needs a non-zero spec"));
        }
        self.channels = spec.channels;
        Ok(())
    }

    fn process(&mut self, _buf: &mut [f32]) {}

    fn reset(&mut self) {}
}

/// Decode the gain factor: exactly 4 LE bytes holding a finite `f32`.
fn gain_factor(params: &[u8]) -> Result<f32> {
    if params.len() != 4 {
        return Err(MediaHostError::OperationFailed(format!(
            "gain params must be 4 LE bytes, got {}",
            params.len()
        )));
    }
    let factor = f32::from_le_bytes([params[0], params[1], params[2], params[3]]);
    if !factor.is_finite() {
        return Err(MediaHostError::OperationFailed(
            "gain factor must be finite".into(),
        ));
    }
    Ok(factor)
}

/// Build one stage; unknown ids and undecodable params fail loudly here.
fn build_stage(desc: &DspStageDesc) -> Result<Box<dyn DspStage>> {
    match desc.id.as_str() {
        GAIN_STAGE_ID => Ok(Box::new(Gain::new(gain_factor(&desc.params)?))),
        PASSTHROUGH_STAGE_ID => {
            if !desc.params.is_empty() {
                return Err(MediaHostError::OperationFailed(
                    "passthrough takes no params".into(),
                ));
            }
            Ok(Box::new(Passthrough::default()))
        }
        unknown => Err(MediaHostError::OperationFailed(format!(
            "unknown DSP stage: {unknown}"
        ))),
    }
}

/// Convert a wire spec to the core spec the stages prepare against.
fn audio_spec(spec: PcmSpecWire) -> Result<AudioSpec> {
    if spec.sample_rate_hz == 0 || spec.channels == 0 {
        return Err(MediaHostError::InvalidMessage(
            "zero PCM channels/sample rate".into(),
        ));
    }
    Ok(AudioSpec::new(
        spec.sample_rate_hz,
        u16::from(spec.channels),
    ))
}

struct Chain {
    stages: Vec<Box<dyn DspStage>>,
    latency: u32,
}

impl ResidentChain for Chain {
    fn latency_frames(&self) -> u32 {
        self.latency
    }

    fn process(&mut self, buf: &mut [f32]) {
        for stage in &mut self.stages {
            stage.process(buf);
        }
    }

    fn reset(&mut self) {
        for stage in &mut self.stages {
            stage.reset();
        }
    }
}

/// Builds [`Gain`]/[`Passthrough`] chains for the media host.
///
/// An empty stage list is a valid chain: it processes nothing, reports zero
/// latency and passes audio through bit-exactly. Refusals happen in `open`,
/// before any ring exists.
#[derive(Debug, Default)]
pub struct StageResident;

impl StageResident {
    /// Create the resident. Stateless: every configure builds a fresh chain.
    pub fn new() -> Self {
        Self
    }
}

impl DspResident for StageResident {
    fn open(&self, spec: PcmSpecWire, stages: &[DspStageDesc]) -> Result<Box<dyn ResidentChain>> {
        let audio = audio_spec(spec)?;
        let mut built = Vec::with_capacity(stages.len());
        for desc in stages {
            built.push(build_stage(desc)?);
        }
        let mut latency = 0u32;
        for stage in &mut built {
            stage
                .prepare(audio)
                .map_err(|error| MediaHostError::OperationFailed(error.to_string()))?;
            latency += stage.latency_frames();
        }
        Ok(Box::new(Chain {
            stages: built,
            latency,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::audio::DspStage;

    const SPEC: PcmSpecWire = PcmSpecWire {
        sample_rate_hz: 44_100,
        channels: 2,
    };

    fn gain_desc(factor: f32) -> DspStageDesc {
        DspStageDesc {
            id: GAIN_STAGE_ID.into(),
            params: factor.to_le_bytes().to_vec(),
        }
    }

    #[test]
    fn spec_rejects_zero_rate_or_channels() {
        assert!(audio_spec(SPEC).is_ok());
        assert!(
            audio_spec(PcmSpecWire {
                sample_rate_hz: 0,
                ..SPEC
            })
            .is_err()
        );
        assert!(
            audio_spec(PcmSpecWire {
                channels: 0,
                ..SPEC
            })
            .is_err()
        );
    }

    #[test]
    fn gain_params_need_four_finite_bytes() {
        assert_eq!(gain_factor(&2.0f32.to_le_bytes()).unwrap(), 2.0);
        assert!(gain_factor(&[0u8; 3]).is_err());
        assert!(gain_factor(&[0u8; 5]).is_err());
        assert!(gain_factor(&f32::NAN.to_le_bytes()).is_err());
        assert!(gain_factor(&f32::INFINITY.to_le_bytes()).is_err());
    }

    #[test]
    fn unknown_stage_and_bad_params_are_loud() {
        let resident = StageResident::new();
        assert!(
            resident
                .open(
                    SPEC,
                    &[DspStageDesc {
                        id: "fuzz".into(),
                        params: Vec::new()
                    }]
                )
                .is_err()
        );
        assert!(
            resident
                .open(
                    SPEC,
                    &[DspStageDesc {
                        id: GAIN_STAGE_ID.into(),
                        params: vec![0u8; 3],
                    }]
                )
                .is_err()
        );
        assert!(
            resident
                .open(
                    SPEC,
                    &[DspStageDesc {
                        id: PASSTHROUGH_STAGE_ID.into(),
                        params: vec![1u8],
                    }]
                )
                .is_err()
        );
    }

    #[test]
    fn empty_chain_is_a_zero_latency_passthrough() {
        let chain = StageResident::new().open(SPEC, &[]).unwrap();
        assert_eq!(chain.latency_frames(), 0);
        let mut chain = chain;
        let mut buf = [0.25, -0.5, 0.75, 1.0];
        chain.process(&mut buf);
        assert_eq!(buf, [0.25, -0.5, 0.75, 1.0]);
        chain.reset();
    }

    #[test]
    fn gain_multiplies_exactly_and_reports_no_latency() {
        let mut chain = StageResident::new().open(SPEC, &[gain_desc(2.0)]).unwrap();
        assert_eq!(chain.latency_frames(), 0);
        let mut buf = [0.25, -0.5, 0.125, 1.0];
        chain.process(&mut buf);
        assert_eq!(buf, [0.5, -1.0, 0.25, 2.0]);
    }

    #[test]
    fn stage_order_applies_left_to_right() {
        let mut chain = StageResident::new()
            .open(
                SPEC,
                &[
                    gain_desc(0.5),
                    DspStageDesc {
                        id: PASSTHROUGH_STAGE_ID.into(),
                        params: Vec::new(),
                    },
                    gain_desc(4.0),
                ],
            )
            .unwrap();
        let mut buf = [1.0, -1.0];
        chain.process(&mut buf);
        assert_eq!(buf, [2.0, -2.0]);
    }

    #[test]
    fn stages_prepare_against_the_configured_spec() {
        let mut gain = Gain::new(1.0);
        assert!(gain.prepare(AudioSpec::new(48_000, 1)).is_ok());
        assert!(gain.prepare(AudioSpec::new(0, 2)).is_err());
        assert!(gain.prepare(AudioSpec::new(44_100, 0)).is_err());
        let mut through = Passthrough::default();
        assert!(through.prepare(AudioSpec::new(44_100, 2)).is_ok());
        assert!(through.prepare(AudioSpec::new(44_100, 0)).is_err());
    }
}
