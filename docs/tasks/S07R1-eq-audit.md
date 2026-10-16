# Task card S07R1 — Audit Report: Equalizer DSP and Engine Wiring

> **Orchestrator notes (added after verification, 2026-10-17).** The report is a
> claim; these three points were checked against the tree.
> 1. **Finding 1 is already fixed.** The DSP chain is handed back now
>    (`DecodeOut::Returned { producer, dsp }`, merged as `8230881`), so it
>    survives consecutive runs; the audit read the pre-fix revision.
> 2. **Finding 3 is real and reproduced:** `rustamp-ui`'s `saved_to_gain_db`
>    divides by `31.0` while the classic scale spans `0..=63` around `31`, so
>    slider values `62` and `63` both give `-12.0 dB` — a one-step dead zone at
>    the bottom. It is a UI-side law, not a DSP bug, and needs an S09-anchored
>    decision (which classic step means exactly `-12 dB`) before it is touched.
> 3. **Finding 5 is the layering point already recorded in HANDOFF** and is
>    being fixed by moving the conversion into `rustamp-proto` (RFC 0011).
> The lane wrote this file into **both** the worktree and the canonical tree
> (boundary violation, kept here because the report is the deliverable).

## 1. Scope Statement

This document audits the equalizer and preamp DSP implementation introduced in Wave 32 (commit `d39ce3f`, with subsequent adjustments `a1938b2` and `d9fbdcd`) in worktree `C:/Users/micro/.local/share/opencode/worktree/26de0e/wave32-s07-audit`.

### Files Under Audit
1. `crates/rustamp-dsp/src/eq.rs` — 10-band peaking equalizer + linear preamp implemented as `DspStage`, accompanied by lock-free `EqControl` parameter sharing.
2. `crates/rustamp-audio/src/engine.rs` — Ownership of `EqState` and `EqControl`, command dispatch (`SetBand`, `SetPreamp`, `SetEqEnabled`, `SetEqAuto`), state publication (`Engine::state().eq`, `Engine::eq()`), and dynamic insertion of `Equalizer` at index 0 of the decode thread's DSP chain.
3. `crates/rustamp-app/src/app.rs` and `crates/rustamp-app/src/host.rs` — Command forwarding without logging unimplemented warnings, and host state bridge mapping `state.eq` to classic `IpcSurface` fields.

### Ground Truth & Evidence Sources
- `specs/S07-eq-dsp.md` — Equalizer interior, value laws, band steps, and signal chain order.
- `specs/S09-plugin-abi.md` — Plugin ABI definition, including `eq_set` anchors (`0 = +12 dB`, `31 = 0 dB`, `63 = -12 dB`).
- `crates/rustamp-plughost/src/state_bridge.rs` — Canonical dB to classic `0..=63` integer mapping (`eq_classic_from_db`).
- `crates/rustamp-proto/src/state.rs` — Protocol state types (`EqState`, `EQ_BANDS = 10`, `MAX_GAIN_DB = 12.0`).
- `crates/rustamp-core/src/audio.rs` — Audio pipeline contracts (`DspStage`, `AudioSpec`, `OutputDevice`).

---

## 2. Findings

### Finding 1 [HIGH]: Installed DSP Chain Discarded Across Playback Runs
- **File & Line**: `crates/rustamp-audio/src/engine.rs:882-886`, `crates/rustamp-audio/src/engine.rs:918-933`
- **Evidence**:
  ```rust
  // crates/rustamp-audio/src/engine.rs:882-886
  let mut dsp = std::mem::take(&mut self.dsp);
  dsp.insert(
      0,
      Box::new(Equalizer::with_control(Arc::clone(&self.eq_control))),
  );
  let (handle, vis_rx) = spawn(cfg, dsp, ring, index, start_at, ctl_rx, out_tx);
  ```
  In `start_run()`, `self.dsp` is drained using `std::mem::take(&mut self.dsp)` and moved into the decode thread via `spawn()`. When the playback run stops or finishes (`join_decode()`), only the ring `producer` is retrieved from `DecodeOut::Returned(producer)`; the remaining DSP stages are never returned to `self.dsp`.
- **Consequence**:
  Any user-installed DSP stages configured via `Engine::set_dsp_chain(dsp)` are permanently lost after the first track run terminates. Subsequent playback runs execute with an empty DSP chain containing only the newly instantiated `Equalizer`.

---

