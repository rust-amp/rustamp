// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! The fade report the user sees: once per configuration.
//!
//! # What the engine reports
//!
//! The engine compares every requested fade with the two buffers the fade is
//! realised in — the device's own buffer and the engine's ring — and reports
//! the conflict when the fade does not fit the shorter of the two
//! (`rustamp_audio::fade_budget`; S18 §2.3's constraint). The report carries
//! the numbers: the fade that was asked for, the buffer that is too short
//! and the buffer length that would hold the whole fade.
//!
//! # Why the app decides when the report is shown
//!
//! The report fires on **every** fade-out: with the shipped configuration a
//! 500 ms fade does not fit the device's 23 ms buffer, so every fade-out
//! produces the very same report. The user has to read it once, not once per
//! fade-out.
//!
//! The app therefore shows the report **once per configuration**: the buffer
//! the device granted, the engine's ring and the fade that was requested.
//! When one of the three numbers changes, the configuration is a new one and
//! the report is shown again.
//!
//! # What the app does not do
//!
//! The wording is the engine's own. The app decides *when* the user sees the
//! report, never *what* it says: the sentence the engine wrote is the
//! sentence the user reads.

/// The configuration a fade report is about.
///
/// The three numbers are the ones the conflict is made of: the buffer the
/// device granted, the engine's ring and the fade that was requested. They
/// are counted in milliseconds — the unit the report and the reference's own
/// constraint sentence use — so two configurations that read the same to the
/// user are the same configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FadeConfiguration {
    /// The device's own buffer, in milliseconds.
    pub(crate) device_buffer_ms: u64,
    /// The engine's ring, in milliseconds.
    pub(crate) ring_ms: u64,
    /// The fade that was requested, in milliseconds.
    pub(crate) fade_ms: u32,
}

/// The fade reports the user has already seen.
///
/// The app keeps the configuration of the report it showed last; a report
/// about that same configuration is dropped, a report about any other one is
/// shown.
#[derive(Debug, Default)]
pub(crate) struct SeenFadeReports {
    /// The configuration of the report that was shown last.
    last: Option<FadeConfiguration>,
}

impl SeenFadeReports {
    /// Whether the report about `configuration` has to be shown.
    ///
    /// The first report is always new. A report about the configuration that
    /// was shown last is not: it says the same thing about the same
    /// buffers, so it is dropped. A report about any other configuration is
    /// new — a buffer or the fade changed, so the user has to read it.
    ///
    /// The configuration that was admitted is remembered, so the next report
    /// about it is dropped as well.
    pub(crate) fn should_show(&mut self, configuration: FadeConfiguration) -> bool {
        if self.last == Some(configuration) {
            return false;
        }
        self.last = Some(configuration);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped configuration: a 23 ms device buffer, a 371 ms ring and a
    /// 500 ms fade.
    fn shipped() -> FadeConfiguration {
        FadeConfiguration {
            device_buffer_ms: 23,
            ring_ms: 371,
            fade_ms: 500,
        }
    }

    #[test]
    fn the_first_report_is_new() {
        let mut seen = SeenFadeReports::default();
        assert!(seen.should_show(shipped()));
    }

    #[test]
    fn the_same_configuration_is_not_new_again() {
        let mut seen = SeenFadeReports::default();
        assert!(seen.should_show(shipped()));
        assert!(
            !seen.should_show(shipped()),
            "the same configuration is not shown twice"
        );
    }

    #[test]
    fn a_changed_buffer_arms_the_report_again() {
        let mut seen = SeenFadeReports::default();
        assert!(seen.should_show(shipped()));
        let other_device = FadeConfiguration {
            device_buffer_ms: 400,
            ..shipped()
        };
        assert!(
            seen.should_show(other_device),
            "a device that granted another buffer is a new report"
        );
        let other_ring = FadeConfiguration {
            ring_ms: 10,
            ..shipped()
        };
        assert!(seen.should_show(other_ring), "another ring is a new report");
    }

    #[test]
    fn a_changed_fade_arms_the_report_again() {
        let mut seen = SeenFadeReports::default();
        assert!(seen.should_show(shipped()));
        let shorter_fade = FadeConfiguration {
            fade_ms: 200,
            ..shipped()
        };
        assert!(
            seen.should_show(shorter_fade),
            "another fade is a new report"
        );
    }

    #[test]
    fn a_report_about_another_cause_is_not_suppressed() {
        // The ring binds instead of the device: the cause of the conflict
        // differs, so the report is new even though the fade is the same.
        let mut seen = SeenFadeReports::default();
        assert!(seen.should_show(shipped()));
        let ring_binds = FadeConfiguration {
            device_buffer_ms: 400,
            ring_ms: 371,
            fade_ms: 500,
        };
        assert!(
            seen.should_show(ring_binds),
            "another cause is not suppressed by the dedupe"
        );
    }
}
