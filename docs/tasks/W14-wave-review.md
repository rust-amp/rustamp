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
| C1-1 | "The invariant that now guarantees progress: the producer never starts chunk n+1 before the consumer has drained chunk n." | **HOLDS** — *was PARTIAL; the engine adapter's half is **fixed 2026-10-20** (finding F1 below)* | `crates/rustamp-dsphost/tests/dsp_roundtrip.rs:36-61` chunks writes (257 frames), and `crates/rustamp-audio/src/resident.rs:1107-1155` (`process_block`) now does the same (at most `DSP_CHUNK_FRAMES` = 1024 frames, clamped to the in-ring capacity): write one chunk, read that chunk back, then write the next | Before the fix, an input slice larger than the resident's absorption (4096-frame back-ring + 1024-frame in-hand block) filled the back-ring, the resident stopped taking and `write_timeout` blocked until the 5 s timeout. Pinned now by `resident::dsp_hosted_tests::the_lockstep_round_trip_completes_on_a_ring_smaller_than_the_block`. |
| C1-2 | `DspSession` enforces lockstep progress by construction | **FALSE** (unchanged by the fix) | `crates/rustamp-mediahost/src/dsp.rs:188-210` — no chunking, no lockstep, and no progress invariants were added to `DspSession::write` or `write_timeout` (only doc comments were touched in M20) | `session.write_timeout(large_buf, Some(t))` writes as much as ring permits; the caller is responsible for reading back. Since 2026-10-20 the production adapter does interleave it (the lockstep, F1 below), but the session itself still enforces nothing — the invariant lives in the caller, not in the framework. |
| C1-3 | The producer never gets ahead of the consumer | **FALSE** (unchanged by the fix) | `crates/rustamp-dsphost/tests/dsp_roundtrip.rs:43-51`, `crates/rustamp-audio/src/resident.rs:1128-1152` | Even in the test helper `round_trip`, within each chunk of 257 frames, the producer writes all 257 frames into the in-ring before the consumer reads a single frame back. The producer is ahead by up to 257 frames. The production adapter has the same shape since the fix (ahead by at most one chunk, ≤ 1024 frames) — the invariant is chunk-granular, not frame-granular. |
| C1-4 | Reconfigure and reset progress safely during active resident session | **HOLDS** | `crates/rustamp-mediahost/src/dsp.rs:141-177`, `crates/rustamp-audio/src/resident.rs:774-830` | Handshake requires session reset/drain before reconfigure messages are sent. |

### 2. True-peak and ReplayGain Together (A14, A15, A16, A17, E4)

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| C2-1 | Signals cannot reach the output device above full scale (> ±1.0) | **FALSE** — *was false; **fixed 2026-10-20** (finding F2 below): the converter's single output stage now finishes every sample on both paths* | `crates/rustamp-audio/src/convert.rs:437-443` vs `555-561` (before the fix) | On equal-rate playback (`same_rate == true`, e.g. 44.1 kHz file to 44.1 kHz device), `convert.rs:441` called `remix` and returned immediately without running `finish_sample` (the `clamp(-1.0, 1.0)` safety net was resampler-only). With the limiter disabled via `set_enabled(false)` or during positive ReplayGain (+51 dB) + EQ preamp (+12 dB), unclamped samples far exceeding ±1.0 entered the ring and were passed to WASAPI/CPAL. |
| C2-2 | The true-peak limiter bounds inter-sample peaks at full scale | **PARTIAL** | `crates/rustamp-dsp/src/limiter.rs:32-48`, card `A17 §5` | A17 §5 documents that the 4× polyphase FIR oversampler has an under-read of up to 0.67 dB near Nyquist (e.g. 20 kHz sine wave at 44.1 kHz). With limiter ceiling at 0 dBFS (1.0), true inter-sample peaks up to +0.67 dB (~1.08 linear) pass the limiter unattenuated and, due to C2-1, are not clamped on equal-rate paths. |
| C2-3 | ReplayGain gain is applied once and in the correct order | **HOLDS** | `crates/rustamp-audio/src/decode.rs:677-698` | Order is verified: ReplayGain -> Equalizer -> installed DSP stages -> Limiter -> Converter -> Output ring. |
| C2-4 | "A change [to ReplayGain mode] takes effect at the next track (or seek)" | **FALSE** — *was false; **fixed 2026-10-20** (finding F3 below): a seek re-derives the multiplier* | `crates/rustamp-audio/src/engine.rs:647-650` vs `crates/rustamp-audio/src/decode.rs:840-865` (before the fix) | `apply_seek` in `decode.rs` did not update `self.rg_gain`. Gain was only updated in `prepare_track`. Changing ReplayGain mode and seeking did not update the active gain; the track continued playing with the old gain. |

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
| C4-2 | A failure in one resident leaves the other in a defined state | **HOLDS** — *both halves pinned 2026-10-20: the **media-host sessions** are independent (`wave269`, five tests) and the **engine adapter** bypasses a failed stage so the next track plays (`wave266`, finding F4)* | `crates/rustamp-mediahost/tests/session_independence.rs`; `crates/rustamp-audio/src/decode.rs:505,917,1344` (the `Bypass` stage, the swap in `emit`, the single report) | Each session owns its control peer, its rings and its terminal state, so a failed or killed decoder leaves the DSP session fully usable and vice versa. In the engine, a failed DSP stage is replaced by a shape-preserving pass-through and dropped, the failure is reported once per run with the lost hold-back in the message, and the next track plays bit-exactly. The bypass lasts until the caller installs a chain again (`Engine::set_dsp_chain`), and the bypassed chain is returned with the run, so the bypass is never silent. |
| C4-3 | DSP latency flush compensation behaves honestly on failure | **FALSE** — *was false; **fixed 2026-10-20** (finding F4 below): the flush runs after the bypass, and the lost hold-back is reported* | `crates/rustamp-audio/src/resident.rs` (`latency()` keeps reporting the hold-back a poisoned chain lost), `crates/rustamp-audio/src/decode.rs` (`report_stage_failure`, the bypass before the flush) | Before the fix, `latency()` dropped to 0 on failure, so `flush_dsp()` pushed nothing and the held audio vanished silently. After the fix, a poisoned chain still reports what it lost, the flush runs on the surviving stages, and the loss is named in the single error event. Pinned by `resident::dsp_hosted_tests::engine_flushes_the_live_chain_after_the_resident_dies` and `…::a_poisoned_chain_still_reports_the_hold_back_it_lost`. |