### Finding 2 [HIGH]: UI Window Desync & Missing `Event::EqChanged` Notification on External Commands
- **File & Line**: `crates/rustamp-audio/src/engine.rs:392-402`, `crates/rustamp-app/src/app.rs:482-484, 1018-1022`
- **Evidence**:
  1. In `engine.rs:392-402`, `Engine::set_eq()` mutates `self.eq` and updates `self.eq_control`, but does not publish `Event::EqChanged(self.eq)` on `self.bus`, despite `Event::EqChanged` being defined in `rustamp-proto/src/event.rs:62`.
  2. In `app.rs:482-484`, `apply_from(command, from_ui)` routes commands to `self.ui` (the `MainWindow`), but not to `self.eq` (the `EqualizerWindow`).
  3. In `app.rs:1018-1022`, `apply_app()` treats EQ commands as no-ops under the assumption that *"The window shows its own prediction; the signal side is the engine's equalizer stage"*.
- **Consequence**:
  When an EQ command (`SetBand`, `SetPreamp`, `SetEqEnabled`, `SetEqAuto`) is submitted externally (via plugin IPC, shortcut keys, or test harnesses), the engine audio stage and `PlayerState` snapshot update, but the UI Equalizer window state (`app.eq_state()`) does not reflect the change. The on-screen slider positions and toggle lights desynchronize from actual engine DSP state. Furthermore, external subscribers on `EventBus` never receive state notifications for EQ adjustments.

---

### Finding 3 [MEDIUM]: Asymmetry and Dead Zone in S09 Anchor Mapping (`0..=63` vs `[-12, +12]` dB)
- **File & Line**: `crates/rustamp-ui/src/equalizer.rs:75-78`, `crates/rustamp-plughost/src/state_bridge.rs:88-95`
- **Evidence**:
  - `specs/S09-plugin-abi.md:113` establishes three anchors: `0 = +12 dB`, `31 = 0 dB`, `63 = -12 dB`.
  - In `crates/rustamp-ui/src/equalizer.rs:75-78`:
    ```rust
    pub fn saved_to_gain_db(value: u8) -> f32 {
        let value = f32::from(value.min(MAX_VALUE));
        ((31.0 - value) * 12.0 / 31.0).clamp(-12.0, 12.0)
    }
    ```
    This formula uses a uniform denominator `31.0` for all values. Consequently:
    - For `value = 62`: `(31.0 - 62.0) * 12.0 / 31.0 = -12.0 dB`.
    - For `value = 63`: `(31.0 - 63.0) * 12.0 / 31.0 = -12.387 dB`, clamped to `-12.0 dB`.
    Both slider values 62 and 63 map to `-12.0 dB`, introducing a 1-step dead zone at the bottom of slider travel.
  - In `crates/rustamp-plughost/src/state_bridge.rs:92-94`:
    ```rust
    ((12.0_f32 - db) * 63.0_f32 / 24.0_f32)
        .floor()
        .clamp(0.0, 63.0) as i32
    ```
    This uses a single linear slope across `0..=63` with 24 dB total span. At `0 dB`, `12.0 * 63.0 / 24.0 = 31.5`, which `.floor()` truncates to `31`. However, inverted linearly, step `31` represents `+0.1905 dB` and step `32` represents `-0.1905 dB`.
- **Consequence**:
  The round-trip between UI slider representation, engine dB state, and classic IPC integers exhibits rounding distortion and a dead zone at maximum attenuation (`value = 62` and `63` collapse to `-12.0 dB`, mapping back to classic `63`).

---

### Finding 4 [LOW]: Non-Transactional Atomic Reads in `EqControl` During Active Writing
- **File & Line**: `crates/rustamp-dsp/src/eq.rs:135-159`, `crates/rustamp-dsp/src/eq.rs:327-345`
- **Evidence**:
  In `EqControl::set()`, 10 band gains, 1 preamp gain, and 1 enabled flag are stored using `Ordering::Relaxed`, followed by `self.generation.fetch_add(1, Ordering::Release)`.
  In `Equalizer::load_control()`, `self.seen_generation = self.control.generation()` performs an `Ordering::Acquire` load, followed by `Ordering::Relaxed` loads across the fields.
  Because there is no sequence lock protocol (e.g., odd/even sequence counter or atomic snapshot), a concurrent writer executing `set()` across multiple fields during `load_control()` can expose a partially updated set of band gains to the reader for a single audio block.
- **Consequence**:
  No undefined behavior occurs (all values are valid atomic `u32` bit patterns). The stage self-heals on the subsequent sub-block poll (within 32 frames / ~0.7 ms) when `poll_control()` detects the newly incremented generation. Given that band gains are slewed across 30 ms (`SLEW_SECONDS`), the transient intermediate state is imperceptible.

---

### Finding 5 [LOW]: Cross-Architecture In-Workspace Dev-Dependency (`rustamp-dsp` -> `rustamp-plughost`)
- **File & Line**: `crates/rustamp-dsp/Cargo.toml:14-17`, `crates/rustamp-dsp/src/eq.rs:487`
- **Evidence**:
  `rustamp-dsp` adds `[dev-dependencies] rustamp-plughost = { path = "../rustamp-plughost" }` to reuse `eq_classic_from_db` in unit tests.
  Per `AGENTS.md` and `docs/PLAN.md`, the player and engine are 64-bit while `rustamp-plughost` is compiled for `i686` as an out-of-process 32-bit bridge.
