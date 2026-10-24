# W29: Preserve DSP diagnostics across ring close and host teardown

Status: **DONE**; all three release gates passed, with repeated host tests.
Owner: `wave129-dspflake` (own pre-created worktree).
Scope: `crates/rustamp-mediahost/**` and this card only.
Baseline: `afae642b0c2f3ece75710a5bf9e9709738cbca48`.

## Goal and sources

Reproduce the release-blocking `dsp_control` flake without changing its
assertions, identify the actual ordering faults, and fix them without sleeps,
longer deadlines, dependencies, protocol changes, or changes in other crates.

Sources: in-tree Rust implementation/tests, RFC 0006 sections 4/6/8 and
RFC 0014 sections 4/7. This is an implementation-order correction to existing
contracts, not a new core contract. No Winamp source, SDK headers, disassembly,
reference binaries, research repository, downloads, or copied third-party code
were used. No git write/commit was performed.

## Unmodified reproduction

Ran `cargo test --offline -p rustamp-mediahost --all-features --test dsp_control`
**50 times before any source/test change**. Each exit code was captured into a
variable immediately after cargo returned. **2/50 failed (runs 2 and 24)**,
both exit 101; the other 48 returned 0. Each failed binary reported 13 passed,
1 failed, finishing in 5.06 s / 5.09 s respectively.

Both failures name:
`midrun_chain_failure_is_a_loud_dsp_error_carrying_the_text`.

Run 2, original `tests/dsp_control.rs:131:24` (`write_pcm`'s successful-write
unwrap, reached while sending the final failing frame):

```text
called `Result::unwrap()` on an `Err` value: Disconnected("DSP host closed its input ring")
```

Run 24, original `tests/dsp_control.rs:578:17` (assertion that the disconnect
contains `resident DSP failed`):

```text
the DspError text must reach the player: media host disconnected or failed: DSP back-ring closed before input finished
```

The following assertion also requires the original resident diagnostic
`clipping detected`. None of these assertions was deleted or weakened.

## Attribution: product fault AND a missing test precondition

1. **Product diagnostic-order fault.** `pump` propagates the chain error with
   `?`. The inner function's `ActiveConsumer`/`ActiveRing` drop guards close
   the shared rings *before* `serve_dsp` queues `DspError`. Independently,
   `ControlPeer::drop` closes its abort handles, even if the writer has flushed
   the diagnostic but the player's monitor has not processed it. A read/write
   can therefore observe closure with a healthy monitor and report a generic
   disconnect; the back-ring read makes that generic reason sticky, losing the
   resident's diagnostic. Writer flush and a large healthy PCM backlog do not
   synchronize the remote monitor.
2. **Test precondition fault.** The final write promises only that the host
   *took* its frame, not successful DSP processing. The failing callback was
   free to finish before that write returned, so a legitimate terminal failure
   could reach a write whose helper unconditionally unwrapped success. A
   test specifically asserting the subsequent read's diagnostic must first
   guarantee that its setup write has completed.

## Deterministic correction

- On DSP failure, endpoint drop only unmaps; the host relinquishes its
  close-only supervision handles before publishing `DspError`. Host teardown
  can no longer overtake the diagnostic through shared ring state. The
  player's monitor stores the sticky diagnostic **before** closing its own
  handles and waking PCM operations, using the existing `State::fail` ordering.
  Control EOF/liveness remains the fallback if diagnostic publication fails.
- Successful EOS, shutdown and reconfigure still close rings explicitly,
  including transitions during the back-ring handshake. No client-side wait,
  timeout, wire type, ring layout, public API, or dependency changed.
- The original fixture now holds the failing callback on an mpsc channel
  until the final write returns. This establishes the missing setup
  precondition without assuming how fast the host or monitor runs.
