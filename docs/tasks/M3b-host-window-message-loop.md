# Task card M3b — The host window and the message loop that serves the surface

**Goal** M3a gave `rustamp-plughost` a table-driven IPC **answering surface** (id →
reply, with the evidence per row). Nothing serves it yet. Build the piece that
does: the host's window and message loop, so that a plugin's `SendMessage` /
`PostMessage` to the host window is dispatched to the surface and the reply goes
back — and prove it with a test plugin of **our own**.

**Lane** M — plugin host. **Owned paths** a new worktree; inside it,
`crates/rustamp-plughost/**` (and `crates/rustamp-ipc/**` if the dispatch needs a
shared shape). Do not touch `rustamp-proto`, `rustamp-core`, the app, `xtask` or
`specs/**` — report those needs instead.

**Allowed sources** the merged M3a surface (`crates/rustamp-plughost/src/ipc.rs`)
and its card `docs/tasks/M3a-plughost-ipc-surface.md`; `specs/S09-plugin-abi.md`
(audited) for the ABI; `crates/rustamp-plugin-abi/**`; `crates/rustamp-ipc/**`;
`docs/rfc/0006-out-of-process-hosting.md` (the host's design); `docs/HANDOFF.md`
§4/§5 for the state. **Clean room:** never read Winamp source, SDK headers,
disassembly, the 2.95 binary, `rustamp-research` or `rustamp-refbin`.

**Scope**
- A **host window** with the class name the ABI fixes (read S09 for it — do not
  invent one), created by the host, with the handle the surface reports for the
  window-selector ids.
- A **message loop** that receives the classic IPC messages and dispatches them
  through the M3a surface, returning the reply as the message result. Keep the
  loop and the surface separable so each is testable alone.
- **Threading and lifetime**: say in comments which thread owns the window and
  which calls may come from a plugin's thread, following S09's threading rules.
- **A test plugin of our own** (in the worktree, clearly marked as ours, not
  derived from any SDK sample) that exercises the surface in-process: it asks the
  version, the playing state, a volume read/write pair, an unknown id, and a
  handle selector, and asserts the replies. This is what proves the dispatch
  works end to end.
- **No invented ids or values**: every id comes from the M3a table; an id the
  table does not have falls through to the measured `1`.

**Hard rules** clean room; no downloads; no `cargo add`; no git write commands;
**run no test** (the orchestrator runs the suite in the Hyper-V test VM); no
window may be opened by anything you do in this session (the host window is
created by the tests, which run in the VM — make sure nothing the build or a
manual command does opens one); every spawned child uses a no-window helper;
English.

**Acceptance** fmt and clippy clean; the dispatch is covered by tests; the test
plugin is our own code with a comment saying so; the surface's table is
unchanged except where a dispatch need forces a documented addition.

**Report** the files; the window class and where S09 fixes it; the dispatch path;
the test plugin and what it asserts; what you could not do without the VM; the
fmt/clippy results; confirmation that you ran no test, opened no window and ran
no git write.
