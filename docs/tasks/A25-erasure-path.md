# A25 — Fix the erasure path: a track start survives a resident death at the start

**Date** 2026-10-18. **Lane** `wave76-erasure-path`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave76-erasure-path`.

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

The last red point, as the guest reported it: one red binary
(`rustamp_audio`, lib) failing on
`resident::dsp_hosted_tests::engine_fails_the_track_when_the_resident_dies`
and `resident::dsp_hosted_tests::engine_gapless_seam_through_the_resident_chain_is_exact`,
both at `assertion failed: h.wait_track(0)` — `duration_frames()` never
published — plus the test fake's crash-surrogate panic
(`panic!("fake resident crash")`). M25 §1.1 worked the mechanism out and
deliberately did not fix it; this card is the fix it handed over ("a safe fix
needs a narrower rule (e.g. publish only `TrackStarted` during the failure
teardown)").

---

## 0. Boundary & ownership

Owned paths: `crates/rustamp-audio/**`, and this card. Touched:
`crates/rustamp-audio/src/engine.rs` (production) and
`crates/rustamp-audio/src/resident.rs` (test modules only — no production code
in that file changed), plus this card.

Read only (not modified): `crates/rustamp-audio/src/decode.rs`,
`crates/rustamp-audio/src/ring.rs`, `crates/rustamp-mediahost/src/dsp.rs`
(`DspSession::reset`/`reconfigure`, `serve_dsp`), the cards M15/M19/M24/M25/
M26/A8 and `docs/HANDOFF.md`.

Sources: the task cards named above, RFC 0006 §6/§8 (and RFC 0012 by
contrast), and the existing clean repository code only. No Winamp source, SDK
material, disassembly, research tree or reference binary was read; no
third-party code was copied; no dependency was added.

---

## 1. The wart and the mechanism

### 1.1 The wart (M25 §1.1, unchanged analysis)

The decode thread queues `DecodeOut::TrackStarted` — which carries
`duration_frames`/`duration_ms` — from `prepare_track` **before** its first
block (`decode.rs:570`). `Engine::poll` checks the latched `DspFailureWatch`
**before** the decode-message queue (`engine.rs:991`), and the failure teardown
`handle_dsp_failure` joins the decode thread through `join_decode`, whose drain
discarded every queued message except `Returned` — including a
`TrackStarted` the decode thread had already published but the engine had not
processed. `duration_frames()` was therefore never published and the window
would show the new track's index (`current` is set synchronously by
`play_index`) with the *previous* track's duration. Both reds fail at
`wait_track(0)` on exactly this: the death test's fake crashes on its third
block, microseconds after `play_index`, racing the test's 1 ms poll cadence
(M25's armed trigger sits *in addition to* the third-call trigger, so the
crash still fires at block 3 — the race persists), and the gapless test's red
was the same erasure symptom with a latch arriving before the queued start was
processed (M25 §1.2: the session dying during the decode thread's start — the
environmental blackout tripping the liveness or the block round-trip bound,
M26 §2).

### 1.2 The fix: hold the erased message and deliver it

- `Engine::join_decode` (`engine.rs:1447`) now **returns** the track start the
  decode thread queued but the engine never processed: the drain holds the
  last queued `TrackStarted` (the track the decode thread had open when the run
  ended) instead of discarding it, and still recovers `Returned` exactly as
  before. The other five callers (`stop`, `finish_run`, `handle_output_error`,
  `start_run`, `Drop`) discard the return — their behavior is byte-identical
  to before.
- `Engine::handle_dsp_failure` (`engine.rs:1286`) delivers the held start
  through the ordinary `on_decode_out` path right after the join
  (`engine.rs:1300-1302`), before the transport freeze and the error
  publication — the narrow rule M25's hand-off asked for: **only**
  `TrackStarted` is ever delivered during a failure teardown.

### 1.3 Why this mechanism is the one that keeps every invariant

The task named three candidates; this is why hold-and-deliver is the one:

- **"Process the queue up to the failure point"** (drain everything through
  `on_decode_out` when the failure arrives) is unsafe for the other queued
  reports: `Seeked` calls `flush_output()` — a failing flush there arms the
  sticky `flush_failure`, which blocks the next run (`start_run` refuses);
  `Error` would publish a second error; `Stopped` clears `to_decode` before
  the teardown's `Stop` is sent, breaking the stop-before-stopped ordering;
  `EndOfStream` sets an end state the teardown then clears. Only the
  `TrackStarted` arm is side-effect-free for a teardown: it sets the track's
  identity fields, clears `end_of_stream` (which the teardown clears anyway),
  and publishes `TrackChanged`/`StreamInfoChanged`/`PositionChanged` — no
  output interaction, no error publication, no control-channel mutation.
- **"Publish the start before latching"** (drain the queue for the start
  before handling the latch) fails on the race the wart is actually about:
  the latch can land while the decode thread is still *inside*
  `prepare_track`, before the start is even queued (the session can die at the
  first `prepare`/`reset`, and the thread sends `TrackStarted` microseconds
  later) — a pre-latch drain finds nothing, and the start that arrives during
  the join is discarded, so the erasure survives in precisely the "resident
  dies while a track is starting" case. It also consumes the queue: the
  non-start messages would have to be dropped or held, and `Returned` would
  have to be recovered, or the next run loses the ring producer.
- **"Hold the erased message and deliver it"** (this fix) covers both
  orderings: a start queued before the poll took the latch waits in the queue
  and the drain holds it; a start queued *during* the teardown is guaranteed
  present after the join, because `prepare_track` reads no control messages, so
  the decode thread cannot exit before sending its start. The delivery goes
  through the same `on_decode_out` arm the normal path uses (one code path,
  no duplicated publication logic), and it is placed after the join and before
  the freeze, so the event order is the natural one (`TrackChanged` →
  `StreamInfoChanged` → `PositionChanged` → `Error` → `StateChanged(Stopped)`
  → `PositionChanged`).

The failure contract is kept exactly:

- **The run stops** — `transport = Stopped`, unchanged.
- **The error is published once** — the delivery is a `TrackStarted` (no
  `Error` event), and a queued decode `Error` is still discarded by the drain;
  `handle_dsp_failure` still publishes the DSP error exactly once.
- **The timeline freezes at the failure** — `frozen_timeline` is computed
  before the delivery, and the `TrackStarted` arm touches no timeline origin,
  so `position_frames()` lands at `frozen_timeline - track_start`, the position
  inside the track that started.
- **The next run starts clean** — `Returned` is recovered by the drain exactly
  as before, the delivery never calls `flush_output` (the sticky
  `flush_failure` cannot be armed), `start_run` is untouched, and the held
  start is consumed by the delivery (no stale state persists; the other callers
  discard it).
- **The teardown ordering is intact** — `poll` is untouched (the latch is still
  handled first), `Stop` is still sent before the join, a queued `Stopped` is
  still only discarded (it never clears `to_decode` early), and the join still
  cannot hang (the decode thread services control messages while parked in
  `write_all`, `decode.rs:744`).

---

## 2. The user-visible change

A track that did start now reports its own duration: `duration_frames()`/
`duration_ms()`/`metadata()`/`source_spec()`/`stream()` are published for the
started track, and the `TrackChanged` event carries its `duration_ms`. Before
the fix, a resident death arriving immediately after a start left the window on
the new track's index with the previous track's duration (or none).

---

## 3. Tests

### 3.1 The two reds (both unchanged — no assertion weakened)

- `engine_fails_the_track_when_the_resident_dies` (`resident.rs:2813`): the
  fake's third-call crash races the start publication by design. With the fix
  the test passes whichever side wins the race: the start is published either
  by the normal poll path or by the failure teardown's delivery, and every
  assertion (the run stops before the track end, one `Error` naming the
  resident, `last_error()` set, the second run finishing naturally at exactly
  44 100) is satisfied either way. The call-count crash trigger is kept
  deliberately: it keeps this test exercising the erasure window (the latch
  before the first poll), which the new test now pins deterministically.
- `engine_gapless_seam_through_the_resident_chain_is_exact`
  (`resident.rs:2743`): its red was the same erasure symptom with the latch
  arriving before the queued start was processed (M25 §1.2, M26 §2 — the
  environmental blackout). The fix removes the erasure: the start is published
  even when the latch wins the race. The test's remaining expectations (both
  tracks buffer through the resident, the 6000-frame bit-exact recording, the
  natural finish) require a healthy resident; the failure contract is
  deliberately unchanged, so a latch still stops the run — now visibly at a
  later step (`wait_buffered`) instead of `wait_track(0)`. The residual
  environmental risk (a >30 s guest freeze) is M26's documented residual, not
  this wart.

### 3.2 The new test — the case the wart implies

`engine_publishes_the_start_when_the_resident_dies_right_after_it`
(`resident.rs:2882`): a failure immediately after a start leaves the engine's
reported duration consistent with the track it actually started —
deterministically, not as a race.

- The queue is `track7_2048.tst` (2048 frames — exactly two blocks, so the
  fake's third-call crash cannot fire during the first run) and
  `track2_44100.tst`, with an armed `fake:die` resident.
- A first run (with "stop after current" armed) publishes the first track's
  duration (`Some(2048)`) and ends before the second track opens — the stale
  duration the erasure would leave behind is exactly this one.
- The crash is then armed and the second track starts; the resident dies on
  its first block — immediately after its start was queued.
- The new `DspHarness::wait_latch` helper (`resident.rs:2663`) waits on the
  engine's failure watch — a condition, with the `WAIT` stall escape — and
  hands the latch back (take + re-latch; exact, because the chain is poisoned
  after its first failure so nothing can latch in between, and the engine is
  not polled meanwhile). The engine's polls are thereby held until the failure
  has latched, so the first poll deterministically finds the failure latched
  with the decode thread's start still unprocessed in the queue: the erasure
  window, not a coin flip. (The latch lands after the start was queued — the
  crash is on the first block, `prepare_track` sends the start first — so the
  start is guaranteed present.)
- Assertions: `duration_frames() == Some(44_100)` (the started track's, not
  the previous track's `Some(2048)`), `current_index() == Some(1)`, transport
  `Stopped`, the timeline frozen at the failure (`position_frames() == 0` —
  nothing was pumped), exactly one `Event::Error` naming the resident,
  `last_error()` set, the `TrackChanged` event for index 1 carrying
  `duration_ms == Some(1000)`, and the next run starting clean and finishing
  naturally at exactly 44 100.
- Without the fix this test fails deterministically: the teardown's drain
  discards the queued start and `duration_frames()` stays `Some(2048)`.

No sleep was introduced as an ordering mechanism: the 1 ms cadences are the
poll cadences the harness already used, and `wait_latch` waits on the watch's
latch (a condition) with the `WAIT` stall escape.

---

## 4. What only the VM can prove, and the exact command

The suite runs in the Hyper-V test VM; this lane ran no test. The orchestrator
should run the full suite there and check, per binary:

- `rustamp_audio` (lib): `engine_fails_the_track_when_the_resident_dies`,
  `engine_gapless_seam_through_the_resident_chain_is_exact` and the new
  `engine_publishes_the_start_when_the_resident_dies_right_after_it` green,
  and no other `rustamp-audio` test regressed (expected: full lib +
  `resident`/`replaygain`/`dsp_chain`/`engine` suites green).
- Everything else in the workspace unchanged (the fix is inside
  `rustamp-audio`'s engine; no other crate's code or tests were touched).

The standard suite command (from the checkout root, in the guest):
`cargo test --workspace --all-features` — or, scoped to the affected binary:
`cargo test -p rustamp-audio --all-features`. Per M26 §5, one green run is not
evidence for a race: at least a few consecutive green runs of the
previously-red binary are the bar. If a red appears, the failing line decides:

- The death test red at its first `wait_track(0)` would refute the fix's
  premise (the delivery did not publish the held start) — dump
  `duration_frames()`, `current_index()` and the drained events there.
- The gapless test red at `wait_track(0)` likewise; red at `wait_buffered` or
  a later step means a latch landed mid-run (the environmental blackout — the
  failure contract working as designed, now visible at the later step, not the
  erasure); a bit-exact mismatch would be a different defect entirely.
- The new test red at `duration_frames() == Some(44_100)` refutes the
  delivery; red at `wait_latch` means the resident's death did not latch.

---

## 5. Files and line counts

Counts include blank lines; all inside the owned paths. `git diff --stat`:

| File | Lines (after) | Change |
|---|---:|---|
| `crates/rustamp-audio/src/engine.rs` | 1513 (was 1482) | `join_decode` returns `Option<DecodeOut>` and holds the last queued `TrackStarted` instead of discarding it (`engine.rs:1447-1477`); `handle_dsp_failure` delivers it through `on_decode_out` after the join, before the freeze and the error publication (`engine.rs:1300-1302`, doc at `:1280-1285`); the other five callers discard the return (`stop` `:807`, `finish_run` `:1165`, `handle_output_error` `:1212`, `start_run` `:1338`, `Drop` `:1510`) — behavior unchanged. Production change. |
| `crates/rustamp-audio/src/resident.rs` | 3015 (was 2887) | `dsp_hosted_tests` only: `DspHarness` gains the engine's failure-watch field (`:2542`, set in `with_resident`); new `wait_latch` helper (`:2663`); new test `engine_publishes_the_start_when_the_resident_dies_right_after_it` (`:2882`). No production code in this file changed; no existing test or assertion changed. |
| `docs/tasks/A25-erasure-path.md` | this card | new |

No `Cargo.toml`/`Cargo.lock` change, no new dependency, no `THIRD_PARTY.md`
change. No file under `crates/rustamp-mediahost`, `crates/rustamp-dsphost`,
`crates/rustamp-dsp` or any other crate was touched.

Named hand-offs (same shape, deliberately not fixed here):

- `Engine::handle_output_error` (RFC 0012's output-failure teardown) has the
  same erasure shape (`poll` checks `stream_error` before the queue;
  `join_decode` discards a queued start). Not changed: its contract is a
  different RFC, no test pins the start there, and this card's scope is the
  resident failure. A follow-up lane may deliver the held start there too.
- `Engine::stop` has the same shape (a user stop with an unprocessed start
  erases it). Not changed: a stop is not a failure, and the task's scope is
  the failure path.
- The death test's fake keeps its third-call crash trigger in addition to the
  armed trigger (M25's design), so the death test still exercises the erasure
  window; the new test pins that window deterministically.

---

## 6. Gate receipts

Executed strictly within this worktree:

- `cargo fmt --all -- --check`: **PASS** (exit 0, no diffs).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **PASS** (exit 0, no warnings; compiles every edited test target — nothing
  was executed). One finding fixed on the way: `toggle_stop_after_current()`
  returns a `Result`, now `.unwrap()`ed like the neighbouring engine calls.
- `cargo build --workspace --offline`: **PASS** (exit 0).
- `cargo test`: **NOT RUN** — not the suite, not `--no-run`, not a single
  binary (Hyper-V orchestrator rule respected).
- No GUI window opened, no audio device accessed, no external dependency
  added, no git commit created, no process spawned (beyond the toolchain's own
  compile steps), no download.

---

## 7. Boundary receipt

`git status --short` inside this worktree (read-only) lists exactly the two
modified files plus this card:

```
 M crates/rustamp-audio/src/engine.rs
 M crates/rustamp-audio/src/resident.rs
?? docs/tasks/A25-erasure-path.md
```

Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or
`D:\code\rustamp-refbin` was read or written, and no file outside the owned
paths was modified.

---

## 8. Provenance

Implemented only from the task cards (M15, M19, M24, M25, M26, A8), the RFCs
they cite (RFC 0006 §6/§8; RFC 0012 read for contrast), and the clean
repository code (read-only outside the owned paths). No Winamp source, SDK
headers, disassembly or forbidden-tree contents were read. No third-party
code copied. License impact: none (no dependency change; `rustamp-audio`
stays GPL-3.0-or-later).
