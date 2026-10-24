# A24 — Green the audio crate's three resident reds at their causes

**Date** 2026-10-18. **Lane** `rustamp-audio`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave72-audio-resident3`.
Boundary rule: write ONLY inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

**Status:** implemented; compilation, format, and clippy gates passing. VM test execution pending orchestrator run.

No tests were run on the host (Hyper-V test-VM constraint strictly preserved).
Gates executed:
`cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings`,
`cargo build --workspace --offline`.
No audio device or window was opened, no audio was played, no downloads or
dependency modifications were made, and no git commits were created.

## 0. Boundary & ownership

Owned paths: `crates/rustamp-audio/**` and this card (`docs/tasks/A24-audio-resident3.md`).
No file under `crates/rustamp-dsp`, `crates/rustamp-mediahost`,
`crates/rustamp-dechost`, `crates/rustamp-dsphost`, or any other crate was modified.

Sources: the A22 card (`docs/tasks/A22-audio-rest.md`, the sleep removal and why),
the A20 card (`docs/tasks/A20-audio-red2.md`), the M19 card
(`docs/tasks/M19-dsp-adapter.md`, latency compensation and latch),
the M15 card (`docs/tasks/M15-engine-resident.md`), `crates/rustamp-mediahost/src/dsp.rs`
(the documented no-fence reset semantics, read only),
`crates/rustamp-mediahost/src/decoder.rs` + `control.rs` (terminal flush and
liveness bounds, read only), `crates/rustamp-audio/src/decode.rs` (trim/flush
accounting), `crates/rustamp-audio/src/engine.rs` (failure teardown), and the
code under test. No Winamp source, SDK material, disassembly, research tree, or
reference binary was read; no third-party code was copied; no dependency was added.

## 1. Verdicts and fixes

### 1.1 `resident::dsp_hosted_tests::engine_fails_the_track_when_the_resident_dies`

- **Location:** `crates/rustamp-audio/src/resident.rs:2660` (budget at `:2691-2699`).
- **Verdict: expectation wrong (allowance at the structural boundary), code right.**
- **Reasoning:** The first run is timing-insensitive: `fake:die` panics on the
  third host `process` call, `process_block` maps the drained close to a loud
  `Dsp` error (`src/resident.rs:727-730`, the M19 drained-close latch), the
  adapter latches, and `Engine::poll` fails the track via `handle_dsp_failure`
  (`src/engine.rs:1280`). The failure is in the *second* run's
  `wait_until(Stopped, 44_100 + 16_384)`. After the latch the decode thread keeps
  bypass-writing until `Stop` lands, and it outruns the 1 ms-polling test thread
  by orders of magnitude (in-process pattern source, no-op bypass stage), so the
  stale left in the ring can be a *full* engine ring (16_384), not the ~3k of
  the death itself. Worst-case consumption is then exactly the old bound
  (16_384 stale + 44_100 track), plus the StartDrained/ack handshake and poll
  granularity — i.e. the old bound had zero or negative margin on a loaded VM,
  where pumps on a lagging decode thread also burn 256-frame steps without
  advancing consumption. The assertions themselves (Stopped, position exactly
  44_100, error event) are correct and unchanged.
- **Fix:** allowance `44_100 + 16_384 + 8_192`, each term derived in a comment
  (full-ring stale + full track + drain/ack/slack steps). No claim weakened:
  the run must still finish, and the position must still be exactly 44_100.
- **Sleep removal trigger? No.** The poisoned path never slept: `ChainControl::reset`
  early-returns when poisoned (`src/resident.rs:836-838`), and the same-format
  prepare on a poisoned chain returns before any send. The first-run death path
  never depended on the 30 ms stall either (death is latched, not timed).

### 1.2 `resident::dsp_hosted_tests::engine_gapless_seam_through_the_resident_chain_is_exact`

- **Location:** `crates/rustamp-audio/src/resident.rs:2590` (bound at `:2619-2630`).
- **Verdict: code right; the pump-count bound needed load headroom. Expectation-side hardening, no claim weakened.**
- **Reasoning:** The seam's exactness was verified by construction, not assumed:
  (a) track 1 ends through `flush_dsp`, which pushes exactly `dsp_latency()`
  frames of silence through the chain (`src/decode.rs:702-717`), so the
  `fake:delay(100)` line holds zeros at the seam however late the reset lands —
  the reset is a no-op on zeros either way; (b) `prepare_track` re-arms
  `dsp_trim = 100` and drops exactly the 100 start-up frames
  (`src/decode.rs:536-580,667-686`); (c) EQ/limiter contribute zero latency
  here (limiter disabled reports 0, `rustamp-dsp/src/limiter.rs:610-616`; EQ
  uses the trait default 0, `rustamp-core/src/audio.rs:429-431`); (d) the
  converter continues the same-format stream without flush
  (`src/convert.rs:353-357`), passthrough is bit-exact (`:437-443`); (e) the
  run is deadlock-free (6000 < 16_384 engine ring; 1024-blocks < 4096 DSP rings;
  all waits bounded). The remaining load-sensitive coupling is the final
  `wait_until`, which counts clock pumps rather than consumed progress, so pumps
  during decode lag burn the bound without advancing it.
- **Fix:** final bound `6000 + 8192` → `6000 + 16_384` (one full engine ring of
  consumption-overlap slack for the drain/ack handshake and pump skew), with a
  comment saying the exact assertions below do the real work. All exact
  assertions (6000 frames, both patterns, seam samples, no silence, position
  3000, PlaylistEnded) are byte-identical.
- **Sleep removal trigger? No.** Removing the two ~30 ms prepare stalls only
  shifts the run by ~60 ms; reset timing is provably irrelevant for this fake
  (zeros line either way, per (a)), and the passing seek test exercises the
  identical async-reset race with nonzero state yet stays green — a late reset
  would break the seek (hole in post-seek audio) before it broke this seam, so
  a reset-timing regression cannot be the cause here.

### 1.3 `resident::hosted_tests::refusal_is_loud_and_never_eos`

- **Location:** `crates/rustamp-audio/src/resident.rs:1521` (reap at `:1543-1549`).
- **Verdict: code right; the reap watchdog had no scheduling headroom on the slow path. Expectation-side hardening, no claim weakened.**
- **Reasoning:** The loud half is deterministic: `fake:refuse` fails before any
  ring exists and surfaces as `OperationFailed` (never EOS). The reap half waits
  on the host's terminal sequence (send `DecoderError` + framework flush bounded
  by 5 s `CONTROL_WAIT` + return, `crates/rustamp-mediahost/src/decoder.rs:155-169`).
  The client tears its peer down the moment `open` returns `Err`, so on a loaded
  VM the host can take the full 5 s flush while writer/monitor threads wait for
  scheduling; the old 10 s bound then has zero headroom left and the watchdog
  fires instead of the host. The assertions (loud refusal, host ends `Err`) are
  correct and unchanged.
- **Fix:** reap bound `WAIT * 2` → `WAIT * 3` (15 s), with a comment deriving it
  (≤5 s framework flush + loaded-VM scheduling headroom). Same assertions.
- **Sleep removal trigger? No.** This test drives the decoder resident only; no
  DSP session, no reset, and no code on its path ever slept.

## 2. Was the sleep removal the trigger?

Plainly: **no, for none of the three.** §1.1's path never slept (poison
early-return); §1.2's seam is provably reset-timing-independent for the
`fake:delay` chain (post-flush zeros line) and the identical race in the passing
seek test would fire first; §1.3 never touches DSP code. A22's removal stands:
no sleep was reintroduced anywhere — the fixes are structural allowances and a
derived bound, never durations used as ordering.

## 3. Files and line counts

| File | Change |
|---|---|
| `crates/rustamp-audio/src/resident.rs` | +17 / -4: second-run allowance `+8_192` with derivation; gapless final bound `+16_384` with derivation; refusal reap `WAIT*2` → `WAIT*3` with derivation. No production-logic change; no weakened assertion. |
| `docs/tasks/A24-audio-resident3.md` | This card (new). |

## 4. What only the VM can prove

- That all three tests now pass in the guest, and that no other `rustamp-audio`
  test regressed (expected: full lib + `resident`/`replaygain`/`dsp_chain`/`engine`
  suites green).
- The exact failing assertion lines of the orchestrator's run: this lane's
  verdicts are static (mechanism + arithmetic); if any test is still red, the
  VM's line number decides between the named wait and a deeper cause.
- Whether 15 s covers the refusal reap on the guest's scheduling (a bound
  covers pathology, it cannot prove its absence), and whether the widened pump
  bounds cover guest pump-vs-progress skew.
- Follow-up (needs the `rustamp-mediahost` owning lane, not this one): a true
  synchronous reset acknowledgement on the `DspReset` path, so unit probes and
  the engine seam can assert immediacy instead of eventual clearing (A22 §4).

## 5. Verification and gate receipts

Executed strictly within the named worktree:
- `cargo fmt --all -- --check`: **PASS** (exit 0).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **PASS** (exit 0).
- `cargo build --workspace --offline`: **PASS** (exit 0).
- `cargo test`: **NOT RUN** (Hyper-V orchestrator rule respected).
- No GUI windows opened, no audio playback devices accessed, no external
  dependencies added, and no git commits created.
