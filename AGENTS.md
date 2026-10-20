# Agent instructions

Read these first, in this order:

1. [docs/HANDOFF.md](docs/HANDOFF.md): the current status, open points and next steps.
2. [CLEANROOM.md](CLEANROOM.md): binding legal rules. Never read Winamp source, SDK headers or disassembly when implementing, and never read `D:\code\rustamp-research`.
3. [docs/PLAN.md](docs/PLAN.md): the architecture and milestones.
4. [CONTRIBUTING.md](CONTRIBUTING.md): the coding rules.

## Short rules

- **One worktree per subagent — always, before it starts** (owner rule, 2026-10-19).
  Every subagent gets its **own `git worktree`** created *before* the subagent is
  launched, never after. Two agents must never share a working directory or a
  crate. The worktree path is the **first line** of the subagent's prompt. An
  agent that cannot show its files from its own worktree has not finished.
  Corollary: the canonical tree `D:\code\Rustamp` is orchestrator-only; a
  subagent writes nothing there. Before creating the worktrees, make sure the
  baseline is **committed** — a worktree is cut from a commit, so uncommitted
  work in the canonical tree is invisible to every worktree.
  **Delete the worktree from disk as soon as its work is merged** (owner rule,
  2026-10-19): a merged lane leaves no worktree behind, so the tree list stays a
  picture of what is actually running.

- **A lane must leave the workspace compiling** (owner rule, 2026-10-19).
  If a lane changes a shared type — adds a field, a variant, a trait method —
  it must update **every consumer** in the same change, or the lane is not done.
  A lane that reports "this breaks `main` because another crate must change" and
  stops has not finished; the orchestrator grants it the minimal extra scope to
  make the workspace build. The done criterion is `cargo clippy --workspace` and
  `cargo test --workspace` green, not the lane's own crate.
  **After every merge, the orchestrator runs `cargo check --workspace --all-targets --all-features` itself** —
  a lane that only ran its own crate's tests can still leave a caller broken, and
  the breakage then surfaces at the *next* merge, far from its cause.
  **`--all-targets` is not optional:** without it the check does not build the
  test and example targets, which is exactly where a consumer is most likely to
  be left behind. A green merge with a test target that cannot compile is a
  false green; it happened when a signature grew a parameter and three test call
  sites kept the old argument list.

- **Garbage output → restart the task on a stronger model** (owner rule, 2026-10-19).
  When a lane reports success but the diff is speculative — invented APIs, alias
  functions/types that all do the same thing, dead code nothing calls — the work
  is **rejected, not merged**. Reset the worktree to `main` and relaunch the same
  task on a stronger model (e.g. `openai/gpt-6.1-sol#high`) with the failed
  attempt named in the prompt so it is not repeated. A lane that "passes clippy"
  while adding only names is a failed lane.

- **Before you write "X does not exist", search the repo for a measurement of X**
  (owner lesson, 2026-10-20). A dirty-room draft claimed `/ADD` was not a switch
  in Winamp 2.95; `docs/HANDOFF.md` had carried a **measured** line
  "`/ADD` enqueues without playing" for days, and the claim still went into an
  audited spec and from there into the CLI, where a lane had to invert a test.
  A spec claim of absence is only as strong as the search behind it: check
  `docs/`, the audited specs and the handoff logs first, and state the search
  that supports the absence.

- **Tests always run in the interactive session** (owner rule, 2026-10-18).
  The guest's session 0 has no desktop, so anything that opens a window, a
  dialog, a menu or a plugin host fails there for the wrong reason. Run the
  suite through the interactive-session path, not the plain session-0 one, and
  never report a session-0 window failure as a product defect without re-running
  it in the interactive session first. **This is about window/GUI tests only:**
  a headless unit test (no window, no device, no spawned process) **runs on the
  host** and must be run by every lane — "the tests need the VM" is never a
  reason to skip `cargo test` for headless tests.

- **No new Claude tasks** (owner rule, 2026-10-19). Do not start any further lane
  on Claude. Existing Claude lanes may finish; nothing new goes there. With
  Google's quota also exhausted until 2026-10-24, the remaining sources are
  **OpenAI** and the **free** models.

- **File search: use `es.exe`, always** (`C:\Program Files\Everything 1.5a\es.exe`),
  never a recursive directory walk. `es.exe <pattern>` prints full paths;
  `es.exe -r <regex>` for a pattern. The owner's standing instruction
  (2026-10-17).
- Talk to the user in German. Code, comments and docs are in English.
- Implement only from `specs/` and from freely licensed sources. Contract changes in `rustamp-core` require a new RFC in `docs/rfc/`.
- The player is 64-bit. Only `rustamp-plughost` is built for i686.
- Ask the user before downloads, before installing or running reference binaries, and before committing — **except adding a Rust dependency**: a well-maintained crate with a compatible license may be added without asking (owner rule, 2026-10-17); prefer it over a self-implementation, check the license, and record it in `THIRD_PARTY.md`. See `CONTRIBUTING.md` §Dependencies.
- Before reporting done, run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo test --workspace --all-features`.
