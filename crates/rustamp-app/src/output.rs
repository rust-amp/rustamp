// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! App-owned output selection and control-thread downgrade reporting.
//!
//! Shared mode retains `default_output()` (WASAPI with cpal fallback). An
//! exclusive request tries WASAPI first; WASAPI itself retries shared mode on
//! the same endpoint. If neither mode opens, cpal remains the final fallback.
//! Outside Windows, exclusive mode is unavailable: use the normal cpal path
//! and report that limitation. Explicit null/silent choices override the mode.
//!
//! The **device** is the configuration's (`audio.device`). The app builds the
//! plan with [`rustamp_output::selection::plan`] — the device to open and the
//! mode to open it in — and resolves it against the devices that exist
//! ([`rustamp_output::selection::resolve`]). A name that matches no device is
//! reported and the player falls back to the default endpoint: visibly, never
//! silently on another device. A resolved name is carried into the backend
//! that opens the device, and the cpal fallback opens the *same* device.
//!
//! The exclusive constructor does not open hardware, and no selection path
//! initialises COM on the caller's thread. The default backend probe and
//! WASAPI open own their COM threads, leaving
//! the GUI thread free for winit's STA/OleInitialize (avoiding
//! RPC_E_CHANGED_MODE when the window enables drag and drop).
//! Real exclusive opens are VM-only; headless tests cover selection/reporting,
//! not a claim that a device grants (or refuses) exclusive access.

use std::sync::mpsc::{Receiver, Sender, channel};

use rustamp_core::audio::OutputDevice;
use rustamp_core::config::OutputMode;
use rustamp_output::selection::{self, DeviceTarget, OutputPlan};
use rustamp_output::{CpalOutput, NullClock, NullOutput, default_output};

#[cfg(windows)]
use rustamp_output::WasapiMode;

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

/// The device the player opens, and what the user must be told about it.
#[derive(Debug, PartialEq, Eq)]
struct DeviceChoice {
    /// The device to open: the system default endpoint, or a device by name.
    target: DeviceTarget,
    /// The note to report: the configured device could not be selected.
    note: Option<String>,
}

/// Resolves the configured device against the devices that exist.
///
/// A name that matches no device (or several) is **reported**: the note names
/// the wanted device and the devices that do exist, and the player falls back
/// to the default endpoint — visibly, never silently on another device.
///
/// The decision is pure — the devices that exist are the input — so it is
/// testable without an audio device.
fn resolve_device(plan: &OutputPlan, available: &[String]) -> DeviceChoice {
    match selection::resolve(plan, available) {
        Ok(resolved) => DeviceChoice {
            target: resolved.device,
            note: None,
        },
        Err(error) => DeviceChoice {
            target: DeviceTarget::Default,
            note: Some(format!(
                "The configured output device could not be selected: {error}. \
                 The default output device is used instead."
            )),
        },
    }
}

/// The devices this machine has, by name — or `None` where the platform
/// cannot enumerate them.
///
/// Windows enumerates its render endpoints on a thread of its own: the
/// enumeration needs COM, and the thread that goes on to create the window
/// must keep its apartment free for `winit`'s `OleInitialize`. Elsewhere the
/// output crate offers no enumeration, so the configured name goes to the
/// backend unchanged: the backend looks the device up with the same rule and
/// refuses to open a device that does not exist — it never plays on another
/// device silently.
#[cfg(windows)]
fn available_devices() -> Option<Vec<String>> {
    Some(selection::render_endpoint_names())
}

#[cfg(not(windows))]
fn available_devices() -> Option<Vec<String>> {
    None
}

/// The device to open for the configured plan, and the note to report.
fn choose_device(plan: &OutputPlan) -> DeviceChoice {
    match available_devices() {
        Some(available) => resolve_device(plan, &available),
        // Without an enumeration the backend resolves the name itself, with
        // the same rule, and refuses a device that does not exist.
        None => DeviceChoice {
            target: plan.device.clone(),
            note: None,
        },
    }
}

