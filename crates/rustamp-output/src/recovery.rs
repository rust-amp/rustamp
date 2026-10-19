// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! What a fatal output failure means, and what the player does about it.
//!
//! RFC 0012 gives the engine one way to learn that an output device died:
//! [`OutputDevice::stream_error`](rustamp_core::audio::OutputDevice::stream_error)
//! hands out an [`Error`](rustamp_core::error::Error). That is deliberately a
//! single, simple surface — but an error *string* alone cannot say whether the
//! player can carry on. This module gives it a meaning in two pure steps:
//!
//! 1. the backend classifies what happened ([`DeviceFault`]);
//! 2. the classification and the way the device was selected
//!    ([`OutputSelection`]) decide what happens to the run ([`Recovery`]).
//!
//! Both steps are pure: no thread is spawned, no device is touched, no state
//! is read. The whole decision table is therefore testable on any machine
//! without an audio device — which is the only way to test a recovery whose
//! real trigger (a device that is unplugged) needs hardware.
//!
//! # The decision table
//!
//! | fault | selection | recovery |
//! |---|---|---|
//! | none | any | [`Recovery::Nothing`] |
//! | [`DeviceFault::Invalidated`] | [`OutputSelection::Default`] | [`Recovery::KeepOnDefault`] |
//! | [`DeviceFault::Invalidated`] | [`OutputSelection::Pinned`] | [`Recovery::Stop`] |
//! | [`DeviceFault::Lost`] | [`OutputSelection::Default`] | [`Recovery::KeepOnDefault`] |
//! | [`DeviceFault::Lost`] | [`OutputSelection::Pinned`] | [`Recovery::Stop`] |
//! | [`DeviceFault::WriteFailed`] | any | [`Recovery::Stop`] |
//!
//! # What the decision does *not* do
//!
//! It never reconnects by itself. RFC 0012 §4.3 and its open question Q1
//! decide that a lost device is a fail-to-stop: the run ends, the error is
//! reported, and the *next explicit start* (the user pressing Play) gives the
//! output a fresh `close`/`open`/`start` lifecycle. An automatic retry would
//! thrash the audio device and could play through an unwanted fallback
//! (laptop speakers) without the user ever asking for it.
//!
//! What the decision adds is the *knowledge* of what that next start will do:
//! a stream that follows the system default lands on the device that is the
//! default *now* (the recovery the user wants after a default change), while a
//! stream bound to a named device can only fail until that device is back. The
//! decision says so in the message the user reads, instead of the one flat
//! "the device disappeared" that every failure used to produce.

/// Why an output stream died.
///
/// The classification is what the backend knows about the failure; it does not
/// depend on the message that is reported to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceFault {
    /// The audio client was invalidated: the endpoint was removed, the system
    /// default endpoint changed, or the endpoint renegotiated its mix format.
    ///
    /// Windows reports this as `AUDCLNT_E_DEVICE_INVALIDATED`. The *client* is
    /// invalid for good — it needs a new `open` — but the *device* may be
    /// perfectly alive (only the default changed) or gone (it was unplugged).
    /// [`decide`] tells the two apart by the selection.
    Invalidated,
    /// The device is gone or stopped responding.
    Lost,
    /// Writing to the device failed.
    WriteFailed,
}

/// How the output device was selected.
///
/// The selection decides where a recovery can land: only a stream that
/// follows the system default can continue on a *different* device, because
/// that is the only case in which the user asked for "whatever the system
/// plays through" rather than for one named device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputSelection {
    /// The stream follows the system default endpoint.
    Default,
    /// The stream is bound to the device the caller named.
    Pinned,
}

/// What the player does about a latched fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// Nothing to do: the output is healthy.
    Nothing,
    /// Stop the run and report the error. The next explicit start tries the
    /// same device again; while the device is gone, that attempt fails and
    /// says so.
    Stop,
    /// Stop the run and report the error. The next explicit start continues on
    /// the device that is the system default *now*.
    KeepOnDefault,
}

/// The recovery for a latched fault: pure, and the whole policy in one place.
///
/// `fault` is the latched failure (`None` for a healthy output), `selection`
/// is how the device was chosen. The rules:
///
/// * A healthy output needs no recovery.
/// * A device that was **invalidated** (removed, or the default endpoint
///   changed) or that is **gone** leaves a stream that follows the system
///   default able to continue on the current default: the user asked for the
///   default, and the default is what plays now.
/// * The same failure on a **pinned** device cannot be recovered by playing
///   somewhere else — the user named that device, and the backend never
///   substitutes another one behind their back.
/// * A **write failure** is not a device change: the device is the same one it
///   was, so there is no other device to continue on.
pub const fn decide(fault: Option<DeviceFault>, selection: OutputSelection) -> Recovery {
    match fault {
        None => Recovery::Nothing,
        Some(DeviceFault::WriteFailed) => Recovery::Stop,
        Some(DeviceFault::Invalidated | DeviceFault::Lost) => match selection {
            OutputSelection::Default => Recovery::KeepOnDefault,
            OutputSelection::Pinned => Recovery::Stop,
        },
    }
}