- **Consequence**:
  Compiling unit tests for `rustamp-dsp` pulls in `rustamp-plughost` on the host target, coupling core DSP test targets with the plugin host crate.

---

### Finding 6 [INFO]: Preamp Evaluated Before Peaking Filter Cascade
- **File & Line**: `crates/rustamp-dsp/src/eq.rs:37-38, 449-459`
- **Evidence**:
  In `Equalizer::process()`, `let mut y = f64::from(dry) * preamp;` scales the sample prior to ticking the 10 biquad filter sections.
  In linear time-invariant math, scalar gain commutes with linear filter transfer functions ($P \cdot H(z) = H(z) \cdot P$). However, applying preamp first affects the dynamic range stored in transposed direct form II delay registers (`z1`, `z2`).
- **Consequence**:
  Filter history stores energy scaled by the preamp. With double-precision `f64` internal state, precision loss and overflow are negligible, but transient behavior during rapid preamp adjustment differs from post-filter preamp architectures. Documented cleanly as an own choice.

---

### Finding 7 [INFO]: Silent Bypass of Bands at or Above Nyquist Guard
- **File & Line**: `crates/rustamp-dsp/src/eq.rs:89, 369-383`
- **Evidence**:
  `NYQUIST_GUARD = 0.45`. Bands where $f_0 \ge 0.45 \cdot f_s$ are flagged inactive (`active = false`) and replaced with `Coefs::IDENTITY`.
  At $f_s = 22,050$ Hz ($0.45 \cdot f_s = 9,922.5$ Hz), the 12 kHz, 14 kHz, and 16 kHz bands are bypassed. At $f_s = 11,025$ Hz, bands $\ge 6$ kHz are bypassed.
- **Consequence**:
  Prevents bilinear transform frequency wrapping and numerical instability near the Nyquist limit. The bypass occurs silently on the DSP side while `EqState` retains the nominal dB requested by the user.

---

### Finding 8 [INFO]: Verified Bit-Exact Transparency for Disabled and Flat States
- **File & Line**: `crates/rustamp-dsp/src/eq.rs:436-439, 447-466`, `crates/rustamp-audio/tests/engine.rs:677-686`
- **Evidence**:
  - When `!self.enabled && self.mix == 0.0`, `process()` returns immediately, leaving PCM samples completely untouched.
  - When enabled and flat (`db == 0.0`, `preamp == 0.0`), all 10 filter sections are marked `active = false` (`Coefs::IDENTITY`), `preamp == 1.0`, and `(dry as f64 * 1.0) as f32` preserves bit-exact identity across IEEE-754 floats.
  - Confirmed by unit tests in both `rustamp-dsp` and `rustamp-audio`.
- **Consequence**:
  No quantization noise, phase distortion, or latency overhead is introduced when the equalizer is disabled or set flat.

---

### Finding 9 [INFO]: Compliance with S07 §10 Chain Ordering Contract
- **File & Line**: `crates/rustamp-audio/src/engine.rs:37-46, 882-886`, `crates/rustamp-audio/src/decode.rs:417-425`
- **Evidence**:
  - `Equalizer` is placed at index 0 of the decode thread's DSP chain, operating on decoded PCM prior to format conversion, the ring buffer, and output volume/balance scaling.
  - Downstream DSP plugins and the visualization tap receive post-EQ audio.
- **Consequence**:
  Satisfies S07 §10 ("EQ runs on the decoded PCM stream before the final output scaling") and explicitly names the chosen ordering for auxiliary DSP stages and visualization tap.

---

## 3. Measured/Audited vs Own-Choice Table

