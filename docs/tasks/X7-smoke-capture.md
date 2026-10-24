# X7 — smoke capture step: the tool succeeded; the PowerShell 5.1 wrapper killed the run

**Date:** 2026-10-18. **Status:** guest script + docs changed, VM not run.
**Worktree / write boundary:** `wave82-smoke-capture` worktree only.
Owned paths: `docs/notes/vm-broker/guest/run-smoke.ps1`,
`docs/tasks/BETA-smoke.md`, this card. The host runner
`docs/notes/vm-run-smoke.ps1` needed **no** change (§5). No crate, test
runner or broker script touched; no download, no new dependency, no git
write, no window, no audio.

## 1. The failure that motivated this

The eighth packaged-player smoke run got further than any before — and died
one statement into the capture step. The guest's own transcript, verbatim:

```text
inner session          1
virtual screen         {X=0,Y=0,Width=1824,Height=768}
config override        C:\rustamp\out\smoke\rustamp.toml
player pid             5704
player alive after 2 s yes
main window matches    1
main window title      Rustamp
main window hwnd       0x1034C
main window client     275x116
main window rect       (130,130)-(421,285)
host child pid         10176
capture --list exit    0
```

then, from the host: `step 'inner-exception' failed in the guest`.

Everything the proof claims up to the capture was true: interactive session
1, the pinned virtual screen, the player alive, the main window found under
the documented title rule at the S01 rest client size, the live bundled-host
child, and `capture --list` exiting 0. The next statement threw — and the
throw was not the tool's.

## 2. What the step after `capture --list` does, line by line

`docs/notes/vm-broker/guest/run-smoke.ps1`, capture section. Line numbers are
**before this lane's edit**; the inner payload is the here-string written to
`inner.ps1`, so inner line = file line − 129.

| file:line | statement | what it assumes |
|---|---|---|
| 362 | `$listRaw = & $ToolExe capture --list 2>&1 \| Out-String` | the tool runs; any stderr is merged into the output |
| 363 | `$listRaw \| Set-Content ... window-list.log` | `$OutDir` exists (the outer half created it) |
| 364 | `Say 'capture --list exit' $LASTEXITCODE` | the exit code is the tool's |
| **365** | `$capRaw = & $ToolExe capture --title Rustamp --out $PngPath 2>&1 \| Out-String` | **← the throw site** |
| 366 | `$capRaw \| Set-Content ... capture.log` | never reached |
| 367–368 | `$capExit = $LASTEXITCODE` / `Say 'capture exit' $capExit` | never reached |
| 369 | `if ($capExit -ne 0) { Finish-Failure 'capture' ... }` | the exit-code gate |
| 373–376 | regex parse of the `captured hwnd=...` line; `Say 'captured'` | the tool's stdout line |
| 377–382 | `Test-Path $PngPath`, PNG signature, ≥512 bytes | the PNG gates |

The transcript's last line (`capture --list exit 0`, file line 364) brackets
the throw to file lines 365–367. Of those, only 365 can throw: 366 is a
`Set-Content` into a directory that already holds `window-list.log`, and 367
is a variable read. **The throw site is `run-smoke.ps1:365`** (inner.ps1 line
236): the capture invocation itself.

## 3. Why it threw: the 5.1 stderr trap, fed by the tool's own note

