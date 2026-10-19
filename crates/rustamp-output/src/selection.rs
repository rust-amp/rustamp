// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Which device the configuration asks for, and whether that device exists.
//!
//! The configuration names the output device (`audio.device`) and the mode
//! (`audio.output_mode`). Two questions follow from it, and this module keeps
//! them apart:
//!
//! 1. **What should be opened?** [`plan`] turns the configuration into an
//!    [`OutputPlan`]: the system default endpoint or the named device, and the
//!    mode to open it in. The plan is *pure* — it opens nothing, enumerates
//!    nothing and reads no state.
//! 2. **Does that device exist?** [`resolve`] answers that against the list of
//!    devices that are present. A name that matches nothing is
//!    [`DeviceError::NotFound`], a name that matches several is
//!    [`DeviceError::Ambiguous`], and a name that is empty is
//!    [`DeviceError::EmptyName`]. The **caller** decides what to do with the
//!    error: the player may report it and fall back to the default device, or
//!    it may refuse to play. Nothing here opens a device, so that decision
//!    stays the caller's.
//!
//! # The matching rule
//!
//! A device matches a name when the **trimmed**, **case-insensitive** name is
//! *contained* in the device's friendly name: `"Speakers"` matches
//! `"Speakers (Realtek)"`, and `" speakers "` matches `"Speakers"`. An empty
//! name matches nothing — a blank configuration is an error, not a name that
//! matches every device.
//!
//! It is the rule the backends already use to pick a device by name
//! ([`crate::CpalOutput::with_device_name`],
//! [`crate::WasapiOutput::with_device_name`]), and both of them share this
//! module's implementation of it ([`matches`]) — so a name that resolves here
//! resolves there too, and the two can never disagree about which device is
//! meant.
//!
//! # The mode
//!
//! The plan carries [`OutputMode`] — the configuration's own type — and not
//! the backend's `WasapiMode`: the mode is a *request*, and the backend
//! decides whether it can be granted (a WASAPI device that refuses an
//! exclusive stream is played in shared mode instead, and says so). The
//! player maps the request onto the backend's type when it opens the device
//! (RFC 0019).
//!
//! # The device list
//!
//! The list [`resolve`] works on is the caller's: enumerating the devices
//! needs the platform's device API, and on Windows that is
//! [`render_endpoint_names`]. The enumeration is the only platform-specific
//! part of the decision — the matching and the normalisation are pure, which
//! is what makes the whole selection testable without an audio device.

use rustamp_core::config::OutputMode;

/// The device an [`OutputPlan`] opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceTarget {
    /// The system default endpoint: the device the system plays through now.
    Default,
    /// A device by name. The text is matched against the friendly names of
    /// the devices that exist (see [`matches`]).
    Named(String),
}

/// What to open, and in which mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputPlan {
    /// The device to open.
    pub device: DeviceTarget,
    /// The mode to open it in.
    pub mode: OutputMode,
}

/// Why a named device could not be resolved.
///
/// The error is **typed** and it names the devices that do exist, so the
/// caller can report the problem and decide the fallback itself: the player
/// may fall back to the default device, or it may refuse to play. Nothing is
/// decided here.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceError {
    /// The configured name is empty (or only whitespace).
    #[error("the configured device name is empty")]
    EmptyName,
    /// No device matches the name.
    #[error("no device is named like \"{wanted}\"{}", alternatives(available))]
    NotFound {
        /// The configured name.
        wanted: String,
        /// The devices that do exist.
        available: Vec<String>,
    },
    /// More than one device matches the name.
    #[error(
        "{matches} devices are named like \"{wanted}\"{}",
        alternatives(available)
    )]
    Ambiguous {
        /// The configured name.
        wanted: String,
        /// How many devices match the name.
        matches: usize,
        /// The devices that do exist.
        available: Vec<String>,
    },
}

impl From<DeviceError> for rustamp_core::error::Error {
    /// The device could not be selected: an output failure the caller may
    /// report (and, if it chooses to, work around).
    fn from(error: DeviceError) -> Self {
        rustamp_core::error::Error::Output(error.to_string())
    }
}

/// The alternatives a failed lookup names: none, or the devices that exist.
fn alternatives(available: &[String]) -> String {
    if available.is_empty() {
        "; no device is available".to_owned()
    } else {
        format!("; available: {}", available.join(" | "))
    }
}

