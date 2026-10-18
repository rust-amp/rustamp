# W19: verified VM test staging and repeat measurements

## Scope and boundary

- Status: implemented; static validation only, guest verification pending.
- Worktree: `C:\Users\micro\.local\share\opencode\worktree\26de0e\wave92-runner`.
- Write only your own files, and nothing under `D:\code\Rustamp`.
- Owned paths: `docs/notes/vm-run-tests.ps1`,
  `docs/notes/vm-broker/guest/run-tests.ps1`, and this card.
- Allowed evidence: the repository's runner/broker scripts and
  `docs/notes/lane-jobs.md`; no Winamp source, SDK material, disassembly,
  research-tree access, copied third-party code, dependencies or contracts.

## Defects and changes

### Incomplete staging reused by `-SkipBuild`

Locations: original host runner lines 69-81 (enumerate, delete, copy) and
107-117 (i686 build still outside the skip guard); revised host runner lines
47-97 (inventory), 134-149 (skip validation), 153-266 (full-stage-only guard
and completion receipt).

Previously the skip branch enumerated staged executable paths, then the common
staging block deleted their parent tree before copying from those same paths.
It also still built the i686 host. A partial enumeration could instead reach
the guest with no matching test binary.

The full-build path now writes `stage-receipt.json` **last**, after successful
build/staging. It records the Cargo-selected artifact lists, SHA-256 inventory
of every staged file, and SHA-256 inventory of workspace source/build inputs,
current EXE/DLL artifacts and CRT inputs. Skip validates both complete
inventories (including additions/deletions), reuses the staging in place, and
does not build or delete it. Missing/changed/unexpected files are named in a
terminating error before any broker request. Missing/invalid receipts refuse
explicitly because completeness cannot be proved; old stages require one
explicit full run, never an automatic fallback build. Changes to artifacts
outside the selection can conservatively invalidate the receipt too.

The i686 binary build exit is now captured immediately and gated independently
of its following test compilation, so a failed build cannot certify a receipt.

### Empty selection read as green

Locations: historical failure recorded in `docs/notes/lane-jobs.md:283-288`;
the existing host gate was at original lines 439-445 and remains at revised
lines 548-554. Host preflight is at revised lines 268-279, before the first
broker request at line 284. Guest actual-start counting is at lines 59, 304
and 413-414, replacing the original result-row count at line 408.

The existing host `ran=0` failure gate remains inside the per-run verdict and
therefore applies to every repetition (a missing `ran` is also a failure).
Host preflight mirrors the guest's `--bless` / `--ignored` prefix stripping,
wildcard binary-name matching and helper exclusions; zero matches throw before
start/restore/copy-in. This only matches filenames: no test binary is launched
on the host. The guest now counts actually started binaries, not result rows
that also include LEFT OUT, MISSING and DID NOT START entries.

## Repeat semantics and orchestrator commands (NOT executed by this lane)

Locations: host parameters at lines 29-30, per-run verdict at 544-564,
sequential loop/summary/final exit gate at 567-623; guest stall marker at
376-378, host stall aggregation at 383-386.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -Filter dsp_roundtrip -Repeat 20 -RepeatSummaryPath D:\code\rustamp-vm\staging\out\dsp-roundtrip-rate.json
powershell -NoProfile -ExecutionPolicy Bypass -File docs\notes\vm-run-tests.ps1 -SkipBuild -Filter real_decode -Repeat 20 -RepeatSummaryPath D:\code\rustamp-vm\staging\out\real-decode-rate.json
```

Run these separately, sequentially, on the orchestrator's quiet host after
integration. The first creates a new receipt. The second can reuse it only if
the source/artifact/runtime inventories still match. If not, explicitly rerun
without `-SkipBuild`. The output directory must already exist.

- `-Repeat` defaults to 1; without either repeat option there is no repeat
  summary, and the ordinary single guest invocation/report remains.
- Build, staging, start/restore and copy-in happen once, outside the repeat
  loop. `-Reset`, if supplied, restores **once**, not between repetitions.
  This measures warm-guest repetitions, not independent checkpoint restores.
- Every completed repetition keeps the existing failed/hung/bless/zero-ran
  gates. Any failed repetition makes the final command exit nonzero, even if
  all later repetitions pass.
- Stalls include `hung` binaries and the resident's own
  `the resident stopped delivering` escape, detected in the full captured
  output before diagnostic truncation. Stalled runs are a subset of failures,
  not a third mutually exclusive total. No arbitrary wall-time threshold is
  used to guess whether a test stalled.
- Each run reports its guest-request wall time (including broker overhead,
  excluding the one-time build/staging/copy-in). Infrastructure-error records
  instead report elapsed attempt time and an explicit error. An infrastructure
  error stops further requests because the guest might still be running;
  unattempted repetitions are counted and force nonzero exit.
- The final `RATE ...` line names the filter, pass fraction/percentage,
  failed/stalled fractions, requested count and unattempted count. Optional
  JSON preserves those totals and every run's timing/counts/error. A failed
  guest run does not suppress the summary.
- Filters still select binaries, not individual Rust test names. No claim is
  made that a selected binary contains a nonzero number of Rust test cases;
  checking that would require executing its harness in the guest.

## Validation and limitations

- `Parser::ParseFile` on `docs/notes/vm-run-tests.ps1`: PASS, 0 parse errors.
- `Parser::ParseFile` on `docs/notes/vm-broker/guest/run-tests.ps1`: PASS,
  0 parse errors.
- This Markdown card is not a PowerShell script; PowerShell parse: not applicable.
- `git diff --check`: PASS.
- **Nothing was executed against the guest. No test, VM, broker, cargo command,
  Rust gate, download, dependency installation or commit was run.** The Rust
  gates were deliberately omitted to leave the host free for the orchestrator.
- Not verified: real full-build receipt creation/reuse, interrupted staging
  rejection at runtime, real broker serialization/guest output, real repeated
  exit codes/timings, timeout/30-second stall classification in a live run, or
  bless copy-out. Parsing and source review do not establish those outcomes.
- Final worktree status is reported to the orchestrator; only the three owned
  paths may differ. The lane neither writes nor checks the canonical trees;
  the orchestrator owns their boundary audit.
