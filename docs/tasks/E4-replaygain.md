# Task card E4 — ReplayGain tags and analysis (metadata half)

**Scope**: `crates/rustamp-meta/**`, `docs/tasks/E4-replaygain.md`.

## 1. Goal

ReplayGain values must survive a read-write round trip losslessly, the
loudness analysis must be a pure function testable without audio hardware,
and a format that cannot carry a value must say so instead of dropping it
silently. Applying the gain to audio belongs to another lane
(`rustamp-audio`) and is explicitly not done here.

## 2. Keys and formats

Carried keys (`replaygain::REPLAYGAIN_KEYS`):

| Key | Tag value | Units |
|---|---|---|
| `REPLAYGAIN_TRACK_GAIN` | e.g. `"-3.50 dB"` | dB, 2 decimals |
| `REPLAYGAIN_TRACK_PEAK` | e.g. `"0.987654"` | linear, 6 decimals |
| `REPLAYGAIN_ALBUM_GAIN` | e.g. `"-5.10 dB"` | dB |
| `REPLAYGAIN_ALBUM_PEAK` | linear | linear |
| `R128_TRACK_GAIN` | e.g. `"-1152"` | Q8 (256 steps per dB) |
| `R128_ALBUM_GAIN` | Q8 integer | Q8 |

Per-format honest answers (from `lofty` 0.25.4's key tables, verified by
reading the registry source, not by guessing):

| Format (primary tag) | 4 classic keys | R128 keys | `REPLAYGAIN_REFERENCE_LOUDNESS` |
|---|---|---|---|
| MP3/AAC/AIFF/WAV (ID3v2 `TXXX`) | read + write | refused, reported | not carried anywhere (no `ItemKey`; unmapped keys are discarded) |
| FLAC/Ogg/Opus/Speex (Vorbis) | read + write | read + write | same |
| M4A/MP4 (ilst freeform `----:com.apple.iTunes:replaygain_*`) | read + write (fixed by this card; the old short-name lookup found nothing in the ilst map and dropped them) | refused, reported | same |
| APE/MPC/WavPack (APEv2) | read + write | refused, reported | same |
| RIFF INFO / AIFF text / ID3v1 | never carry these keys; never the write target (no file format uses them as its primary tag) | — | — |

`REPLAYGAIN_REFERENCE_LOUDNESS` is exported as
`replaygain::REFERENCE_LOUDNESS_KEY` so callers can name it consistently;
every write entry point lists it as skipped.

## 3. What was built

- `crates/rustamp-meta/src/replaygain.rs` (new): key constants,
  `ReplayGainTags` (`from_metadata` / `apply_to_metadata`, never invents a
  value, never clears), gain/peak parse + format, R128 Q8 conversions,
  `analyze_replaygain`, `write_replaygain` + `ReplayGainWriteReport`,
  `replaygain_supported_for_extension` / `r128_supported_for_extension` /
  `replaygain_supported_for_tag_type`.
- `src/read.rs`: `R128_*_GAIN` added to the `extra` copy list, so they ride
  the normal read path on Vorbis.
- `src/write.rs`: `extra` is now matched through a canonical name→`ItemKey`
  table first (this is the MP4 fix) with the format's own key table as
  fallback; `write_tags_report` returns `WriteReport { unwritten_extra }`;
  `write_tags` keeps its signature and delegates (existing callers
  unchanged).
- `src/lib.rs`: re-exports; `replaygain` is a public module.
- Tests: 12 unit tests in `replaygain.rs` (pure functions only, no files);
  5 new integration tests in `tests/tags.rs`; the old
  `write_read_roundtrip_on_each_family` now asserts the ReplayGain key on all
  five families including MP4.

## 4. The analysis, plainly

`analyze_replaygain(samples, channels, sample_rate_hz)` is pure: no I/O, no
globals, no audio hardware. Convention: the original ReplayGain 1.0 proposal
(David Robinson, 2001-07-31, replaygain.org; Hydrogenaudio "ReplayGain 1.0
specification" summary) — 89 dB SPL pink-noise reference, anchored digitally
so a full-scale sine is 0.00 dB; gain = `20·log10(REFERENCE_RMS / rms)`.

Test signals (all unit tests, no audio files): digital silence → 0.0 dB /
0.0 peak; full-scale 440 Hz sine → ≈0 dB / peak 1.0; half-scale (−6 dB) sine
→ ≈+6.02 dB; one full-scale sample in a second of silence → full peak 1.0
with ≈+43.4 dB gain (loudness and peak disagree by construction);
full-scale square vs. full-scale sine → same peak, ≈3.01 dB apart; 1e-6 and
1000.0 constants → clamped to +51 / −51 dB; channels 0, rate 0, empty input,
ragged frames, NaN/Inf samples → all refused with `InvalidArgument`.

What it is not: not EBU R128/BS.1770 integrated loudness (no K-weighting, no
gating, no channel weights — the rate argument is validated but does not
change the number), not perceptual (no equal-loudness contour), and not a
substitute for the reference implementations' exact numbers
(mp3gain/metaflac/foobar2000/loudgain will differ).

Own choices (all documented in code): silence → neutral 0 dB; clamp ±51 dB;
reference anchor at full-scale sine; R128 `db_to_r128_q8` saturates via `as`;
no clearing of tag fields; case-insensitive extra-name lookup.

## 5. What the audio side still has to do

`rustamp-audio` (another lane owns it) must: read `ReplayGainTags` (or the
`extra` entries) for the playing track, convert the track/album gain dB to a
linear multiplier, combine it with the existing volume/balance gain in the
engine's gain stage, decide track-vs-album policy plus preamp/headroom and
clip protection for hot signals, and handle missing tags (silence policy:
play unadjusted). Suggested surface to consume: `ReplayGainTags::from_metadata`
followed by `track_gain_db.or(album_gain_db)`. The limiter work (A6 wave) is
the natural neighbor for the clip-protection half.

## 6. Verification

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`: clean.
- `cargo build --workspace` (and `--offline`): clean.
- `cargo test` was **not** run on the host: the suite runs in the Hyper-V
  test VM and only the orchestrator runs it. All new tests are therefore
  unverified claims until the VM run; the risky one is named explicitly:
  the MP4 freeform round trip (write path verified by reading `lofty`'s
  `merge_tag`/`write_freeform`/`TryFrom<&ItemKey>` code, not by execution).
- No window opened, no audio played, no commit made, no dependency added,
  no file written outside `crates/rustamp-meta/**` and this card.
