// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Property tests for the host-death contract.
//!
//! The contract is the layer that decides *when a dead host is replaced* and
//! *when the player gives up* on the host side: how an exit code is
//! classified ([`classify_exit`]), how many crashes happened in a row
//! ([`CrashLedger`]) and what the supervisor must do about it
//! ([`RestartPolicy::decide`]). Those are decisions about a sequence of
//! deaths, so they are pinned here as **properties over random sequences**
//! instead of as single hand-picked scenarios.
//!
//! The invariants:
//!
//! 1. **The ledger counts the crashes in a row.** A death that is not a
//!    crash ends the row, and so does a host that ran longer than the crash
//!    window: one crash after an hour of playing is an accident, not a loop.
//!    The count never exceeds the number of crashes that happened.
//! 2. **The policy gives up only on a crash that reaches the limit.** Every
//!    other death is followed by a restart.
//! 3. **The delay grows and is capped.** Each further consecutive crash
//!    doubles the delay, up to the policy's maximum.
//! 4. **The report matches the decision.** A restart says that a fresh host
//!    starts, a give-up says that the host is not started again — and
//!    neither report says the opposite.
//! 5. **The classification is total.** Every exit code has exactly one
//!    meaning, and the host's own fault code is never mistaken for a
//!    Windows exception status (nor the other way round).
//! 6. **No panic.** Any sequence — an empty one, a storm of deaths — must
//!    leave the policy decidable.
//!
//! Every sequence is driven by a printed seed, so a failing run is
//! replayable.

#[path = "support/rng.rs"]
mod rng;

use std::time::Duration;

use rustamp_plughost::PLUGIN_FAULT_EXIT_CODE;
use rustamp_plughost::restart::{
    CrashLedger, DEFAULT_CRASH_WINDOW, DEFAULT_MAX_CONSECUTIVE_CRASHES, DEFAULT_MAX_RESTART_DELAY,
    HostExit, RestartDecision, RestartPolicy, classify_exit,
};

use rng::Rng;

/// The seeds of the property runs. A failing run prints its seed, so the
/// sequence is replayable.
const SEEDS: [u64; 12] = [
    0x0000_0001,
    0x0000_00a3,
    0x0000_07ff,
    0x0000_ffff,
    0x0001_2345,
    0x00c0_ffee,
    0x0bad_c0de,
    0x5eed_0001,
    0x5eed_0002,
    0xdead_beef,
    0xface_b00c,
    0xffff_ffff,
];

/// The number of deaths in one random sequence.
const STEPS: usize = 32;

/// The number of deaths in the storm: a sequence that keeps coming must
/// never panic and never decide anything else.
const STORM: usize = 4_000;

/// The policy under test: the defaults the supervisor applies.
fn policy() -> RestartPolicy {
    RestartPolicy::new(
        DEFAULT_MAX_CONSECUTIVE_CRASHES,
        Duration::from_secs(5),
        DEFAULT_MAX_RESTART_DELAY,
        DEFAULT_CRASH_WINDOW,
    )
}

/// Is this death a crash the ledger counts?
fn is_crash(exit: HostExit) -> bool {
    matches!(exit, HostExit::Crashed(_) | HostExit::PluginFault)
}

/// One death of the host: how it ended and how long it was up.
#[derive(Clone, Copy, Debug)]
struct Death {
    exit: HostExit,
    lived: Duration,
}

/// A random death.
fn random_death(rng: &mut Rng) -> Death {
    let exit = match rng.below(6) {
        0 => HostExit::Clean,
        1 => HostExit::Failed(rng.below(64) as i32 + 1),
        2 => HostExit::Crashed(0xC000_0005 + rng.below(16) as u32),
        3 => HostExit::PluginFault,
        4 => HostExit::Killed,
        _ => HostExit::Crashed(0xC000_001D),
    };
    // How long the host was up: below the crash window, or beyond it.
    let lived = match rng.below(3) {
        0 => Duration::from_secs(0),
        1 => Duration::from_secs(1),
        _ => DEFAULT_CRASH_WINDOW + Duration::from_secs(1),
    };
    Death { exit, lived }
}

