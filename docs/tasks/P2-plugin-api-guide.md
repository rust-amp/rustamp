# Task card P2 — The plugin-author API guide (`docs/plugin-api.md`)

**Goal** Write the guide a plugin author needs to build a classic plugin against
Rustamp: the struct layouts, export names, call order, lifetimes and threading
rules of the `in_`/`out_`/`vis_`/`dsp_`/`gen_` interfaces, as our host implements
them. `docs/notes/plugin-authors.md` (the licensing guide) names this as its
follow-up and deliberately does not contain it.

**Lane** P — paperwork/API docs. **Owned paths** `docs/plugin-api.md` (new file;
the only file you may change). Everything else is read-only — in particular do
**not** edit the crate, `docs/notes/plugin-authors.md`, `specs/**` or
`THIRD_PARTY.md`; report needs instead.

**Allowed sources** — all in the repo and all safe:
- `specs/S09-plugin-abi.md` (**audited**): the ABI facts.
- `crates/rustamp-plugin-abi/src/**` and its `OPEN_QUESTIONS.md`: the concrete
  types, constants, export names and the 116 layout asserts.
- `crates/rustamp-plugin-abi/OPEN_QUESTIONS.md` for what is **not** settled.
- `docs/notes/plugin-authors.md` (the licensing guide, for the boundary between
  licensing and API) and `docs/rfc/0006-out-of-process-hosting.md` §5 for the
  decoder/DSP helper story.
- `docs/HANDOFF.md` §4/§5 for the host's state and its open items.

**Forbidden** Winamp source, SDK headers or documentation, disassembly, the 2.95
binary, `rustamp-research`, `rustamp-refbin`. No downloads, no web fetching.

**Scope**
- **In scope:** per plugin kind — the module struct and its fields, the export
  name and its expected signature, the call order (init/quit, config, play/pause
  notifications, the vis data callback and its threading), ownership and lifetime
  rules (who frees what, which callbacks may be null), the units (sample formats,
  the 576-sample vis frame), what the host guarantees and what it does not, and a
  minimal skeleton **described in prose** for each kind.
- **Out of scope:** copying any header or SDK text; inventing API that S09 does
  not record (mark it provisional or as an open question instead); the licensing
  analysis (that is `plugin-authors.md`); building a plugin.
- **Never promote a gap to a fact.** Every statement must trace to S09 or to the
  crate; anything else is marked **provisional** or listed as an open question.

**Acceptance**
- The guide exists at `docs/plugin-api.md`, is written for a plugin author (not
  for us), and contains no copied SDK text and no code copied from a forbidden
  source. Small illustrative snippets of **our own** ABI types are fine if they
  are described as such.
- It contradicts neither S09 nor the crate; where the crate's layout asserts
  encode something S09 does not state, say which is authoritative and why.
- `git status --short` shows only the new file.

**Report** the path; the per-kind coverage in one line each; what you marked
provisional or left as an open question; anything you could not state from the
allowed sources; confirmation that you read no forbidden source and touched
nothing else.
