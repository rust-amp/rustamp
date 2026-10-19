//! The host-death contract: how a dead plugin host is classified, and what
//! the supervisor does about it.
//!
//! # The contract
//!
//! The plugin host is a separate process (RFC 0006 §8), so its death never
//! takes the player down — but it does take the plugins down with it, and
//! without this contract the death would be *silent*: the supervisor would
//! notice a dead child, replace it and say nothing. This module is the one
//! place that says **how the host died** and **what happens next**:
//!
//! * [`classify_exit`] turns the host's exit code into a [`HostExit`].
//!   The host's own fault exit code ([`crate::fault::PLUGIN_FAULT_EXIT_CODE`])
//!   means "the host caught a plugin fault and said so on stderr"; a Windows
//!   exception status means "the host died from an exception" (a plugin that
//!   faulted is the likely cause, but the host does not claim it); any other
//!   non-zero code is an ordinary failure.
//! * [`RestartPolicy`] decides whether a fresh host starts, **after how long**
//!   (a growing backoff) and **what the report says**.
//! * [`CrashLedger`] counts the crashes that happened in a row, so the retry
//!   is bounded: a host that keeps dying is left down with a loud report
//!   instead of being respawned forever.
//!
//! # The policy (own decision, documented here)
//!
//! * **The player keeps playing.** Nothing in this module touches the audio
//!   path; a dead host costs the plugins, not the playback.
//! * **The plugins are dropped, not reloaded.** They died with the host
//!   process. The fresh host starts empty, and the app loads nothing by
//!   itself (it has no plugin registry yet), so a plugin that crashed is not
//!   loaded a second time behind the user's back.
//! * **The retry is bounded.** After [`DEFAULT_MAX_CONSECUTIVE_CRASHES`]
//!   crashes in a row the host is left down and the report says so.
//! * **The delay grows.** The first restart waits the supervisor's cooldown,
//!   each further consecutive crash doubles it, and
//!   [`DEFAULT_MAX_RESTART_DELAY`] caps it.
//! * **A host that ran long enough is not a crash loop.** A host that was up
//!   for longer than [`DEFAULT_CRASH_WINDOW`] counts as a fresh start: one
//!   crash after an hour of playing is an accident, not a loop.
//!
//! Licensed `MIT OR Apache-2.0` (RFC 0004).

use std::time::Duration;

use crate::fault::PLUGIN_FAULT_EXIT_CODE;

/// How many consecutive crashes are tolerated before the host is left down.
pub const DEFAULT_MAX_CONSECUTIVE_CRASHES: u32 = 3;

/// The upper bound for the growing restart delay.
pub const DEFAULT_MAX_RESTART_DELAY: Duration = Duration::from_secs(60);

/// How long a host must have been up to count as "not part of a crash loop".
pub const DEFAULT_CRASH_WINDOW: Duration = Duration::from_secs(60);

/// How the plugin host process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostExit {
    /// The host ended by itself with code 0.
    Clean,
    /// The host ended with a non-zero code that is not a Windows exception
    /// status (a channel error, for instance).
    Failed(i32),
    /// The host died from a Windows exception; the value is the NTSTATUS
    /// (`0xC0000005` is an access violation — the usual plugin fault).
    Crashed(u32),
    /// The host caught a plugin fault itself and ended with
    /// [`PLUGIN_FAULT_EXIT_CODE`]. The host's own stderr line names the
    /// plugin and the exception.
    PluginFault,
    /// The host was killed from outside and left no exit code.
    Killed,
}

/// Turn the host's exit code into a [`HostExit`].
///
/// `None` means the process left no exit code (killed from outside). The
/// Windows exception range (`0xC0000000..=0xC000FFFF`) is what an unhandled
/// exception looks like from the outside; the host's own fault exit code sits
/// outside that range, so the two are never confused.
pub fn classify_exit(code: Option<i32>) -> HostExit {
    let Some(code) = code else {
        return HostExit::Killed;
    };
    if code == 0 {
        return HostExit::Clean;
    }
    let raw = code as u32;
    if raw == PLUGIN_FAULT_EXIT_CODE {
        return HostExit::PluginFault;
    }
    if is_exception_status(raw) {
        return HostExit::Crashed(raw);
    }
    HostExit::Failed(code)
}

/// Is this value a Windows exception status?
fn is_exception_status(raw: u32) -> bool {
    (0xC000_0000..=0xC000_FFFF).contains(&raw)
}

/// How many crashes happened in a row.
///
/// The supervisor keeps one of these. A host that was up long enough is not
/// part of a crash loop, so its death starts the count again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CrashLedger {
    consecutive: u32,
}