/// The count the ledger must hold after `deaths`.
///
/// The model is the policy in one place: a death that is not a crash ends
/// the row, a host that ran longer than the window starts it again, and
/// every crash adds one.
fn model_ledger(deaths: &[Death], window: Duration) -> u32 {
    let mut count = 0u32;
    for death in deaths {
        if !is_crash(death.exit) {
            count = 0;
            continue;
        }
        if death.lived >= window {
            count = 0;
        }
        count = count.saturating_add(1);
    }
    count
}

/// The ledger counts the crashes in a row, whatever the sequence is.
#[test]
fn the_ledger_counts_the_crashes_in_a_row() {
    let policy = policy();
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let mut ledger = CrashLedger::new();
        let mut deaths = Vec::new();
        let mut crashes = 0u32;
        for _ in 0..STEPS {
            let death = random_death(&mut rng);
            crashes += u32::from(is_crash(death.exit));
            deaths.push(death);
            let count = ledger.record(death.exit, death.lived, policy.crash_window);
            assert_eq!(
                count,
                model_ledger(&deaths, policy.crash_window),
                "seed {seed}: the ledger must count the crashes in a row"
            );
            assert_eq!(ledger.consecutive(), count, "seed {seed}: the count");
            assert!(
                count <= crashes,
                "seed {seed}: the ledger must never count more than the crashes that happened"
            );
        }
    }
}

/// A death that is not a crash ends the row, and a host that ran long
/// enough starts it again.
#[test]
fn a_death_that_is_not_a_crash_ends_the_row() {
    let policy = policy();
    let crash = HostExit::Crashed(0xC000_0005);
    let mut ledger = CrashLedger::new();
    assert_eq!(
        ledger.record(crash, Duration::from_secs(1), policy.crash_window),
        1
    );
    assert_eq!(
        ledger.record(crash, Duration::from_secs(1), policy.crash_window),
        2
    );
    // A host that exited on its own is not a crash: the count is over.
    assert_eq!(
        ledger.record(HostExit::Clean, Duration::from_secs(1), policy.crash_window),
        0
    );
    assert_eq!(ledger.consecutive(), 0);
    // A host that ran longer than the window is not part of a crash loop.
    assert_eq!(
        ledger.record(crash, Duration::from_secs(1), policy.crash_window),
        1
    );
    assert_eq!(
        ledger.record(
            crash,
            policy.crash_window + Duration::from_secs(1),
            policy.crash_window
        ),
        1,
        "a host that ran long enough starts the count again"
    );
}

/// The policy gives up only on a crash that reaches the limit, and every
/// other death is followed by a restart.
#[test]
fn the_policy_gives_up_only_on_a_crash_at_the_limit() {
    let policy = policy();
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for _ in 0..STEPS {
            let death = random_death(&mut rng);
            let count = rng.below(8) as u32;
            let decision = policy.decide(death.exit, count);
            let crashed = is_crash(death.exit);
            let gives_up = crashed && count >= policy.max_consecutive_crashes;
            match decision {
                RestartDecision::GiveUp { report } => {
                    assert!(
                        gives_up,
                        "seed {seed}: only a crash at the limit ends the retries: {death:?} {count}"
                    );
                    assert!(
                        report.contains("not started again"),
                        "seed {seed}: the give-up must say the host stays down: {report}"
                    );
                    assert!(
                        !report.contains("A fresh host starts"),
                        "seed {seed}: the give-up must not promise a restart: {report}"
                    );
                }
                RestartDecision::Restart { after, report } => {
                    assert!(
                        !gives_up,
                        "seed {seed}: a crash at the limit must end the retries: {death:?} {count}"
                    );
                    let expected = if crashed {
                        policy.backoff(count)
                    } else {
                        policy.base_delay
                    };
                    assert_eq!(
                        after, expected,
                        "seed {seed}: the delay must be the policy's"
                    );
                    assert!(
                        report.contains("A fresh host starts"),
                        "seed {seed}: the restart must say a host starts: {report}"
                    );
                    assert!(
                        !report.contains("not started again"),
                        "seed {seed}: the restart must not give up: {report}"
                    );
                }
            }
        }
    }
}

