# X6 — smoke copy-in: the bare name was never the runner's source

**Date:** 2026-10-18. **Status:** runner + docs changed, VM not run.
**Worktree / write boundary:** `wave80-smoke-copyin` worktree only.
Owned paths: `docs/notes/vm-run-smoke.ps1`, `docs/tasks/BETA-smoke.md`,
this card. No crate, guest script, test runner or broker script touched;
no download, no new dependency, no git write, no window, no audio.

## 1. The failure and the wrong premise

The seventh packaged-player smoke run (2026-10-18 08:03) failed at the
copy-in step:

```text
broker op 'copyin' failed: Der Zugriff auf den Pfad "rustamp-plughost32.exe" wurde verweigert.
```

The path in that message has no directory, so the first reading was "the
smoke runner handed the broker a bare file name for a copy-in", and an
earlier lane had already read it that way (BETA-smoke.md defect (4), now
corrected). That reading is disproven by the broker's own log:
`D:\code\rustamp-vm\broker\broker.log` records every request's arguments,
and both failing copy-ins carried the absolute staging path:

```text
2026-10-17T20:38:30 op=copyin args={...,"dest":"C:\\rustamp\\smoke","source":"D:\\code\\rustamp-vm\\staging\\smoke"}
2026-10-18T08:03:20 op=copyin args={...,"dest":"C:\\rustamp\\smoke","source":"D:\\code\\rustamp-vm\\staging\\smoke"}
```

