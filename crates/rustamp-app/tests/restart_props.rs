// Rustamp is licensed GPL-3.0-or-later with an additional permission; see PLUGIN-EXCEPTION.md.
//! Property tests for the plugin host's restart policy.
//!
//! The supervisor ([`rustamp_app::supervise::Supervisor`]) is the layer that
//! decides *when a dead resident is replaced* and *when the player gives up*:
//! it counts the failures that happened in a row, it holds the cooldown
//! between two spawn attempts, it reports a host that died, and it stops
//! trying once the policy says the retries are over. Those are decisions
//! about a sequence of events, so they are pinned here as **properties over
//! random event sequences** instead of as single hand-picked scenarios.
//!
//! The events a sequence may contain:
//!
//! * **start ok** — the host starts and greets: the supervision succeeded.
//! * **crash** — the running host dies from a Windows exception.
//! * **fault** — the running host caught a plugin fault and ended with the
//!   host's own exit code (see `rustamp_plughost::restart`).
//! * **quit** — the running host exited on its own.
//! * **hang** — the running host stops answering: the missed-`Pong` failure
//!   detaches the link and the host is replaced.
//! * **handshake reject** — the host greets with a foreign protocol version.
//! * **user stop** — the user stops the supervision.
//!
//! The invariants:
//!
//! 1. **The counter is consecutive.** A success resets it, a failure
//!    increments it, and only `MAX_CONSECUTIVE_FAILURES` failures *in a row*
//!    end the retries. A host that came up resets the count, so a player
//!    that works is never given up on.
//! 2. **The cooldown bounds the retries.** Within a cooldown window no
//!    restart is attempted, whatever the events.
//! 3. **A user stop is never a failure.** It moves neither the counter nor
//!    the crash ledger, and it never reaches the give-up.
//! 4. **The terminal state is sticky.** Once the retries are over, no event
//!    starts a host again — the only way out is the explicit user action
//!    ([`Supervisor::resume`], the API behind "restart the player").
//! 5. **The report matches the decision.** A give-up is reported, and a
//!    report never promises a restart the supervisor will not make.
//! 6. **No panic.** Any sequence — an empty one, a storm of events, a
//!    script that runs out — must leave the player running.
//!
//! Every sequence is driven by a printed seed, so a failing run is
//! replayable. The supervisor runs against a scripted backend: no process is
//! ever spawned, and the handshake, the crash, the hang and the refusal are
//! all produced in process.

#[path = "support/rng.rs"]
mod rng;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io;
use std::rc::Rc;
use std::time::{Duration, Instant};

use rustamp_app::host::{HostStateLink, PING_INTERVAL, PONG_TIMEOUT, PollOutcome};
use rustamp_app::supervise::{
    DEFAULT_MAX_CONSECUTIVE_FAILURES, HostBackend, HostProcess, Supervisor, SupervisorState,
};
use rustamp_ipc::{MemoryTransport, Message, PROTOCOL_VERSION, Transport};
use rustamp_plughost::PLUGIN_FAULT_EXIT_CODE;
use rustamp_plughost::restart::DEFAULT_MAX_CONSECUTIVE_CRASHES;

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

/// The number of events in one random sequence.
const STEPS: usize = 40;

/// The number of events in the storm: a sequence that keeps coming must
/// never panic and never start a host behind the user's back.
const STORM: usize = 2_000;

/// The access violation a crashing host dies from (`0xC0000005`).
const CRASH_CODE: i32 = 0xC000_0005u32 as i32;

/// The exit code the host uses when it caught a plugin fault.
const FAULT_CODE: i32 = PLUGIN_FAULT_EXIT_CODE as i32;

/// The cooldown of the property runs.
///
/// Zero, so the supervisor is a pure function of the event sequence: the
/// cooldown and the startup barrier never wait for the wall clock. The
/// cooldown itself is tested separately, with a window no test can leave.
const COOLDOWN: Duration = Duration::ZERO;

/// The startup barrier of the property runs: a host that does not greet is
/// a failure on the very next step, so a handshake timeout is a step of the
/// sequence and not a wait.
const HANDSHAKE_TIMEOUT: Duration = Duration::ZERO;

