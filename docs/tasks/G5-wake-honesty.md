# Task card G5 — Make the wake-timeout claim honest (W10 finding F7)

> **Orchestrator notes (2026-10-18).**
> Settles W10 finding F7 against task card G3: the wake-timeout measurement
> did not settle the question it claimed to settle. The G3 claim is narrowed,
> the mechanism's own timing contract is pinned by a deterministic test, the
> OS behaviour is measured on the host with a reproducible probe, and what
> only the VM or real hardware can add is stated plainly.

**Goal**

1. Establish exactly what the G3 test and card claimed, with file:line, and
   where the claim is weaker than it sounds: the relaxed bound, the tick
   description, the missing artefact.
2. Make the claim honest: measure what can actually be measured here with a
   bound that follows from the mechanism, and narrow the claim to what the
   measurement supports, saying plainly what remains unmeasured and what
   would measure it.
3. Keep the existing cross-process wake checks (`cargo xtask wake-cross`)
   working, and say what only the VM or real hardware could add.

**Lane** G — plugin host / IPC (`crates/rustamp-ipc/`, RFC 0007 in `docs/rfc/`).

**Owned paths**

- `crates/rustamp-ipc/src/shm/wake.rs` (the mechanism's timing contract
  extracted into `timeout_ms` and pinned by a test; no behaviour change)
- `crates/rustamp-ipc/src/shm/ring.rs` (timeout tests: honest doc comments,
  one rename; no assertion bound changed)
- `crates/rustamp-ipc/examples/wake_timeout_probe.rs` (new probe)
- `docs/tasks/G3-wake-timeout.md` (claim narrowed)
- `docs/tasks/G5-wake-honesty.md` (this card)