/// The name of the device a target names, if it names one.
fn target_name(target: &DeviceTarget) -> Option<&str> {
    match target {
        DeviceTarget::Named(name) => Some(name.as_str()),
        DeviceTarget::Default => None,
    }
}

/// The cpal output for a device: the device that was requested.
///
/// A named device stays that device — the fallback never plays on another
/// one — and the default endpoint is the default cpal device.
fn cpal_fallback(target: &DeviceTarget) -> CpalOutput {
    match target_name(target) {
        Some(name) => CpalOutput::with_device_name(name.to_owned()),
        None => CpalOutput::new(),
    }
}

/// The note for an exclusive request that this platform cannot grant.
#[cfg(not(windows))]
fn exclusive_unsupported_note(target: &DeviceTarget) -> String {
    match target_name(target) {
        Some(name) => format!(
            "Exclusive audio output is only supported on Windows; using the \
             cpal output on \"{name}\" instead."
        ),
        None => "Exclusive audio output is only supported on Windows; using the default cpal output instead."
            .to_owned(),
    }
}

/// The backend's mode for the configured mode.
///
/// The plan carries the configuration's own mode; the backend has its own
/// type for it, and the player maps the request onto it (RFC 0019).
#[cfg(windows)]
fn wasapi_mode(mode: OutputMode) -> WasapiMode {
    match mode {
        OutputMode::Shared => WasapiMode::Shared,
        OutputMode::Exclusive => WasapiMode::Exclusive,
    }
}

/// The shared-mode output for the chosen device.
///
/// Without a name the machine's default backend decides (WASAPI when shared
/// mode is available, cpal otherwise). A named device is opened on WASAPI,
/// and the cpal fallback behind it opens the same device.
#[cfg(windows)]
fn shared_output(
    notes: Sender<String>,
    target: &DeviceTarget,
    mode: WasapiMode,
) -> Box<dyn OutputDevice> {
    match target {
        DeviceTarget::Default => default_output(),
        DeviceTarget::Named(_) => Box::new(windows::WindowsOutput::new(notes, target, mode)),
    }
}

#[cfg(not(windows))]
fn shared_output(_notes: Sender<String>, target: &DeviceTarget) -> Box<dyn OutputDevice> {
    match target {
        DeviceTarget::Default => default_output(),
        DeviceTarget::Named(_) => Box::new(cpal_fallback(target)),
    }
}

pub(crate) struct CreatedOutput {
    pub device: Box<dyn OutputDevice>,
    pub clock: Option<NullClock>,
    pub notes: Receiver<String>,
}

