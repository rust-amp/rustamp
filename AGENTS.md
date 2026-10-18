# Agent instructions

Read these first, in this order:

1. [docs/HANDOFF.md](docs/HANDOFF.md): the current status, open points and next steps.
2. [CLEANROOM.md](CLEANROOM.md): binding legal rules. Never read Winamp source, SDK headers or disassembly when implementing, and never read `D:\code\rustamp-research`.
3. [docs/PLAN.md](docs/PLAN.md): the architecture and milestones.
4. [CONTRIBUTING.md](CONTRIBUTING.md): the coding rules.

## Short rules

- **Tests always run in the interactive session** (owner rule, 2026-10-18).
  The guest's session 0 has no desktop, so anything that opens a window, a
  dialog, a menu or a plugin host fails there for the wrong reason. Run the
  suite through the interactive-session path, not the plain session-0 one, and
  never report a session-0 window failure as a product defect without re-running
  it in the interactive session first.

- **File search: use `es.exe`, always** (`C:\Program Files\Everything 1.5a\es.exe`),
  never a recursive directory walk. `es.exe <pattern>` prints full paths;
  `es.exe -r <regex>` for a pattern. The owner's standing instruction
  (2026-10-17).
- Talk to the user in German. Code, comments and docs are in English.
- Implement only from `specs/` and from freely licensed sources. Contract changes in `rustamp-core` require a new RFC in `docs/rfc/`.
- The player is 64-bit. Only `rustamp-plughost` is built for i686.
- Ask the user before downloads, before installing or running reference binaries, and before committing — **except adding a Rust dependency**: a well-maintained crate with a compatible license may be added without asking (owner rule, 2026-10-17); prefer it over a self-implementation, check the license, and record it in `THIRD_PARTY.md`. See `CONTRIBUTING.md` §Dependencies.
- Before reporting done, run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo test --workspace --all-features`.
