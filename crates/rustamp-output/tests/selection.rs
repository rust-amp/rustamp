// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The device selection, seen from outside the crate: **headless**.
//!
//! The unit tests of [`rustamp_output::selection`] cover the decision; this
//! test pins the *public* path the player uses — the configuration's device
//! name and output mode in, the plan (and the typed error) out.
//!
//! It opens no device, starts no thread of its own and needs no audio
//! endpoint, so it runs on any machine.

use rustamp_core::config::{AudioConfig, OutputMode};
use rustamp_output::selection::{DeviceError, DeviceTarget, matches, plan, resolve};

/// The devices a machine has.
fn devices() -> Vec<String> {
    vec![
        "Speakers (Realtek)".to_owned(),
        "Headphones (USB)".to_owned(),
        "CABLE Output".to_owned(),
    ]
}

#[test]
fn a_configuration_without_a_device_asks_for_the_default_endpoint() {
    let config = AudioConfig::default();
    assert_eq!(config.device, None);

    let plan = plan(config.device.as_deref(), config.output_mode);
    assert_eq!(plan.device, DeviceTarget::Default);
    assert_eq!(plan.mode, OutputMode::Shared);
}

#[test]
fn a_configuration_with_a_device_asks_for_that_device() {
    let config = AudioConfig {
        device: Some("CABLE Output".to_owned()),
        output_mode: OutputMode::Exclusive,
        ..AudioConfig::default()
    };

    let plan = plan(config.device.as_deref(), config.output_mode);
    assert_eq!(plan.device, DeviceTarget::Named("CABLE Output".to_owned()));
    assert_eq!(plan.mode, OutputMode::Exclusive);

    let resolved = resolve(&plan, &devices()).expect("the device exists");
    assert_eq!(
        resolved.device,
        DeviceTarget::Named("CABLE Output".to_owned())
    );
    assert_eq!(resolved.mode, OutputMode::Exclusive);
}

#[test]
fn a_device_that_does_not_exist_is_a_typed_error_the_caller_decides_about() {
    let plan = plan(Some("CABLE Input"), OutputMode::Shared);
    let error = resolve(&plan, &devices()).expect_err("no device is named like that");
    assert!(
        matches!(&error, DeviceError::NotFound { wanted, .. } if wanted == "CABLE Input"),
        "{error:?}"
    );
    // The error carries the devices that do exist, so the caller can report
    // them and decide the fallback.
    let message = error.to_string();
    assert!(message.contains("CABLE Input"), "{message}");
    assert!(message.contains("Speakers (Realtek)"), "{message}");
    // It converts into the crate-wide error type as well.
    let error: rustamp_core::error::Error = error.into();
    assert!(error.to_string().contains("CABLE Input"), "{error}");
}

#[test]
fn the_matching_rule_is_trimmed_and_case_insensitive() {
    assert!(matches("speakers", "Speakers (Realtek)"));
    assert!(matches("  SPEAKERS  ", "Speakers (Realtek)"));
    assert!(!matches("", "Speakers (Realtek)"));
}
