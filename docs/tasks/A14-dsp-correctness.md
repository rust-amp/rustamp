# A14 — DSP correctness: the equaliser cannot stop the resampler rebuilding peaks

**Date** 2026-10-17. **Lane** `rustamp-dsp` + `rustamp-audio`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave59-dsp-correctness`.
Write only your own files inside this worktree, and nothing under `D:\code\Rustamp`.

**Status:** implemented; host compilation/linting only, VM test execution pending.

Nothing was executed or tested on the host (Hyper-V test-VM rule). Only
`cargo fmt --all`, `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace` were used. No window opened, no audio device
opened, no audio played, no download, no dependency addition, no commit or
other git write.

## Scope and sources

The open defect from the DSP wave (A6 finding 3, carried by A7 and A11 as
"not fixed, documented"): the look-ahead limiter guarantees a sample-peak
ceiling at its own output, but the band-limited resampler after it
reconstructs the waveform between samples and rebuilds peaks above the
ceiling — and above full scale. Three claims of that adversarial review
hold; this card settles the one defect it left named and open.

Owned changes: `crates/rustamp-dsp/src/**`, `crates/rustamp-audio/src/**`
(docs only, plus the converter's final ceiling),
`crates/rustamp-audio/tests/dsp_chain.rs` (new),
`crates/rustamp-audio/tests/engine.rs` (two tests),
this card. No other file was touched or modified. In particular
`rustamp-core` (frozen contracts, RFC 0014 is still a proposal and is not
implemented here), `rustamp-output`, `rustamp-app` and every other crate are
unchanged.

Sources: the A6 review (`docs/tasks/A6-dsp-adversarial-review.md`), the A7
repair card, A11/A12/A13, H5, spec S07 (the measured anchors and the
equaliser's purpose), RFC 0011 (the dB↔classic conversion), the code in the
two owned crates, and the rubato 5.0.1 behaviour already documented in
`convert.rs`. No reference behaviour is inferred. No Winamp source, SDK
material, disassembly, research tree or reference binary was opened. No
third-party code was copied and no dependency was added.

## 1. The facts, numerically (analytic estimates + the tests that pin them)

The chain is `decoder -> Equalizer -> [installed stages] -> Limiter ->
FormatConverter -> ring -> volume/balance (render callback) -> device`.
Volume and balance only attenuate (see `gain.rs`, pinned by its tests), so
the converter output is the last point whose level the chain can raise.

The A6 tone `x[n] = S·sin(πn/2 + π/4)` at 44.1 kHz has samples of exactly
`±S·√2/2`. At 48 kHz the fs/4 tone (11.025 kHz, well inside the passband,
flat to ±0.05 dB per `passband_is_flat_from_44100_to_48000`) is sampled near
its true peak `S`:

| S | sample peak (pre) | limiter action | post (unclamped, estimated) | post (enforced) |
|---|---|---|---|---|
| 1.0 | 0.7071 (passes any sample-peak limiter untouched) | none | ≈ 0.994–1.0 | ≤ 1.0 |
| 1.3662 | 0.96605 = the −0.3 dBFS ceiling to 1.5e-6 (limiter passes bit for bit) | none | ≈ 1.358 (**+2.7 dBFS**) | **exactly 1.0** |

So with `S = 1.0` the rebuild is `post/pre ≈ 1.406`, over **+40 %** above
the sample peak, invisible to every sample-peak criterion before the
resampler; with `S = 1.3662` it is about **+3 dB** over the sample peak
(+2.7 dBFS absolute). These are analytic estimates from the filter's
documented passband; the exact decimals are measured by the new tests in
the VM (see §5).

The equaliser side, over the measured anchors (S09: `0 = +12 dB`,
`31 = 0 dB`, `63 = −12 dB`; preamp an exact `10^(dB/20)` factor):

* every one of the ten bands hits its centre gain at both anchors within
  ±0.05 dB (was: band 4 only);
* the intermediate law is linear in dB (own choice): ±6 dB land halfway;
* DC passes at unity through fully boosted bands (peaking sections are
  unity at DC by construction), times the exact preamp factor;
* a +12 dB boost on a 0.5 sine reaches ≈ 1.99 pre-converter with the
  limiter off — the chain must survive that too.

## 2. What the chain does about it (decision, and why)

**Decision (own product choice, not measured behaviour):**

1. The limiter stays where it is — last DSP stage, sample-peak, −0.3 dBFS
   ceiling. It must see EQ/DSP overs at the source rate, where its
   look-ahead latency compensation is built and tested (A7). Moving it
   after the converter would multiply its cost by up to 4× at high device
   rates and reopen the seam/latency accounting; a true-peak limiter would
   be a new, unmeasured DSP design that cannot be validated here.
2. No fixed headroom is carved before the resampler. Headroom wastes
   loudness on the equal-rate path (where the rebuild is exactly zero) and
   no single number follows from the anchors — the rebuild is signal-,
   not level-dependent.
3. The converter holds a **final `±1.0` (0 dBFS) ceiling on the resampled
   path** (`finish_sample` in `convert.rs::run_block`, covering both
   `convert` and `flush`): non-finite → silence, subnormal → zero, anything
   above full scale → clamped to `±1.0`. Because nothing downstream can
   raise the level, a bound here is a bound at the device: the ring never
   holds a sample above full scale however the equaliser and the limiter
   are set. The equal-rate path is untouched, so its bit-exact promise
   stands.

   > **Superseded (2026-10-20).** Finding **F2** of
   > `docs/tasks/W14-wave-review.md` proved that the equal-rate path's
   > missing clamp contradicts this card's own promise ("the ring never
   > holds a sample above full scale"): with the limiter off, a positive
   > ReplayGain correction or an equalizer preamp put above-full-scale
   > samples into the ring. The ceiling now lives in the converter's
   > **single output stage** (`convert.rs::emit_device_samples`) and holds
   > on **both** paths; the equal-rate path stays bit-exact for in-range
   > input. The resampled path's behaviour is unchanged. See F2 in
   > `docs/tasks/W14-wave-review.md`.
4. The bound is `1.0`, not the limiter's `0.9661`. The clip point is
   0 dBFS; the limiter's ceiling is a dynamics choice, and legitimate
   near-full-scale peaks must not be touched by the safety net.

**Why this is the right answer for a player that must not clip:** the final
authority has to sit at or after the last stage that can raise peaks, and
that stage is the resampler. A sample-peak limiter before it is blind to
reconstruction by physics, not by implementation quality. The post-converter
clamp is therefore the only place a hard guarantee can live without moving
the limiter or inventing a true-peak stage. It is a safety net, not
dynamics: on ordinary program material the limiter keeps the signal out of
it; on torture signals it flat-tops briefly instead of emitting above-scale
floats. A true-peak limiter (oversampled detection, smooth gain) would avoid
the flat-topping and is named as the future work that could replace the net;
the tests below would pin it unchanged (`post <= ceiling` with margin,
plus a flat-top count of zero).

## 3. Stage policy for hostile input (own choice, stated plainly)

| Input | Equalizer | Limiter | Converter (resampled path) | Equal-rate path |
|---|---|---|---|---|
| NaN / ±Inf (signal) | → `0.0` at stage input (**new**) | → `0.0` (exists: required-gain 0 + `guard`) | → `0.0` (**new**) | untouched (bit-exact promise) |
| Non-finite (control gains) | → flat / clamped (exists) | n/a | n/a | engine `clamp_gain` (exists) |
| Subnormal (signal) | passes through; history flushed (exists) | passes through (exact `1.0` fast path) | → `0.0` (**new**) | untouched (bit-exact promise) |
| DC | passes (peaking unity at DC; preamp applies) | passes (delayed, or held at ceiling when hot) | passes at unity (existing test) | passes |

> **Updated (2026-10-20, finding F2).** The converter's column now holds on
> **both** paths: the finishing is the converter's single output stage, so the
> equal-rate path turns a non-finite sample into silence, a subnormal into
> `0.0` and a sample above full scale into `±1.0` as well. The equal-rate
> path keeps its bit-exact promise **for in-range input**. See F2 in
> `docs/tasks/W14-wave-review.md`.

Rationale: NaN spreads (one NaN lodges in biquad history forever, and
through a band-limited filter contaminates a whole FFT block plus overlap),
so it becomes silence at every boundary. Subnormals cannot poison anything —
they are a performance-hygiene question, not a correctness one — so the
stages pass them and flush only history; the resampled path flushes them
because the bit-exact promise does not apply there and the render thread
must never see denormal load it did not ask for. DC is signal (and the
existing test pins that it passes at unity), so it is never removed.

## 4. What changed (files)

* `crates/rustamp-dsp/src/lib.rs`: shared `sanitize_sample` policy helper;
  crate doc corrected (a directly built `Limiter` defaults to enabled and
  must be switched off explicitly — the old "disabled by default" line
  described the engine default, not the stage).
* `crates/rustamp-dsp/src/eq.rs`: non-finite input samples become `0.0`
  before the filters (one line + module-doc policy); five new tests
  (all-band anchors, ±6 dB linearity, DC unity, poison-then-clean
  bit-equality with zeros-then-signal, subnormal pass-through).
* `crates/rustamp-dsp/src/limiter.rs`: `ceiling_linear()` helper
  (`10^(CEILING_DB/20)`, used by the stage itself); module docs corrected
  (the resampler *can* rebuild above the ceiling — the old "never raise the
  level above unity" sentence was the exact error A6 finding 3 proves);
  three new tests (hot/quiet DC, subnormal pass-through, no poison after
  non-finite).
* `crates/rustamp-audio/src/convert.rs`: final-ceiling paragraph in the
  module docs; `finish_sample` applied to every resampled output frame;
  four new tests (rebuild ratio, torture clamp, subnormal silence,
  non-finite silence).
* `crates/rustamp-audio/src/decode.rs`, `engine.rs`, `lib.rs`: doc-only
  (chain guarantee stated at each place that previously said "not fixed").
* `crates/rustamp-audio/tests/dsp_chain.rs` (**new**): the three chain
  tests of §1 (music-like, torture, limiter-off).
* `crates/rustamp-audio/tests/engine.rs`: two run-level tests on a 48 kHz
  device (limiter on: exact length + `peak <= 1.0`; limiter off: exact
  length + `peak <= 1.0` + the safety net demonstrably engaged).

## 5. Tests and what needs the VM or hardware

New tests: 5 (eq) + 3 (limiter) + 4 (convert) + 3 (chain) + 2 (engine) = 17.
None was run on this host. Every numeric bound was chosen analytically with
at least an order of magnitude of margin over the filter arithmetic (see
§1), except the exact-`1.0` clamp assertions, which hold by construction
(the clamp writes the literal `1.0f32` wherever the unclamped value would
exceed it, and the unclamped torture peak is ≥ 1.35 by the passband bound).

VM (orchestrator) must confirm: the suite stays green; the measured rebuild
numbers (§1 table); the torture clamp count; and the two engine tests'
exact lengths. Hardware (loopback or listening, future lane) must confirm:
whether the flat-topping on hot resampled material is audible; what a real
WASAPI mixer does with near-scale floats; and the true-peak follow-up.

## 6. What needs the engine side, what needs hardware, what a listener notices

* **Engine side:** done here — chain order unchanged, no contract change,
  no new latency (the clamp adds no delay, holds nothing back), gapless and
  seek behaviour untouched (`run_block` is inside the existing trim/flush
  accounting). Nothing for the engine lane to do.
* **Hardware:** the 0 dBFS guarantee ends where the floats leave the ring.
  Endpoint-driver behaviour above scale, mixer treatment of near-scale
  floats, and audibility need a loopback capture or a listening test — named
  above, not claimed here.
* **Listener:** on ordinary material, nothing — the limiter keeps program
  audio out of the safety net, and equal-rate playback is bit-identical to
  before (the equal-rate path is byte-untouched). On pathological hot
  content at mismatched rates, brief flat-topping at 0 dBFS replaces
  above-scale output. The limiter default (on), the −0.3 dB ceiling, the 5 ms
  look-ahead, the 0 dBFS net, the pass-through of DC/subnormals and the
  linear intermediate band law are all own choices, stated as such wherever
  they appear; none is cited as 2.95 behaviour.

## 7. Verification and hygiene

* `cargo fmt --all -- --check`: **PASS** (exit 0).
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`:
  **PASS** (exit 0, no warnings; two lints fixed on the way:
  `if_same_then_else` in `finish_sample`, `approx_constant` in a test).
* `cargo build --workspace`: **PASS** (exit 0).
* `cargo test`: **NOT RUN** (Hyper-V test-VM rule respected; no test
  executed on this host).
* No window opened, no audio device opened, no audio played, no download,
  no dependency addition, no commit or other git write.
* Every command ran in the named worktree. Final `git status --short` is
  recorded in the lane report; it contains only owned paths.
