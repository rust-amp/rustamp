# W28: Keep the IPC CPU observation window independent of OS timeout expiry

## Scope and provenance

- Own `crates/rustamp-ipc/**` and this card only; write only in the assigned
  `wave128-ipcwait` worktree, nothing under `D:\code\Rustamp`.
- Sources: this repository, RFC 0007 sections 1, 4.3, 6.4 and 14.5, and the
  history of the affected tests. No Winamp source, SDK, disassembly, research
  tree, reference binaries, third-party code, dependencies or downloads.
- No production behavior, contracts, other crates, git writes or commits.

## What the bound means

Both `a_blocking_read_sleeps_instead_of_spinning` and
`a_blocking_write_sleeps_instead_of_spinning` test the no-busy-spin requirement:
an empty read / full write times out with zero frames, uses less than 25% of
one core, and consumes less than 50,000 microseconds of thread CPU time.

Commit `49644514d49f70009fe27ad00f26687628514529` introduced the tests and
their 300 ms constants at the original `crates/rustamp-ipc/src/shm/ring.rs:2446`
and `:2493`, together with the strict minimum-wall assertions at `:2467-2470`
and `:2516-2519`. The rationale at `:2441-2442` is a sufficiently long CPU
accounting interval, not a maximum operation latency or a two-process
handshake. RFC 0007 sections 1 and 4.3 prohibit spinning; section 14.5 only
requires expiry after *approximately* the requested duration. No specification
promises this strict per-call minimum or mandates the particular 300 ms value.

The test is at fault, not the product: it equates a sample interval with one
kernel timeout. `wake.rs:135-182` rounds requests up to milliseconds and returns
the OS outcome; its documented caveat and commit `0f123408` explicitly avoid
claiming exact wall-clock expiry. W27 observed 299.6367-299.9142 ms returns.
This is not evidence of a missed signal or an IPC latency regression.

## Fix

Only the two Windows CPU tests and a shared test-only helper change.
`observe_blocked_thread` keeps one `Instant` and one thread-CPU baseline for
the entire observation. After every blocking call it observes elapsed time;
if the 300 ms window is incomplete it calls the same operation with exactly
the remaining budget. It does not restart a full timeout or reset CPU time.

The original `wall >= WINDOW`, `cpu * 4 < wall`, and `cpu < 50_000` assertions
are unchanged. Zero frames is now asserted on **every** call, not just once.
No tolerance, larger timeout, sleep, ignore, deletion, product change or
cross-crate change is needed. `wake.rs` and its exact timeout-conversion test
are untouched.

The accounting interval is deterministic: the helper stops only when its
observed elapsed time reaches the existing window. It no longer assumes that
one kernel timeout is a precise stopwatch. This is not an optimization or a
retry-until-pass: all CPU spent in every retry stays in the measured sample,
so a polling implementation still fails the same limits. Thread CPU accounting
and scheduler behavior remain measurements, not hard real-time guarantees.

## Verification

Every Cargo exit code was captured into a variable immediately after its
command. All commands ran offline and headless in this worktree on the host.
Logs are in the ignored `target/w28-*.log` files.

| Command | Exit |
| --- | --- |
| `cargo fmt --all -- --check` | 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 |
| `cargo test --workspace --all-features`, run 1 | 0 |
| `cargo test --workspace --all-features`, run 2 | 0 |
| `cargo test --workspace --all-features`, run 3 (restored final source) | 0 |

All three full runs executed 117 test binaries and 24 doctest groups: 2329 passed,
45 ignored, zero failed. The focused Cargo run passed all three `a_blocking_`
tests. Both CPU regressions then passed 100 direct binary executions each
(200 tests, zero failures).
The restored final source also passed `cargo test -p rustamp-ipc --all-features`
(exit 0): 90 passed including the doctest, 2 ignored, zero failed.

### Controlled measurement checks (temporary changes removed)

1. Temporarily cap each requested wait at 1 ms while retaining the entire
   300 ms measurement window. Both tests pass (exit 0): read 309.4776 ms,
   write 309.4029 ms, each 0 microseconds CPU. This forces repeated early
   returns relative to the observation window without sleeping for ordering.
2. Temporarily request zero-timeout polls instead. Both tests fail exactly at
   the unchanged CPU ratio assertion (expected exit 101): read 156250 us /
   300.0072 ms (52.082%), write 140625 us / 300.0005 ms (46.875%). Merely
   satisfying the new window loop cannot hide a busy-spin.
3. Restore `wait(remaining)` before the final three gates. The final fmt,
   Clippy and third all-features workspace run each exited 0, as recorded above.

`git diff --check` passed. The final source delta is `ring.rs` +38/-18 lines,
plus this new task card. Both worktree and canonical status were checked;
`git -C D:\code\Rustamp status --short` returned no lines (exit 0).

## Limits and integration

No interactive VM test was run. Only the interactive VM can confirm the
ignored GUI/window, actual audio-device/loopback, plugin-DLL and cross-process
acceptance paths under the project's host restrictions, including x86/x64
interop. The 45 ignored tests were not claimed as passes. No out-of-scope
change is required; this lane changes no production IPC code or wire layout.