/// The startup barrier of the test that watches a *slow* host: the greeting
/// must arrive within this window, and the test greets on the very next
/// step, so the window is never in the way.
const SLOW_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// What the scripted host does when the supervisor spawns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Spawn {
    /// A live host that greets with our protocol version: the host is up.
    Hello,
    /// A live host that greets with a foreign protocol version: the
    /// handshake is rejected and the host is replaced.
    Foreign,
    /// A live host that never greets: the startup barrier times out.
    Silent,
    /// A live host that greets only when the test tells it to: a host that
    /// takes its time to start.
    Late,
    /// A host that is already dead when the supervisor spawns it.
    Dead,
    /// The spawn fails: no binary, no stdio, no host at all.
    Refused,
}

/// A host the supervisor spawned: the test acts on it to simulate a death.
#[derive(Clone)]
struct HostHandle {
    alive: Rc<Cell<bool>>,
    exit_code: Rc<Cell<i32>>,
}

/// What the next spawn attempt produces, with the host it will start.
///
/// The host handle is created when the outcome is queued, so the test and
/// the model talk about the same host even before it is spawned.
#[derive(Clone)]
struct Scripted {
    outcome: Spawn,
    handle: HostHandle,
}

/// The scripted backend: no process is ever spawned.
struct ScriptedBackend {
    /// The outcomes the next spawn attempts produce, in order.
    script: Rc<RefCell<VecDeque<Scripted>>>,
    /// The hosts that were spawned, so the test can act on the running one.
    hosts: Rc<RefCell<Vec<HostHandle>>>,
    /// The host ends of the control channels. They are kept alive, so a
    /// host that says nothing reads as silence and not as a closed pipe.
    peers: Vec<MemoryTransport>,
    /// The host ends of the hosts that greet late: the test greets them
    /// through these.
    greeters: Rc<RefCell<Vec<MemoryTransport>>>,
}

/// The liveness of a scripted host.
struct ScriptedProcess {
    alive: Rc<Cell<bool>>,
    exit_code: Rc<Cell<i32>>,
}

impl HostProcess for ScriptedProcess {
    fn is_running(&mut self) -> bool {
        self.alive.get()
    }

    fn exit_code(&mut self) -> Option<i32> {
        if self.alive.get() {
            None
        } else {
            Some(self.exit_code.get())
        }
    }
}

impl HostBackend for ScriptedBackend {
    type Link = MemoryTransport;
    type Process = ScriptedProcess;

    fn spawn(&mut self) -> io::Result<(MemoryTransport, ScriptedProcess)> {
        let Some(scripted) = self.script.borrow_mut().pop_front() else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "the script is exhausted: no host to start",
            ));
        };
        if scripted.outcome == Spawn::Refused {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no host binary"));
        }
        let (app_end, mut peer) = MemoryTransport::pair();
        match scripted.outcome {
            Spawn::Hello => peer.send(&Message::hello()).expect("the host greets"),
            Spawn::Foreign => peer
                .send(&Message::Hello {
                    protocol_version: PROTOCOL_VERSION + 1,
                    pid: 4242,
                })
                .expect("the host greets"),
            Spawn::Silent => {}
            Spawn::Late => {}
            Spawn::Dead => scripted.handle.alive.set(false),
            Spawn::Refused => unreachable!("the refusal is handled above"),
        }
        match scripted.outcome {
            // The host greets when the test says so: the test reaches it
            // through the greeters.
            Spawn::Late => self.greeters.borrow_mut().push(peer),
            // Every other host keeps its end of the channel alive, so a host
            // that says nothing reads as silence and not as a closed pipe.
            _ => self.peers.push(peer),
        }
        self.hosts.borrow_mut().push(scripted.handle.clone());
        Ok((
            app_end,
            ScriptedProcess {
                alive: scripted.handle.alive,
                exit_code: scripted.handle.exit_code,
            },
        ))
    }
}

/// One event of a random sequence.
#[derive(Clone, Copy, Debug)]
enum Event {
    /// Nothing changes; the supervisor just gets a step.
    Idle,
    /// The next spawn attempt produces this outcome.
    Spawn(Spawn),
    /// The running host dies from a Windows exception.
    Crash,
    /// The running host caught a plugin fault and ended with its own code.
    Fault,
    /// The running host exited on its own.
    Quit,
    /// The running host stops answering: the link detaches.
    Hang,
    /// The user stops the supervision.
    UserStop,
    /// The user resumes the supervision.
    Resume,
}

