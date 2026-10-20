// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Private HWND readiness marker shared with the cross-process regression test.
//! A version reply alone can be delivered reentrantly during startup. This
//! marker means the handler is installed AND the first full UI tick completed.
//! It is not a classic IPC capability or a promise that later sends cannot hang.

pub(crate) const READY_PROPERTY: &str = "Rustamp.ExternalControl.Ready";
