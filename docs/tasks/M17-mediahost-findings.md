# M17: Media-host concurrency findings

## Boundary and provenance

Worktree: `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave57-codex-mediahost`.
Write only `crates/rustamp-mediahost/**`, `crates/rustamp-dechost/**` and this
card. No access to the canonical tree, research tree or reference binaries.
Sources: repository instructions, M10/M12/M13/M14 and existing clean repository
code. No third-party code copied, dependency added, manifest/lockfile changed,
download, install, git write or commit. No core or wire contract changes.

Evidence below is inspection and compile-only evidence. **No tests executed.**

## 1. The flaky wait

- The direct defect was `format_change_drains_old_pcm_before_new_channel_count`'s
  `session.read(..., Some(WAIT))` (previously `tests/pcm_transfer.rs:209`). A
  healthy fixture may take arbitrarily long to run under VM load. Its progress
  is data, a format transition, EOS or terminal failure, not five elapsed seconds.
  Increasing that bound cannot make the inference correct: scheduler delay is
  not evidence of a failed transfer.
- There was also a hidden five-second bound in `DecoderSession::next`:
  `None` became `CONTROL_WAIT` after a drained ring while waiting for the next
  format/offer/EOS message. Merely changing the test to `read(None)` would leave
  that failure mode intact. `ControlPeer::recv_blocking` now waits on the command
  queue; monitor failure/exit disconnects it. `next(None)` uses it, while explicit
  caller deadlines retain their existing timeout behavior.
- Functional PCM reads and the shared shutdown completion wait in the fixture
  suite now use conditions. Successful ignored cross-process decoder/DSP reads
  use `None` too. Explicit liveness/watchdog checks and bounded handshake/seek
  policies remain deadlines by design, not progress assumptions.
- The decoder timeout test no longer races a 150 ms sleeping resident: a channel
  gate prevents any PCM until the test observes `TimedOut`, then releases it and
  reads without a deadline. No elapsed-time assertion remains in that test.

## 2. F3: Reset invariant retained and serviced while idle

Decision: a reset is legal mid-run and must reach the chain without requiring
another PCM write, without closing/replacing either ring, and without discarding
queued PCM. Restricting it to between-run use would contradict the existing
session API and reset test.

The IPC ring API has no non-closing interruptible read/wait operation. Without
editing IPC or adding a message, the DSP worker now uses **20 ms wait slices**
for both input and back-pressure waits. Slice expiry only rechecks commands; it
never becomes EOS, a terminal timeout, or permission to lose a frame. This is
an own worker scheduling choice, not a 20 ms real-time latency guarantee.
The monitor still closes/signals rings on actual failure/shutdown/reconfigure;
`DspReset` deliberately does not close them.

All queued resets are drained, including after an input read and before its
samples touch stage state. After drained EOS, resets actually reset the chain
and stay in the command loop instead of silently starting an unwanted ring
handshake. `DspSession::reset` documents that it is asynchronous, not a PCM/control
ordering fence or acknowledgement; immediate writes on the other channel do
not acquire a new ordering guarantee.

`tests/dsp_control.rs` supplies a stateful chain and reset counter. It holds the
first reset callback, queues a second reset with no PCM, and waits on the counter
before publishing any data. It then verifies cleared history on the next block,
unchanged generations, another idle reset, and reset after sticky EOS. The old
one-command-then-infinite-read loop cannot service the second reset without PCM.
Every functional wait in this test is a condition, not a wall-clock assertion.

## 3. F2: Honest truncated-file contract

The resident is a transparent adapter: actual backend errors are loud, but EOS
means **no more decodable frames**, not verified file integrity. The current
registry decoder explicitly converts container `UnexpectedEof` to EOS
(`crates/rustamp-decoders/src/symphonia_dec.rs:401-403`). Once converted to
`Ok(0)`, the adapter/framework cannot distinguish a truncated file from ordinary
end of stream. Rounded/estimated duration is not a universal integrity oracle.
No guessed duration threshold or partial WAV validator was added here.

The renamed truncated-WAV test now requires exactly **short EOS** for this
backend/fixture: positive but fewer than the original 4000 frames in the direct
decode, bit-identical hosted samples, sticky `Ok(0)`, zero transport drops, and
an `Ok(())` host shutdown. Any decode error fails the test; the old EOS-or-error
alternative and discarded host outcome are gone. Module docs name the limit.
This clarifies/pins F2; it does **not** claim universal truncation detection.