/// The next event of a random sequence.
///
/// The choice respects the model: a stopped or failed supervisor only wakes
/// up through the explicit user action, so the generator takes it (and
/// sometimes does not, to watch the terminal state stick).
fn choose_event(rng: &mut Rng, state: SupervisorState) -> Event {
    match state {
        SupervisorState::Running => match rng.below(12) {
            0..=3 => Event::Spawn(random_spawn(rng)),
            4 => Event::Crash,
            5 => Event::Fault,
            6 => Event::Quit,
            7 => Event::Hang,
            8 => Event::UserStop,
            9 => Event::Idle,
            _ => Event::Spawn(random_spawn(rng)),
        },
        SupervisorState::Stopped | SupervisorState::PersistentFailure => match rng.below(4) {
            0 => Event::Idle,
            _ => Event::Resume,
        },
    }
}

/// A random outcome for the next spawn attempt.
fn random_spawn(rng: &mut Rng) -> Spawn {
    match rng.below(6) {
        0 => Spawn::Hello,
        1 => Spawn::Foreign,
        2 => Spawn::Silent,
        3 => Spawn::Dead,
        4 => Spawn::Late,
        _ => Spawn::Refused,
    }
}

/// Is this exit code a crash the crash ledger counts?
///
/// The rule is the one the host-death contract states: a Windows exception
/// status and the host's own fault code are crashes, every other death is
/// not.
fn is_crash(code: i32) -> bool {
    let raw = code as u32;
    raw == PLUGIN_FAULT_EXIT_CODE || (0xC000_0000..=0xC000_FFFF).contains(&raw)
}

/// What the model expects the supervisor to do.
///
/// The model is written from the policy, not from the code: a success
/// resets the counter, a failure increments it, a crash counts towards the
/// crash ledger, a death that is not a crash ends the crash loop, and the
/// give-up is the union of the two limits. The property then checks the
/// supervisor against it.
struct Model {
    /// The outcomes the supervisor has not spent yet.
    script: Rc<RefCell<VecDeque<Scripted>>>,
    /// How long the startup barrier waits for the greeting.
    handshake_timeout: Duration,
    /// The consecutive failure counter the supervisor must hold.
    failures: u32,
    /// The consecutive crash count the crash ledger must hold.
    crashes: u32,
    /// The state the supervisor must be in.
    state: SupervisorState,
    /// The host the supervisor holds.
    attached: Option<HostHandle>,
    /// Whether the link has a transport.
    link_attached: bool,
    /// The host that waits for its startup greeting.
    pending: Option<HostHandle>,
    /// How many spawn attempts the model expects.
    spawns: u64,
    /// How many outcomes the script holds.
    queued: usize,
    /// What the sequence reached.
    coverage: Coverage,
}

impl Model {
    /// A model for a fresh supervisor.
    fn new(script: Rc<RefCell<VecDeque<Scripted>>>, handshake_timeout: Duration) -> Self {
        Model {
            script,
            handshake_timeout,
            failures: 0,
            crashes: 0,
            state: SupervisorState::Running,
            attached: None,
            link_attached: false,
            pending: None,
            spawns: 0,
            queued: 0,
            coverage: Coverage::default(),
        }
    }

    /// The event changed the world, before the supervisor's step.
    fn apply(&mut self, event: Event, link: &mut HostStateLink) {
        match event {
            Event::Idle => {}
            Event::Spawn(outcome) => {
                let handle = HostHandle {
                    alive: Rc::new(Cell::new(true)),
                    exit_code: Rc::new(Cell::new(0)),
                };
                self.script
                    .borrow_mut()
                    .push_back(Scripted { outcome, handle });
                self.queued += 1;
            }
            Event::Crash => self.kill_attached(CRASH_CODE),
            Event::Fault => self.kill_attached(FAULT_CODE),
            Event::Quit => self.kill_attached(0),
            Event::Hang => self.hang(link),
            Event::UserStop => {
                // The deliberate off-switch: the host is killed, the link is
                // detached, and neither the counter nor the ledger moves.
                self.state = SupervisorState::Stopped;
                self.attached = None;
                self.link_attached = false;
                self.pending = None;
            }
            Event::Resume => {
                // The explicit user action: a fresh start, not a
                // continuation of the crash loop.
                self.state = SupervisorState::Running;
                self.failures = 0;
                self.crashes = 0;
                self.pending = None;
            }
        }
    }