So the runner's copy-in call (`docs/notes/vm-run-smoke.ps1:149`,
`-Source $Stage`, with `$Stage = Join-Path $StageRoot 'smoke'` and
`$StageRoot` defaulting to `D:\code\rustamp-vm\staging`) was never bare and
never relative — not in this worktree's version, and not in any committed
or uncommitted version anywhere (every lane worktree and the main checkout
were swept; the copy-in line is identical in all of them). The earlier
lane's diagnosis (the staging root; its commit 6873089 added only a
comment — the `$Stage` line predates it) was wrong, and its own commit
message said so ("this change assumes the staging root was the reason. It
is not proven"). The error came back unchanged, which is what finally
disproved it.

## 2. What the error actually is

- It is a `System.Management.Automation.RemoteException` — raised INSIDE
  the guest, by the broker's own copy-in mechanics, not while validating
  the runner's arguments. The broker's copy-in clears the guest
  destination (`Remove-Item -Recurse -Force C:\rustamp\smoke` over
  PowerShell Direct) and then copies the staged tree in (`Copy-Item -Path
  (Join-Path $fullSource '*') -Destination $destFull -ToSession $session
  -Recurse -Force`, `vm-broker.ps1:223-231`).
- It is not the broker's path-policy refusal: that refusal is the
  broker's own English throw, `copyin source must live under
  D:\code\rustamp-vm\staging (got <path>)`, and it did not fire.
- It names `rustamp-plughost32.exe` — the staged bundled host binary
  (`player\rustamp-plughost32.exe`). Access denied on delete/overwrite of a
  file means something in the guest holds it open; on Windows a running
  process's image file cannot be deleted or overwritten (a local probe
  with a held-open temp file confirms the denial family and that the
  file-system cmdlets report the full path — so the bare leaf name in this
  message is the form the copy-in's session-side file write reported, not
  a missing source path).
- The holders are the previous run's processes: the guest payload's failure
  paths (`Finish-Failure` → `exit 3`) exit without killing the player; the
  player lives on to its `--seconds 120` backstop, and its eagerly-spawned
  host child can outlive it (the guest script's own recorded
  `hostReaped: false` finding). A lingering `rustamp-plughost32.exe` keeps
  `C:\rustamp\smoke\player\rustamp-plughost32.exe` locked, and the next
  copy-in cannot replace it.
- broker.log's timeline confirms it: the smoke copy-in succeeded three
  times from clean-guest states (10-17 20:33 and 20:34, 10-18 07:41;
  ~3-4 s each) and failed twice, ~1 s after the request — the broker's
  first in-guest file operation — each time right after a run whose
  payload had started the player and failed before the kill step
  (10-17 20:34 → 20:38; 10-18 07:41 → 08:03, ~20 minutes later, so the
  long-lived holder is the host child).

## 3. The broker's rule, as found in its own code

`docs/notes/vm-broker/vm-broker.ps1`:

- `copyin` (lines 206-217): the source must
  `[System.IO.Path]::GetFullPath`-resolve under the host staging root
  `D:\code\rustamp-vm\staging` (`StartsWith`, `OrdinalIgnoreCase`), and the
  destination must resolve under the guest root `C:\rustamp`. The broker
  then clears the destination in the guest and copies the source's
  CONTENTS in over PowerShell Direct.
- `copyout` (lines 234-253): the guest source must start with
  `C:\rustamp\out\` and the host destination must resolve under
  `D:\code\rustamp-vm\staging\out`.
- `guest` (lines 121-134): the script argument must be a plain file name
  matching `^[A-Za-z0-9._-]+\.ps1$` — no path separators — taken from the
  allow-listed `docs/notes/vm-broker/guest/` directory.

## 4. What changed (`docs/notes/vm-run-smoke.ps1`)

- The staged tree is resolved to an absolute full path
  (`[System.IO.Path]::GetFullPath`) where it is built, so even a relative
  `-StageRoot` argument cannot produce a relative copy-in source; the
  results directory is resolved the same way.
- Right before the copy-in call, the runner verifies the broker's rule on
  the host: the source is under the staging root and the destination under
  the guest root (and the copy-out destination under the results root). A
  violation fails as `STEP copyin FAILED: ...` / `STEP copyout FAILED:
  ...` naming the rule, instead of inside the broker.
- The copy-in call is wrapped: a path-policy refusal (the broker's own
  `copyin ... must live under ...` throw) is reported as such; any other
  failure is reported as a broker-side failure after the paths passed,
  with the known case spelled out (access-denied naming a staged file =
  the guest still holds that file open), the remedy (a fresh guest:
  re-run with `-Reset`, which restores the `base-provisioned` checkpoint,
  or stop+start the guest), and a pointer to broker.log's
  `op=copyin args=` line — the one look that settles whether the runner's
  `-Source` was the problem (it was not, in either failing run).
- The stale comment that claimed the staging fix ("so the broker's copyin
  rule ... always passes") is replaced by the accurate statement: the path
  is absolute, and the rule is checked before the call.

## 5. Audit: the same mistake elsewhere in the script

No. Every broker invocation and path was checked:

- copy-in (line 149): source `$Stage` — absolute, under the staging root
  (now resolved + verified). Destination `C:\rustamp\smoke` — absolute,
  under the guest root (now verified).
- copy-out (lines 166, 178): source `C:\rustamp\out\smoke` — an absolute
  guest path, exactly what the broker's copyout rule requires. Destination
  `$OutDir` — absolute, under the staging results root (now resolved +
  verified).
- guest (line 154): `-Script 'run-smoke.ps1'` — a bare NAME, but that is
  the broker's allow-list contract (plain name, no path), not a relative
  path.
- The staging `Copy-Item` calls (lines 125-128, 132): every source
  (`$playerExe`, `$bundledHost`, `$toolExe`, `$fixtureSrc`, the CRT `$p`)
  is an absolute path built with `Join-Path` over an absolute root — the
  same shape as the test runner's staging of the i686 plugin host
  (`vm-run-tests.ps1:117-124`, whose log line "staged the i686 plugin host
  beside the binaries" marks a single binary staged with an absolute
  source, and whose copy-ins at lines 188/202/217/231 all pass absolute
  paths under the same staging root).

## 6. What did NOT change (smoke properties kept)

Byte-for-byte staging of the packaged player and bundled host (nothing
rebuilt), the capture tool built only on the host, the player run with
`--no-audio` (null output) and a real window in the interactive session,
the `275x116` client-size check, the live bundled-host child requirement,
the capture with PNG signature + size gates, the silent-path stdout marker,
the player stop with exit recorded, the hash recompute against the guest
`hashes.json`, and the window step that reports the windows it saw.

## 7. If the broker refuses a path again — where to look first

1. Read `D:\code\rustamp-vm\broker\broker.log` FIRST. The `op=copyin
   args=` (or `op=copyout args=`) line records the exact source and
   destination the broker received. If the source is the absolute staging
   path, the runner's `-Source` is NOT the problem — do not re-diagnose
   it.
2. Distinguish the two refusal shapes:
   - The broker's own policy refusal — `copyin source must live under
     D:\code\rustamp-vm\staging (got <path>)` (and the destination /
     copyout twins) — names the root and the offending path: fix the
     runner's path expression.
   - A `RemoteException` — raised inside the guest by the broker's own
     copy mechanics: the arguments were already accepted; look at the
     GUEST's state, not the runner's paths.
3. The known RemoteException: access denied naming a staged file
   (`rustamp-plughost32.exe`) = the guest still holds that file open — a
   leftover player / host child from a previous smoke run (the payload's
   failure paths exit without killing the player; the host child can
   outlive it). Remedy: a fresh guest — re-run with `-Reset` (restores the
   `base-provisioned` checkpoint) or stop+start the guest. Timing
   signature in broker.log: the failure lands ~1 s after the request (the
   broker's first in-guest file operation); a healthy smoke copy-in takes
   ~3-4 s.
4. The deeper fix belongs to the guest script (its failure paths should
   kill the player before exiting), which this lane does not touch.

## 8. Verification and boundary receipt

- PowerShell parser on the edited script
  (`[System.Management.Automation.Language.Parser]::ParseFile`):
  `docs/notes/vm-run-smoke.ps1` — 0 errors (checked with both the
  Windows PowerShell 5.1 parser and the PowerShell 7 parser). Controls,
  untouched and read-only: `docs/notes/vm-broker/vm-broker.ps1`,
  `docs/notes/vm-broker/invoke-broker.ps1`,
  `docs/notes/vm-broker/guest/run-smoke.ps1`, `docs/notes/vm-run-tests.ps1`
  — 0 errors each.
- No VM, broker, player, host, capture tool, test binary or reference
  binary was run; no window opened; no download, install, commit or git
  write; no new dependency.
- Provenance: clean-tree sources only (the runner, the broker and its
  client, the guest script, the test runner, `broker.log`, the sixth
  run's copied-out proof at `staging\out\smoke\smoke\`, and a local
  file-lock probe with a temp file that was removed afterwards). No
  Winamp source, SDK header, disassembly or dirty-room material read or
  touched.
- `git status --short` (worktree): exactly the three owned files
  (`docs/notes/vm-run-smoke.ps1` and `docs/tasks/BETA-smoke.md` modified,
  `docs/tasks/X6-smoke-copyin.md` new); no tracked modification outside
  them, and the main checkout is clean.
