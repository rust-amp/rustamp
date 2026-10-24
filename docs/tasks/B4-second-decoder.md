# Task card B4 — a second real decoder factory (lane B)

**Goal**
Give the decoder registry a real second factory, so RFC 0013's selection rule
(content decides, an equal-confidence tie goes to the earlier registration, no
claim at all is a refusal) is reachable in `default_registry` with two real
implementations instead of only the fake claimants in `tests/robustness.rs`.
This closes the W10 finding **B3-5** ("the tie rule is reachable only via a test
factory; the tie/refusal policy is dead code until a second factory exists").

**Lane** B — decoders (`rustamp-decoders`).

**Owned paths**
- `crates/rustamp-decoders/**` (new `src/wave_dec.rs`, `src/lib.rs`,
  `src/symphonia_dec.rs` for two `pub(crate)` markers, `tests/selection.rs`,
  `tests/robustness.rs`, `CONTRACT_NOTES.md`)
- This card (`docs/tasks/B4-second-decoder.md`) only.
- Do **not** touch `rustamp-core` (the rule is not changed, only exercised),
  `rustamp-app`, `rustamp-audio`, `rustamp-dechost`, `rustamp-mediahost`,
  `rustamp-plughost`, `rustamp-ui`, `rustamp-skin` or any other crate.

**Allowed sources**
- `docs/rfc/0013-decoder-selection.md` (the rule), `docs/rfc/0002-audio-contracts.md`.
- `docs/tasks/B3-decoder-selection.md`, `docs/tasks/W10-batch-review.md` (B3-5).
- The public RIFF/WAVE container layout: chunked RIFF, the `fmt `/`data` chunks,
  the `WAVE_FORMAT_EXTENSIBLE` layout and the standard KSDATAFORMAT sub-format
  GUIDs. Freely licensed, public format documentation; nothing copied.
- `crates/rustamp-core/src/audio.rs`, `crates/rustamp-decoders/**`.

**Forbidden sources**
- Winamp source, SDK headers or disassembly, the Winamp 2.95 binary,
  `D:\code\rustamp-research`, `D:\code\rustamp-refbin`.
- No downloads, no `cargo add`, no new dependencies, no window, no audio.
- **No test execution on this host** (the suite runs in the VM); no commits.

## 1. The second factory

`WaveDecoderFactory` (`wave_dec.rs`, name `"wave"`) decodes **RIFF/WAVE** —
`WAVE_FORMAT_PCM` (8-bit unsigned, 16/24/32-bit signed LE),
`WAVE_FORMAT_IEEE_FLOAT` (32/64-bit) and `WAVE_FORMAT_EXTENSIBLE` resolving to
PCM or IEEE float. It is the classic Winamp `in_wave` format family.

**Why that is honest**

- It is a real, fully specified, publicly documented container that people
  actually play; the decoder reads real files and produces real interleaved
  `f32` PCM (bit-exact against the samples a writer stored, see §3).
- It was implemented from that public layout only. No Winamp source, SDK header
  or disassembly was read for it; nothing was copied, so there is no
  `THIRD_PARTY.md` entry to add.
- It is a genuinely **independent** implementation, not a wrapper around
  symphonia: that is the whole point. A second factory that delegated to
  symphonia would be a test double wearing a real name and would not exercise
  the rule.
- The overlap is genuine, not staged: it claims the same extensions (`wav`,
  `wave`) and the same 12-byte content signature (`RIFF`…`WAVE`) as symphonia,
  at the same confidence. Both facts are properties of the format, not of the
  test.

**Dependency note.** No dependency was added. `hound` is already a *dev*
dependency and is used only to write WAV bytes in the tests; the decoder itself
is `std`-only. Moving `hound` into `[dependencies]` would change the crate's
normal dependency closure and require a `THIRD_PARTY.md` update, which is outside
this card's owned paths.

**Registration order.** `default_registry` registers `symphonia` first and `wave`
second. Registration order is the RFC 0013 tie-break and is therefore part of the
contract: for the media both claim, symphonia is the more complete decoder (it
also reads ADPCM and other WAV codecs), so **the default behaviour for WAV is
unchanged** by this card. `wave` is the second claimant and the fallback that
does not route the most basic format through the symphonia dependency.

Stated plainly: because both factories claim exactly the same WAV media, `wave`
never wins a tie inside `default_registry` — that is what "earlier registration
wins" means here. It is reachable as a real decoder when a caller registers it
first or alone (the `tests/selection.rs` registries do both, and that is how it
decodes a real file in this card). Its purpose in `default_registry` is to make
the tie-break a real code path rather than a policy that only a test double could
reach (W10 B3-5), and to be an independent fallback for the most basic format.

## 2. How the tie and the refusal are now real

| Rule | Before (B3) | Now |
| --- | --- | --- |
| Content beats extension | pinned with a fake extension claimant | `tests/selection.rs`: real FLAC/MP3 bytes wearing `.wav`; `wave` really scores `EXTENSION`, `symphonia` really scores `CONTENT`, and the content claimant wins with the extension claimant registered first |
| Tie by registration order | only two `ClaimingFactory` doubles | a real WAV file is a real tie: `symphonia.probe == wave.probe == CONTENT` (signature) and `== EXTENSION` (`.wav` name). Both registration orders are asserted, and the winner decodes the file to the same frame count |
| Refusal | only fake claimants answered `NONE` | a real RIFF container that is not WAVE (AVI) is claimed by neither real factory → `probe` is `None`, `open` is `Err(UnsupportedFormat("avi"))`; signature-less bytes with no extension give `"unknown"` |
| Reachability | one factory in `default_registry` | two, asserted by name and order; the B3 test that asserted one claimant was replaced |

The rule itself was **not** changed: `rustamp-core` was not touched. Nothing in
RFC 0013 §6 (Q1–Q4) is pre-empted — the baseline stays "earlier registration
wins", and this card only makes that baseline observable with real decoders.

## 3. Tests

`crates/rustamp-decoders/tests/selection.rs` (two real factories):

- `the_default_registry_has_two_real_factories`
- `a_real_wav_file_is_a_genuine_tie_resolved_by_registration_order`
- `a_real_content_signature_beats_a_real_extension_claim_in_both_orders`
- `two_real_factories_refuse_media_that_neither_claims`
- `each_real_factory_decodes_a_real_file_on_its_own` (symphonia: the
  `sine.flac` fixture; wave: a real WAV written to a temp file and compared
  sample-by-sample)

`crates/rustamp-decoders/src/wave_dec.rs` (unit tests, generated in memory):
probe claims, 16-bit exact decode, 8-bit offset binary, extensible 20-valid-in-24
container shift, 32/64-bit float, unknown chunk + odd padding, non-WAVE RIFF
refusal, `data` size past the end, declared size larger than the file, trailing
partial frame, unknown length (streaming marker + no reportable end), sample
exact seek, and a table of hostile/unsupported `fmt ` chunks.

`crates/rustamp-decoders/tests/robustness.rs`: the B3 one-claimant test was
replaced by `the_default_registry_has_two_real_claimants_so_the_tie_rule_is_reachable`.
The fake-claimant tests stay: they pin the *rule*, the real pair pins its
*reachability*.

## 4. What is still only covered by the test factory

- **Custom confidence values.** RFC 0013 speaks of "any score ≥ 90" and "any
  score < 90". Only `ClaimingFactory` can produce such a score (or a
  `Confidence` that is neither 0, 40 nor 90); both real factories only ever
  return the three named constants.
