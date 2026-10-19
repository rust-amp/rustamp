// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! App-owned output selection and control-thread downgrade reporting.
//!
//! Shared mode retains `default_output()` (WASAPI with cpal fallback). An
//! exclusive request tries WASAPI first; WASAPI itself retries shared mode on
//! the same endpoint. If neither mode opens, cpal remains the final fallback.
//! Outside Windows, exclusive mode is unavailable: use the normal cpal path
//! and report that limitation. Explicit null/silent choices override the mode.
//!
//! The exclusive constructor does not open hardware, and no selection path
//! initialises COM on the caller's thread. The default backend probe and
//! WASAPI open own their COM threads, leaving
//! the GUI thread free for winit's STA/OleInitialize (avoiding
//! RPC_E_CHANGED_MODE when the window enables drag and drop).
//! Real exclusive opens are VM-only; headless tests cover selection/reporting,
//! not a claim that a device grants (or refuses) exclusive access.

use std::sync::mpsc::{Receiver, channel};

use rustamp_core::audio::OutputDevice;
use rustamp_core::config::OutputMode;
use rustamp_output::{NullClock, NullOutput, default_output};

use crate::cli::OutputChoice;

#[derive(Debug, PartialEq, Eq)]
enum Selection {
    Null,
    Default,
    Exclusive,
}

fn selection(choice: OutputChoice, mode: OutputMode) -> Selection {
    match (choice, mode) {
        (OutputChoice::Null | OutputChoice::NoAudio, _) => Selection::Null,
        (OutputChoice::Default, OutputMode::Shared) => Selection::Default,
        (OutputChoice::Default, OutputMode::Exclusive) => Selection::Exclusive,
    }
}

pub(crate) struct CreatedOutput {
    pub device: Box<dyn OutputDevice>,
    pub clock: Option<NullClock>,
    pub notes: Receiver<String>,
}

pub(crate) fn create_output(choice: OutputChoice, mode: OutputMode) -> CreatedOutput {
    let (notes_tx, notes) = channel();
    let (device, clock): (Box<dyn OutputDevice>, _) = match selection(choice, mode) {
        Selection::Null => {
            let (output, clock) = NullOutput::pair();
            (Box::new(output), Some(clock))
        }
        Selection::Default => (default_output(), None),
        Selection::Exclusive => {
            #[cfg(windows)]
            let output: Box<dyn OutputDevice> = Box::new(windows::ExclusiveOutput::new(notes_tx));
            #[cfg(not(windows))]
            let output = {
                let _ = notes_tx.send(
                    "Exclusive audio output is only supported on Windows; using the default cpal output instead."
                        .into(),
                );
                default_output()
            };
            (output, None)
        }
    };
    CreatedOutput {
        device,
        clock,
        notes,
    }
}

#[cfg(windows)]
mod windows {
    use std::sync::mpsc::Sender;
    use std::time::Duration;

    use rustamp_core::audio::{AudioSpec, OutputDevice, OutputFormat, RenderFn};
    use rustamp_core::error::{Error, Result};
    use rustamp_output::{CpalOutput, WasapiMode, WasapiOutput};

    enum Backend {
        Wasapi(WasapiOutput),
        Cpal(CpalOutput),
    }

    /// Keeps the concrete WASAPI status accessible after the engine opens it.
    /// Messages are sent only by `open`, never by the render callback or poll.
    pub(super) struct ExclusiveOutput {
        backend: Backend,
        notes: Sender<String>,
    }

    fn downgrade_note(mode: WasapiMode, reason: Option<&str>) -> Option<String> {
        match mode {
            WasapiMode::Exclusive => None,
            WasapiMode::Shared => Some(format!(
                "Exclusive audio output was refused; using WASAPI shared mode: {}",
                reason.unwrap_or("the endpoint did not provide a refusal reason")
            )),
        }
    }

    impl ExclusiveOutput {
        pub(super) fn new(notes: Sender<String>) -> Self {
            Self {
                backend: Backend::Wasapi(WasapiOutput::new().with_mode(WasapiMode::Exclusive)),
                notes,
            }
        }

        fn device(&self) -> &dyn OutputDevice {
            match &self.backend {
                Backend::Wasapi(output) => output,
                Backend::Cpal(output) => output,
            }
        }

        fn device_mut(&mut self) -> &mut dyn OutputDevice {
            match &mut self.backend {
                Backend::Wasapi(output) => output,
                Backend::Cpal(output) => output,
            }
        }
    }

    impl OutputDevice for ExclusiveOutput {
        fn name(&self) -> &str {
            self.device().name()
        }

