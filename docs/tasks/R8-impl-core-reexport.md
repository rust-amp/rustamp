# Task card R8 — Implement RFC 0008: re-export the payload enums (M1)

**Goal** `rustamp-core` re-exports the three RFC 0005 payload enums so its facade
is complete, with a test that fails if the list ever loses one. RFC 0008 is
accepted-in-substance; implement it, do not redesign it.

**Lane** I — contracts. **Owned paths** `crates/rustamp-core/**` only. Anything
else is a finding, not an edit (in particular do not touch `rustamp-proto`,
`rustamp-ui`, the root `Cargo.toml`, `docs/**`).

**Allowed sources** `docs/rfc/0008-core-reexport-payloads.md` (your contract),
`docs/rfc/0004-plugin-licensing.md` (the permissive-crate invariant),
`crates/rustamp-proto/src/command.rs`, `crates/rustamp-core/src/command.rs`.

**Forbidden** Winamp source/SDK/disassembly/binary, `D:\code\rustamp-research`,
`D:\code\rustamp-refbin`. No downloads.

**Scope** Add `JumpTarget`, `OpenMediaKind`, `PrefsPage` to core's `command`
re-export; add a compile-time test that names all three through
`rustamp_core::command::…` so a missing re-export fails the build; state the rule
in the module docs (a re-exported variant drags its payload types with it).
**Out of scope:** switching to a glob re-export (RFC §6 Q1 stays open), changing
any wire type, `rustamp-ui`'s own imports.

**Acceptance** `cargo fmt --all -- --check`, `cargo clippy --workspace
--all-targets --all-features -- -D warnings`, `cargo test --workspace
--all-features` green; the permissive-closure proof still clean.
**No test may open a window and none may spawn a child on the host** (GUI runs
only in the VM/sandbox); do not run the suite yourself — the orchestrator runs it
in the test VM.

**Report** files changed; how the test fails if an enum is dropped; the gate
results you could run without executing tests; findings.
