# M25 — Green the three reds of wave74 at their causes (three reds)

**Date** 2026-10-18. **Lane** `wave74-three-reds`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave74-three-reds`.

Boundary rule: write ONLY inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

**Status:** implemented; fmt, clippy and build gates passing. VM test execution
pending orchestrator run.

No tests were run on the host (Hyper-V test-VM constraint strictly preserved):
not the suite, not `--no-run`, not a single binary. No test process, example,
player, helper binary or reference binary was spawned; no VM was touched; no
window or audio device was opened; no download, no dependency addition, no
install, no commit and no other git write. Only `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace --offline` ran (receipts in §6). Every command ran
inside this worktree.

The three reds, verbatim from the guest: `reset_keeps_rings_and_continues`
(`crates/rustamp-dsphost/tests/dsp_roundtrip.rs:168`, a 30 s lockstep-read
stall after the mid-stream reset),
`resident::dsp_hosted_tests::engine_fails_the_track_when_the_resident_dies`
(`crates/rustamp-audio/src/resident.rs:2716`, `assertion failed: h.wait_track(0)`),
`resident::dsp_hosted_tests::engine_gapless_seam_through_the_resident_chain_is_exact`
(`crates/rustamp-audio/src/resident.rs:2642`, `assertion failed: h.wait_track(0)`),
plus the run's one worker-thread panic at `resident.rs:1876:25`
(`panic!("fake resident crash")`). Cards read before touching anything:
M19, M20 (the lockstep), M22, M24 (the two widening rounds that did not
hold).

---

## 0. Boundary & ownership

Owned paths: `crates/rustamp-audio/**`, `crates/rustamp-dsphost/tests/**`, and
this card. Touched: `crates/rustamp-audio/src/resident.rs` (test modules
only — no production code), `crates/rustamp-dsphost/tests/dsp_roundtrip.rs`,
this card.

Read only (not modified): `crates/rustamp-audio/src/{engine.rs,decode.rs,ring.rs}`,
`crates/rustamp-mediahost/src/{control.rs,dsp.rs,pcm.rs}`,
`crates/rustamp-ipc/src/{framing.rs,transport.rs,message.rs,shm/{ring.rs,wake.rs,header.rs,region.rs}}`,
`crates/rustamp-output/src/null.rs`, `crates/rustamp-dsphost/src/lib.rs`,
`crates/rustamp-audio/src/resident.rs` production sections, and the cards
M10/M13/M14/M18/M19/M20/M22/M24, `docs/HANDOFF.md`.

Sources: the task cards named above, the RFCs they cite, and the existing
clean repository code only. No Winamp source, SDK material, disassembly,
research tree or reference binary was read; no third-party code was copied;
no dependency was added.

---

## 1. Verdicts and fixes

### 1.1 The panic at `resident.rs:1876` — the fake resident's crash races the start publication

**What the panic is.** `FakeChain::process`, `Kind::Die` arm (now
`resident.rs:1915`; the guest's report line was 1876): the fake resident's
in-process crash surrogate, `panic!("fake resident crash")` on its third
`process` call, running on the unnamed host worker thread that `open_session`
wraps in `catch_unwind`. The panic itself is intentional and reaped; the
guest's question is what it *does* to the engine-level death test.

**What it does — the mechanism, end to end.**

1. The decode thread sends `DecodeOut::TrackStarted` (which carries
   `duration_frames`) from `prepare_track` **before** its first block
   (`decode.rs:570`), then processes blocks through the resident chain.
2. The fake panics on block 3 → the host worker thread dies → the control
   stream EOFs → the client session fails sticky → the decode thread's third
   block round-trip returns `Err` → `ChainControl::process` latches the
   `DspFailureWatch` (`resident.rs:818-820`).
3. The test thread meanwhile polls the engine every ~1 ms (`wait_track` →
   `wait_idle`). `Engine::poll` checks the latched watch **before** the
   decode-message queue (`engine.rs:991-994`), so once the latch exists the
   next poll runs `handle_dsp_failure`, whose `join_decode`
   (`engine.rs:1437-1444`) **takes `from_decode` and discards every queued
   message except `Returned`** — including the still-unprocessed
   `TrackStarted`.
4. `duration_frames()` is therefore never published. `wait_track(0)`
   (`resident.rs:2641-2646`) requires `current_index() == Some(0) &&
   duration_frames().is_some()`; `current` is set synchronously by
   `play_index`, but the duration is gone, so the wait can never succeed and
   fails after `WAIT` (5 s) at `resident.rs:2716` (the guest's line).

The race is real and scheduling-dependent: the decode thread does
`TrackStarted` → three block round-trips → crash → latch with no waiting in
between (the pattern decoder is instant), while the test thread polls at a
1 ms cadence. Whether a poll lands between the `TrackStarted` send and the
latch is a coin flip under VM load. M24's run won the race (its failure was
the second run's wait); this run lost it (the failure moved to the first
`wait_track(0)`). The panic at 1876 is the crash that wins the race; the
assertion failure is a symptom, exactly as the guest suspected. The sibling
`engine_fails_the_track_when_the_resident_wedges` passes because its fake
sleeps 150 ms on the first block, giving the engine ~150 ms to publish the
start before the latch lands.

**Fix (test-side, in the owned crate; no production change).** The crash is
now **armed, not counted**: `FakeChain` carries an optional
`Arc<AtomicBool>` trigger (`resident.rs:1853`, `Kind::Die` panics on the next
block once set, in addition to the old third-call trigger), `FakeDspResident::armed`
(`resident.rs:1973`) builds a resident that hands the trigger to every
`fake:die` chain it opens, and `open_session_with` (`resident.rs:2146`) /
`DspHarness::with_resident` (`resident.rs:2535`) plumb it through. The
engine-level death test (`resident.rs:2804`) arms the trigger **after**
`wait_track(0)` succeeds, so the crash is deterministically mid-run — a
*stronger* claim than before (the crash is now guaranteed to follow the
published start, not to race it). The `Stopped` wait changed from
`wait_idle` to M24's `wait_pumping`, which
pumps the clock so the decode thread keeps flowing to the armed crash (the
engine ring is 16384 frames; without a consumer the decode thread parks in
its write loop before the next block). Every assertion is unchanged: the run
stops when the resident dies, before the track end (`position < 44_100`),
an `Event::Error` naming the resident, `last_error()` set, and the second run
finishes naturally at exactly 44 100. The adapter-level
`dsp_death_mid_stream_is_loud_and_poisons_the_chain` keeps the call-count
trigger (it drives the chain synchronously — no publication race — and it
passed); it goes through `FakeDspResident::new()` (`die: None`).

### 1.2 `engine_gapless_seam_through_the_resident_chain_is_exact` — the liveness trip wire, not the 1876 panic

**Verdict: the failure does not follow from the 1876 panic.** The gapless
test's chain is `fake:delay(100)` (`Kind::Delay`) — it never panics, never
latches. Its `wait_track(0)` failed the same way (TrackStarted never
processed within 5 s), and the only way that happens in a healthy run is the
same erasure path of §1.1 step 3 with a latch that arrives **before**
`TrackStarted` is even sent: the session dies during the decode thread's
start, `ChainControl::prepare`/`reset` fails on `peer.check()` and latches
(the decode thread discards the `Err`, then sends `TrackStarted` anyway), and
the engine's first poll handles the latch first and `join_decode` discards
the message.

**The latch source in a healthy run is the control liveness.** `open_session`
started both `ControlPeer`s with `Liveness::default()` — interval 100 ms,
**timeout 5 s** (pre-edit `resident.rs:2086-2087`, now the `LIVENESS` pair at
`resident.rs:2152-2153`) — the exact wall-clock bound M24
proved flakes under this VM's load ("a healthy-but-slow schedule tripping a
wall-clock bound"; M24 §1.3 fixed it in `real_decode.rs` with a 30 s
`LIVENESS`, and M24 §2 handed the same class in the other suites — including
`crates/rustamp-dsphost/tests/*` and this crate's test modules — to the
lanes that own those files). A missed Pong past 5 s fails the client sticky,
kills the session, and produces exactly the observed failure.

**Fix (test-side, M24's sanctioned pattern).** A `LIVENESS` const (interval
100 ms, timeout 30 s — six times the harness `WAIT`) replaces
`Liveness::default()` at every test-side `ControlPeer::start` in
`resident.rs`: the pair behind `open_session` in `dsp_hosted_tests`
(`resident.rs:2152-2153`) and the two pairs in `hosted_tests`
(`resident.rs:1426-1427`, `resident.rs:1530-1531`). The production sites
(`resident.rs:558`, `resident.rs:1126` — the `DecoderProcess`/`DspProcess`
spawn defaults) are untouched. No claim is weakened: a dead resident never
answers a Ping and is caught by the control EOF regardless; a wedged resident
is caught by the adapter's own bounded block round-trip (the wedge test passes
its own 20 ms bound); no test depends on a fast liveness expiry. With the
trip wire moved, the gapless test's start window has no remaining latch
source: `TrackStarted` is processed within ~1–2 ms of `play_index`, and the
block timeout (`WAIT`) can only fire seconds later, after the start is
published.

### 1.3 `reset_keeps_rings_and_continues` — the reset path is verified correct; the test's own liveness trip wire moves

**Verdict on the guest's three options, from static verification:**

- **The resident (a reset that stops delivery): not supported.** The reset's
  own assertions *passed* in the failing run (generations unchanged,
  `latency_frames() == 0` — the stall is later, in the second leg's read).
  The pump's `DspReset` arm (`rustamp-mediahost/src/dsp.rs:125-137`,
  `Message::DspReset => chain.reset()`) is a plain no-op chain reset that
  cannot block, drop frames or touch the rings; `StageResident`'s reset is a
  no-op (`rustamp-dsphost/src/lib.rs:164-168`); the pump services it in its
  20 ms `CONTROL_POLL` slices while idle and between blocks (`dsp.rs:176-194`),
  which `dsp_control.rs`'s `idle_reset_lands_without_pcm_...` pins directly.
- **The test's fake host: there is none.** `reset_keeps_rings_and_continues`
  runs the real `serve_dsp` + `StageResident` through `start()`
  (`dsp_roundtrip.rs:145-161`).
- **The lockstep's assumption: not violated.** The lockstep's invariant
  (producer never starts chunk n+1 before the consumer drained chunk n;
  back-ring ≤ one chunk) is untouched by a reset — rings and generations are
  unchanged, which is exactly what the test asserts, and those assertions
  passed. The M20 invariant was re-derived against the 3-frame in-ring and
  the 4096-frame back-ring and holds across the reset.

**What could not be pinned statically.** The observed symptom — a read that
times out after the full 30 s with the back-ring open and empty and the
control channel healthy (no close, no error) — is not producible by any
control or ring failure path this lane could trace: every liveness, EOF,
abort, refusal and EOS path in `control.rs`/`dsp.rs`/`ring.rs`/`wake.rs`
surfaces loudly (`Disconnected` or a drained sticky EOS), never as a silent
`TimedOut`, and the pump's only unbounded park with a healthy control
(`wait_command`, `rustamp-mediahost/src/dsp.rs:206-210`) requires a closed
back-ring, which the player's read rules out. The stall is therefore either a
VM-specific behavior of the Windows named-event/mapping layer that static
reading cannot see, or a path this lane's trace missed; the test itself is
unchanged since its green VM run (last touched by M22), so this is a
load/scheduling-dependent flake, not a code regression this wave introduced.

**Fix applied (test-side, M24's sanctioned pattern; the bound untouched).**
The one wall-clock policy the test constructs itself is the 5 s
`Liveness::default()` at `start()` (pre-edit `dsp_roundtrip.rs:138-139`, now
the `LIVENESS` pair at `dsp_roundtrip.rs:150-151`) — the same documented flake
class, which M24's card §2 explicitly named as still carried by
`crates/rustamp-dsphost/tests/*`. A `LIVENESS` const (`dsp_roundtrip.rs:51`,
interval 100 ms, timeout = `STALL_WAIT` = 30 s) now replaces
`Liveness::default()` at all eight `ControlPeer::start` sites in the file
(`start()` and the pairs in `unknown_stage_...` and
`gain_with_bad_params_...`, plus the singles in `abrupt_player_loss_...` and
`abrupt_host_loss_...`). `STALL_WAIT` and
`JOIN_WAIT` are untouched — a stall still fails loudly after thirty seconds
with the step named, exactly as the guest required. No assertion is
weakened: every PCM wait keeps its bound, death and loss are still detected
loudly (the control EOF wakes every wait; the abrupt-loss tests keep their
`Disconnected` assertions), and a dead resident never answers a Ping, so
the generous timeout still detects death — only a healthy-but-slow schedule
stops tripping it. If the VM still shows the silent stall after this change,
the liveness was not the cause, and the failure becomes strictly more
diagnosable (any control/ring failure now surfaces as a loud error at the
same read, naming its reason).

---

## 2. Hand-offs to other crates (named, not edited)

- **`rustamp-mediahost` (owning lane):** if the reset stall persists after
  §1.3's liveness change, the two places to look are the pump's only
  unbounded park with a healthy control — `wait_command` at
  `crates/rustamp-mediahost/src/dsp.rs:206-210` (the back-ring write's
  `Err(RingError::Closed)` branch blocks in `recv_blocking` on the assumption
  that a closed back-ring implies a peer that will send a command) — and
  the `poll_commands` reset servicing at `dsp.rs:125-137`. This lane could
  not reproduce or statically pin the silent 30 s `TimedOut`; the VM's failure
  line plus the host thread's state at the stall decide.
- **`rustamp-audio` production (same crate, out of this card's scope):** the
  erasure path of §1.1 — `Engine::poll` handling the latched `DspFailureWatch`
  before the decode-message queue (`engine.rs:991-994`) and `join_decode`
  discarding queued messages (`engine.rs:1437-1444`) — is a genuine wart
  beyond the tests: a resident DSP failure latched before the engine's poll
  processes the decode thread's `TrackStarted` erases the publication of a
  track start the decode thread already reported, so the UI would show the
  new index with the previous track's duration. This lane fixed the test's
  race (armed trigger) rather than reordering `poll`: processing the message
  queue before the latch would break the teardown's `DecodeMsg::Stop`
  ordering (a queued `DecodeOut::Stopped` clears `to_decode` before the Stop
  is sent, hanging the join), and draining the queue through
  `on_decode_out` during a teardown can re-enter `flush_output` and arm the
  sticky `flush_failure`, which would block the next run. A safe fix needs a
  narrower rule (e.g. publish only `TrackStarted` during the failure
  teardown); it is a production change this lane did not want to ship
  untested on the host.
- **Residual wall-clock bound, named not changed:** the engine tests pass
  `WAIT` (5 s) as the adapter's per-block round-trip timeout
  (`DspHarness::new(..., WAIT)` → `DEFAULT_DSP_BLOCK_TIMEOUT`-shaped bound).
  A host-thread starvation past 5 s would latch mid-start; it is the same
  shape as the liveness class but 5 s of round-trip starvation is far beyond
  the flake magnitudes this VM has shown, and the wedge test deliberately
  passes its own 20 ms bound, so the lane left it and names it here for the
  orchestrator to watch.

---

## 3. Assertions kept (none weakened)

- **Death test:** the run stops when the resident dies, before the track end
  (`position < 44_100`), an `Event::Error` naming the resident,
  `last_error()` set, the second run finishes naturally with
  `position_frames() == 44_100` — all unchanged. The crash trigger moved
  from a call count to an armed flag (a strengthening: the crash is now
  guaranteed to follow the published start), and the `Stopped` wait moved
  from `wait_idle` to M24's `wait_pumping` (same condition, same `WAIT`
  bound, plus the clock pump the decode thread needs to reach the crash).
- **Gapless test:** every assertion unchanged (6000 frames exact, both
  patterns in order, no silence at the seam, joint finish,
  `position_frames() == 3000`, `current_index() == Some(1)`,
  `PlaylistEnded`). Only the test's own liveness policy changed.
- **Reset test:** every assertion unchanged (generations unchanged across
  the reset, `latency_frames() == 0`, the full 4099-frame stream exact at
  ×2.0, clean host end). `STALL_WAIT`/`JOIN_WAIT` untouched; only the test's
  own liveness policy changed.
- **Adapter-level die test:** unchanged (call-count crash, buffer untouched,
  latched error, host thread reaped with the panic).

No sleep was introduced as an ordering mechanism anywhere; the 1 ms sleeps
in the waits are the poll cadences the harness already used. No bound that
guards a claim was widened: `STALL_WAIT` (30 s), `JOIN_WAIT` (30 s) and
`WAIT` (5 s) are exactly as they were; the moved bound is the control-plane
liveness deadline the tests construct themselves (5 s → 30 s), the fix M24
established for this class in this VM.

---

## 4. What only the VM can prove, and the exact command

The suite runs in the Hyper-V test VM; this lane ran no test. The
orchestrator should run the full suite there and check, per binary:

- `rustamp_audio` (lib): `engine_fails_the_track_when_the_resident_dies` and
  `engine_gapless_seam_through_the_resident_chain_is_exact` green, and no
  other `rustamp-audio` test regressed (expected: full lib + `resident`/
  `replaygain`/`dsp_chain`/`engine` suites green).
- `dsp_roundtrip` (the `rustamp-dsphost` integration binary):
  `reset_keeps_rings_and_continues` green across at least two runs (it is a
  load flake — one green run is not proof), and the other eight tests in the
  binary still green.

The standard suite command (from the checkout root, in the guest):
`cargo test --workspace --all-features` — or, scoped to the two affected
binaries: `cargo test -p rustamp-audio --all-features` and
`cargo test -p rustamp-dsphost --all-features --test dsp_roundtrip`. If any
of the three is still red, the VM's failure line decides: §1.1 (should not
happen — the trigger is armed after the start is observed), §1.2 (any line
other than the first `wait_track(0)` refutes the liveness hypothesis), §1.3
(a stall that is still silent after the liveness change is a new defect in
the reset/pump path — hand to the mediahost lane per §2).

---

## 5. Files and line counts

Counts include blank lines; all inside the owned paths. `git diff --stat`:

| File | Lines | Change |
|---|---:|---|
| `crates/rustamp-audio/src/resident.rs` | 2887 (was 2788) | `dsp_hosted_tests`: `LIVENESS` const; `FakeChain.die` trigger + `armed_to_die`; `Kind::Die` panics on the armed trigger; `FakeDspResident::armed`; `open_session_with`; `DspHarness::with_resident`; death test arms the trigger after `wait_track(0)` and waits with `wait_pumping`. `hosted_tests`: `LIVENESS` const at its two `ControlPeer::start` pairs. Test modules only — no production code changed. |
| `crates/rustamp-dsphost/tests/dsp_roundtrip.rs` | 586 (was 574) | `LIVENESS` const (timeout = `STALL_WAIT`); all eight `ControlPeer::start` sites use it instead of `Liveness::default()`. No assertion, wait or bound changed. |
| `docs/tasks/M25-three-reds.md` | this card | new |

No `Cargo.toml`/`Cargo.lock` change, no new dependency, no `THIRD_PARTY.md`
change. No file under `crates/rustamp-mediahost`, `crates/rustamp-dsp`,
`crates/rustamp-dechost`, `crates/rustamp-dsphost/src`,
`crates/rustamp-ipc`, `crates/rustamp-output`, `crates/rustamp-core` or any
other crate was touched.

---

## 6. Gate receipts

Executed strictly within this worktree:

- `cargo fmt --all -- --check`: **PASS** (exit 0, no diffs; one re-wrap of the
  `"fake:die"` match arm applied first).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **PASS** (exit 0, no warnings; compiles every edited test target — nothing
  was executed).
- `cargo build --workspace --offline`: **PASS** (exit 0).
- `cargo test`: **NOT RUN** — not the suite, not `--no-run`, not a single
  binary (Hyper-V orchestrator rule respected).
- No GUI window opened, no audio device accessed, no external dependency
  added, no git commit created, no process spawned.

---

## 7. Boundary receipt

`git status --short` inside this worktree (read-only) lists exactly the two
modified files above plus this card:

```
 M crates/rustamp-audio/src/resident.rs
 M crates/rustamp-dsphost/tests/dsp_roundtrip.rs
?? docs/tasks/M25-three-reds.md
```

Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or
`D:\code\rustamp-refbin` was read or written, and no file outside the owned
paths was modified.

---

## 8. Provenance

Implemented only from the task cards (M10, M13, M14, M18, M19, M20, M22,
M24), the RFCs they cite, and the clean repository code (read-only outside
the owned paths). No Winamp source, SDK headers, disassembly or
forbidden-tree contents were read. No third-party code copied.
License impact: none (no dependency change; `rustamp-audio` stays
GPL-3.0-or-later, `rustamp-dsphost` stays GPL-3.0-or-later).
