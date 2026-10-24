# W23 — RFC 0012 implementation audit and lifecycle blocker

## Status and boundaries

**Blocked on owner clarification; no production code or tests changed.**
RFC 0012 is Accepted (2026-10-18). The requested hook and most of its
implementation already exist in this worktree, including the H5 task card.
This lane must not claim those pre-existing changes as its implementation.

Write only your own files, and nothing under `D:\code\Rustamp`.
The only writable checkout is
`C:\Users\micro\.local\share\opencode\worktree\26de0e\wave109-rfc0012`.
Owned paths are `crates/rustamp-core/**`, `crates/rustamp-output/**`,
`crates/rustamp-audio/**`, and this card. The initial worktree status was empty.
Application files below were read only, to understand the neighbouring lane.

## Accepted scope, per crate

- **core:** `OutputDevice::stream_error(&self) -> Option<Error>`, default `None`,
  polled on the control thread, non-blocking and lock-free. Existing implementors
  remain compatible. This is already present at `audio.rs:388-401`.
- **output:** read the existing WASAPI integer/cpal boolean latches with Acquire
  ordering and return `Error::Output` naming the device. NullOutput has an atomic
  latch, `NullClock::fail()`, and a reset on successful `open()`. These already
  exist at `wasapi_out.rs:423-426`, `cpal_out.rs:400-409`, and
  `null.rs:148,210-216,263-275`. No audio-thread event publishing is needed.
- **audio:** while Playing, poll before draining decode reports; on failure end
  decoding, pause output, stop transport, freeze position, then publish Stopped,
  PositionChanged, and Error(Output). Do not silently reconnect or change devices.
  The poll hook already exists at `engine.rs:1249-1255`; teardown and publication
  exist at `engine.rs:1484-1512`. The RFC also promises a fresh `open()` on a later
  explicit Play (§4.3 item 8); that recovery path is missing.
- No wire, protocol-version, plugin ABI, dependency or licence changes are needed.
  The RFC's historical protocol-version parenthesis does not authorize changing
  the current version.

## Stop condition: decisions required from the owner

### W23-Q1 — How does explicit Play obtain a fresh output lifecycle?

RFC 0012 §4.3 item 8 requires a clean `open()` after the user presses Play again.
In the existing engine, `open()` is only called in `Engine::new` (`engine.rs:292`).
After a NullOutput failure, teardown leaves its latch set; `start_run()` then
returns that latched error (`engine.rs:1560-1567`). Neither `play()` nor
`start_run()` reopens the device. WASAPI similarly retains its fatal latch.
The existing test `device_failure_before_play_errors_immediately` pins a pre-run
rejection, not the promised recovery after a previous run failed.

**Question:** Should the same engine close/open its output and reconstruct the
render callback/ring for explicit retry, or should the application recreate the
engine/output? Specify which state is preserved (including the frozen position)
and the behaviour if the newly granted output format differs. `start()` takes
ownership of the callback; `close()` destroys it in the backends, so simply
inserting `open()` is not a complete lifecycle fix. No recovery mechanism chosen.

### W23-Q2 — Does fatal stream teardown flush, and how does that compose with recreation?

RFC 0012 §4.3 lists deactivate/join, pause, stop/freeze, events. The current
`handle_output_error()` additionally calls `flush_output()` before joining
(`engine.rs:1485-1496`). That function deliberately makes any flush refusal
terminal, closes the output, and sets a sticky `flush_failure`
(`engine.rs:1521-1543`). This neighbouring contract must remain intact.

cpal unconditionally refuses flush (`cpal_out.rs:381-385`). Therefore a fatal
cpal stream error also arms the terminal-flush path. The application's
`surface_error()` then recreates the engine/output when it sees that marker
(`crates/rustamp-app/src/app.rs:1452,1470-1480`). `recreate_engine()` preserves
queue/settings but does not copy the frozen timeline (`app.rs:411-447`). The
engine's freeze event can consequently be followed by a fresh zero-position
engine rather than the persistent frozen snapshot described by RFC 0012.

**Question:** Should stream-error teardown omit flushing as §4.3 describes, or
retain the existing flush and its caller-side recreation? If recreation is
required, who preserves the failure position and when does reopening occur?
Changing the shared teardown without an answer could also break failed-seek
handling, which uses it at `engine.rs:1189,1275`. No flush guard, teardown call,
or application recreation path was changed.

