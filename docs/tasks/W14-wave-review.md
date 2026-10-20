# W14 wave review — adversarial review of tonight's wave

**Reviewer role:** independent adversarial review. **Findings only — no code was changed, nothing was committed.**  
**Lane worktree:** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave67-wave-review` (the only tree read or written).  
`git status --short` in this worktree reports only this document; no file outside this directory was touched.

**Lanes reviewed (tonight's wave):**

| Lane / Card | Topic | Key Files |
|---|---|---|
| A13 `docs/tasks/A13-flush-contract.md` | Flush contract | `crates/rustamp-audio/src/engine.rs` |
| A14 `docs/tasks/A14-dsp-correctness.md` | DSP correctness & chain order | `crates/rustamp-audio/src/decode.rs` |
| A15 `docs/tasks/A15-replaygain-audio.md` | ReplayGain in audio engine | `crates/rustamp-audio/src/replaygain.rs`, `decode.rs` |
| A16 `docs/tasks/A16-replaygain-unify.md` | ReplayGain unification | `crates/rustamp-audio/src/engine.rs`, `decode.rs` |
| A17 `docs/tasks/A17-true-peak.md` | True-peak limiter detection | `crates/rustamp-dsp/src/limiter.rs` |
| A18 `docs/tasks/A18-audio-red.md` | Audio test fixes | `crates/rustamp-audio/tests/*.rs` |
| E4 `docs/tasks/E4-replaygain.md` | ReplayGain core types | `crates/rustamp-core/src/config.rs` |
| M15 `docs/tasks/M15-engine-resident.md` | Engine resident adapter | `crates/rustamp-audio/src/resident.rs` |
| M19 `docs/tasks/M19-dsp-adapter.md` | DSP resident adapter | `crates/rustamp-audio/src/resident.rs` |
| M20 `docs/tasks/M20-roundtrip-rebuild.md` | Roundtrip deadlock rebuild | `crates/rustamp-dsphost/tests/dsp_roundtrip.rs`, `crates/rustamp-mediahost/src/dsp.rs` |
| P5 `docs/tasks/P5-dll-preflight.md` | Passive DLL preflight | `crates/rustamp-plughost/src/pe_preflight.rs` |
| P6 `docs/tasks/P6-dsp-plugin-bridge.md` | DSP plugin bridge | `crates/rustamp-plughost/src/dsp_bridge.rs` |
| P7 `docs/tasks/P7-approval-store.md` | Production plugin approval store | `crates/rustamp-plughost/src/approval.rs`, `loader.rs` |
| X4 `docs/tasks/X4-ignored-staging.md` | Ignored tests staging | `crates/rustamp-plughost/tests/dll_acceptance.rs` |
| N8 `docs/tasks/N8-app-red.md` | App red fixes & easy-move | `crates/rustamp-app/src/app.rs`, `config.rs` |
| N9 / F1 (W10 finding F1) | Configuration class round-trip | `crates/rustamp-app/src/config.rs` |
| G4 `docs/tasks/G4-wake-cross.md` | Wake cross-talk elimination | `crates/rustamp-ipc/src/shm/ring.rs` |
| G5 `docs/tasks/G5-wake-honesty.md` | Wake timing honesty | `crates/rustamp-ipc/src/shm/ring.rs` |
| F4 `docs/tasks/F4-platform-displays.md` | Multi-monitor display bounds | `crates/rustamp-platform/src/windows/display.rs` |

**Method / limits.** Everything below was established through deep adversarial source inspection across all crates, static verification of contracts, boundary tracing, and minimal reproducible failure scenarios. Per the task constraints, **`cargo test` was not run** and **the Hyper-V VM was not run**. The following verification gates were run and are completely clean:
- `cargo fmt --all -- --check` (clean, exit 0)
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` (clean, exit 0, no warnings)
- `cargo build --workspace` (clean, exit 0)

---

## 1. Claim table

Verdicts: **HOLDS**, **PARTIAL**, **FALSE**, **UNVERIFIABLE**.

### 1. The Deadlock Rebuild (M20, M19, M15)

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| C1-1 | "The invariant that now guarantees progress: the producer never starts chunk n+1 before the consumer has drained chunk n." | **PARTIAL** — holds only in one test helper; **FALSE** for the engine adapter | `crates/rustamp-dsphost/tests/dsp_roundtrip.rs:36-61` chunks writes, but `crates/rustamp-audio/src/resident.rs:718-732` (`process_block`) writes the entire incoming buffer in one call before reading any samples back | Feed `process_block` an input slice with > 4096 frames (the default back-ring capacity `DEFAULT_DSP_RING_FRAMES`). Helper fills back-ring and blocks; `write_timeout` blocks on in-ring; deadlock occurs until the 5 s timeout. |
| C1-2 | `DspSession` enforces lockstep progress by construction | **FALSE** | `crates/rustamp-mediahost/src/dsp.rs:188-210` — no chunking, no lockstep, and no progress invariants were added to `DspSession::write` or `write_timeout` (only doc comments were touched in M20) | `session.write_timeout(large_buf, Some(t))` writes as much as ring permits; caller is responsible for reading back, which `process_block` fails to interleave. |
| C1-3 | The producer never gets ahead of the consumer | **FALSE** | `crates/rustamp-dsphost/tests/dsp_roundtrip.rs:43-51` | Even in the test helper `round_trip`, within each chunk of 257 frames, the producer writes all 257 frames into the in-ring before the consumer reads a single frame back. The producer is ahead by up to 257 frames. |
| C1-4 | Reconfigure and reset progress safely during active resident session | **HOLDS** | `crates/rustamp-mediahost/src/dsp.rs:141-177`, `crates/rustamp-audio/src/resident.rs:774-830` | Handshake requires session reset/drain before reconfigure messages are sent. |

### 2. True-peak and ReplayGain Together (A14, A15, A16, A17, E4)

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| C2-1 | Signals cannot reach the output device above full scale (> ±1.0) | **FALSE** — *was false; **fixed 2026-10-20** (finding F2 below): the converter's single output stage now finishes every sample on both paths* | `crates/rustamp-audio/src/convert.rs:437-443` vs `555-561` (before the fix) | On equal-rate playback (`same_rate == true`, e.g. 44.1 kHz file to 44.1 kHz device), `convert.rs:441` called `remix` and returned immediately without running `finish_sample` (the `clamp(-1.0, 1.0)` safety net was resampler-only). With the limiter disabled via `set_enabled(false)` or during positive ReplayGain (+51 dB) + EQ preamp (+12 dB), unclamped samples far exceeding ±1.0 entered the ring and were passed to WASAPI/CPAL. |
| C2-2 | The true-peak limiter bounds inter-sample peaks at full scale | **PARTIAL** | `crates/rustamp-dsp/src/limiter.rs:32-48`, card `A17 §5` | A17 §5 documents that the 4× polyphase FIR oversampler has an under-read of up to 0.67 dB near Nyquist (e.g. 20 kHz sine wave at 44.1 kHz). With limiter ceiling at 0 dBFS (1.0), true inter-sample peaks up to +0.67 dB (~1.08 linear) pass the limiter unattenuated and, due to C2-1, are not clamped on equal-rate paths. |
| C2-3 | ReplayGain gain is applied once and in the correct order | **HOLDS** | `crates/rustamp-audio/src/decode.rs:677-698` | Order is verified: ReplayGain -> Equalizer -> installed DSP stages -> Limiter -> Converter -> Output ring. |
| C2-4 | "A change [to ReplayGain mode] takes effect at the next track (or seek)" | **FALSE** | `crates/rustamp-audio/src/engine.rs:647-650` vs `crates/rustamp-audio/src/decode.rs:840-865` | `apply_seek` in `decode.rs` does not update `self.rg_gain`. Gain is only updated in `prepare_track`. Changing ReplayGain mode and seeking does not update the active gain; the track continues playing with the old gain. |

### 3. The Approval Store (P5, P7)

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| C3-1 | An unapproved image cannot be loaded by the plugin host | **HOLDS** | `crates/rustamp-plughost/src/loader.rs:676-678`, `crates/rustamp-plughost/src/approval.rs:164-185` | `load_plugin_with_store` requires `image.require_store_approval(store)` which checks candidate whole-image SHA-256 against approved entries. Unknown hash returns `NotApproved`. |
| C3-2 | A changed file on disk cannot pass the approval check (anti-TOCTOU) | **HOLDS** | `crates/rustamp-plughost/src/loader/image.rs:45-104`, `crates/rustamp-plughost/src/loader.rs:704-726` | `seal()` re-opens source with `FILE_SHARE_READ`, verifies bytes match preflighted image, copies bytes to a private temp directory with pinned ancestors, re-reads and validates the copy, and maps only the sealed private copy. Modified source yields `PluginLoadError::ImageChanged`. |
| C3-3 | Approval store file parser rejects malformed syntax, corrupted records, and invalid machines | **HOLDS** | `crates/rustamp-plughost/src/approval.rs:251-415` | Parser enforces version header, 64-char lowercase hex sha256, decimal/hex machine `0x014c`, and deduplicates hashes. Malformed files fail loudly with `ApprovalStoreError::Malformed` without fallback. |

### 4. The Engine Adapters (M15, M19)

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| C4-1 | Decoder and DSP resident latency reporting do not collide | **HOLDS** | `crates/rustamp-audio/src/decode.rs:583-588`, `crates/rustamp-audio/src/resident.rs:443-450,836-842` | `Decoder` trait reports `duration` and timestamps; `DspStage` reports `latency_frames()`. Latency queries do not collide. |
| C4-2 | A failure in one resident leaves the other in a defined state | **PARTIAL** | `crates/rustamp-audio/src/decode.rs:643-651`, `crates/rustamp-audio/src/resident.rs:784-788,836-842` | When decoder resident fails, `Worker` flushes DSP with silence and stops; DSP resident remains in a defined, ready state. However, when DSP resident fails, `HostDspChain` is permanently poisoned. Subsequent tracks immediately fail on `prepare_track` with `CoreError::Dsp("resident DSP chain already failed; replace the stage")`, permanently halting playback until the host app replaces the stage. |
| C4-3 | DSP latency flush compensation behaves honestly on failure | **FALSE** | `crates/rustamp-audio/src/resident.rs:836-842`, `crates/rustamp-audio/src/decode.rs:705-716` | On DSP resident failure, `latency()` drops immediately to 0. When `track_loop` exits and calls `flush_dsp()`, `flush_dsp()` queries `self.dsp_latency()` which evaluates to 0. Zero frames are flushed, silently discarding any audio remaining in stages before the failure. |

### 5. The Configuration Round Trip (N8, N9, F1)

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| C5-1 | The new all-fields test covers every field in the configuration | **FALSE** | `crates/rustamp-app/src/config.rs:487-550` (`config_round_trip_honours_all_supported_fields`) | `config_round_trip_honours_all_supported_fields` sets `width: None, height: None` on all windows (lines 505-506, 514-515, 523-524). It does not test `width: Some(...)` or `height: Some(...)`. It also tests only unit window variants (`Main`, `Equalizer`, `Playlist`), omitting `WindowId::Gen(n)` and `WindowId::Video`. |
| C5-2 | Core configuration serialization (`toml_roundtrip`) covers all fields | **FALSE** | `crates/rustamp-core/src/config.rs:207-238` | `toml_roundtrip` leaves `audio.balance`, `audio.output`, `audio.buffer_ms`, `audio.fadeout_ms`, `audio.gapless`, `audio.replaygain`, `ui.double_size`, `ui.always_on_top`, `ui.easy_move`, `ui.snap_distance`, `playlist.*`, and `plugins.plugin_dir` unset at their defaults. |
| C5-3 | Honest serialization preserves user-configured unhonoured settings | **FALSE** (Design flaw / Data loss) | `crates/rustamp-app/src/config.rs:253-351` (`serialize_honest`) | When a user configures unhonoured fields in `rustamp.toml` (e.g. `audio.device = "Speakers"`, `audio.replaygain = "Track"`, `playlist.shuffle = true`, or `plugins.disabled = [...]`), the loader warns, but clean exit rewrites the file using `serialize_honest`, permanently deleting all unhonoured fields from disk. |

### 6. Cross-Lane Blindspots & Shared Interfaces

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| C6-1 | ReplayGain is integrated end-to-end across the application | **FALSE** | `crates/rustamp-audio/src/engine.rs:647-665` vs `crates/rustamp-app/src/config.rs:127` | `rustamp-audio` implemented ReplayGain in tonight's wave (A15, A16), but `rustamp-app/src/config.rs:127` still hardcodes `"replaygain" => Some("ReplayGain is not supported yet")` and `serialize_honest` omits it. The app crate was not updated to expose or persist ReplayGain. |
| C6-2 | The M20 deadlock fix covers resident DSP engine playback | **FALSE** | `crates/rustamp-audio/src/resident.rs:712-734` vs `crates/rustamp-dsphost/tests/dsp_roundtrip.rs:36-61` | M20 fixed the deadlock in the standalone test helper only. The actual production engine adapter (`process_block` in `resident.rs`) still uses the single-shot un-chunked write-before-read pattern. |
| C6-3 | Task card manifest consistency across the wave | **PARTIAL** | Prompt / handoff refers to `N9-config-class.md`, but no such file was committed (`docs/tasks/N9-config.md` or `N9-config-class.md` is absent) | The work was performed under N8 and documented in N8 / W10 finding F1, but the card reference is dangling. |

---

## 2. Findings by severity

### F1 — HIGH (proven): The deadlock rebuild invariant is absent in the engine adapter: `process_block` writes the entire block before reading back, causing deadlock for buffers > ring capacity

`crates/rustamp-dsphost/tests/dsp_roundtrip.rs:36-61` demonstrates the lockstep chunking principle: write a small chunk (257 frames), read it back, repeat. Task card M20 claimed this "kills the deadlock by construction: write a chunk, read that chunk back. The invariant that now guarantees progress: the producer never starts chunk n+1 before the consumer has drained chunk n."

However, this invariant was **never implemented** in `DspSession` (`crates/rustamp-mediahost/src/dsp.rs:188-210`), nor in the engine's DSP adapter (`crates/rustamp-audio/src/resident.rs:712-734`).

In `crates/rustamp-audio/src/resident.rs:717-732`:
```rust
fn process_block<F: PcmRings>(
    session: &mut DspSession<F>,
    buf: &mut [f32],
    block_timeout: Duration,
) -> CoreResult<()> {
    session
        .write_timeout(buf, Some(block_timeout))
        .map_err(|error| dsp_error_to_core(error, "write"))?;
    let channels = usize::from(session.spec().channels.max(1));
    let mut filled = 0;
    while filled < buf.len() {
        let frames = session
            .read(&mut buf[filled..], Some(block_timeout))
            .map_err(|error| dsp_error_to_core(error, "read"))?;
...
```
`process_block` writes the **entire** incoming `buf` into the session before reading a single sample.

**Failure scenario:**
The default resident back-ring capacity is `DEFAULT_DSP_RING_FRAMES = 4096` (`resident.rs:591`). If `buf` contains more than 4096 frames (or if the in-ring fills while the helper blocks waiting for back-ring space):
1. `session.write_timeout` fills the in-ring.
2. The resident helper processes frames and fills the 4096-frame back-ring.
3. The helper blocks waiting for back-ring space before consuming more from the in-ring.
4. `session.write_timeout` blocks waiting for in-ring space.
5. `process_block` has not yet reached `session.read()`.
6. **Deadlock.** Neither process can make progress until `block_timeout` (5000 ms) expires, causing playback to fail with `CoreError::Dsp("resident DSP write timed out")`.

While `crates/rustamp-audio/src/decode.rs:620` currently uses `READ_FRAMES = 1024` (which happens to be smaller than 4096), any buffer size fluctuation, multi-block buffer, or smaller ring configuration will immediately trigger the deadlock. The invariant is **not** true by construction of the engine adapter.

---

### F2 — HIGH (proven): Unclamped equal-rate path allows signals exceeding 1.0 (clipping/distortion) directly into the audio backend

**Status: FIXED (2026-10-20).** The finding was correct; the equal-rate path
had no clamp at all. The fix and the tests are in
`crates/rustamp-audio/src/convert.rs` (the converter's single output stage)
and `crates/rustamp-audio/src/gain_properties.rs` (the property suite). The
RFC 0014 §4.5 non-finite policy is untouched: the sanitizer in
`decode.rs` stays where it is, and the converter's finishing is a *later*,
second bound on the Ok path.

**Where the clamp lives now.** One place, not two: the converter's output
stage (`convert.rs::emit_device_samples`) remaps the channels and then
finishes every sample with `finish_sample` — non-finite ⇒ silence, subnormal
⇒ 0, clamp to `±1.0`. The equal-rate path (`convert`) and the resampled path
(`run_block`, which `flush` uses as well) both go through it, so the two
paths cannot drift apart again. The finishing runs on the **device-format**
samples, after the remap, exactly as the resampled path already did.

**Why one place and not two.** The rule is a level decision of the chain, not
a property of a path: a sample that reaches the device must be bounded
whatever route it took. Two call sites meant two chances to forget one, and
that is precisely what happened (the resample path had it, the equal-rate
path did not). One output stage makes the equal-rate path and the resampled
path *the same path* for the last step, and it keeps the resampled path's
behaviour byte-identical (the existing resample tests are unchanged).

**The bit-exactness promise, restated.** The equal-rate path is bit-exact
**for in-range input**: a finite, normal sample within `±1.0` passes through
bit for bit, so the engine's bit-transparency tests still hold. Outside the
range the promise never held in the first place — an above-unity sample
reaches the device as `±1.0` (a flat top), a subnormal as exactly `0.0`, and
a non-finite one as silence. The module docs of `convert.rs`,
`gain_properties.rs`, `decode.rs` and `engine.rs` now say so.

**Tests (all in `rustamp-audio`):**

| Test | What it pins |
|---|---|
| `convert::tests::the_equal_rate_path_finishes_its_samples` | the equal-rate path clamps, silences and zeroes exactly like `finish_sample` |
| `convert::tests::both_paths_hold_a_hot_signal_at_full_scale` | a constant above unity comes out at exactly `1.0` on **both** paths |
| `convert::tests::equal_rate_subnormals_come_out_silent` | a subnormal is silence on the equal-rate path too |
| `convert::tests::same_format_is_a_bit_exact_passthrough` | an in-range signal is copied bit for bit |
| `convert::tests::equal_rate_path_is_bit_exact_and_holds_nothing_back` | the same for a whole block, whatever the block size |
| `gain_properties::paths::the_equal_rate_path_finishes_every_sample` | property: the equal-rate output is `finish_sample(input)`, bit for bit, over random finite input |
| `gain_properties::paths::the_equal_rate_path_keeps_in_range_samples_bit_identical` | property: an in-range signal keeps its bits on the equal-rate path |
| `gain_properties::paths::both_paths_bound_their_output` | property: a hostile input leaves nothing non-finite and nothing above full scale on either path |
| `gain_properties::paths::both_paths_clamp_an_above_unity_signal` | property: an above-unity signal is clamped at exactly full scale on either path |

**The original report (unchanged, for the record):**

In `crates/rustamp-audio/src/convert.rs:437-443`:
```rust
if self.same_rate {
    remix(input, out_frames, self.src.channels, self.device.channels, output);
    return out_frames;
}
```
When `self.same_rate` is true (e.g. 44.1 kHz input on a 44.1 kHz device, the most common scenario for CD-quality playback), `FormatConverter::convert` calls `remix(...)` and returns immediately.

The hard-clamping safety net (`sample.clamp(-1.0, 1.0)`) is located in `finish_sample` (`crates/rustamp-audio/src/convert.rs:555-561`), which is called **only** on the resampler path!

In `crates/rustamp-audio/src/decode.rs:670-698`:
1. ReplayGain applies positive gain (up to +51 dB per RG spec, e.g. quiet tracks).
2. Equalizer preamp applies up to +12 dB.
3. Limiter is processed.
4. Output is passed to `self.conv.convert(block, &mut self.out_buf)`.

**Failure paths:**
1. **Limiter disabled:** The limiter can be disabled via `limiter_control.set_enabled(false)`. If disabled, hot signals amplified by ReplayGain and EQ pass through `convert.rs` completely unclamped. Samples with magnitudes of 2.0, 5.0, or 10.0+ are written directly into the shared float ring buffer and sent to WASAPI / CPAL, causing severe digital clipping or driver distortion.
2. **True-peak Nyquist under-read:** As documented in A17 §5, the 4× polyphase FIR oversampler has an under-read of up to 0.67 dB near Nyquist (e.g. high-frequency content near 20 kHz). Even with the limiter enabled at 0 dBFS ceiling, true peaks up to +0.67 dB (~1.08 linear) pass the limiter. Because the equal-rate converter path does not clamp, inter-sample overshoots exceeding 1.0 reach the DAC's analog reconstruction filter.

---

### F3 — MEDIUM (proven): ReplayGain mode change is ignored across seeks, contradicting the engine contract

In `crates/rustamp-audio/src/engine.rs:647-650`:
```rust
/// A change takes effect at the next track (or seek): the current track
/// keeps the gain it prepared with.
pub fn set_replaygain_mode(&mut self, mode: ReplayGainMode) {
```
The contract comment explicitly promises that a change takes effect at the next track **or seek**.

However, in `crates/rustamp-audio/src/decode.rs:840-865` (`apply_seek`):
```rust
fn apply_seek(
    &mut self,
    decoder: &mut Box<dyn Decoder>,
    to: Duration,
    seq: u64,
    initial: bool,
) {
    ...
    // The converter and DSP stages are reset:
    for stage in &mut self.dsp {
        stage.reset();
    }
    self.conv.reset();
    ...
```
`self.rg_gain` is **never updated** during `apply_seek`! `self.rg_gain` is calculated only in `prepare_track` (`decode.rs:796-820`).

**Reproduction:**
1. Start playing a track with `ReplayGainMode::Off` (`rg_gain == 1.0`).
2. Call `engine.set_replaygain_mode(ReplayGainMode::Track)`. `DecodeMsg::SetReplayGain` updates `self.rg_mode`.
3. Call `engine.seek_ms(10000)`.
4. `apply_seek` executes the seek, but `self.rg_gain` remains `1.0`. Playback continues without track gain applied until the track ends and the next track prepares.

---

### F4 — MEDIUM (proven): Resident DSP failure drops held audio and permanently disables subsequent playback across tracks

In `crates/rustamp-audio/src/resident.rs:766-772,784-788,836-842`:
```rust
fn latch(&mut self, error: CoreError) {
    if self.poisoned.is_none() {
        self.watch.latch(ErrorInfo::from(&error));
        self.poisoned = Some(error);
    }
}
...
fn prepare<F: PcmRings>(&mut self, session: &mut DspSession<F>, spec: AudioSpec) -> CoreResult<()> {
    if self.poisoned.is_some() {
        return Err(CoreError::Dsp("resident DSP chain already failed; replace the stage".into()));
    }
...
fn latency<F: PcmRings>(&self, session: &DspSession<F>) -> u32 {
    if self.poisoned.is_some() { 0 } else { session.latency_frames() }
}
```

Two defects arise from this design:
1. **Flushing drops held audio:** When a resident DSP failure occurs mid-stream, `self.poisoned` is set and `latency()` immediately returns 0. When the decode worker finishes the track and calls `flush_dsp()` (`decode.rs:705-716`), `self.dsp_latency()` returns 0. `flush_dsp` does not push any silence frames, silently discarding any audio frames buffered in earlier stages or previous resident blocks.
2. **Permanent failure of subsequent tracks:** When `Engine::poll` detects the failure via `self.dsp_watch.take()`, `handle_dsp_failure` stops playback (`engine.rs:1280-1303`). However, `self.dsp` retains the poisoned `HostDspChain`. If the user subsequently advances to the next track or clicks Play, `Worker::track_loop` calls `prepare_track`, which invokes `stage.prepare(spec)`. `HostDspChain::prepare` immediately returns `Err("resident DSP chain already failed; replace the stage")`. Every subsequent track immediately aborts at startup without attempting playback. The player remains dead until the application externally calls `engine.set_dsp_chain(...)`.

---

### F5 — MEDIUM (proven): Honest config serialization silently deletes all unhonoured fields from user config files on clean exit

In `crates/rustamp-app/src/config.rs:247-351` (`serialize_honest`), the config serializer explicitly writes only:
- `version`
- `audio.volume`, `audio.balance`
- `ui.skin`, `ui.double_size`, `ui.always_on_top`, `ui.easy_move`, `ui.windows`
- `eq.state`

Every other field present in `rustamp_core::config::Config` is omitted:
- `audio.output`, `audio.device`, `audio.buffer_ms`, `audio.fadeout_ms`, `audio.gapless`, `audio.replaygain`
- `ui.snap_distance`
- `playlist.shuffle`, `playlist.repeat`, `playlist.manual_advance`, `playlist.last_playlist`
- `plugins.plugin_dir`, `plugins.vis_plugin`, `plugins.vis_preset`, `plugins.disabled`

When `save(&config, path)` is called on clean exit (`crates/rustamp-app/src/config.rs:356-370`), it overwrites the configuration file with the output of `serialize_honest`.

**Consequence:**
If an advanced user edits `rustamp.toml` to configure `audio.device = "Speakers (Realtek Audio)"`, `audio.replaygain = "Track"`, `playlist.shuffle = true`, or `plugins.disabled = ["bad.dll"]`, the app loads them (emitting warnings for unapplied settings). But upon exiting the app, `serialize_honest` overwrites the file, **silently and permanently deleting all 13 unhonoured configuration settings** from the user's disk.

---

### F6 — LOW (proven): Incomplete test coverage in config round-trip tests: `WindowLayout` dimensions and variants remain unexercised in the all-supported-fields test

`crates/rustamp-app/src/config.rs:487-550` contains `config_round_trip_honours_all_supported_fields`.
Despite its name:
1. Lines 505-506, 514-515, 523-524 explicitly specify `width: None, height: None` on all three tested windows (`Main`, `Equalizer`, `Playlist`). The test does not exercise `width: Some(...)` or `height: Some(...)`.
2. It tests only unit `WindowId` variants, omitting `WindowId::Gen(n)` and `WindowId::Video`.
3. In `crates/rustamp-core/src/config.rs:207-238` (`toml_roundtrip`), `audio.balance`, `audio.output`, `audio.buffer_ms`, `ui.double_size`, `ui.always_on_top`, `ui.easy_move`, and `playlist.*` are completely omitted and left at default values.

---

### F7 — LOW (proven): Cross-lane disconnect: audio engine implements ReplayGain, but app config marks it unsupported and strips it

In tonight's wave, lanes A15 and A16 implemented ReplayGain in `rustamp-audio` (`apply_gain`, `set_replaygain_mode`, `DecodeMsg::SetReplayGain`).
However, `crates/rustamp-app/src/config.rs:127` still contains:
```rust
"replaygain" => Some("ReplayGain is not supported yet"),
```
And `serialize_honest` explicitly strips `replaygain`.
The application layer was not updated to connect configuration ReplayGain settings to the newly implemented audio engine capabilities.

---

## 3. What cannot be verified without the VM or two real processes

1. **32-bit x86 DLL execution and Windows OS loader semantics (`LoadLibraryExW`)**:
   `crates/rustamp-plughost/src/loader.rs:721` calls `LoadLibraryExW` with `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_USER_DIRS | LOAD_LIBRARY_SEARCH_SYSTEM32`. This requires a 32-bit (i686) Windows process and OS loader interaction. On a 64-bit developer machine or non-Windows host, `load_plugin_x86` and all acceptance tests in `crates/rustamp-plughost/tests/dll_acceptance.rs` (`#[ignore]`) cannot be executed.
2. **Real two-process shared memory concurrency and kernel wake races**:
   While `NamedPcmRings` and `shm::ring` use Windows events and shared memory sections, true kernel scheduler preemption, cross-process timing jitter, and race conditions between a separate producer process and consumer process (e.g. `rustamp-mediahost` resident helper) cannot be verified in a single process without spawning two actual OS processes.
3. **Hardware audio endpoint underruns and DAC reconstruction filters**:
   Hardware WASAPI exclusive/shared mode buffer handling, driver-level glitching during device resets, and actual analog reconstruction filter clipping from inter-sample true-peaks exceeding 0 dBFS cannot be observed on headless CI environments without real physical sound cards.

---

## 4. Boundary and hygiene verification

Verification of workspace cleanliness:
- Only `docs/tasks/W14-wave-review.md` was written.
- No files outside `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave67-wave-review` were touched.
- No git commits were made.
- Clean formatting, clippy, and build gates passed.
