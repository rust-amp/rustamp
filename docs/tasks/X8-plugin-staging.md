# X8 — Plugin fixture staging report for the loader acceptance tests

**Date:** 2026-10-18. **Scope:** make the VM runner stage the freely licensed
plugin fixture and report its staging outcome plainly. No test was changed; no
binary was built, run, or loaded; no VM or broker ran.

**Boundary:** only this worktree. Owned paths touched: `docs/notes/vm-run-tests.ps1`,
`docs/notes/vm-broker/guest/run-tests.ps1`, and this card. No crate, no lockfile,
no smoke runner, no broker script, no git write.

## 1. What the two loader tests expect (variable and layout)

Both tests resolve the same fixture name through the same variable, with the
same nested layout first:

- `crates/rustamp-plughost/tests/dll_acceptance.rs:44` reads
  `std::env::var("RUSTAMP_TESTPLUGINS")`. When the value is a directory it tries
  `in_openmpt.dll` directly (`:49-52`) and then
  `libopenmpt/Winamp/in_openmpt.dll` (`:53-56`). It falls back to the guest
  staged paths `C:\rustamp\testplugins\libopenmpt\Winamp\in_openmpt.dll` (`:60`)
  and `C:\rustamp\testplugins\in_openmpt.dll` (`:64`), then to the orchestrator
  path `D:\code\rustamp-testplugins\libopenmpt\Winamp\in_openmpt.dll` (`:70`).
  A missing binary is a loud `panic!`, never a silent skip (`:75-82`).
- `crates/rustamp-plughost/src/loader.rs:1098-1099` requires
  `std::env::var_os("RUSTAMP_TESTPLUGINS")` via `expect` (unset means panic,
  i.e. loud failure, not a skip). A file value is used directly (`:1101`); a
  directory value tries `libopenmpt/Winamp/in_openmpt.dll` first (`:1104`) and
  then `in_openmpt.dll` (`:1108`), and asserts the file name is exactly
  `in_openmpt.dll` (`:1111-1115`).

So the contract is: environment variable `RUSTAMP_TESTPLUGINS` pointing at a
directory whose tree contains `libopenmpt\Winamp\in_openmpt.dll` (plus its
sibling dependency `openmpt-mpg123.dll` beside it, per P4 §1), or at the file
itself.

## 2. What was staged and how (paths, names, why)

No new staging mechanism was invented. The runner already staged the fixture
exactly the way it stages the other two fixtures (X4 lane), and that shape
satisfies both lookups above, so it is kept unchanged:

- Host (`docs/notes/vm-run-tests.ps1:222-237`): when the owner-approved host
  directory `D:\code\rustamp-testplugins` exists, it is copied wholesale through
  a staging copy (`<stage-dir>\testplugins`) and then `copyin` to guest path
  `C:\rustamp\testplugins`. Wholesale copy preserves the
  `libopenmpt\Winamp\in_openmpt.dll` nesting and the sibling dependency, which
  is why a flat single-file stage was not used: the loader resolves
  `openmpt-mpg123.dll` from the plugin's own directory.
- Guest (`docs/notes/vm-broker/guest/run-tests.ps1:64-67`): when
  `C:\rustamp\testplugins` exists, the guest exports
  `RUSTAMP_TESTPLUGINS=C:\rustamp\testplugins` (directory form, which both tests
  resolve, including the nested layout).

This mirrors the proven media/skins pattern deliberately: read-only copies plus
an opt-in environment variable (`RUSTAMP_TESTDATA` /
`RUSTAMP_TESTSKINS` / `RUSTAMP_TESTPLUGINS`), staged only when the owner's host
directory exists, with the tests refusing/skipping loudly otherwise.

## 3. What the report says now (this lane's change)

Staging existed; plain reporting of its outcome did not. Added:

- Host staging records `$pluginsStaged` (`vm-run-tests.ps1:228-236`): `$true`
  only after the staging copy plus broker `copyin` succeed (any copy failure
  still throws, so `$false` means "never staged", never "maybe staged").
