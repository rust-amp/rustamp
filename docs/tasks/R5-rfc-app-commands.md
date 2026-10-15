# Task card R5 — Draft RFC 0005: the missing app-level `Command`s (M1/M2)

**Goal**
Write the **draft** (proposed, not accepted) of `docs/rfc/0005-app-commands.md`:
which new `Command` variants the ~30 shortcut actions from the D3 findings need,
with exact names, payloads and semantics, plus what each decision costs the wire
protocol and the IPC version. No implementation — the RFC is the deliverable.

**Lane** R — RFC planning (docs only).

**Owned paths**
- `docs/rfc/0005-app-commands.md` (new file; the only file you may change).
- Everything else is **read-only**. In particular do not edit `rustamp-core`,
  `rustamp-proto`, `rustamp-ipc`, `rustamp-ui`, `docs/HANDOFF.md` or any other
  file. If the RFC needs a change you cannot make from the draft (for example a
  `PlayerState` field), write it into the RFC as an open question.

**Allowed sources**
- `docs/HANDOFF.md` §5 open point 18: the ~30 actions with their command ids and
  the three called-out problems (`LoadSkin` needs a path, `OpenPrefs` cannot
  carry a page, the volume-step placeholder).
- `crates/rustamp-proto/src/command.rs` (the current `Command` enum),
  `crates/rustamp-core` re-exports, `crates/rustamp-ipc` protocol v1
  (`Message`, framing, `PROTOCOL_VERSION`), `crates/rustamp-ui/src/shortcuts.rs`
  (`Unhandled(action)` and the command ids).
- `docs/rfc/0001-core-contracts.md`, `docs/rfc/0004-plugin-licensing.md` (the
  permissive-crate invariant: anything the plugin host must speak lives in
  `rustamp-proto`, never in a GPL-only crate), `specs/S03-shortcuts.md`
  (audited), `specs/S02-main-window-behaviour.md` (audited, for effects).
- The repository's existing RFCs for shape and tone.

**Forbidden sources**
- Winamp 5.x source, Winamp SDK headers or documentation, disassembly, the
  Winamp 2.95 binary, `D:\code\rustamp-research`, `D:\code\rustamp-refbin`.
  Nothing from them.
- Do not download anything. Do not run a reference binary.

**Scope**
- **In scope:** one RFC draft covering, for each of the ~30 actions:
  the proposed variant name and payload (or a reasoned "no command: UI-local /
  app-local" verdict); where it is handled (engine, app, UI machine); whether
  it crosses IPC and what that means for `PROTOCOL_VERSION` and the v1 framing;
  the `LoadSkin`-path and `OpenPrefs`-page problems; the volume-step question
  (name the constant, keep it provisional, do not invent a measured value); the
  playlist-window and equalizer-window actions that are unfillable until those
  windows exist; acceptance criteria for the later implementation lane.
- **Out of scope:** implementing anything; changing any contract; accepting the
  RFC (that is the project owner's decision); S04 menus, S05 docking, S14 title
  formatting beyond naming the dependency.
- Follow RFC 0001/0002/0004 conventions: units (ms as `u64`, volume
  `0..=255`), backwards-compatibility rules, and an explicit "what stays
  UI-local" section. Every variant must say whether a plugin host must forward
  it (hence `rustamp-proto`) or must never see it.

**Acceptance tests**
- [ ] The draft exists at `docs/rfc/0005-app-commands.md` and follows the shape
  of the earlier RFCs (motivation, proposal per item, wire/IPC impact,
  backwards compatibility, acceptance criteria, open questions).
- [ ] Every action in HANDOFF open point 18 appears exactly once: with a variant
  proposal or a reasoned no-command verdict.
- [ ] Nothing outside the owned paths was modified (`git status --short` shows
  only the new file).
- [ ] No `cargo` build is required (docs only), but the draft must not promise an
  API that contradicts the current `Command` enum — quote current variant names
  exactly.

**Provenance notes**
- Specs are facts, not code. Command ids from S03 are public interoperability
  ids and may appear as constants.

**Report back**
The RFC path; per-action decisions in one compact table (action, id, verdict:
new variant with name / UI-local / app-local / deferred, IPC impact); the
wire-version recommendation with reasoning; the open questions left for the
owner; anything you could not decide from the allowed sources.