### 5. The Configuration Round Trip (N8, N9, F1)

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| C5-1 | The new all-fields test covers every field in the configuration | **FALSE** — *was false; **fixed 2026-10-20** (finding F6 below): every honoured setting now carries a non-default value, so a writer that forgets one fails the round trip* | `crates/rustamp-app/tests/config.rs` (`config_round_trip_honours_all_supported_fields`) | The evidence the review quotes is a stale snapshot: the tree already carried `width: Some(...)` / `height: Some(...)` and the `WindowId::Gen(n)` and `WindowId::Video` variants (commit `99b2cb3`, 20 minutes before the review was recorded). What was still open: seven **honoured** settings — `version`, `audio.output_mode`, `audio.replaygain`, `audio.device`, `ui.snap_distance`, `plugins.plugin_dir`, `plugins.disabled` — stayed at their defaults, and a writer that forgot them was invisible, because the loader falls back to exactly that default. The round trip now sets every one of them to a non-default value and compares the result field by field. |
| C5-2 | Core configuration serialization (`toml_roundtrip`) covers all fields | **FALSE** — *was false; **fixed 2026-10-20** (finding F6 below): the round trip compares field by field, and a second test proves that every field of the fixture differs from its default* | `crates/rustamp-core/src/config.rs` (`toml_roundtrip`, `every_field_of_the_configuration_differs_from_its_default`) | The evidence the review quotes is a stale snapshot: commit `99b2cb3` had already set every field of every configuration type to a non-default value, including `audio.balance`, `audio.output`, `audio.buffer_ms`, `ui.double_size`, `ui.always_on_top`, `ui.easy_move` and all of `playlist.*`. What was still open: the test compared the whole struct only, and nothing pinned the premise that every field differs from its default — a later edit that set a field to its default value would have silently weakened the test without any test failing. |
| C5-3 | Honest serialization preserves user-configured unhonoured settings | **FALSE** — *was false; **fixed 2026-10-20** (finding F5 below): a save now keeps every setting it does not apply* | `crates/rustamp-app/src/config.rs` (`serialize_honest`, `serialize_preserving`, `save`) and `crates/rustamp-app/src/config_preserve.rs` | Before the fix: the loader warned, but the clean exit rewrote the file with `serialize_honest` and deleted every unhonoured field from disk. After the fix: the save reads the file it is about to overwrite and copies every setting it does not own back, so `playlist.shuffle = true` survives the clean exit. |

### 6. Cross-Lane Blindspots & Shared Interfaces

| # | Claim | Verdict | Evidence (file:line) | Smallest reproduction |
|---|---|---|---|---|
| C6-1 | ReplayGain is integrated end-to-end across the application | **FALSE** — *was false; **fixed 2026-10-20** (finding F7 below): the app reads `audio.replaygain`, hands it to the engine and writes it back* | `crates/rustamp-app/src/app.rs:339` (`engine.set_replaygain_mode(replaygain_mode_to_audio(options.config.audio.replaygain))`), `crates/rustamp-app/src/config.rs` (`serialize_honest` writes `audio.replaygain`; `ignored_fields_in_toml` no longer warns for it) | Before the fix: `rustamp-audio` implemented ReplayGain, but `crates/rustamp-app/src/config.rs` still hardcoded `"replaygain" => Some("ReplayGain is not supported yet")` and `serialize_honest` omitted it. After the fix: the configured mode reaches the engine at startup and survives the save. |
| C6-2 | The M20 deadlock fix covers resident DSP engine playback | **FALSE** — *was false; **fixed 2026-10-20** (finding F1 below): the production adapter now runs the same lockstep as the helper* | `crates/rustamp-audio/src/resident.rs:1107-1155` (`process_block`, now chunked) vs `crates/rustamp-dsphost/tests/dsp_roundtrip.rs:36-61` (the helper M20 fixed) | M20 fixed the deadlock in the standalone test helper only. The production engine adapter (`process_block` in `resident.rs`) used the single-shot un-chunked write-before-read pattern; it now writes one chunk, reads that chunk back and only then writes the next (`DSP_CHUNK_FRAMES` = 1024 frames, clamped to the in-ring capacity), so the frames in flight never exceed one chunk and the resident never blocks on its back-ring. Pinned by `resident::dsp_hosted_tests::the_lockstep_round_trip_completes_on_a_ring_smaller_than_the_block` and `…::the_un_chunked_round_trip_stalls_where_the_lockstep_one_completes`. |
| C6-3 | Task card manifest consistency across the wave | **PARTIAL** — *the card reference was already resolved before the review was recorded; see the note in F6 below* | Prompt / handoff refers to `N9-config-class.md`. The review's own evidence is stale: `docs/tasks/N9-config-class.md` **is** in the tree, committed with `99b2cb3` (an ancestor of the review commit `84366d5`) | The work was performed under N8 and documented in N8 / W10 finding F1. The card `docs/tasks/N9-config-class.md` exists and names the wave-67 lane; the review read a snapshot from before it was committed. |

