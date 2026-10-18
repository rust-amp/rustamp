# W23 - Close the accepted RFC 0012 lifecycle gaps

## 1. Scope and inherited-work review

Implementation complete; runtime verification belongs to the orchestrator's
interactive-session VM. No tests, VM runner, broker, reference binaries,
downloads, new dependencies, commits, merges, or Git writes were run here.

Write only your own files, and nothing under `D:\code\Rustamp`.
The only writable checkout for this lane is
`C:\Users\micro\.paseo\worktrees\3ikyavhh\paseo-wave111-rfc0012fix`.
Owned crates: core, output, audio, app; owned card: this file. Build intermediates
and temporary files are directed into this worktree's `target` directory.

The initial `git status --short` contained five modified files: `app.rs`,
`engine.rs`, `flush_contract.rs`, `output_error.rs`, and `null.rs`. Each complete
owned-file diff was reviewed against `main`, then against this checkout's HEAD
(`f52eb4b`). Main has unrelated later commits; those branch differences are not
lane edits and must not be integrated as deletions.

Kept from the exhausted lane:

- Separate stream-loss state and failure cause; engine-owned explicit retry of
  the existing output object, rebuilding both ring halves, handshake state, and
  render callback using the newly granted format.
- Null-output lifecycle counters that survive open; their unit test; the real
  callback-audio retry test, the flush-refusing-endpoint and missing-device tests.
- The app-level responsibility discriminator and its explanatory comment.

Discarded or replaced, with reasons:

- Removed the blanket `if stream_lost { return Ok(()) }` from `flush_output`.
  It falsely reported an unperformed flush as successful and also bypassed the
  contract for later explicit Stop/Seek commands. Instead only stream-error
  teardown omits the flush; the flush helper itself is unchanged from HEAD.
- Replaced the app test's nonexistent `advance_null_clock_for_test` call and
  incompatible borrowed closure with a predicate/deadline loop and the actual
  mutable `advance_null_clock` API. The inherited test did not compile.
- Replaced clearing the retry marker immediately after open with clearing it
  after successful start. cpal creates the actual stream in start; a refused
  start consumes the callback and otherwise makes the next attempt irrecoverable.
- Replaced the inherited assertion in comments that reopening does "strictly
  more than flush promises". A new lifecycle does not retract audio already
  handed to real hardware. The fake endpoint's retained-buffer oracle is not
  evidence about hardware silence.
- Added clock rebasing on unsuccessful open, rescaling of frame-based state on
  a changed rate, cleanup and snapshot restoration on failed retry-start, and
  stronger event/decode-completion assertions. Inherited coverage did not
  exercise these failure boundaries. Existing harness sleeps in the endpoint
  helper were replaced with yields; ordering is always a predicate, never delay.
  Its stream-error accessor now uses an Acquire atomic load rather than the
  inherited endpoint mutex, respecting the trait even in the scripted output.

## 2. Gap one: the promised next Play never reopened

RFC 0012 §4.3 item 8 explicitly assigns a clean open sequence on a later Play to
`Engine::play()`. Before this change, only `Engine::new` opened the output and
`start_run` rejected its persistent failure latch forever.

`stream_lost` now records a run terminated by the stream-error hook, including
the existing paused-resume detection. Polling stops/reports once without opening
anything. A later explicit start (Play, PlayIndex, or explicit navigation which
starts a stopped run) closes/opens the same output object, rebuilds its callback,
ring, and sync state, and starts a new run. There is no background reconnect,
automatic playlist advance, factory invocation, or backend switch.

The request remains `cfg.spec`; the granted format becomes `device`. The decoder
converts to that format as on the first open, with ring capacity calculated in
its granted channels. Rate-dependent state, including a stopped seek target,
is rescaled. Ordinary Play starts the current track from its beginning, as the
existing stopped-play policy does; RFC 0012 does not promise automatic resume
at the old frame. The frozen position remains until the explicit retry.

Failed open publishes/returns the backend's diagnostic and leaves retry armed.
Failed retry-start stops and joins its decode worker, restores the frozen
snapshot, reports the error, and leaves retry armed. A successful retry-start
clears the stream-loss marker and stale engine error. Pre-first-run health
rejection remains intact: a device failed before any run is not silently reopened.

The old error returned by close is not mistaken for failure to release a stream:
WASAPI close joins the render thread and clears its spec/started state before
returning its latched render error; cpal close drops stream/device/config/spec
before returning its old failure. The subsequent open is still attempted and
its result controls recovery. This follows the shipped backends and the trait's
stop-and-release lifecycle; it is not permission to ignore a live flush refusal.

## 3. Gap two: stream teardown accidentally triggered caller recreation

RFC 0012 §4.3 requires deactivate/join, pause, Stopped, frozen position, and
ordered Stopped/PositionChanged/Error publication. It does not require flushing
the failed device. Previously `handle_output_error` additionally called
`flush_output`; cpal refuses that operation, which armed `flush_failure`. App
`surface_error` then replaced the engine, destroying the frozen snapshot.

`OutputFailure::StreamLost` now omits flushing only in that teardown. It still
stops/joins decoding, pauses output, delivers a queued track start before the
freeze, clears pending seek/drain/end state, and emits the original failure
once. No close/open occurs until explicit retry. Errors applying decode reports
and stopped-seek failures use `OnLiveDevice`, retaining their flush semantics.

## 4. Gap three: ownership, evidence, and preservation of today's work

The accepted RFC and landed W22 fit without a new contract decision:

| Failure | Owner | Retry timing and state |
|---|---|---|
| Fatal stream failure (`stream_error`) | Engine | Preserve the stopped/frozen snapshot; close/open the same owned object only on an explicit start. |
| Actual flush refusal/failure | Application | Keep the terminal latch and close; recreate engine and output using the existing application factory and full live settings transfer. |

