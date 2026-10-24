# Task card M3a — The plugin host answers the IPC surface it already knows

**Goal** `rustamp-plughost` is a stub. Its first real job is the one S10 measured:
answer the classic `WM_USER` IPC surface in-process, so that a loaded plugin gets
the replies the reference gives. This is also the **in-process caller** the S10
audit is blocked on: with it, the pointer ids and the version constant can be
cross-checked from inside the process instead of by posting messages from
outside.

**Lane** M — plugin host. **Owned paths** a new worktree; inside it,
`crates/rustamp-plughost/**` and, if the surface needs to be shared,
`crates/rustamp-ipc/**`. Do not touch `rustamp-proto`, `rustamp-core`, the app or
`specs/**` — report those needs instead.

**Allowed sources** `specs/S10-ipc.md` **if it exists**, otherwise the measured
replies in `docs/HANDOFF.md` §5 and the audit summary the orchestrator names in
your prompt; `specs/S09-plugin-abi.md` (audited) for the ABI; the crate
`rustamp-plugin-abi` (the ids and constants); `crates/rustamp-ipc/**` (what our
side already speaks); `docs/rfc/0006-out-of-process-hosting.md` (the host's
design). **Clean room:** never read Winamp source, SDK headers, disassembly, the
2.95 binary, `rustamp-research` or `rustamp-refbin`.

**Scope**
- Implement the **answering surface** for the measured ids: the version query
  (`0x2905` per RFC 0009), playing state, list position and length, output time,
  the equalizer read/write pair, shuffle and repeat, the two `-666` queries, the
  module name, the window-handle selectors, the menu handle, the video queries,
  and the **unsupported-id reply of `1`** (not `0`).
- **Units and conventions from the measurements**: the reply is a 32-bit value
  where a "negative" is the low 32 bits unsigned; **volume is unsigned 0..=255
  and pan is signed**; a state that needs a playing reference must say so rather
  than invent a value.
- A **table-driven** shape: one place that maps id → reply, so the surface can be
  audited against S10 line by line, with the source of each reply named in a
  comment (the run and the step).
- What the host **cannot** answer yet (the ids that need a plugin in process, and
  the ones S10 leaves open) must be explicit: an unknown id is not silently
  answered.
- Unit tests for the table and the conventions (the orchestrator runs them in the
  test VM).

**Hard rules** clean room; no downloads; no `cargo add`; no git write commands;
**run no test** (the orchestrator runs the suite in the Hyper-V test VM); no
window may be opened; every spawned child uses a no-window helper; English.

**Acceptance** fmt and clippy clean; the surface compiles; the unknown-id path is
proven by a test; the reply table names its evidence per row.

**Report** the files; the id table with each row's evidence; the conventions
implemented; what you could not answer and why; the fmt/clippy results;
confirmation that you ran no test, no window and no git write.