/// The delay grows with every further consecutive crash and is capped.
#[test]
fn the_delay_grows_and_is_capped() {
    let policy = policy();
    assert_eq!(policy.backoff(0), Duration::ZERO, "no crash means no delay");
    assert_eq!(policy.backoff(1), policy.base_delay);
    let mut previous = Duration::ZERO;
    for crash in 1..=64u32 {
        let delay = policy.backoff(crash);
        assert!(
            delay >= previous,
            "the delay must not shrink: crash {crash} waits {delay:?} after {previous:?}"
        );
        assert!(
            delay <= policy.max_delay,
            "the delay must be capped: crash {crash} waits {delay:?}"
        );
        previous = delay;
    }
    // The delay doubles until the cap.
    for crash in 1..=8u32 {
        let expected = policy
            .base_delay
            .saturating_mul(1u32 << (crash - 1))
            .min(policy.max_delay);
        assert_eq!(policy.backoff(crash), expected, "crash {crash}");
    }
}

/// Every exit code has exactly one meaning, and the host's own fault code
/// is never mistaken for a Windows exception status.
#[test]
fn the_classification_is_total_and_never_confuses_the_fault_code() {
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        for _ in 0..64 {
            let code = match rng.below(6) {
                0 => 0,
                1 => PLUGIN_FAULT_EXIT_CODE as i32,
                2 => 0xC000_0005u32 as i32,
                3 => 0xC000_0000u32 as i32 + rng.below(0x1_0000) as i32,
                4 => rng.below(0x7FFF_FFFF) as i32,
                _ => -(rng.below(0x7FFF_FFFF) as i32),
            };
            let raw = code as u32;
            let exit = classify_exit(Some(code));
            if raw == PLUGIN_FAULT_EXIT_CODE {
                assert_eq!(exit, HostExit::PluginFault, "seed {seed}: the fault code");
            } else if code == 0 {
                assert_eq!(exit, HostExit::Clean, "seed {seed}: the clean exit");
            } else if (0xC000_0000..=0xC000_FFFF).contains(&raw) {
                assert_eq!(exit, HostExit::Crashed(raw), "seed {seed}: the exception");
            } else {
                assert_eq!(exit, HostExit::Failed(code), "seed {seed}: the failure");
            }
        }
        // A process without an exit code was killed from outside.
        assert_eq!(classify_exit(None), HostExit::Killed, "seed {seed}");
    }
    // The host's own fault code sits outside the exception range, so the two
    // are never confused.
    assert!(
        !(0xC000_0000..=0xC000_FFFF).contains(&PLUGIN_FAULT_EXIT_CODE),
        "the fault code must not look like an exception status"
    );
}

/// No panic, whatever the sequence: an empty one and a storm of deaths.
#[test]
fn no_sequence_of_deaths_panics() {
    let policy = policy();
    // An empty sequence: the ledger has seen nothing.
    let ledger = CrashLedger::new();
    assert_eq!(ledger.consecutive(), 0);
    assert!(matches!(
        policy.decide(HostExit::Clean, 0),
        RestartDecision::Restart { .. }
    ));

    // A storm: the deaths keep coming and every one of them is decided.
    for seed in SEEDS {
        let mut rng = Rng::new(seed);
        let mut ledger = CrashLedger::new();
        for _ in 0..STORM {
            let death = random_death(&mut rng);
            let count = ledger.record(death.exit, death.lived, policy.crash_window);
            let _ = policy.decide(death.exit, count);
        }
    }
}
