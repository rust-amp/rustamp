# X3: Runner hung-test survival

**Problem:** The guest half of the test runner (`run-tests.ps1`) ran each test binary with a plain synchronous call (`& $exe @extra 2>&1 | Out-String`) with no per-binary timeout and no progress line. One test that blocks forever therefore blocks the entire run, and the run reports nothing at all.

**Status:** fixed once wrongly and then properly. The first fix put the run in a `Start-Job` and parsed clean, but the job body could not see the two variables it needed, so **no binary ran at all** - the guest reported `Der Ausdruck nach "&" in einem Pipelineelement hat ein ungültiges Objekt erzeugt...`, which is PowerShell saying that `&` was handed nothing. This card describes what is in the tree now and, because a parser cannot see a scope error, exactly why the first version parsed clean and still did nothing.

## Root cause of the broken fix, in the terms that matter

The old body was `{ & $exe @extra 2>&1 }` and it needed exactly two values:

| value | where it came from | what the job body saw | how it arrives now |
| --- | --- | --- | --- |
| `$exe` | the `foreach` loop variable, a full path under `D:\code\Rustamp\target\debug\deps` | `$null` | `-Exe $exe`, a mandatory parameter of `Invoke-TestBinary`, assigned straight to `$psi.FileName` |
| `$extra` | `@('--ignored')` or empty, parsed from the broker's filter argument | `$null` | `-ArgList $extra`, an explicit parameter, quoted per element and joined into `$psi.Arguments` |

`Start-Job` runs its body in its **own runspace**. Script variables are not inherited across that boundary, so `& $exe @extra` collapsed to `& $null @null` and PowerShell raised the "invalid object after &" error before any process existed. Nothing about that is a syntax problem: `[System.Management.Automation.Language.Parser]::ParseFile` returns 0 errors for a script whose job body can never see its variables, which is precisely why the defect survived the first fix and why the fix is now argued in this card rather than only checked with a parser.

Two smaller faults lived in the same six lines:

- **A progress line that named nothing.** `Write-Host ("running: {0}" -f $name)` sat *above* the loop, where `$name` does not exist yet, so it printed `running: ` once and never named a binary.
- **An exit code that could not exist.** `$LASTEXITCODE` is set in the runspace that ran the native command. Inside a job it stays in the job, so the value read after `Receive-Job` was a stale one from whatever native command the *guest script* had run earlier. A failing binary therefore had no path to being reported as a failure.

## What the runner does now

Mechanism: **no job, no `Start-Process` - one `[System.Diagnostics.Process]` per binary**, launched by `Invoke-TestBinary` in `docs/notes/vm-broker/guest/run-tests.ps1`:

- `$psi.FileName = $Exe`, `$psi.Arguments` from `-ArgList`, `UseShellExecute = $false`, `CreateNoWindow = $true` (windowless), stdout and stderr redirected.
- `WaitForExit($TimeoutSeconds * 1000)` is the five-minute bound; `$proc.Kill()` ends a hung binary, and the object *is* the binary, so no second mechanism is needed to reach it (a `cmd` wrapper or a job would have left the hung binary running in the guest).
- The exit code is read from that same object, so it is the binary's own. It is never defaulted to `0`: an unreadable exit code becomes `-1` plus a recorded note.
- Both pipes are drained with `ReadToEndAsync()` on the thread pool from the first moment. A pipe holds ~64 KB, so a chatty child would fill it, block on write and be reported as hung although it is merely loud; the async drain is what makes the bound mean "slow" rather than "noisy".

Why not the two obvious alternatives, both measured locally rather than assumed:

- `Start-Job`: the scope defect above, plus a whole extra PowerShell process per binary.
- `Start-Process -PassThru -NoNewWindow -RedirectStandardOutput/Error`: the combination itself works and the output lands in the files, but in Windows PowerShell 5.1 its `-PassThru` object **loses the exit code** (`Start-Process` enables events on the object, which releases the process handle), so `$proc.ExitCode` reads back `$null` - every binary would carry an exit code of nothing.

## Properties kept

- **Progress line:** `Write-Host ("running: {0}" -f $name)` is inside the loop, after `$name` exists and before the binary starts. It goes to the guest's host stream on purpose: `Write-Output` would enter the success stream, which is the one object the broker turns into `result`, and an extra line there breaks every field read on the host.
- **Five-minute bound:** `$TestTimeoutSeconds = 300` in the guest half. The host half's broker bound is now derived from it (`60 + (300 + 60) * <binary count>`) instead of a flat 3600 s, because a run with several hangs would otherwise be cut off by the broker before the report existed - the same "the run reports nothing at all" failure this card exists to prevent. The two constants cannot share a value across the host/guest allow-list boundary, so they are kept in step by hand and both say so.
- **A hung binary is recorded as hung, with the output it managed to produce:** `hung = $true`, `exit = -1`, `summary = 'HUNG after timeout'`, and the last 25 non-empty output lines as `diagnostic` (the tail is where it stopped; an unbounded dump would bury every other finding). Runner-level findings (a kill that did not land, an unreadable exit code, a pipe a grandchild kept open) ride along in the same `diagnostic` list.
- **The run carries on:** the hung branch `continue`s, so the remaining binaries still run.
- **The host prints `[HNG]`:** `docs/notes/vm-run-tests.ps1` prints the `[HNG]` row, the hang sentence, and the captured lines verbatim. The seconds now come from a new `timeoutSeconds` field on every result instead of `$_.exit`, which printed `timeout of -1 seconds` for the exact binary it was describing.
- **Nothing weakens into passing:** a timeout can only produce `exit = -1`, `hung = $true`; the guest's `failed` count includes it; and the host exits 1 on `failed -ne 0` **or** `hungCount -ne 0`, so a drift in the counting cannot hide a stall.
- **Failure windows and diagnostics for binaries that do finish are unchanged:** the same `test .* FAILED` extraction, the same 10-line window behind each `FAILED`/`panicked at` line, the same first-5-lines when there is no `test result:` summary. Two fidelity improvements come free from not going through `Out-String`: lines are no longer padded to the console width, and stderr (where the test harness writes panics) is a real captured stream rather than error records. Its block now follows the stdout block instead of interleaving racy with it, which is why `panicked at` is matched in its own right - it already was.

## Report output for a hung binary

```
  [HNG] rustamp-test-binary-slow                  HUNG after timeout
         HUNG after timeout of 300 seconds - binary was killed, last output captured
         | captured test output line 1
         | captured test output line 2
         | ...
```

The host headline now also carries the hang count: `ran 34, failed 3, hung 1`.

## Verification

- Both scripts parse with `[System.Management.Automation.Language.Parser]::ParseFile`: **0 errors** each (`run-tests.ps1` 1405 tokens, `vm-run-tests.ps1` 1247 tokens).
- Scoping checked on the AST rather than by eye: inside `Invoke-TestBinary` the only variables referenced are its own parameters (`Exe`, `ArgList`, `TimeoutSeconds`, `DrainMs`) or names it assigns itself - nothing is read from an enclosing scope. `Start-Job`, `Wait-Job`, `Receive-Job` and `$LASTEXITCODE` are invoked **0** times as code in the guest script.
- The shipped helper was executed as shipped against trivial local child processes (no VM, no broker, no test binary, no Rustamp binary): a finished child returned its real exit code and both streams; `--ignored` and a spaced argument arrived quoted; a child that sleeps 120 s was reported `hung=True` after the bound with its pre-hang line kept and no leftover process; a nonexistent path returned `started=False` with the start error as a note instead of ending the run.
- No VM, broker, test binary or reference binary was run. No downloads, no new dependencies, no git writes, no window, no audio.