    /// The running host dies with `code`.
    fn kill_attached(&mut self, code: i32) {
        if let Some(handle) = &self.attached {
            handle.alive.set(false);
            handle.exit_code.set(code);
        }
    }

    /// The running host stops answering: the missed-`Pong` failure detaches
    /// the link, exactly as the liveness path does it.
    fn hang(&mut self, link: &mut HostStateLink) {
        if self.attached.is_none() {
            return;
        }
        let now = Instant::now();
        link.poll_at(now);
        link.poll_at(now + PING_INTERVAL);
        assert_eq!(
            link.poll_at(now + PING_INTERVAL + PONG_TIMEOUT),
            PollOutcome::MissedPong,
            "a host that stops answering must be detected"
        );
        self.link_attached = false;
    }

    /// What the supervisor's step must do.
    fn step(&mut self) {
        // A stopped or failed supervisor does nothing at all.
        if self.state != SupervisorState::Running {
            return;
        }
        // The host died: the crash policy counts the crash, the failure
        // counter counts the failure.
        if let Some(handle) = &self.attached
            && !handle.alive.get()
        {
            let code = handle.exit_code.get();
            self.attached = None;
            self.link_attached = false;
            self.failures += 1;
            if is_crash(code) {
                self.crashes += 1;
            } else {
                // A death that is not a crash ends the crash loop.
                self.crashes = 0;
            }
            if self.failures >= DEFAULT_MAX_CONSECUTIVE_FAILURES
                || self.crashes >= DEFAULT_MAX_CONSECUTIVE_CRASHES
            {
                self.state = SupervisorState::PersistentFailure;
                return;
            }
        }
        // A live host whose link is detached is replaced, not reused.
        if let Some(handle) = &self.attached
            && handle.alive.get()
            && !self.link_attached
        {
            self.attached = None;
            self.failures += 1;
            if self.failures >= DEFAULT_MAX_CONSECUTIVE_FAILURES {
                self.state = SupervisorState::PersistentFailure;
                return;
            }
        }
        // The startup barrier: a host that does not greet in time is a
        // failure, and the step is spent on it. A barrier that waits (a
        // host that takes its time to start) spends the step and keeps
        // waiting for the greeting.
        if let Some(pending) = self.pending.take() {
            if self.handshake_timeout.is_zero() {
                self.failures += 1;
                if self.failures >= DEFAULT_MAX_CONSECUTIVE_FAILURES {
                    self.state = SupervisorState::PersistentFailure;
                }
            } else {
                self.pending = Some(pending);
            }
            return;
        }
        // A live attached host needs nothing.
        if self.attached.is_some() {
            return;
        }
        // Otherwise the supervisor starts a fresh host.
        let scripted = self.script.borrow().front().cloned();
        self.spawns += 1;
        if self.queued > 0 {
            self.queued -= 1;
        }
        self.coverage
            .observe_outcome(scripted.as_ref().map(|s| s.outcome));
        match scripted {
            Some(Scripted {
                outcome: Spawn::Hello,
                handle,
            }) => {
                self.attached = Some(handle);
                self.link_attached = true;
                // A host that came up is not a failure: the count starts
                // again.
                self.failures = 0;
            }
            Some(Scripted {
                outcome: Spawn::Silent,
                handle,
            })
            | Some(Scripted {
                outcome: Spawn::Late,
                handle,
            }) => {
                // The host waits for its greeting: the startup barrier.
                self.pending = Some(handle);
            }
            Some(Scripted {
                outcome: Spawn::Foreign,
                ..
            })
            | Some(Scripted {
                outcome: Spawn::Dead,
                ..
            }) => {
                self.failures += 1;
            }
            Some(Scripted {
                outcome: Spawn::Refused,
                ..
            })
            | None => {
                self.failures += 1;
            }
        }
        if self.failures >= DEFAULT_MAX_CONSECUTIVE_FAILURES {
            self.state = SupervisorState::PersistentFailure;
        }
    }
}

