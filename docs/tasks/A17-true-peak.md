# A17 — True-peak criterion in the limiter: catch the rebuild, drop the flat-topping

**Date** 2026-10-17. **Lane** `rustamp-dsp` (the look-ahead limiter stage).
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave62-truepeak`.
Write only your own files inside this worktree, and nothing under `D:\code\Rustamp`.

**Status:** implemented; host compilation/linting only, VM test execution pending.

Nothing was executed or tested on the host (Hyper-V test-VM rule). Only
`cargo fmt --all`, `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace` were used. No window opened, no audio device opened,
no audio played, no download, no dependency addition, no commit or other git
write.

## Scope and sources

The named follow-up of A14 (`docs/tasks/A14-dsp-correctness.md` §2): the
converter's final `±1.0` ceiling is a safety net, not dynamics — on hot
resampled material it flat-tops the signal, and "a true-peak limiter
(oversampled detection, smooth gain) would avoid the flat-topping and is
named as the future work that could replace the net". This card implements
that true-peak criterion in the limiter stage.

Owned changes: `crates/rustamp-dsp/src/limiter.rs`,
`crates/rustamp-dsp/src/lib.rs` (one doc line), this card. **No other file
was touched.** In particular `rustamp-audio` (the converter and its tests),
`rustamp-core`, `rustamp-output` and every other crate are unchanged; §6
names the two `rustamp-audio` tests the change deliberately breaks and what
their lane must do about them.

Sources: the A14 card (the measurements and the decision), the A6 review
(finding 3), A11 (the limiter default and the latency accounting), spec S07
(the chain position), the code of the two crates (the limiter, the equaliser,
`convert.rs` read-only), and ITU-R BS.1770-4 §3.3 for the 4× oversampling
factor of true-peak *measurement* (a published standard, cited for the
factor only). No Winamp source, SDK material, disassembly, research tree or
reference binary was opened. No third-party code was copied and no dependency
was added — the detector is 64 precomputed coefficients and a 32-float
history per channel, hand-rolled for the reasons recorded in the module doc
("Why not a crate"); the task also forbids `cargo add`.

## 1. The measured problem (A14, restated with its numbers)

The chain is `decoder -> Equalizer -> [stages] -> Limiter -> FormatConverter
-> ring -> volume/balance -> device`. The limiter guarantees a **sample**-peak
ceiling (`10^(-0.3/20) = 0.96605`) at its own output. The band-limited
resampler after it reconstructs the waveform between the samples and rebuilds
peaks above that ceiling — and above full scale.

The A14 anchor tone is `x[n] = S·sin(πn/2 + π/4)` at 44.1 kHz: samples of
exactly `±S·√2/2`, a true peak of `S`. Its peaks fall exactly halfway between
samples (`n = 0.5 + 2k`), so at 48 kHz (11.025 kHz, inside the passband, flat
to ±0.05 dB) the converter samples it near its true peak:

| S | sample peak (pre) | old limiter action | post (unclamped) | post (enforced) |
|---|---|---|---|---|
| 1.0 | 0.70711 (passes untouched) | none | ≈ 0.994–1.0 (+40 % rebuild) | ≤ 1.0 |
| 1.3662 | 0.96605 = the ceiling (to 1.5e-6) | none (bit for bit) | ≈ 1.358 (**+2.7 dBFS**) | **exactly 1.0** |

So the old criterion is blind to a rebuild of `√2` (+3 dB) on this signal,
and on the torture case the converter's safety net clamps the rebuilt tone
flat at exactly `1.0`: the rebuilt sine (`A ≈ 1.358`) exceeds full scale for
`|sin| > 1/1.358 = 0.7363`, i.e. 170.3° of every 360° period — **≈ 47 % of
the 48 000 output samples, ≈ 22 700 samples, sit at exactly ±1.0**
(analytic estimate; the VM's chain test measures the exact count). The clamp
is the safety net doing its job; the flat-topping is what it costs.

## 2. The criterion and its cost

**The criterion (own engineering choice).** Per frame, the sidechain now asks
for the peak of the *reconstructed* signal, not the peak of the samples:

```
linked peak p = max(sample peak of the frame, true-peak estimate of the frame)
required gain r = min(1, ceiling / p)          (unchanged formula, larger p)
```

The true-peak estimate is the standard honest way: **oversampling the
sidechain**. For every frame, a polyphase interpolator evaluates the
band-limited reconstruction at `TRUE_PEAK_OVERSAMPLE` sub-positions of the
newest interval whose kernel support is fully inside the detector's history,
and the linked (largest over channels) maximum of those values is the
estimate.

* **How much oversampling, and why: 4×** — the factor ITU-R BS.1770-4
  specifies for true-peak measurement. A 4× grid under-reads a tone's true peak
  by at most `1 − cos(π f / (4 fs))`: **0.17 dB at a quarter of the sample
  rate** (the A14 anchor), growing to 0.67 dB at Nyquist, where the
  resampler's own transition band attenuates by more. 8× would halve the
  under-read at twice the cost; the −0.3 dB ceiling margin absorbs the in-band
  part of the 4× residual. Both factors are named constants
  (`TRUE_PEAK_OVERSAMPLE`, `TRUE_PEAK_HALF_TAPS`).
* **The kernel:** a symmetric Blackman-Harris-windowed sinc of half-length
  `TRUE_PEAK_HALF_TAPS = 8` (16 taps per phase) — the same window family the
  converter's resampler uses — **DC-normalized** so every phase sums to one
  (DC passes at unity) and phase 0 reproduces the sample itself. Zero-phase,
  so the only delay is the detector's own.
* **The detector's delay: `half` = 8 frames** (clamped to the look-ahead). A
  zero-phase estimate of the interval `[n, n+1)` needs samples up to
  `n + half`, so the estimate pushed with frame `f` describes the interval
  `[f − half, f − half + 1)` — always inside the 5 ms look-ahead. The delay
  costs **no latency**: the detector is sidechain only; the signal path is the
  same delay line as before and `latency_frames()` is unchanged.
* **The extra cost: sidechain CPU only.** Per frame the detector adds
  `TRUE_PEAK_OVERSAMPLE · 2 · TRUE_PEAK_HALF_TAPS` = **64 f64
  multiply-adds per channel** (plus the same number of abs/max and a
  16-float history push per channel) — ≈ 6.1M f64 FMAs/s at 48 kHz stereo. The
  signal path (delay, gain multiply, guard) is untouched, so the stage's
  per-frame work grows about 40× from a tiny base: a small fraction of one
  core. **No benchmark was run** (the host runs no tests): the cost is stated
  as an op count, not a wall clock. State grows by `2·half` f32 per channel
  plus the 64-double table; `prepare` allocates, `process` never allocates
  and never locks.

**Guarantees kept (the task's requirement 2):**

* **Ceiling semantics:** no output sample exceeds `10^(−0.3/20)` — **exact,
  unchanged**. The no-overshoot proof only uses
  `required ≤ min(1, ceiling / sample peak)`, which the larger linked peak
  preserves (the `f32`↔`f64` comparison change is order-preserving and
  semantically identical).
* **Look-ahead and latency:** unchanged — same `L = round(5 ms · rate)`
  (221 frames at 44.1 kHz, 240 at 48 kHz), same delay line, same
  `latency_frames()`, same attack ramp and release.
* **Latch behaviour:** unchanged — `LimiterControl`, latched at the next
  `prepare`/`reset`.
* **Disabled bit-exact bypass:** unchanged — `process` returns before
  touching the buffer (and before touching the detector).
* **Transparency when enabled** is re-scoped, honestly: it now holds "while
  every frame's *linked* peak — sample peak and true-peak estimate — is at or
  below the ceiling". That is a **behaviour change on hot full-band material
  at equal rates** (the output side's own reconstruction overshoot is now
  limited too — correct for a true-peak limiter, and the reason true-peak
  limiters exist), and it is why three existing tests were re-scoped (§4).

## 3. The numbers, before and after

On the exact A14 signal, through the new criterion (limiter at 44.1 kHz, mono;
the settled region of a 1 s tone; the estimate of this tone is ≈ `S·(1 ± 0.002)`
because its peaks fall exactly on sub-positions, so the generic 0.17 dB
worst case does not apply):

| Quantity | S = 1.0, before | S = 1.0, after | S = 1.3662, before | S = 1.3662, after |
|---|---|---|---|---|
| limiter sees (linked peak) | 0.70711 | ≈ 1.000 | 0.96605 | ≈ 1.3662 |
| limiter action | none (bit-exact pass) | gain ≈ 0.966 (−0.29 dB) | none (bit-exact pass) | gain ≈ 0.7072 (**−3.01 dB = the rebuild**) |
| output sample peak | 0.70711 | ≈ 0.683 | 0.96605 | ≈ 0.683 |
| output true peak | 1.000 | ≈ 0.966 (= the ceiling) | 1.3662 | ≈ 0.966 (= the ceiling) |
| converter output peak | ≈ 0.994–1.0 | ≈ 0.966–0.968 | ≈ 1.358, **clamped flat at 1.0** | ≈ 0.966–0.968 |
| samples at exactly ±1.0 | ≈ 0 | **0** | ≈ 47 % (≈ 22 700) | **0** |

So the criterion catches what the sample peak missed — a full `√2` (+3 dB) of
peak on the anchor, +2.7 dBFS on the torture case — and the flat-topping the
safety net caused is gone on both anchors: the limiter now does the limiting
(smooth gain, no clipping), and the converter's net has nothing to clamp. The
residual: the output true peak sits at the ceiling ± the estimator's error
(≤ +0.02 dB here), i.e. 0.966–0.968 — below full scale with the −0.3 dB margin
to spare. The limiter-level numbers are pinned by the new tests; the
converter-level numbers (post ≈ 0.967, clamp count 0) are measured by the
`rustamp-audio` chain test in the VM (§6.1).

## 4. Tests

`crates/rustamp-dsp/src/limiter.rs` now carries **17 tests** (was 11): 6 new,
3 re-scoped, 8 unchanged. None was run on this host. Every numeric bound was
chosen analytically with margin over the hand-derived value (the A14
convention), because the estimator's own error is part of what is being
bounded; the VM must confirm them.

New:

* `the_true_peak_criterion_catches_the_band_edge_tone` — the S = 1.0 anchor:
  pins the input's sample peak (0.70711, exactly `√2/2`), its reference true
  peak (1.0 ± 0.03, ratio > 1.3 = the +3 dB), that the sample peak is below
  the ceiling while the true peak is not; then through the limiter: no output
  sample exceeds the ceiling (exact), engagement (`out_peak < sample_peak`),
  settled sample peak ∈ [0.66, 0.70] (≈ 0.683), settled gain ∈ [0.94, 0.99]
  (≈ 0.966), settled true peak ∈ [0.94, 0.99] (≈ 0.966, below full scale), and
  `latency_frames()` unchanged.
* `the_limiter_pulls_the_torture_tone_down_instead_of_passing_it` — the
  S = 1.3662 anchor: pins the samples at the ceiling (± 1e-3, the A14 bound),
  the reference true peak (1.3662 ± 0.03), engagement, settled sample peak
  ∈ [0.66, 0.70] (≈ 0.683), settled gain ∈ [0.69, 0.73] (≈ 0.7072, the
  −3.01 dB rebuild), settled true peak ∈ [0.94, 0.99].
* `the_detector_kernel_is_dc_normalized_and_sample_exact` — every phase sums
  to 1 ± 1e-12 (DC unity), phase 0 is the sample itself (centre tap 1, others
  < 1e-12), and the symmetric kernel mirrors between phases `j` and `K − j`.
* `the_true_peak_criterion_is_linked_over_channels` — tone on the left,
  silence on the right: limited by the left's inter-sample peak, right channel
  exactly zero.
* `a_nan_does_not_poison_the_true_peak_detector` — a NaN silences its frame
  and the sanitized history keeps the estimate finite; the tone around it is
  still limited (settled peak ∈ [0.66, 0.70]).
* `a_low_rate_prepare_still_holds_the_ceiling` — at 1 kHz the look-ahead is 5
  frames, the kernel half-length clamps to it, and the sample-peak guarantee
  holds regardless of the kernel.

Re-scoped (the premise was a sample-peak premise):

* `a_quiet_signal_passes_bit_exact_when_enabled` — "quiet" now means *linked*
  peak below the ceiling. The old input (±0.5 white noise) is no longer
  provably quiet: the kernel's L1 bound on its estimate is
  `0.5·(S⁺ + |S⁻|) ≈ 0.91`, within 6 % of the ceiling, and a true-peak
  limiter may legitimately engage there. The test now uses a band-limited sine
  (true peak = amplitude 0.5) plus ±0.125 noise (L1-bound ≈ 0.46, 2× margin).
* `transparency_returns_after_a_burst` — tail scaled to ±0.125 (L1-bound
  ≈ 0.23), burst ×64 (±8) so a reduction is still forced; the released
  envelope still snaps to exactly 1.0 and the tail is bit-exact.
* `a_hot_signal_is_held_at_the_ceiling` — the right channel's noise is scaled
  ×4 → ×0.5 so its true-peak estimate (L1-bound ≈ 0.46) stays below the sine's
  3.0 and the linked criterion is set by the sine; the lower bound is relaxed
  0.999 → 0.99 because the output *sample* peak is now
  `ceiling · (sample peak / linked peak)` ≈ 0.9978·ceiling for a 1 kHz sine
  (the criterion's own target — the estimated true peak held at the ceiling —
  is pinned by the band-edge tests).

Unchanged (8): `disabled_is_a_bit_exact_bypass`,
`the_default_stage_is_enabled`,
`look_ahead_shapes_the_transient_instead_of_clipping_it`,
`non_finite_samples_are_silenced`, `reset_clears_the_history`,
`switching_is_latched_until_reset`, `dc_is_delayed_exact_and_hot_dc_is_held`
(DC is unity through the DC-normalized kernel, so the hot-DC bound still
holds), `subnormal_signal_passes_through_unchanged`.

The VM (orchestrator) must confirm: the suite stays green except the two
named `rustamp-audio` tests (§6); the measured numbers of §3 (settled sample
peak ≈ 0.683, settled true peak ≈ 0.966–0.968, input reference true peaks
1.0 / 1.3662, kernel sums); and the two engine tests' exact lengths.

## 5. What a true-peak estimate cannot promise

* **The sample-peak guarantee is exact; the true-peak bound is an estimate.**
  The 4× grid under-reads by up to 0.17 dB at fs/4 (0.67 dB at Nyquist)
  before the kernel's own error (≈ 1e-4 in band). The −0.3 dB ceiling margin
  absorbs it below ~0.4·fs; near Nyquist the resampler's transition band
  attenuates by more than the under-read, and whatever remains is caught by
  the converter's `±1.0` net — which is **not removed** (it is not mine to
  remove, and it is the last resort for exactly this residual).
* **The detector's `half`-frame delay dilutes the attack average** by up to
  `half / (L + 1)` of the gain deficit on step transients whose preceding
  `half` frames are quiet (2.9 % at 48 kHz, 20 % at 8 kHz). Sustained tones —
  the measured problem — have **zero** dilution: every term of the average
  carries the bound. The sample-peak guarantee is unaffected either way.
* **Downsampling over-reads.** The estimator models the full-band
  reconstruction at the stage's rate; when the device rate is lower, content
  the resampler will remove is counted, so the limiter may limit a little more
  than strictly needed. Conservative, never the other way.
* **Equal rates change the sound on hot full-band material** (the output
  side's own reconstruction overshoot is now limited). Quiet material is
  unaffected: its true peak is its sample peak, or below the ceiling.
* **Discontinuities.** Within `2·half` frames of a stream start (or any
  discontinuity) the kernel's truncation error is a few percent, so the held
  true peak there is the ceiling ± a few percent; where the missing history
  inflates the estimate the stage limits a little more, never less. The net
  still bounds the absolute output. (The anchor tests measure the settled
  region; the start transient is the one place the new tests deliberately do
  not assert a true-peak bound.)
* **No benchmark.** The cost is an op count (64 f64 FMAs per channel-frame),
  not a wall clock; the VM can benchmark if the owner wants a number.
* **Hardware only.** Whether the flat-topping was audible on real material,
  what a real WASAPI mixer does with near-scale floats, and the residual
  near-Nyquist behaviour need a loopback capture or a listening test (a
  future lane). The 0 dBFS guarantee still ends where the floats leave the
  ring.

## 6. What the engine side must do (named here, not done — another lane owns `rustamp-audio`)

**Nothing is required.** The latency contract is unchanged
(`latency_frames()` = `L`), so `decode.rs`'s trim/flush accounting
(`dsp_latency()` sums the stages' `latency_frames()`) needs no change, the
converter's `±1.0` net stays, and the chain order is untouched. Two
`rustamp-audio` tests are affected and their lane must update them:

1. **`crates/rustamp-audio/tests/dsp_chain.rs::limiter_cannot_stop_the_rebuild_but_the_converter_holds_0dbfs`
   — will fail by design** (two assertions: `(pre − ceiling).abs() < 1e-3`
   and `(post − 1.0).abs() < 1e-6`). The limiter now stops the rebuild, so
   the test's premise is gone. Rewrite it to pin the new behaviour — the A14
   card's own prose already names the intended assertions ("`post <= ceiling`
   with margin, plus a flat-top count of zero"): expected `pre ≈ 0.683` (the
   limited tone's sample peak), `post ≈ 0.966–0.968` (the resampled true
   peak; the resampled samples reach within a fraction of a dB of it, so
   `ceiling·0.99 ≤ post ≤ 1.0` is the safe form), and `clamped == 0` (before:
   `post` exactly 1.0, ≈ 47 % of samples clamped). Rename the test and fix
   its doc comment — the limiter *can* now stop the rebuild.
2. **`crates/rustamp-audio/tests/engine.rs::limiter_limits_hot_signal_to_ceiling_and_disabling_returns_to_bit_exact`
   — the `peak_enabled >= ceiling * 0.99` bound (~line 1499) is at risk.**
   It is a sample-peak assertion: on the white 2×-boosted pattern (samples in
   0.2..2.2) the linked true-peak estimate (kernel-L1 bound ≈ 3.0) exceeds
   the sample peak (2.2), so the output *sample* peak can legitimately sit
   below 0.99·ceiling (analytic range ≈ [0.71, 1.00]·ceiling; the exact value
   is deterministic but was not computed on this host — no test was run).
   The audio lane must relax the lower bound (e.g. `0.7·ceiling`) or assert on
   the true-peak estimate instead. Every other assertion in that test holds:
   `peak ≤ ceiling` (exact), disabled → bit-exact, disabled peak > ceiling.

Expected to hold, VM to confirm (all upper-bound, deterministic or
limiter-off assertions): `default_engine_limiter_is_enabled_and_limits_overs`
(≤ ceiling, exact); `limiter_toggle_latches_on_seek`; `a_restart_with_the_default_limiter_matches_a_fresh_run`;
`a_hot_resampled_run_with_the_limiter_holds_0dbfs` (≤ 1.0);
`eq_boost_then_limiter_then_resampler_holds_0dbfs` (`pre ≤ ceiling` — now
≈ 0.9978·ceiling because the +12 dB sine's linked peak is its true peak;
`post ≤ 1.0`); `disabled_limiter_still_holds_0dbfs` (limiter off —
unchanged). The two gapless bit-exact tests
(`the_enabled_limiter_keeps_a_gapless_join_exact_and_undelayed`,
`the_default_limiter_loses_no_tail_and_adds_no_seam_silence`) hold by the
kernel's L1 bound: the 0.05..0.55 pattern's estimate is
`0.55·S⁺ − 0.05·|S⁻| = 0.55 + 0.5·|S⁻| ≈ 0.755` (the DC-normalized positive-tap
sum is `S⁺ = 1 + |S⁻|`, with `|S⁻| ≈ 0.41` hand-computed for the 16-tap
Blackman-Harris kernel, ≤ 0.6 with margin) — below the ceiling 0.9661 with
≥ 12 % margin, so the limiter stays linear and the bit-exact assertions
stand.

Optional, not required, not done: delete the converter's net once the
criterion is trusted in the VM (not recommended now — it is the last resort
for the Nyquist residual and the discontinuity error); benchmark the
detector; expose an 8× oversampling constant.

## 7. Verification and hygiene

* `cargo fmt --all -- --check`: **PASS** (after `cargo fmt --all` fixed five
  formatting nits in the new code).
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **PASS** (exit 0, no warnings; one type error fixed on the way: an
  `f32`/`f64` comparison in a new test).
* `cargo build --workspace`: **PASS** (exit 0).
* `cargo test`: **NOT RUN** (Hyper-V test-VM rule respected; no test executed
  on this host).
* No window opened, no audio device opened, no audio played, no download, no
  dependency addition, no commit or other git write.
* Every command ran in the named worktree. Final `git status --short`:

```
 M crates/rustamp-dsp/src/lib.rs
 M crates/rustamp-dsp/src/limiter.rs
?? docs/tasks/A17-true-peak.md
```

i.e. only owned paths (`crates/rustamp-dsp/**` and this card).

## Files and line counts

* `crates/rustamp-dsp/src/limiter.rs`: 680 → **1128 lines** (+448: the true-peak
  criterion, the detector, the kernel builder, module-doc sections; 17 tests,
  6 new, 3 re-scoped, 8 unchanged).
* `crates/rustamp-dsp/src/lib.rs`: 33 → **30 lines** (crate doc: the limiter is
  now described as the look-ahead *true-peak* limiter).
* `docs/tasks/A17-true-peak.md`: this card (new).
* `git diff --stat`: 2 files changed, 569 insertions(+), 45 deletions(-), plus
  the new card.