Evidence: RFC §4.3 item 8 names Engine Play; core `OutputDevice::flush` expressly
requires caller recreation only for a real refusal; W22 implements exactly that
policy in `recreate_engine`, `ensure_engine_ready`, and `surface_error`. An engine
already owns an output object on which it may call close/open; this does not
require or invent a device factory. Creating a replacement object after terminal
flush refusal remains the application's duty. If the user explicitly Stop/Seeks
after stream loss and that actual flush is refused, the terminal latch wins;
the engine does not reopen around it.

Today's strict `flush_output` implementation is byte-for-byte unchanged from
HEAD. Stop, natural finish, and successful-seek flush calls remain, and live
output error teardown retains its call. No `Unsupported` is silenced. No core
trait, protocol, or plugin ABI was changed.

Today's caller recreation implementation is also unchanged: queue, cursor,
volume, balance, EQ, limiter, ReplayGain mode/preamp, shuffle, repeat, and
stop-after-current transfer all remain. App changes are explanatory documentation
and one regression test, not a second recovery path. The app still only recreates
on the flush latch, so stream loss no longer accidentally erases its snapshot.

## 5. Regression tests and falsifiable oracles

These are source/compile evidence only, not passing test receipts.

| Test | What makes it fail |
|---|---|
| output `lifecycle_counters_count_every_open_close_and_start` | Resetting lifecycle counts on open, counting refused opens/starts, or not counting a real second lifecycle breaks exact counter assertions. |
| audio `explicit_play_after_a_failed_run_reopens_the_output_and_plays_again` | Missing close/open/start, forgotten callback/ring replacement, clearing no latch, teardown flush, silence-only output, or a nonadvancing new timeline breaks lifecycle/audio assertions. |
| audio `a_stream_error_on_a_flush_refusing_endpoint_keeps_its_position_and_reopens` | Any teardown flush/recreation, changed 256-frame snapshot, wrong event order/value/count, no decoder destruction before return, repeated error, stale endpoint/ring samples, or a resumed rather than rebuilt callback breaks assertions. A live decoder fills the ring first; its drop counter must then change 0→1. Reopened samples must equal the track's first 256 frames bit-for-bit. |
| audio `play_retries_the_reopen_while_the_device_is_still_missing` | Silently starting an absent device, losing the retry marker/snapshot, hiding the named diagnostic, missing actual open attempts, or arming terminal flush failure breaks assertions across three failed attempts and a successful fourth retry. Close resets the fake clock. |
| audio `a_refused_start_after_reopen_stays_retryable_and_keeps_the_failure_position` | Clearing retry at open, failing to recover a consumed callback, losing frozen frames, starting during refusal, or failure to produce exact audio after two refused starts breaks assertions. |
| audio `a_reopen_adopts_the_new_granted_rate_and_channel_count` | Reusing stereo/44.1k state after mono/48k grant breaks spec, full-ring frame capacity, one-second duration, callback sample count, finite/nonzero audio, and frame/ms clock assertions. |
| audio `an_explicit_stop_after_stream_loss_still_fails_closed_if_flush_is_refused` | Restoring the inherited blanket successful-flush bypass breaks the required Stop error, actual flush count, sticky terminal marker, and prohibition on engine-owned reopen. |
| app `a_stream_error_is_recovered_by_the_engine_not_by_app_recreation` | Eager app recreation loses the old failed clock/frozen snapshot; no retry or a replacement clock instead of the same object's second open breaks assertions. Repeated pumps/ensure-ready must retain the visible error and nonzero frozen time. |

The original refused-flush tests and caller-state-transfer tests remain in place.
Waits observe buffered frames or events with a stall deadline; yields only let
the worker run and do not establish order. No sleep was introduced.

## 6. Verification and delivery receipts

All Cargo gates ran offline, with target and TEMP/TMP inside this worktree.
Every command's exit code was captured immediately in a variable. Formatting
initially returned 1; the inherited and added formatting differences were fixed.
Final receipts:

| Gate | Exit code |
|---|---:|
| `cargo fmt --all -- --check` | 0 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 0 |
| `cargo build --workspace` | 0 |
| `git diff --check` | 0 |

Changed lines against this checkout's HEAD (not unrelated later main commits):

| File | Added | Removed |
|---|---:|---:|
| `crates/rustamp-app/src/app.rs` | 83 | 0 |
| `crates/rustamp-audio/src/engine.rs` | 166 | 14 |
| `crates/rustamp-audio/tests/flush_contract.rs` | 323 | 9 |
| `crates/rustamp-audio/tests/output_error.rs` | 93 | 1 |
| `crates/rustamp-output/src/null.rs` | 68 | 0 |
| `docs/tasks/W23-rfc0012-gaps.md` (new) | 188 | 0 |

This card is the sixth changed file (new). The final canonical-checkout
`git -C D:\code\Rustamp status --short` produced no output (exit 0). No paths
outside the five inherited files and this card appear in worktree Git status.

Only the orchestrator may run `cargo test --workspace --all-features`, in the VM's
interactive session. A successful Clippy compile of test targets is not a test
execution. VM evidence is still required for every regression above, the existing
flush/caller-state suite, and full window/plugin stopped/frozen/error propagation.
Real WASAPI/cpal removal/reappearance and callback/start faults require controlled
device scenarios; the fake output does not prove driver teardown or hardware
silence. The lane has not exercised an audio device or opened a window.

Provenance: project source, RFC 0012, W22/W23 task cards, and project rules only.
No Winamp source, SDK, disassembly, reference binary, or research tree was read.
No third-party code was copied.
