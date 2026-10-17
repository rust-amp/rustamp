# A15 — ReplayGain on the audio path

**Date** 2026-10-17. **Lane** `rustamp-audio`.
**Worktree** `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave60-replaygain-audio`.
Write only your own files inside this worktree, and nothing under `D:\code\Rustamp`.

**Status:** implemented; host compilation/linting only, VM test execution pending.

Nothing was executed or tested on the host (Hyper-V test-VM rule). Only
`cargo fmt --all`, `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets --all-features -- -D warnings` and
`cargo build --workspace` were used. No window opened, no audio device
opened, no audio played, no download, no dependency addition, no commit or
other git write.

## Scope and sources

The metadata half (E4) reads and writes the tags losslessly and analyses
loudness as a pure function; nothing applied the gain. This card wires the
audio side: read the tags for the playing track, turn the gain into a linear
multiplier, combine it with volume/balance, and decide the policy questions.

Owned changes: `crates/rustamp-audio/src/replaygain.rs` (new),
`crates/rustamp-audio/src/{engine,decode,lib}.rs` (docs + wiring),
`crates/rustamp-audio/tests/replaygain.rs` (new),
`crates/rustamp-audio/tests/common/mod.rs` (`rg_` test tracks), this card.
No other file was touched. In particular `rustamp-meta`, `rustamp-core`,
`rustamp-app`, `rustamp-ui`, `rustamp-dsp`, `rustamp-output`,
`rustamp-mediahost` and `xtask` are unchanged.