Not owned, not touched: `xtask/**` (the G4 lane's harness),
`crates/rustamp-audio/**`, `crates/rustamp-mediahost/**`, every other crate,
`docs/HANDOFF.md`, `docs/tasks/W10-batch-review.md`,
`docs/tasks/G2-ipc-blocking.md`, `docs/tasks/BETA-*.md`,
`docs/tasks/W11-docs-sweep.md`.

**Allowed sources**

- `docs/rfc/0007-blocking-consumer.md` (blocking consumer contract)
- `docs/rfc/0003-shm-rings.md` (shared-memory rings)
- `crates/rustamp-ipc/src/shm/**`, `crates/rustamp-ipc/examples/**`,
  `xtask/src/wake_cross.rs` (read only)
- `docs/tasks/G2-ipc-blocking.md`, `docs/tasks/G3-wake-timeout.md`,
  `docs/tasks/G4-wake-cross.md`, `docs/tasks/W10-batch-review.md`
- Public Win32 API documentation (`WaitForSingleObject`,
  `NtQueryTimerResolution`, Windows system timer resolution)

**Forbidden sources**

- Winamp source code, SDK headers, disassembly, dirty references.
- No network access, no downloads, no new dependencies, no `cargo add`.
- No `cargo test` (the suite runs in the Hyper-V VM; only the orchestrator
  runs it). The VM was not run.

## 1. The claim as written, and what it really rests on

Line numbers below are from the tree as of `94f1ffa` (before this lane).

| # | Claim | Where | What it really rests on |
|---|---|---|---|
| C1 | "In the VM run, `read_into_blocking` with a 10 ms timeout returned `Ok(0)` in ~7.8 ms." | `docs/tasks/G3-wake-timeout.md:80` | A single VM observation. No run log, capture or manifest for it exists anywhere in the tree (W10 G3-5: FALSE in-tree). It cannot be reproduced or checked from the repository. |
| C2 | "Verdict: The wait implementation is correct; the return in ~7.8 ms is expected Windows OS behavior." | `G3-wake-timeout.md:81` | Asserted, not demonstrated. The documented Win32 caveat supports "may time out in less than the specified length" for sub-clock-resolution intervals, but nothing in-tree shows that the VM kernel does so, or by how much. |
| C3 | "the kernel timer interrupt fires at the next tick boundary (~7–8 ms later), evaluating the timer as expired" | `G3-wake-timeout.md:82-89`; repeated in the test doc comment at `ring.rs:1824-1828` | Self-inconsistent: a tick boundary 7–8 ms after the call *precedes* the 10 ms expiry, so a 10 ms timer cannot be evaluated as expired at that boundary (W10 G3-4). |
| C4 | "asserts `elapsed >= 1 ms`, proving the thread entered the kernel wait state" (for the 10 ms case) | `G3-wake-timeout.md:96-98`; test `blocking_timeout_accuracy_across_durations` at `ring.rs:1871-1918` | True but weak: for a 10 ms request the only lower-bound assertion is `>= 1 ms`, so an early return of up to 9 ms passes. The `>= 8 ms` bound that failed in the VM was relaxed to `>= 1 ms` (`ring.rs:1840-1843`) rather than the behaviour established (W10 G3-2, G3-3). |
| C5 | "Multi-tick durations are honored within the physical resolution of the operating system clock" | `G3-wake-timeout.md:111` | Not measured: the multi-tick bound is `requested − 16 ms` — an allowance, not a measurement — so a 15 ms early return at 30 ms also passes. |
| C6 | "Settle the open question … with measurements" (goal; echoed as "the wake timeout question is settled by measurement" in `docs/HANDOFF.md:233`) | `G3-wake-timeout.md:11`, `docs/HANDOFF.md:233` | The measurement pins the code's behaviour (kernel entry, outcome, no hang) but cannot separate an early kernel return from an implementation bug — a property of the OS contract, not a defect the test can fix (W10 F7). |

The mechanism itself (`wake.rs:135-166`, `ring.rs:908-953`): the code returns
`Ok(0)` / `TimedOut` only when the kernel reports `WAIT_TIMEOUT`; the only
thing the implementation controls about wait timing is the millisecond value
it hands to `WaitForSingleObject` (round up to whole ms, positive sub-ms →
1 ms, `ZERO` → poll, `None` → `INFINITE`, RFC 0007 §6.4). Until this lane
that mapping was prose only.

## 2. What the mechanism guarantees (the honest core)

1. **The ms mapping** (`wake.rs:145`, `timeout_ms`): exact, deterministic,
   pure. This is the bound that follows from the mechanism, and it is now
   pinned by a test.
2. **No early return of its own**: `Ok(0)` / `TimedOut` only on the kernel's
   `WAIT_TIMEOUT`. The check-then-wait loop re-waits with the full timeout
   after a signal, so a signal can only add time, never subtract it
   (G3 card, "Spurious signals").
3. **No hang**: bounded by the requested timeout plus scheduling jitter.
4. **The OS decides the rest**: the documented caveat allows a wait shorter
   than the system clock resolution to expire early. How early, on which
   system, is not the implementation's to guarantee — and not measurable by
   a wall-clock assertion in a test that must stay green everywhere it runs.

## 3. What was measured here (host)

**Method (committed, reproducible):**
`crates/rustamp-ipc/examples/wake_timeout_probe.rs`, run as
`cargo run -p rustamp-ipc --example wake_timeout_probe`. It drives the real
mechanism — `RingConsumer::read_into_blocking` on an empty wake ring, the
exact path the claim is about — for 25 runs at each of 1, 10, 30, 60 and
100 ms, prints min/median/max and the number of runs that expired before the
requested duration, and prints the system timer resolution
(`NtQueryTimerResolution`) before and after. It opens no window, plays no
audio, sends no input, takes no arguments, and exits non-zero only if the
mechanism itself misbehaves (a wait that returns data on an empty ring, or an
error).

**This lane's run.** Host: Windows 11 24H2, build 26100.9711, x86_64, AMD
Ryzen 7 5800H, 32 GB. The example was compiled with `rustc` against the
workspace's own build (`cargo build --workspace`, then `rustc --edition 2024
--extern rustamp_ipc=target/debug/deps/librustamp_ipc-<hash>.rlib -L
dependency=target/debug/deps`) because this lane's constraint list names only
fmt/clippy/build as cargo commands, and launched with `CREATE_NO_WINDOW` (the
project's own rule for spawned console programs) with stdout piped, so no
window could appear. Two full runs, 125 waits each, both exit 0:

```text
wake_timeout_probe: rustamp-ipc blocking-wait timeout probe (RFC 0007 §6.4, task G5)
os=windows arch=x86_64
timer_resolution_ms (before): maximum=15.625 minimum=0.500 current=1.000 (NtQueryTimerResolution, 100 ns units: 156250 5000 10000)
ring: pcm capacity=8 frame_size=8 wake=attached empty=yes producer_writes=no
path: RingConsumer::read_into_blocking on an empty wake ring
duration_ms runs min_ms median_ms max_ms early_runs
1 25 8.732 15.507 16.469 0
10 25 14.991 15.457 16.228 0
30 25 29.885 31.169 32.183 1
60 25 61.704 62.249 63.670 0
100 25 107.529 109.437 110.619 0
timer_resolution_ms (after): maximum=15.625 minimum=0.500 current=1.000 (NtQueryTimerResolution, 100 ns units: 156250 5000 10000)
summary: 125 waits, 1 expired before the requested duration
probe: done
```

```text
wake_timeout_probe: rustamp-ipc blocking-wait timeout probe (RFC 0007 §6.4, task G5)
os=windows arch=x86_64
timer_resolution_ms (before): maximum=15.625 minimum=0.500 current=1.000 (NtQueryTimerResolution, 100 ns units: 156250 5000 10000)
ring: pcm capacity=8 frame_size=8 wake=attached empty=yes producer_writes=no
path: RingConsumer::read_into_blocking on an empty wake ring
duration_ms runs min_ms median_ms max_ms early_runs
1 25 7.350 15.620 16.226 0
10 25 14.882 15.487 16.350 0
30 25 30.366 31.041 32.370 0
60 25 60.783 62.239 63.728 0
100 25 107.477 109.257 110.338 0
timer_resolution_ms (after): maximum=15.625 minimum=0.500 current=1.000 (NtQueryTimerResolution, 100 ns units: 156250 5000 10000)
summary: 125 waits, 0 expired before the requested duration
probe: done
```

**What the host measurement shows:**

- All 250 waits returned `Ok(0)`: the mechanism's outcome contract held every
  time (no data on an empty ring, no error, no hang).
- **No early expiry of the size the G3 card reported.** 249 of 250 waits
  expired at or after the requested duration. The single exception was a
  30 ms wait measured at 29.885 ms — 0.115 ms "early", within the plausible
  skew between the performance counter the probe reads and the kernel's
  tick clock. The G3 figure (~7.8 ms for a 10 ms request, i.e. 2.2 ms early)
  did not reproduce anywhere near that size on this host.
- **Sub-tick requests expired late, not early.** 1 ms requests took
  7.350–16.469 ms and 10 ms requests took 14.882–16.350 ms: expiry
  quantized to the 15.625 ms *maximum* timer resolution — the wait returns at
  the first tick boundary at or after the requested duration — plus up to
  ~1.3 ms of return/scheduling jitter on this busy host.
- The reported *current* timer resolution (1.000 ms) does not match the
  observed wait-expiry grid (15.625 ms) on this host. The probe records both
  numbers and does not explain the gap. Establishing it would take a
  controlled `timeBeginPeriod` experiment, which mutates global host state
  (power draw for every process on the machine); this lane deliberately did
  not run one. The VM guest — a pinned OS build with no other process raising
  the resolution — is the clean environment for that.

## 4. What the tests now assert, and why each bound follows from the mechanism

| Test | Assertion | Why it follows from the mechanism |
|---|---|---|
| `wake.rs:507` `timeout_ms_rounds_the_request_up_to_whole_milliseconds` (new) | 10 ms→10, 1 ms→1, 1.5 ms→2, 500 µs→1, 1 ns→1, `ZERO`→0, `None`→`INFINITE`, >u32 ms→clamp | The mapping *is* the mechanism's timing contract (RFC 0007 §6.4) and a pure function, so its exact values are pinned deterministically. Cannot flake. |
| `ring.rs:1837` `blocking_timeout_returns_timed_out` (name kept; G2's card references it) | `Ok(0)`, `elapsed >= 1 ms`, `elapsed < 200 ms` | `>= 1 ms` is the kernel-entry smoke floor — the only lower bound that holds on every system the documented caveat covers. `< 200 ms` is a hang guard. |
| `ring.rs:1858` `wait_readable_returns_timeout` | `TimedOut`, same bounds | Same. |
| `ring.rs:1892` `blocking_timeout_bounds_hold_across_durations` (renamed from `blocking_timeout_accuracy_across_durations`) | per run: `Ok(0)`, `elapsed >= max(requested − 16 ms, 1 ms)`, `elapsed < requested + 500 ms` | The lower bound is *derived*, not measured: the code never returns early itself, and the documented caveat allows sub-clock-resolution waits to expire early, so one tick of early expiry is the loosest safe allowance. It catches immediate returns and gross early returns; it cannot catch a sub-tick early expiry — and its doc comment now says so. The upper bound is a hang guard. |

The rename is deliberate: the old name claimed a measurement of *accuracy*
that the assertions do not make. The doc comments on all three tests now
state what each bound proves and what it cannot. No assertion bound was
changed, and no mechanism code changed: `do_wait` (`wake.rs:166`) calls the
extracted `timeout_ms` (`wake.rs:145`) with identical semantics — the mapping
moved, it did not change. The blocking methods, the wake events, the
check-then-wait loop and every other test are untouched.

## 5. What stays unmeasured, and what would measure it

- **The VM's per-run elapsed times, and whether the ~7.8 ms figure reproduces
  at all.** Only the VM has the timer behaviour the G3 card described, and
  only the orchestrator may run the suite there. What would measure it:
  `cargo run -p rustamp-ipc --example wake_timeout_probe` inside the guest,
  plus the suite's own `blocking_timeout_*` tests, with the probe's per-run
  output committed as the artefact — the same shape as §3, one environment
  over. Until then the ~7.8 ms figure stays marked in the G3 card as an
  unevidenced single observation.
- **Sub-tick precision.** Would need `timeBeginPeriod(1)` (global, and
  discouraged for power/battery impact) or a system whose clock tick is
  already 1 ms. Either way it is a property of the environment, not of
  `rustamp-ipc`.
- **The relationship between the reported current timer resolution and wait
  expiry** (the 1.000 ms vs 15.625 ms gap observed on the host). Would need a
  controlled `timeBeginPeriod` experiment; not run here because it mutates
  global host state.
- **Real hardware adds nothing to this question.** The wake path is a kernel
  timer and a shared-memory ring; no audio device, window or speaker is
  involved. A real machine adds only a different OS build and timer state —
  i.e. another environment row in the same table, not a new kind of evidence.

## 6. The cross-process wake checks

`cargo xtask wake-cross` (the G4 lane) is untouched and keeps working:

- `xtask/src/wake_cross.rs` and `crates/rustamp-ipc/examples/ring_peer.rs`
  were read, not modified. The timeout phase drives `TEST_TIMEOUT 50` and the
  peer asserts `read == 0`, `elapsed >= 1 ms`, `elapsed <= ms + 1000 ms`
  (`ring_peer.rs:720-749`) — the same smoke floor and hang guard this lane
  kept, so the cross-process check's semantics are unchanged.
- The mechanism (`do_wait` → `WaitForSingleObject`) is behaviour-identical:
  the only code change is that the ms mapping moved into `timeout_ms`.
- wake-cross builds `ring_peer` for both bitness targets; this lane's new
  example is a separate file and does not affect it. `cargo clippy
  --workspace --all-targets --all-features` checks both examples and is
  green.
- What only the VM could add to the cross checks: nothing structural —
  wake-cross already runs headless on Windows. The VM adds the guest's timer
  behaviour for the timeout phase's *numbers* (the same open item as §5).

## 7. Follow-ups for the orchestrator (not owned by this lane)

- `docs/HANDOFF.md:233` still says "the wake timeout question is settled by
  measurement (the G3 card)". After this lane the honest statement is: the
  mechanism's timing contract is pinned exactly, the OS behaviour is measured
  on the host (this card, §3), and the VM's numbers are still unmeasured.
  The handoff line should be reworded by its owner.
- `docs/tasks/BETA-readiness-2.md:52` tracks F7 as "STILL OPEN as a prose
  problem"; this card closes it. `docs/tasks/BETA-what-is-in-it.md:245` and
  `docs/tasks/W11-docs-sweep.md:89` cite the old G3 mechanism/figure and
  should be re-read against the narrowed G3 card.
- The orchestrator's VM run of the suite must confirm the new and renamed
  tests: `timeout_ms_rounds_the_request_up_to_whole_milliseconds` (new) and
  `blocking_timeout_bounds_hold_across_durations` (renamed). This lane could
  not run `cargo test` per its constraint; fmt/clippy/build are green here.

## 8. Verification

- `cargo fmt --all -- --check` — exit 0.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  (run with `--offline`) — exit 0, no warnings.
- `cargo build --workspace` (run with `--offline`) — exit 0.
- The probe was compiled with `rustc` against the workspace build and run
  twice on the host, headless (`CREATE_NO_WINDOW`), no window, no audio, no
  input; exit 0 both times. Its full output is recorded in §3.
- **`cargo test` was not run** (task constraint: the suite runs in the Hyper-V
  VM, only the orchestrator runs it). The VM was not run. No commit was made.
- `git status --short` in this worktree lists only the owned paths above.
  The canonical tree `D:\code\Rustamp` is untouched (its `git status --short`
  is empty). The worktree did not exist when this lane started (the failed
  Google job wrote nothing); it was created with the orchestrator's own
  `worktree_create` tool at `main` (`2ebaaf0`) and confirmed clean before the
  first write.

## 9. Provenance

- All work is from `docs/rfc/0007-blocking-consumer.md`, RFC 0003, the
  `rustamp-ipc` sources, the task cards G2/G3/G4/W10, and public Win32 API
  behaviour (`WaitForSingleObject`, `NtQueryTimerResolution`).
- No Winamp source, SDK material or disassembly was read; no reference binary
  was run; nothing was downloaded; no new dependency was added. The probe
  uses only `rustamp-ipc`'s public API, `std`, and one `ntdll` extern declared
  in the example itself.
- The host measurement in §3 is this lane's own run, on this lane's machine,
  with the method committed so anyone can reproduce it on any Windows host or
  in the guest.