- The inner payload runs under `powershell.exe` — Windows PowerShell 5.1
  (the one-shot task's action, `run-smoke.ps1:464`). In 5.1, a native
  command's stderr redirected with `2>&1` is wrapped in ErrorRecords, and
  with `$ErrorActionPreference = 'Stop'` (inner line 1) those records are a
  **terminating error**: the statement throws the moment the tool writes its
  first stderr line, mid-pipeline.
- The tool wrote a stderr line — and still exited 0. `xtask/src/capture.rs`:
  `--title` is a case-insensitive **substring** match (`:149`); with more
  than one match it prints `note: N windows matched; using the first
  (hwnd=0x...)` to **stderr** (`:153-159`) and captures the first match
  anyway; the PrintWindow fallback prints a second note (`:218-222`); a tool
  *error* also goes to stderr (`xtask/src/main.rs:58`, exit 1).
- Two windows match `--title Rustamp` in the smoke run: the main window
  (`Rustamp`) and the playlist window (`Rustamp Playlist`,
  `window.rs:1114`). The playlist is **open by default**: `app.rs:237`
  `playlist_open: pl_layout.map(|l| l.visible).unwrap_or(true)`, and the
  smoke run's isolated config (`RUSTAMP_CONFIG` → absent file → defaults)
  takes that default; the presenter creates the OS window
  (`window.rs:1093-1101`). BETA-smoke.md already listed "when main and
  playlist both match" as the known ambiguity.
- `--list` (line 362) has the identical `2>&1` shape and did **not** throw,
  because list mode writes only to stdout (`capture.rs:118-137`) — which is
  exactly why the transcript ends at `capture --list exit 0`.

So: the tool did its job (matched, noted on stderr, captured, exit 0); the
PowerShell 5.1 wrapper turned the note into a terminating error; the catch
leg (`:438`) reported `failed='inner-exception'` with only the exception
message.

**The value that would have named it without a re-run:** `$capRaw` — the
tool's merged stdout+stderr — plus `$LASTEXITCODE`. Both were lost with the
throw: the pipeline never completed, so `$capRaw` was never assigned,
`capture.log` (line 366) was never written, and `Say 'capture exit'` (368)
never ran. The note text survived only inside the exception message (the
`detail` of the copied-out `proof.json`: `note: 2 windows matched; using
the first (hwnd=0x...)`). `window-list.log` (written at 363) did survive and
would have shown both matching windows.

## 4. Was the diagnostics gap the real problem? Partly — and it is fixed too

The step could *bracket* the throw (the transcript's last line) but could
not *name* it: no `capture exit`, no `capture.log`, no tool output, and the
catch named no step, line or category — `inner-exception` plus a message
that only a reader who already knows the 5.1 trap would connect to the
capture invocation. That is the same lesson as X5, one step later: a step
that cannot say what it saw costs a whole VM run.

## 5. What changed (`docs/notes/vm-broker/guest/run-smoke.ps1` only)

- **Capture section:** `$ErrorActionPreference` is relaxed to `'Continue'`
  around **both** tool invocations and restored immediately after. The
  transcript labels, the log files and the exit-code reads are unchanged.
  The tool's stderr notes now flow into `capture.log` / `window-list.log`
  instead of killing the payload. Side effect, also a fix: a genuine tool
  failure (exit 1, `error: ...` on stderr) previously **also** died as an
  unnamed `inner-exception` at the same statement; it is now a named
  `capture` step failure whose message carries the exit code and whose
  `capture.log` carries the tool's own error text.
- **Catch leg:** stamps the throw site into the transcript and the proof —
  `at` (`<script>:<line>` from `$_.InvocationInfo`), the exception type
  (`ActionPreferenceStopException` would have named the mechanism), the
  category and the fully-qualified error id (`NativeCommandError`).
- **Outer half:** the `inner-exception` failure message now includes
  ` at <site>` when the proof carries it (older proofs without `at` format
  unchanged).
- **Header comment:** the capture step's stderr tolerance is stated up
  front.

`docs/notes/vm-run-smoke.ps1` (host runner): **no change needed.** The
guest's throw message already flows through the broker into the host's
existing rethrow (`vm-run-smoke.ps1:229`), so the site appears in the host's
failure line, and X5's failure-path copyout already brings `capture.log`
and the transcript back.

## 6. What did NOT change (smoke properties kept)

- Byte-for-byte staging of the packaged player and bundled host (nothing
  rebuilt; the capture tool is still built on the host only).
- The player still runs with `--no-audio` (null output) and a real window
  in the interactive session; the client size is still asserted `275x116`.
- The live bundled-host child is still required (~10 s).
- The capture is still `xtask capture --title Rustamp` with the exit-code
  gate, the PNG signature gate and the ≥512-byte gate — **no check was
  weakened**; the tool's notes are now merely tolerated, not ignored.
- The window dump on timeout, the silent-path stdout marker, the player stop
  with its exit recorded, and the hash recompute against the guest
  `hashes.json` are untouched.

## 7. Honest narrowing (no check weakened)

The PNG may show the playlist window, not the main window: both match
`--title Rustamp`, the tool captures the first match in `EnumWindows` order,
and the tool has no exact-title or hwnd option (xtask is not this lane's
path). The proof does not claim which window the PNG shows; it records the
selected hwnd/title/size parsed from the tool's own `captured` line, and
now also the tool's `note:` line (preserved in `capture.log`) and the full
`--list`, so the choice is visible. BETA-smoke.md's "what it still does not
prove" states this as a known fact instead of a conditional.

## 8. Still open (not this lane's scope)

- The payload's failure paths still exit without killing the player
  (X6 §7.4): after any inner failure the player lives to its
  `--seconds 120` backstop and can lock `rustamp-plughost32.exe` for the
  next run's copy-in. Remedy: `-Reset`. The kill-on-failure fix belongs to
  a future guest-script lane; it is the top candidate now that this file's
  failure paths have each cost a whole VM run in one morning.
- Which pixels the PNG holds (PrintWindow vs the BitBlt fallback) becomes
  answerable from `capture.log`'s note on the next run — that note was
  previously lost with the throw.

## 9. Verification and boundary receipt

- PowerShell parser on the edited script
  (`[System.Management.Automation.Language.Parser]::ParseFile`), with both
  the PowerShell 7 parser and the Windows PowerShell 5.1 parser:
  `docs/notes/vm-broker/guest/run-smoke.ps1` — **0 errors** (both parsers);
  `docs/notes/vm-run-smoke.ps1` — untouched, **0 errors** (both parsers).
- No VM, broker, player, host, capture tool, test binary or reference
  binary was run; no window opened; no audio; no download, install, commit
  or git write; no new dependency.
- Provenance: clean-tree sources only (the two scripts, `BETA-smoke.md`,
  `xtask/src/capture.rs`, `xtask/src/main.rs`,
  `crates/rustamp-app/src/app.rs:237`,
  `crates/rustamp-app/src/window.rs:1093-1116`, the eighth run's
  transcript). No Winamp source, SDK header, disassembly or dirty-room
  material read or touched.
- `git status --short` (worktree): exactly the owned files —
  `docs/notes/vm-broker/guest/run-smoke.ps1` and `docs/tasks/BETA-smoke.md`
  modified, `docs/tasks/X7-smoke-capture.md` new; the host runner
  unmodified; no tracked modification outside them.
