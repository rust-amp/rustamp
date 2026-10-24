// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The app's DSP chain: the stages the app installs in the engine, the bypass
//! the engine reports, and the manual recovery from it.
//!
//! # The policy (W14 finding F4)
//!
//! The engine's recovery policy for a failed DSP stage is the **bypass**: the
//! failed stage is replaced by a pass-through, the failure is reported once
//! per run, and the chain runs without that stage until the caller installs a
//! chain again ([`Engine::set_dsp_chain`]). The app is that caller, so the
//! app owns three things:
//!
//! * the **chain** it installed — the stages it handed to the engine;
//! * the **bypass** it remembers — so the preferences page does not claim
//!   that a DSP is running while the engine runs without it; and
//! * the **recovery** — a fresh chain, installed when the user asks for it.
//!
//! # The recovery is manual, never automatic
//!
//! A bypass means the stage failed. Building the same chain again and
//! installing it again would fail the same way, so the app never does that by
//! itself: there is no retry loop. The chain comes back when the user
//! switches the DSP off and on again (Preferences → DSP/Effect), which is the
//! one action that says "try again".
//!
//! # The chain is a recipe
//!
//! The engine takes the stages' ownership and drops the ones that fail, so the
//! app cannot keep the stages: it keeps the recipes and builds a fresh chain
//! whenever one is installed.

use std::sync::Arc;

use rustamp_core::audio::DspStage;
use rustamp_core::error::ErrorInfo;

/// The app's DSP chain: the stages the app installs in the engine, in order.
///
/// The chain is empty until a stage can be built for it. A classic DSP
/// plug-in runs in the 32-bit plug-in host, and the host has no message that
/// could carry a block of audio to the engine's decode thread, so the app has
/// no engine-side stage for a plug-in yet (RFC 0020 Q6 is open).
#[derive(Clone, Default)]
pub struct DspChain {
    /// How to build each stage, in order.
    recipes: Vec<Arc<dyn Fn() -> Box<dyn DspStage> + Send + Sync>>,
    /// The names of the stages of the last chain the app built, in order.
    ///
    /// They are the stages' own names ([`DspStage::name`]), so the app and
    /// the engine name a stage the same way.
    names: Vec<String>,
}

impl DspChain {
    /// Adds a stage to the chain.
    ///
    /// The recipe is called once per installation: the engine drops the stage
    /// it was given when that stage fails, so a second installation needs a
    /// second stage.
    pub fn with_stage(
        mut self,
        build: impl Fn() -> Box<dyn DspStage> + Send + Sync + 'static,
    ) -> Self {
        self.recipes.push(Arc::new(build));
        self
    }

    /// The names of the stages of the last chain the app built, in order.
    pub fn stage_names(&self) -> Vec<&str> {
        self.names.iter().map(String::as_str).collect()
    }

    /// Builds the chain's stages, in order, and remembers their names.
    pub fn build(&mut self) -> Vec<Box<dyn DspStage>> {
        let stages: Vec<Box<dyn DspStage>> = self.recipes.iter().map(|build| build()).collect();
        self.names = stages.iter().map(|stage| stage.name().to_owned()).collect();
        stages
    }
}

/// A DSP stage the engine bypassed after it failed.
///
/// The engine reports the bypass once per run, in the message of an
/// [`Event::Error`](rustamp_core::event::Event::Error) with the kind
/// [`Dsp`](rustamp_core::error::ErrorKind::Dsp). The app remembers what the
/// report said, so the preferences page can say that the chain runs without
/// that stage instead of claiming that a DSP is running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BypassedDsp {
    /// The stage's name, as the engine reported it.
    pub stage: String,
    /// Why the stage failed, as the engine reported it.
    pub cause: String,
}

impl BypassedDsp {
    /// Reads the bypass out of the engine's failure report.
    ///
    /// The engine's report has a fixed shape (see `rustamp-audio`'s
    /// `report_stage_failure`): `dsp error: stage '<name>' failed: <cause>;
    /// the track ends here and the stage is bypassed from now on; <what is
    /// lost>`. The app reads the stage and the cause out of it and shows the
    /// rest of the report to nobody: the loss of the held-back frames is the
    /// engine's accounting, and the app's note says what the user can act on.
    ///
    /// A report that does not have that shape is still a bypass — the engine
    /// only reports a `Dsp` error when a stage failed — so the whole message
    /// becomes the cause then.
    pub fn from_error(info: &ErrorInfo) -> Self {
        let message = &info.message;
        BypassedDsp {
            stage: between(message, "stage '", '\'').unwrap_or_else(|| message.clone()),
            cause: between(message, "failed: ", ';').unwrap_or_else(|| message.clone()),
        }
    }
}

/// The text between `start` and the next `end` after it.
fn between(text: &str, start: &str, end: char) -> Option<String> {
    let rest = text.split_once(start)?.1;
    let tail = rest.find(end)?;
    Some(rest[..tail].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustamp_core::audio::AudioSpec;
    use rustamp_core::error::{ErrorKind, Result};

    /// The report the engine writes for a failed stage (see
    /// `rustamp-audio`'s `report_stage_failure`).
    fn report(stage: &str, cause: &str) -> ErrorInfo {
        ErrorInfo {
            kind: ErrorKind::Dsp,
            message: format!(
                "dsp error: stage '{stage}' failed: {cause}; the track ends here and the stage \
                 is bypassed from now on; the block it was processing is lost"
            ),
        }
    }

    #[test]
    fn the_stage_and_the_cause_are_read_out_of_the_report() {
        let bypass = BypassedDsp::from_error(&report("resident-dsp", "the helper died"));
        assert_eq!(bypass.stage, "resident-dsp");
        assert_eq!(bypass.cause, "the helper died");
    }

    #[test]
    fn a_report_without_that_shape_still_names_the_bypass() {
        let bypass = BypassedDsp::from_error(&ErrorInfo {
            kind: ErrorKind::Dsp,
            message: "dsp error: something else went wrong".to_owned(),
        });
        assert_eq!(bypass.stage, "dsp error: something else went wrong");
        assert_eq!(bypass.cause, "dsp error: something else went wrong");
    }

    #[test]
    fn a_chain_builds_its_stages_in_order() {
        let mut chain = DspChain::default()
            .with_stage(|| Box::new(NullStage("first")))
            .with_stage(|| Box::new(NullStage("second")));
        assert_eq!(chain.stage_names(), Vec::<&str>::new());
        let stages = chain.build();
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].name(), "first");
        assert_eq!(stages[1].name(), "second");
        // The chain knows the names of the stages it built: they are the
        // stages' own, so the app and the engine name a stage the same way.
        assert_eq!(chain.stage_names(), vec!["first", "second"]);
    }

    #[test]
    fn a_chain_builds_fresh_stages_every_time() {
        // The engine drops a stage that failed, so a second installation
        // needs a second stage: the recipe builds one per call.
        let mut chain = DspChain::default().with_stage(|| Box::new(NullStage("stage")));
        assert_eq!(chain.build().len(), 1);
        assert_eq!(chain.build().len(), 1);
    }

    /// A stage that does nothing, for the chain tests.
    struct NullStage(&'static str);

    impl DspStage for NullStage {
        fn name(&self) -> &str {
            self.0
        }

        fn prepare(&mut self, _spec: AudioSpec) -> Result<()> {
            Ok(())
        }

        fn process(&mut self, _buf: &mut [f32]) -> Result<()> {
            Ok(())
        }

        fn reset(&mut self) {}
    }
}