- Added
  `chain_failure_reaches_the_monitor_before_ring_close_even_after_host_teardown`.
  A framing-aware reader holds exactly `DspError` before the player's monitor
  can consume it. The test waits for the host to finish **and drop its control
  peer**, verifies both shared rings are still open, releases the diagnostic,
  then demands a typed disconnect containing `resident DSP failed` and
  `clipping detected`, followed by the identical sticky write failure.
  All ordering is channel-observed; existing stall bounds are only escape
  failures, not success criteria.

**Red/green evidence:** the new regression was run with only the fixture/test
changes against the original product implementation. It failed exit 101 with
`input closed before DspError` (0 passed, 1 failed, 14 filtered out, 0.06 s).
After the product correction the complete binary passed all 15 tests, exit 0.
Thus the product change is independently necessary, not merely test tuning.

## Verification receipts

All commands run on this host from the assigned worktree, with
`CARGO_TARGET_DIR=<worktree>/target` and `--offline` for cargo test/clippy.
Raw logs and immediately captured exit lists are retained locally in
`crates/rustamp-mediahost/target/w29-evidence/` (ignored build evidence).

- Before: `before-001.log` through `before-050.log`, `before-exits.txt`.
- Red regression: `regression-before.log`, exit 101 as expected.
- First green binary: `fix-first.log`, exit 0, all 15 tests passed.
- Forced-order regression alone: **100/100 green**, each exit **0**; invoked
  the built `dsp_control` executable with `--exact` and the new regression's
  name (`ordering-regression-*.log`, `ordering-regression-exits.txt`).
- Clippy: workspace/all-targets/all-features/`-D warnings`, exit **0**.
- Full workspace/all-features under the concurrent repeated-binary load:
  **3/3 green**, each exit **0**, each reporting **2341 passed, 0 failed,
  45 ignored** across 141 unit/integration/doctest summaries
  (`workspace-concurrent-001.log` through `workspace-concurrent-003.log`,
  `workspace-concurrent-exits.txt`).
- Repeated complete `dsp_control` binary: **100/100 green**, each exit **0**,
  each reporting all **15 tests passed** (`after-001.log` through
  `after-100.log`, `after-exits.txt`). This is in addition to the first green
  binary and its executions inside the full workspace runs.
- Full workspace/all-features after the 100-run loop: another **3/3 green**,
  each exit **0**, each reporting **2341 passed, 0 failed, 45 ignored** across
  141 summaries (`workspace-001.log` through `workspace-003.log`,
  `workspace-exits.txt`). **Total full workspace runs after the fix: 6/6 green**,
  three concurrent with the DSP loop and three after it.
- Final `cargo fmt --all -- --check`: exit **0** (`fmt.log`, `fmt-exit.txt`).
- Final gates: **fmt = 0; clippy = 0; workspace/all-features tests = 0**
  (all six test invocations). Every native command exit was captured into a
  variable immediately after the command.

Diff: `src/control.rs` +8/-0, `src/dsp.rs` +23/-24,
`tests/dsp_control.rs` +118/-10, plus this new card. No test was removed, no
assertion weakened, no deadline increased, and no sleep introduced.

## Coverage limits

Host results exercise real named shared-memory rings, production framing,
control reader/writer/monitor threads, DSP pumping, and forced adversarial
ordering in-process. They do not constitute execution of ignored interactive
VM tests: real child-process kill/reap and pipe teardown, device/output tests,
windows/dialogs/menus, or real plugin DLL behavior. Those still require the
interactive VM path and, for reference/plugin binaries, the owner's approval.
For this crate specifically, `tests/dsp_process.rs` keeps the three ignored
two-process round-trip/abrupt-exit/kill tests; their fixture is selected by
`RUSTAMP_DSP_FIXTURE`. No claim is made that those tests ran here.
`RUSTAMP_TESTDATA` was unset, so optional external `classic-base` golden
fixtures that silently skip are not claimed as covered either.

No outside-owned-path change is needed for this fix. The worktree's status
contains only those three Rust files and this new card. Read-only
`git -C D:\code\Rustamp status --short` returned no output (exit 0); the
canonical tree was not modified. No commit/staging/reset/git write was done.
