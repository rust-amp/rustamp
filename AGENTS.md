# Agent instructions

Read these first, in this order:

1. [docs/HANDOFF.md](docs/HANDOFF.md): the current status, open points and next steps.
2. [CLEANROOM.md](CLEANROOM.md): binding legal rules. Never read Winamp source, SDK headers or disassembly when implementing, and never read `D:\code\rustamp-research`.
3. [docs/PLAN.md](docs/PLAN.md): the architecture and milestones.
4. [CONTRIBUTING.md](CONTRIBUTING.md): the coding rules.

## Short rules

- Talk to the user in German. Code, comments and docs are in English.
- Implement only from `specs/` and from freely licensed sources. Contract changes in `rustamp-core` require a new RFC in `docs/rfc/`.
- The player is 64-bit. Only `rustamp-plughost` is built for i686.
- Ask the user before downloads, before installing or running reference binaries, and before committing.
- Before reporting done, run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` and `cargo test --workspace --all-features`.
