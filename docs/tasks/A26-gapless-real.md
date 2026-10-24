# A26 — The gapless seam's real red: the unfenced `DspReset` lands after the new track's first block

**Date** 2026-10-18. **Lane** `wave77-gapless-real`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave77-gapless-real`.
Boundary rule: write ONLY inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

**Status:** diagnosis established at the mechanism level; the false doc claim
that misled A24 corrected in `rustamp-audio`; the fix handed to
`rustamp-mediahost` (+ the wire enum in `rustamp-ipc`) with exact file:line —
this lane's owned paths are `crates/rustamp-audio/**` only, so the fence itself
is named, not edited. fmt, clippy and build gates passing. VM test execution
pending orchestrator run.

No tests were run on the host (Hyper-V test-VM constraint strictly preserved):
not the suite, not `--no-run`, not a single binary. No test process, example,
player, helper binary or reference binary was spawned; no VM was touched; no
window or audio device was opened; no download, no dependency addition, no
install, no commit and no other git write. Only `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace --offline` ran (receipts in §7). Every command ran
inside this worktree.

The last full run reported the `rustamp_audio` lib binary at **81 passed / 1
failed**, the one red being
`resident::dsp_hosted_tests::engine_gapless_seam_through_the_resident_chain_is_exact`
failing in **three seconds** — every other red in the suite fails after the
thirty-second stall bound, the project's established marker of a guest freeze
under load. Cards read before touching anything: A22, A24, M19, M25, M26, A25
(plus M24's proxy-flake finding they cite).

---

## 0. Boundary & ownership

Owned paths: `crates/rustamp-audio/**`, and this card
(`docs/tasks/A26-gapless-real.md`). Touched:
`crates/rustamp-audio/src/resident.rs` (doc comments only — no code, no test,
no bound) and this card.

Read only (not modified): `crates/rustamp-audio/src/{engine.rs,decode.rs}`,
`crates/rustamp-mediahost/src/{control.rs,dsp.rs}`,
`crates/rustamp-ipc/src/{message.rs,shm/ring.rs}`,
`crates/rustamp-mediahost/tests/dsp_control.rs`,
`crates/rustamp-plughost/src/control.rs`, `crates/rustamp-mediahost/src/lib.rs`,
and the cards A22/A24/M19/M24/M25/M26/A25.

Sources: the task cards named above, RFC 0006 §6.2/§8, and the existing clean
repository code only. No Winamp source, SDK material, disassembly, research
tree or reference binary was read; no third-party code was copied; no
dependency was added.

---

## 1. The failing assertion

### 1.1 Which of the test's three claims fails

The test (`crates/rustamp-audio/src/resident.rs:2743`) asserts three things,
in this order:

1. **Both tracks buffer** — `wait_track(0)` (:2750) and `wait_buffered()`
   (:2751-2754).
2. **The seam is bit-exact** — pump 6000 frames and record (:2756-2763), then
   `assert_eq!(left.len(), 6000)` (:2765),
   `assert_eq!(&left[..3000], &expected(7, 0, 3000)[..])` (:2766),
   `assert_eq!(&left[3000..], &expected(2, 0, 3000)[..])` (:2767),
   `assert_eq!(left[2999], pattern(7, 2999))` (:2768),
   `assert_eq!(left[3000], pattern(2, 0))` (:2769),
   `assert!(!left.contains(&0.0), "the seam must not contain silence")` (:2770).
3. **The run finishes naturally** — `wait_pumping(Stopped, WAIT)` (:2772-2775),
   `assert_eq!(position_frames(), 3000)` (:2776),
   `assert_eq!(current_index(), Some(1))` (:2777),
   `assert!(drain_events().contains(&Event::PlaylistEnded))` (:2778).

**The failing claim is (2): the seam is not bit-exact.** The failing assertion
is `assert_eq!(&left[3000..], &expected(2, 0, 3000)[..])` at
`crates/rustamp-audio/src/resident.rs:2767` — the second track's half of the
recording — with the first mismatch at slice index 924: the recording holds
`0.0` where the pattern holds `pattern(2, 924)` (the pattern is in
[0.1, 1.1] and never zero — `pattern`, `resident.rs:2429-2435` — which is why
the no-silence assertion at :2770 exists and would also fail, later).

The same defect races the run's **first** track start as well: the reset sent
at `prepare_track(0)` can land the same way, corrupting track 1's frames
[924..1024] and failing :2766 first. The two races are independent coin flips
with the same mechanism (§1.3); which comparison fires first depends on which
race a given run loses. The single-track sibling
`engine_compensates_the_resident_latency_exactly` (`resident.rs:2705`)
exercises the identical first-start reset race and passed in the same run, so
the first start's race is the one that is usually won and the seam's is the one
that fails. §5 gives the VM command that settles it per run.

### 1.2 Why the three-second failure can only be the seam claim

- Every wait in the test carries the 30 s stall escape (`WAIT`,
  `resident.rs:1799`; `wait_track`/`wait_buffered`/`wait_pumping` at
  `:2646-2651`, `:2677-2679`, `:2613-2630`). A wait expiry costs at least
  thirty seconds; the observed failure costs three — the test runs to
  (near) completion and fails on a fast assertion.
- Claim (1) holds: `wait_track(0)` succeeds because the erasure is fixed
  (A25) and this test's chain is healthy — `fake:delay(100)` never latches, so
  `handle_dsp_failure` (engine.rs:1286) never runs; A25's production change is
  inert on this test's path. `wait_buffered` returning proves all 6000 frames
  are already in the engine ring: `EndOfStream` is sent by the decode thread
  only after the last write (`decode.rs:465-470`), and the engine processes
  the decode queue FIFO (`engine.rs:1001-1015`), so the recording loop cannot
  underrun and record silence.
- Claim (3) holds whenever the recording is exact: track 2's `TrackStarted`
  is queued before `EndOfStream`, so it is processed first — `current =
  Some(1)`, `track_start = 3000` (`engine.rs:1043-1070`); `finish_run` freezes
  the timeline at `frozen_end = 6000` (`engine.rs:1159`, `1172`), so
  `position_frames() = 6000 - 3000 = 3000` (`engine.rs:449-465`); and
  `PlaylistEnded` fires because `current + 1 >= queue.len()`
  (`engine.rs:1188-1194`).
- The frame count also survives the corruption (§1.3 step 7), so :2765 passes
  and the failure is the value comparison.

### 1.3 The mechanism, end to end

The chain is one `fake:delay(100)` stage: a 100-frame look-ahead —
`FakeChain`, `Kind::Delay` (`resident.rs:1906-1914`): every output frame is
the input frame from 100 frames ago; `reset` clears the line to zeros
(`resident.rs:1939-1942`). The reported latency is 100 (the EQ and the
disabled limiter report 0), so `dsp_latency() == 100`.

1. **Track 1 ends.** `flush_dsp` pushes `dsp_latency()` = 100 frames of silence
   through the chain (`decode.rs:705-717`), emitting track 1's held tail. The
   delay line is left holding 100 zeros — its initial state.
2. **Track 2 starts.** `prepare_track` (`decode.rs:536-580`) calls
   `stage.prepare(src)` on every stage (`:544-546`). The resident chain's
   `ChainControl::prepare` takes the same-format path (`wire ==
   session.spec()`, `resident.rs:793`) and calls `self.reset(session)`
   (`:794`) → `ChainControl::reset` (`resident.rs:835-845`) →
   `DspSession::reset` (`crates/rustamp-mediahost/src/dsp.rs:577-580`): a bare
   `peer.send(Message::DspReset)`, documented at `dsp.rs:573-576` as *"not an
   acknowledgement or an ordering fence for writes on the independent PCM
   channel."*
3. `prepare_track` re-arms `dsp_trim = dsp_latency() = 100`
   (`decode.rs:547`), sends `TrackStarted` (`:570`), and `track_loop`
   reads the first 1024-frame block (`decode.rs:596-657`); `emit` runs it
   through the chain (`decode.rs:670-700`).
4. **The race.** The control path from `send` to pump-visibility crosses three
   more threads than the PCM path: client writer thread
   (`control.rs:105-120`) → byte stream → host reader thread
   (`control.rs:95-104`) → host monitor thread (`control.rs:128-221`,
   `incoming_tx.try_send` at `:202`) → the pump's `poll_commands`
   (`dsp.rs:125-137`). The PCM path is one hop: decode thread → ring → the
   pump's `read_into_blocking` (`dsp.rs:181`). The pump applies `DspReset` at
   its control polls — top of loop (`dsp.rs:176`), after reading a block and
   *before* processing it (`dsp.rs:192`, commented "Apply it before this first
   new block touches state"), and during write-back (`dsp.rs:198`). Whether
   the reset is visible at the pre-process poll is a race between the
   three-hop control path and the decode thread's remaining prepare work
   (trim re-arm, two buffer allocations, the `TrackStarted` send, the decoder
   read — `decode.rs:547-579` — plus the gain/EQ/limiter passes, `:677-681`)
   and one wake. The pump's pre-process poll narrows the race; it does not
   close it.
5. **When the control path loses** (the guest's scheduler), the pump processes
   track 2's first block with the line still holding the flush zeros — output
   `[100 zeros, track2[0..924]]`; the trim drops the first 100 frames
   (`decode.rs:682-684`), 924 real frames are written, `dsp_trim` is consumed —
   and the reset lands at a later poll, when the line holds track 2's **real**
   frames `track2[924..1024]`. `FakeChain::reset` clears them to zeros.
6. **Track 2's second block** is then processed through the cleared line:
   output `[100 zeros, track2[1024..1948]]`. `dsp_trim` is already consumed,
   so the ring receives 100 frames of silence at track 2's frames [924..1024]
   and the 100 real frames are gone. The frame count per track is still 3000
   (the end-of-track flush still emits the tail), so `:2765` and the lengths
   pass and the boundary samples `:2768-2769` are intact — but the second half
   is not the pattern.
7. **The recording:** `left[3924..4024] == 0.0` where `expected(2, 924..1024)`
   lies in [0.1, 1.1]. With the control path a few thread-wakes behind the PCM
   path, the reset lands between the first and second block — the common
   case; a landing at any later block boundary inside track 2 corrupts a later
   window and fails :2767 at that index instead. A landing inside track 1
   fails :2766 first (§1.1).

The other engine tests do not hit this: the death and wedge tests' chains are
`fake:die`/`fake:slow` (latency 0 — no hold-back frames to destroy), the seek
test's drain gives the control path milliseconds of slack, and the
single-track test carries the race once instead of twice.

### 1.4 Why the earlier verdicts were wrong

- **A22** removed the 30 ms sleep from `ChainControl::reset` arguing the
  protocol is "robust to the race by construction (a stale prefix lands inside
  the trimmed latency window)". The trim bounds the *stale prefix* — the flush
  zeros the chain emits at the start of the new track. It does not bound the
  reset's *clearing* of real held frames later in the track. The sleep was
  load-bearing for exactly this seam: it gave the control path time to land
  while the line still held only flush zeros (a no-op), before the first real
  block. A22's own §4 named the real fix — a session-level acknowledgement
  owned by `rustamp-mediahost` — but treated it as a probe-visibility nicety,
  not a correctness requirement.
- **A24 §1.2** "proved" the sleep removal was not the cause, with two
  arguments, both wrong:
  - *"the `fake:delay(100)` line holds zeros at the seam however late the
    reset lands — the reset is a no-op on zeros either way."* True only until
    the first real block of the new track is processed. "However late" is
    exactly the case not covered: after block 1 the line holds the new track's
    real frames, and clearing them is destructive, not a no-op.
  - *"the passing seek test exercises the identical async-reset race with
    nonzero state yet stays green — a late reset would break the seek before
    it broke this seam."* Backwards. The seek path's drain (`await_drain`,
    `decode.rs:740-741` — required to discard pre-seek audio) delays the first
    post-seek block by many milliseconds, giving the three-hop control path
    ample time to land; the gapless seam has no drain between the flush and
    the first block. The seek test's green is evidence of the drain's slack,
    not of the race's absence.
- A24's observed failure (the final pump-count wait) was the M24-class proxy
  flake, not evidence about the seam. The waves after it (M25's liveness trip
  wire, M26's wall-clock sweep, A25's erasure fix) repaired the *earlier*
  failures, which is why the seam failure is visible only now: the test finally
  runs past `wait_track(0)` and reaches the recording.

---

## 2. The fix — handed to `rustamp-mediahost` (+ `rustamp-ipc`), named, not edited

The cause is the missing delivery fence on the `DspReset` path: the adapter
must deliver the `DspStage` contract (fresh state at `prepare`, like the
in-process stages) but the resident applies the reset asynchronously, at an
arbitrary block boundary. No honest fix exists inside `rustamp-audio`:

- **A sleep is forbidden** (owner rule) and would only narrow the race — the
  very mistake A20 made.
- **Skipping the reset at the seam** diverges from the in-process chain (the
  engine's own EQ and limiter leave `prepare` with fresh state) and re-opens
  the stateful-plugin seam click the reset exists to prevent.
- **A same-format reconfigure** would be fresh and synchronous but churns both
  rings per track start — the pinned contract of
  `dsp_prepare_with_the_same_format_clears_state_without_reconfigure`
  (`resident.rs:2299`, "a same-format prepare never touches the rings") forbids
  it, and weakening that claim is out.
- **A probe cannot detect the landing:** at the seam the flush has already
  left the chain in its initial state, so the reset's output is identical
  whether it has landed or not. The adapter-level probes
  (`RESET_SETTLE_BLOCKS`, `resident.rs:1820`, `:2257-2266`, `:2322-2331`)
  tolerate the race only because their synthetic blocks make the stale tail
  visible; the engine seam carries real audio and cannot re-probe.

So the fix is the **synchronous reset acknowledgement** — the "true delivery
fence" A22 §4 named — owned by `rustamp-mediahost` with the wire variant in
`rustamp-ipc`:

1. `crates/rustamp-ipc/src/message.rs:240` — add a `Message::DspResetAck` unit
   variant next to `DspReset` (enum at `:77`). A wire change: per
   CONTRIBUTING §contract changes it needs a short RFC in `docs/rfc/` first.
   Matches to update: `crates/rustamp-mediahost/src/lib.rs:155` (stub arm;
   the match has a catch-all at `:159`, add an explicit arm for clarity) and
   optionally the foreign-message array in
   `crates/rustamp-plughost/src/control.rs:318-326`.
2. `crates/rustamp-mediahost/src/dsp.rs:128` — in `poll_commands`, after
   `Message::DspReset => chain.reset()`, send the ack
   (`peer.send(Message::DspResetAck)`). This one site covers all three pump
   poll sites (`dsp.rs:176`, `:192`, `:198`). Same at the post-EOS idle loop,
   `dsp.rs:368`.
3. `crates/rustamp-mediahost/src/dsp.rs:577-580` — `DspSession::reset`: after
   `peer.send(Message::DspReset)`, wait bounded (the session's block timeout /
   `CONTROL_WAIT`) for the ack via `peer.recv(...)`; a timeout is a loud
   `MediaHostError::TimedOut`, never a silent pass. Update the doc at
   `dsp.rs:573-576` from "not an acknowledgement" to the fenced semantics. The
   ack must be consumed by this wait and never left in the queue to trip the
   "unexpected DSP message while writing/reading" rejections (`dsp.rs:497-500`,
   `:536-539`).
4. With the fence landed, `ChainControl::reset`
   (`crates/rustamp-audio/src/resident.rs:835-845`) needs **no code change**:
   `session.reset()` returns only after the chain is cleared, so the seam is
   exact by construction — the reset lands while the line still holds flush
   zeros (a no-op), before the first real block. The seek path and the
   adapter-level probe tests keep working: the probes observe the cleared
   state, and the fenced reset makes the first probe succeed.
5. The engine seam test (`resident.rs:2743`) then passes **unchanged** — no
   assertion, wait, or bound is touched by this lane, and the mediahost lane
   should not touch one either.

## 3. The test's expectation is right (no claim weakened, nothing re-pinned)

The seam's bit-exactness is the `DspStage` contract: `prepare` leaves fresh
state exactly as the in-process stages do (M19 choice #6), and RFC 0006 §6.2's
trim/flush compensation assumes it. No contract changed under the test — the
adapter violates the contract whenever the reset lands late. Nothing in this
lane weakens a claim, widens a bound, adds a sleep, or re-pins an expectation.

## 4. What this lane changed in `rustamp-audio` (doc only)

- `crates/rustamp-audio/src/resident.rs` — `ChainControl::reset`'s doc
  (`:823-834`) corrected: the false claim "the decode protocol is robust to
  the race by construction (a stale prefix lands inside the trimmed latency
  window)" is replaced by the A26 finding (a late reset clears real held
  frames and injects `latency` silence outside the trim window; the seek
  survives only via its drain) plus the named hand-off;
  `ChainControl::prepare`'s doc (`:774-779`) gained a cross-reference to it.
  The false comment is what let A24's refutation stand; it now states the
  truth at the exact call site `prepare_track` drives.
- No production logic, test, assertion, wait, or bound changed. No
  `Cargo.toml`/`Cargo.lock` change, no new dependency, no `THIRD_PARTY.md`
  change. No file under `crates/rustamp-mediahost`, `crates/rustamp-ipc`,
  `crates/rustamp-dsphost`, `crates/rustamp-dechost`, `crates/rustamp-dsp` or
  any other crate was touched.

## 5. What only the VM can prove, and the exact command

1. **Confirm the diagnosis** — the one thing static analysis cannot decide:
   which of the test's two track-start races is lost in a given run, and the
   corruption's exact position. Run the single test in the guest with the
   output captured:

   `cargo test -p rustamp-audio --all-features --lib -- resident::dsp_hosted_tests::engine_gapless_seam_through_the_resident_chain_is_exact --exact --nocapture`

   The panic names the failing line (`:2766` or `:2767`) and prints left/right;
   a first mismatch at slice index 924 (`left[3924] == 0.0` in the seam case)
   confirms the injected 100-frame silence at track 2's frames [924..1024].
   Repeat two to three times: the failing line may move between `:2766` and
   `:2767` (both races are live) — that movement is itself the diagnosis.
2. **After the mediahost lane lands the fence:** the full suite in the guest,
   from the checkout root:

   `cargo test --workspace --all-features`

   scoped: `cargo test -p rustamp-audio --all-features`. Per M26 §5, the bar is
   **at least 5 consecutive green full-suite runs** with the previously-red
   binary green in every one (the race is probabilistic; one green run is not
   evidence), plus 2–3 extra repeats of the `rustamp_audio` lib binary. The
   guest also proves the fence's bounded wait always suffices on its scheduler
   and that the adapter-level probe tests and the seek test stay green with
   the fenced reset.

## 6. Files and line counts

Counts include blank lines; all inside the owned paths. `git diff --stat`:

| File | Lines (after) | Change |
|---|---:|---|
| `crates/rustamp-audio/src/resident.rs` | 3032 (was 3015) | `ChainControl::reset` doc corrected (false "robust by construction" claim replaced by the A26 finding + the named mediahost hand-off); `ChainControl::prepare` doc cross-reference. Doc comments only — no code, test, bound or assertion changed. |
| `docs/tasks/A26-gapless-real.md` | this card | new |

## 7. Gate receipts

Executed strictly within this worktree:

- `cargo fmt --all -- --check`: **PASS** (exit 0).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **PASS** (exit 0, no warnings; compiles every edited target — nothing was
  executed).
- `cargo build --workspace --offline`: **PASS** (exit 0).
- `cargo test`: **NOT RUN** — not the suite, not `--no-run`, not a single
  binary (Hyper-V orchestrator rule respected).
- No GUI window opened, no audio device accessed, no external dependency
  added, no git commit created, no process spawned beyond the toolchain's own
  compile steps, no download.

## 8. Boundary receipt

`git status --short` inside this worktree (read-only) lists exactly the two
files above:

```
 M crates/rustamp-audio/src/resident.rs
?? docs/tasks/A26-gapless-real.md
```

Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or
`D:\code\rustamp-refbin` was read or written, and no file outside the owned
paths was modified.

## 9. Provenance

Implemented only from the task cards (A22, A24, M19, M24, M25, M26, A25), the
RFCs they cite (RFC 0006 §6.2/§8), and the clean repository code (read-only
outside the owned paths). No Winamp source, SDK headers, disassembly or
forbidden-tree contents were read. No third-party code copied. License
impact: none (no dependency change; `rustamp-audio` stays GPL-3.0-or-later).
