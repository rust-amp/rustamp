//! Deterministic DSP resident fixture. No file, codec, device or audio output.
//!
//! Stage ids mirror the real resident (`gain`, `passthrough`) plus two
//! fixture-only ids: `fixture:fail` refuses the configure loudly and
//! `fixture:exit` terminates the process after its first block without
//! closing the rings (a crash surrogate for the VM-only kill test).

use rustamp_mediahost::dsp::{DspResident, ResidentChain};
use rustamp_mediahost::{MediaHostError, Result};
use rustamp_proto::host::{DspStageDesc, PcmSpecWire};

pub struct StubDspResident;

struct StubChain {
    factors: Vec<f32>,
    crash: bool,
    blocks: usize,
}

impl ResidentChain for StubChain {
    fn latency_frames(&self) -> u32 {
        0
    }

    fn process(&mut self, buf: &mut [f32]) -> Result<()> {
        if self.crash && self.blocks == 0 {
            // Only the VM-only example selects this mode. Deliberately do
            // not close the rings: this models a resident crashing mid-run.
            self.blocks += 1;
            std::thread::sleep(std::time::Duration::from_millis(50));
            std::process::exit(23);
        }
        self.blocks += 1;
        for factor in &self.factors {
            for sample in buf.iter_mut() {
                *sample *= *factor;
            }
        }
        Ok(())
    }

    fn reset(&mut self) {}
}

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

impl DspResident for StubDspResident {
    fn open(&self, spec: PcmSpecWire, stages: &[DspStageDesc]) -> Result<Box<dyn ResidentChain>> {
        if spec.channels == 0 || spec.sample_rate_hz == 0 {
            return Err(MediaHostError::InvalidMessage(
                "zero PCM channels/sample rate".into(),
            ));
        }
        let mut factors = Vec::with_capacity(stages.len());
        let mut crash = false;
        for stage in stages {
            match stage.id.as_str() {
                "gain" => factors.push(gain_factor(&stage.params)?),
                "passthrough" => {
                    if !stage.params.is_empty() {
                        return Err(MediaHostError::OperationFailed(
                            "passthrough takes no params".into(),
                        ));
                    }
                }
                "fixture:fail" => {
                    return Err(MediaHostError::OperationFailed(
                        "fixture DSP failure".into(),
                    ));
                }
                "fixture:exit" => crash = true,
                unknown => {
                    return Err(MediaHostError::OperationFailed(format!(
                        "unknown DSP stage: {unknown}"
                    )));
                }
            }
        }
        Ok(Box::new(StubChain {
            factors,
            crash,
            blocks: 0,
        }))
    }
}