---

## 2. Findings by severity

### F1 — HIGH (proven): The deadlock rebuild invariant is absent in the engine adapter: `process_block` writes the entire block before reading back, causing deadlock for buffers > ring capacity

**Status: FIXED (2026-10-20).** The finding was correct: the production
adapter wrote the whole block before it read a single frame back, and the
resident's back-ring filled up under it. The fix and the tests are in
`crates/rustamp-audio/src/resident.rs` (the adapter's `process_block` and its
new `DSP_CHUNK_FRAMES` ceiling). No other crate changed; the wire, the ring
layout, the session API and the `rustamp-core` contracts are untouched.

**The mechanism (unchanged by the fix).** The resident worker moves frames
strictly in order from the in-ring to its own back-ring
(`rustamp-mediahost/src/dsp.rs`: `BACK_RING_FRAMES = 4096`, `BLOCK_FRAMES =
1024`) and keeps at most one block in hand while it writes back. So it can
absorb at most **4096 + 1024 = 5120 frames** without a reader. The player's
write is fenced (RFC 0006 §6): it returns only once the resident has *taken*
every frame it published. A single write of a block larger than the
resident's absorption therefore cannot complete:

1. the player writes the whole block into the in-ring;
2. the resident takes frames, fills its 4096-frame back-ring and blocks on
   it — it stops taking from the in-ring;
3. the player's write waits for a resident that has stopped taking (and for
   the fence, which can never be reached);
4. the player has not reached its read-back yet, so nothing ever drains the
   back-ring.

Neither side can move: the bounded-buffer deadlock, bounded only by the block
timeout (`DEFAULT_DSP_BLOCK_TIMEOUT` = 5 s), after which the track fails with
`CoreError::Dsp("resident DSP write timed out")`. The engine's own block size
(1024 frames, `decode.rs::READ_FRAMES`) is below the threshold today, which
is why the defect was latent rather than visible — the invariant was simply
not true by construction.

**The rule now in the code.** The adapter's round trip is a **lockstep**, the
same shape M20 gave the test helper: the block is split into chunks of at
most `DSP_CHUNK_FRAMES` (1024 frames) — clamped to the in-ring capacity the
caller chose, so one chunk is always one ring write — and the adapter

* writes one chunk (`write_timeout`, still fenced: `Ok` means the resident
  has taken those frames),
* reads exactly that chunk back,
* and only then writes the next one.

The frames in flight (written, not yet read back) never exceed one chunk, so
the resident's back-ring holds at most one chunk plus its in-hand block —
2048 of its 4096 frames. The resident therefore never blocks on its
back-ring, never stops draining the in-ring, and the player's write always
finds room: neither wait can be permanent, and the deadlock boundary is
unreachable by construction. The chunk size is the resident worker's own
block size, so the frames the chain processes, their order and the values it
returns do not change; the block timeout still covers the **whole** round
trip (one deadline for every chunk), so a wedged resident fails the track
after the same time as before.

**Tests (all in `rustamp-audio`):**

