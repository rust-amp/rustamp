# M29 — Wall-clock sweep: finish the remaining 5 s bounds, match fixture liveness, and convert pump proxies

**Date** 2026-10-18. **Lane** `wave85-liveness-rest`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave85-liveness-rest`.

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

---

## 0. Boundary & ownership

Owned paths: `crates/rustamp-audio/tests/**`, `crates/rustamp-mediahost/examples/**`,
`crates/rustamp-mediahost/tests/**` (only where the same class remains), and this card.
Touched (all test/example surface; no production code):

- `crates/rustamp-mediahost/examples/decoder_fixture.rs`
- `crates/rustamp-mediahost/examples/dsp_fixture.rs`
- `crates/rustamp-audio/tests/flush_contract.rs`
- `crates/rustamp-audio/tests/render_rt.rs`
- `crates/rustamp-audio/tests/common/mod.rs`
- `crates/rustamp-audio/tests/engine.rs`
- `docs/tasks/M29-liveness-rest.md` (this card)

Read only (not modified): `crates/rustamp-audio/src/**`, `crates/rustamp-mediahost/src/**`,
`crates/rustamp-mediahost/tests/**`, `crates/rustamp-audio/tests/resident.rs`,
`crates/rustamp-audio/tests/{dsp_chain,limiter_gain,output_error,replaygain}.rs`,
and the cards M10, M20, M22, M24, M25, M26, `docs/HANDOFF.md`.

Sources: the task cards named above, RFCs 0003/0006/0007 (as cited by them),
and the existing clean repository code only. No Winamp source, SDK material,
disassembly, research tree or reference binary was read; no third-party code
was copied; no dependency was added.

---

## 1. Changes applied per named place

### 1.1 `crates/rustamp-mediahost/examples/{decoder_fixture,dsp_fixture}.rs` (Child-side liveness policy)

- **Previous state:** Both fixture examples started `ControlPeer` with `Liveness::default()`
  (100 ms interval, **5 s timeout**). M26 §3 identified this child-side asymmetry: while
  the parent tests (`pcm_process.rs`, `dsp_process.rs`) had moved to a 30 s `LIVENESS` const,
  the child fixtures spawned into their own process still retained the 5 s trip wire.
- **Change:** Introduced an explicit `LIVENESS` const (`interval: 100 ms`, `timeout: 30 s`)
  matching the parent test peer. If the guest freezes under load for >5 s, the child will
  no longer drop the connection due to a missed Pong. A dead parent process closes
  stdin/stdout and is caught immediately by EOF regardless.

### 1.2 `crates/rustamp-audio/tests/flush_contract.rs` (Stall escape bound)

- **Previous state:** `const WAIT: Duration = Duration::from_secs(5);` at line 21, used
  in `Harness::wait` (`let deadline = Instant::now() + WAIT;`) and in
  `Harness::drive_to_seek_result` (`let deadline = Instant::now() + WAIT;`).
- **Change:** Changed `WAIT` from 5 s to 30 s (`Duration::from_secs(30)`) with a doc comment
  explaining the stall escape rule. Both wait loops assert `Instant::now() < deadline`
  loudly on timeout naming the step (`"condition did not settle"`, `"seek never reached decoder"`).
  No fast timeout assertion exists in this file; all test claims are exact buffer/state checks.

### 1.3 `crates/rustamp-audio/tests/render_rt.rs` (Ring priming deadline)

- **Previous state:** Line 79 used an inline `let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);`
  while waiting for `h.engine.buffered_frames() == 0`, asserting `"the ring must be primed"`.
- **Change:** Changed the 5 s deadline to 30 s (`Duration::from_secs(30)`), and updated the
  assertion message to loudly name the step: `"stall: the ring was never primed within 30s"`.
  The deliberate allocation checks (`soak == 0`, `tap_allocations == 0`) and the 50 ms pause
  before entering the counting zone remain intact.

### 1.4 `crates/rustamp-audio/tests/common/mod.rs` (Shared Harness wait helpers)

- **Previous state:**
  - `wait_track`: hardcoded `Duration::from_secs(5)`.
  - `wait_buffered`: hardcoded `Duration::from_secs(5)`.
  - `settle_seek`: hardcoded `Duration::from_secs(5)`.
  - `settle_drain`: hardcoded `Duration::from_secs(5)`.
  - `wait_refilled`: hardcoded `Duration::from_secs(5)`.
  - `wait_until`: only provided a pump-count proxy (`pumped < max_frames`), with no condition-based
    wall-clock escape.
- **Change:**
  - Added `pub const WAIT: Duration = Duration::from_secs(30);` documented with the M10/M24/M25/M26 rule.
  - Added `Harness::wait_pumping(&mut self, mut cond: impl FnMut(&Engine) -> bool, timeout: Duration) -> bool`,
    which measures the condition directly and pumps 256-frame steps with a 1 ms poll cadence until the
    condition holds or `timeout` elapses (matching the helper in `tests/resident.rs`).
  - Marked `wait_until` as deprecated in favor of `wait_pumping`.
  - Updated `wait_track`, `wait_buffered`, `settle_seek`, `settle_drain`, and `wait_refilled` to use
    `WAIT` (30 s).

### 1.5 `crates/rustamp-audio/tests/engine.rs` (Deadlines, pump proxies, and ordering sleeps)

- **Previous state:**
  - 12 call sites used `wait_until(..., total + 8192)` or `wait_until(..., track_len + 8192)` as
    arithmetic pump-count proxies to wait for `transport() == TransportState::Stopped`.
  - 6 call sites used short wall-clock bounds: lines 244 (2 s) and 1605 (2 s) in `wait_idle`, line 323 (3 s)
    in `vis_tap_is_filled_without_the_consumer_running`, line 1865 (5 s) in `seek_before_play_plays_the_target_audio`,
    and 5 inline 5 s deadlines in `output_flush_on_stop_discards_buffered_frames_and_restart_drains_silent`
    (lines 2322, 2360, 2370, 2384, 2405).
  - 2 call sites used sleeps as ordering mechanisms: line 1336 (`std::thread::sleep(Duration::from_millis(50))`)
    hoping the decode thread had buffered frames, and line 2074 (`std::thread::sleep(Duration::from_millis(20))`)
    hoping drain and start handshakes were pending.
- **Change:**
  - Imported `WAIT` from `common`.
  - Converted all 12 `wait_until` pump-count proxies to `wait_pumping(|e| e.transport() == TransportState::Stopped, WAIT)`:
    - lines 536, 576, 596 (`stop_after_current` tests)
    - lines 771, 790 (`dsp_stage_survives_across_track_and_engine_restarts`)
    - lines 879, 927 (`underrun_at_end_of_stream...`, `normal_run_finishes...`)
    - lines 1031, 1057 (`second_run_resets_consumed...`)
    - line 1308 (`stop_while_stopped_at_end_of_track...`)
    - lines 2104, 2168 (`a_restart_after_a_natural_end...`, `natural_end_flushes...`)
    Removed now-unused local `total` variables to keep clippy warnings clean.
  - Converted all short stall deadlines to `WAIT` (30 s), with loud failure messages naming the step:
    - lines 244 and 1605 (`wait_idle(Stopped, WAIT)`)
    - line 323 (`vis_tap_is_filled...` deadline changed to `WAIT` with `expect("stall: no vis frame was published within WAIT")`)
    - line 1865 (seek drain deadline changed to `WAIT`, still asserting `"the drain never happened"`)
    - lines 2322, 2360, 2370, 2384, 2405 (all 5 deadlines in `output_flush_on_stop...` now use `WAIT`,
      retaining their exact named assertion panics: `"stall: track 1 never opened"`, `"stall: track 2 never opened"`,
      `"stall: the ring never refilled after the restart"`, `"the start drain never settled"`,
      `"stall: the decode thread wrote nothing after the start drain"`).
  - Replaced the two ordering sleeps with condition waits:
    - line 1336: replaced 50 ms sleep with `assert!(h.wait_idle(|e| e.buffered_frames() > 0, WAIT), "stall: decode thread never buffered frames in ring")`.
    - line 2074: replaced 20 ms sleep with `assert!(h.wait_idle(|e| e.is_drain_pending() && e.is_start_pending(), WAIT), "stall: drain and start were never pending after restart")`.

---

## 2. What was deliberately left fast and why

Every deliberate fast bound that tests a timeout or latency claim was preserved:

1. **`crates/rustamp-mediahost/tests/pcm_transfer.rs:440`**:
   `read(&mut out, Some(Duration::from_millis(20)))` in
   `read_timeout_blocks_only_for_the_requested_duration_and_is_not_eos`:
   The test explicitly asserts that a 20 ms read timeout returns `TimedOut` in ~20 ms without EOS.
   Widening this would invalidate the promptness claim.
2. **`crates/rustamp-mediahost/tests/pcm_transfer.rs:498` & `:526`**:
   `missing_pong_is_bounded_even_with_an_open_control_stream`:
   Configures an explicit 200 ms liveness policy (`Duration::from_millis(200)`) and asserts
   `start.elapsed() < Duration::from_secs(5)` to prove missing Pong detection is prompt.
   Kept at 5 s explicitly.
3. **`crates/rustamp-mediahost/tests/{pcm_named,dsp_named}.rs`**:
   `write_from_blocking(..., Some(Duration::from_secs(1)))`:
   Direct in-process write into a newly created empty shared memory ring. Completes synchronously
   in microseconds; 1 s is an ample ceiling for an in-memory operation.
4. **`crates/rustamp-mediahost/tests/pcm_process.rs:102`**:
   `result.recv_timeout(Duration::from_secs(10))`:
   In `host_exits_mid_stream_and_infinite_read_survives`, waiting on an mpsc channel where the child
   process terminates abruptly. The pipe EOF triggers immediately on process exit.
5. **Mock Device Latency (`crates/rustamp-audio/tests/engine.rs:2240`)**:
   `fn latency(&self) -> Duration { Duration::from_millis(10) }`:
   Returns simulated device latency for `OutputDevice`, not a wait deadline.
6. **Poll Cadences (1 ms sleeps)**:
   All `std::thread::sleep(Duration::from_millis(1))` across wait loops in `common/mod.rs`,
   `engine.rs`, `flush_contract.rs`, and `render_rt.rs`:
   These are polling tick intervals between engine polls, not ordering mechanisms.

---

## 3. Assertions kept (none weakened)

- Every functional condition is untouched: exact waveform and sample patterns, bit-exact comparisons,
  event sequencing, transport state transitions, ring counter invariants, stream info, ReplayGain values.
- All wait timeouts remain loud failures: expiry panics with the specific step named; never a silent pass.
- `wait_until` pump-count proxies are replaced by condition waits (`wait_pumping`), which assert the
  exact same condition (`transport() == Stopped`) without relying on an arithmetic proxy that burns
  steps during underruns or start drains.
- Sleeps that previously served as ordering mechanisms (waiting 20 ms or 50 ms hoping for state updates)
  are now explicit condition waits with generous stall escapes.

---

## 4. What only the VM can prove

The test suite executes inside the 4-vCPU Hyper-V test VM; no tests were run on the host.
The orchestrator should run:
`cargo test -p rustamp-audio --test engine --test flush_contract --test render_rt`
and the full suite:
`cargo test --workspace --all-features`

Expected outcome:
- All `rustamp-audio` integration test binaries (`engine`, `flush_contract`, `render_rt`, `resident`,
  `output_error`, `replaygain`, `dsp_chain`, `limiter_gain`) pass reliably under load.
- Any failure tripping at 5 s in these binaries is now eliminated. A failure at 30 s indicates a systemic
  guest freeze (>30 s), not a test-side wall-clock defect.

---

## 5. Files and line counts

Line counts after changes (`git diff --stat`):

```
 crates/rustamp-audio/tests/common/mod.rs           |  43 ++++++++--
 crates/rustamp-audio/tests/engine.rs               |  97 ++++++++--------------
 crates/rustamp-audio/tests/flush_contract.rs       |   7 +-
 crates/rustamp-audio/tests/render_rt.rs            |   7 +-
 crates/rustamp-mediahost/examples/decoder_fixture.rs  |  19 +++--
 crates/rustamp-mediahost/examples/dsp_fixture.rs   |  19 +++--
 docs/tasks/M29-liveness-rest.md                    | 259 ++++++++++++++++++++++
 7 files changed, 367 insertions(+), 84 deletions(-)
```

File line totals:
- `crates/rustamp-audio/tests/common/mod.rs`: 574 lines (was 544)
- `crates/rustamp-audio/tests/engine.rs`: 2395 lines (was 2425)
- `crates/rustamp-audio/tests/flush_contract.rs`: 483 lines (was 479)
- `crates/rustamp-audio/tests/render_rt.rs`: 122 lines (was 120)
- `crates/rustamp-mediahost/examples/decoder_fixture.rs`: 30 lines (was 24)
- `crates/rustamp-mediahost/examples/dsp_fixture.rs`: 31 lines (was 25)
- `docs/tasks/M29-liveness-rest.md`: this card (new)

---

## 6. Gate receipts

Executed strictly within this worktree:

- `cargo fmt --all -- --check`: **PASS** (exit 0).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **PASS** (exit 0).
- `cargo build --workspace --offline`: **PASS** (exit 0).
- `cargo test`: **NOT RUN** — not the suite, not `--no-run`, not a single binary (Hyper-V orchestrator rule respected).
- No GUI window opened, no audio device accessed, no external dependency added, no git commit created, no process spawned.

---

## 7. Boundary receipt

`git status --short` inside this worktree (read-only) lists exactly the modified files and this card:

```
 M crates/rustamp-audio/tests/common/mod.rs
 M crates/rustamp-audio/tests/engine.rs
 M crates/rustamp-audio/tests/flush_contract.rs
 M crates/rustamp-audio/tests/render_rt.rs
 M crates/rustamp-mediahost/examples/decoder_fixture.rs
 M crates/rustamp-mediahost/examples/dsp_fixture.rs
?? docs/tasks/M29-liveness-rest.md
```

Nothing under `D:\code\Rustamp`, `D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written,
and no file outside the owned paths was modified.

---

## 8. Provenance

Implemented only from the task cards (M10, M20, M22, M24, M25, M26), the RFCs they cite,
and the clean repository code. No Winamp source, SDK headers, disassembly or forbidden-tree
contents were read. No third-party code copied. License impact: none (no dependency change).