/// The plan for a configuration: the device to open and the mode to open it in.
///
/// `device` is the configured device name (`None` = the system default
/// endpoint) and `mode` the configured output mode. The plan is pure — it
/// opens nothing, enumerates nothing and reads no state, so it can be built
/// long before a device is touched.
///
/// The name is taken as it is configured. [`resolve`] turns it into the name
/// of a device that really exists.
pub fn plan(device: Option<&str>, mode: OutputMode) -> OutputPlan {
    OutputPlan {
        device: match device {
            None => DeviceTarget::Default,
            Some(name) => DeviceTarget::Named(name.to_owned()),
        },
        mode,
    }
}

/// Resolve the plan's device against the devices that exist.
///
/// The default endpoint needs no resolution: the plan comes back unchanged.
/// A named device is looked up with the matching rule ([`matches`]) — exactly
/// one match is the device, and the plan comes back naming **that device**
/// (the friendly name of the match, not the configured text). No match is
/// [`DeviceError::NotFound`], several matches are [`DeviceError::Ambiguous`],
/// and an empty name is [`DeviceError::EmptyName`].
///
/// The list is the caller's: enumerating the devices needs the platform's
/// device API (on Windows: [`render_endpoint_names`]), while the matching
/// itself is pure — which is what makes the decision testable without an
/// audio device.
pub fn resolve(plan: &OutputPlan, available: &[String]) -> Result<OutputPlan, DeviceError> {
    let DeviceTarget::Named(wanted) = &plan.device else {
        return Ok(plan.clone());
    };
    if wanted.trim().is_empty() {
        return Err(DeviceError::EmptyName);
    }
    let matching: Vec<&String> = available
        .iter()
        .filter(|name| matches(wanted, name))
        .collect();
    match matching.as_slice() {
        [only] => Ok(OutputPlan {
            device: DeviceTarget::Named((*only).clone()),
            mode: plan.mode,
        }),
        [] => Err(DeviceError::NotFound {
            wanted: wanted.clone(),
            available: available.to_vec(),
        }),
        many => Err(DeviceError::Ambiguous {
            wanted: wanted.clone(),
            matches: many.len(),
            available: available.to_vec(),
        }),
    }
}

/// The matching rule: a device matches a name when the trimmed, lowercased
/// name is contained in the trimmed, lowercased friendly name of the device.
///
/// "Speakers" therefore matches "Speakers (Realtek)", and " speakers "
/// matches "Speakers". An empty name matches **nothing**: a blank
/// configuration is an error, not a name that matches every device.
///
/// The backends use this rule to pick the device they open
/// ([`crate::CpalOutput::with_device_name`],
/// [`crate::WasapiOutput::with_device_name`]), so a name that resolves with
/// [`resolve`] opens that same device.
pub fn matches(wanted: &str, name: &str) -> bool {
    let needle = wanted.trim().to_lowercase();
    if needle.is_empty() {
        return false;
    }
    name.trim().to_lowercase().contains(&needle)
}