        fn open(&mut self, spec: AudioSpec) -> Result<OutputFormat> {
            let Backend::Wasapi(output) = &mut self.backend else {
                return self.device_mut().open(spec);
            };
            match output.open(spec) {
                Ok(format) => {
                    if let Some(note) =
                        downgrade_note(output.mode_in_use(), output.downgrade_reason())
                    {
                        let _ = self.notes.send(note);
                    }
                    Ok(format)
                }
                Err(wasapi_error) => {
                    // WASAPI has already tried shared on the same endpoint.
                    // Preserve the cross-platform backend fallback, visibly.
                    let mut fallback = CpalOutput::new();
                    let format = fallback.open(spec).map_err(|cpal_error| {
                        Error::Output(format!(
                            "Exclusive/shared WASAPI failed: {wasapi_error}; cpal fallback failed: {cpal_error}"
                        ))
                    })?;
                    let _ = self.notes.send(format!(
                        "Exclusive/shared WASAPI could not open: {wasapi_error}; using {} instead (exclusive mode is unavailable).",
                        fallback.name()
                    ));
                    self.backend = Backend::Cpal(fallback);
                    Ok(format)
                }
            }
        }

        fn start(&mut self, render: RenderFn) -> Result<()> {
            self.device_mut().start(render)
        }

        fn pause(&mut self) -> Result<()> {
            self.device_mut().pause()
        }

        fn resume(&mut self) -> Result<()> {
            self.device_mut().resume()
        }

        fn flush(&mut self) -> Result<()> {
            self.device_mut().flush()
        }

        fn latency(&self) -> Duration {
            self.device().latency()
        }

        fn played_frames(&self) -> u64 {
            self.device().played_frames()
        }

        fn set_volume(&mut self, volume: f32) -> Result<()> {
            self.device_mut().set_volume(volume)
        }

        fn stream_error(&self) -> Option<Error> {
            self.device().stream_error()
        }

        fn close(&mut self) -> Result<()> {
            self.device_mut().close()
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_exclusive_selection_requests_exclusive_without_opening_a_device() {
            let (tx, rx) = std::sync::mpsc::channel();
            let output = ExclusiveOutput::new(tx);
            let Backend::Wasapi(wasapi) = output.backend else {
                panic!("an exclusive request must try WASAPI first");
            };
            assert_eq!(wasapi.requested_mode(), WasapiMode::Exclusive);
            assert!(rx.try_recv().is_err(), "no open, no downgrade claim");
        }

        #[test]
        fn shared_after_an_exclusive_request_reports_the_actual_refusal() {
            let note = downgrade_note(WasapiMode::Shared, Some("endpoint busy")).unwrap();
            assert!(note.contains("WASAPI shared"), "{note}");
            assert!(note.contains("endpoint busy"), "{note}");
            assert!(downgrade_note(WasapiMode::Exclusive, None).is_none());
            assert!(downgrade_note(WasapiMode::Shared, None).is_some());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_mode_selects_the_default_or_exclusive_path() {
        assert_eq!(
            selection(OutputChoice::Default, OutputMode::Shared),
            Selection::Default
        );
        assert_eq!(
            selection(OutputChoice::Default, OutputMode::Exclusive),
            Selection::Exclusive
        );
    }

    #[test]
    fn explicit_silent_outputs_override_both_config_modes() {
        for choice in [OutputChoice::Null, OutputChoice::NoAudio] {
            for mode in [OutputMode::Shared, OutputMode::Exclusive] {
                assert_eq!(selection(choice, mode), Selection::Null);
                let output = create_output(choice, mode);
                assert!(output.clock.is_some());
                assert!(output.notes.try_recv().is_err());
            }
        }
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "VM-only: opens a real audio endpoint; never run on the host"]
    fn the_app_exclusive_path_opens_a_real_endpoint_or_reports_its_downgrade() {
        use rustamp_core::audio::AudioSpec;

        let mut output = create_output(OutputChoice::Default, OutputMode::Exclusive);
        output
            .device
            .open(AudioSpec::new(44_100, 2))
            .expect("the VM needs an audio endpoint");
        let name = output.device.name().to_owned();
        let notes: Vec<_> = output.notes.try_iter().collect();
        if name.contains("[wasapi exclusive]") {
            assert!(notes.is_empty(), "exclusive access was granted: {notes:?}");
        } else {
            assert!(
                !notes.is_empty(),
                "a downgrade must report its reason: {name}"
            );
        }
        output.device.close().expect("close the real endpoint");
    }
}