impl CrashLedger {
    /// A ledger that has seen no crash.
    pub fn new() -> Self {
        Self::default()
    }

    /// How many crashes happened in a row.
    pub fn consecutive(&self) -> u32 {
        self.consecutive
    }

    /// Forget the history (a deliberate resume of the supervisor).
    pub fn reset(&mut self) {
        self.consecutive = 0;
    }

    /// Record one death and return the number of crashes in a row.
    ///
    /// `lived` is how long the dead host was up. A host that ran longer than
    /// `window` is not part of a crash loop, so the count starts again
    /// instead of growing towards the limit. A death that is **not** a crash
    /// (a clean exit, an ordinary failure code) ends the crash loop: the
    /// count returns to zero.
    pub fn record(&mut self, exit: HostExit, lived: Duration, window: Duration) -> u32 {
        if !matches!(exit, HostExit::Crashed(_) | HostExit::PluginFault) {
            self.consecutive = 0;
            return 0;
        }
        if lived >= window {
            self.consecutive = 0;
        }
        self.consecutive = self.consecutive.saturating_add(1);
        self.consecutive
    }
}

/// The policy the supervisor applies after the host died.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartPolicy {
    /// How many consecutive crashes are tolerated before the host is left
    /// down. The crash that reaches this number is the one that ends the
    /// retries.
    pub max_consecutive_crashes: u32,
    /// The delay before the first restart after a crash. The supervisor's own
    /// cooldown is the natural value: it is already "the minimum time between
    /// two spawn attempts".
    pub base_delay: Duration,
    /// The upper bound for the growing delay.
    pub max_delay: Duration,
    /// How long a host must have been up to count as "not part of a crash
    /// loop".
    pub crash_window: Duration,
}

impl RestartPolicy {
    /// A policy with the given limits.
    pub const fn new(
        max_consecutive_crashes: u32,
        base_delay: Duration,
        max_delay: Duration,
        crash_window: Duration,
    ) -> Self {
        Self {
            max_consecutive_crashes,
            base_delay,
            max_delay,
            crash_window,
        }
    }

    /// The delay before the restart that follows the `crash_number`-th
    /// consecutive crash (1-based).
    ///
    /// The delay doubles with every further consecutive crash and is capped
    /// at [`RestartPolicy::max_delay`].
    pub fn backoff(&self, crash_number: u32) -> Duration {
        if crash_number == 0 {
            return Duration::ZERO;
        }
        // The shift is capped so a large crash number cannot overflow the
        // shift itself; `saturating_mul` then caps the product.
        let factor = 1u32 << crash_number.saturating_sub(1).min(16);
        self.base_delay.saturating_mul(factor).min(self.max_delay)
    }

    /// Decide what to do after the host died.
    ///
    /// `consecutive_crashes` is the count the [`CrashLedger`] holds *after*
    /// this death was recorded. A crash that reaches
    /// [`RestartPolicy::max_consecutive_crashes`] ends the retries; every
    /// other death is followed by a restart after the policy's delay.
    pub fn decide(&self, exit: HostExit, consecutive_crashes: u32) -> RestartDecision {
        let crashed = matches!(exit, HostExit::Crashed(_) | HostExit::PluginFault);
        if crashed && consecutive_crashes >= self.max_consecutive_crashes {
            return RestartDecision::GiveUp {
                report: give_up_report(exit, consecutive_crashes),
            };
        }
        let after = if crashed {
            self.backoff(consecutive_crashes)
        } else {
            self.base_delay
        };
        RestartDecision::Restart {
            after,
            report: restart_report(
                exit,
                consecutive_crashes,
                after,
                self.max_consecutive_crashes,
            ),
        }
    }
}

/// What the supervisor must do after the host died.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartDecision {
    /// Start a fresh host after `after`. `report` says what happened and is
    /// meant to be shown to the user.
    Restart {
        /// How long to wait before the fresh host starts.
        after: Duration,
        /// The loud report about the death.
        report: String,
    },
    /// Do not start another host: the retry is bounded. `report` says why.
    GiveUp {
        /// The loud report about the death.
        report: String,
    },
}