- Step 5 prints one fixture line on **every** run, whatever the filter
  (`vm-run-tests.ps1:266-281`):
  - `plugin fixture: staged (host ... copied; guest reports RUSTAMP_TESTPLUGINS=...)`,
  - `plugin fixture: staged on the host, but the guest reported no
    RUSTAMP_TESTPLUGINS value; Group D tests cannot prove anything in this run`,
  - `plugin fixture: NOT STAGED (no D:\code\rustamp-testplugins on the host,
    so RUSTAMP_TESTPLUGINS is unset); Group D tests refuse loudly and prove
    nothing in this run`.
- Guest result object carries `testplugins = $env:RUSTAMP_TESTPLUGINS`
  (`guest/run-tests.ps1:358-370`), i.e. guest ground truth (`$null` when
  unstaged). Additive field; older guest scripts simply report nothing, which
  the host phrases as the middle case above rather than as a pass.
- Ignored summary (`vm-run-tests.ps1:334-370`): Group D items that neither
  passed nor failed while the host never staged print
  `[MISSING FIXTURE] ... (no D:\code\rustamp-testplugins on the host;
  RUSTAMP_TESTPLUGINS unset)` and are counted separately; the totals line now
  reads `{ran} ran, {blocked} blocked ..., {missing} missing plugin fixture,
  {total} total in inventory`. A missing fixture can therefore never read as a
  pass or a quiet `[NOT RUN]`.

## 4. Deliberately not claimed / left for the orchestrator

- **Expectation vs fixture:** both tests' lookups are satisfied by the wholesale
  directory stage (nested layout first, flat fallback, exact `in_openmpt.dll`
  file name). No mismatch found in the tests' expectations. The actual host
  fixture bytes were **not** inspected in this lane (no VM, no execution), so
  the orchestrator should still confirm that
  `D:\code\rustamp-testplugins\libopenmpt\Winamp\in_openmpt.dll` exists with its
  sibling `openmpt-mpg123.dll` before the guest pass.
- **Execution proof still outstanding:** schedule one
  `vm-run-tests.ps1 -Filter '--ignored'` guest pass; Group D proves the P5
  security path only when both loader tests report `[PASS]`.
- **Untouched:** Group E audio-endpoint decisions, Group C fixtures, Group F
  corpus staging — all behave exactly as X4 left them.

## 5. Preserved runner properties (verified by read, not by run)

- `--ignored` prefix handling and `--ignored:<name>` binary-name filtering:
  unchanged (`guest/run-tests.ps1:36-45`).
- Serial test threads `--test-threads=1` in both normal and `--ignored` modes:
  unchanged (`guest/run-tests.ps1:37,43`).
- Staged fixture examples (`decoder_fixture` / `dsp_fixture` build, stage,
  `RUSTAMP_DECODER_FIXTURE` / `RUSTAMP_DSP_FIXTURE` export): unchanged
  (host `:60-98`, guest `:69-82`).
- i686 plughost binary plus i686 plughost test binaries, static CRT,
  x86_64-plughost-test exclusion, timeout accounting over both binary sets:
  unchanged (host `:100-155,251`).

## 6. Verification and boundary receipt

- PowerShell parser
  (`[System.Management.Automation.Language.Parser]::ParseFile`), error counts:
  `docs/notes/vm-run-tests.ps1`: **0 errors** (375 lines);
  `docs/notes/vm-broker/guest/run-tests.ps1`: **0 errors** (370 lines).
- No `cargo` invocation, no test run, no VM start, no broker call, no window,
  no audio, no download, no new dependency, no git write or commit. Build
  outputs untouched (worktree-local `target/` not modified by this lane).
- No Winamp source, SDK, disassembly, or forbidden-tree contents read; no
  reference/plugin binary inspected or executed.
- `git status --short` shows exactly the two edited runners plus this card and
  nothing else (see report).