/// What a sequence reached.
///
/// The tests assert that the seeds together reach every part of the policy:
/// a property that stops reaching a part would be a silent false green.
#[derive(Clone, Copy, Debug, Default)]
struct Coverage {
    /// A host started and greeted.
    hello: bool,
    /// A host greeted with a foreign protocol version.
    foreign: bool,
    /// A host never greeted.
    silent: bool,
    /// A host greets only when the test says so.
    late: bool,
    /// A host was already dead when it was spawned.
    dead: bool,
    /// A spawn was refused.
    refused: bool,
    /// The running host died from an exception.
    crash: bool,
    /// The running host caught a plugin fault.
    fault: bool,
    /// The running host exited on its own.
    quit: bool,
    /// The running host stopped answering.
    hang: bool,
    /// The user stopped the supervision.
    user_stop: bool,
    /// The user resumed the supervision.
    resume: bool,
    /// The retries ended.
    failed: bool,
    /// The supervision was stopped.
    stopped: bool,
    /// The supervision was running.
    running: bool,
}

impl Coverage {
    /// Add what another sequence reached.
    fn merge(&mut self, other: Coverage) {
        self.hello |= other.hello;
        self.foreign |= other.foreign;
        self.silent |= other.silent;
        self.late |= other.late;
        self.dead |= other.dead;
        self.refused |= other.refused;
        self.crash |= other.crash;
        self.fault |= other.fault;
        self.quit |= other.quit;
        self.hang |= other.hang;
        self.user_stop |= other.user_stop;
        self.resume |= other.resume;
        self.failed |= other.failed;
        self.stopped |= other.stopped;
        self.running |= other.running;
    }

    /// Add what one event reached.
    fn observe_event(&mut self, event: Event) {
        match event {
            Event::Spawn(_) => {}
            Event::Crash => self.crash = true,
            Event::Fault => self.fault = true,
            Event::Quit => self.quit = true,
            Event::Hang => self.hang = true,
            Event::UserStop => self.user_stop = true,
            Event::Resume => self.resume = true,
            Event::Idle => {}
        }
    }

    /// Add what one spawn attempt produced.
    fn observe_outcome(&mut self, outcome: Option<Spawn>) {
        match outcome {
            Some(Spawn::Hello) => self.hello = true,
            Some(Spawn::Foreign) => self.foreign = true,
            Some(Spawn::Silent) => self.silent = true,
            Some(Spawn::Late) => self.late = true,
            Some(Spawn::Dead) => self.dead = true,
            Some(Spawn::Refused) | None => self.refused = true,
        }
    }

    /// Add what one state reached.
    fn observe_state(&mut self, state: SupervisorState) {
        match state {
            SupervisorState::Running => self.running = true,
            SupervisorState::Stopped => self.stopped = true,
            SupervisorState::PersistentFailure => self.failed = true,
        }
    }

    /// Did the sequences reach every part of the policy?
    fn everything(&self) -> bool {
        self.hello
            && self.foreign
            && self.silent
            && self.late
            && self.dead
            && self.refused
            && self.crash
            && self.fault
            && self.quit
            && self.hang
            && self.user_stop
            && self.resume
            && self.failed
            && self.stopped
            && self.running
    }
}

/// The supervisor under test, the world it acts on and the model of what it
/// must do.
struct Harness {
    supervisor: Supervisor<ScriptedBackend>,
    link: HostStateLink,
    script: Rc<RefCell<VecDeque<Scripted>>>,
    hosts: Rc<RefCell<Vec<HostHandle>>>,
    greeters: Rc<RefCell<Vec<MemoryTransport>>>,
    model: Model,
}

impl Harness {
    /// A supervisor with the given cooldown, the script and the world it
    /// acts on.
    fn new(cooldown: Duration) -> Self {
        Self::with_handshake_timeout(cooldown, HANDSHAKE_TIMEOUT)
    }

    /// A supervisor with the given cooldown and startup barrier.
    fn with_handshake_timeout(cooldown: Duration, handshake_timeout: Duration) -> Self {
        let script = Rc::new(RefCell::new(VecDeque::new()));
        let hosts = Rc::new(RefCell::new(Vec::new()));
        let greeters = Rc::new(RefCell::new(Vec::new()));
        let backend = ScriptedBackend {
            script: Rc::clone(&script),
            hosts: Rc::clone(&hosts),
            peers: Vec::new(),
            greeters: Rc::clone(&greeters),
        };
        Harness {
            supervisor: Supervisor::with_failure_policy(
                backend,
                cooldown,
                handshake_timeout,
                DEFAULT_MAX_CONSECUTIVE_FAILURES,
            ),
            link: HostStateLink::detached(),
            model: Model::new(Rc::clone(&script), handshake_timeout),
            script,
            hosts,
            greeters,
        }
    }

