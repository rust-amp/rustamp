# A21 — Green the DSP crate's red tests

**Date** 2026-10-18. **Lane** `rustamp-dsp`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave68-dsp-red2`.
Write only owned paths inside this worktree; nothing under `D:\code\Rustamp`,
`D:\code\rustamp-research` or `D:\code\rustamp-refbin` was read or written.

**Status:** implemented; host compilation/linting only, VM test execution pending.

Nothing was executed or tested on the host (Hyper-V test-VM rule). Only
`cargo fmt --all`, `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace --offline` were used. No window opened, no audio device
opened, no audio played, no download, no dependency addition, no commit or other
git write.

## 0. Boundary

Owned paths: `crates/rustamp-dsp/src/limiter.rs` and this card
`docs/tasks/A21-dsp-red2.md`. No file under `crates/rustamp-audio`,
`crates/rustamp-mediahost`, `crates/rustamp-dsphost`, `crates/rustamp-meta`,
`crates/rustamp-decoders` or any other crate was touched.

## 1. What the VM reported, and root-cause analysis

The VM reported the `rustamp-dsp` suite with 29 passed and 4 failed (out of 33 tests
total: 16 in `eq.rs`, 17 in `limiter.rs`).
No test was run on this host; all root causes were derived analytically from first
principles, the true-peak filter mathematics (ITU-R BS.1770-4 4× oversampled
polyphase sinc interpolator, Blackman-Harris windowed sinc, DC-normalized), and
source code analysis.

### 1.1 Test 1: `the_limiter_pulls_the_torture_tone_down_instead_of_passing_it`

* **Location:** `crates/rustamp-dsp/src/limiter.rs:1060-1110`
* **Verdict:** Code wrong (`a6_tone` helper bug at line 682).
* **Root cause:**
  In `a6_tone(s: f64, frames: usize)` at `crates/rustamp-dsp/src/limiter.rs:682`:
  ```rust
  (0..frames)
      .map(|n| (s * (std::f64::consts::FRAC_PI_4 + PI * n as f64 / 2.0)).sin() as f32)
      .collect()
  ```
  The parentheses were misplaced: the scale factor `s` multiplied the angle inside
  `.sin()`, rather than scaling the amplitude of the resulting sine.
  For `s = 1.0` (in `the_true_peak_criterion_catches_the_band_edge_tone`), `1.0 * phase`
  coincidentally produced the identical angle, so that test passed.
  However, for `s = 1.3662`, `s * angle` yielded an angle progression corresponding
  to ~15.06 kHz with amplitude 1.0, rather than an 11.025 kHz band-edge tone of
  amplitude 1.3662 (true peak 1.3662, sample peak = ceiling = 0.96605).
  Consequently, `sample_peak` evaluated to 1.0 instead of ceiling (0.96605), failing
  the assertion `assert!((sample_peak - ceiling).abs() <= 1e-3)` at line 1071.
* **Fix:** Correct the parentheses in `a6_tone` at `limiter.rs:682` to:
  ```rust
  (s * (std::f64::consts::FRAC_PI_4 + PI * n as f64 / 2.0).sin()) as f32
  ```

### 1.2 Test 2: `the_true_peak_criterion_is_linked_over_channels`

* **Location:** `crates/rustamp-dsp/src/limiter.rs:1154-1177`
* **Verdict:** Code wrong (`a6_tone` helper bug at line 682).
* **Root cause:**
  This test calls `a6_tone(1.3662, rate as usize)` at line 1159. Due to the operator
  precedence bug described in §1.1, the left channel received a 1.0-amplitude tone at
  ~15.06 kHz rather than the torture tone. This prevented the limiter from applying
  the required -3.01 dB gain reduction, causing the settled output true peak to fail
  `assert!((0.94..0.99).contains(&settled_tp))` at line 1173.
* **Fix:** Fixed by the correction to `a6_tone` at line 682.

### 1.3 Test 3: `a_nan_does_not_poison_the_true_peak_detector`

* **Location:** `crates/rustamp-dsp/src/limiter.rs:1183-1203`
* **Verdict:** Code wrong (`a6_tone` helper bug at line 682).
* **Root cause:**
  This test calls `a6_tone(1.3662, rate as usize)` at line 1188. With amplitude 1.0
  instead of 1.3662, the limiter applied gain of ~0.966 rather than ~0.707, leaving
  the output sample peak near ~0.966 instead of ~0.683. This failed the assertion
  `assert!((0.66..0.70).contains(&settled_peak))` at line 1199.
* **Fix:** Fixed by the correction to `a6_tone` at line 682.

### 1.4 Test 4: `a_hot_signal_is_held_at_the_ceiling`

* **Location:** `crates/rustamp-dsp/src/limiter.rs:784-822`
* **Verdict:** Expectation wrong (test still written on a sample-peak premise).
* **Root cause:**
  Under A14 (sample-peak limiter), this test asserted `assert!(peak >= ceiling * 0.999)`.
  Under A17 (true-peak limiter), the criterion targets the continuous reconstructed
  true peak at the ceiling, not the discrete sample peak. In A17, the lower bound
  assertion was relaxed from 0.999 to 0.99 (`assert!(peak >= ceiling * 0.99)`) rather
  than pinning the true-peak contract.
  Relaxing assertions weakens claims; instead, the test must pin the true-peak
  contract (`reference_true_peak` of settled output sits at `ceiling`) and derive the
  tight lower bound on the sample peak directly from the criterion:
  For a 1 kHz sine at 48 kHz, the sample peak is `ceiling · (sample_peak / linked_peak)`
  ≈ `0.9978 · ceiling`.
* **Fix:**
  1. Pin the new true-peak contract: measure `settled_tp = reference_true_peak(settled, 16)`
     on the settled channel and assert `assert!((0.94..0.99).contains(&settled_tp))`.
  2. Derive the tightened lower bound directly from the true-peak criterion:
     `assert!(peak >= ceiling * 0.997, "not brought to the ceiling: {peak}")`,
     preserving the claim without weakening it.

## 2. Verification and hygiene

* `cargo fmt --all -- --check`: **PASS** (exit 0).
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **PASS** (exit 0, zero warnings).
* `cargo build --workspace --offline`: **PASS** (exit 0).
* `cargo test`: **NOT RUN** (Hyper-V test-VM rule strictly respected; no test run on host).
* No git commit was made.
* Worktree boundary strictly respected:
  `git status --short`:
  ```
   M crates/rustamp-dsp/src/limiter.rs
  ?? docs/tasks/A21-dsp-red2.md
  ```
  Only owned paths modified.
