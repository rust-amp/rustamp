# M18: Hang hunt — the blocking-wait audit of the resident test surface

**Date:** 2026-10-17. **Lane:** wave60-mistral-hang (hang hunt, read-mostly).
**Worktree:** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave60-mistral-hang`.

## Boundary and scope

Write only inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.
Owned paths: `crates/rustamp-mediahost/tests/**`,
`crates/rustamp-dechost/tests/**`, `crates/rustamp-dsphost/tests/**`,
`crates/rustamp-audio/tests/**` and this card. `docs/notes/**`, the broker,
the runner scripts, every `src/` file and every other crate were **not**
touched; where a fix belongs in a `src/` file it is named below with
`file:line` instead of being made.

The hunt is static: **no `cargo test`, no test binary, no example, no player,
no VM, no window, no audio, no process spawn, no download, no dependency
addition, no commit.** Only `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace --offline` ran (receipts in §7).

## The run record, as evidence

- r32 was staged with the DSP resident (`d76a1be`), the tray (`4682139`),
  the decoder resident (`68c72f8`, after `cdead5f` greened its two failing
  tests) and the engine-side adapter (`a9b80d3`). It hung inside the guest
  test script and never returned. r33 never started (the broker was held by
  r32). r34, a few merges later (up to `e2a8f3b`, still before the M17
  condition-wait change `0a22649`), hung the same way.
- The guest script (`docs/notes/vm-broker/guest/run-tests.ps1`) runs each
  staged binary synchronously (`& $exe`), sorted by name, with no per-binary
  timeout and no progress line, and without `--ignored`. So the `#[ignore]`d
  two-process tests (`pcm_process.rs`, `dsp_process.rs`) are **not** in the
  main suite, and one test thread that never returns hangs the whole run.
- The last run that *returned* was the red run behind `cdead5f` ("67 runs,
  5 binaries, 7 tests"): six-lane batch plus the decoder resident. It
  completed. Therefore the hang is in something that was **new or unmasked**
  at r32: `cdead5f`'s fixes, the DSP resident (`dsp_roundtrip.rs`,
  `dsp_named.rs`), the engine adapter (`resident.rs`), the tray, or a test
  that failed *early* in the red run and only reached its waiting phase after
  `cdead5f` (`buffered_device_drops_tail_on_stop_preventing_stale_replay` in
  `engine.rs`, and `seek_inside_matches_direct_decode` in `real_decode.rs`).
- The M10 note (flakiness of `format_change_drains_old_pcm_before_new_channel_count`
  at a 5 s bound under VM load) proves the guest is slow enough that
  wall-clock bounds that are too tight produce *flaky failures*; it also
  sanctions the fix form used here: "a bound so generous that only a real
  stall reaches it, with the message saying which".
- The boundary "hung before the M17 condition-wait change" matters twice:
  at r32/r34 `pcm_transfer.rs` still had its 5 s watchdogs, and the M17
  change (`0a22649`) *removed* several of them (see candidate 2). The
  current tree therefore carries both the pre-M17 hang candidates and the
  post-M17 relaxations; this card fixes the current tree.

## The audit: every blocking wait in the surface

Inventory (current tree), with the wait, its waker, and whether it can block
forever:

| # | Wait | file:line | Waker | Can block forever? |
|---|---|---|---|---|
| 1 | `done.recv()` (host-join channel receive) | `mediahost/tests/pcm_transfer.rs:225` (`finish`, 4 call sites), `:361` (`decoder_error`), `:415` (`abrupt_player_loss` `result.recv`) | the host thread's `done.send` after `serve_decoder` returns | **Yes** — the sender thread can wedge or panic (e.g. `TestRings::open`'s `assert_eq!` inside the mutex at `:130`, or any host-side panic) and then never send. M17 removed the 5 s `recv_timeout` here. |
| 2 | `observed.recv()` ×4 (host reset callbacks) | `mediahost/tests/dsp_control.rs:127,137,153,181` (now `recv_timeout`) | the host's `chain.reset()` → `resets.send` | **Yes** — if the host never services a reset (thread wedged/panicked). New file in M17; never had a bound. |
| 3 | `release.recv()` / `death.recv()` (gated fixtures) | `pcm_transfer.rs:212` (`GatedDecoder`), `:462`; `dsp_control.rs:109` (`StatefulChain`) | the test thread's `send` | **Yes** — if the test thread panics before releasing, the fixture thread blocks forever (leaked; the *test* only hangs if it is the test thread waiting). |
| 4 | `session.read(&mut out, None)` → `read_into_blocking(None)` | `pcm_transfer.rs:249,265,279,311,330,347,475`; `real_decode.rs` (7 tests, now via `read_pcm`); `dsp_roundtrip.rs` `read_all`/`read_sticky`; `dsp_control.rs:146,159,175,188,206,217,221` | data, ring close, abort | **Yes** — a resident that stops producing without closing the ring and without failing control leaves the read asleep forever. Pre-M17 the *control* leg had a 5 s fallback (`next(None)` → `recv(CONTROL_WAIT)` → `TimedOut`); M17's `recv_blocking` (`control.rs:291`) made it truly infinite. |
| 5 | `session.write(&block)` → `write_from_blocking(None)` | `dsp.rs:487` reached from `dsp_roundtrip.rs` `write_sweep` (`:151,318,354`), `dsp_control.rs:144,157,204`, `dsp_process.rs:54,108` | ring space (the host's reads) or close | **Yes** — the only *producer-side* infinite wait in the surface; see candidate 1. The `DspSession::write` API takes no timeout, so the test surface cannot bound it (named src fix below). |
| 6 | `wait_readable(None)` | `pcm_transfer.rs:409` (now `Some(STALL_WAIT)`) | data or close | **Yes** — same class as 4. |
| 7 | `player.recv()` / `transport.recv()` (raw `StreamTransport`, blocking, no deadline) | `dsp_roundtrip.rs:412,439` (fixed handshake), `:478` (fake-host thread) | next peer message or EOF (`Err(Closed)`) | **Only if the peer never sends and never closes.** On the fake-host thread this leaks a thread, not a test hang; on the test thread (pre-fix `abrupt_player_loss`) it was a latent panic, not a hang (see finding F-α below). |
| 8 | Four deadline-less `while` condition loops | `audio/tests/engine.rs:2318,2356,2366,2401` (`buffered_device_drops_tail_on_stop_preventing_stale_replay`: track-open ×2, `buffered_frames() < 1024`, `< 256`) | the engine's decode thread / render callback | **Yes** — the only unbounded condition-waits in `rustamp-audio/tests`; unmasked by `cdead5f` at r32. Fixed with 5 s deadlines (§6). |
| 9 | Engine-side waits | `audio/src/ring.rs:275` `wait_for` (250 ms bound), `engine.rs:1366` `handle.join()` (unbounded, but the worker's blocking calls are bounded: `HostDecoder::read`/`seek` use 5 s timeouts, `resident.rs:192,215`) | counter atomics / control messages | No — bounded by construction; the join completes because `join_decode` sets `active(false)` first. |
| 10 | Bounded-by-design waits (not candidates) | `pcm_transfer.rs` `peer.recv(WAIT)` (`:512,:538`), `missing_pong` `recv_timeout(WAIT)` (`:516`), `pcm_process.rs:95` / `dsp_process.rs:123` `recv_timeout(10s)`, `real_decode.rs`/`dsp_roundtrip.rs` `recv_timeout(JOIN_WAIT)` joins, `dsp_roundtrip.rs:447,452` `Some(5s)` ring ops, `resident.rs` 5 s poll loops, `common/mod.rs` 5 s helpers, `ipc` `TEST_WAIT` waits and scoped joins, `flush_contract.rs` `wait`/`drive_to_seek_result` 5 s deadlines, `render_rt.rs`/`output_error.rs`/`replaygain.rs`/`limiter_gain.rs`/`dsp_chain.rs` (bounded or pure computation) | — | No. |

## Candidates ranked by likelihood of being the r32/r34 hang

### 1. `dsp_roundtrip.rs` — the write-then-read split against the fixed 4096-frame back-ring (my bet)

`crates/rustamp-dsphost/tests/dsp_roundtrip.rs` (`write_sweep` + `read_all`),
against `BACK_RING_FRAMES = 4096` (`rustamp-mediahost/src/dsp.rs:36`) and the
test's in-ring capacities 3/7.

**The interleaving:** `write_sweep(&mut session, TOTAL=4099)` writes *all*
4099 frames before `read_all` reads a single one. The player blocks in
`DspSession::write` → `to_host.write_from_blocking(..., None)`
(`src/dsp.rs:487`) whenever the in-ring (capacity 3) is full; the host blocks
in its back-ring write (`src/dsp.rs:201-204`) once 4096 frames sit unread in
the back-ring, holding at most one in-ring-sized block. That is the textbook
bounded-buffer deadlock: player waiting for the host to drain the in-ring,
host waiting for the player to drain the back-ring, nobody reading.

**The arithmetic:** the host can absorb `4096` (back-ring) + `h` (its current
block, `1 ≤ h ≤ 3`, it always holds at least one frame when it blocks) +
`3` (in-ring) = up to 4102 frames, against 4099 written. The margin is
**1–3 frames** — `TOTAL = 4099 = BACK_RING_FRAMES + in-ring capacity` exactly.
So the current code does *not* deadlock on the happy path (I re-derived this
four times, including the one-frame-at-a-time write pattern of
`reset_keeps_rings_and_continues` and the 257-frame chunks of the others), but
the test sits precisely on the deadlock boundary: one frame of chain latency,
one extra block of host buffering, one capacity change, or one frame lost to
an interleaving turns it into a permanent hang. `JOIN_WAIT` does not save it —
the join sits *after* `read_all` in the test body, so a blocked `write_sweep`
or `read_all` hangs the test thread before any join is reached.

**Evidence for:** first executed at r32 (M13 was compile-verified only — the
M13 card says "written, not executed here"); it is the only file in the
surface with an infinite *producer-side* wait (`DspSession::write` — the
decoder path never blocks the player writing, only the DSP path does); the
deadlock shape is unique in the surface; both hung runs staged it.
**Evidence against:** the current arithmetic leaves a positive margin, so a
deterministic hang needs the margin eaten or a scheduling pathology; the
file also contains a latent *panic* (F-α below) that would have made the
binary report a failure rather than hang — unless the panic and the hang are
in different tests of the same binary, which they are (the panic is in
`abrupt_player_loss_stops_the_host_without_hanging`; the split is in
`passthrough_round_trip_is_bit_exact_and_eos_is_sticky`,
`gain_chain_changes_audio_measurably`, `reset_keeps_rings_and_continues`).

### 2. `pcm_transfer.rs` — M17 removed the 5 s watchdogs on the host-join receives

`finish()`'s `done.recv()` (was `done.recv_timeout(WAIT)`), `decoder_error`'s
`done.recv()`, `abrupt_player_loss`'s `result.recv()` and `wait_readable(None)`
— all relaxed to unbounded by `0a22649` (diff verified). A host thread that
wedges or panics now hangs the test **forever** instead of failing in 5 s.
This is the clearest "bounded wait removed" in the surface, but it is **not**
the r32/r34 hang (those runs predate M17 and had the bounds). It is the top
*current-tree* hazard, and it is fixed (§6).

### 3. `engine.rs` `buffered_device_drops_tail_on_stop_preventing_stale_replay` — four deadline-less loops

`crates/rustamp-audio/tests/engine.rs` (the two track-open loops, the
`buffered_frames() < 1024` loop, the `buffered_frames() < 256` loop). These
are the only unbounded condition-waits in `rustamp-audio/tests` — every
sibling loop in the file (and the drain loop inside this very test) has a 5 s
deadline. The test failed *early* in the red run (at the bit-for-bit
comparison); `cdead5f` unmasked it by adding the drain loop, so at r32 it
ran further than ever before. In the happy path the loops terminate
(`buffered_frames()` reports the stale backlog while the start drain is
pending, so the `< 1024` loop exits immediately), but any engine wedge —
decode thread not writing after the drain, track never opening — spins
forever with no report. Fixed with 5 s deadlines (§6).

### 4. `real_decode.rs` — infinite PCM reads (pre-M17 control leg had a 5 s fallback)

`session.read(&mut buf, None)` in all seven tests. The ring wait is woken by
data/close/abort; the control leg (`DecoderSession::next`) pre-M17 fell back
to `recv(CONTROL_WAIT)` → `Err(TimedOut)` (a *failure*), post-M17 it is
`recv_blocking` (truly infinite). A resident that stops delivering without a
terminal signal hangs the read. The host paths all close the ring or fail
control on every terminal path (verified in `serve_inner`), so this needs a
host-side wedge to hang — less likely than 1–3, but the file's
`seek_inside_matches_direct_decode` was also unmasked by `cdead5f` (its
pre-fix version failed at `assert_eq!(frames, direct_frames)` before ever
looping). Fixed with stall bounds (§6).

### 5. `dsp_control.rs` — `observed.recv()` ×4 and `finish`'s `done.recv()`

New in M17, never bounded, never executed before M17. Reliable in the happy
path (the 20 ms slices service resets even while idle), but a host wedge
hangs four waits plus the join. Not the r32 hang (the file did not exist).
Fixed (§6).

### 6. `dsp_roundtrip.rs` `player.recv()` (raw transport, no deadline) — and finding F-α

**F-α (found while fixing, latent red test):** the pre-fix
`abrupt_player_loss_stops_the_host_without_hanging` asserted
`player.recv() == RingReady{..}` as the *first* message, but the host's
`ControlPeer::start` queues its own `Hello` first (`control.rs:125-127`), so
the test would panic on `Hello != RingReady` the first time it ever ran —
which was r32. It is fixed in §6 by consuming the Hello and answering pings
(mirroring the proven decoder-side pattern in `pcm_transfer.rs:387-404`),
keeping every assertion. This is a correctness fix, not a weakening.

### 7. Not candidates (checked, bounded or waker-guaranteed)

`resident.rs` (engine adapter tests): every wait is a 5 s-deadline poll loop;
`ReapedDecoder::drop` uses `recv_timeout`. `common/mod.rs` helpers: 5 s
deadlines. `flush_contract.rs`: 5 s deadlines. `pcm_named.rs`/`dsp_named.rs`:
1 s writes, `None` reads only with data present or a closed ring. `pcm_process.rs`/
`dsp_process.rs` kill tests: `recv_timeout(10 s)`. `rustamp-ipc` tests: `TEST_WAIT`
bounds and scoped joins. `ring.rs` handshake: 250 ms bound. The tray and
`rustamp-platform` (staged at r32 but outside this card's surface) are bounded
by inspection (`recv_timeout(5 s)`, `STOP_TIMEOUT`/`FORWARD_TIMEOUT` deadlines,
detach-on-drop joins).

## My bet, and what would confirm it

**Bet: candidate 1 — `dsp_roundtrip.rs`'s write-then-read split against the
fixed 4096-frame back-ring — is the file that hung r32/r34**, with the actual
blocking wait being either the player's infinite `DspSession::write`
(in `write_sweep`) or the host's infinite back-ring write meeting a full
back-ring, i.e. the deadlock shape whose margin is exactly the in-ring
capacity (4099 = 4096 + 3). The margin arithmetic says the *current* code
survives by 1–3 frames, so the honest version of the bet is: the hang is in
this file, in one of the three tests that write everything before reading
anything, and the trigger is whatever eats that 1–3-frame margin in the guest
(a scheduling interleaving where the host's in-hand block plus the in-ring
occupancy plus one frame of converter/chain buffering exceeds 4102 — or a
subtly different interleaving than the ones I could enumerate statically).
Candidates 2, 3 and 5 are real hazards but cannot be the r32/r34 hang
(bounded or absent at r32); candidate 3 and 4 became reachable only through
`cdead5f`, which fits the timeline but not the mechanism.

**A single guest run with per-binary progress would confirm it in one
step:** the runner already sorts binaries by name, so the first binary whose
name is printed without a following `test result:` line is the hanging one.
Expected: `dsp_roundtrip-<hash>.exe` (alphabetically after `dsp_named` and
before `engine`/`flush_contract`/…), hanging inside one of
`passthrough_round_trip_is_bit_exact_and_eos_is_sticky`,
`gain_chain_changes_audio_measurably` or `reset_keeps_rings_and_continues`.
If instead the hang is in `engine-<hash>.exe` at
`buffered_device_drops_tail_on_stop_preventing_stale_replay`, or in
`real_decode-<hash>.exe`, candidates 3/4 win; a hang in `pcm_transfer` would
point at candidate 2's mechanism having a pre-M17 trigger after all.

## Fixes made (test surface only)

Principle (the M10 note's sanctioned form): every wait that waits on the host
now carries a **generous stall escape** — 30 s (`STALL_WAIT`), six times the
bound that flaked under VM load, far beyond any healthy schedule for these
fixtures — and reaching it **fails loudly with the step named, never a
pass**. The explicit short-timeout tests (`Some(20 ms)` / `Some(10 ms)`,
which pin `TimedOut` semantics) are untouched. Reads that are themselves the
assertion under test (the infinite read that must wake on control EOF, bounded
by an outer `recv_timeout`) keep `None`.

- `crates/rustamp-mediahost/tests/pcm_transfer.rs` (495 → 541 lines):
  `finish()` and the two other host-join receives restored to
  `recv_timeout(STALL_WAIT)` with named stall messages; `wait_readable(None)`
  → `Some(STALL_WAIT)`; `GatedDecoder`'s `release.recv()` →
  `recv_timeout(STALL_WAIT)`; the fake host's infinite ring write and
  `death.recv()` bounded; all functional reads go through a new `read_pcm`
  helper that panics with the step name on `TimedOut`.
- `crates/rustamp-mediahost/tests/dsp_control.rs` (155 → 225): new
  `STALL_WAIT`; `finish()`'s `done.recv()` → `recv_timeout`; the four
  `observed.recv()` → `recv_timeout` with the reset number named;
  `StatefulChain`'s `release.recv()` bounded; all reads via a new `read_pcm`
  helper.
- `crates/rustamp-dechost/tests/real_decode.rs` (380 → 418): new
  `STALL_WAIT` + `read_pcm` helper; every functional `read(None)` in all
  seven tests bounded with the step named (the 30 s joins were already
  bounded).
- `crates/rustamp-dsphost/tests/dsp_roundtrip.rs` (466 → 536): new
  `STALL_WAIT`; `read_all` and the sticky-EOS reads (new `read_sticky`
  helper) bounded with named panics; the first read of
  `abrupt_host_loss_is_loud_and_never_eos` bounded; **F-α fixed** —
  `abrupt_player_loss_stops_the_host_without_hanging` now consumes the
  host's `Hello` and answers pings during the handshake (mirroring
  `pcm_transfer.rs`), keeping every assertion; `write_sweep` carries a
  comment naming the src gap.
- `crates/rustamp-audio/tests/engine.rs` (2396 → 2419): the four
  deadline-less loops in
  `buffered_device_drops_tail_on_stop_preventing_stale_replay` now carry 5 s
  deadlines with named stall messages, matching the drain loop and the
  file's other helpers.
- `crates/rustamp-mediahost/tests/pcm_process.rs` (84 → 97) and
  `dsp_process.rs` (105 → 125): the functional reads in the ignored
  two-process tests bounded with `STALL_WAIT` + named panics.

## Fixes named but NOT made (they need a `src/` file)

1. **`crates/rustamp-mediahost/src/dsp.rs:464-503` — `DspSession::write` takes
   no timeout** and hardcodes the infinite `write_from_blocking(..., None)`
   at `:487`. This is the producer-side half of candidate 1: no test can
   bound it. Needed fix: a bounded write variant (e.g.
   `write_bounded(&mut self, buf: &[f32], timeout: Option<Duration>)` or a
   `write_timeout` field on the session), used by `write_sweep` in
   `dsp_roundtrip.rs`, the writes in `dsp_control.rs` and `dsp_process.rs`.
   The decoder path has no such wait (the host writes, never the player).
2. **`crates/rustamp-mediahost/src/control.rs:291-298` —
   `ControlPeer::recv_blocking`** is a truly infinite channel receive, woken
   only by a message or monitor exit. That is correct for the *host* side
   (its monitor owns liveness), but it is what makes a wedged-but-alive peer
   indistinguishable from a healthy idle one for a `read(None)` caller. A
   caller-side stall bound (as the tests now apply) is the test-surface
   answer; a production-side bound would be a policy decision (RFC), not a
   local fix.
3. **`crates/rustamp-mediahost/src/decoder.rs:141` / `src/dsp.rs:201`** — the
   host pump's infinite ring writes are production semantics (the monitor
   closes the ring on seek/shutdown/reconfigure, which wakes them); no change
   proposed.

## Files and line counts

Counts include blank lines; all inside the owned paths.

| File | Lines | Change |
|---|---:|---|
| `crates/rustamp-mediahost/tests/pcm_transfer.rs` | 541 | modified (was 495) |
| `crates/rustamp-mediahost/tests/dsp_control.rs` | 225 | modified (was 155) |
| `crates/rustamp-mediahost/tests/pcm_process.rs` | 97 | modified (was 84) |
| `crates/rustamp-mediahost/tests/dsp_process.rs` | 125 | modified (was 105) |
| `crates/rustamp-dechost/tests/real_decode.rs` | 418 | modified (was 380) |
| `crates/rustamp-dsphost/tests/dsp_roundtrip.rs` | 536 | modified (was 466) |
| `crates/rustamp-audio/tests/engine.rs` | 2419 | modified (was 2396) |
| `crates/rustamp-audio/tests/resident.rs` | 570 | unchanged (audited: all waits bounded) |
| `crates/rustamp-audio/tests/common/mod.rs` | 543 | unchanged (audited: all waits bounded) |
| `docs/tasks/M18-hang-hunt.md` | this card | new |

Diffstat: 7 files, +351/−71. No `Cargo.toml`/`Cargo.lock` change, no new
dependency, no `src/` file touched.

## Gate receipts

- `cargo fmt --all -- --check`: **exit 0** (after one `cargo fmt --all` pass
  over the edited files).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **exit 0**, no warnings (compiles every edited test target; nothing was
  executed).
- `cargo build --workspace --offline`: **exit 0** (45.6 s).
- **`cargo test` was NOT run** — not the suite, not `--no-run`, not a single
  binary. No test process, example, player, helper binary or reference
  binary was spawned; no VM was touched; no window or audio device was
  opened; no download, no dependency addition, no install, no commit and no
  other git write. Every command ran inside this worktree.

## Boundary receipt

`git status --short` inside this worktree (read-only) lists exactly the seven
modified test files above plus this card — nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written,
and no file outside the owned paths was modified.
