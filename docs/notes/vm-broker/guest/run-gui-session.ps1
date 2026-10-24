# Allow-listed guest launcher for the FULL suite in the INTERACTIVE session
# (owner rule, 2026-10-18), not a separate four-test implementation. The fixed
# staged run-tests.ps1 payload owns fixtures, per-binary 300 s timeouts and
# results. The broker stays in session 0 and never compiles or runs GUI tests.
#
#   broker -Op guest -Script run-gui-session.ps1 -GuestArgs <filter>
#
# Normal/--ignored/--bless filters have exactly the suite runner's meaning.
# No desktop-free fallback: no session, no screen, no completion, no proof or
# no executed tests means a loud failure. Proof + per-binary logs are written
# under C:\rustamp\out\gui-session for the existing copyout operation.

param([string]$Filter = '')

$ErrorActionPreference = 'Stop'
if ($Filter -like '--desktop-free*') { throw 'desktop-free selections must use the explicit desktop-free runner path' }
$TaskName = 'RustampGuiSession'
$WorkDir = 'C:\rustamp\out\gui-session'
$InnerPath = Join-Path $WorkDir 'inner.ps1'
$DonePath = Join-Path $WorkDir 'done.json'
$ProofPath = Join-Path $WorkDir 'session-proof.json'
$RunnerPath = 'C:\rustamp\code\Rustamp\vm-test-runner\run-tests.ps1'
if (-not (Test-Path -LiteralPath $RunnerPath)) { throw "missing shared runner $RunnerPath; stage with vm-run-tests.ps1 (no old staging fallback)" }

# Explorer's owner is the provisioned auto-logon user. An unattached console
# need not be labelled Active; the inner payload also proves a usable screen.
$interactiveSession = $null
$interactiveUser = $null
$interactiveDomain = $null
$explorer = Get-Process -Name explorer -ErrorAction SilentlyContinue |
    Where-Object { $_.SessionId -gt 0 } | Select-Object -First 1
if ($explorer) {
    $interactiveSession = $explorer.SessionId
    $owner = Get-CimInstance Win32_Process -Filter ("ProcessId = " + $explorer.Id) | Invoke-CimMethod -MethodName GetOwner
    $interactiveUser = $owner.User
    $interactiveDomain = $owner.Domain
} else {
    foreach ($line in (query session 2>$null)) {
        if ($line -match '^\s*>?\s*console\s+(\S+)\s+(\d+)\s+Active') {
            $interactiveUser = $Matches[1]
            $interactiveSession = [int]$Matches[2]
            $interactiveDomain = $env:COMPUTERNAME
            break
        }
    }
}
if ($null -eq $interactiveSession -or $interactiveSession -le 0 -or [string]::IsNullOrWhiteSpace($interactiveUser)) {
    throw 'no interactive desktop user found; refusing to run the suite in session 0'
}

# Cover the same worst case as the host bound (300 s + kill/drain margin per
# binary), with another 120 s reserved by the host for task launch/return.
# Keep these constants in step with vm-run-tests.ps1 and run-tests.ps1.
$binaries = @(Get-ChildItem -LiteralPath 'C:\rustamp\code\Rustamp\target\debug\deps' -Filter '*.exe' -File |
    Where-Object { $_.Name -notin 'rustamp-plughost32.exe', 'rustamp.exe' })
$taskTimeoutSeconds = 60 + (360 * [math]::Max($binaries.Count, 1))
# Do not erase an older live task's evidence or silently start concurrent runs.
$existing = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
if ($existing -and $existing.State -in 'Running', 'Queued') { throw 'a previous interactive suite task is still running/queued; refusing to overlap it' }
if ($existing) { Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false }
if (Test-Path $WorkDir) { Remove-Item -Recurse -Force $WorkDir }
$null = New-Item -ItemType Directory -Force -Path $WorkDir
$runId = [guid]::NewGuid().ToString('N')
# Encode data, never interpolate a filter into executable PowerShell syntax.
$filter64 = [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($Filter))
$inner = @'
$ErrorActionPreference = 'Stop'
$WorkDir = 'C:\rustamp\out\gui-session'
$DonePath = Join-Path $WorkDir 'done.json'
$ProofPath = Join-Path $WorkDir 'session-proof.json'
try {
    $me = (Get-Process -Id $PID).SessionId
    if ($me -le 0) { throw 'interactive suite task landed in session 0, which has no desktop; refusing' }
    Add-Type -AssemblyName System.Windows.Forms
    $screen = [System.Windows.Forms.SystemInformation]::VirtualScreen
    if ($screen.Width -le 0 -or $screen.Height -le 0) { throw 'no usable desktop: the virtual screen is empty' }
    $marker = Test-Path 'C:\rustamp\code\Rustamp'
    if (-not $marker) { throw 'the staged repository marker is missing' }
    $filter = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('__FILTER64__'))
    $result = & 'C:\rustamp\code\Rustamp\vm-test-runner\run-tests.ps1' -Filter $filter
    if (-not $result -or $null -eq $result.results -or $null -eq $result.failed) { throw 'the shared suite payload returned no complete report' }
    $result | Add-Member -NotePropertyName runId -NotePropertyValue '__RUNID__'
    $result | Add-Member -NotePropertyName screen -NotePropertyValue $screen.ToString()
    $result | Add-Member -NotePropertyName marker -NotePropertyValue $marker
    $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $ProofPath -Encoding UTF8
    $ok = ($result.failed -eq 0 -and $result.ran -gt 0 -and $result.testsRun -gt 0 -and
        @($result.results | Where-Object { $_.hung -or $_.stalled }).Count -eq 0)
    @{ ok = $ok; runId = '__RUNID__' } | ConvertTo-Json | Set-Content -LiteralPath $DonePath -Encoding UTF8
    if (-not $ok) { exit 1 }
} catch {
    @{ ok = $false; runId = '__RUNID__'; error = $_.Exception.Message } |
        ConvertTo-Json | Set-Content -LiteralPath $DonePath -Encoding UTF8
    exit 3
}
'@
$inner = $inner.Replace('__FILTER64__', $filter64).Replace('__RUNID__', $runId)
$inner | Set-Content -LiteralPath $InnerPath -Encoding UTF8