    /// The host that waits for its greeting greets now.
    fn greet(&mut self) {
        let mut greeters = self.greeters.borrow_mut();
        let greeter = greeters
            .last_mut()
            .expect("a host that waits for its greeting");
        greeter.send(&Message::hello()).expect("the host greets");
    }

    /// The event changed the world, without the model: the tests that only
    /// watch the spawn attempts do not need one.
    fn disturb(&mut self, event: Event) {
        match event {
            Event::Idle => {}
            Event::Spawn(outcome) => {
                self.script.borrow_mut().push_back(Scripted {
                    outcome,
                    handle: HostHandle {
                        alive: Rc::new(Cell::new(true)),
                        exit_code: Rc::new(Cell::new(0)),
                    },
                });
            }
            Event::Crash | Event::Fault | Event::Quit => {
                let code = match event {
                    Event::Crash => CRASH_CODE,
                    Event::Fault => FAULT_CODE,
                    _ => 0,
                };
                let mut hosts = self.hosts.borrow_mut();
                if let Some(host) = hosts.last_mut() {
                    host.alive.set(false);
                    host.exit_code.set(code);
                }
            }
            Event::Hang => {
                // The missed-`Pong` failure detaches the link, exactly as
                // the liveness path does it.
                let now = Instant::now();
                self.link.poll_at(now);
                self.link.poll_at(now + PING_INTERVAL);
                self.link.poll_at(now + PING_INTERVAL + PONG_TIMEOUT);
            }
            Event::UserStop => self.supervisor.stop(&mut self.link),
            Event::Resume => self.supervisor.resume(),
        }
    }

    /// The event changed the world and the model expects it.
    ///
    /// The user actions act on the supervisor itself — the stop is the
    /// deliberate off-switch, the resume the explicit way out of the
    /// give-up — and the model expects what they do.
    fn apply(&mut self, event: Event) {
        match event {
            Event::UserStop => self.supervisor.stop(&mut self.link),
            Event::Resume => self.supervisor.resume(),
            _ => {}
        }
        self.model.apply(event, &mut self.link);
    }
}

/// The report must match the decision.
///
/// A give-up is reported (the player runs without plugins, and the user has
/// to know), and a report never promises a restart the supervisor will not
/// make.
fn check_report(
    before: SupervisorState,
    after: SupervisorState,
    report: Option<&str>,
    label: &str,
) {
    if before != SupervisorState::PersistentFailure && after == SupervisorState::PersistentFailure {
        assert!(
            report.is_some(),
            "{label}: the retries are over, and the user must be told: no report"
        );
    }
    match after {
        SupervisorState::PersistentFailure => {
            if let Some(text) = report {
                assert!(
                    !text.contains("A fresh host starts"),
                    "{label}: the report promises a restart the supervisor will not make: {text}"
                );
            }
        }
        _ => {
            if let Some(text) = report {
                assert!(
                    !text.contains("not started again"),
                    "{label}: the report gives up while the supervisor keeps going: {text}"
                );
            }
        }
    }
}