If the owner selects application-managed stream-error recovery, the exact
outside-owned-path follow-up is in `crates/rustamp-app/src/app.rs`:
`surface_error()`, `ensure_engine_ready()` and `recreate_engine()` must distinguish
fatal stream loss from terminal flush refusal, implement the selected retry
timing, and preserve/publish the selected stopped-position policy. This lane
cannot make those edits. The selection itself is intentionally left open.

### W23-Q3 — Keep or revise the existing pre-command policy?

RFC §8 Q3 still presents pre-command polling as a question (recommendation: poll
and resume). Existing code checks both resume (`engine.rs:953-955`) and every
start (`engine.rs:1560-1567`). Confirm how the pre-start check should compose with
the fresh-open retry required by §4.3; retaining it unchanged prevents recovery.
No additional command checks were introduced and no existing check was removed.

Per the assignment, discovering these unresolved lifecycle cases stops
implementation rather than granting permission to invent a new contract.

## Tests: existing evidence and missing proof

**New tests: zero.** No new test encodes an owner decision that remains open.
No test was run, no VM or broker was invoked, and no timing sleep was introduced.
The following are existing tests, inspected as source, not passing receipts:

| Existing test | What can make it fail for the relevant reason / limitation |
|---|---|
| core `stream_error_defaults_to_none` | Removing the default breaks the minimal implementor at compile time; returning an error breaks the assertion. |
| null `stream_error_returns_none_initially` | Incorrect initial latch or unconditional error breaks the assertion. |
| null `stream_error_reflects_injected_failure` | Removing the fail store, ignoring the latch, or returning the wrong error breaks assertions. |
| null `open_clears_injected_failure` | Removing the reset on open breaks the post-open assertions. It tests opening a failed but unopened output, not close/open recovery of a started stream. |
| WASAPI `wasapi_stream_error_reflects_failure_streak` | Incorrect streak threshold, latch reader or message breaks assertions. The test manually calls `fail` after ticking: it does not prove the real render loop connects the threshold to the latch. |
| cpal `cpal_stream_error_reflects_error_callback` | Ignoring the injected latch or wrong text breaks assertions. It directly stores the flag, so it does not prove delivery from an actual cpal callback. |
| audio `engine_stops_and_publishes_error_when_output_fails_mid_run` | Removing the poll hook, stop transition, error publication or pause breaks assertions. Despite its comment, it does not explicitly assert PositionChanged, event ordering, or decoder join; zero further pumping follows from the failed null latch alone. |
| audio `failure_is_reported_once_not_per_poll` | Missing first error or duplicate later errors/state events break assertions. |
| audio `healthy_device_is_unaffected` | An unconditional failure path breaks the Playing/no-error assertions. |
| audio `failure_observed_within_one_poll` | Delayed/missing detection breaks the single-poll assertions. |
| audio `device_failure_while_paused_prevents_resume` | Resuming a failed paused stream or omitting error publication breaks assertions. |
| audio `device_failure_before_play_errors_immediately` | Removing the pre-run rejection breaks assertions; it must not be misreported as a retry-after-failure test. |

After lifecycle clarification, add explicit ordered-event assertions, a nonzero
failure-position oracle independent of the null device refusing to pump, and a
decode completion/returned-producer witness. Add an explicit retry test using
open/close/start counters and a scripted failure/recovery; assert the selected
format and frozen-state semantics. Use channels/atomics or observed predicates
with a failure deadline, never sleeps as ordering. These tests are not added
now because they would partly encode the unresolved decisions above.

## Verification and delivery

Gate commands used offline Cargo and a target/temp directory inside this worktree;
each native exit code was captured immediately into a variable:

| Gate | Exit code |
|---|---:|
| `cargo fmt --all -- --check` | 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 |
| `cargo build --workspace` | 0 |

Only this card was added; all three owned crates have zero changed lines.
The final canonical-checkout `git status --short` was empty.
No downloads, dependencies, Git writes or commits were requested or performed.

Only the orchestrator's interactive-session VM may run the suite. VM evidence is
still needed for the existing tests, the new lifecycle regression tests after
clarification, real WASAPI/cpal device-loss notification, and the full
engine-to-window/plugin stopped/frozen/error propagation. A VM run alone cannot
prove physical USB disconnects or every third-party driver's teardown behaviour
without an appropriately controlled endpoint/fault scenario.

Provenance: own repository source, task cards, RFC 0012/0002 and project rules
only. No Winamp source, SDK, disassembly, reference binaries or research tree was
read; no third-party code was copied.
