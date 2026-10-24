# M22: Green the two resident round-trip suites at their causes (red2)

**Date:** 2026-10-18. **Lane:** `wave70-resident-red2`.
**Worktree:** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave70-resident-red2`.

## Boundary and scope

Write only inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

Owned paths: `crates/rustamp-dsphost/tests/**`,
`crates/rustamp-dechost/tests/**`, `crates/rustamp-mediahost/tests/**`,
`crates/rustamp-mediahost/src/**`, and this card. Touched:
`crates/rustamp-dsphost/tests/dsp_roundtrip.rs`,
`crates/rustamp-dechost/tests/real_decode.rs`, and this card.
`crates/rustamp-audio`, `crates/rustamp-dsp`, `crates/rustamp-decoders`,
`crates/rustamp-core`, `crates/rustamp-ipc` and every other crate were
**not** touched. In particular no `src/` behavior was changed; both fixes
are test-surface corrections to expectations the VM proved wrong.

Static only: **no `cargo test`** — not the suite, not `--no-run`, not a
single binary. No test process, example, player, helper binary or reference
binary was spawned; no VM was touched; no window or audio device was opened;
no download, no dependency addition, no install, no commit and no other git
write. Only `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace --offline` ran (receipts in §6). Every command ran
inside this worktree.

Sources: the M18 diagnosis, the M20 lockstep rebuild, RFCs 0003/0006/0007,
the M10/M12/M13/M17 cards, and the existing clean repository code only. No
Winamp source, SDK material or disassembly was read; no third-party code
copied.

The VM record this lane greens: `dsp_roundtrip` 7 passed / 2 failed,
`real_decode` 6 passed / 1 failed, 0 hung (the M20 lockstep plus the
runner's per-binary bound ended the hangs; what is left is real failure,
not a stall). No per-test VM log reached this lane, so each verdict below
is a static proof with `file:line`, not a guess from a name.

## 1. DSP failure 1 (deterministic): the fake host speaks the handshake in the wrong order

**Test:** `abrupt_host_loss_is_loud_and_never_eos`
(`crates/rustamp-dsphost/tests/dsp_roundtrip.rs:504`).

**Verdict: the test was wrong; the resident/host code was right.** Fixed in
the test; no assertion weakened.

**Reasoning.** The production order is pinned in two places. `serve_dsp`
sends the in-ring `RingReady` first and only then the back-ring offer
(`crates/rustamp-mediahost/src/dsp.rs:310-314`). The player side enforces
the same order: `handshake` sends the in-ring offer, waits for the in-ring
`RingReady`, and only then accepts the back-ring `RingOffer`
(`crates/rustamp-mediahost/src/dsp.rs:657-693` — first `recv` expects
`RingReady`, second `recv` expects `RingOffer`).

The fake host did the opposite. On `DspConfigure` it created the back-ring
**and sent its offer immediately**, before it had ever seen the player's
in-ring offer (old `dsp_roundtrip.rs:517-523`). The player's first
handshake `recv` therefore received a `RingOffer` where it required a
`RingReady` and failed with `InvalidMessage("expected in-ring Ready")` —
a deterministic `DspSession::open` failure, i.e. a red test that never
reaches its real assertion (loud disconnect, never EOS).

**Fix (test only).** The fake host now buffers the back-ring offer on
`DspConfigure` and sends it only after answering the in-ring `RingOffer`
with its `RingReady` (new `dsp_roundtrip.rs:517-538`), mirroring
`serve_dsp`. The handshake shape, the published block, the `DspReady`,
the `die` release, the infinite read expecting `Disconnected`, and the
never-`Ok(0)` assertion are unchanged.

## 2. DSP failure 2 (load-flake): two 5 s direct-ring bounds the rebuild missed

**Tests:** `abrupt_player_loss_stops_the_host_without_hanging`
(`dsp_roundtrip.rs:481-489`) and the same file's abrupt-host fake-host
write (`dsp_roundtrip.rs:541-544`).

**Verdict: the test bounds were wrong (too tight); the resident was right.**
Fixed in the test; no assertion weakened.

**Reasoning.** M20 bounded every functional PCM wait with `STALL_WAIT`
(30 s, the M10-sanctioned form: generous enough that only a real stall
reaches it, and expiry names the step). Three direct ring operations in
the two abrupt tests were missed: they used `Some(Duration::from_secs(5))`
with a bare `unwrap()`. The M10 note proves 5 s flakes under VM load for
exactly this shape (a healthy-but-slow schedule tripping a wall-clock
bound). A 5 s expiry here reports a timeout where the honest verdict is
"still progressing", i.e. a load-flake red, not a resident defect.

**Fix (test only).** All three use `Some(STALL_WAIT)` now
(`dsp_roundtrip.rs:483,488,543`). The operations, the capacities (1 and 3),
the values written/read, and the loud-disconnect assertions are unchanged.
`death.recv_timeout(30 s)` already equalled `STALL_WAIT` and was left
alone.

## 3. Decoder failure (expectation): the direct comparison assumed one call fills the request

**Test:** `seek_inside_matches_direct_decode`
(`crates/rustamp-dechost/tests/real_decode.rs:291`).

**Verdict: the test expectation was wrong; the resident was right.** Fixed
in the test; nothing weakened — the count (100 frames) and the bit-exact
equality still pin the contract.

**Reasoning.** The hosted side already loops to collect the 100 post-seek
frames (`real_decode.rs:300-309`: `while got.len() < 200`, asserting
`frames > 0` each iteration), because a ring read returns what is
available, not the whole request. The direct side did the opposite: a
single `direct.read(&mut want)` followed by
`assert_eq!(direct_frames, 100)` (`real_decode.rs:317-322` before this
lane). That asserts a fill guarantee the `Decoder::read` contract does not
make — a backend may return packet-sized short reads — so a healthy seek
that delivers 100 frames in two short reads fails the comparison setup,
not the resident. The hosted-vs-direct equality (`assert_eq!(got, want)`)
then compares a correctly gathered 100 against a short first chunk.

**Fix (test only).** The direct side now loops exactly like the hosted
side (new `real_decode.rs:311-328`): collect until 200 samples, assert
`frames > 0` per iteration, truncate, and assert the length is 200. The
seek target (1000 ms), the expected `reached == 1000`, the 100-frame count
and the exact `got == want` comparison are unchanged.

## 4. The lockstep still blocking anywhere? No — and why that is stated plainly

The more important finding would be a remaining block in the M20 lockstep.
There is none in the owned surface, by the same invariant M20 proved:

- The producer never starts chunk n+1 before the consumer drained chunk n
  (`round_trip`, `dsp_roundtrip.rs:202`). Frames in flight never exceed one
  `CHUNK` (257).
- The back-ring therefore holds at most 257/4096 frames, so the host's
  back-ring write (`write_from_blocking(..., Some(CONTROL_POLL))` in
  `pump`, `dsp.rs:201-204`) always finds space immediately and never stops
  draining the in-ring.
- The producer's `write_timeout(..., Some(STALL_WAIT))` always finds room
  (the host drains within its 20 ms control-poll slices); the consumer's
  `read(..., Some(STALL_WAIT))` always finds data (the host processes what
  the in-ring holds).
- Every wait that waits on the host carries `STALL_WAIT` with the step
  named; the only `None` read left is the abrupt-host test's intentional
  infinite read, bounded from outside by `JOIN_WAIT` (`dsp_roundtrip.rs:567-570`)
  and woken by the control-EOF abort. No test can hang past its bound.

If the VM still names a stall after this lane, it is a new defect, not this
one: the three fixes above remove the deterministic handshake failure, the
known-flaky 5 s bounds, and the short-read assumption, and change no
progress shape.

## 5. Assertions kept (none weakened)

- DSP: bit-exact passthrough over 4099 frames, exact ×2.0/×0.5 gain math per
  sample, `latency_frames() == 0`, spec echo, both generations unchanged on
  reset and both bumped on reconfigure, `frames_dropped() == 0`, sticky EOS
  reads, loud refusals before any ring (unknown stage, bad params), clean
  `Shutdown` vs loud `Disconnected` on player/host loss — all kept.
- Decoder: handshake (`44_100 Hz`/stereo, `duration_ms 1500`, seekable,
  title), full-file bit-identity with the same in-process decoder, zero
  drops, sticky EOS, drained-before-EOS, clamped past-the-end seek that
  terminates, exact post-seek bit-identity, loud `OperationFailed` for
  garbage/empty with a loud host end, backend short-EOS preserved
  bit-exactly with sticky EOS — all kept.
- The only changes are the three expectation corrections above (§1–§3);
  each keeps the count, the values and the loudness it pins.

## 6. Gate receipts

- `cargo fmt --all -- --check`: **exit 0**.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **exit 0**, no warnings.
- `cargo build --workspace --offline`: **exit 0**.
- **`cargo test` was NOT run** — not the suite, not `--no-run`, not a single
  binary. No test process, example, player, helper binary or reference
  binary was spawned; no VM was touched; no window or audio device was
  opened; no download, no dependency addition, no install, no commit and no
  other git write. Every command ran inside this worktree.

## 7. Files and line counts

Counts include blank lines; all inside the owned paths.

| File | Lines | Change |
|---|---:|---|
| `crates/rustamp-dsphost/tests/dsp_roundtrip.rs` | 574 | modified (was 567): fake-host handshake reordered to production order; three 5 s direct-ring bounds widened to `STALL_WAIT` |
| `crates/rustamp-dechost/tests/real_decode.rs` | 428 | modified (was 418): direct post-seek comparison loops to 100 frames instead of assuming one call fills |
| `docs/tasks/M22-resident-red2.md` | this card | new |

No `Cargo.toml`/`Cargo.lock` change, no new dependency, no `THIRD_PARTY.md`
change. No file under `crates/rustamp-audio`, `crates/rustamp-dsp`,
`crates/rustamp-decoders`, `crates/rustamp-core`, `crates/rustamp-ipc` or
any other crate was touched.

## 8. Boundary receipt

`git status --short` inside this worktree (read-only) lists exactly the two
modified test files above plus this card — nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written,
and no file outside the owned paths was modified.