The comparison reference here is the same in-process registry decoder. No
exported spec describes Winamp 2.95's truncated-file terminal behavior; that
reference behavior remains a measurement/spec request, not an implementation
claim. No reference executable was inspected or run.

## 4. Prose-only gaps and test scope

| Gap | Written coverage (not executed) | Remaining limitation |
|---|---|---|
| URL refusal | `rustamp-dechost/src/lib.rs`: unit test requires the exact `OperationFailed` refusal for a URL | No network request or decoding is attempted |
| Abrupt player loss on decoder path | `mediahost/tests/pcm_transfer.rs::abrupt_player_loss_wakes_a_full_ring_decoder_writer`: raw framed player, publication condition fills capacity-one ring, pipes vanish without `Shutdown`/ring close, host must end `Disconnected` and abort ring | One process, fixture resident; not a real player-process kill |
| DSP bounded read | `mediahost/tests/dsp_control.rs::bounded_dsp_read_timeout_is_not_eos_and_retry_delivers_pcm`: no input before `TimedOut`, then exact data and sticky drained EOS | Worker/control threads, real Windows named rings, no audio |
| DSP reset while no PCM arrives | Stateful counter/history regression described above | No wire acknowledgement or cross-channel fence introduced |

All requested three headless gaps now have tests. A stage-generated mid-run DSP
diagnostic cannot be tested through `ResidentChain::process -> ()` (M14 F1);
changing that API/core contract belongs to the RFC lane. Real child-process
death/exit codes, real helper spawning and Winamp reference behavior are not
verified here. Existing ignored process tests still require VM staging. F4
(test-double direction validation), other M14 queue/generation findings and
unowned engine/resident code are outside this card's implementation scope.

## 5. Files and final line counts

Counts include blank lines; MH = `crates/rustamp-mediahost`,
DH = `crates/rustamp-dechost`. No manifests, dependencies or unowned source files
changed. This card is the only changed document.

| File | Lines |
|---|---:|
| MH `src/control.rs` | 326 |
| MH `src/decoder.rs` | 668 |
| MH `src/dsp.rs` | 803 |
| MH `tests/pcm_transfer.rs` | 495 |
| MH `tests/pcm_process.rs` | 84 |
| MH `tests/dsp_process.rs` | 105 |
| MH `tests/dsp_control.rs` (new) | 155 |
| MH `tests/support/streams.rs` (new) | 63 |
| DH `src/lib.rs` | 225 |
| DH `tests/real_decode.rs` | 380 |
| `docs/tasks/M17-mediahost-findings.md` | 153 |

## 6. Compile-only receipts and boundary proof

- `cargo fmt --all -- --check`: exit 0.
- `cargo clippy --offline --locked --workspace --all-targets --all-features -- -D warnings`: exit 0 (5.04 s final pass, including test typechecking).
- `cargo build --offline --locked --workspace`: exit 0 (59.08 s).
- Build artifacts confined to this worktree's `target` via `CARGO_TARGET_DIR`.
- Initial formatting differences and a test import unavailable without a new
  dependency were corrected; the abrupt-loss test uses the framework's existing
  dependencies instead. No dependency was added to bypass the constraint.
- No `cargo test` (including `--no-run`), test/example/player/helper execution,
  application/child-process spawn, window, audio output, VM job, download or
  commit. Only permitted compiler/format tooling and read-only inspections ran.
- No forbidden-tree reads/writes. A worktree-local status cannot independently
  attest to another tree; that tree was not accessed, even to inspect its status.

Final worktree-local `git status --short` (read-only, optional locks disabled):

```
 M crates/rustamp-dechost/src/lib.rs
 M crates/rustamp-dechost/tests/real_decode.rs
 M crates/rustamp-mediahost/src/control.rs
 M crates/rustamp-mediahost/src/decoder.rs
 M crates/rustamp-mediahost/src/dsp.rs
 M crates/rustamp-mediahost/tests/dsp_process.rs
 M crates/rustamp-mediahost/tests/pcm_process.rs
 M crates/rustamp-mediahost/tests/pcm_transfer.rs
?? crates/rustamp-mediahost/tests/dsp_control.rs
?? crates/rustamp-mediahost/tests/support/streams.rs
?? docs/tasks/M17-mediahost-findings.md
```
