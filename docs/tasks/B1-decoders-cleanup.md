# Task card B1 — Decoder follow-up: drop the redundant `Mutex`, fix the notes (M1)

**Goal**
Close HANDOFF open point 1: remove the now-redundant `Mutex` in the decoder's
reader wrapper (the `ReadSeek: Send + Sync` bound from RFC 0002 C2 makes it
unnecessary) and correct the wording in `CONTRACT_NOTES.md` that describes the
old arrangement. No behaviour change, no contract change.

**Lane** B — decoders (`rustamp-decoders`).

**Owned paths**
- `crates/rustamp-decoders/**`
- Do **not** edit the root `Cargo.toml`, `rustamp-core`, `rustamp-proto`,
  `rustamp-audio`, `xtask`, `docs/**`, `tests/golden/**` or `THIRD_PARTY.md`.
  If something outside these paths must change, report it as a finding instead
  of editing it. No new dependencies.

**Allowed sources**
- `docs/rfc/0002-audio-contracts.md` (accepted): C1 (`skipped_packets`), C2
  (`ReadSeek: Send + Sync` — the reason the `Mutex` is redundant), C3
  (`FormatChanged`).
- `crates/rustamp-decoders/CONTRACT_NOTES.md` (the wording to fix),
  `crates/rustamp-core/src/audio.rs` (the `Decoder`/`DecoderFactory` traits,
  read-only in effect — do not edit).
- The repository and the symphonia API already vendored.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`, `D:\code\rustamp-refbin`.
  Nothing from them.
- Do not download anything. Do not run a reference binary.
- **Headless only.** Another agent may be driving the reference Winamp on this
  same machine while you work: your tests must never open a window, send
  input, or run an example that opens a window. The decoder tests already run
  headless — keep it that way.

**Scope**
- **In scope:** find the `Mutex` in the reader wrapper the C2 follow-up refers
  to and remove it (replacing it with the direct `Send + Sync` use the bound
  allows); prove thread-safety is preserved (the types were already `Sync` —
  that is the whole point, state where the bound now carries the guarantee);
  update `CONTRACT_NOTES.md` so no sentence describes the removed `Mutex` or
  the pre-C2 arrangement; keep the `skipped_packets` override and the
  `FormatChanged` behaviour exactly as they are.
- **Out of scope:** new formats (no Opus — it needs libopus via FFI), the
  sidplayfp decision (RFC 0006, accepted; implementation waits for Q1), gapless
  or seek-behaviour changes, property scanning (`duration_ms`/`bitrate_kbps`
  stay decoder-filled per the meta contract).
- Never promote a gap to a fact.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green, with the decoder tests
      (25) still passing; add a multithreaded read test if none covers shared
      `&`-access through the wrapper (the guarantee the `Mutex` used to give).
- [ ] `cargo test -p rustamp-skin` still green.
- [ ] `cargo test -p rustamp-ipc --target i686-pc-windows-msvc`,
      `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` and
      `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` still green.
- [ ] `cargo xtask golden` reports no mismatch; `cargo xtask ring-cross` still passes.
- [ ] No test opens a window or sends input; `grep -rn Mutex` in the crate
      shows no remaining synchronisation that the `Sync` bound already covers
      (report each remaining `Mutex`, if any, with its reason).

**Provenance notes**
- None expected (our own code and RFC).

**Report back**
Files changed; where the `Mutex` was and what replaced it; the exact
`CONTRACT_NOTES.md` wording changes; the new/updated test list and full gate
results; anything you could not verify.