- **A tie between two factories that both claim by *content* but decode
  differently.** The real pair agrees on the samples, so the tie test proves
  *which* factory was selected (via `probe`, the contract's selection surface)
  and that the winner decodes the file, not that a different decoder produced
  different output.
- **`open()` after a probe that was not the winner's** (i.e. backtracking,
  RFC 0013 Q4) — untouched, as before.
- **Priority ordering (Q2) and extension-fallback suppression (Q3)** — still
  open questions; no behaviour was added for them.

## 5. Acceptance gates (this worktree)

- [x] `cargo fmt --all -- --check` clean.
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
      clean (no warnings).
- [x] `cargo build --workspace` clean.
- [ ] `cargo test --workspace --all-features` — **not run on this host** (VM
      job). The new tests are reasoned-only; see the review note below.
- [x] No test opens a window, plays audio or downloads; only this worktree was
      written; nothing was committed.

**Review note (honesty about the unrun tests).** The unit tests were desk-checked
against the exact byte layout the helpers write (chunk offsets, the extensible
sub-format GUID tail at body bytes 26..40, `cbSize` at 16, `wValidBitsPerSample`
at 18) and against the `hound` writer's actual output (16-bit stereo is
`WAVE_FORMAT_PCM`; more than 2 channels or more than 16 bits is
`WAVE_FORMAT_EXTENSIBLE`). The bit-exact WAV assertions use the same
`q / 2^(bits-1)` convention as `tests/decode.rs`, which is exact for a power-of-two
divisor. They are still unverified by execution until the VM run.

## 6. Provenance

- Specs / free sources used: RFC 0013, RFC 0002, the public RIFF/WAVE container
  layout (chunked RIFF, `fmt `/`data`, `WAVE_FORMAT_EXTENSIBLE`, KSDATAFORMAT
  sub-format GUIDs).
- No Winamp source, SDK material or disassembly was viewed.
- No third-party code was copied.