Sources: the E4 card (the metadata half and its §5 list of what the audio
side still has to do), the A14 card (chain order, the limiter's place, the
converter's final full-scale bound), the A6 DSP-chain review, the A11
ring/limiter-default card, the A13 flush contract, `rustamp-meta`'s reader
(for the key spelling and gain-string semantics only — no code was copied
and no dependency was added), and the code in the owned crate. No Winamp
source, SDK material, disassembly, research tree or reference binary was
opened. No third-party code was copied.

## 1. Where the gain is applied, and why

The per-track multiplier is applied on the **decode thread**, to the raw
decoder output **before the first DSP stage** (the equalizer):

```text
decoder -> ReplayGain scale -> Equalizer -> [installed stages] -> Limiter
        -> FormatConverter -> ring -> volume/balance (render callback) -> device
```

It is not a stage, not the volume path, and not a per-run render gain — it
is a per-track constant scale at the decoder output, chosen for three
reasons:

1. **It must not fight the limiter.** The limiter sits last in the DSP
   chain and must see everything earlier stages produced. A hot correction
   (quiet track, positive gain) scaled before the chain is limited at the
   −0.3 dBFS ceiling like any other hot signal; scaled after the limiter it
   would bypass the limiter's whole look-ahead.
2. **It must not fight the converter's full-scale bound.** A14 puts the
   final `±1.0` authority on the resampled path because the resampler is the
   last stage that can raise the level. A boost applied in the render
   callback, next to volume/balance, would sit after that authority with no
   stage left to catch it. Volume and balance only ever attenuate
   (`gain.rs`), which is why the A14 invariant ("nothing downstream can
   raise the level") holds for them and would not hold for a ReplayGain
   boost there.
3. **It commutes with the equalizer, so first is the honest place.** Both
   the correction and the EQ preamp/bands are linear scales (the limiter is
   the only non-linear stage, and it comes later either way), so the order
   against the EQ changes no number; first expresses that the correction
   normalizes the source.

Consequences that fall out of the placement, not extra machinery: a gain
of exactly `1.0` (missing tag, 0 dB tag, disabled mode) is a bit-exact
no-op down to the last bit; the vis tap sees the normalized, equalized
signal; seeks keep the track's gain (the pipeline state is re-opened, not
the track); a mid-stream format change keeps it (same track); gapless and
latency accounting are untouched (`run_block` is inside the existing
trim/flush accounting, the scale adds no delay and holds nothing back).

The `PlayerState` wire contract is unchanged (as with the limiter switch,
a changed contract would require an RFC): the settings live engine-side.

One deliberate deviation from the E4 suggestion: E4 names
`ReplayGainTags::from_metadata` as the surface to consume. This crate does
not depend on `rustamp-meta` (adding it would be a new dependency, which
this lane forbids itself), so the two consumed key spellings and the gain
parsing are mirrored in `replaygain.rs` with identical semantics (same key,
same optional `dB` suffix, same finite-number rule). If the metadata
encoding ever changes, this mirror must change with it; the unit tests pin
the parsing. The R128 Q8 keys and the peak keys are not consumed: the
analysis behind the Q8 values is the same dB number, and peaks are
deliberately unused (see §2).

## 2. Policy questions and answers (own choices, marked)

| Question | Answer | Whose choice |
|---|---|---|
| Track vs. album gain, and where does the choice come from? | A user setting: `Engine::set_replaygain_mode` with `ReplayGainMode::{Disabled, Track, Album}`. `Track` prefers the track gain and falls back to the album gain when the track has none; `Album` is the mirror. The fallback follows E4's `track.or(album)` suggestion, symmetrized. | Own choice (names, shape, fallback, and the default below). No reference-player behaviour is claimed. |
| Default mode? | `Disabled`. The player never changes loudness until the user asks; flipping existing users' levels on upgrade would be the surprising behaviour. | Own product choice. |
| Preamp? | A second user setting, `Engine::set_replaygain_preamp_db`, default `0.0` dB, clamped to ±24 dB (non-finite keeps the previous value, like the EQ clamp). Added on top of the selected gain in dB before the linear conversion. | Own choice (name, default, ±24 dB range). |
| Peak above full scale — pre-attenuate? | **No.** Peak tags are untrusted estimates, and attenuating by `peak × gain` would double-correct material the limiter already shapes. A correction whose peak times gain exceeds full scale is handled downstream: the limiter holds it at its ceiling, the converter clamps it at full scale (pinned by the two hot-signal tests). | Own choice. |
| Missing (or malformed) tag? | Play unadjusted: gain `1.0`, bit-exact, even with a preamp dialed in. A missing value is not a license to guess; malformed values count as absent, as in `rustamp-meta`. | The honest default from E4's silence policy; the preamp-ignored half is an own choice. |

Not carried: R128 Q8 tags (same dB number by another encoding), the
reference-loudness key (the tag layer cannot carry it, per E4), and any
per-album grouping logic (the mode is a per-track source selection; the
playlist/album model is another lane's business).

## 3. What changed (files)

* `crates/rustamp-audio/src/replaygain.rs` (**new**, ~470 lines with
  tests): [`ReplayGainMode`], [`ReplayGainControl`] (two atomics,
  lock-free, snapshotted once per track), key mirrors, `parse_gain_db`,
  `select_gain_db` (with cross-fallback), `gain_db_to_linear` (±51 dB
  clamp mirroring the estimator, non-finite → 1.0), `effective_gain`, and
  `apply_gain` (one multiply per sample; `1.0` is a bit-exact no-op;
  non-finite is refused). 9 unit tests.
* `crates/rustamp-audio/src/decode.rs` (docs + wiring): `Config` gains
  `replay_gain: Arc<ReplayGainControl>`; `Worker` gains `rg_gain: f32`;
  `prepare_track` snapshots it from the decoder's metadata `extra`;
  `emit` scales the raw block before the DSP chain.
* `crates/rustamp-audio/src/engine.rs` (docs + wiring): owns the
  `Arc<ReplayGainControl>`; new accessors `replaygain_mode`,
  `set_replaygain_mode`, `replaygain_preamp_db`,
  `set_replaygain_preamp_db`, `replaygain_control`; a `# ReplayGain` module
  section; the stale "ReplayGain" entry removed from "Not in this
  milestone". No `Command` variants (the command enum is a frozen core
  contract); the app layer calls the setters directly, as for the limiter.
* `crates/rustamp-audio/src/lib.rs`: module + re-exports.
* `crates/rustamp-audio/tests/common/mod.rs`: `rg_` test tracks
  (`rg_<track>_<album>_<seed>_<frames>`, `none` = untagged) whose
  `PatternDecoder` reports `REPLAYGAIN_*_GAIN` in `extra` formatted exactly
  the way `rustamp-meta` writes them. Untagged names are byte-identical to
  before.
* `crates/rustamp-audio/tests/replaygain.rs` (**new**): 7 integration
  tests (see §4).

## 4. Tests and what needs the VM or hardware

Unit (in `replaygain.rs`, 9): gain-string parsing mirroring the metadata
crate; track/album preference and both fallbacks; malformed counts as
absent; dB→linear factors (−6.02 ≈ 0.5, +6.02 ≈ 2.0, ±51 dB clamp,
non-finite → 1.0); preamp shifts a selected gain and nothing else;
unity/non-finite leave the buffer bit-exact; a known gain moves every
sample by exactly that factor (exact `==`); control round-trip including
preamp clamp and non-finite-ignored.

Integration (in `tests/replaygain.rs`, 7, all on the virtual null device):

| Test | Pin |
|---|---|
| `track_gain_scales_samples_by_the_documented_factor` | −6.02 dB moves all 1024 left samples by exactly `10^(−6.02/20)` (±1e-7), after asserting the factor halves (≈ 0.5). |
| `missing_tags_play_unadjusted` | Untagged track, Track mode, +6 dB preamp: bit-exact `expected(1, p0, 1024)`. |
| `volume_balance_and_replaygain_combine_in_documented_order` | Left = source × RG × (128/255) (±1e-7); right = 0.0 under full-left balance. |
| `album_mode_uses_album_gain_and_falls_back_to_track` | Track mode reads −2 dB, Album mode reads −5 dB from one doubly-tagged track; Album mode with only a track tag reads −6.02 dB. |
| `hot_replaygain_is_held_at_the_limiter_ceiling` | +6.02 dB (peak × gain = 2.2) held at the limiter ceiling with the limiter on; control run with the limiter off reproduces the doubled pattern exactly, proving the ceiling came from the limiter, not from a quiet source or hidden pre-attenuation. |
| `hot_replaygain_is_held_at_full_scale_after_resampling` | Chain level, limiter off: +12 dB on a 0.5 sine (pre peak > 1.5) through 44.1 → 48 kHz holds post peak at exactly 1.0. |
| `replaygain_is_disabled_by_default` + `a_resampled_device_makes_no_difference_to_the_documented_order` | Defaults (Disabled, 0.0 dB); a 48 kHz device plays the corrected track end to end with the peak bounded (≤ 1.0, near the limiter ceiling for a cut). |

None was run on this host. VM (orchestrator) must confirm: the suite stays
green (in particular that the exact-`==` and ±1e-7 assertions hold on the
VM's float path, and the `exactly 1.0` clamp assertions); the
`rg_-6.02_none_1_44100`-style filenames are legal on the VM checkout; and
the resampled-device test's device grant is honoured by `NullOutput` there
as here. Hardware (loopback or listening, future lane) must confirm what
this lane cannot: whether hot ReplayGain corrections limited at −0.3 dBFS
and flat-topped at 0 dBFS sound different from a pre-attenuating design,
and where a true-peak limiter would belong.

## 5. Verification and hygiene

* `cargo fmt --all -- --check`: **PASS** (exit 0, after one `cargo fmt --all`).
* `cargo clippy --workspace --all-targets --all-features -- -D warnings`: **PASS** (exit 0, no warnings).
* `cargo build --workspace`: **PASS** (exit 0).
* `cargo test`: **NOT RUN** (Hyper-V test-VM rule respected; no test executed on this host).
* No window opened, no audio device opened, no audio played, no download, no dependency addition (in particular `rustamp-meta` was not added as a dependency; `Cargo.toml`/`Cargo.lock` untouched), no commit or other git write.
* Every command ran in the named worktree. Final `git status --short` is recorded in the lane report; it contains only owned paths.