/// Run one random event sequence and check the supervisor against the model.
///
/// Returns what the sequence reached.
fn run_sequence(seed: u64, cooldown: Duration, steps: usize) -> Coverage {
    let mut harness = Harness::new(cooldown);
    let mut rng = Rng::new(seed);

    for step in 1..=steps {
        let event = choose_event(&mut rng, harness.model.state);
        let label = format!("seed {seed}, step {step}, {event:?}");

        // 1. the event changes the world.
        let failures_before = harness.supervisor.consecutive_failures();
        harness.apply(event);
        match event {
            Event::UserStop => {
                assert_eq!(
                    harness.supervisor.consecutive_failures(),
                    failures_before,
                    "{label}: a user stop is not a failure"
                );
                assert!(
                    !harness.supervisor.is_failed(),
                    "{label}: a user stop never reaches the give-up"
                );
            }
            Event::Resume => {
                assert_eq!(
                    harness.supervisor.consecutive_failures(),
                    0,
                    "{label}: the explicit user action starts the count again"
                );
            }
            _ => {}
        }
        harness.model.coverage.observe_event(event);

        // 2. the model predicts what the supervisor's step does.
        harness.model.step();

        // 3. the supervisor takes its step.
        let before = harness.supervisor.state();
        let report = harness.supervisor.supervise(&mut harness.link);

        // 4. the invariants.
        assert_eq!(
            harness.supervisor.consecutive_failures(),
            harness.model.failures,
            "{label}: the failure counter is consecutive"
        );
        assert_eq!(
            harness.supervisor.state(),
            harness.model.state,
            "{label}: the state"
        );
        assert_eq!(
            harness.supervisor.spawn_attempts(),
            harness.model.spawns,
            "{label}: the spawn attempts"
        );
        assert_eq!(
            harness.link.is_attached(),
            harness.model.link_attached,
            "{label}: the link is attached exactly while a host is up"
        );
        assert_eq!(
            harness.model.script.borrow().len(),
            harness.model.queued,
            "{label}: the script"
        );
        check_report(
            before,
            harness.supervisor.state(),
            report.as_deref(),
            &label,
        );

        // 5. the coverage.
        harness
            .model
            .coverage
            .observe_state(harness.supervisor.state());
    }
    harness.model.coverage
}

/// The state machine of the supervisor under a random sequence of events:
/// the consecutive counter, the crash ledger, the give-up and the sticky
/// terminal state.
#[test]
fn a_random_event_sequence_keeps_the_restart_policy_consistent() {
    let mut coverage = Coverage::default();
    for seed in SEEDS {
        coverage.merge(run_sequence(seed, COOLDOWN, STEPS));
    }
    assert!(
        coverage.everything(),
        "the seeds must reach every part of the policy: {coverage:?}"
    );
}

/// The cooldown bounds the retries: within the window no restart is
/// attempted, whatever the events.
///
/// The window is an hour, so no test can leave it: a second spawn would
/// have to wait an hour, and the property is about that wait.
#[test]
fn the_cooldown_bounds_the_retries() {
    let window = Duration::from_secs(3_600);
    for seed in SEEDS {
        let mut harness = Harness::new(window);
        let mut rng = Rng::new(seed);
        for step in 1..=STEPS {
            let event = choose_event(&mut rng, harness.supervisor.state());
            let label = format!("seed {seed}, step {step}, {event:?}");
            harness.disturb(event);
            harness.supervisor.supervise(&mut harness.link);
            assert!(
                harness.supervisor.spawn_attempts() <= 1,
                "{label}: no restart is attempted inside the cooldown window"
            );
        }
    }
}

/// Without a cooldown the supervisor does replace the host: the cooldown is
/// what bounds the retries, not the policy giving up at once.
#[test]
fn without_a_cooldown_the_host_is_replaced() {
    let mut harness = Harness::new(Duration::ZERO);
    harness.apply(Event::Spawn(Spawn::Hello));
    harness.model.step();
    harness.supervisor.supervise(&mut harness.link);
    assert_eq!(harness.supervisor.spawn_attempts(), 1);
    assert!(harness.link.is_attached());

    // The host dies: the next step replaces it.
    harness.apply(Event::Crash);
    harness.model.step();
    harness.supervisor.supervise(&mut harness.link);
    assert_eq!(
        harness.supervisor.spawn_attempts(),
        2,
        "a dead host is replaced"
    );
}