/// The friendly names of the render endpoints of this machine (Windows only).
///
/// The enumeration needs COM in the multi-threaded apartment, and an MTA
/// thread can never become a single-threaded apartment afterwards: the
/// caller — in the player, the thread that goes on to create the window — would
/// lose the ability to create a window with drag-and-drop support (`winit`
/// calls `OleInitialize`, which fails with `RPC_E_CHANGED_MODE`). The
/// enumeration therefore runs on a short-lived thread of its own, like the
/// backend probe behind [`crate::default_output`].
///
/// A machine without an endpoint yields an empty list; the caller decides what
/// an empty list means.
#[cfg(windows)]
pub fn render_endpoint_names() -> Vec<String> {
    std::thread::Builder::new()
        .name("rustamp-wasapi-enum".into())
        .spawn(|| {
            if wasapi::initialize_mta().is_err() {
                return None;
            }
            let enumerator = wasapi::DeviceEnumerator::new().ok()?;
            Some(crate::wasapi_out::endpoint_names(&enumerator))
        })
        .ok()
        .and_then(|probe| probe.join().ok())
        .flatten()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The devices a machine has, for the tests below.
    fn devices() -> Vec<String> {
        vec![
            "Speakers (Realtek)".to_owned(),
            "Headphones (USB)".to_owned(),
            "CABLE Output".to_owned(),
        ]
    }

    #[test]
    fn no_device_name_selects_the_system_default() {
        let plan = plan(None, OutputMode::Shared);
        assert_eq!(plan.device, DeviceTarget::Default);
    }

    #[test]
    fn a_device_name_selects_that_device() {
        let plan = plan(Some("CABLE Output"), OutputMode::Shared);
        assert_eq!(plan.device, DeviceTarget::Named("CABLE Output".to_owned()));
    }

    #[test]
    fn the_mode_is_carried_through() {
        for mode in [OutputMode::Shared, OutputMode::Exclusive] {
            assert_eq!(plan(None, mode).mode, mode);
            assert_eq!(plan(Some("Speakers"), mode).mode, mode);
            let resolved = resolve(&plan(Some("Speakers"), mode), &devices()).expect("resolves");
            assert_eq!(resolved.mode, mode, "the mode survives the lookup");
        }
    }

    #[test]
    fn a_name_that_matches_nothing_is_not_found() {
        let error = resolve(&plan(Some("CABLE Input"), OutputMode::Shared), &devices())
            .expect_err("no device is named like that");
        assert!(
            matches!(&error, DeviceError::NotFound { wanted, available }
                if wanted == "CABLE Input" && available.len() == 3),
            "{error:?}"
        );
        // The message names the wanted device and the ones that do exist.
        let message = error.to_string();
        assert!(message.contains("CABLE Input"), "{message}");
        assert!(message.contains("Speakers (Realtek)"), "{message}");
    }

    #[test]
    fn a_name_that_matches_several_devices_is_ambiguous() {
        let many = vec!["CABLE Input".to_owned(), "CABLE Output".to_owned()];
        let error = resolve(&plan(Some("CABLE"), OutputMode::Shared), &many)
            .expect_err("the name is not unique");
        assert!(
            matches!(&error, DeviceError::Ambiguous { matches, .. } if *matches == 2),
            "{error:?}"
        );
        let message = error.to_string();
        assert!(message.contains('2'), "{message}");
        assert!(message.contains("CABLE Input"), "{message}");
        assert!(message.contains("CABLE Output"), "{message}");
    }

    #[test]
    fn an_empty_name_is_an_error_and_matches_nothing() {
        let error = resolve(&plan(Some("   "), OutputMode::Shared), &devices())
            .expect_err("a blank name is not a device");
        assert_eq!(error, DeviceError::EmptyName);
        assert!(!matches("", "Speakers"), "a blank name matches no device");
        assert!(!matches("  ", "Speakers"), "a blank name matches no device");
    }

    #[test]
    fn the_matching_ignores_case_and_surrounding_space() {
        assert!(matches("speakers", "Speakers (Realtek)"));
        assert!(matches("SPEAKERS", "Speakers (Realtek)"));
        assert!(matches("  speakers  ", "Speakers (Realtek)"));
        assert!(matches("speakers", "  Speakers (Realtek)  "));
        assert!(!matches("headphone", "Speakers (Realtek)"));
    }

    #[test]
    fn the_resolved_plan_names_the_device_that_exists() {
        let resolved = resolve(&plan(Some("headphones"), OutputMode::Shared), &devices())
            .expect("the device exists");
        assert_eq!(
            resolved.device,
            DeviceTarget::Named("Headphones (USB)".to_owned()),
            "the plan names the device, not the configured text"
        );
    }

    #[test]
    fn the_default_target_needs_no_device_list() {
        // A machine without any endpoint still gets the default endpoint:
        // the list is only consulted for a named device.
        let resolved = resolve(&plan(None, OutputMode::Shared), &[]).expect("the default resolves");
        assert_eq!(resolved.device, DeviceTarget::Default);
    }

    #[test]
    fn an_empty_list_reports_that_no_device_is_available() {
        let error = resolve(&plan(Some("Speakers"), OutputMode::Shared), &[])
            .expect_err("there is no device at all");
        assert!(
            error.to_string().contains("no device is available"),
            "{error}"
        );
    }

    /// The enumeration runs on a thread of its own, so the caller's COM
    /// apartment stays free for the window — the same property the backend
    /// probe has (see the crate docs).
    #[cfg(windows)]
    #[test]
    fn the_enumeration_leaves_the_calling_thread_free_for_an_sta() {
        // The list may be empty on a machine without an endpoint; the
        // apartment is the property under test.
        let _names = render_endpoint_names();
        let entered = wasapi::initialize_sta();
        if entered.is_ok() {
            // Balance our own initialisation; the thread is left as found.
            wasapi::deinitialize();
        }
        assert!(
            entered.is_ok(),
            "the enumeration left this thread in the multi-threaded apartment \
             (CoInitializeEx(COINIT_APARTMENTTHREADED) returned {entered:?}); a window \
             created on it would panic in OleInitialize with RPC_E_CHANGED_MODE"
        );
    }
}
