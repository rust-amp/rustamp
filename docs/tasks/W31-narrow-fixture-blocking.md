# W31: Narrow missing-capture blocking to its consuming test

## Goal and boundary

Keep the seven explicitly built, i686-checked and SHA-256-checked plugin fixture
DLLs. A missing external base291 capture must not abort the ignored suite before
its first binary. Block the consumer loudly, record NOT RUN, continue the other
binaries, and retain a nonzero overall runner exit for a selected missing fixture.

- Lane worktree: `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave131-update`.
- Owned paths: `docs/notes/vm-run-tests.ps1`, `docs/notes/vm-broker/guest/**`, this card.
- Write only these owned files in the lane worktree; nothing under `D:\code\Rustamp`.
- Sources: project runner/test source, CLEANROOM.md, CONTRIBUTING.md and owner request.
- Forbidden: research tree, Winamp source, SDK material, disassembly, downloads,
  new dependencies, crate-source changes, git writes or commits.
- No core/protocol/API contract change; no reference binary, DLL or GUI execution.

## Previous failure and new behavior

Previously the host's selected-binary preflight called `Assert-Base291Captures`
and threw on `D:\code\rustamp-testcaptures\base291\session-proof.json`. The guest
also checked captures globally before its loop. One unavailable capture fixture
therefore prevented all otherwise-ready ignored acceptance tests from executing.

The host now records complete/missing availability in a freshly staged
`base291-captures/runner-status.json`. Complete real capture copies retain their
presence and hash checks. Missing inputs produce a loud named diagnostic and
the original lookup paths; no synthetic captures or proof are manufactured.
The availability file is always copied, including without `-Reset`, so stale
guest captures cannot override the current missing host source.

The guest checks availability and all required files when selecting the one
ignored base291 capture consumer. Missing/empty files, invalid/missing status,
or a pair lost after host staging yield a `blocked = true`, `exit = -1` row:

```text
[BLOCKED] base291_acceptance-....exe NOT RUN: base291_vm_presented_pixels_match_reference_captures; required capture missing or empty: D:\code\rustamp-testcaptures\base291\session-proof.json; ...
```

The row has no passed/failed test-case claim and does not increment `ran` or
`testsRun`. It does increment the failed-binary aggregate. The per-binary log,
copied session proof (`results`), host binary report and W12 ignored inventory
all retain the test name and blocker. The inventory adds this capture test and
prints `[BLOCKED] ... (NOT RUN: ... reason/path ...)`, never PASS.
The full missing-path list is in the named guest availability file.

Other selected binaries continue. Both full `--ignored` and targeted
`--ignored:base291_acceptance` exit 1 while captures are missing; the latter also
hits the existing zero-executed-binary/test gates. Unconfirmed capture environment
is an independent deferred host failure gate, not an early reporting abort.
Normal non-ignored base291 tests do not need captures and remain unaffected.
The Rust capture test itself is unchanged and still rejects absent/invalid real
captures; the runner's presence check cannot certify pixel equality or provenance.
This is narrow blocking, not optional acceptance or a silent skip.

Build failures, missing/empty/wrong-architecture i686 DLLs, failed copies,
hash mismatches and incomplete staging receipts still abort up front unchanged.

## Verification and next guest pass

- Windows PowerShell 5.1 parser: all three changed scripts must have zero errors.
- `test-fixture-staging.ps1` executes extracted production clauses with sentinel
  files and a fake child launcher: no broker, compiler, DLL load or GUI.
- Regression coverage: mixed ignored pass continues, targeted capture refusal,
  NOT RUN counts/log/inventory, independent host failure gate, complete input,
  normal/unrelated selections, post-staging loss, stale guest data, and preserved
  seven-DLL build/staging refusal tests.
- Required host gates: fmt, strict workspace/all-targets/all-features clippy,
  workspace/all-features tests; capture each exit code immediately.
- No guest run is claimed by this lane. After integration, rebuild staging once
  without `-SkipBuild` (the runner hash changed), then run the normal interactive
  ignored pass. Expect actual unrelated acceptance execution instead of the
  pre-suite abort; expect a named base291 BLOCKED/NOT RUN row and exit 1, not
  a fabricated all-green result. Other existing failures/blockers remain visible.

Verification receipts are recorded below after the gates finish.