/// The terminal state is sticky: once the retries are over, no event starts
/// a host again — and the explicit user action is the only way out.
#[test]
fn the_give_up_is_sticky_until_the_user_resumes() {
    let mut harness = Harness::new(COOLDOWN);
    // Three refusals in a row end the retries.
    for _ in 0..3 {
        harness.apply(Event::Spawn(Spawn::Refused));
        harness.model.step();
        harness.supervisor.supervise(&mut harness.link);
    }
    assert!(harness.supervisor.is_failed());
    let attempts = harness.supervisor.spawn_attempts();

    // Nothing brings the host back.
    for event in [
        Event::Idle,
        Event::Spawn(Spawn::Hello),
        Event::Crash,
        Event::Quit,
        Event::Hang,
    ] {
        harness.apply(event);
        harness.model.step();
        harness.supervisor.supervise(&mut harness.link);
        assert_eq!(
            harness.supervisor.spawn_attempts(),
            attempts,
            "the give-up is sticky: {event:?} started a host"
        );
        assert!(
            !harness.link.is_attached(),
            "the give-up is sticky: {event:?} attached a host"
        );
        assert!(harness.supervisor.is_failed());
    }

    // The user stop is the other explicit user action: it stops the
    // supervision, and it does not start a host either.
    harness.apply(Event::UserStop);
    harness.model.step();
    harness.supervisor.supervise(&mut harness.link);
    assert_eq!(
        harness.supervisor.spawn_attempts(),
        attempts,
        "the give-up is sticky: the user stop started a host"
    );
    assert!(harness.supervisor.is_stopped());

    // The explicit user action is the way out.
    harness.apply(Event::Resume);
    harness.model.step();
    assert_eq!(harness.supervisor.state(), SupervisorState::Running);
    assert_eq!(harness.supervisor.consecutive_failures(), 0);
    harness.apply(Event::Spawn(Spawn::Hello));
    harness.model.step();
    harness.supervisor.supervise(&mut harness.link);
    assert!(
        harness.link.is_attached(),
        "the user action starts the host again"
    );
}

/// A user stop is never a failure: it moves neither the counter nor the
/// crash ledger, and it never reaches the give-up.
#[test]
fn a_user_stop_is_never_a_failure() {
    let mut harness = Harness::new(COOLDOWN);
    // A failure first, so the counter is not zero to begin with.
    harness.apply(Event::Spawn(Spawn::Refused));
    harness.model.step();
    harness.supervisor.supervise(&mut harness.link);
    assert_eq!(harness.supervisor.consecutive_failures(), 1);

    harness.apply(Event::UserStop);
    harness.model.step();
    assert_eq!(
        harness.supervisor.consecutive_failures(),
        1,
        "a user stop is not a failure"
    );
    assert!(harness.supervisor.is_stopped());
    assert!(!harness.supervisor.is_failed());

    // The stop sticks: the failures that follow are not counted either.
    harness.apply(Event::Spawn(Spawn::Refused));
    harness.model.step();
    harness.supervisor.supervise(&mut harness.link);
    assert_eq!(
        harness.supervisor.consecutive_failures(),
        1,
        "a stopped supervisor counts no failures"
    );
    assert!(harness.supervisor.is_stopped());
}

/// A host that takes its time to start is a success like any other: the
/// startup barrier waits for the greeting, and the count starts again when
/// it arrives.
#[test]
fn a_host_that_greets_late_still_resets_the_counter() {
    let mut harness = Harness::with_handshake_timeout(Duration::ZERO, SLOW_HANDSHAKE_TIMEOUT);

    // A failure first, so the count is not zero to begin with.
    harness.apply(Event::Spawn(Spawn::Refused));
    harness.supervisor.supervise(&mut harness.link);
    assert_eq!(harness.supervisor.consecutive_failures(), 1);

    // A host that takes its time: the barrier waits for the greeting, and
    // the step is spent on it.
    harness.apply(Event::Spawn(Spawn::Late));
    harness.supervisor.supervise(&mut harness.link);
    assert!(
        !harness.link.is_attached(),
        "the link must stay detached until the greeting arrives"
    );
    assert_eq!(
        harness.supervisor.consecutive_failures(),
        1,
        "a host that is still starting is not a failure"
    );

    // The greeting arrives: the barrier is passed and the count starts
    // again.
    harness.greet();
    harness.supervisor.supervise(&mut harness.link);
    assert!(
        harness.link.is_attached(),
        "the greeting must attach the host"
    );
    assert_eq!(
        harness.supervisor.consecutive_failures(),
        0,
        "a host that came up resets the count"
    );
}

/// No panic, whatever the sequence: an empty one and a storm of events.
#[test]
fn no_sequence_of_events_panics() {
    // An empty sequence: the supervisor is simply running.
    let harness = Harness::new(COOLDOWN);
    assert_eq!(harness.supervisor.state(), SupervisorState::Running);
    assert_eq!(harness.supervisor.consecutive_failures(), 0);
    assert_eq!(harness.supervisor.spawn_attempts(), 0);

    // A storm: the events keep coming, the script runs out, and the
    // supervisor keeps the player running.
    for seed in SEEDS {
        run_sequence(seed, COOLDOWN, STORM);
    }
}
