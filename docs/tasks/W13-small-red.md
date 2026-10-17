# Task card W13 — the two small red tests (decoders, meta)

Two suites that had never been executed each reported one failure in the VM:
`rustamp-decoders` (lib target: 24 passed, 1 failed of 25) and `rustamp-meta`
(`tests/tags.rs`: 28 passed, 1 failed of 29). Both are new work
([B4](B4-second-decoder.md), [E4](E4-replaygain.md)) written on the host and
first run in the guest. This card records what each failure really was.

**Lane** — cleanup; owned paths `crates/rustamp-decoders/**`,
`crates/rustamp-meta/**` and this card. No other crate was touched.

## 1. `a_stream_of_unknown_length_reads_to_the_end_without_a_duration`

`crates/rustamp-decoders/src/wave_dec.rs`.

**Verdict: the expectation (the test's own byte patch), not the decoder.**

The test wants a source that cannot report its length plus the streaming
`data` marker `0xFFFFFFFF`, so the container has no length. It built a
**stereo** fixture (`pcm16`) and then wrote the marker at `bytes.len() - 8`.
For the two mono tests that idiom happens to be correct, because a two-sample
mono payload is exactly four bytes; here the payload is eight bytes, so the
patch landed on the first four **audio** bytes and the size field still said
`8`. The decoder therefore did the honest thing with the file it was given:

```text
file  52 bytes: RIFF 12 | fmt 24 | "data"+size at 36..44 | payload 44..52
patch at 52-8 = 44  ->  payload, not the size field
data size stays 8   ->  data.len = Some(8) -> total_frames = 8/4 = 2
duration() = Some(duration_of(2)), the test expected None
```

The decoder is right per `CONTRACT_NOTES.md` ("the streaming marker
`0xFFFFFFFF` means to EOF; in a source that cannot report its length the
duration is then `None`"). The file really did declare eight bytes.

**Fix:** a `patch_data_size` helper in the test module locates the `data` chunk
id after the 12-byte RIFF header and writes the size at `id + 4`. All three
tests that patch a size use it. The claims are unchanged (a 4 GiB claim and a
1000-byte claim are still clamped to the file; the unknown-length case still
asserts `duration() == None` and four samples), and the intended streaming
path is now actually exercised for the first time.

## 2. `r128_keys_roundtrip_on_vorbis_and_are_refused_loudly_elsewhere`

`crates/rustamp-meta/tests/tags.rs` → `crates/rustamp-meta/src/replaygain.rs`.

**Verdict: the code.**

The test requires the refused keys in `REPLAYGAIN_KEYS` order —
`["R128_TRACK_GAIN", "R128_ALBUM_GAIN"]` — and `ReplayGainWriteReport::written`
is documented as being "in `REPLAYGAIN_KEYS` order". `write_replaygain` built
both lists by iterating `TrackMetadata::extra`, which is a `BTreeMap`, so the
order was alphabetical, i.e. `R128_ALBUM_GAIN` before `R128_TRACK_GAIN`. The
expectation is not a guess: it is the order the type documents and the order the
card's key table uses. The failure was a real contract miss, and a
`BTreeMap`-shaped accident that would have changed if a key were renamed.

**Fix:** `write_replaygain` sorts both lists by their index in
`REPLAYGAIN_KEYS` (`key_rank`, unknown names last, stable). The
`skipped_unsupported` doc now states the order, so the two lists describe one
request instead of two map iterations.

## 3. Do the two WAV implementations disagree?

No finding. Nothing in either failure is a disagreement between
`WaveDecoderFactory` and `SymphoniaDecoderFactory`: the wave fixture in the
failing test was never decoded by symphonia, and the tie/selection tests were
not implicated. The only shared surface the failures touch is the format's own
`data`-size field, and the decoder's reading of it (clamp to the source, end at
EOF for the streaming marker) matches what the tests claim.

## 4. Verification (host, compile-only)

- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets --all-features --offline -- -D warnings`: clean.
- `cargo build --workspace --offline`: clean.
- **No test was run on the host** (the suite runs in the Hyper-V VM and only
  the orchestrator runs it); no window opened, no audio played, no download,
  no dependency added, nothing committed.
- Both fixes are reasoned-only until the VM run: the decoder one from the
  fixture's byte layout, the meta one from `BTreeMap` ordering and lofty
  0.25.4's `VORBIS_MAP` / `ID3V2_MAP` (which carry the R128 keys only for
  Vorbis comments).

## 5. Provenance

- Sources: the public RIFF/WAVE layout, RFC 0013, RFC 0002 and the E4 card's
  key table; `lofty` 0.25.4's key maps read from the vendored source.
- No Winamp source, SDK material or disassembly was viewed; no third-party
  code was copied.