| Parameter / Behavior | Status | Ground Truth Source | Implementation Choice / Detail |
|---|---|---|---|
| **Band Count** | Audited | `specs/S01-window-geometry.md` §3, `specs/S09-plugin-abi.md` §3.1 | 10 bands (`EQ_BANDS = 10`) |
| **Printed Band Labels** | Audited | `specs/S01-window-geometry.md` §3 | `60, 170, 310, 600, 1K, 3K, 6K, 12K, 14K, 16K` |
| **Band Center Frequencies** | Own Choice | Unmeasured in reference binary | Exact label values: `[60.0, 170.0, 310.0, 600.0, 1000.0, 3000.0, 6000.0, 12000.0, 14000.0, 16000.0]` Hz |
| **Filter Topology** | Own Choice | Unmeasured in reference binary | Transposed direct form II biquad peaking sections (Audio EQ Cookbook / R. Bristow-Johnson) with `f64` state |
| **Filter Bandwidth / Q** | Own Choice | Unmeasured in reference binary | Constant $Q = \sqrt{2} \approx 1.414$ (1-octave bandwidth) across all 10 bands |
| **Slider Endpoints & Flat Center** | Audited | `specs/S01-window-geometry.md` §3.1, `specs/S09-plugin-abi.md` §3.1 | `0 = +12 dB` (top), `31 = 0 dB` (center), `63 = -12 dB` (bottom) |
| **Slider Intermediate Law** | Own Choice | `specs/S07-eq-dsp.md` §7.2 (open item O10) | Linear in dB between anchors; `rustamp-ui` uses `(31 - v) * 12 / 31` |
| **Preamp Gain Function** | Own Choice | Unmeasured curve | Linear amplitude factor $10^{\text{dB}/20}$ |
| **Preamp Placement** | Own Choice | Unmeasured in `specs/S07-eq-dsp.md` | Input scaled by preamp prior to the 10 peaking filters |
| **Signal Chain Position** | Audited | `specs/S07-eq-dsp.md` §10 | Decoded PCM $\to$ EQ $\to$ Volume/Balance scaling |
| **Auxiliary DSP Position** | Own Choice | Contract requires explicit choice (S07 §10) | Equalizer placed at `dsp[0]`; auxiliary DSP stages receive post-EQ audio |
| **Visualization Tap Stream** | Own Choice | Open item O11 (`specs/S07-eq-dsp.md` §10) | Tap receives equalized, post-DSP audio prior to format conversion and ring |
| **Enable/Disable Transition** | Own Choice | Unmeasured in reference binary | 10 ms linear wet/dry crossfade (`FADE_SECONDS = 0.010`) |
| **Parameter Slew / De-zippering** | Own Choice | Unmeasured in reference binary | Linear slew over 30 ms (`SLEW_SECONDS = 0.030`), coefficient update every 32 frames |
| **Nyquist Guard** | Own Choice | Digital filter constraint | Bands with $f_0 \ge 0.45 \cdot f_s$ bypassed (`NYQUIST_GUARD = 0.45`) |
| **Over-Range Sample Handling** | Own Choice | Unmeasured in reference binary | No clipping or limiting applied; raw floats passed to ring buffer |
| **`auto` Flag Signal Effect** | Audited | `specs/S07-eq-dsp.md` §1 | Stored in `EqState` for preset handling; exact zero signal effect on DSP |

---

## 4. Open Questions

1. **Reference Filter Topology & Bandwidth Profile**: Does Winamp 2.95 use an IIR cascade, an FFT filter, or custom analog-modeled biquads? Do upper bands (12k, 14k, 16k) maintain constant Q or narrow to reduce excessive mutual overlap?
2. **Preamp vs Band Order**: Does Winamp 2.95 evaluate the preamp factor before the band filters or after?
3. **Low Sample-Rate Behavior**: How does Winamp 2.95 handle streams at 22,050 Hz or lower when high-frequency bands (12k, 14k, 16k) are boosted? Does it clamp, fold, or disable those bands?
4. **Intermediate Slider-to-dB Function**: Is the gain progression truly linear in dB across `0..=63`, or is there a deadband / nonlinear taper around 0 dB?
5. **Saturation and Clipping Policy**: How does Winamp 2.95 respond to digital full-scale overshoots caused by extreme boosts (+12 dB preamp combined with +12 dB band boost)? Does it hard clip at 16-bit integer bounds or soft saturate?
6. **Auxiliary DSP Ordering (S07 O11 / S09 U-12)**: In Winamp 2.95, when both a DSP plug-in (`dsp_process`) and the built-in equalizer are active, which stage processes the PCM buffer first?
7. **Preset File Format & Submenu Actions (S07 O6 / O12)**: What are the exact semantics of the Presets menu (Load / Save / Delete) and how do they map to `winamp.ini` keys?

---

## 5. Unverified

1. **Digital Filter Responses Against 2.95 Output**: Frequency and phase response curves of `rustamp-dsp` peaking filters have not been compared against audio captured from 2.95 via `out_disk`.
2. **Extreme Headroom / Dynamic Range**: Behavior under extreme boost (+24 dB total gain) followed by downstream attenuation has not been verified against reference PCM captures.
3. **IPC 128 External Mutation Loop**: Out-of-process IPC 128 (`IPC_SETEQDATA`) write back into active playback has not been verified end-to-end with a live 32-bit Winamp DSP DLL.
4. **Behavior on Subnormal Floats**: Flushing of subnormal numbers (`flush_tiny()`, $10^{-200}$) has not been verified on non-x86 architectures (e.g. ARM flush-to-zero mode).
5. **Sample Rate Edge Cases**: Dynamic format changes mid-stream across the Nyquist threshold (e.g. 44.1 kHz $\to$ 22.05 kHz) have not been exercised with live playback tests.