/// The report for a death that is followed by a restart.
fn restart_report(
    exit: HostExit,
    consecutive_crashes: u32,
    after: Duration,
    max_consecutive_crashes: u32,
) -> String {
    match exit {
        HostExit::Crashed(code) => format!(
            "the plugin host crashed (Windows exception 0x{code:08X}); \
             the plugins it had loaded are gone and the player keeps playing. \
             A fresh host starts in {after:?} (crash {consecutive_crashes} of {max_consecutive_crashes})."
        ),
        HostExit::PluginFault => format!(
            "the plugin host reported a plugin fault and stopped; the host's stderr line \
             names the plugin and the exception. The plugins it had loaded are gone and the \
             player keeps playing. A fresh host starts in {after:?} \
             (crash {consecutive_crashes} of {max_consecutive_crashes})."
        ),
        HostExit::Clean => {
            "the plugin host exited on its own (code 0). A fresh host starts in {after:?}."
                .to_string()
        }
        HostExit::Failed(code) => {
            format!("the plugin host exited with code {code}. A fresh host starts in {after:?}.")
        }
        HostExit::Killed => {
            "the plugin host was killed from outside. A fresh host starts in {after:?}.".to_string()
        }
    }
}

/// The report for the death that ends the retries.
fn give_up_report(exit: HostExit, consecutive_crashes: u32) -> String {
    match exit {
        HostExit::Crashed(code) => format!(
            "the plugin host crashed {consecutive_crashes} times in a row \
             (Windows exception 0x{code:08X}); it is not started again, so the player runs \
             without plugins. The player keeps playing; restart the player to try again."
        ),
        HostExit::PluginFault => format!(
            "the plugin host reported a plugin fault {consecutive_crashes} times in a row; \
             it is not started again, so the player runs without plugins. The host's stderr \
             lines name the plugin and the exception. The player keeps playing; restart the \
             player to try again."
        ),
        // A non-crash death never reaches the give-up branch; the report is
        // still a fact, not a guess.
        other => format!(
            "the plugin host ended {consecutive_crashes} times in a row ({other:?}); \
             it is not started again, so the player runs without plugins."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> RestartPolicy {
        RestartPolicy::new(
            DEFAULT_MAX_CONSECUTIVE_CRASHES,
            Duration::from_secs(5),
            DEFAULT_MAX_RESTART_DELAY,
            DEFAULT_CRASH_WINDOW,
        )
    }

    #[test]
    fn a_clean_exit_is_not_a_crash() {
        assert_eq!(classify_exit(Some(0)), HostExit::Clean);
    }

    #[test]
    fn the_hosts_own_fault_code_is_recognised() {
        assert_eq!(
            classify_exit(Some(PLUGIN_FAULT_EXIT_CODE as i32)),
            HostExit::PluginFault
        );
    }

    #[test]
    fn a_windows_exception_status_is_a_crash() {
        // 0xC0000005 (access violation) as the signed exit code Windows
        // reports.
        assert_eq!(
            classify_exit(Some(0xC000_0005u32 as i32)),
            HostExit::Crashed(0xC000_0005)
        );
        assert_eq!(
            classify_exit(Some(0xC000_001Du32 as i32)),
            HostExit::Crashed(0xC000_001D)
        );
    }

    #[test]
    fn an_ordinary_failure_code_is_neither_a_crash_nor_a_fault() {
        assert_eq!(classify_exit(Some(1)), HostExit::Failed(1));
        assert_eq!(classify_exit(Some(42)), HostExit::Failed(42));
    }

    #[test]
    fn a_process_without_an_exit_code_was_killed() {
        assert_eq!(classify_exit(None), HostExit::Killed);
    }

    #[test]
    fn the_backoff_doubles_and_is_capped() {
        let policy = policy();
        assert_eq!(policy.backoff(1), Duration::from_secs(5));
        assert_eq!(policy.backoff(2), Duration::from_secs(10));
        assert_eq!(policy.backoff(3), Duration::from_secs(20));
        assert_eq!(policy.backoff(4), Duration::from_secs(40));
        // Capped at the policy's maximum.
        assert_eq!(policy.backoff(5), Duration::from_secs(60));
        assert_eq!(policy.backoff(50), Duration::from_secs(60));
        // No crash yet means no delay.
        assert_eq!(policy.backoff(0), Duration::ZERO);
    }

    #[test]
    fn a_crash_restarts_the_host_after_the_growing_delay() {
        let policy = policy();
        let decision = policy.decide(HostExit::Crashed(0xC000_0005), 1);
        let RestartDecision::Restart { after, report } = decision else {
            panic!("the first crash must be followed by a restart: {decision:?}");
        };
        assert_eq!(after, Duration::from_secs(5));
        assert!(
            report.contains("the plugin host crashed (Windows exception 0xC0000005)"),
            "the report must name the crash: {report}"
        );
        assert!(
            report.contains("player keeps playing"),
            "the report must say the player is unaffected: {report}"
        );
        assert!(
            report.contains("crash 1 of 3"),
            "the report must count the crashes: {report}"
        );

        // The second crash waits twice as long.
        let RestartDecision::Restart { after, .. } =
            policy.decide(HostExit::Crashed(0xC000_0005), 2)
        else {
            panic!("the second crash must be followed by a restart");
        };
        assert_eq!(after, Duration::from_secs(10));
    }

    #[test]
    fn the_third_crash_in_a_row_ends_the_retries_with_a_loud_report() {
        let policy = policy();
        let decision = policy.decide(HostExit::Crashed(0xC000_0005), 3);
        let RestartDecision::GiveUp { report } = decision else {
            panic!("the third crash must end the retries: {decision:?}");
        };
        assert!(
            report.contains("crashed 3 times in a row"),
            "the report must name the crash loop: {report}"
        );
        assert!(
            report.contains("0xC0000005"),
            "the report must name the exception: {report}"
        );
        assert!(
            report.contains("player keeps playing"),
            "the report must say the player is unaffected: {report}"
        );
    }

    #[test]
    fn a_plugin_fault_is_reported_as_a_fault_not_as_a_crash() {
        let policy = policy();
        let decision = policy.decide(HostExit::PluginFault, 1);
        let RestartDecision::Restart { after, report } = decision else {
            panic!("a plugin fault must be followed by a restart: {decision:?}");
        };
        assert_eq!(after, Duration::from_secs(5));
        assert!(
            report.contains("reported a plugin fault"),
            "the report must name the fault: {report}"
        );
        assert!(
            report.contains("stderr line names the plugin"),
            "the report must point at the host's own report: {report}"
        );
    }

    #[test]
    fn an_ordinary_death_is_not_counted_against_the_crash_limit() {
        let policy = policy();
        // Even with the crash count at the limit, a plain failure is followed
        // by a restart: only a crash ends the retries.
        let decision = policy.decide(HostExit::Failed(1), 3);
        let RestartDecision::Restart { after, report } = decision else {
            panic!("an ordinary failure must be followed by a restart: {decision:?}");
        };
        assert_eq!(after, Duration::from_secs(5));
        assert!(
            report.contains("exited with code 1"),
            "the report must name the exit code: {report}"
        );
    }

    #[test]
    fn the_ledger_counts_crashes_in_a_row() {
        let mut ledger = CrashLedger::new();
        assert_eq!(ledger.consecutive(), 0);
        let crash = HostExit::Crashed(0xC000_0005);
        assert_eq!(
            ledger.record(crash, Duration::from_secs(1), DEFAULT_CRASH_WINDOW),
            1
        );
        assert_eq!(
            ledger.record(crash, Duration::from_secs(1), DEFAULT_CRASH_WINDOW),
            2
        );
        assert_eq!(
            ledger.record(crash, Duration::from_secs(1), DEFAULT_CRASH_WINDOW),
            3
        );
    }

    #[test]
    fn a_host_that_ran_long_enough_starts_the_count_again() {
        let mut ledger = CrashLedger::new();
        let crash = HostExit::Crashed(0xC000_0005);
        assert_eq!(
            ledger.record(crash, Duration::from_secs(1), DEFAULT_CRASH_WINDOW),
            1
        );
        assert_eq!(
            ledger.record(crash, Duration::from_secs(1), DEFAULT_CRASH_WINDOW),
            2
        );
        // This host ran for longer than the window: it is not part of a crash
        // loop, so the count starts again.
        assert_eq!(
            ledger.record(crash, Duration::from_secs(120), DEFAULT_CRASH_WINDOW),
            1
        );
    }

    #[test]
    fn an_ordinary_death_ends_the_crash_loop() {
        let mut ledger = CrashLedger::new();
        let crash = HostExit::Crashed(0xC000_0005);
        assert_eq!(
            ledger.record(crash, Duration::from_secs(1), DEFAULT_CRASH_WINDOW),
            1
        );
        // A host that exited on its own is not a crash: the count is over.
        assert_eq!(
            ledger.record(
                HostExit::Clean,
                Duration::from_secs(1),
                DEFAULT_CRASH_WINDOW
            ),
            0
        );
        assert_eq!(ledger.consecutive(), 0);
    }

    #[test]
    fn a_resumed_supervisor_forgets_the_crashes() {
        let mut ledger = CrashLedger::new();
        let crash = HostExit::Crashed(0xC000_0005);
        assert_eq!(
            ledger.record(crash, Duration::from_secs(1), DEFAULT_CRASH_WINDOW),
            1
        );
        ledger.reset();
        assert_eq!(ledger.consecutive(), 0);
    }
}
