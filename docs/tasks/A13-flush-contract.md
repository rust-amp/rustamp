# A13: Honest endpoint flush and failed-seek preservation (W10 F3/F4)

**Status:** implemented; host compilation/linting only, VM test execution pending.
**Date:** 2026-10-17. **Lane:** `wave57-codex-flush`.
**Worktree:** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave57-codex-flush`.
Write only your own files inside this worktree, and nothing under `D:\code\Rustamp`.

## Scope and sources

Addresses W10's F3 and F4 only. Sources: the clean-room repository's
`W10-batch-review.md`, `A12-output-flush.md`, `H5-output-error-hook.md`, existing
`OutputDevice`/decoder contracts, all three output implementations, engine,
decode/ring handshake and audio tests. No reference behaviour is inferred.
No Winamp source, SDK material, disassembly, research tree or reference binary
was opened. No third-party code was copied and no dependency was added.

Owned changes: `rustamp-output/src/cpal_out.rs`, `rustamp-audio/src/engine.rs`,
`rustamp-audio/src/decode.rs`, new `rustamp-audio/tests/flush_contract.rs`, this
card. Other backends, existing tests and shared contracts are unchanged.

## F3: cpal cannot reset an endpoint through its portable API

cpal can negotiate, start, pause/resume and destroy a stream. Its portable API
does not expose a reset/drop of samples already rendered into the endpoint's
buffer. Draining the engine ring only removes samples not yet rendered; it
cannot remove that endpoint tail. Pause/resume retains that same stream and is
not a flush. Destroying a stream cannot retract samples already beyond it in
the audio hardware, either: instantaneous physical silence is not promised.

`<CpalOutput as OutputDevice>::flush` now returns `Error::Unsupported`, explicitly
saying to close and recreate the output stream. It discards nothing, does not
advance/reset the clock, does not clear a fatal-error latch, and does not itself
latch device loss. This is documented both on that trait implementation method
and in its module documentation. Even an unopened cpal object reports the same
capability refusal instead of pretending it performed a device reset.

**Honest engine consequence (own policy):** every required flush goes through
`Engine::flush_output`. Any flush error, including unsupported, deactivates the
ring, pauses and closes the output and latches a permanent restart barrier.
The caller must create a **new Engine/output lifecycle**, with a new render
callback, before playback can start again. There is no automatic rebuild,
backend switch or reopen in this change. In particular, the cpal fallback is
not feature-equivalent to WASAPI: a stop, confirmed seek, stopped seek or
natural end requiring a flush ends that engine's usable output lifecycle.
Ordinary cpal pause/resume without a discontinuity remains available.

`stop` and a stopped seek return the error as well as publishing
`Event::Error(Output)`. A running seek flush error follows H5's fail-to-stop
teardown; `poll` reports the error through events and `last_error`, as it does
for an asynchronously observed device error. Natural completion retains its
content-end position and publishes the flush failure. `play` and `start_run`
both enforce the restart barrier, even if backend `close` clears `stream_error`.
A failed flush is not retried during teardown. A secondary close error is
included in the diagnostic; close is best effort, not a hardware reset claim.
The timeline is frozen before close, since close need not retain its clock.

The old A12 universal stale-replay claim is therefore **superseded**, not
repeated: WASAPI schedules its native reset, null has no endpoint backlog,
and cpal refuses rather than silently claiming success. WASAPI's asynchronous
reset completion/driver compliance and any audible device tail remain real-
device checks; this lane does not claim to prove or change them.

### Shared-trait documentation boundary

The task also asks for central trait documentation, but explicitly forbids
editing `rustamp-core`. That boundary is preserved: no shared trait file or
signature was changed. The cpal trait-implementation docs and this card carry
the limitation. For the integration owner's central-doc follow-up, the intended
clarification is: a backend unable to drop its endpoint backlog must return an
error, not an empty success; callers requiring the drop must release that
stream and establish a fresh output lifecycle, not resume the retained stream.
No new shared capability/error type or RFC is introduced by this lane.

## F4: invalidate only on an established discontinuity, never on a refusal

**Rule:** a live seek authorizes endpoint invalidation only after the decoder
has successfully repositioned and the consumer has drained the old ring.
The requested target alone, a sequence-number completion, or the producer's
write cursor is not evidence that the audible timeline moved. A successful
seek still resets the pipeline even if a numeric target happens to equal a
previous displayed position. Stop/end and a new run retain their existing
invalidation policy; pause/resume retains playable audio.

Enforcement:

1. `Worker::apply_seek` sends `DecodeOut::Seeked` **only** on decoder success,
   after `begin_seek` and `await_drain`. The existing flush-before-engine-ack
   order remains, before the first post-seek write.
2. The error branch sends the separate internal `SeekFailed { seq, initial }`
   and the original decoder error. It resets no DSP/converter/ring state.
3. `Engine::on_decode_out` flushes/rebases/acks only `Seeked`. `SeekFailed`
   completes only its matching pending request and leaves the device buffer,
   ring, runtime clock origin and pending start drain intact. A refused initial
   seek undoes only the optimistic stopped-target display; it does not jump to
   the decode write cursor or discard elapsed playback. Its `initial` tag is
   independent of the newest pending request's sequence number.
4. On an actual flush failure in decode-report handling, `poll` restores its
   temporarily taken receiver **before** H5 teardown. The producer/DSP return
   messages are recovered by `join_decode`; queued seek reports cannot rebase
   an already stopped run on a later poll.

This is an internal report distinction, not a `rustamp-core`/wire change.
No `is_seekable` guess replaces the actual decoder result. As in the finding's
case, continuity after a refusal assumes that the decoder itself preserves its
read position on error; this change neither makes a destructive decoder seek
transactional nor attempts to infer an unknown decoder position.

## Headless regression tests (source added; NOT executed here)

The new endpoint double keeps **actual rendered samples** in a retained queue,
separately from the ring. Rendering advances its device clock; hearing drains
only that queue. Pause retains it, flush drops it, and its refusal mode leaves
it untouched until the engine closes it. Thus a missing flush loses the
successful-seek assertion, and an extra flush loses the failed-seek assertion.

| Test in `rustamp-audio/tests/flush_contract.rs` | Pin |
|---|---|
| `a_failed_seek_preserves_endpoint_and_ring_audio_and_the_audible_timeline` | Exact already-rendered audio and following ring samples survive; no clock jump, flush or close. Runs against flush-capable and flush-refusing endpoints. |
| `a_successful_seek_drops_the_endpoint_tail_before_new_audio_is_written` | Actual retained tail is empty after flush; the first new sample is exactly the target, not dropped. |
| `pause_resume_retains_audio_and_stop_start_still_discards_the_previous_run` | Pause retains audio; stop clears it; restart reuses the flush-capable stream, performs A8's silent drain and plays track 2 from frame zero. |
| `a_refused_flush_on_seek_closes_output_and_blocks_all_restart_paths` | One flush attempt/close/error, stopped transport, no retained callback, no resume/restart or later duplicate error/rebase. |
| `stop_and_stopped_seek_report_a_refused_flush_instead_of_silent_success` | Both synchronous entry points return failure and block playback. |
| `natural_end_keeps_its_position_and_reports_a_refused_flush` | Content-end position survives fail-closed completion. |
| `a_failed_initial_seek_on_a_fresh_run_starts_at_the_real_position` | Rejected optimistic target rolls back and first source frame survives. |
| `a_failed_initial_seek_undoes_the_target_without_cancelling_the_start_drain` | A8 start drain remains pending and first new-run frame survives after rollback. |

`rustamp-output/src/cpal_out.rs` adds
`flush_refuses_without_claiming_a_drop_or_changing_the_clock`: directly invokes
the real cpal trait method without opening a device, checks unsupported on a
healthy object and checks that refusal preserves a nonzero clock/failure latch.
Replacing the implementation with the former empty `Ok` fails this test.

Own test choices: deterministic existing synthetic pattern/factory, limiter
disabled for bit-exact comparisons, an 8192-frame ring, 256-frame priming,
1024-frame progress callbacks (worker block size), a 64-frame drain callback,
and bounded 5-second waits. These are harness choices, not measured device
properties. The sample queue/mutex is test-only; no callback code was added to
the production audio path. Existing A8/A12/H5 tests are unchanged.

## Verification and hygiene

- `cargo fmt --all -- --check`: PASS, exit 0 after formatting-only patch edits.
- `cargo clippy --offline --workspace --all-targets --all-features -- -D warnings`:
  PASS, exit 0, no warnings; includes checking the new test targets, not running them.
- `cargo build --offline --workspace`: PASS, exit 0.
- `cargo test`: **NOT RUN**. No test executable, example or player was run.
- No window opened, no audio device opened, no audio played, no download, no
  dependency addition, no commit or other git write.
- Every command ran in the named worktree. Source/doc writes and build/temp
  output are inside it. No forbidden canonical/research/reference tree was read
  or written. Final `git status --short` is recorded in the lane report; it must
  contain only the five owned paths listed above.
