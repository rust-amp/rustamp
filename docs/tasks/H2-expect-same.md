# Task card H2 — `expect_same`: identity against a named step (harness)

**Goal** The harness can assert that a capture **differs** from a named earlier
step (`expect_change: "<step>"`), and that it is **identical** to its immediate
predecessor (`expect_change: false`) — but it **cannot** assert that a capture is
**identical to a named earlier step**. That is exactly what a determinism twin
and a reproduction check need, and the confusion has now cost three runs: S04's
three `*_twin` steps and S02R2's four reproduction steps all wrote
`expect_change: "<name>"` meaning *"must equal that"* while the harness read it as
*must differ*, and every one of them failed with "identical … so the action had
no effect".

**Owned paths** `D:\code\rustamp-research\tools\vm-capture\**` — the harness. Do
not touch any `jobs\*.json` except to add nothing; the jobs belong to their lanes.
`jobs\s01r2-open.json` is the frozen export basis and must not change.

**What to add** a documented step field (suggested `expect_same: "<step name>"`,
accepting the same reference forms `expect_change` accepts, including a path) that
asserts the capture is **identical** to that reference, failing loudly with the
differing-pixel count when it is not. Keep `expect_change` exactly as it is —
`true` = must differ, `false` = must be identical to the resolved default
reference — and make the two mutually exclusive with a clear validation error if
both are set. Document the pair in the README with one worked example of each,
since the current wording ("the name of an earlier step") is what misled three
jobs.

**Also** state in the README, plainly, that **`expect_change` can only assert that
something changed, never what changed** — and that a step must therefore never
claim a specific row index, glyph or value in its action text (the existing §12.15
note is easy to miss).

**Hard rules** no sandbox start; no live input; no downloads; no git writes; leave
files uncommitted; never descend into `extracted/`. Extend
`_selftest\Test-ExpectChange.ps1` with cases for the new field (identity satisfied,
identity violated, both fields set → validation error) and keep every existing
case and the frozen job's `-ValidateOnly` run passing.

**Report** the field name and semantics; the files changed; the self-test results;
the frozen job's validation result; whether a real capture is needed to confirm it
(the orchestrator runs one); confirmation that you started no sandbox.