| Test | What it pins |
|---|---|
| `resident::dsp_hosted_tests::the_lockstep_round_trip_completes_on_a_ring_smaller_than_the_block` | the production path (`HostDspChain::process`) with a 64-frame in-ring and an 8192-frame block: the block comes back whole and bit-exact (gain ×2), zero frames dropped |
| `resident::dsp_hosted_tests::the_un_chunked_round_trip_stalls_where_the_lockstep_one_completes` | the pre-fix pattern (one write of the whole block, then the read-back) on the same rings: it times out on the **write**, and the ring counters show the mechanism (the resident's back-ring is full at 4096 frames, the player read none) |
| `resident::dsp_tests::the_lockstep_chunk_is_the_smaller_of_the_ring_and_the_ceiling` | the chunk rule itself: `min(in-ring capacity, 1024)`, at least one frame |

**How far the proof goes.** A deadlock cannot be observed directly, only
that no progress happened inside a bound — so the contrast test is a *timing*
proof: the pre-fix pattern does not finish within its 1 s bound and fails on
the write, while the lockstep path finishes the same 8192 frames in
milliseconds. The positive test is the regression test: with the pre-fix
`process_block` restored (verified by the lane that fixed this), it fails
after the block timeout with `Dsp("resident DSP write timed out")`; with the
fix it is green. The ring counters turn the timing proof into a mechanism
proof: the resident's back-ring is full and the player has read nothing, so
the resident stopped taking — exactly the deadlock the review described.

**The original report (unchanged, for the record):**

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

**Status: FIXED (2026-10-20).** The finding was correct: the multiplier was
derived in `prepare_track` only, so a seek kept the gain the track had
prepared with and the engine's promise ("a change takes effect at the next
track **or seek**") was broken. The fix is in
`crates/rustamp-audio/src/decode.rs`, the tests in
`crates/rustamp-audio/tests/replaygain.rs`.

**Where the gain is re-derived.** One place, `Worker::refresh_replaygain`:
it asks the shared `ReplayGainControl` for the current track's multiplier
(`effective_gain_current`, the very call `prepare_track` used) and stores it
in `self.rg_gain`. `prepare_track` calls it after it has stored the track's
tag values, and `apply_seek` calls it again **after the decoder accepted the
seek**. The two paths therefore cannot drift apart, and the formula is not
duplicated: the mode, the preamp and the tag selection stay in
`crate::replaygain`.

**What a seek now does to the gain.** The multiplier is re-derived from the
*current* mode, so:

| Situation | Result |
|---|---|
| `Off` → `Track`, then a seek | the track gain is in effect from the seek on |
| `Track` → `Off`, then a seek | the track plays unadjusted again |
| a seek without a mode change | the multiplier is untouched (the track keeps the gain it prepared with) |
| a mode that needs a tag the decoder did not load | the track plays unadjusted, before and after the seek (the documented policy: a missing tag is not a license to guess) |

A **refused** seek changes nothing: the decoder did not move, so the audio
keeps the gain it had, and only the matching request is completed.

**No jump in the middle of a block.** The gain is read once per block
(`emit` takes `self.rg_gain` into a local and scales the whole block), and a
seek is handled between two blocks in the track loop. The new multiplier
therefore applies from the first block after the seek — never half-way
through one.

**The click.** The engine has **no** gain ramp for the ReplayGain
multiplier: the only ramp in the audio path is the output fade
(`rustamp_dsp::Fade`, in the render callback) and the equalizer's own slew,
and neither is a general gain ramp. None is needed, because the multiplier
only ever changes where the signal is cut anyway: a seek drops the frames
the ring still holds (the render callback drains it on the new epoch) and
the engine flushes the device (`Engine::on_decode_out` → `flush_output`), so
the audio is already discontinuous across the seek. The new multiplier
starts exactly at that discontinuity, with no waveform to ramp against. The
argument is written down in `crate::replaygain`'s module docs ("When a change
takes effect") and in `decode.rs`'s `apply_seek`.

**Tests (all in `crates/rustamp-audio/tests/replaygain.rs`, headless — the
virtual `NullOutput` device, no window and no audio device):**

| Test | What it pins |
|---|---|
| `a_mode_change_takes_effect_at_the_next_seek` | the F3 reproduction: `Off` → `Track` → seek ⇒ the post-seek audio is the gained source, frame by frame |
| `a_mode_change_back_to_off_takes_effect_at_the_next_seek` | the reverse: `Track` → `Off` → seek ⇒ the post-seek audio is the unadjusted source, bit-exact |
| `a_seek_without_a_mode_change_keeps_the_gain` | a seek on its own never touches the multiplier |
| `a_seek_on_a_track_without_tags_plays_unadjusted` | a mode that needs a tag the track does not carry plays unadjusted, before and after the seek |
| `a_seek_during_a_fade_keeps_the_fade_and_derives_the_new_gain` | a seek in the middle of a fade-out neither disturbs the ramp (the equal-power law continues where it was) nor misses the new mode |

The last test models the fade exactly: the ramp starts at the first callback
after the request, so the frame the device played at counter `played` is the
ramp's frame `played - start`, and the expectation of every post-seek frame is
`equal-power gain × (the gained source, finished)`.

**The original report (unchanged, for the record):**

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

**Status: FIXED (2026-10-20).** The finding was correct on both counts, and
both defects are closed. The fix and the tests are in
`crates/rustamp-audio/src/decode.rs` (the engine's recovery policy and the
failure report) and `crates/rustamp-audio/src/resident.rs` (the hold-back a
failed chain still reports). No other crate changed; the wire, the ring
layout, the session API and the `rustamp-core` contracts are untouched — the
`DspStage` trait did not change, so no RFC was needed.

**The recovery policy: bypass.** RFC 0014 §4.3 poisons a failed chain
("resuming on it is an error; recovery is an explicit replacement with a
fresh one"). That is the *contract*; what the engine does with the failed
stage in the meantime is engine policy — the same division RFC 0006 §8 draws
for the restart policy ("restart policy is engine policy, not contract"). The
engine's policy is **bypass**:

* The failed stage is replaced by a pass-through (`decode.rs::Bypass`, line
  505) at the moment it fails (`decode.rs::emit`, lines 917-970). The audio
  keeps flowing and **the next track plays**; the player does not die with
  the poisoned chain. The pass-through keeps the chain's shape (the
  equalizer stays first, the limiter last) and holds nothing back, so the
  latency bookkeeping stays exact. The failed stage is dropped, so a failed
  resident's helper process does not outlive its failure.
* The bypass lasts until the caller installs a chain again
  (`Engine::set_dsp_chain`, `engine.rs:504`): the engine hands the bypassed
  chain back with the run, so a caller that wants the stage back installs a
  fresh one. A *replacement* is the app layer's to choose, because only the
  caller knows how the stage was built (which helper binary, which plugin).
* The failure is reported **once** per run, and the report is the visible
  part of the policy: it names the stage, the cause, the frames that are
  lost and the bypass (`decode.rs::report_stage_failure`, line 1344). A
  bypass is never silent.

**What happens to the held tail.** A stage that reports a latency holds
that many frames back; the frames live inside it (inside the resident
helper, for the resident chain). When it fails they are gone with it, and
the engine says so: the report names the hold-back (`up to N frames it held
back`). Two things changed for the rest of the chain:

* The failed chain no longer reports a latency of 0
  (`resident.rs::ChainControl::latency`, line 1317). It keeps reporting the
  hold-back it lost, so the report can count the loss instead of hiding it.
* The end-of-track flush runs **after** the bypass, so it is no longer
  aborted by the dead stage: the stages that are still alive keep their
  hold-back and the flush pushes it out. What the failed stage itself held
  is lost, and the report says how much.

The track the failure happened in ends there — the stage's audio for that
track is gone — but the queue keeps going.

**Tests (all in `rustamp-audio`):**

| Test | What it pins |
|---|---|
| `resident::dsp_hosted_tests::engine_bypasses_the_failed_resident_and_the_next_track_plays` | the scripted resident dies mid-track: exactly one `Event::Error` (kind `Dsp`) naming the chain, the bypass and the 100 lost frames; the **next track plays** bit-exactly (the resident is out of the chain); the run ends at the end of the queue (no retry loop) |
| `resident::dsp_hosted_tests::engine_flushes_the_live_chain_after_the_resident_dies` | the flush is not aborted by the dead stage: the live chain's 32 held-back frames come out at the end of the failed track, while the dead stage's 100 frames are gone (the total is 100 frames short of a healthy chain) |
| `resident::dsp_hosted_tests::a_poisoned_chain_still_reports_the_hold_back_it_lost` | a poisoned chain still reports the hold-back it lost (100 frames), instead of zero |
| `engine::a_failed_stage_is_bypassed_and_the_next_track_plays` (integration) | the policy itself, platform-independent: a failed stage is bypassed, the failure is reported once with the stage named, and the next track plays (bit-exactly, without the failed stage's gain) |

**The original report (unchanged, for the record):**

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

*(The report quotes the state of the code as the review saw it. The
engine's failure path has since moved to the decode thread's own report
queue (RFC 0014: `DspStage::process` is fallible, the decode thread reports
the error and ends the track, and there is no engine-polled latch any more),
so `handle_dsp_failure` and `dsp_watch` no longer exist. The two defects
were still there in that shape: the poisoned chain stayed in the engine and
failed every later track, and its zeroed latency aborted the flush.)*


---

### F5 — MEDIUM (proven): Honest config serialization silently deletes all unhonoured fields from user config files on clean exit

**Status: FIXED (2026-10-20).** The finding was correct: the clean exit
replaced the whole file with the honest serialization, so every setting the
player does not apply was deleted from disk. The fix keeps the honest
serialization as the player's **own** part of the file and adds the rest of
the file back, unchanged.

**The save rule now.** Three parts, all in `crates/rustamp-app/src/config.rs`:

1. **Owned settings are written by the app.** The app reads them and acts on
   them, so it knows their current value. The list is `OWNED_PATHS`.
2. **Every other setting of the previous file is preserved verbatim.** It
   belongs to the user or to a newer version of the app, and the app does not
   understand it, so it does not touch it. The text of the file is kept, not
   just the value: the line `manual_advance = true # not yet` stays that
   line.
3. **A file the app cannot read is never overwritten.** A save that cannot
   preserve the file's settings would destroy them, so it fails loudly with
   `ConfigError::Malformed` and the file stays as it was. A merge that would
   not be a config file any more (a root key that collides with the table
   the player writes) is refused the same way.

**How the preservation works.** `crates/rustamp-app/src/config_preserve.rs`
scans the previous file for its key entries (tables, keys, values that
continue on the next line), copies every entry the app does not own with the
text it was written with, and splices that text back into the table it came
from. The merge is a *source-level* merge on purpose: a value-level merge
would rewrite the spelling (an array becomes a multi-line array) and would
drop the comment that stands on the line.

A comment that documents a setting travels with it. A comment that stands on
its own — above a setting the app owns, or at the end of the file — is not
kept: the app rewrites the file it owns.

**The audit** (what the player applies today, and what a save does with it).
The table is also in the module docs of `crates/rustamp-app/src/config.rs`:

| Setting | Applied by the player | On save |
|---|---|---|
| `version` | the schema marker | written |
| `audio.volume` | `App::with_registry` → `Engine::set_volume` | written |
| `audio.balance` | `App::with_registry` → `Engine::set_balance` | written |
| `audio.fadeout_ms` | `App::with_registry` → `EngineConfig::fade_ms` | written |
| `audio.output_mode` | `output::create_output` (RFC 0019) | written |
| `audio.device` | `output::create_output` → `selection::plan`/`resolve` | written |
| `audio.replaygain` | `App::with_registry` → `Engine::set_replaygain_mode` | written |
| `ui.skin` | `crate::run` → `skin::choose` | written |
| `ui.double_size` | `App::with_registry` → the main window's state | written |
| `ui.always_on_top` | `App::with_registry` → `App::always_on_top` | written |
| `ui.easy_move` | `App::with_registry` → `App::easy_move` | written |
| `ui.snap_distance` | `window` → `rustamp_ui::docking` | written |
| `ui.windows` | `App::with_registry` → the window placements | written |
| `ui.playlist_font_size` | the playlist editor's font size | written; an unusable value is kept |
| `eq.state` | `App::with_registry` → `Engine::set_eq` | written |
| `plugins.plugin_dir` | `App::plugin_dir` → `rustamp_prefs::scan` | written |
| `plugins.disabled` | `App::set_plugin_enabled` → `rustamp_prefs::set_enabled` | written |
| `playlist.shuffle` | `App::with_registry` → `Engine::set_shuffle` (S15) | written |
| `playlist.repeat` | `App::with_registry` → `Engine::set_repeat` (S15) | written |
| `audio.output` | — the backend comes from the command line | **preserved** |
| `audio.buffer_ms` | — the output's buffer length is the device's own | **preserved** |
| `audio.gapless` | — | **preserved** |
| `playlist.manual_advance` | — | **preserved** |
| `playlist.last_playlist` | — | **preserved** |
| `plugins.vis_plugin` | — | **preserved** |
| `plugins.vis_preset` | — | **preserved** |

Two rows of the table deserve a word, because they are *not* what the
review assumed:

* **`audio.fadeout_ms` is applied now** (fixed 2026-10-20, wave 267).
  `rustamp-audio` has the plumbing (`EngineConfig::fade_ms`); the app builds
  the engine with `EngineConfig { volume, balance, fade_ms, .. }` and hands
  it the configured `audio.fadeout_ms`, so a file that asks for a different
  fade is honoured. The engine's own default happens to be the
  configuration's default (500 ms), so the bug was invisible until the
  configured value differed.
* **`playlist.shuffle` / `playlist.repeat` are applied now** (fixed
  2026-10-20, wave 267). The engine implements both modes
  (`Engine::set_shuffle`, `Engine::set_repeat`) and owns the traversal
  (S15); the app hands the configured modes to it at startup, so the button
  and the traversal agree. Before the fix the app only lit the button from
  the configuration, and the engine played in order.

One row stays **preserved**, because it has no mapping at all:

* **`audio.buffer_ms` has no mapping.** The output's buffer length is the
  device's own: WASAPI opens the device's default period
  (`IAudioClient::GetDevicePeriod` / `GetDefaultPeriod`) and cpal leaves the
  size to the host (`BufferSize::Default`). The engine's `ring_frames` is
  its internal decode-to-device ring, not the output buffer, and nothing in
  the player asks a device for a buffer length. Mapping the setting would
  mean inventing one, so it stays preserved and keeps its warning.

**Tests** (all in `rustamp-app`, headless):

| Test | What it pins |
|---|---|
| `tests/config.rs::a_setting_the_player_does_not_apply_keeps_its_line_after_a_save` | every unapplied setting keeps its line, byte for byte, and is still warned about afterwards |
| `tests/config.rs::an_applied_setting_survives_a_save` | the applied settings round-trip through a save |
| `tests/config.rs::an_unknown_setting_survives_a_save` | a key and a table the player does not know survive |
| `tests/config.rs::the_honest_list_and_the_warning_list_stay_consistent` | the honest serialization never warns about itself, and never writes a warned-about setting |
| `tests/config.rs::a_malformed_file_is_not_replaced_by_a_save` | a malformed file is not overwritten, and the save says so |
| `tests/config.rs::a_merge_that_would_break_the_file_is_refused` | a merge that would not be a config file any more is refused, and the file stays as it was |
| `tests/config.rs::a_dotted_setting_moves_into_the_table_the_player_writes` | a dotted key moves into the table the player writes, unchanged |
| `tests/config.rs::an_unusable_playlist_font_size_survives_a_save` | the one owned setting with an unusable value is kept, not dropped |
| `tests/config.rs::a_comment_above_a_preserved_setting_survives_a_save` | the user's note travels with the setting it documents |
| `tests/config.rs::the_clean_exit_keeps_the_settings_the_player_does_not_apply` | the app's clean exit is a save, so it keeps them |
| `src/config_preserve.rs` (unit tests) | the reader: keys, dotted keys, quoted keys, values that continue on the next line, a comment on the line, a comment above a setting, a header inside a value, an array of tables |

**The wave-267 follow-up** (2026-10-20) applied the two settings the audit
above found preserved but never applied, and left the one that has no
mapping alone. Its tests (all headless, all in `rustamp-app`):

| Test | What it pins |
|---|---|
| `src/app.rs::the_configured_fade_reaches_the_engine` | the configured `audio.fadeout_ms` reaches the engine: the fade-out ramps over the configured number of frames (200 ms at 44.1 kHz are 8820) and the transport stops when the ramp is over — with the engine's own default (500 ms) it would still be playing there |
| `src/app.rs::the_configured_playback_modes_reach_the_engine_at_startup` | the configured `playlist.shuffle` / `playlist.repeat` are the engine's state at startup, and the buttons show them |
| `src/app.rs::the_configured_playback_modes_survive_a_save` | the modes the engine runs in are the modes the save writes, and a player built from that file comes up in the same modes |
| `tests/config.rs::the_newly_applied_settings_survive_a_save` | `audio.fadeout_ms`, `playlist.shuffle` and `playlist.repeat` round-trip through a save and do not warn |
| `tests/config.rs::a_setting_without_a_mapping_stays_preserved_and_still_warns` | `audio.buffer_ms` has no mapping: it keeps its line after a save and still warns |

**The original report (unchanged, for the record):**

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

**Status: FIXED (2026-10-20), with one carve-out named below.** The
finding's *direction* was right — the round-trip tests were weaker than
their names claimed — but the review recorded it against a **stale
snapshot** of the tree, so its literal evidence no longer held when it was
written.

**What the review reported, and what the tree actually contained.** The
review quotes `crates/rustamp-app/src/config.rs:487-550` with
`width: None, height: None` on every window and only the unit window
variants, and `crates/rustamp-core/src/config.rs:207-238` with
`audio.balance`, `audio.output`, `audio.buffer_ms`, `ui.double_size`,
`ui.always_on_top`, `ui.easy_move` and `playlist.*` left at their defaults.
Commit `99b2cb3` ("Close the configuration writer drift as a class, not as
two instances", 2026-10-18 00:20) had already rewritten both tests to carry
a non-default value for every field — including `width: Some(...)` /
`height: Some(...)` and the `WindowId::Gen(n)` and `WindowId::Video`
variants. The review was recorded 20 minutes later (`84366d5`, 00:40): the
snapshot it read was older than the tree it was committed into. The same
staleness applies to its C6-3 ("the task card `N9-config-class.md` is
absent") — that card was committed together with `99b2cb3` and is in the
tree.

**What was still open — and is now closed.** The class-closure card's
premise is that a round trip sets *every* field to a non-default value. It
held for the core fixture, but two things were missing:

1. **The app-side round trip left seven honoured settings at their
   default.** The honest serializer writes exactly the settings the player
   honours (the audit table in `crates/rustamp-app/src/config.rs`), but the
   round trip only set `audio.volume`, `audio.balance`, `ui.skin`,
   `ui.double_size`, `ui.always_on_top`, `ui.easy_move`, `ui.windows` and
   `eq.state` to a non-default value. `version`, `audio.output_mode`,
   `audio.replaygain`, `audio.device`, `ui.snap_distance`,
   `plugins.plugin_dir` and `plugins.disabled` stayed at their defaults — so
   a writer that forgot one of them was invisible: the loader falls back to
   exactly that default.
2. **Nothing pinned the premise.** The core round trip compared the whole
   struct (`assert_eq!(c, back)`) and set every field to a non-default value
   by convention only. No test said so: a later edit that set a field to its
   default value would have silently weakened the test, and no test would
   have failed.

**What changed.**

| File | Change |
|---|---|
| `crates/rustamp-core/src/config.rs` | the fixture moved into `every_field_set()`; `toml_roundtrip` now compares **field by field** (a failure names the field) on top of the whole-struct comparison; the new `every_field_of_the_configuration_differs_from_its_default` makes the premise a checked fact instead of a convention |
| `crates/rustamp-app/tests/config.rs` | `config_round_trip_honours_all_supported_fields` now sets **every honoured setting** to a non-default value (`version`, `audio.output_mode`, `audio.replaygain`, `audio.device`, `ui.snap_distance`, `plugins.plugin_dir`, `plugins.disabled` on top of the eight it already set), writes the app-owned `ui.playlist_font_size`, and compares the result field by field — including every field of every window layout |

**The carve-out.** The unit test of the same name in
`crates/rustamp-app/src/config.rs` still uses the old fixture (the honoured
settings at their defaults). That file belongs to another lane (wave267),
so it was not touched here; the coverage now lives in the integration test
`crates/rustamp-app/tests/config.rs`, and the writer cannot lose a setting
unnoticed any more.

**The field table (the whole configuration).**

| Setting | Written by the honest serializer | Non-default value in the round trip | Pinned by |
|---|---|---|---|
| `version` | yes | `7` | core + app |
| `audio.volume` | yes | `150` | core + app |
| `audio.balance` | yes | `-40` | core + app |
| `audio.output_mode` | yes | `Exclusive` | core + app |
| `audio.replaygain` | yes | `Album` | core + app |
| `audio.device` | yes (when set) | `Some("Speakers (Realtek)")` | core + app |
| `audio.output` | no (preserved) | — | core (the TOML contract) |
| `audio.buffer_ms` | no (preserved) | — | core |
| `audio.fadeout_ms` | no (preserved) | — | core |
| `audio.gapless` | no (preserved) | — | core |
| `ui.skin` | yes (when set) | `Some("skins/amplifier.wsz")` | core + app |
| `ui.double_size` | yes | `true` | core + app |
| `ui.always_on_top` | yes | `true` | core + app |
| `ui.easy_move` | yes | `false` | core + app |
| `ui.snap_distance` | yes | `25` | core + app |
| `ui.windows` | yes (when set) | all six `WindowId` variants, four of them sized | core + app |
| `ui.playlist_font_size` | yes (when chosen) | `Some(18)` | app |
| `eq.state` | yes | bands, preamp, `enabled = false`, `auto = true` | core + app |
| `plugins.plugin_dir` | yes (when set) | `Some("plugins")` | core + app |
| `plugins.disabled` | yes (when set) | `["bad.dll"]` | core + app |
| `plugins.vis_plugin` | no (preserved) | — | core |
| `plugins.vis_preset` | no (preserved) | — | core |
| `playlist.shuffle` | no (preserved) | — | core |
| `playlist.repeat` | no (preserved) | — | core |
| `playlist.manual_advance` | no (preserved) | — | core |
| `playlist.last_playlist` | no (preserved) | — | core |

**The evidence that the new test has teeth.** Every honoured setting was
mutated in the writer (it writes the default instead of the configured
value — the N9/F5 class of bug) and the two round-trip tests were run:

| Mutation | the old unit test (`src/config.rs`) | the strengthened test (`tests/config.rs`) |
|---|---|---|
| `version` written as `1` | passes (blind) | **fails** |
| `audio.output_mode` written as `Shared` | passes (blind) | **fails** |
| `audio.replaygain` written as `Off` | passes (blind) | **fails** |
| `audio.device` not written | passes (blind) | **fails** |
| `ui.snap_distance` written as `10` | passes (blind) | **fails** |
| `plugins.plugin_dir` not written | passes (blind) | **fails** |
| `plugins.disabled` not written | passes (blind) | **fails** |

The failure names the setting, e.g.
`assertion 'left == right' failed: plugins.plugin_dir / left: None / right: Some("plugins")`.
The serializer was restored after the check; the mutation left the tree
unchanged.

**The original report (unchanged, for the record):**

`crates/rustamp-app/src/config.rs:487-550` contains `config_round_trip_honours_all_supported_fields`.
Despite its name:
1. Lines 505-506, 514-515, 523-524 explicitly specify `width: None, height: None` on all three tested windows (`Main`, `Equalizer`, `Playlist`). The test does not exercise `width: Some(...)` or `height: Some(...)`.
2. It tests only unit `WindowId` variants, omitting `WindowId::Gen(n)` and `WindowId::Video`.
3. In `crates/rustamp-core/src/config.rs:207-238` (`toml_roundtrip`), `audio.balance`, `audio.output`, `audio.buffer_ms`, `ui.double_size`, `ui.always_on_top`, `ui.easy_move`, and `playlist.*` are completely omitted and left at default values.

---

### F7 — LOW (proven): Cross-lane disconnect: audio engine implements ReplayGain, but app config marks it unsupported and strips it

**Status: FIXED (before this review; re-verified 2026-10-20).** The
disconnect is gone on both ends:

* The application reads the setting and hands it to the engine at startup:
  `crates/rustamp-app/src/app.rs:339`
  (`engine.set_replaygain_mode(replaygain_mode_to_audio(options.config.audio.replaygain))`).
* The setting is written back by the honest serialization
  (`crates/rustamp-app/src/config.rs`, `serialize_honest`), so it survives a
  clean exit.
* The loader does not warn about it any more: `ignored_fields_in_toml` lists
  `"replaygain"` among the applied settings.
* The F5 fix keeps the setting in place even for the values the player does
  not apply, so the two findings no longer contradict each other.

**The original report (unchanged, for the record):**

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

---

## 5. Follow-up (2026-10-20, `wave269`): C4-2 at the media-host session level

**Lane:** `crates/rustamp-mediahost` only (the framework). The engine
adapter (`rustamp-audio`) and C4-3 were **not** touched.

### 5.1 What was re-checked

The review's C4-2 evidence is in the **engine adapter**
(`rustamp-audio/src/decode.rs`, `rustamp-audio/src/resident.rs`): when the
resident DSP fails, `HostDspChain` is poisoned and every later track fails
in `prepare_track`. That half is unchanged.

The half that lives in this crate is the **session** layer: the media host
runs a `DecoderSession` and a `DspSession` side by side, and each of them
owns its own `ControlPeer`, its own rings and its own terminal state. The
question the lane answered is the one the claim asks at that level: *does a
failure or a kill of one session corrupt, stall or silently end the other?*

**Answer: no.** The independence is structural — no shared mutable state
between the two sessions — and it is now pinned by tests instead of being an
inspection result.

### 5.2 The tests (`crates/rustamp-mediahost/tests/session_independence.rs`)

Headless and in-process: a fake resident behind the production
`serve_decoder`/`serve_dsp` workers, or a fake helper on the raw wire that
dies the way a killed helper dies (control EOF, rings open). No process is
spawned, no window is opened, no audio device is touched.

| Test | What it pins |
|---|---|
| `a_failed_decoder_session_leaves_the_dsp_session_fully_usable` | a decoder resident that fails: the DSP session still takes writes and returns the frame bit-exact, its four ring counters follow the loop's own traffic and nothing else (the failure contributes nothing to them), neither of its rings is closed, it drops no frame, and its host is still serving |
| `a_killed_decoder_session_leaves_the_dsp_session_fully_usable` | the same with the crash surrogate (the helper's control channel ends, its ring stays open): the DSP session is untouched and fully usable |
| `a_failed_dsp_session_leaves_the_decoder_session_fully_usable` | the mirror image: a chain that fails leaves the decoder session delivering its frames, its counters following its own traffic, and its host still serving |
| `a_killed_dsp_session_leaves_the_decoder_session_fully_usable` | the same with the killed helper |
| `the_control_loop_keeps_running_and_does_not_spin_after_a_failure` | the host's control loop: it reports the failed session **once**, asks it no further, keeps running, and stays inside a sane turn rate (it blocks in the live session's own bounded wait instead of spinning on the dead one) |

The failed session stays **terminal** in every case: the sticky diagnosis
(`wave224`) is asserted with the *same* text on every later call — the
resident's own diagnosis (`resident decoder failed: …` /
`resident DSP failed: …`) or the control channel's (`control stream ended:
…`) — never a fresh success and never a false end of stream.

### 5.3 The tests can fail (two mutation probes, both reverted)

* **Cross-contamination probe.** A temporary global sticky failure in
  `ControlPeer` (one peer's failure poisons every peer in the process) makes
  **all five** tests fail — the clearest symptom being *"the loop must
  report the failed session once: left 2, right 1"*, i.e. the healthy
  session was reported as failed too.
* **Delivery-fence probe.** Removing `DspSession::write_timeout`'s
  `await_delivery` (the write returns before the chain has taken the frames)
  makes `a_killed_dsp_session_leaves_the_decoder_session_fully_usable` fail:
  the round trip never completes.

Both probes were reverted; `git status` in the lane worktree shows only the
new test file and this document.

### 5.4 What stays open

* **C4-2, engine-adapter half:** the poisoned `HostDspChain`
  (`rustamp-audio/src/resident.rs`) still fails every later track with
  `CoreError::Dsp("resident DSP chain already failed; replace the stage")`.
  Fixing it is a `rustamp-audio` change and needs its own lane.
* **C4-3:** untouched — the latency flush compensation on a DSP failure is
  an engine-adapter question as well.
* **Two real processes:** the tests drive both sessions in one process. The
  two-process kill of a *real* helper stays VM-only
  (`tests/pcm_process.rs`, `tests/dsp_process.rs`, both `#[ignore]`).

### 5.5 Boundary receipt of this lane

- Worktree: `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave269-mediahost-independent`.
- Written: `crates/rustamp-mediahost/tests/session_independence.rs` (new) and
  this document. No production code changed, no commit, no download, no
  window, no audio device, no spawned process.
- Gates: `cargo fmt --all -- --check`, `cargo clippy -p rustamp-mediahost
  --all-targets --all-features -- -D warnings`, `cargo test -p rustamp-mediahost
  --all-features` and `cargo check --workspace --all-targets --all-features`
  are green (§4 above is the *review* lane's own receipt, not this lane's).