pub(crate) fn create_output(
    choice: OutputChoice,
    device: Option<&str>,
    mode: OutputMode,
) -> CreatedOutput {
    let (notes_tx, notes) = channel();
    // The plan is pure: the device to open and the mode to open it in. It
    // opens nothing and reads no state, so it can be built before a device is
    // touched.
    let plan = selection::plan(device, mode);
    let (device, clock): (Box<dyn OutputDevice>, _) = match selection(choice, mode) {
        Selection::Null => {
            let (output, clock) = NullOutput::pair();
            (Box::new(output), Some(clock))
        }
        Selection::Default => {
            let chosen = choose_device(&plan);
            if let Some(note) = chosen.note {
                let _ = notes_tx.send(note);
            }
            #[cfg(windows)]
            let output: Box<dyn OutputDevice> =
                shared_output(notes_tx, &chosen.target, wasapi_mode(plan.mode));
            #[cfg(not(windows))]
            let output: Box<dyn OutputDevice> = shared_output(notes_tx, &chosen.target);
            (output, None)
        }
        Selection::Exclusive => {
            let chosen = choose_device(&plan);
            if let Some(note) = chosen.note {
                let _ = notes_tx.send(note);
            }
            #[cfg(windows)]
            let output: Box<dyn OutputDevice> = Box::new(windows::WindowsOutput::new(
                notes_tx,
                &chosen.target,
                wasapi_mode(plan.mode),
            ));
            #[cfg(not(windows))]
            let output: Box<dyn OutputDevice> = {
                let _ = notes_tx.send(exclusive_unsupported_note(&chosen.target));
                let device: Box<dyn OutputDevice> = match &chosen.target {
                    DeviceTarget::Default => default_output(),
                    DeviceTarget::Named(_) => Box::new(cpal_fallback(&chosen.target)),
                };
                device
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
    use rustamp_output::selection::DeviceTarget;
    use rustamp_output::{CpalOutput, WasapiMode, WasapiOutput};

    use super::cpal_fallback;

    enum Backend {
        Wasapi(WasapiOutput),
        Cpal(CpalOutput),
    }

    /// The app's Windows output: WASAPI, with the cpal fallback behind it.
    ///
    /// WASAPI is the primary backend on Windows. When it cannot open the
    /// endpoint, the cpal backend opens the **same** device — the name is
    /// carried over — and the downgrade is reported, so the player never
    /// plays on another device behind the user's back.
    ///
    /// The constructor does not open hardware: the device and the mode are
    /// requests, and the endpoint decides whether it grants them.
    pub(super) struct WindowsOutput {
        backend: Backend,
        notes: Sender<String>,
        /// The device that was requested: the cpal fallback opens the same one.
        target: DeviceTarget,
        /// The mode the stream was requested in.
        mode: WasapiMode,
    }

    impl WindowsOutput {
        /// A WASAPI output for the requested device and mode.
        pub(super) fn new(notes: Sender<String>, target: &DeviceTarget, mode: WasapiMode) -> Self {
            let mut wasapi = match target {
                DeviceTarget::Named(name) => WasapiOutput::with_device_name(name.clone()),
                DeviceTarget::Default => WasapiOutput::new(),
            };
            wasapi = wasapi.with_mode(mode);
            Self {
                backend: Backend::Wasapi(wasapi),
                notes,
                target: target.clone(),
                mode,
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

    impl OutputDevice for WindowsOutput {
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
                        downgrade_note(self.mode, output.mode_in_use(), output.downgrade_reason())
                    {
                        let _ = self.notes.send(note);
                    }
                    Ok(format)
                }
                Err(wasapi_error) => {
                    // WASAPI has already tried shared on the same endpoint.
                    // Preserve the cross-platform backend fallback, visibly,
                    // and on the device that was requested.
                    let mut fallback = cpal_fallback(&self.target);
                    let format = fallback.open(spec).map_err(|cpal_error| {
                        Error::Output(fallback_error(
                            self.mode,
                            &wasapi_error.to_string(),
                            &cpal_error.to_string(),
                        ))
                    })?;
                    let _ = self.notes.send(fallback_note(
                        self.mode,
                        &wasapi_error.to_string(),
                        fallback.name(),
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

    /// The note for an exclusive request that the endpoint refused.
    ///
    /// A shared request that stays shared is no downgrade, so it says
    /// nothing.
    fn downgrade_note(
        requested: WasapiMode,
        in_use: WasapiMode,
        reason: Option<&str>,
    ) -> Option<String> {
        if requested != WasapiMode::Exclusive || in_use == WasapiMode::Exclusive {
            return None;
        }
        Some(format!(
            "Exclusive audio output was refused; using WASAPI shared mode: {}",
            reason.unwrap_or("the endpoint did not provide a refusal reason")
        ))
    }

    /// The note for a WASAPI open that failed and the cpal fallback took over.
    fn fallback_note(mode: WasapiMode, error: &str, device: &str) -> String {
        match mode {
            WasapiMode::Exclusive => format!(
                "Exclusive/shared WASAPI could not open: {error}; using {device} instead (exclusive mode is unavailable)."
            ),
            WasapiMode::Shared => {
                format!("Shared WASAPI could not open: {error}; using {device} instead.")
            }
        }
    }

    /// The error for a WASAPI open that failed and whose cpal fallback failed
    /// too: both reasons, so the user learns what went wrong.
    fn fallback_error(mode: WasapiMode, wasapi: &str, cpal: &str) -> String {
        match mode {
            WasapiMode::Exclusive => {
                format!("Exclusive/shared WASAPI failed: {wasapi}; cpal fallback failed: {cpal}")
            }
            WasapiMode::Shared => {
                format!("Shared WASAPI failed: {wasapi}; cpal fallback failed: {cpal}")
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn the_exclusive_selection_requests_exclusive_without_opening_a_device() {
            let (tx, rx) = std::sync::mpsc::channel();
            let output = WindowsOutput::new(tx, &DeviceTarget::Default, WasapiMode::Exclusive);
            let Backend::Wasapi(wasapi) = output.backend else {
                panic!("an exclusive request must try WASAPI first");
            };
            assert_eq!(wasapi.requested_mode(), WasapiMode::Exclusive);
            assert!(rx.try_recv().is_err(), "no open, no downgrade claim");
        }

        #[test]
        fn a_named_device_is_requested_from_wasapi() {
            let (tx, _rx) = std::sync::mpsc::channel();
            let target = DeviceTarget::Named("Speakers (Realtek)".to_owned());
            let output = WindowsOutput::new(tx, &target, WasapiMode::Shared);
            let Backend::Wasapi(wasapi) = output.backend else {
                panic!("a named device must be opened on WASAPI first");
            };
            assert_eq!(wasapi.requested_mode(), WasapiMode::Shared);
            assert_eq!(
                output.target,
                DeviceTarget::Named("Speakers (Realtek)".to_owned()),
                "the output keeps the device that was requested"
            );
        }

        #[test]
        fn shared_after_an_exclusive_request_reports_the_actual_refusal() {
            let note = downgrade_note(
                WasapiMode::Exclusive,
                WasapiMode::Shared,
                Some("endpoint busy"),
            )
            .expect("a refused exclusive request is reported");
            assert!(note.contains("WASAPI shared"), "{note}");
            assert!(note.contains("endpoint busy"), "{note}");
            assert!(
                downgrade_note(WasapiMode::Exclusive, WasapiMode::Exclusive, None).is_none(),
                "an exclusive stream that was granted is no downgrade"
            );
            assert!(
                downgrade_note(WasapiMode::Shared, WasapiMode::Shared, None).is_none(),
                "a shared request that stays shared is no downgrade"
            );
        }

        #[test]
        fn the_fallback_note_names_the_device_that_took_over() {
            let note = fallback_note(WasapiMode::Shared, "the endpoint is gone", "Speakers");
            assert!(note.contains("Shared WASAPI"), "{note}");
            assert!(note.contains("the endpoint is gone"), "{note}");
            assert!(note.contains("Speakers"), "{note}");
            let exclusive = fallback_note(WasapiMode::Exclusive, "denied", "Speakers");
            assert!(
                exclusive.contains("exclusive mode is unavailable"),
                "{exclusive}"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The devices a machine has.
    fn devices() -> Vec<String> {
        vec![
            "Speakers (Realtek)".to_owned(),
            "Headphones (USB)".to_owned(),
            "CABLE Output".to_owned(),
        ]
    }

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
                let output = create_output(choice, None, mode);
                assert!(output.clock.is_some());
                assert!(output.notes.try_recv().is_err());
            }
        }
    }

    #[test]
    fn no_device_name_selects_the_system_default() {
        let plan = selection::plan(None, OutputMode::Shared);
        let chosen = resolve_device(&plan, &devices());
        assert_eq!(chosen.target, DeviceTarget::Default);
        assert!(chosen.note.is_none(), "the default needs no explanation");
    }

    #[test]
    fn a_device_name_pins_that_device() {
        // The configured text is resolved to the device that exists: the plan
        // carries the friendly name, which is what the backends match by.
        let plan = selection::plan(Some("cable output"), OutputMode::Shared);
        let chosen = resolve_device(&plan, &devices());
        assert_eq!(
            chosen.target,
            DeviceTarget::Named("CABLE Output".to_owned()),
            "the plan names the device that exists, not the configured text"
        );
        assert!(chosen.note.is_none());
        // The mode survives the lookup and reaches the backend.
        assert_eq!(plan.mode, OutputMode::Shared);
    }

    #[test]
    fn the_mode_is_carried_through_the_plan() {
        for mode in [OutputMode::Shared, OutputMode::Exclusive] {
            let plan = selection::plan(Some("Speakers"), mode);
            assert_eq!(plan.mode, mode);
            let resolved = selection::resolve(&plan, &devices()).expect("resolves");
            assert_eq!(resolved.mode, mode, "the mode survives the lookup");
        }
    }

    #[test]
    fn a_device_that_does_not_exist_is_reported_and_the_default_is_used() {
        let plan = selection::plan(Some("CABLE Input"), OutputMode::Shared);
        let chosen = resolve_device(&plan, &devices());
        assert_eq!(
            chosen.target,
            DeviceTarget::Default,
            "the fallback is the default endpoint"
        );
        let note = chosen.note.expect("the user must see it");
        assert!(note.contains("CABLE Input"), "{note}");
        assert!(note.contains("Speakers (Realtek)"), "{note}");
        assert!(
            note.contains("default output device"),
            "the note says what happens instead: {note}"
        );
    }

    #[test]
    fn an_ambiguous_device_name_is_reported_too() {
        let many = vec!["CABLE Input".to_owned(), "CABLE Output".to_owned()];
        let plan = selection::plan(Some("CABLE"), OutputMode::Shared);
        let chosen = resolve_device(&plan, &many);
        assert_eq!(chosen.target, DeviceTarget::Default);
        let note = chosen.note.expect("the user must see it");
        assert!(note.contains("CABLE"), "{note}");
        assert!(note.contains("2"), "{note}");
    }

    #[test]
    fn an_empty_device_name_is_reported() {
        let plan = selection::plan(Some("   "), OutputMode::Shared);
        let chosen = resolve_device(&plan, &devices());
        assert_eq!(chosen.target, DeviceTarget::Default);
        assert!(
            chosen.note.is_some(),
            "a blank name is an error, not a device"
        );
    }

    #[test]
    fn the_cpal_fallback_opens_the_requested_device() {
        // The fallback must not switch to another device: it opens the device
        // that was requested, under the name the backend matches by.
        let target = DeviceTarget::Named("CABLE Output".to_owned());
        assert_eq!(target_name(&target), Some("CABLE Output"));
        assert_eq!(target_name(&DeviceTarget::Default), None);
        // The name the fallback is given selects exactly the device that was
        // requested — the rule the backend matches by.
        let name = target_name(&target).expect("a named target names a device");
        let available = devices();
        let matching: Vec<&String> = available
            .iter()
            .filter(|device| selection::matches(name, device))
            .collect();
        assert_eq!(
            matching,
            vec!["CABLE Output"],
            "the fallback's name must select the requested device and no other"
        );
    }

    #[test]
    #[cfg(not(windows))]
    fn an_exclusive_request_on_another_platform_says_what_is_used_instead() {
        let note = exclusive_unsupported_note(&DeviceTarget::Named("Speakers".to_owned()));
        assert!(note.contains("only supported on Windows"), "{note}");
        assert!(note.contains("Speakers"), "{note}");
        let default = exclusive_unsupported_note(&DeviceTarget::Default);
        assert!(default.contains("default cpal output"), "{default}");
    }

    #[test]
    #[cfg(windows)]
    fn a_configured_device_that_does_not_exist_is_reported() {
        // The end-to-end path: the configured name is resolved against the
        // machine's render endpoints, and a name that matches none is
        // reported — the player never plays on another device silently.
        let output = create_output(
            OutputChoice::Default,
            Some("rustamp-no-such-output-device"),
            OutputMode::Shared,
        );
        let notes: Vec<_> = output.notes.try_iter().collect();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(
            notes[0].contains("rustamp-no-such-output-device"),
            "{notes:?}"
        );
        assert!(notes[0].contains("default output device"), "{notes:?}");
    }

    #[test]
    #[cfg(windows)]
    #[ignore = "VM-only: opens a real audio endpoint; never run on the host"]
    fn the_app_exclusive_path_opens_a_real_endpoint_or_reports_its_downgrade() {
        use rustamp_core::audio::AudioSpec;

        let mut output = create_output(OutputChoice::Default, None, OutputMode::Exclusive);
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
