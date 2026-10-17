# X3: Runner hung-test survival

**Problem:** The guest half of the test runner (`run-tests.ps1`) ran each test binary with a plain synchronous call (`& $exe @extra 2>&1 | Out-String`) at line 63 with no per-binary timeout and no progress line. One test that blocks forever therefore blocks the entire run, and the run reports nothing at all.

**Solution:** Two scripts were modified (both parse-clean, 0 PowerShell parse errors):

## `docs/notes/vm-broker/guest/run-tests.ps1` (guest half)

- **Timeout constant:** `$TestTimeoutSeconds = 300` (5 minutes). The suite's slowest honest binary runs in a couple of minutes; 5 min gives generous margin. A binary that outlives it is **hung, not slow**.
- **Progress line:** `Write-Host ("running: {0}" -f $name)` printed before each binary so a reader of the log knows exactly which one was running when it stopped.
- **Timeout mechanism:** Each binary is run inside a `Start-Job` with `Wait-Job -Timeout $TestTimeoutSeconds`. If the job completes, output is captured normally. If the timeout expires the job is force‑stopped (`Stop-Job -Force`), the captured output (last lines before hang) is recorded as diagnostic, and the result is marked `hung = $true` so the run continues with remaining binaries.
- **Result shape:** Each result object now carries a `hung` boolean field (`$false` by default, `$true` for timed‑out binaries). The existing `exe`, `exit`, `summary`, `failed`, `diagnostic` fields are preserved unchanged, so the host report contract is intact.
- **Comment rationale:** In-file comment explains the timeout value and that a hang is a finding about the code under test, not about the runner.

## `docs/notes/vm-run-tests.ps1` (host half)

- **Hung detection:** The result‑printing loop checks `$_.hung`. If true, it prints `[HNG]` in place of `[ok  ]` or `[FAIL]` — a code that is impossible to mistake for a pass or for a missing binary.
- **Prominent message:** Accompanied by `HUNG after timeout of N seconds - binary was killed, last output captured` so the operator sees at a glance that a hang occurred.
- **Captured output:** The diagnostic lines (the binary's last output) are printed verbatim below the `[HNG]` row, providing evidence of where the binary hung.

## Report output for a hung binary

```
  [HNG] rustamp-test-binary-slow                          HUNG after timeout of 300 seconds - binary was killed, last output captured
         | captured test output line 1
         | captured test output line 2
         | ...
```

The `[HNG]` prefix, the timeout seconds, and the captured diagnostic lines make a hung result unmistakable.

## Verification

- Both scripts parse successfully with `[System.Management.Automation.Language.Parser]::ParseFile` — 0 errors each.
- No VM, broker, test binary, or reference binary was run.
- No downloads, no new dependencies, no git writes, no window, no audio.