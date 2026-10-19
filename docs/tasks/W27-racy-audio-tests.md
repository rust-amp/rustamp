# W27: Stable setup for two prefetched-audio regressions

## Goal and boundaries

- Fix the host-sensitive setup of late repeat enablement and mid-run shuffle
  replanning without weakening their assertions or changing production behavior.
- Own `crates/rustamp-audio/**` and this card only. Write only in the assigned
  `wave127-races` worktree, nothing under `D:\code\Rustamp`.
- Allowed sources: this repository and audited `specs/S15-shuffle-repeat.md`
  sections 4.1 and 4.3. No Winamp source, SDK, disassembly or research tree was
  accessed; no third-party code, dependencies or downloads were added.
- No contract changes, git writes or commits.

## Diagnosis: test setup, not a product stall

The supplied empty-ring explanation does not match this checkout's failing
lines: `Harness::wait_track` calls `wait_idle`, which never pumps. It requires
`current_index() == Some(0)` plus metadata. `Engine::poll` drains every queued
decode report; each `TrackStarted` updates `current_index`, even for prefetched
tracks. A worker that produces both/all three short tracks before that poll
therefore leaves decoder index 1/2, not 0. The ring remains completely unconsumed
and `audible_index()` correctly stays 0. Waiting for decoder index 0 then stalls
for the 30-second escape because no audio is pumped and no new decode work can
restore that index with repeat off.

A faster host can lose the brief observation window; a slower guest can allow
the first poll to see track 0 before the worker advances. This is scheduling,
not an engine failure to advance/report a track, and not an empty-ring pump at
these initial waits. The unchanged baseline passed one Cargo run and 20 direct
runs of all four `regression_` tests here; that does not disprove the race.

A temporary, subsequently removed headless probe forced the missed observation
window without sleeps: start playback, wait on the ring backlog with
`yield_now` (no polling or pumping), then poll for EOS. It passed (exit 0) and
reported:

```text
queue=2 current=Some(1) audible=Some(0) counters=(2000, 0) eos=Some(2000)
queue=3 current=Some(2) audible=Some(0) counters=(3000, 0) eos=Some(3000)
```

Thus the worker really progressed and preserved all audio; it is the assumed
initial decoder-index observation that is invalid. No production change is
needed.

## Preconditions and fixes

### `regression_late_repeat_enablement_at_queue_end`

Wait without pumping for both `buffered_frames() == total` and
`end_of_stream() == Some(total)`. Assert `(written, consumed) == (total, 0)`,
decoder index 1 and audible index 0 before enabling repeat. The test must
actually revoke a published EOS while all first-cycle audio remains queued;
merely seeing track 0 or waiting for some buffered data proves neither.
The existing EOS revocation, Playing and second-cycle assertions remain.

### `regression_shuffle_mid_run_replan_anchoring_to_audible_track`

Wait without pumping for the whole queue backlog and decoder index 2; assert
the exact ring counters and retain audible index 0. This explicitly establishes
the discriminator: decoder has advanced, listener has not. The plan must still
start at audible track 0, not prefetched track 2. EOS is not required for this
behavior, so it is not this test's synchronization condition. Existing plan
length and anchoring assertions remain.

### Why not copy the neighboring repeat-disablement wait?

That test starts with repeat ON and waits for more than one full cycle in the
ring before pumping A: it needs second-cycle prefetch while B becomes audible.
Both repaired tests start with repeat OFF; more than a cycle can never appear
until repeat is enabled. They share the principle of a non-pumping observable
wait, but need different predicates. No sleep, timeout increase, test ignore,
weaker assertion or harness-wide semantic change was introduced.

## Verification

Every gate captured `$LASTEXITCODE` into a variable immediately after the
command. The completed required gate run reported:

| Command | Exit |
| --- | --- |
| `cargo fmt --all -- --check` | 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 |
| `cargo test --workspace` | 0 |

The workspace run executed 117 test binaries and 24 doctest groups: 2320 passed,
45 ignored, zero failed. The focused `shuffle_repeat` binary passed all 17
tests (exit 0). Each repaired regression then passed 100 separate executions
(200 total, exit 0). `git diff --check` also passed. Initial background gate
attempts hit the shell's 120-second command timeout during compilation/lock
contention; they are not counted as completed test gates. The completed run
used no shell timeout.

### Separate all-features blocker outside this lane

The additional `cargo test --workspace --all-features` run and one full retry
both exited 101 in unchanged `rustamp-ipc`. The first failed
`shm::ring::tests::a_blocking_write_sleeps_instead_of_spinning` at
`crates/rustamp-ipc/src/shm/ring.rs:2516`: 299.9142 ms versus the strict
`wall >= 300 ms` assertion. The retry failed that assertion at 299.6367 ms and
the equivalent read assertion at line 2467 at 299.7215 ms. Both audio regressions
passed in both runs. This is not a green all-features suite.

No IPC file was changed. The exact follow-up scope is those two wall-clock
assertions and the Windows `backend::do_wait`/`timeout_ms` implementation in
`crates/rustamp-ipc/src/shm/wake.rs:135-182`: its existing documentation explicitly
allows sub-clock-resolution early OS expiry, whereas the tests demand a strict
minimum. An IPC-owned lane must reconcile that contract mismatch, without
silently weakening assertions or adding sleeps. No out-of-scope change is
needed for the W27 fixes themselves; the separate all-features blocker remains
reported rather than modified here.

Host tests were headless. GUI, physical audio-device/loopback and plugin-window
acceptance remain interactive-VM-only; these results do not confirm actual
endpoint output or GUI behavior. No VM run was performed by this lane.
