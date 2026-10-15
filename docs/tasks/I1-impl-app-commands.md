# Task card I1 — Implement RFC 0005 app-level `Command`s (M1)

**Goal**
Implement the accepted RFC 0005: the 20 new `Command` variants (appended after
`OpenPrefs`), the `OpenMediaKind`/`JumpTarget`/`PrefsPage` enums, the new
`WindowId::Video` variant, the stop-after-current `PlayerState` field,
`PROTOCOL_VERSION` 2, the UI dispatch mapping with machine predictions, the
engine application of the engine-owned commands, and the plugin-host id table.
RFC 0005 is accepted — implement it, do not re-design it. Where the RFC says
"owner decided, see §17", the decision is final.

**Lane** I — implementation (`rustamp-proto`, `rustamp-ipc`, `rustamp-ui`,
`rustamp-audio`, `rustamp-plughost`).

**Owned paths**
- `crates/rustamp-proto/**`
- `crates/rustamp-ipc/**`
- `crates/rustamp-ui/**`
- `crates/rustamp-audio/**`
- `crates/rustamp-plughost/**`
- Do **not** edit the root `Cargo.toml`, `rustamp-core` (the re-exports carry
  the new items automatically — verify, do not edit), `rustamp-skin`,
  `rustamp-gfx`, `rustamp-decoders`, `rustamp-meta`, `rustamp-playlist`,
  `xtask`, `docs/**`, `tests/golden/**` or `THIRD_PARTY.md`. If something
  outside these paths must change, report it as a finding instead of editing
  it. No new dependencies: nothing you need is missing from `Cargo.lock`.

**Allowed sources**
- **`docs/rfc/0005-app-commands.md` (accepted in substance)** — your contract,
  including §17 (owner decisions: O2 `WindowId::Video`, O8 `PlayerState`
  field) and the measurement updates (O10 step is 4, O6/O7 are prefs pages,
  `OpenVisOptions` is gone).
- `specs/S02-main-window-behaviour.md`, `specs/S03-shortcuts.md` (audited).
- `crates/rustamp-proto/src/command.rs`, `state.rs`, `event.rs` (current shapes
  to extend), `crates/rustamp-ipc/src/message.rs` (version history),
  `crates/rustamp-ui/src/shortcuts.rs` (dispatch + `VOLUME_STEP`),
  `crates/rustamp-audio` (engine applier), `crates/rustamp-plughost` (id table).

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`, `D:\code\rustamp-refbin`.
  Nothing from them.
- Do not download anything. Do not run a reference binary.
- **No window, no input, no example runs.** Another agent may be driving the
  reference Winamp while you work: your tests must be headless (no `winit`
  windows, no sent input, no opened examples). Build the example if you must,
  never run it.

**Scope**
- **In scope (RFC §15 criteria 1–9, adapted):**
  1. The 20 variants + 3 enums in `rustamp-proto`, appended after `OpenPrefs`;
     `WindowId::Video`; the stop-after-current `PlayerState` field;
     `rustamp-core` re-exports verified (not edited).
  2. `VOLUME_STEP: u8 = 4` (measured; keep the name and a provenance comment).
  3. `PROTOCOL_VERSION = 2` with the version history updated. **Coordination:**
     RFC 0006 also proposes v2 — it goes 2→3 later when it lands (sequential
     bumps, per HANDOFF). Your bump covers the RFC 0005 sets only.
  4. `Message::CommandToHost(Command)` for the §5.4 bridge (additive, rides v2).
  5. UI dispatch: every main-table action maps to a `Command` or a documented
     verdict; `Unhandled` remains only for the deferred actions + app-local
     `NewInstance`; machine predictions for `PlayIndex`/`PlayEntry`/`PlayLast`
     (→ Playing) and `SkipBy` (no transport change).
  6. Engine application of `SkipBy`, `PlayLast`, `PlayIndex(0)` (alreadyworks —
     verify), and stop-after-current (halt at track end instead of advancing).
  7. The host id→`Command` table for every public id with a variant; ids
     without variants are logged and dropped.
  8. Old-bytes-unchanged: a wire test showing every pre-0005 `Command` encodes
     exactly as before.
  9. The permissive-closure proof re-runs clean (no GPL in proto/plugin-abi/
     ipc/plughost closures).
- **Out of scope:** deferred-action handling (minibrowser, video window beyond
  the enum variant, playlist search, M3 vis bridge, bookmark store, skin
  browser UI, prefs UI), S04/S05/S13/S15 decisions, O5/O15 measurements.
- Never promote a gap to a fact. Anything the RFC leaves open stays open.

**Acceptance tests**
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.
- [ ] `cargo test --workspace --all-features` green, with new tests for: every
      new variant's UI dispatch (action → expected `Command`); `Unhandled`
      exactly for the deferred set + `NewInstance`; volume keys with step 4
      (clamp 0/255, partial end steps); `SkipBy`/`PlayLast`/stop-after-current
      engine behaviour; host id-table coverage; old-bytes-unchanged wire test;
      `PROTOCOL_VERSION = 2`.
- [ ] `cargo test -p rustamp-skin` still green.
- [ ] `cargo test -p rustamp-ipc --target i686-pc-windows-msvc`,
      `cargo test -p rustamp-plugin-abi --target i686-pc-windows-msvc` and
      `cargo build -p rustamp-plughost --target i686-pc-windows-msvc` still green.
- [ ] `cargo xtask golden` reports no mismatch; `cargo xtask ring-cross` still passes.
- [ ] No test opens a window or sends input; the examples still build.

**Provenance notes**
- Specs are facts, not code. Command ids from S03 are public interoperability
  ids and appear as documented constants.

**Report back**
Files changed; per-criterion result (the 9 items above); the old-bytes wire
proof; the closure proof; the new test list and full gate results; anything
you could not verify; and questions the implementation exposed.
