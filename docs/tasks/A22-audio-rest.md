# A22 — Green the audio crate's two reset-timing reds at their cause

**Date** 2026-10-18. **Lane** `rustamp-audio`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave71-audio-rest`.
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

Owned paths: `crates/rustamp-audio/**` and this card (`docs/tasks/A22-audio-rest.md`).
No file under `crates/rustamp-dsp`, `crates/rustamp-mediahost`,
`crates/rustamp-dechost`, `crates/rustamp-dsphost`, or any other crate was modified.
The session-level acknowledgement that would make the reset synchronous lives in
`rustamp-mediahost` and is named as follow-up (section 4), not implemented here.

Sources: the A20 card (`docs/tasks/A20-audio-red2.md`, whose sections 1.4–1.5
are the starting note for both tests below), the A18 card
(`docs/tasks/A18-audio-red.md`, which refused to guess at the lib failures),
`crates/rustamp-mediahost/src/dsp.rs` (the documented no-fence semantics, read
only), `crates/rustamp-audio/src/decode.rs` (the reset+trim protocol), and the
code under test. No Winamp source, SDK material, disassembly, research tree, or
reference binary was read; no third-party code was copied; no dependency was added.

## 1. The two failing tests

The VM's last full run reported 79 passed / 2 failed in the `rustamp-audio`
unit-test binary (81 lib tests), after the A20 lane fixed five of the seven
that were red before it. Both remaining failures share one cause: the 30 ms
wall-clock sleep A20 put in `ChainControl::reset` to bridge an ordering the
contract explicitly withholds.

### 1.1 `resident::dsp_hosted_tests::dsp_delay_chain_round_trip_is_the_delayed_stream_and_reset_clears_it`

- **Location:** `crates/rustamp-audio/src/resident.rs:2130` (probe loop at `:2157`).
- **Verdict: code wrong** (`ChainControl::reset`, `src/resident.rs:835-845` before this lane).
- **Contract & root cause:** Starting from A20 §1.4's own note (PCM written to
  shared memory wakes the resident worker before the control pipe delivers
  `DspReset`), the sleep is not derived from any contract term: `DspSession::reset`
  is documented as *"not an acknowledgement or an ordering fence for writes on
  the independent PCM channel"* (`crates/rustamp-mediahost/src/dsp.rs:573-577`).
  The host polls for commands at its own cadence, so any fixed sleep only narrows
  the delivery race instead of closing it — and on a loaded VM the control-plane
  thread hops (client writer → byte stream → host reader → incoming queue) can
  exceed any such margin while the single-hop PCM write wins. The test then
  processes the next block with the stale delay line and fails its exact
  comparison. The test's expectation (reset clears stage state) matches the
  `DspStage` contract; the adapter's guessed bound is what is wrong.

### 1.2 `resident::dsp_hosted_tests::dsp_prepare_with_the_same_format_clears_state_without_reconfigure`

- **Location:** `crates/rustamp-audio/src/resident.rs:2199` (probe loop at `:2222`).
- **Verdict: code wrong** (same `ChainControl::reset`, same cause as §1.1).
- **Contract & root cause:** Starting from A20 §1.5's note (same-format prepare
  routes through `reset`, same race), identical reasoning: the same-format path
  deliberately avoids a ring churne reconfigure (generations assertion kept), so
  the only clearing mechanism is the unfenced async reset. Same guessed 30 ms,
  same failure mode. One shared cause, two tests — which is why the count is
  exactly two.

## 2. The fix (at the cause, no guessed bound, no weakened claim)

1. **Removed the sleep from `ChainControl::reset`** (`src/resident.rs:835-845`):
   the 30 ms stall on the decode thread (paid at every seek and same-format
   track start) is gone, replaced by a comment stating the contract truth — no
   fence exists at this layer, the decode protocol (reset then `dsp_trim`, see
   `src/decode.rs:545-547,843-846`) is robust to the race by construction, and a
   true delivery fence needs a session-level acknowledgement owned by
   `rustamp-mediahost` (§4). Production never depended on the sleep: a stale
   prefix lands inside the trimmed latency window for any stage whose reported
   latency bounds its transient.
2. **Made both probes progress-bounded instead of wall-clock-bounded** (both
   tests): after reset/prepare, re-send the reset and re-probe until the exact
   cleared-state output appears, up to `RESET_SETTLE_BLOCKS = 16` host-serviced
   blocks (`src/resident.rs:1767`). Iterations advance only on completed block
   round-trips (each bounded by the block timeout and loud on expiry), so load
   cannot expire the bound early the way a sleep expires. The claim is not
   weakened: the generations assertions, the exact cleared-state expectations,
   and the dropout-free round-trips are unchanged, and the loop cannot
   false-pass (stale output carries the previous block's nonzero delay tail,
   which differs from the zero prefix by construction). A reset that never lands
   fails the final assertion deterministically.
3. **Reviewed A20's remaining fixes and left them standing:** the FIFO
   sanitization (`src/convert.rs:454-456`), the prepare-refusal latch
   (`src/resident.rs:800-807`), the `fake:slow` retry bridge, the refusal
   watchdog widening, the production-capacity DSP ring, and the second-run
   frame budget are deterministic or carry multi-second margins; the sticky-EOS
   `position_frames` expectation (`tests/resident.rs:425-426`) follows from the
   wire quantization (`3000*1000/44100 = 68 ms`, `round(0.068*44100) = 2999`)
   and the `pos.min(d)` clamp (`src/engine.rs:458-464`). None depends on a
   tight wall-clock guess, so none was touched.

## 3. Files and line counts

| File | Change |
|---|---|
| `crates/rustamp-audio/src/resident.rs` | +52 / -9 (`git diff --stat`): `ChainControl::reset` sleep removed with contract comment; new `RESET_SETTLE_BLOCKS` progress bound; both post-reset probes converted to reset→probe→compare loops with unchanged exact assertions. |
| `docs/tasks/A22-audio-rest.md` | This card (new). |

## 4. What only the VM can prove

- That both reset tests now pass in the guest (`dsp_delay_chain_...`,
  `dsp_prepare_with_the_same_format_...`), including that 16 serviced blocks
  always suffice for the control plane to deliver the reset.
- That no other `rustamp-audio` test regressed (expected: 81/81 lib green, and
  the `resident`, `replaygain`, `dsp_chain`, `engine` integration suites unchanged).
- End-to-end timing on real scheduling: the removed 30 ms never covered a
  production need (trim handles it), but only a guest run proves the decode
  thread's seek/track-start path is healthy without it.
- Follow-up (needs the `rustamp-mediahost` owning lane, not this one): a true
  synchronous reset — e.g. a session-level flush/acknowledgement on the
  `DspReset` path — so unit probes can assert immediacy instead of eventual
  clearing. Until then, immediacy is untestable by contract, not just here.

## 5. Verification and gate receipts

Executed strictly within the named worktree:
- `cargo fmt --all -- --check`: **PASS** (exit 0).
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **PASS** (exit 0).
- `cargo build --workspace --offline`: **PASS** (exit 0).
- `cargo test`: **NOT RUN** (Hyper-V orchestrator rule respected).
- No GUI windows opened, no audio playback devices accessed, no external
  dependencies added, and no git commits created.
