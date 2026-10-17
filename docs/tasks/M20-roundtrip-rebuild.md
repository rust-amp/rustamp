# M20: Rebuild the DSP resident round-trip so producer and consumer progress together

**Date:** 2026-10-17. **Lane:** wave63-dsp-deadlock (round-trip rebuild).
**Worktree:** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave63-dsp-deadlock`.

## Boundary and scope

Write only inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

Owned paths: `crates/rustamp-dsphost/tests/**`,
`crates/rustamp-mediahost/src/dsp.rs`,
`crates/rustamp-mediahost/tests/**` (only the sibling tests that share the
same producer-wait / write-then-read shape: `dsp_control.rs`,
`dsp_process.rs`) and this card. `crates/rustamp-audio`,
`crates/rustamp-dsp`, `crates/rustamp-decoders`, `crates/rustamp-meta` and
every other crate were **not** touched. In particular
`crates/rustamp-audio/src/resident.rs` (M19's adapter, already a
`write_timeout` user) was left alone.

Static only: **no `cargo test`** — not the suite, not `--no-run`, not a
single binary. No test process, example, player or reference binary was
spawned; no VM was touched; no window or audio device was opened; no
download, no dependency addition, no install, no commit and no other git
write. Only `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace --offline` ran (receipts in §7). Every command ran
inside this worktree.

Sources: the M18 diagnosis (candidate 1), RFCs 0003/0006/0007, the M10/M13/
M17/M19 cards, the existing clean repository code only. No Winamp source,
SDK material or disassembly was read; no third-party code copied.

## 1. The deadlock, and the restructuring that fixes it (not the symptom)

M18 candidate 1 (my bet, and the file that hung r32/r34): the round trip in
`crates/rustamp-dsphost/tests/dsp_roundtrip.rs` was a **write-then-read
split** — `write_sweep` pushed *all* `TOTAL = 4099` frames before `read_all`
read a single one. The player blocks in `DspSession::write` when the in-ring
(capacity 3/7) is full; the host stops draining the in-ring once its
back-ring (`BACK_RING_FRAMES = 4096`, `rustamp-mediahost/src/dsp.rs:36`) is
full, because the player is not reading. That is the textbook bounded-buffer
deadlock, and the arithmetic sat exactly on the boundary: the host can
absorb `4096` (back-ring) + `3` (in-ring) + `h` (its in-hand block,
`1 ≤ h ≤ 3`) = 4100–4102 frames against 4099 written — **a 1–3-frame
margin** (`TOTAL = 4099 = BACK_RING_FRAMES + in-ring capacity` exactly).
M18's bounded waits (`STALL_WAIT`) were right but not enough: they turn
the hang into a loud failure, they do not make the round trip progress.

### The rebuild: lockstep chunks

`write_sweep` + `read_all` are replaced by one helper, `round_trip(session,
from, to)` (`dsp_roundtrip.rs:202`): **write one bounded chunk of at most
`CHUNK = 257` frames, read that chunk back, repeat.** The host processes
strictly in order, so the frames a chunk's read-back waits for are exactly
that chunk. `reset_keeps_rings_and_continues` and
`reconfigure_reoffers_both_rings_with_new_sound` run two `round_trip` legs
around their mid-stream event; the one-frame-at-a-time write patterns of the
old bodies are gone (same frames, same values, same assertions — the write
granularity was never an assertion).

### The invariant that now guarantees progress

**The producer never starts chunk n+1 before the consumer has drained
chunk n.** Because the host moves frames strictly in order from the in-ring
to the back-ring, the frames in flight (written but not yet read back)
never exceed one `CHUNK` (257). Therefore:

- the back-ring occupancy never exceeds **257 frames** — 3839 below its
  4096 capacity — so the host's back-ring write (`write_from_blocking(...,
  Some(CONTROL_POLL))` in `pump`) always finds space immediately and never
  stops draining the in-ring;
- the producer's write therefore always finds room (the in-ring is drained
  within the host's 20 ms control-poll slices), and the consumer's read
  always finds data (the host processes what the in-ring holds).

Neither wait can be permanent: the only cycle would be producer-waits-for-
host-reads plus host-waits-for-producer-reads, and the second edge is gone
because the host never waits for the player to read. The deadlock boundary
(4096 + in-ring capacity + the host's in-hand block) is unreachable by
construction.

### Why the margin is no longer one to three frames

The old split forced the whole 4099-frame sweep through the pipeline with
**no reader**: the system had to absorb 4099 frames against a no-reader
capacity of 4100–4102 (cap 3) — M18's 1–3-frame margin. The lockstep caps the
no-reader absorption at one chunk: the back-ring holds at most 257 frames,
so the host would need **4096 − 257 = 3839** more frames in flight before
its back-ring write could ever block. The margin is 3839 frames, not 1–3,
and it holds for every in-ring capacity the file uses (3 and 7), because
the chunk (257), not the in-ring, is the binding constraint. The sweep still
flows 4099 frames through the 4096-frame back-ring (and 4099 frames through
the 3/7-frame in-ring, wrapping it ~86/~29 times per chunk), so the
longer-than-capacity, wrapping and lossless assertions keep their meaning.
On top of the structural fix, every wait is still bounded by `STALL_WAIT`
(30 s, six times the bound that flaked under VM load), so even a wedged
resident fails loudly with the step named instead of hanging the run.

## 2. The producer side in the source: the bounded write

**The bounded variant the M18 diagnosis named already exists at HEAD.**
M19 (`746fce4`, "Let the engine run its DSP chain in the resident…") added
`DspSession::write_timeout(&mut self, buf: &[f32], timeout: Option<Duration>)`
to `crates/rustamp-mediahost/src/dsp.rs` and made `write` delegate to it
with `None` — i.e. `write` keeps its existing signature for its callers,
exactly as the diagnosis required. M18's card still listed the variant under
"fixes named but NOT made" because M18's worktree predated M19's merge; the
variant itself was never missing from this tree. What *was* missing is what
this lane closes:

- **The test surface still called the infinite `write`.** `write_sweep`
  carried a comment claiming no bounded variant existed. It is deleted; every
  functional write in the owned test surface now goes through
  `write_timeout(buf, Some(STALL_WAIT))` via a `write_pcm` helper that
  panics with the step named on `TimedOut`: `dsp_roundtrip.rs` (the lockstep
  write, plus the fake-host test's setup write), `dsp_control.rs` (its three
  block writes, mirroring the file's `read_pcm`), `dsp_process.rs` (both
  tests). `crates/rustamp-audio/src/resident.rs` already used `write_timeout`
  (M19) and was not touched.
- **The timeout semantics are now stated in the source.** The doc of
  `write_timeout` (`dsp.rs:468`) previously said only "a bounded wait,
  never data loss and never false EOS". It now states what a timeout means:
  a **stall that fails loudly, never false EOS and never a silent short
  write** — `Ok(())` means the *whole* buffer was accepted, and the return
  type carries no accepted count, so a caller must treat the stream as
  stalled rather than continue writing. Frames the ring accepted before the
  timeout stay delivered (the timeout itself drops nothing); the rest of the
  buffer is not written. The implementation already had these semantics
  (`Ok(0)` from the ring on expiry → `TimedOut`; whole-call deadline via
  `remaining(deadline)`, mirroring the bounded `read`); this lane verified
  them against `RingProducer::write_from_blocking` (`rustamp-ipc
  src/shm/ring.rs:532`: writes `min(offered, free)` whole frames, `Ok(0)`
  means the timeout elapsed with the ring still full) and made the docs say
  so. `write`'s doc now points at the bounded form.

No `src/` behavior changed in this lane: the diff in
`crates/rustamp-mediahost/src/dsp.rs` is documentation only (+5/−4 lines of
doc comments). No wire, ring-layout, generation or `PROTOCOL_VERSION` change;
no `rustamp-core` contract change (none needed — the variant is a new method
on the media host's own session type, not a shared contract).

## 3. Assertions kept (none weakened, none was wrong)

| Assertion | Where it lives now | Status |
|---|---|---|
| Handshake contract: echoed spec, `latency_frames() == 0` | `passthrough_round_trip_is_bit_exact_and_eos_is_sticky` (unchanged lines) | kept |
| Bit-exact passthrough over all 4099 frames (`to_bits`) | same test, `round_trip` output vs `expected` | kept; the drain moved inside the lockstep, the comparison is over the same 4099 frames |
| No early EOS | `round_trip`'s read-back loop: `assert!(frames > 0, "host must not end the stream early")` (was `read_all`'s assert) | kept, same wording |
| Sticky EOS after the drain: two consecutive `Ok(0)` reads | `read_sticky` ×2 after `finish_writes` | kept; the first read is the drained close, the second sticks |
| Measurable gain change: `assert_ne!` + exact `input[i] * 2.0` per sample | `gain_chain_changes_audio_measurably` | kept (×2.0 in `f32` is exact: exponent bump, no rounding) |
| Reset keeps the rings: generations unchanged, `latency_frames() == 0`, stream exact across the reset | `reset_keeps_rings_and_continues` (two `round_trip` legs around `reset()`) | kept; the reset now lands between chunks instead of after a write-only phase — the pump services it in its 20 ms slices either way, and gain is stateless per sample |
| Reconfigure bumps both generations; new sound after it (×0.5 exact per sample) | `reconfigure_reoffers_both_rings_with_new_sound` | kept |
| No frames dropped: `frames_dropped() == 0` | passthrough, gain and both `dsp_process` tests | kept; still a 4099-frame stream through a 4096-frame back-ring |
| Refusals are loud, never EOS (unknown stage, bad params) | `unknown_stage_is_a_loud_error_before_any_ring`, `gain_with_bad_params_is_refused` | unchanged files' sections |
| Player drop ends the host `Ok`; vanished player ends it `Err`; dead host wakes the infinite read with `Disconnected`, never `Ok(0)` | `dropping_the_player_stops_the_host_cleanly`, `abrupt_player_loss_…`, `abrupt_host_loss_…` | unchanged (the fake-host test's setup write is now bounded, its assertions untouched) |
| Two-process: bit-exact ×2, zero drops, sticky EOS (two reads), abrupt helper exit wakes the infinite read | `dsp_process.rs` | kept; the write-all-then-drain loop became the lockstep, the drain-loop's terminating `0` plus the old sticky `0` are preserved as two consecutive sticky reads |

No assertion was wrong and none was weakened into passing. The only
assertion-adjacent changes are the ones the restructuring forces, all
disclosed above: the drain moved from "after `finish_writes`" into the
lockstep (the sticky reads still pin the drained-close-then-stick contract),
the reset/reconfigure bodies write 257-frame chunks instead of one frame at
a time (same frames, same expected values), and the two-process test's
drain loop became its lockstep read-back plus the same two sticky reads.

## 4. What still needs the VM, and what the two-process cases add

Everything in the owned files is written for the Hyper-V test VM and was
**not executed on this host** (lane constraint; `cargo test` is the
orchestrator's, never a lane's). Specifically still needing the VM:

- Executing the rebuilt `dsp_roundtrip` suite at all: the lockstep's
  progress under the guest's real scheduler, the 30 s stall bounds never
  tripping on a healthy host, and the deadlock staying dead — the exact
  regression this card exists to close (the three hung runs were guest
  runs). The runner's per-binary progress lines (M18's confirmation step)
  would show `dsp_roundtrip-<hash>.exe` completing.
- The real `NamedPcmRings` path: Windows shared-memory sections with
  auto-reset wake events, exercised in-process here only at compile time.
- Everything `#[ignore]`d: `dsp_process.rs` needs two real processes, the
  staged `dsp_fixture` example and `RUSTAMP_DSP_FIXTURE` set in the guest
  (the main suite runs without `--ignored`, so these are guest-explicit).

What the two-process cases add beyond the in-process byte-stream pairs:

- A **real spawned helper** (`DspProcess::spawn`, `CREATE_NO_WINDOW`): real
  stdio transport instead of in-process mpsc byte streams, real
  cross-process named shared-memory rings, real process isolation — none
  of which the in-process pairs can prove.
- The **same lockstep invariant across a process boundary**:
  `named_dsp_two_process_gain_round_trip_then_sticky_eos` now runs the
  identical write-block/read-block-back lockstep through a real helper
  (bit-exact ×2, zero drops, sticky EOS), i.e. the progress guarantee where
  the engine will actually use it (M19's `ProcessDspChain` drives the same
  wire).
- A **real process death mid-stream** (`fixture:exit`):
  `host_exits_mid_stream_and_infinite_read_survives` kills the helper after
  one block; the infinite player read must wake `Disconnected`, never
  `Ok(0)` — a real exit, not a dropped in-process stream.
- The M19-named VM items that ride on the same fixture: bit-identity of a
  gain chain against in-process `rustamp-dsp` stages, an abrupt
  `ProcessDspChain::terminate()` mid-stream (next `process` latches,
  `Engine::poll` fails the track), the missing-helper degradation, and the
  RFC 0006 §10.3 "reported latency matches measured delay" measurement with
  a real nonzero-latency stage.

## 5. Files and line counts

Counts include blank lines; all inside the owned paths.

| File | Lines | Change |
|---|---:|---|
| `crates/rustamp-dsphost/tests/dsp_roundtrip.rs` | 567 | modified (was 536): `write_sweep`/`read_all` replaced by `write_pcm`/`read_pcm`/`round_trip`; four test bodies restructured onto the lockstep; the fake-host test's write bounded; header docs name the lockstep |
| `crates/rustamp-mediahost/src/dsp.rs` | 821 | modified (was 816): doc only — `write_timeout`'s timeout semantics stated (loud stall, whole-buffer-or-error, never a silent short write), `write` points at the bounded form |
| `crates/rustamp-mediahost/tests/dsp_control.rs` | 251 | modified (was 225): `write_pcm` helper added (mirrors `read_pcm`), the three block writes bounded with the step named; header notes the closed producer side |
| `crates/rustamp-mediahost/tests/dsp_process.rs` | 155 | modified (was 125): the round trip restructured onto the same lockstep (bounded write + read-back per block, two sticky reads kept); the kill test's setup write bounded; `write_pcm`/`read_pcm` helpers added |
| `docs/tasks/M20-roundtrip-rebuild.md` | this card | new |

Diffstat: 4 files, +203/−111 (plus this card). No `Cargo.toml`/`Cargo.lock`
change, no new dependency, no `THIRD_PARTY.md` change. No file under
`crates/rustamp-audio`, `crates/rustamp-dsp`, `crates/rustamp-decoders`,
`crates/rustamp-meta` or any other crate was touched.

## 6. Own choices

1. **Lockstep chunks, not a reader thread.** `DspSession` owns both rings,
   so `write` and `read` both need `&mut self`; a reader thread would either
   own the session (and the writer could not write) or share it behind a
   lock (and the writer would hold the lock while blocked — the deadlock
   again). The single-thread lockstep with bounded chunks sized against the
   ring capacities is the task-sanctioned alternative and matches the
   engine's real usage (M19's adapter is one synchronous block round-trip).
2. **`CHUNK = 257`.** Keeps the old sweep's block size and the 514-sample
   read buffer; any chunk size works for the invariant, 257 keeps the
   back-ring ≤ 257/4096 and the in-ring wrapping on every chunk.
3. **`STALL_WAIT` (30 s) on the writes too.** Same M10-sanctioned form as
   the reads: generous enough that only a real stall reaches it, and the
   expiry message names the step. The explicit short-timeout tests
   (`Some(20 ms)`/`Some(10 ms)`) are untouched.
4. **The bounded variant was verified, not re-added.** It exists since M19;
   this lane verified it against the diagnosis (mirrors the bounded `read`:
   same `remaining(deadline)` pattern, `Ok(0)` → `TimedOut`, closed ring →
   loud disconnect, whole-call deadline), stated its timeout semantics in
   its doc, and wired it into the test surface, which was the actual gap.
5. **Sibling tests touched only where the shape is shared.** `dsp_control.rs`
   (unbounded producer writes in a DSP round trip) and `dsp_process.rs`
   (the same write-then-read split against the fixed back-ring, plus the
   unbounded writes) share the shape; `pcm_transfer.rs`, `dsp_named.rs`,
   `real_decode.rs` and every other test file were not touched
   (`pcm_transfer.rs`'s decoder path never blocks the player writing —
   the host writes, the player reads).

## 7. Gate receipts

- `cargo fmt --all -- --check`: **exit 0** (after one `cargo fmt --all`
  pass that re-wrapped two over-long `write_pcm` calls in `dsp_control.rs`).
- `cargo clippy --offline --workspace --all-targets --all-features -- -D warnings`:
  **exit 0**, no warnings (36.8 s; compiles every edited test target —
  nothing was executed).
- `cargo build --offline --workspace`: **exit 0** (52.2 s).
- **`cargo test` was NOT run** — not the suite, not `--no-run`, not a single
  binary. No test process, example, player, helper binary or reference
  binary was spawned; no VM was touched; no window or audio device was
  opened; no download, no dependency addition, no install, no commit and no
  other git write.

## 8. Boundary receipt

`git status --short` inside this worktree (read-only) lists exactly the four
modified files above plus this card — nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written,
and no file outside the owned paths was modified.

```
$ git status --short
 M crates/rustamp-dsphost/tests/dsp_roundtrip.rs
 M crates/rustamp-mediahost/src/dsp.rs
 M crates/rustamp-mediahost/tests/dsp_control.rs
 M crates/rustamp-mediahost/tests/dsp_process.rs
?? docs/tasks/M20-roundtrip-rebuild.md
```
