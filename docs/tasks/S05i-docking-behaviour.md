# Task card S05i — Implement the docking and snap behaviour the spec fixes

**Goal** `specs/S05-docking.md` §5 states what the app must do on a drag or a snap,
and the measurements behind it are audited. Nothing implements it yet. Implement
it, from the spec only.

**Lane** S/UI implementation. **Owned paths** a new worktree; inside it,
`crates/rustamp-ui/**` and `crates/rustamp-app/**`. Do not touch `rustamp-core`,
`rustamp-proto`, `rustamp-ipc`, the plugin host or `specs/**` — report those needs
instead.

**Allowed sources** `specs/S05-docking.md` (**audited** — the authority; §3 the
positions, §4 the docking and snap rules, §5 the behavioural contract, §7 the open
questions), `specs/S01-window-geometry.md` (audited) for the window geometry,
`specs/S02-main-window-behaviour.md` for the window states, and the existing
`crates/rustamp-ui/**` / `crates/rustamp-app/**` code. **Clean room:** never read
Winamp source, SDK headers, disassembly, the 2.95 binary, `rustamp-research` or
`rustamp-refbin`.

**Scope**
- The **neighbour-tracking rule**: a docked neighbour follows the main window and
  tracks its **bottom edge** (the spec's 102 px = 116 − 14 observation is the
  evidence for the shaded case).
- The **snap rules** §4 fixes, including which edges snap and the docked-versus-
  adjacent distinction — and honour §7: where the spec says the snap **tolerance**
  is unmeasured, do **not** invent a number; put it in one named constant with a
  comment saying it is provisional and why.
- The **move test** the spec records (+100 then +200 px) as the behaviour a test
  can assert.
- The **windowshade and double-size** size changes as the spec states them
  (275×14 shaded; 550×232 double-size for main and equalizer while the playlist
  editor stays 275×116) — the state changes, not a pixel copy.
- Tests for each rule that do not need a window (the geometry and the tracking are
  pure functions of positions and sizes); a window-opening test must be
  `#[ignore]`d.

**Hard rules** clean room; no downloads; no `cargo add`; no git write commands;
**run no test** (the orchestrator runs the suite in the Hyper-V test VM); no
window may be opened by anything you do; every spawned child uses a no-window
helper; English.

**Acceptance** fmt and clippy clean; each implemented rule cites the spec section
in a comment; nothing contradicts the spec, and anything the spec leaves open is
left open in the code too (a named provisional constant, not a guess).

**Report** the files; each rule and the spec section it comes from; the
provisional constants and why; the tests; what you could not implement and why;
the fmt/clippy results; confirmation that you ran no test, opened no window and
ran no git write.