/// The text that names the fault itself.
pub(crate) const fn fault_text(fault: DeviceFault) -> &'static str {
    match fault {
        DeviceFault::Invalidated => {
            "the endpoint was invalidated: the device was removed, the default device changed, \
             or the endpoint renegotiated its format"
        }
        DeviceFault::Lost => "the device disappeared or stopped responding",
        DeviceFault::WriteFailed => "writing to the device failed",
    }
}

/// The text that names the recovery the decision describes.
const fn recovery_text(fault: DeviceFault, selection: OutputSelection) -> &'static str {
    match decide(Some(fault), selection) {
        Recovery::KeepOnDefault => "press Play to continue on the current default device",
        Recovery::Stop => match selection {
            OutputSelection::Pinned => "press Play to try it again, or choose another device",
            OutputSelection::Default => "press Play to try it again",
        },
        // `decide` never answers `Nothing` for a fault; the arm keeps the
        // function total.
        Recovery::Nothing => "the output is healthy",
    }
}

/// The message a backend reports for a latched fault: the device, what
/// happened, and what the recovery is.
///
/// The recovery text is the part that matters to the user: it says what
/// pressing Play will do, instead of leaving "the device disappeared" as the
/// only answer.
pub fn message(fault: DeviceFault, selection: OutputSelection, device: &str) -> String {
    format!(
        "{device}: {}; {}",
        fault_text(fault),
        recovery_text(fault, selection)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The decision table, row by row. It is the whole policy, so it is
    /// pinned here rather than only implied by the message texts.
    #[test]
    fn a_healthy_output_needs_no_recovery() {
        assert_eq!(decide(None, OutputSelection::Default), Recovery::Nothing);
        assert_eq!(decide(None, OutputSelection::Pinned), Recovery::Nothing);
    }

    #[test]
    fn an_invalidated_default_device_continues_on_the_new_default() {
        // The default endpoint changed: the stream follows the default, so the
        // next explicit start plays through the device that is the default now.
        assert_eq!(
            decide(Some(DeviceFault::Invalidated), OutputSelection::Default),
            Recovery::KeepOnDefault
        );
    }

    #[test]
    fn a_lost_default_device_continues_on_the_current_default() {
        // The device was unplugged: the stream follows the default, so the next
        // explicit start plays through the device that is the default now.
        assert_eq!(
            decide(Some(DeviceFault::Lost), OutputSelection::Default),
            Recovery::KeepOnDefault
        );
    }

    #[test]
    fn a_pinned_device_that_is_gone_cannot_be_replaced_silently() {
        // The user named the device; the backend never substitutes another one
        // behind their back, so there is no other device to continue on.
        assert_eq!(
            decide(Some(DeviceFault::Lost), OutputSelection::Pinned),
            Recovery::Stop
        );
        assert_eq!(
            decide(Some(DeviceFault::Invalidated), OutputSelection::Pinned),
            Recovery::Stop
        );
    }

    #[test]
    fn a_write_failure_is_not_a_device_change() {
        // The device is the same one it was, so there is no other device to
        // continue on — the run stops and the next start tries it again.
        assert_eq!(
            decide(Some(DeviceFault::WriteFailed), OutputSelection::Default),
            Recovery::Stop
        );
        assert_eq!(
            decide(Some(DeviceFault::WriteFailed), OutputSelection::Pinned),
            Recovery::Stop
        );
    }

    #[test]
    fn the_message_names_the_device_the_fault_and_the_recovery() {
        let msg = message(
            DeviceFault::Invalidated,
            OutputSelection::Default,
            "Speakers (Realtek)",
        );
        assert!(msg.starts_with("Speakers (Realtek): "), "{msg}");
        assert!(msg.contains("invalidated"), "{msg}");
        assert!(
            msg.contains("press Play to continue on the current default device"),
            "{msg}"
        );
    }

    #[test]
    fn the_message_of_a_pinned_device_offers_another_device() {
        let msg = message(DeviceFault::Lost, OutputSelection::Pinned, "CABLE Input");
        assert!(msg.contains("CABLE Input"), "{msg}");
        assert!(msg.contains("disappeared"), "{msg}");
        assert!(msg.contains("choose another device"), "{msg}");
        assert!(
            !msg.contains("current default device"),
            "a pinned device never continues on the default: {msg}"
        );
    }

    #[test]
    fn the_message_of_a_write_failure_offers_a_retry() {
        let msg = message(
            DeviceFault::WriteFailed,
            OutputSelection::Default,
            "Speakers",
        );
        assert!(msg.contains("writing to the device failed"), "{msg}");
        assert!(msg.contains("press Play to try it again"), "{msg}");
    }
}
