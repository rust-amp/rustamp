# A23 — Green the DSP crate's last red test (an equaliser test, not the limiter)

**Date** 2026-10-18. **Lane** `rustamp-dsp`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave71-dsp-rest`.
Write only owned paths inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

**Status:** implemented; host compilation/linting only, VM test execution pending.

Nothing was executed or tested on the host (Hyper-V test-VM rule). Only
`cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace --offline` were used. No window opened, no audio
device opened, no audio played, no download, no dependency addition, no commit
or other git write.

## 0. Boundary

Owned paths: `crates/rustamp-dsp/**` and this card `docs/tasks/A23-dsp-rest.md`.
No file under `crates/rustamp-audio` or any other crate was touched.

## 1. What the VM reported, and what this lane established

The VM's last full run reported the `rustamp-dsp` suite with **32 passed and 1
failed** of 33 tests (16 in `eq.rs`, 17 in `limiter.rs`). A21's card
(`docs/tasks/A21-dsp-red2.md`) attributes the last failure to
`limiter.rs::a_hot_signal_is_held_at_the_ceiling` ("the lane that fixed three and
left one"). That attribution is **wrong**; this card records why.

No test was run on this host. The root cause was established by an exact
numerical simulation of both stages — every test's inputs, the biquads, the
slew/crossfade state machines, the true-peak detector, the hold/attack
envelopes, and the f32 rounding at every step — validated against the VM's own
known outcomes:

* With A21's pre-fix `a6_tone` helper (the misplaced parentheses), the
  simulation reproduces the pre-A21 run's failures **exactly**:
  `the_limiter_pulls_the_torture_tone_down_instead_of_passing_it`,
  `the_true_peak_criterion_is_linked_over_channels` and
  `a_nan_does_not_poison_the_true_peak_detector` fail with precisely the modes
  A21 §1.1–1.3 quotes (sample peak 1.0 instead of the ceiling, no −3.01 dB
  engagement, settled peak ≈ 0.966 instead of ≈ 0.683), and
  `a_hot_signal_is_held_at_the_ceiling` **passes** (its output sample peak is
  the ceiling exactly).
* With A21's fixed helper, all 17 limiter tests pass, including
  `a_hot_signal_is_held_at_the_ceiling` (peak = ceiling, ratio 1.000000000;
  settled true peak 0.96615, inside (0.94, 0.99)).
* The 16 eq tests: 15 pass; exactly one fails —
  `non_finite_input_becomes_silence_without_poisoning_history`.

Count check: the pre-A21 run = (16 eq − 1) + (17 limiter − 3) = **29 passed /
4 failed**, exactly A21's reported count — the fourth red of that run was the
eq test, not the limiter test A21's card §1.4 names (that section quotes no
failing assertion, unlike §1.1–1.3). The last run = 15 eq + 17 limiter =
**32 passed / 1 failed**, exactly the reported count. A21's `a6_tone` fix
cured the three limiter reds; the eq test was never in its scope, so it is the
one red left.

### 1.1 The failing test: `non_finite_input_becomes_silence_without_poisoning_history`

* **Location:** `crates/rustamp-dsp/src/eq.rs:847-888` (the failing assertion
  was the `assert_eq!(bits(&a), bits(&b), "history after poison == history
  after zeros")` at eq.rs:874-878 before this card; the wrong baseline was the
  `vec![0.0; 2048]` at eq.rs:870, replaced at eq.rs:876-880).
* **Verdict: expectation wrong; the code is right.** The test feeds a hostile
  block — deterministic noise with four non-finite samples at 0/500/1000/1500
  — to an enabled, fully boosted stage, then asserts that the *next* block (a
  440 Hz probe) comes out bit-identical to a fresh stage that saw **2048
  zeros**. The used stage's biquad history after the hostile block carries the
  filtered *noise* (non-zero; the simulation measures the first probe sample at
  −0.417 against the baseline's 0.0); the zeros stage's history is zero. The
  two histories differ by construction, so the assertion fails on every one of
  the 2048 probe samples. The test conflates "the poison leaves no trace" with
  "the whole block leaves no trace"; the noise is signal and legitimately
  leaves filter history.
* **What the code does right:** `Equalizer::process` maps every non-finite
  sample to exactly `0.0` before the filters (`sanitize_sample`, eq.rs:469),
  which is the documented policy (module doc, "Signal hygiene") and the
  mechanism the test's own doc comment names. A stage that saw the hostile
  block is therefore *exactly* a stage that saw the same block with zeros in
  the non-finite positions — bit for bit, history included.

### 1.2 The fix (at the cause; no claim weakened)

The baseline now keeps the signal and replaces only the non-finite samples
with `0.0` — the same mapping the sanitizer applies — so the assertion pins
precisely the documented contract: the poison is indistinguishable from a zero
sample, amid signal, over a signal-bearing probe.

* `eq.rs:876-880`: the fresh stage is warmed with the sanitized hostile block
  (was: `vec![0.0; 2048]`).
* The doc comment (eq.rs:841-845) and the assertion message are reworded to
  state the claim that is actually pinned. The old wording ("comes out as
  silence … a stage that saw zeros instead") described an all-poison block:
  the poison samples' *output* is the history's zero-input response, not
  silence, and the block's noise is not zeros.
* Nothing else changed: the hostile block, the stage state, the probe, the
  finiteness assertion ("nothing non-finite leaves") and the bit-exact
  comparison are untouched. The fixed test still detects a broken sanitizer:
  without it, NaN lodges in `z1`/`z2` and the probe output diverges from the
  baseline from the first sample.

The simulation confirms: current form — `bits(a) == bits(b)` false (all 2048
probe samples differ); fixed form — true, with the used stage's history
non-zero after the hostile block (the equality is a real check, not a trivial
one).

### 1.3 Why `a_hot_signal_is_held_at_the_ceiling` is NOT the red (A21 §1.4)

A21's card lists it as the fourth red of the pre-A21 run and rewrote it (a
settled true-peak assertion plus a 0.997 sample-peak lower bound). The
simulation shows it **passes both before and after** A21: the test's 1 kHz
sine at 48 kHz hits its peak exactly on a sample (48 samples per period, peak
at n = 12), so the sample peak equals the true peak (3.0), the detector's
phase-0 row reproduces the sample exactly, the linked peak is exactly 3.0, and
the output sample peak is the ceiling exactly (ratio 1.000000000; the settled
true peak 0.96615 is inside (0.94, 0.99)). The old 0.99 bound and the new 0.997
bound both hold. The rewrite is therefore unnecessary but harmless, and it is
left standing. (Its comment's "≈ 0.9978 · ceiling" figure is wrong — the ratio
is 1.0000 for this sine — but the assertion itself is true; noted here, not
changed, to keep this card's diff to the cause.) The conditional instruction
"if it still asserts a sample-peak premise, rewrite it to pin the true-peak
contract" does not apply: the failing test asserts no sample-peak premise, and
the limiter test that does is green.

## 2. What only the VM can prove

* That the fixed `non_finite_input_becomes_silence_without_poisoning_history`
  passes in the guest, and that the `rustamp-dsp` suite is 33/33.
* That no other test regressed (this card changes no limiter code; expected
  17/17 limiter, 16/16 eq).
* The exact failing assertion line of the orchestrator's run if anything is
  still red — this card's verdicts are static (mechanism + arithmetic); the
  simulation is a model, not the suite.

## 3. Files and line counts

| File | Change |
|---|---|
| `crates/rustamp-dsp/src/eq.rs` | `non_finite_input_becomes_silence_without_poisoning_history` (eq.rs:841-888): doc comment reworded, the all-zero baseline replaced by the sanitized hostile block, assertion message updated. No production code touched. |
| `docs/tasks/A23-dsp-rest.md` | This card (new). |

## 4. Verification and hygiene

Executed strictly within the named worktree:

* `cargo fmt --all -- --check`: **PASS** (exit 0).
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **PASS** (exit 0).
* `cargo build --workspace --offline`: **PASS** (exit 0).
* `cargo test`: **NOT RUN** (Hyper-V test-VM rule; only the orchestrator runs it).
* No window opened, no audio device opened, no audio played, no download, no
  dependency addition, no commit or other git write.
* `git status --short` shows only the owned paths above.