# Demand-start only: a trigger one minute in the future could re-run a short
# suite while its proof is being copied. Limited + Interactive, never S4U.
$action = New-ScheduledTaskAction -Execute 'powershell.exe' `
    -Argument ('-NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}"' -f $InnerPath)
$principal = New-ScheduledTaskPrincipal -UserId ("{0}\{1}" -f $interactiveDomain, $interactiveUser) `
    -LogonType Interactive -RunLevel Limited
$settings = New-ScheduledTaskSettingsSet -ExecutionTimeLimit (New-TimeSpan -Seconds $taskTimeoutSeconds)
Register-ScheduledTask -TaskName $TaskName -Action $action -Principal $principal -Settings $settings -Force | Out-Null
$done = $null
try {
    Start-ScheduledTask -TaskName $TaskName
    $launchDeadline = (Get-Date).AddSeconds(60)
    $deadline = (Get-Date).AddSeconds($taskTimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        Start-Sleep -Seconds 2
        if (Test-Path -LiteralPath $DonePath) {
            try { $candidate = Get-Content -LiteralPath $DonePath -Raw | ConvertFrom-Json } catch { $candidate = $null }
            if ($candidate -and $candidate.runId -eq $runId) { $done = $candidate; break }
        }
        $state = (Get-ScheduledTask -TaskName $TaskName).State
        if ($state -eq 'Disabled' -or ((Get-Date) -ge $launchDeadline -and $state -notin 'Running', 'Queued')) {
            $lastResult = (Get-ScheduledTaskInfo -TaskName $TaskName).LastTaskResult
            throw "interactive task is $state with no done sentinel (task result $lastResult); launch/runner failed, not green"
        }
    }
} finally {
    # Bound the task itself as well as each child. Never leave a timed-out task
    # running after unregistering its definition and call that a completed run.
    $task = Get-ScheduledTask -TaskName $TaskName
    if ($task.State -in 'Running', 'Queued') {
        Stop-ScheduledTask -TaskName $TaskName
        $stopDeadline = (Get-Date).AddSeconds(10)
        do {
            Start-Sleep -Milliseconds 200
            $task = Get-ScheduledTask -TaskName $TaskName
        } while ($task.State -in 'Running', 'Queued' -and (Get-Date) -lt $stopDeadline)
        if ($task.State -in 'Running', 'Queued') { throw 'interactive task did not stop within 10 s; its definition is retained; do not stage another run' }
    }
    Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
}
if (-not $done) { throw "the interactive task produced no done sentinel within $taskTimeoutSeconds s; the run is incomplete, not green" }
if ($done.error) { throw ("interactive suite failed: " + $done.error) }
if (-not (Test-Path -LiteralPath $ProofPath)) { throw 'interactive task finished without a session proof; refusing' }
$proof = Get-Content -LiteralPath $ProofPath -Raw | ConvertFrom-Json
if ($proof.runId -ne $runId -or $proof.session -ne $interactiveSession -or $proof.session -le 0 -or -not $proof.marker -or -not $proof.screen) {
    throw 'interactive session proof does not match this run/user session/staged tree; refusing'
}
if ($null -eq $proof.ran -or $proof.ran -le 0 -or $null -eq $proof.testsRun -or $proof.testsRun -le 0) {
    Write-Host 'FAILED: the interactive task executed no test binary/case; this run proves nothing'
}
# Return failures too, in the original suite schema. The host prints every
# diagnostic and applies its failure/hang/stall/empty/bless/repeat exit gates.
$proof
