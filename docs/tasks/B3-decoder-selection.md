# Task card B3 — Decoder registry selection rule (lane B)

**Goal**  
Settle the decoder registry selection rule (RFC 0013): guarantee that content
decides over extension, establish deterministic tie-breaking by registration order,
guarantee clean refusal on unclaimed inputs, and pin the guarantees with multi-factory
integration tests using fake claimants and repository fixtures.

**Lane** B — decoders (`rustamp-decoders`) & Lane I (contracts / `rustamp-core`).

**Owned paths**
- `docs/rfc/0013-decoder-selection.md`
- `crates/rustamp-core/src/audio.rs` (only the registry contract and doc comments)
- `crates/rustamp-decoders/**` (tests, `CONTRACT_NOTES.md`, `src/lib.rs`)
- This card (`docs/tasks/B3-decoder-selection.md`) only.
- Do **not** touch `rustamp-app`, `rustamp-ui`, `rustamp-skin`, `rustamp-audio`,
  `rustamp-ipc`, `rustamp-mediahost`, or `rustamp-plughost`.

**Allowed sources**
- `docs/rfc/0001-core-contracts.md` (M0 contracts).
- `docs/rfc/0002-audio-contracts.md` (accepted).
- `docs/rfc/0013-decoder-selection.md` (proposed).
- `docs/tasks/B2-decoder-robustness.md` (audit findings).
- `crates/rustamp-core/src/audio.rs` and `crates/rustamp-decoders/**`.

**Forbidden sources**
- Winamp 5.x source, SDK headers or disassembly, the Winamp 2.95 binary,
  `D:\code\rustamp-research`, `D:\code\rustamp-refbin`. Nothing from them.
- No downloads, no `cargo add`, no new dependencies, no window, no audio.
- **No test execution on this host** (the suite runs in a VM); no commits.

**Selection guarantees and what changed**

| Guarantee | Before | After (RFC 0013) |
| --- | --- | --- |
| Content vs Extension | Handled by raw scores (90 vs 40), but unverified across multiple factories; extension could win if registered first | Pinned by contract and tests: `Confidence::CONTENT` strictly beats `Confidence::EXTENSION` regardless of registration order |
| Tie between factories | Undocumented registration-order resolution | Deterministic tie-break: earlier registration wins; does not refuse; open questions record owner choices |
| Unclaimed input | Silent fallback / probe returns None | Deterministic refusal: `probe` returns `None`, `open` returns `Err(Error::UnsupportedFormat(ext))` |
| Multi-factory coexistence | Only one factory registered in practice | Verified with fake competing factories alongside `SymphoniaDecoderFactory` |

**Tests**
- Integration (`tests/robustness.rs`):
  - `content_beats_extension_even_when_extension_factory_registered_first`: FLAC content with fake extension claimant registered first.
  - `tie_break_is_deterministic_by_registration_order_for_both_content_and_extension`: bidirectional tests for CONTENT and EXTENSION ties.
  - `nothing_claims_is_refused_with_clear_unsupported_format_error`: extension-provided vs unknown extension refusal.
  - `real_symphonia_path_opens_every_repo_fixture_with_competing_extension_factory`: all 5 repo fixtures (`sine.flac`, `sine.mp3`, `sine.ogg`, `sine_aac.m4a`, `sine_alac.m4a`) open and decode cleanly despite competing extension claimant.

**Open questions for the owner (RFC 0013 §6)**
- Q1: Outright refusal on ties vs registration order.
- Q2: User/Plugin priority ordering.
- Q3: Extension fallback suppression on sniff failure.
- Q4: Backtracking on factory `open` failure.

**Acceptance gates (this worktree)**
- [x] `cargo fmt --all -- --check` clean.
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [x] `cargo build --workspace` clean.
- [ ] `cargo test --workspace --all-features` — **not run on this host** (VM job);
      the new tests are reasoned-only.
- [x] No test opens a window, plays audio, or downloads; only this worktree was
      written; nothing was committed.
