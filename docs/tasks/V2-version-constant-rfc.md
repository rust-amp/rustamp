# Task card V2 — RFC 0009: correct the IPC version constant

**Goal** The measured `IPC_GETVERSION` is **0x2905**, not the `0x2095` that the
audited S09 and the crate constant state (two independent sandbox runs agree).
Write **RFC 0009** for the correction and apply it. The measured value is a fact
of the reference; the S09 claim was SDK-derived and never measured, and the two
values sit on opposite sides of the `0x2900` threshold plugins test.

**Owned paths** `docs/rfc/0009-ipc-version-constant.md` (new),
`crates/rustamp-plugin-abi/**` (the constant and its doc/asserts),
`crates/rustamp-plugin-abi/OPEN_QUESTIONS.md`. Nothing else — report other needs.

**Allowed sources** `specs/S09-plugin-abi.md` §8.3 (the claim you are
correcting), the measurement evidence in `docs/HANDOFF.md` §5 (the new item),
`docs/notes/SDK-licensing.md` for why the SDK is not a source of facts, and the
crate. Do **not** read `rustamp-research` or the reference binary.

**Scope** The RFC states the contradiction, the evidence (two runs, the id, the
probe shape), why the constant matters (the `0x2900` test), the decision, the
backwards-compatibility note (plugins that branch on the version), and the
**decisive cross-check that is still pending**: an in-process (plugin-DLL) caller.
Then change the constant and every assert/doc that quotes it, keeping the old
value visible in the RFC as the corrected claim.

**Acceptance** the RFC exists in the house style; the constant is the measured
value with a comment naming the RFC and the evidence; `cargo fmt --check` and
`cargo clippy … -D warnings` clean; **run no test** (the orchestrator runs the
suite in the test VM); no window, no git write.

**Report** the RFC path; the files changed; what still quotes the old value; the
cross-check that remains; confirmation that you ran no test and no git write.
